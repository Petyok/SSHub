//! Terminal-side receivers of the phone's share reply (design: "Share release", response
//! channels): the microphone hears the audio burst, the camera reads the phone's looping reverse
//! QR. One API for the unlock modal:
//!
//! ```ignore
//! let support = media::available();              // compile-time feature AND a device present
//! let (tx, rx) = std::sync::mpsc::channel();
//! let handle = media::listen(&[MediaKind::Audio, MediaKind::Camera], tx);
//! // poll rx: Status / Progress / Reply(kind, ShareResponseWire bytes) for unlock_paths::open_phone_reply
//! drop(handle);                                  // stops capture, releases the devices
//! ```
//!
//! [`listen_with`] takes [`ListenOpts`] to hand over other documents (phone enrollment frames,
//! several in a row) instead of the single `ShareResponseWire`.
//!
//! Capture runs only while the [`ListenHandle`] lives. Nothing is recorded to disk; sample and
//! frame buffers are dropped when a channel stops. A channel stops itself after its first valid
//! reply (the device is released at once); the other channel keeps listening until the handle
//! is dropped.
//!
//! `fsk` and `qrscan` are the decoders: always compiled, tested in CI. `mic` (feature `audio`,
//! `cpal`) and `cam` (feature `camera`, `nokhwa`) only move samples and frames into them.

pub mod fsk;
pub mod qrscan;

#[cfg(feature = "camera")]
mod cam;
#[cfg(feature = "audio")]
mod mic;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::sync::wire::{EnrollFrame, ShareResponseWire};

/// A response channel the terminal can listen on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// Microphone, FSK burst.
    Audio,
    /// Webcam, the phone's looping reverse QR.
    Camera,
}

/// What this build and this machine can listen with right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MediaSupport {
    pub audio: bool,
    pub camera: bool,
}

/// What a running listener tells the modal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaEvent {
    /// One line for the channel's row in the modal ("listening", "device busy", ...).
    Status(MediaKind, String),
    /// Reverse QR only: blocks held and needed of the reply.
    Progress(MediaKind, usize, usize),
    /// The complete bytes of a `ShareResponseWire` (a valid frame, request id not yet checked):
    /// hand them to `unlock_paths::open_phone_reply`.
    Reply(MediaKind, Vec<u8>),
}

/// Compile-time features and run-time devices. May open the capture subsystem briefly (device
/// enumeration), so call it when the modal opens, not per frame.
pub fn available() -> MediaSupport {
    MediaSupport {
        audio: audio_present(),
        camera: camera_present(),
    }
}

#[cfg(feature = "audio")]
fn audio_present() -> bool {
    mic::present()
}
#[cfg(not(feature = "audio"))]
fn audio_present() -> bool {
    false
}
#[cfg(feature = "camera")]
fn camera_present() -> bool {
    cam::present()
}
#[cfg(not(feature = "camera"))]
fn camera_present() -> bool {
    false
}

/// Human-readable lines for the devices behind [`available`], `audio: ...` and `camera: ...`
/// (for diagnostics such as `examples/media-probe.rs`).
pub fn devices() -> Vec<String> {
    #[allow(unused_mut)]
    let mut out: Vec<String> = Vec::new();
    #[cfg(feature = "audio")]
    out.extend(mic::devices().into_iter().map(|d| format!("audio: {d}")));
    #[cfg(feature = "camera")]
    out.extend(cam::devices().into_iter().map(|d| format!("camera: {d}")));
    out
}

/// Running capture. Dropping it stops the threads and releases the devices (blocks for at most
/// a fraction of a second while they wind down).
pub struct ListenHandle {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for ListenHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Which documents a listener hands over and whether it stops after the first.
#[derive(Clone, Copy)]
pub struct ListenOpts {
    /// Called with every complete document (an audio frame payload or a reassembled QR
    /// document); only documents it accepts become `Reply` events.
    pub accept: fn(&[u8]) -> bool,
    /// False: a channel ends after its first accepted reply. True: it keeps listening until
    /// the handle is dropped; a document identical to the last one it handed over is not sent
    /// again (the phone loops the same document until the next step).
    pub keep_listening: bool,
}

/// `bytes` is a `ShareResponseWire` (the unlock reply).
pub fn accept_share_response(bytes: &[u8]) -> bool {
    ShareResponseWire::decode(bytes).is_ok()
}

/// `bytes` is an enrollment frame (`EnrollFrame` kinds 6, 7, 8).
pub fn accept_enroll_frame(bytes: &[u8]) -> bool {
    EnrollFrame::decode(bytes).is_ok()
}

impl ListenOpts {
    /// What [`listen`] uses: the first valid `ShareResponseWire`, then stop.
    pub const SHARE_RESPONSE: ListenOpts = ListenOpts {
        accept: accept_share_response,
        keep_listening: false,
    };

    /// Enrollment frames, listening until the handle is dropped.
    pub const ENROLL_FRAMES: ListenOpts = ListenOpts {
        accept: accept_enroll_frame,
        keep_listening: true,
    };
}

/// Reports one channel's progress; validates what it is given.
pub(crate) struct Emitter {
    kind: MediaKind,
    tx: Sender<MediaEvent>,
    opts: ListenOpts,
    last: RefCell<Option<Vec<u8>>>,
}

impl Emitter {
    pub(crate) fn new(kind: MediaKind, tx: Sender<MediaEvent>, opts: ListenOpts) -> Self {
        Emitter {
            kind,
            tx,
            opts,
            last: RefCell::new(None),
        }
    }

    /// False once the modal stopped reading: the capture thread should end.
    pub(crate) fn status(&self, text: impl Into<String>) -> bool {
        self.tx
            .send(MediaEvent::Status(self.kind, text.into()))
            .is_ok()
    }

    #[cfg_attr(not(feature = "camera"), allow(dead_code))]
    pub(crate) fn progress(&self, held: usize, needed: usize) -> bool {
        self.tx
            .send(MediaEvent::Progress(self.kind, held, needed))
            .is_ok()
    }

    /// Hands `bytes` over if `accept` takes them. True when the channel should stop: the
    /// reply was sent and the caller wants no more, or nobody listens any longer.
    #[cfg_attr(not(any(feature = "audio", feature = "camera")), allow(dead_code))]
    pub(crate) fn reply(&self, bytes: Vec<u8>) -> bool {
        if !(self.opts.accept)(&bytes) {
            return !self.status("received something this step does not expect, still listening");
        }
        if self.opts.keep_listening && self.last.borrow().as_deref() == Some(&bytes[..]) {
            return false;
        }
        if self.opts.keep_listening {
            *self.last.borrow_mut() = Some(bytes.clone());
        }
        let sent = self.tx.send(MediaEvent::Reply(self.kind, bytes)).is_ok();
        !sent || !self.opts.keep_listening
    }
}

/// Starts capture for `kinds` (each at most once). A kind this build or machine cannot serve
/// reports why through a `Status` event and starts nothing.
pub fn listen(kinds: &[MediaKind], tx: Sender<MediaEvent>) -> ListenHandle {
    listen_with(kinds, tx, ListenOpts::SHARE_RESPONSE)
}

/// [`listen`] with the documents to hand over chosen by the caller (e.g.
/// [`ListenOpts::ENROLL_FRAMES`] for the phone-enrollment flow).
pub fn listen_with(kinds: &[MediaKind], tx: Sender<MediaEvent>, opts: ListenOpts) -> ListenHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        if kinds[..i].contains(kind) {
            continue;
        }
        let em = Emitter::new(*kind, tx.clone(), opts);
        let stop = stop.clone();
        let started = std::thread::Builder::new()
            .name(format!("sshub-media-{kind:?}"))
            .spawn(move || match em.kind {
                MediaKind::Audio => run_audio(&stop, &em),
                MediaKind::Camera => run_camera(&stop, &em),
            });
        match started {
            Ok(t) => threads.push(t),
            Err(e) => {
                let _ = tx.send(MediaEvent::Status(
                    *kind,
                    format!("cannot start capture: {e}"),
                ));
            }
        }
    }
    ListenHandle { stop, threads }
}

#[cfg(feature = "audio")]
fn run_audio(stop: &AtomicBool, em: &Emitter) {
    mic::run(stop, em);
}
#[cfg(not(feature = "audio"))]
fn run_audio(_stop: &AtomicBool, em: &Emitter) {
    em.status("built without the `audio` feature");
}
#[cfg(feature = "camera")]
fn run_camera(stop: &AtomicBool, em: &Emitter) {
    cam::run(stop, em);
}
#[cfg(not(feature = "camera"))]
fn run_camera(_stop: &AtomicBool, em: &Emitter) {
    em.status("built without the `camera` feature");
}
