//! Hardware check for the reply receivers (microphone FSK, webcam reverse QR) of the unlock
//! modal. Run it on the machine and play or show the phone's reply:
//!
//! ```text
//! cargo run --example media-probe --features audio,camera -- [seconds]
//! ```
//!
//! Lists what this build and machine can listen with, listens for `seconds` (default 60) and
//! prints every event; exits at the first complete reply. Nothing is recorded or saved.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sshub::sync::media::{self, MediaEvent, MediaKind};
use sshub::sync::wire::ShareResponseWire;

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60);
    let support = media::available();
    println!(
        "audio available: {}   camera available: {}",
        support.audio, support.camera
    );
    for d in media::devices() {
        println!("  {d}");
    }
    let mut kinds = Vec::new();
    if support.audio {
        kinds.push(MediaKind::Audio);
    }
    if support.camera {
        kinds.push(MediaKind::Camera);
    }
    if kinds.is_empty() {
        println!(
            "nothing to listen with (build with --features audio,camera and plug a device in)"
        );
        return;
    }
    let (tx, rx) = mpsc::channel();
    let handle = media::listen(&kinds, tx);
    let start = Instant::now();
    println!("listening for {secs} s: play the audio reply or show the reverse QR");
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
