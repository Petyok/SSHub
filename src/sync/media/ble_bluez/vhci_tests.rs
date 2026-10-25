//! The real BlueZ transport against a real BlueZ on two virtual controllers (`hci_vhci` + BlueZ's
//! `btvirt`): the public `media::listen_with` serves the channel on one controller
//! (`SSHUB_BLE_ADAPTER`), a `bluer` GATT client plays the phone page on the other
//! (`SSHUB_BLE_TEST_CLIENT`) with the link layer's test-only phone side.
//!
//! `#[ignore]`d: it needs those controllers, a running `bluetoothd` and a system D-Bus, which
//! only the container of `tests/ble-vhci/` provides (see `poc/results/ble.md`):
//!
//! ```text
//! SSHUB_BLE_ADAPTER=hci0 SSHUB_BLE_TEST_CLIENT=hci1 \
//!   cargo test --lib --features bluetooth vhci_tests -- --ignored --nocapture --test-threads=1
//! ```

use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use bluer::gatt::remote::Characteristic;
use bluer::gatt::{CharacteristicReader, CharacteristicWriter};
use bluer::{AddressType, Device, Uuid};
use futures::{pin_mut, StreamExt};
use tokio::time::{sleep, timeout};

use super::super::ble_link::{
    info_bytes, local_name, valid_frame, Event, Link, B_USER, INFO_UUID, RX_UUID, SERVICE_UUID,
    TX_UUID,
};
use super::super::{listen_with, ListenOpts, MediaEvent, MediaKind};

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap()
}

fn frame(kind: u8, len: usize, seed: u8) -> Vec<u8> {
    let mut f = b"SWR1".to_vec();
    f.extend_from_slice(&[0, 1, kind]);
    f.extend((0..len - 7).map(|i| (i as u8).wrapping_mul(29).wrapping_add(seed)));
    f
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} (see the module doc)"))
}

/// Every advertising report in the HCI trace: all from the terminal, the name exactly the
/// contract name, the only other fields flags and our 128-bit service UUID.
fn check_air(trace: &str, name: &str, addr: &str, forbidden: &[String]) {
    let mut blocks: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    for line in trace.lines() {
        if line.starts_with(['>', '<', '@']) {
            blocks.extend(cur.take());
            if line.starts_with('>') {
                cur = Some(String::new());
            }
        }
        if let Some(b) = cur.as_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    blocks.extend(cur);
    let reports: Vec<&String> = blocks
        .iter()
        .filter(|b| b.contains("Advertising Report"))
        .collect();
    assert!(
        reports.len() >= 2,
        "expected advertising and scan response reports, saw {}",
        reports.len()
    );
    for b in &reports {
        for l in b
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("Address:"))
        {
            assert!(l.contains(addr), "report from another device: {l}");
        }
        for bad in [
            "Manufacturer",
            "Service Data",
            "Appearance",
            "Name (short)",
            "Class of Device",
            "Device ID",
            "Connection Interval",
            "Target",
        ] {
            assert!(!b.contains(bad), "{bad} in an advertisement:\n{b}");
        }
        for w in forbidden {
            assert!(!b.contains(w.as_str()), "{w:?} leaked into:\n{b}");
        }
        for l in b.lines().map(str::trim).filter(|l| l.starts_with("Name")) {
            assert_eq!(l, format!("Name (complete): {name}"), "advertised name");
        }
    }
    assert!(reports
        .iter()
        .any(|b| b.contains(&format!("Name (complete): {name}"))));
    assert!(reports.iter().any(|b| b.contains("128-bit Service UUIDs")));
    eprintln!(
        "air ok: {} reports, name {name:?}, flags and one 128-bit UUID",
        reports.len()
    );
}

/// Stops btmon with the test, pass or fail.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The phone page: HELLO over RX, TX notifications in, with the test-only link of the phone side.
struct Phone {
    link: Link,
    writer: CharacteristicWriter,
    reader: CharacteristicReader,
    frames: Vec<Vec<u8>>,
    events: Vec<Event>,
}

async fn characteristic(device: &Device, svc: &str, chr: &str) -> Characteristic {
    for s in device.services().await.expect("services") {
        if s.uuid().await.unwrap() == uuid(svc) {
            for c in s.characteristics().await.expect("characteristics") {
                if c.uuid().await.unwrap() == uuid(chr) {
                    return c;
                }
            }
        }
    }
    panic!("characteristic {chr} not found: the GATT service did not register as published");
}

/// The terminal's device object on the client's BlueZ: ConnectDevice by address (btdev gives BlueZ
/// no DeviceFound to discover it by), or the object the scan already created.
async fn reach(adapter: &bluer::Adapter, addr: bluer::Address) -> Device {
    match adapter.connect_device(addr, AddressType::LePublic).await {
        Ok(d) => d,
        Err(e) if e.kind == bluer::ErrorKind::AlreadyExists => adapter.device(addr).unwrap(),
        Err(e) => panic!("connect to the terminal: {e}"),
    }
}

async fn connect(device: &Device, tag: &str) -> Phone {
    for attempt in 0..3 {
        match device.connect().await {
            Ok(()) => break,
            Err(e) if e.kind == bluer::ErrorKind::AlreadyConnected => break,
            Err(e) if attempt < 2 => eprintln!("connect retry: {e}"),
            Err(e) => panic!("connect failed: {e}"),
        }
    }
    let info = characteristic(device, SERVICE_UUID, INFO_UUID).await;
    assert_eq!(
        info.read().await.unwrap(),
        info_bytes(tag),
        "INFO is proto, max frame, tag"
    );
    let tx = characteristic(device, SERVICE_UUID, TX_UUID).await;
    let rx = characteristic(device, SERVICE_UUID, RX_UUID).await;
    // contract order: notifications first, then the first write
    let reader = tx.notify_io().await.expect("notify session");
    let writer = rx.write_io().await.expect("write socket");
    let mtu = writer.mtu().min(reader.mtu());
    let proposal = 244.min(mtu.saturating_sub(3)).max(20);
    eprintln!(
        "connected: ATT mtu {} write, {} notify; proposing {proposal}",
        writer.mtu(),
        reader.mtu()
    );
    Phone {
        link: Link::phone(Instant::now(), proposal),
        writer,
        reader,
        frames: Vec::new(),
        events: Vec::new(),
    }
}

impl Phone {
    /// One turn: flush the link's messages, take at most one notification, tick.
    async fn turn(&mut self) {
        for m in self.link.take_outbox() {
            self.writer.send(&m).await.expect("write");
        }
        if let Ok(Ok(m)) = timeout(Duration::from_millis(20), self.reader.recv()).await {
            self.link.on_message(Instant::now(), &m);
        }
        self.link.tick(Instant::now());
        for e in self.link.take_events() {
            if let Event::Frame(f) = &e {
                self.frames.push(f.clone());
            }
            self.events.push(e);
        }
    }
}

struct Server {
    rx: Receiver<MediaEvent>,
    statuses: Vec<String>,
    replies: Vec<Vec<u8>>,
    sent: usize,
}

impl Server {
    fn drain(&mut self) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                MediaEvent::Status(_, s) => {
                    eprintln!("  [terminal] {s}");
                    self.statuses.push(s);
                }
                MediaEvent::Reply(_, f) => self.replies.push(f),
                MediaEvent::Sent(_) => self.sent += 1,
                MediaEvent::Progress(..) => {}
            }
        }
    }
}

/// Runs both sides until `done` or `secs` pass.
async fn pump(
    phone: &mut Phone,
    server: &mut Server,
    secs: u64,
    done: impl Fn(&Phone, &Server) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        phone.turn().await;
        server.drain();
        if done(phone, server) {
            return;
        }
        assert!(
            Instant::now() < end,
            "timed out after {secs} s; terminal said {:?}; phone events {:?}",
            server.statuses,
            phone.events
        );
    }
}

async fn scenario(server_adapter: &str, client_adapter: &str, btmon_path: &std::path::Path) {
    let forbidden: Vec<String> = std::env::var("SSHUB_BLE_TEST_FORBIDDEN")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let (tx, rx) = channel();
    let handle = listen_with(
        &[MediaKind::Ble],
        tx,
        ListenOpts {
            accept: valid_frame,
            keep_listening: true,
        },
    );
    let tag = handle
        .ble_tag()
        .expect("the bluetooth feature is on")
        .to_string();
    let name = local_name(&tag);
    let mut server = Server {
        rx,
        statuses: Vec::new(),
        replies: Vec::new(),
        sent: 0,
    };
    // the transport reports a failure to reach BlueZ as a status
    let started = Instant::now();
    while !server.statuses.iter().any(|s| s.contains("pick SSHub-")) {
        server.drain();
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "transport did not start: {:?}",
            server.statuses
        );
        sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !server.statuses.iter().any(|s| s.contains("unavailable")),
        "BlueZ unreachable: {:?}",
        server.statuses
    );

    let session = bluer::Session::new().await.expect("system bus");
    let adapter = session.adapter(client_adapter).expect("client adapter");
    adapter
        .set_powered(true)
        .await
        .expect("power the client controller");

    // 1. what the air carries: scan on the client for a few seconds, then read the HCI trace
    //    (btdev reports no RSSI, so the kernel drops BlueZ's DeviceFound; the reports still
    //    reach the client controller and btmon shows them byte for byte)
    let server_addr = session
        .adapter(server_adapter)
        .expect("server adapter")
        .address()
        .await
        .expect("server address");
    {
        let events = adapter.discover_devices().await.expect("discovery starts");
        pin_mut!(events);
        let _ = timeout(Duration::from_secs(6), async {
            while events.next().await.is_some() {}
        })
        .await;
    }
    sleep(Duration::from_millis(500)).await;
    let trace = std::fs::read_to_string(btmon_path).expect("btmon trace");
    check_air(&trace, &name, &server_addr.to_string(), &forbidden);
    let device = reach(&adapter, server_addr).await;

    // 2. first session: a kind 1 frame down and a kind 2 frame up at three sizes
    handle.send_frame(frame(1, 200, 1));
    let mut phone = connect(&device, &tag).await;
    pump(&mut phone, &mut server, 20, |p, _| !p.frames.is_empty()).await;
    assert_eq!(
        phone.frames[0],
        frame(1, 200, 1),
        "the queued frame arrives right after the handshake"
    );
    eprintln!(
        "  handshake and first frame ok, messages of {:?}",
        phone.events.first()
    );
    for (i, size) in [200usize, 3000, 20000].into_iter().enumerate() {
        let t0 = Instant::now();
        let down = frame(1, size, 10 + i as u8);
        let up = frame(2, size, 50 + i as u8);
        let (frames_before, replies_before, sent_before) =
            (phone.frames.len(), server.replies.len(), server.sent);
        handle.send_frame(down.clone());
        phone.link.send_frame(Instant::now(), up.clone()).unwrap();
        pump(&mut phone, &mut server, 60, |p, s| {
            p.frames.len() > frames_before
                && s.replies.len() > replies_before
                && s.sent > sent_before
        })
        .await;
        assert_eq!(phone.frames.last().unwrap(), &down, "down {size}");
        assert_eq!(server.replies.last().unwrap(), &up, "up {size}");
        eprintln!(
            "  {size} B down and up in {:.2} s",
            t0.elapsed().as_secs_f64()
        );
    }

    // 3. the phone leaves: the terminal advertises again under the same name
    drop(phone);
    device.disconnect().await.expect("disconnect");
    let t0 = Instant::now();
    while !server
        .statuses
        .iter()
        .any(|s| s.contains("still advertising as"))
    {
        server.drain();
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "no re-advertising: {:?}",
            server.statuses
        );
        sleep(Duration::from_millis(100)).await;
    }
    assert!(server
        .statuses
        .iter()
        .any(|s| s.contains(&format!("still advertising as {name}"))));
    // the same peripheral again: BlueZ only connects while the terminal really advertises
    let device = reach(&adapter, server_addr).await;
    eprintln!("reconnected to {server_addr}, same tag {tag}");

    // 4. second session: the current frame is pushed again, frames still flow
    let last_down = frame(1, 20000, 12);
    let mut phone = connect(&device, &tag).await;
    pump(&mut phone, &mut server, 40, |p, _| !p.frames.is_empty()).await;
    assert_eq!(
        phone.frames[0], last_down,
        "the step's frame is pushed again to the new session"
    );
    let up = frame(2, 3000, 99);
    let before = server.replies.len();
    phone.link.send_frame(Instant::now(), up.clone()).unwrap();
    pump(&mut phone, &mut server, 40, |_, s| s.replies.len() > before).await;
    assert_eq!(server.replies.last().unwrap(), &up);

    // 5. closing the channel says goodbye
    drop(handle);
    pump(
        &mut phone,
        &mut Server {
            rx: channel().1,
            statuses: Vec::new(),
            replies: Vec::new(),
            sent: 0,
        },
        20,
        |p, _| p.events.iter().any(|e| matches!(e, Event::Closed(_))),
    )
    .await;
    assert!(
        phone.events.contains(&Event::Closed(B_USER)),
        "{:?}",
        phone.events
    );
    eprintln!("vhci scenario ok");
}

#[test]
#[ignore = "needs two BlueZ controllers, see tests/ble-vhci/ and poc/results/ble.md"]
fn the_real_bluez_transport_exchanges_frames_with_a_gatt_client_on_the_second_controller() {
    let server = env("SSHUB_BLE_ADAPTER");
    let client = env("SSHUB_BLE_TEST_CLIENT");
    assert_ne!(server, client);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // the HCI trace of the whole run: the air check reads what the controllers really exchanged
    let trace = std::env::temp_dir().join("sshub-ble-vhci-btmon.txt");
    let btmon = std::process::Command::new("stdbuf")
        .args(["-oL", "btmon"])
        .stdout(std::fs::File::create(&trace).unwrap())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("btmon (package bluez) is in the test image");
    let _stop = KillOnDrop(btmon);
    std::thread::sleep(Duration::from_millis(500));
    rt.block_on(scenario(&server, &client, &trace));
}
