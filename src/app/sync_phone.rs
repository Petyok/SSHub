//! The phone from the terminal: enrolling it (the commit, reveal, words and
//! confirm exchange of `sync::phone_enroll`, each frame a looping QR or a pasted
//! base64 frame) and taking in the ops it copies out (kind 5). Every step that
//! needs the device key runs as a job on the sync worker and leaves its result
//! in a shared outbox, which the tick turns into the next modal.

use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use super::sync_trust::qr_frames;
use super::*;
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
    app.phone_flow = Some(PhoneFlow {
        enroll: enroll.clone(),
        out: out.clone(),
        last: None,
    });
    app.run_sync_job(move |env| {
        let (e, first) = PhoneEnroll::begin(&env.identity, &crate::sync::runtime::device_name());
        *lock(&enroll) = Some(e);
        lock(&out).push(FlowOut::Show(first));
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
    app.run_sync_job(move |env| {
        let mut slot = lock(&enroll);
        let Some(e) = slot.as_mut() else {
            lock(&out).push(FlowOut::Error("the enrollment has not started".into()));
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
    let Some(flow) = app.phone_flow.as_ref() else {
        return;
    };
    let outs: Vec<FlowOut> = std::mem::take(&mut *lock(&flow.out));
    for o in outs {
        match o {
            FlowOut::Error(e) => reshow(app, &e),
            o => present(app, o),
        }
    }
}

fn present(app: &mut App, out: FlowOut) {
    if matches!(
        out,
        FlowOut::Show(_) | FlowOut::Compare { .. } | FlowOut::Paste
    ) {
        if let Some(f) = app.phone_flow.as_mut() {
            f.last = Some(out.clone());
        }
    }
    match out {
        FlowOut::Show(frame) => qr_prompt(
            app,
            &frame,
            vec![
                "Show this code to the phone's camera.".into(),
                "Then paste the frame the phone shows next (copy it on the phone).".into(),
            ],
            "Phone's next frame (base64)",
            feed_text,
        ),
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
