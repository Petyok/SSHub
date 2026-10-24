//! Phone enrollment from the terminal through the real worker, against a phone
//! written the way the page does it.

use super::sync_paths::{decode_frames, pump, type_and_enter};
use super::*;
use crate::app::sync_ui::SyncUi;
use crate::credentials::{FilePasswordStore, PasswordStore, QuorumGatedStore};
use crate::profile::ProfilePaths;
use crate::sync::client::spawn_sync_thread;
use crate::sync::crypto::{sas_words, DeviceIdentity};
use crate::sync::enroll::{confirm_message, transcript_hash, Party, Reveal, INITIATOR, RESPONDER};
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::DeviceKind;
use crate::sync::protocol::tests::Dev;
use crate::sync::runtime::{guard_synced_credentials, round_fn, RoundDeps};
use crate::sync::wire::EnrollFrame;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::sync::Arc;

fn file_secrets(p: &ProfilePaths) -> Box<dyn PasswordStore> {
    Box::new(FilePasswordStore::new(p.credentials_file()))
}

/// This app as the only terminal of a ring with no phone yet.
fn app_without_phone() -> (App, Arc<DeviceIdentity>, tempfile::TempDir) {
    app_with_ring(true)
}

/// `forced` makes every Red change count as in force (no ceremony to run).
fn app_with_ring(forced: bool) -> (App, Arc<DeviceIdentity>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ProfilePaths {
        data_root: dir.path().to_path_buf(),
        id: "t".into(),
        name: "t".into(),
        root: dir.path().to_path_buf(),
        config_file: dir.path().join("config.toml"),
        ssh_config: dir.path().join("ssh_config"),
        compat: false,
    };
    let gated = QuorumGatedStore::new(file_secrets(&paths));
    let gate = gated.gate_handle();
    let mut app = test_app_with_store(vec![], Box::new(gated));
    guard_synced_credentials(&gate, app.store.clone());
    app.profile = Some(paths.clone());
    let me = Arc::new(load_or_init_identity(&paths, file_secrets(&paths).as_ref()).unwrap());
    crate::sync::ceremony::test_force(&app.store, forced);
    app.store
        .sync_register_device(
            &me.public(),
            "me",
            crate::sync::oplog::DeviceRole::Holder,
            crate::sync::oplog::Attendance::Attended,
            DeviceKind::Terminal,
            crate::sync::oplog::VerifiedBy::Sas,
        )
        .unwrap();
    // A second terminal, so the ring is a ring.
    let other = Dev::new();
    app.store
        .sync_register_device(
            &other.id.public(),
            "other",
            crate::sync::oplog::DeviceRole::Holder,
            crate::sync::oplog::Attendance::Attended,
            DeviceKind::Terminal,
            crate::sync::oplog::VerifiedBy::Sas,
        )
        .unwrap();
    let deps = RoundDeps {
        paths,
        store: app.store.clone(),
        gate,
        argon: crate::sync::crypto::Argon2Profile {
            mem_kib: 64,
            iters: 1,
        },
        open_secrets: file_secrets,
        ssh_program: None,
    };
    app.attach_sync_worker(spawn_sync_thread(round_fn(deps)));
    app.refresh_sync_view();
    (app, me, dir)
}

fn prompt(app: &App) -> Option<(String, Vec<String>, Vec<Vec<u8>>)> {
    match app.sync_ui.as_ref()? {
        SyncUi::Prompt {
            label,
            lines,
            frames,
            ..
        } => Some((label.clone(), lines.clone(), frames.clone())),
        _ => None,
    }
}

fn paste(app: &mut App, frame: &[u8]) {
    type_and_enter(app, &B64.encode(frame));
}

fn wait_label(app: &mut App, what: &str) -> (Vec<String>, Vec<Vec<u8>>) {
    pump(app, what, |a| {
        prompt(a).is_some_and(|(l, _, _)| l.contains(what))
    });
    let (_, lines, frames) = prompt(app).unwrap();
    (lines, frames)
}

#[test]
fn the_terminal_enrolls_a_phone_by_pasted_frames_and_proposes_it_as_a_red_holder() {
    let (mut app, me, _dir) = app_without_phone();
    let phone_id = DeviceIdentity::generate();
    let party = Party::new(&phone_id, "my phone", true);
    crate::app::sync_phone::start(&mut app);

    // 1. The terminal's commitment is on screen as a QR.
    let (_, frames) = wait_label(&mut app, "Phone's next frame");
    assert!(matches!(
        EnrollFrame::decode(&decode_frames(&frames)).unwrap(),
        EnrollFrame::Commit {
            role: RESPONDER,
            ..
        }
    ));
    // 2. The phone's commitment goes in; the terminal reveals.
    paste(
        &mut app,
        &EnrollFrame::Commit {
            role: INITIATOR,
            commit: party.commit(INITIATOR),
        }
        .encode(),
    );
    pump(&mut app, "the terminal's reveal", |a| {
        prompt(a).is_some_and(|(_, _, f)| {
            !f.is_empty()
                && matches!(
                    EnrollFrame::decode(&decode_frames(&f)),
                    Ok(EnrollFrame::Reveal { .. })
                )
        })
    });
    let (_, _, frames) = prompt(&app).unwrap();
    let EnrollFrame::Reveal { reveal, .. } = EnrollFrame::decode(&decode_frames(&frames)).unwrap()
    else {
        unreachable!()
    };
    let term_reveal = Reveal::from_bytes(&reveal).unwrap();
    // 3. The phone's reveal goes in; the words are shown and agree with the phone's.
    paste(
        &mut app,
        &EnrollFrame::Reveal {
            role: INITIATOR,
            reveal: party.reveal.bytes(),
        }
        .encode(),
    );
    let (lines, _) = wait_label(&mut app, "Do the words match");
    let th = transcript_hash(&party.reveal, &term_reveal);
    assert!(
        lines
            .iter()
            .any(|l| *l == format!("Words: {}", sas_words(&th))),
        "{lines:?}"
    );
    type_and_enter(&mut app, "yes");
    // 4. The phone confirms; the phone is registered and its admission proposed.
    wait_label(&mut app, "Phone's confirmation");
    paste(
        &mut app,
        &EnrollFrame::Confirm {
            role: INITIATOR,
            ok: true,
            sig: phone_id.sign(&confirm_message(INITIATOR, true, &th)),
        }
        .encode(),
    );
    let (lines, frames) = wait_label(&mut app, "Enter when the phone is done");
    assert!(lines.iter().any(|l| l.contains("is enrolled")), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.to_lowercase().contains("phone") || l.contains("proof")),
        "{lines:?}"
    );
    let EnrollFrame::Confirm { role, ok, .. } =
        EnrollFrame::decode(&decode_frames(&frames)).unwrap()
    else {
        panic!("the terminal's confirmation is shown to the phone")
    };
    assert!(ok && role == RESPONDER);
    assert_eq!(
        crate::sync::quorum::phone_device(&app.store).unwrap(),
        Some(phone_id.public().device_id())
    );
    drop(me);
}

#[test]
fn a_frame_that_is_not_base64_is_refused_and_the_same_prompt_stays() {
    let (mut app, _me, _dir) = app_without_phone();
    crate::app::sync_phone::start(&mut app);
    wait_label(&mut app, "Phone's next frame");
    type_and_enter(&mut app, "***not base64***");
    assert!(app
        .host_notice
        .as_deref()
        .is_some_and(|n| n.contains("not base64")));
    assert!(prompt(&app).is_some_and(|(l, _, _)| l.contains("Phone's next frame")));
}

#[test]
fn pasted_phone_ops_that_are_garbage_change_nothing_and_say_so() {
    let (mut app, _me, _dir) = app_without_phone();
    crate::app::sync_phone::paste_ops(&mut app);
    type_and_enter(&mut app, "AAAA");
    pump(&mut app, "the refusal", |a| a.sync.last_error.is_some());
}

#[test]
fn a_change_whose_ceremony_is_not_complete_is_not_shown_or_used_as_in_force() {
    use crate::app::sync::{CredModeView, DeviceRoleView};
    use crate::sync::oplog::{CredMode, DeviceRole, OpBody};
    let (mut app, me, _dir) = app_with_ring(false);
    let other = app
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.name == "other")
        .unwrap();
    // The proposer applies its own Red ops at once; the ring view must not.
    app.store
        .sync_append_local(
            &me,
            OpBody::DeviceRoleChange {
                device_id: other.id.clone(),
                to: DeviceRole::Mirror,
            },
        )
        .unwrap();
    app.store
        .sync_append_local(
            &me,
            OpBody::DeviceRoleChange {
                device_id: other.id.clone(),
                to: DeviceRole::Holder,
            },
        )
        .unwrap();
    app.store
        .sync_append_local(
            &me,
            OpBody::ModeChange {
                to: CredMode::Quorum,
                migrated_on: me.public().device_id(),
                cosigner: None,
            },
        )
        .unwrap();
    assert_eq!(app.store.sync_cred_mode().unwrap(), CredMode::Quorum);
    app.refresh_sync_view();
    let shown = app.sync.devices.iter().find(|d| d.id == other.id).unwrap();
    assert_eq!(shown.role, DeviceRoleView::Mirror, "promotion not in force");
    assert_eq!(app.sync.cred_mode, Some(CredModeView::Vault));
    // The switch is this device's unfinished one: the flow shows its ceremony instead of proposing again.
    app.sync.this_device = Some(me.public().device_id());
    crate::app::sync_flows::mode(&mut app);
    pump(&mut app, "the job's outcome", |a| {
        a.mode == AppMode::SyncInfo || a.host_notice.is_some() || a.sync.last_error.is_some()
    });
    assert_eq!(
        crate::sync::quorum::ops_of(&app.store, "ModeChange", None)
            .unwrap()
            .len(),
        1,
        "no second proposal"
    );
}

#[test]
fn the_camera_feeds_the_phones_frames_while_the_flow_is_open() {
    use crate::sync::media::{listen_with, ListenOpts, MediaEvent, MediaKind};
    let (mut app, _me, _dir) = app_without_phone();
    let phone_id = DeviceIdentity::generate();
    let party = Party::new(&phone_id, "my phone", true);
    crate::app::sync_phone::start(&mut app);
    wait_label(&mut app, "Phone's next frame");
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = listen_with(&[], tx.clone(), ListenOpts::ENROLL_FRAMES);
    crate::app::sync_phone::use_camera_channel(&mut app, handle, rx);
    tx.send(MediaEvent::Reply(
        MediaKind::Camera,
        EnrollFrame::Commit {
            role: INITIATOR,
            commit: party.commit(INITIATOR),
        }
        .encode(),
    ))
    .unwrap();
    pump(
        &mut app,
        "the terminal's reveal after the camera frame",
        |a| {
            prompt(a).is_some_and(|(_, _, f)| {
                !f.is_empty()
                    && matches!(
                        EnrollFrame::decode(&decode_frames(&f)),
                        Ok(EnrollFrame::Reveal { .. })
                    )
            })
        },
    );
}

#[test]
fn closing_the_prompt_ends_the_flow_and_releases_the_camera() {
    let (mut app, _me, _dir) = app_without_phone();
    crate::app::sync_phone::start(&mut app);
    wait_label(&mut app, "Phone's next frame");
    app.handle_key(key(KeyCode::Esc)).unwrap();
    pump(&mut app, "the flow to end", |a| a.phone_flow.is_none());
}
