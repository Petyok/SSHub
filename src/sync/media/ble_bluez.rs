//! Linux transport of the Bluetooth channel: BlueZ over D-Bus through the `bluer` crate (the official
//! BlueZ Rust binding). `bluer` is async on tokio; that runtime is confined to ONE thread owned by this
//! transport (a `current_thread` runtime, nothing async leaks out): the driver in `ble.rs` stays a plain
//! thread talking to it over `mpsc` channels, as the rest of sync does.
//!
//! GATT is served with BlueZ's "acquire" (file descriptor) methods, so every write and notification
//! is one datagram and the MTU is known: `AcquireWrite` for RX, `AcquireNotify` for TX. No pairing, no
//! bonding: the characteristics need no encryption and none is relied on (contract C20).

use std::future::pending;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use bluer::adv::{Advertisement, AdvertisementHandle, Type};
use bluer::gatt::local::{
    characteristic_control, Application, Characteristic, CharacteristicControlEvent,
    CharacteristicNotify, CharacteristicNotifyMethod, CharacteristicRead, CharacteristicWrite,
    CharacteristicWriteMethod, Service,
};
use bluer::gatt::{CharacteristicReader, CharacteristicWriter};
use bluer::{Address, Uuid};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::ble::{Poll, Transport};
use super::ble_link::{
    info_bytes, local_name, BLE_CHUNK_MIN, INFO_UUID, RX_UUID, SERVICE_UUID, TX_UUID,
};

/// Driver to the Bluetooth thread.
enum Down {
    Send(Vec<u8>),
    Advertise(bool),
    Disconnect,
    Stop,
}

/// Bluetooth thread to the driver.
enum Up {
    Connected(usize),
    Message(Vec<u8>),
    Disconnected,
    Failed(String),
}

pub(super) struct BluezTransport {
    up: Receiver<Up>,
    down: UnboundedSender<Down>,
    thread: Option<JoinHandle<()>>,
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

impl BluezTransport {
    /// Starts the Bluetooth thread. A failure to reach BlueZ arrives as `Poll::Failed`.
    pub(super) fn start(tag: &str) -> BluezTransport {
        let (up_tx, up) = channel();
        let (down, down_rx) = unbounded_channel();
        let tag = tag.to_string();
        let failed = up_tx.clone();
        let thread = std::thread::Builder::new()
            .name("sshub-ble-bluez".into())
            .spawn(move || {
                let outcome = match runtime() {
                    Ok(rt) => rt.block_on(serve(tag, up_tx, down_rx)),
                    Err(e) => Err(format!("cannot start the Bluetooth thread: {e}")),
                };
                if let Err(why) = outcome {
                    let _ = failed.send(Up::Failed(why));
                }
            })
            .ok();
        BluezTransport { up, down, thread }
    }
}

impl Transport for BluezTransport {
    fn poll(&mut self, wait: Duration) -> Poll {
        match self.up.recv_timeout(wait) {
            Ok(Up::Connected(cap)) => Poll::Connected { notify_cap: cap },
            Ok(Up::Message(m)) => Poll::Message(m),
            Ok(Up::Disconnected) => Poll::Disconnected,
            Ok(Up::Failed(why)) => Poll::Failed(why),
            Err(RecvTimeoutError::Timeout) => Poll::Idle,
            Err(RecvTimeoutError::Disconnected) => {
                Poll::Failed("the Bluetooth thread ended".into())
            }
        }
    }

    fn send(&mut self, msg: &[u8]) -> bool {
        self.down.send(Down::Send(msg.to_vec())).is_ok()
    }

    fn advertise(&mut self, on: bool) {
        let _ = self.down.send(Down::Advertise(on));
    }

    fn disconnect(&mut self) {
        let _ = self.down.send(Down::Disconnect);
    }
}

impl Drop for BluezTransport {
    fn drop(&mut self) {
        let _ = self.down.send(Down::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// An adapter exists. Opens a throwaway runtime and D-Bus connection, so call it rarely.
pub(super) fn present() -> bool {
    adapter_name().is_some()
}

pub(super) fn devices() -> Vec<String> {
    adapter_name()
        .map(|n| vec![format!("adapter {n}")])
        .unwrap_or_default()
}

fn adapter_name() -> Option<String> {
    let rt = runtime().ok()?;
    rt.block_on(async {
        let session = bluer::Session::new().await.ok()?;
        let adapter = adapter_of(&session).await.ok()?;
        Some(adapter.name().to_string())
    })
}

/// The adapter to serve on: the one named by `SSHUB_BLE_ADAPTER` (for example `hci1` on a machine
/// with several, or a virtual controller in a test), else BlueZ's default.
async fn adapter_of(session: &bluer::Session) -> bluer::Result<bluer::Adapter> {
    match std::env::var("SSHUB_BLE_ADAPTER") {
        Ok(name) if !name.is_empty() => session.adapter(&name),
        _ => session.default_adapter().await,
    }
}

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).expect("contract UUIDs are valid")
}

/// What a central connection holds on the Bluetooth thread.
#[derive(Default)]
struct Central {
    reader: Option<CharacteristicReader>,
    writer: Option<CharacteristicWriter>,
    address: Option<Address>,
}

impl Central {
    fn connected(&self) -> bool {
        self.writer.is_some() || self.reader.is_some()
    }
}

async fn serve(
    tag: String,
    up: Sender<Up>,
    mut down: UnboundedReceiver<Down>,
) -> Result<(), String> {
    let session = bluer::Session::new()
        .await
        .map_err(|e| format!("cannot reach bluetoothd (is it running?): {e}"))?;
    let adapter = adapter_of(&session)
        .await
        .map_err(|e| format!("no Bluetooth adapter: {e}"))?;
    adapter
        .set_powered(true)
        .await
        .map_err(|e| format!("cannot power the adapter on: {e}"))?;

    let service = uuid(SERVICE_UUID);
    let advertisement = Advertisement {
        advertisement_type: Type::Peripheral,
        service_uuids: [service].into_iter().collect(),
        local_name: Some(local_name(&tag)),
        discoverable: Some(true),
        ..Default::default()
    };

    let (rx_control, rx_handle) = characteristic_control();
    let (tx_control, tx_handle) = characteristic_control();
    let info = info_bytes(&tag);
    let app = Application {
        services: vec![Service {
            uuid: service,
            primary: true,
            characteristics: vec![
                Characteristic {
                    uuid: uuid(RX_UUID),
                    write: Some(CharacteristicWrite {
                        write: true,
                        write_without_response: true,
                        method: CharacteristicWriteMethod::Io,
                        ..Default::default()
                    }),
                    control_handle: rx_handle,
                    ..Default::default()
                },
                Characteristic {
                    uuid: uuid(TX_UUID),
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: CharacteristicNotifyMethod::Io,
                        ..Default::default()
                    }),
                    control_handle: tx_handle,
                    ..Default::default()
                },
                Characteristic {
                    uuid: uuid(INFO_UUID),
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(move |_req| {
                            let value = info.clone();
                            async move { Ok(value) }.boxed()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let _app_handle = adapter
        .serve_gatt_application(app)
        .await
        .map_err(|e| format!("cannot publish the GATT service: {e}"))?;

    tokio::pin!(rx_control);
    tokio::pin!(tx_control);
    let mut advertising: Option<AdvertisementHandle> = None;
    let mut central = Central::default();
    loop {
        tokio::select! {
            cmd = down.recv() => match cmd {
                None | Some(Down::Stop) => break,
                Some(Down::Advertise(true)) => {
                    if advertising.is_none() {
                        match adapter.advertise(advertisement.clone()).await {
                            Ok(h) => advertising = Some(h),
                            Err(e) => return Err(format!("cannot advertise: {e}")),
                        }
                    }
                }
                Some(Down::Advertise(false)) => advertising = None,
                Some(Down::Send(m)) => {
                    let sent = match &central.writer {
                        Some(w) => w.send(&m).await.is_ok(),
                        None => false,
                    };
                    if !sent && central.connected() {
                        central = Central::default();
                        let _ = up.send(Up::Disconnected);
                    }
                }
                Some(Down::Disconnect) => {
                    if let Some(addr) = central.address.take() {
                        if let Ok(device) = adapter.device(addr) {
                            let _ = device.disconnect().await;
                        }
                    }
                    central = Central::default();
                }
            },
            event = rx_control.next() => match event {
                Some(CharacteristicControlEvent::Write(req)) => {
                    central.address = Some(req.device_address());
                    match req.accept() {
                        Ok(r) => central.reader = Some(r),
                        Err(e) => return Err(format!("cannot accept the phone's writes: {e}")),
                    }
                }
                Some(_) => {}
                None => break,
            },
            event = tx_control.next() => match event {
                Some(CharacteristicControlEvent::Notify(w)) => {
                    // a new subscription replaces whatever was there
                    let cap = w.mtu().saturating_sub(3).max(BLE_CHUNK_MIN);
                    central.address = Some(w.device_address());
                    central.writer = Some(w);
                    let _ = up.send(Up::Connected(cap));
                }
                Some(_) => {}
                None => break,
            },
            received = async {
                match &central.reader {
                    Some(r) => r.recv().await,
                    None => pending().await,
                }
            } => match received {
                Ok(m) if !m.is_empty() => {
                    let _ = up.send(Up::Message(m));
                }
                _ => {
                    central = Central::default();
                    let _ = up.send(Up::Disconnected);
                }
            },
            _ = async {
                match &central.writer {
                    Some(w) => { let _ = w.closed().await; }
                    None => pending().await,
                }
            } => {
                central = Central::default();
                let _ = up.send(Up::Disconnected);
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod vhci_tests;
