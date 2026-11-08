//! `--ble` for the phone verbs (`sync_phone_cmd.rs`): the terminal advertises over Bluetooth as
//! `SSHub-<tag>` for the length of the verb (feature `bluetooth`, contract C20), the frames the verb
//! prints also go to the phone page that connects, and the frames the phone sends back arrive as if
//! they had been typed. Stdin keeps working next to it, so a verb can finish by either route.

use std::io::{BufRead, Write};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::json;

use super::sync_phone_cmd::{b64, emit, EnrollSession};
use crate::store::LauncherStore;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::media::{self, ListenHandle, ListenOpts, MediaEvent, MediaKind};

const POLL: Duration = Duration::from_millis(100);

/// The Bluetooth side of one verb. Dropping it stops advertising and drops the phone.
pub struct BleLink {
    handle: ListenHandle,
    events: Receiver<MediaEvent>,
    tag: String,
    json: bool,
}

/// What arrived since the last [`BleLink::pump`].
pub struct Pumped {
    /// Whole frames from the phone that the verb's `accept` took.
    pub frames: Vec<Vec<u8>>,
    /// The frame sent with [`BleLink::send`] reached the phone.
    pub sent: bool,
}

impl BleLink {
    /// Starts advertising. An error when this build has no Bluetooth channel.
    pub fn new(json: bool, opts: ListenOpts) -> Result<BleLink> {
        let (tx, events) = channel();
        let handle = media::listen_with(&[MediaKind::Ble], tx, opts);
        let tag = handle
            .ble_tag()
            .map(str::to_string)
            .context("this build has no Bluetooth channel: build with `--features bluetooth`")?;
        Ok(BleLink {
            handle,
            events,
            tag,
            json,
        })
    }

    /// Prints the name to pick in the phone app's device list.
    pub fn announce(&self, out: &mut impl Write) {
        let name = media::ble_link::local_name(&self.tag);
        emit(
            out,
            self.json,
            "bluetooth",
            format!("pick {name} in the phone app's device list"),
            json!({"name": name, "tag": self.tag}),
        );
    }

    /// Queues a frame for the phone (sent when it is connected, again if a newer one is queued).
    pub fn send(&self, frame: &[u8]) {
        self.handle.send_frame(frame.to_vec());
    }

    /// Drains the channel's events: statuses are printed, frames and the delivery notice returned.
    pub fn pump(&self, out: &mut impl Write) -> Pumped {
        let mut p = Pumped {
            frames: Vec::new(),
            sent: false,
        };
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                MediaEvent::Reply(_, bytes) => p.frames.push(bytes),
                MediaEvent::Status(_, s) => {
                    emit(out, self.json, "bluetooth", s.clone(), json!({"status": s}));
                }
                MediaEvent::Sent(_) => {
                    emit(
                        out,
                        self.json,
                        "sent",
                        "the phone has the frame".into(),
                        json!({}),
                    );
                    p.sent = true;
                }
                MediaEvent::Progress(..) => {}
            }
        }
        p
    }
}

/// Lines of stdin as they come; the channel closes at end of input.
fn stdin_lines() -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// `sync-phone-enroll --ble`: the enrollment session with the phone's frames from Bluetooth or
/// stdin and the terminal's frames printed and sent. The four words are still answered on stdin.
pub fn enroll(
    store: &LauncherStore,
    id: &DeviceIdentity,
    out: &mut impl Write,
    json: bool,
) -> Result<i32> {
    let ble = BleLink::new(json, ListenOpts::ENROLL_FRAMES)?;
    let mut s =
        EnrollSession::new(store, id, &crate::sync::runtime::device_name(), json).with_ble(&ble);
    s.start(out);
    ble.announce(out);
    let stdin = stdin_lines();
    loop {
        for frame in ble.pump(out).frames {
            s.feed(&b64(&frame), out);
            if let Some(ok) = s.done {
                return Ok(i32::from(!ok));
            }
        }
        match stdin.recv_timeout(POLL) {
            Ok(line) => {
                s.feed(&line, out);
                if let Some(ok) = s.done {
                    return Ok(i32::from(!ok));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                bail!("the input ended before the phone was verified")
            }
        }
    }
}

/// The phone's answer to a challenge: a frame over Bluetooth (as base64) or a line on stdin,
/// whichever comes first within `wait`.
pub fn wait_reply(ble: &BleLink, wait: Duration, out: &mut impl Write) -> Option<String> {
    let stdin = stdin_lines();
    let end = Instant::now() + wait;
    while Instant::now() < end {
        if let Some(frame) = ble.pump(out).frames.into_iter().next() {
            return Some(b64(&frame));
        }
        match stdin.recv_timeout(POLL) {
            Ok(line) if !line.trim().is_empty() => return Some(line),
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
        }
    }
    None
}

/// `sync-phone-ops --ble`: the phone's kind 5 frame over Bluetooth, as the base64 text the verb
/// takes in.
pub fn ops_text(json: bool, wait: u64, out: &mut impl Write) -> Result<String> {
    let ble = BleLink::new(
        json,
        ListenOpts {
            accept: |b| crate::sync::wire::PhoneOpsWire::decode(b).is_ok(),
            keep_listening: false,
        },
    )?;
    ble.announce(out);
    let end = Instant::now() + Duration::from_secs(wait);
    while Instant::now() < end {
        if let Some(frame) = ble.pump(out).frames.into_iter().next() {
            return Ok(b64(&frame));
        }
        std::thread::sleep(POLL);
    }
    bail!("no ops from the phone within {wait} s")
}

/// `sync-notice --ble` and `sync-cert-request --ble`: hand the frame to the phone page and wait
/// until it has it.
pub fn push(frame: &[u8], json: bool, wait: u64, out: &mut impl Write) -> Result<i32> {
    let ble = BleLink::new(
        json,
        ListenOpts {
            accept: |_| false,
            keep_listening: true,
        },
    )?;
    ble.announce(out);
    ble.send(frame);
    let end = Instant::now() + Duration::from_secs(wait);
    while Instant::now() < end {
        if ble.pump(out).sent {
            return Ok(0);
        }
        std::thread::sleep(POLL);
    }
    bail!("the phone did not take the frame within {wait} s")
}
