//! Driver of the Bluetooth frame channel (contract C20): runs one [`Link`] per connected central over
//! a [`Transport`], moves frames between the link and the unlock/enrollment code (`Emitter` events in,
//! [`BleCmd`] frames out) and owns the advertising window. The transports are BlueZ over D-Bus
//! (`ble_bluez.rs`), CoreBluetooth (`ble_mac.rs`), and a fake in the tests; none of them is trusted
//! beyond "bytes in, bytes out": all checking is in the link and, above it, in the frames themselves.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::ble_link::{local_name, Event, Link, B_DONE, B_USER};
use super::{BleCmd, Emitter};

#[cfg(test)]
mod tests;

/// What a transport reports, one event per [`Transport::poll`].
pub(super) enum Poll {
    Idle,
    /// A central connected and is ready: it subscribed to notifications. `notify_cap` is the largest
    /// message a notification can carry (ATT MTU minus 3).
    Connected {
        notify_cap: usize,
    },
    /// One GATT write from the central.
    Message(Vec<u8>),
    /// The central went away.
    Disconnected,
    /// The radio cannot be used (no adapter, daemon not running, permission): the channel ends.
    Failed(String),
}

pub(super) trait Transport {
    /// Waits up to `wait` for the next event.
    fn poll(&mut self, wait: Duration) -> Poll;
    /// One notification to the central; false if there is no central any more.
    fn send(&mut self, msg: &[u8]) -> bool;
    fn advertise(&mut self, on: bool);
    /// Drop the connected central, if any. Does not produce a `Disconnected` poll.
    fn disconnect(&mut self);
}

/// Send what the link has queued; false when the transport lost the central.
fn flush(link: &mut Link, t: &mut impl Transport) -> bool {
    link.take_outbox().iter().all(|m| t.send(m))
}

/// Serves the channel until the caller stops it, a reply ends it, the radio fails, or the advertising
/// window (`window`, `BLE_ADVERTISE_SECS` in production) passes without a central.
pub(super) fn serve(
    stop: &AtomicBool,
    em: &Emitter,
    tag: &str,
    cmds: &Receiver<BleCmd>,
    t: &mut impl Transport,
    window: Duration,
) {
    let started = Instant::now();
    let name = local_name(tag);
    let mut link: Option<Link> = None;
    let mut current: Option<Vec<u8>> = None;
    t.advertise(true);
    em.status(format!("Bluetooth: pick {name} in the phone's list"));
    loop {
        let now = Instant::now();
        if stop.load(Ordering::Relaxed) {
            if let Some(l) = link.as_mut() {
                l.bye(now, B_USER);
                flush(l, t);
            }
            t.disconnect();
            return;
        }
        let mut gone = false;
        let mut done = false;
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                BleCmd::Frame(f) => {
                    current = Some(f.clone());
                    if let Some(l) = link.as_mut() {
                        if let Err(e) = l.send_frame(now, f) {
                            em.status(format!("Bluetooth: cannot send: {e}"));
                        }
                    }
                }
                BleCmd::DropCentral => {
                    if let Some(l) = link.as_mut() {
                        l.bye(now, B_USER);
                        flush(l, t);
                        gone = true;
                        em.status("Bluetooth: phone dropped");
                    }
                }
            }
        }
        match t.poll(Duration::from_millis(50)) {
            Poll::Idle => {}
            Poll::Failed(why) => {
                em.status(format!("Bluetooth unavailable: {why}"));
                return;
            }
            Poll::Connected { notify_cap } => {
                t.advertise(false);
                let mut l = Link::terminal(now, notify_cap);
                if let Some(f) = &current {
                    // queued; sent right after the handshake
                    let _ = l.send_frame(now, f.clone());
                }
                link = Some(l);
                em.status("Bluetooth: phone connected, handshaking");
            }
            Poll::Message(m) => {
                if let Some(l) = link.as_mut() {
                    l.on_message(now, &m);
                }
            }
            Poll::Disconnected => gone = true,
        }
        if let Some(l) = link.as_mut() {
            l.tick(now);
            if !flush(l, t) {
                gone = true;
            }
            for ev in l.take_events() {
                match ev {
                    Event::Opened { chunk } => {
                        em.status(format!(
                            "Bluetooth: phone connected (messages of {chunk} bytes)"
                        ));
                    }
                    Event::Frame(f) => done |= em.reply(f),
                    Event::Sent => {
                        em.sent();
                    }
                    Event::SendFailed(r) => {
                        em.status(format!(
                            "Bluetooth: the phone did not take the frame (reason {r})"
                        ));
                    }
                    Event::Closed(_) => gone = true,
                }
            }
            if done {
                l.bye(now, B_DONE);
                flush(l, t);
            }
        }
        if done {
            t.disconnect();
            return;
        }
        if gone {
            if link.take().is_some() {
                t.disconnect();
            }
            if now.duration_since(started) < window {
                t.advertise(true);
                em.status(format!(
                    "Bluetooth: no phone connected, still advertising as {name}"
                ));
            }
        }
        if link.is_none() && now.duration_since(started) >= window {
            t.advertise(false);
            em.status(
                "Bluetooth: advertising ended after 5 minutes; reopen this dialog to try again",
            );
            return;
        }
    }
}

/// The advertising window of a production channel.
#[cfg(feature = "bluetooth")]
pub(super) const WINDOW: Duration = Duration::from_secs(super::ble_link::BLE_ADVERTISE_SECS);

#[cfg(feature = "bluetooth")]
pub(super) fn run(stop: &AtomicBool, em: &Emitter, tag: &str, cmds: Receiver<BleCmd>) {
    #[cfg(target_os = "linux")]
    let mut transport = super::ble_bluez::BluezTransport::start(tag);
    #[cfg(target_os = "macos")]
    let mut transport = super::ble_mac::MacTransport::start(tag);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    serve(stop, em, tag, &cmds, &mut transport, WINDOW);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (stop, tag, cmds);
        em.status("Bluetooth is not implemented on this OS");
    }
}

/// An adapter exists (and, on macOS, access is not refused).
#[cfg(feature = "bluetooth")]
pub(super) fn present() -> bool {
    #[cfg(target_os = "linux")]
    return super::ble_bluez::present();
    #[cfg(target_os = "macos")]
    return super::ble_mac::present();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return false;
}

#[cfg(feature = "bluetooth")]
pub(super) fn devices() -> Vec<String> {
    #[cfg(target_os = "linux")]
    return super::ble_bluez::devices();
    #[cfg(target_os = "macos")]
    return super::ble_mac::devices();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Vec::new();
}
