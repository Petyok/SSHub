//! The phone from the terminal: enrolling it (the commit, reveal, words and
//! confirm exchange of `sync::phone_enroll`, each frame a looping QR or a pasted
//! base64 frame) and taking in the ops it copies out (kind 5). Every step that
//! needs the device key runs as a job on the sync worker and leaves its result
//! in a shared outbox, which the tick turns into the next modal.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use super::sync_trust::qr_frames;
use super::*;
use crate::sync::media::{listen_with, ListenHandle, ListenOpts, MediaEvent, MediaKind};
use crate::sync::phone_enroll::{admit_phone, PhoneEnroll, Step};

const TITLE: &str = "Add my phone";

/// What the next modal shows.
#[derive(Clone)]
pub(crate) enum FlowOut {
    /// A frame for the phone to scan; the field takes the phone's next frame.
    Show(Vec<u8>),
    /// The four words, to compare with the phone's.
    Compare {
        words: String,
        phone_name: String,
    },
    /// Nothing to show: the phone's next frame is expected.
    Paste,
    /// The phone is registered and proposed; the terminal's confirmation for the phone.
    Admitted {
        frame: Vec<u8>,
        lines: Vec<String>,
    },
    Stopped {
        why: String,
        frame: Option<Vec<u8>>,
    },
    Error(String),
}

/// An enrollment in progress: the state machine and what its jobs produced.
pub(crate) struct PhoneFlow {
    enroll: Arc<Mutex<Option<PhoneEnroll>>>,
    out: Arc<Mutex<Vec<FlowOut>>>,
    /// The last prompt, shown again after a refused frame.
    last: Option<FlowOut>,
    /// Jobs started and not yet finished: the flow lives while any runs or a modal is open.
    running: Arc<AtomicUsize>,
    /// The webcam listens for the phone's frames while the flow is open (a build with
    /// the `camera` feature and a device); dropping it releases the device.
    camera: Option<(ListenHandle, Receiver<MediaEvent>)>,
    /// Frames the camera produced that no step has taken yet.
    inbox: Vec<Vec<u8>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn is_yes(text: &str) -> bool {
    matches!(text.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Menu entry: start enrolling the phone.
pub(crate) fn start(app: &mut App) {
    if matches!(crate::sync::quorum::phone_device(&app.store), Ok(Some(_))) {
        app.host_notice =
            Some("the ring already has a phone: replacing it is a recovery (M, Recovery)".into());
        return;
    }
    let enroll = Arc::new(Mutex::new(None));
    let out = Arc::new(Mutex::new(Vec::new()));
    let running = Arc::new(AtomicUsize::new(1));
    let (tx, rx) = std::sync::mpsc::channel();
    let mut kinds = vec![MediaKind::Camera];
    if cfg!(feature = "bluetooth") {
        kinds.push(MediaKind::Ble);
    }
    let handle = listen_with(&kinds, tx, ListenOpts::ENROLL_FRAMES);
    app.phone_flow = Some(PhoneFlow {
        enroll: enroll.clone(),
        out: out.clone(),
        last: None,
        running: running.clone(),
        camera: Some((handle, rx)),
        inbox: Vec::new(),
    });
    app.run_sync_job(move |env| {
        let (e, first) = PhoneEnroll::begin(&env.identity, &crate::sync::runtime::device_name());
        *lock(&enroll) = Some(e);
        lock(&out).push(FlowOut::Show(first));
        running.fetch_sub(1, Ordering::SeqCst);
        Ok(String::new())
    });
}

/// A step's result, as the modal to show; `None` for a verified phone (the caller admits it).
fn present_step(step: Step) -> Option<FlowOut> {
    Some(match step {
        Step::Show(f) => FlowOut::Show(f),
        Step::Compare { words, phone_name } => FlowOut::Compare { words, phone_name },
        Step::Wait => FlowOut::Paste,
        Step::Stopped { why, frame } => FlowOut::Stopped { why, frame },
        Step::Verified(_) => return None,
    })
}

/// Run one state machine call on the worker and queue what it says next.
fn step_job(
    app: &mut App,
    call: impl FnOnce(&mut PhoneEnroll, &crate::sync::crypto::DeviceIdentity) -> Result<Step>
        + Send
        + 'static,
) {
    let Some(flow) = app.phone_flow.as_ref() else {
        return;
    };
    let (enroll, out) = (flow.enroll.clone(), flow.out.clone());
    let running = flow.running.clone();
    running.fetch_add(1, Ordering::SeqCst);
    app.run_sync_job(move |env| {
        let mut slot = lock(&enroll);
        let Some(e) = slot.as_mut() else {
            lock(&out).push(FlowOut::Error("the enrollment has not started".into()));
            running.fetch_sub(1, Ordering::SeqCst);
            return Ok(String::new());
        };
        let next = match call(e, &env.identity) {
            Ok(Step::Verified(v)) => admit_phone(&env.store, &env.identity, &v).and_then(|op| {
                let report = crate::cli::sync_admin::op_report(
                    &env.store,
                    &op,
                    format!("{} is enrolled; its admission is proposed", v.name),
                    crate::store::now_ts(),
                )?;
                Ok(FlowOut::Admitted {
                    frame: v.frame,
                    lines: report.lines,
                })
            }),
            Ok(step) => Ok(present_step(step).expect("verified handled above")),
            Err(e) => Err(e),
        };
        lock(&out).push(next.unwrap_or_else(|e| FlowOut::Error(format!("{e:#}"))));
        running.fetch_sub(1, Ordering::SeqCst);
        Ok(String::new())
    });
}

/// The phone's frame as pasted text (base64, as the page copies it).
fn feed_text(app: &mut App, text: String) {
    match B64.decode(text.split_whitespace().collect::<String>()) {
        Ok(bytes) => feed_bytes(app, bytes),
        Err(_) => reshow(
            app,
            "that is not base64: paste the frame as the phone shows it",
        ),
    }
}

/// A frame of the phone from any source (a paste, the camera).
pub(crate) fn feed_bytes(app: &mut App, bytes: Vec<u8>) {
    step_job(app, move |e, id| e.scan(id, &bytes));
}

fn answer(app: &mut App, ok: bool) {
    step_job(app, move |e, id| e.answer(id, ok));
}

fn reshow(app: &mut App, why: &str) {
    app.host_notice = Some(why.to_string());
    if let Some(last) = app.phone_flow.as_ref().and_then(|f| f.last.clone()) {
        present(app, last);
    }
}

/// Per tick: turn what the jobs produced into the next modal.
pub(crate) fn tick(app: &mut App) {
    let Some(flow) = app.phone_flow.as_mut() else {
        return;
    };
    let outs: Vec<FlowOut> = std::mem::take(&mut *lock(&flow.out));
    if let Some((_, rx)) = flow.camera.as_ref() {
        flow.inbox.extend(rx.try_iter().filter_map(|e| match e {
            MediaEvent::Reply(_, bytes) => Some(bytes),
            _ => None,
        }));
    }
    let idle = flow.running.load(Ordering::SeqCst) == 0;
    // Closed with Esc and nothing in flight: release the camera.
    if outs.is_empty() && idle && app.sync_ui.is_none() {
        app.phone_flow = None;
        return;
    }
    // A frame of the camera counts only while the phone's next frame is expected, not while the
    // words are compared; frames of an earlier step are dropped when the next modal opens.
    let expecting =
        idle && outs.is_empty() && matches!(flow.last, Some(FlowOut::Show(_) | FlowOut::Paste));
    let camera_frame = (expecting && !flow.inbox.is_empty()).then(|| flow.inbox.remove(0));
    for o in outs {
        match o {
            FlowOut::Error(e) => reshow(app, &e),
            o => present(app, o),
        }
    }
    if let Some(bytes) = camera_frame {
        feed_bytes(app, bytes);
    }
}

fn present(app: &mut App, out: FlowOut) {
    if let Some(f) = app.phone_flow.as_mut() {
        f.inbox.clear();
    }
    if matches!(
        out,
        FlowOut::Show(_) | FlowOut::Compare { .. } | FlowOut::Paste
    ) {
        if let Some(f) = app.phone_flow.as_mut() {
            f.last = Some(out.clone());
        }
    }
    // Bluetooth (feature `bluetooth`): the frame on screen also goes to a connected phone, and the
    // session tag tells the person which entry of the phone's device list is this terminal.
    let ble_line = app
        .phone_flow
        .as_ref()
        .and_then(|f| f.camera.as_ref())
        .and_then(|(h, _)| {
            if let FlowOut::Show(f)
            | FlowOut::Admitted { frame: f, .. }
            | FlowOut::Stopped { frame: Some(f), .. } = &out
            {
                h.send_frame(f.clone());
            }
            h.ble_tag().map(|t| {
                format!("Bluetooth: choose SSHub-{t} in the phone page's device list to use it instead.")
            })
        });
    match out {
        FlowOut::Show(frame) => {
            let mut lines = vec![
                "Show this code to the phone's camera. The terminal's webcam reads the phone's frames too when this build has the `camera` feature.".into(),
                "Then paste the frame the phone shows next (copy it on the phone).".into(),
                format!(
                    "Companion page of this build: {} ({}).",
                    crate::sync::companion::COMPANION_FINGERPRINT,
                    crate::sync::companion::COMPARE_HINT
                ),
            ];
            lines.extend(ble_line);
            qr_prompt(app, &frame, lines, "Phone's next frame (base64)", feed_text);
        }
        FlowOut::Paste => qr_prompt(
            app,
            &[],
            vec!["The phone confirms once its words matched too.".into()],
            "Phone's confirmation (base64)",
            feed_text,
        ),
        FlowOut::Compare { words, phone_name } => qr_prompt(
            app,
            &[],
            vec![
                format!("Words: {words}"),
                format!(
                    "The phone calls itself {phone_name}. The phone must show the same four words."
                ),
                "Different words mean someone sits between you: answer no.".into(),
            ],
            "Do the words match? (yes/no)",
            |app, text| {
                let ok = is_yes(&text);
                answer(app, ok);
            },
        ),
        FlowOut::Admitted { frame, mut lines } => {
            lines.push("Show this code to the phone so it can finish.".into());
            lines.extend(ble_line);
            qr_prompt(
                app,
                &frame,
                lines,
                "Enter when the phone is done",
                |app, _| {
                    app.phone_flow = None;
                    app.refresh_sync_view();
                },
            );
        }
        FlowOut::Stopped { why, frame } => {
            app.phone_flow = None;
            match frame {
                Some(f) => qr_prompt(
                    app,
                    &f,
                    vec![why, "Show this code to the phone so it stops too.".into()],
                    "Enter to close",
                    |_, _| {},
                ),
                None => app.sync_info(TITLE, vec![why]),
            }
        }
        FlowOut::Error(e) => app.host_notice = Some(e),
    }
}

fn qr_prompt(
    app: &mut App,
    frame: &[u8],
    lines: Vec<String>,
    label: &str,
    on_submit: impl FnOnce(&mut App, String) + 'static,
) {
    let frames = if frame.is_empty() {
        Vec::new()
    } else {
        match qr_frames(frame) {
            Ok(f) => f,
            Err(e) => {
                app.host_notice = Some(format!("{e:#}"));
                return;
            }
        }
    };
    app.sync_prompt_qr(TITLE, lines, frames, label, on_submit);
}

/// Menu entry: the ops list the phone copied (kind 5), through the courier ingest.
pub(crate) fn paste_ops(app: &mut App) {
    app.sync_prompt(
        "Paste phone ops",
        "The phone's ops (base64, as the page copies them)",
        false,
        |app, text| {
            app.run_sync_job(move |env| {
                let s = crate::sync::unlock_paths::phone_ops_from_text(&env.store, &text)?;
                Ok(format!(
                    "phone ops taken in: {} received, {} applied, {} waiting for approval",
                    s.received, s.applied, s.pending_arrived
                ))
            });
        },
    );
}

/// Tests stand in for the webcam: frames sent on `rx`' sender arrive as the camera's.
#[cfg(test)]
pub(crate) fn use_camera_channel(app: &mut App, handle: ListenHandle, rx: Receiver<MediaEvent>) {
    if let Some(f) = app.phone_flow.as_mut() {
        f.camera = Some((handle, rx));
    }
}
