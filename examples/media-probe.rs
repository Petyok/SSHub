//! Hardware check for the reply receivers of the unlock modal: microphone FSK, webcam reverse QR
//! and, with `--ble`, the Bluetooth channel. Run it on the machine and play, show or connect the
//! phone's reply:
//!
//! ```text
//! cargo run --example media-probe --features audio,camera -- [seconds]
//! cargo run --example media-probe --features bluetooth -- [seconds] --ble
//! ```
//!
//! Without `--ble`: lists what this build and machine can listen with, listens on the microphone
//! and the camera for `seconds` (default 60) and prints every event; exits at the first complete
//! `ShareResponseWire`.
//!
//! With `--ble` (contract C20): advertises as `SSHub-<tag>`, sends a synthetic 3000 byte frame
//! (kind 1) to the first phone page that connects, prints every frame the page sends back (length
//! and CRC-32; any frame of kinds 1 to 10 is accepted) and the delivery notice, until `seconds` are
//! up. Nothing is recorded or saved; no ring and no keys are involved.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sshub::sync::media::ble_link::{crc32, valid_frame};
use sshub::sync::media::{self, ListenOpts, MediaEvent, MediaKind};
use sshub::sync::wire::ShareResponseWire;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(60);
    let ble = args.iter().any(|a| a == "--ble");
    let support = media::available();
    println!(
        "audio available: {}   camera available: {}   bluetooth available: {}",
        support.audio, support.camera, support.ble
    );
    for d in media::devices() {
        println!("  {d}");
    }
    let mut kinds = Vec::new();
    if ble {
        kinds.push(MediaKind::Ble);
    } else {
        if support.audio {
            kinds.push(MediaKind::Audio);
        }
        if support.camera {
            kinds.push(MediaKind::Camera);
        }
    }
    if kinds.is_empty() {
        println!(
            "nothing to listen with (build with --features audio,camera or bluetooth and plug a device in)"
        );
        return;
    }
    let (tx, rx) = mpsc::channel();
    let opts = if ble {
        ListenOpts {
            accept: valid_frame,
            keep_listening: true,
        }
    } else {
        ListenOpts::SHARE_RESPONSE
    };
    let handle = media::listen_with(&kinds, tx, opts);
    if ble {
        let Some(tag) = handle.ble_tag() else {
            println!("this build has no Bluetooth channel (build with --features bluetooth)");
            return;
        };
        println!("advertising as SSHub-{tag}: pick it in the phone page's device list");
        let mut frame = b"SWR1".to_vec();
        frame.extend_from_slice(&[0, 1, 1]);
        frame.extend((0..3000u32).map(|i| (i * 31 + 7) as u8));
        println!(
            "test frame to send: {} bytes, crc32 {:08x}",
            frame.len(),
            crc32(&frame)
        );
        handle.send_frame(frame);
    } else {
        println!("listening for {secs} s: play the audio reply or show the reverse QR");
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        let t = start.elapsed().as_secs_f32();
        match ev {
            MediaEvent::Status(k, s) => println!("[{t:6.1}s] {k:?}: {s}"),
            MediaEvent::Progress(k, held, needed) => {
                println!("[{t:6.1}s] {k:?}: {held}/{needed} blocks")
            }
            MediaEvent::Sent(k) => println!("[{t:6.1}s] {k:?}: the test frame reached the phone"),
            MediaEvent::Reply(k, bytes) if ble => println!(
                "[{t:6.1}s] {k:?}: FRAME {} bytes, kind {}, crc32 {:08x}",
                bytes.len(),
                bytes[6],
                crc32(&bytes)
            ),
            MediaEvent::Reply(k, bytes) => {
                let id = ShareResponseWire::decode(&bytes).map(|w| w.request_id);
                println!(
                    "[{t:6.1}s] {k:?}: REPLY {} bytes, request id {id:?}",
                    bytes.len()
                );
                break;
            }
        }
    }
    drop(handle);
    println!("capture stopped, devices released");
}
