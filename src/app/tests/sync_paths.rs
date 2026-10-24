//! A Quorum credential opened through the unlock modal, one test per path
//! that can bring the second share: the phone's pasted reply, the 24 words,
//! the short code, and a holder under a trust certificate (over a fake ssh
//! that reaches an in-process `sync-serve`).

use super::*;
use crate::credentials::{
    host_key, FilePasswordStore, PasswordStore, QuorumGatedStore, SecretGate, SecretRef,
};
use crate::profile::ProfilePaths;
use crate::sync::client::spawn_sync_thread;
use crate::sync::fountain::FountainDecoder;
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::{CertScope, DeviceKind, HostUid, OpBody};
use crate::sync::protocol::run_sync_serve;
use crate::sync::protocol::tests::Dev;
use crate::sync::quorum::own_shares;
use crate::sync::quorum::tests::{all_to_all, append, payload, quorum_mode};
use crate::sync::runtime::{guard_synced_credentials, round_fn, Peers, RoundDeps};
use crate::sync::share_release::share_to_bytes;
use crate::sync::transport::{Conn, Limits};
use crate::sync::wire::{ChallengeWire, ShareResponseWire};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAST: crate::sync::crypto::Argon2Profile = crate::sync::crypto::Argon2Profile {
    mem_kib: 64,
    iters: 1,
};

fn file_secrets(p: &ProfilePaths) -> Box<dyn PasswordStore> {
    Box::new(FilePasswordStore::new(p.credentials_file()))
}

pub(super) fn pump(app: &mut App, what: &str, until: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !until(app) {
        assert!(
            Instant::now() < end,
            "timed out waiting for {what}; error {:?}, notice {:?}",
            app.sync.last_error,
            app.host_notice
        );
        app.poll_sync();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// An app that is device 1 of a Quorum ring: 0 phone, 1 this app, 2 a holder.
pub(super) struct Ring {
    pub(super) app: App,
    pub(super) gate: Arc<SecretGate>,
    pub(super) paths: ProfilePaths,
    pub(super) d: Vec<Dev>,
    pub(super) host: HostUid,
    pub(super) key: String,
    _dir: tempfile::TempDir,
}

pub(super) fn ring(ssh_program: Option<String>) -> Ring {
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
    let mine = Dev {
        store: app.store.clone(),
        did: me.public().device_id(),
        id: me,
    };
    let d = vec![Dev::new(), mine, Dev::new()];
    for (i, x) in d.iter().enumerate() {
        for (j, y) in d.iter().enumerate() {
            if i != j {
                x.trust_as(
                    y,
                    if j == 0 {
                        DeviceKind::Phone
                    } else {
                        DeviceKind::Terminal
                    },
                );
            }
        }
    }
    // Enrollment records this device itself in its own ring.
    d[1].store
        .sync_register_device(
            &d[1].id.public(),
            "me",
            crate::sync::oplog::DeviceRole::Holder,
            crate::sync::oplog::Attendance::Attended,
            DeviceKind::Terminal,
            crate::sync::oplog::VerifiedBy::Sas,
        )
        .unwrap();
    quorum_mode(&d[2]);
    all_to_all(&d);
    let row = app
        .store
        .create_host(&crate::store::NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let host = app.store.host_uid(row.id).unwrap().unwrap();
    crate::sync::cred_ops::cred_put(&d[2].store, &d[2].id, &host, &payload("trusted")).unwrap();
    all_to_all(&d);

    let deps = RoundDeps {
        paths: paths.clone(),
        store: app.store.clone(),
        gate: gate.clone(),
        argon: FAST,
        open_secrets: file_secrets,
        ssh_program,
    };
    app.attach_sync_worker(spawn_sync_thread(round_fn(deps)));
    app.refresh_sync_view();
    Ring {
        app,
        gate,
        paths,
        d,
        host,
        key: host_key(row.id),
        _dir: dir,
    }
}

/// Ask for the credential the way a connect does; the modal opens.
fn ask(r: &mut Ring) {
    let blocked = r
        .app
        .preflight_unlock(vec![SecretRef::Key(r.key.clone())], "web", |app| {
            app.host_notice = Some("resumed".into());
            Ok(())
        });
    assert!(!blocked);
    assert_eq!(r.app.mode, AppMode::SyncUnlock);
}

fn unlocked(r: &Ring) -> bool {
    r.gate.get(&r.key).is_some()
}

fn status(app: &App, kind: crate::app::sync::UnlockPathKind) -> Option<String> {
    app.sync
        .unlock
        .as_ref()?
        .paths
        .iter()
        .find(|p| p.kind == kind)
        .map(|p| p.status.clone())
}

pub(super) fn type_and_enter(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
}

fn phone_share(r: &Ring) -> crate::sync::crypto::Share {
    own_shares(&r.d[0].store, &r.d[0].id, &r.host)
        .unwrap()
        .remove(0)
        .1
}

/// What the phone page does with the QR on screen: read the frames, answer
/// with the share sealed to the challenge's ephemeral key.
fn phone_reply_text(frames: &[Vec<u8>], share: &crate::sync::crypto::Share) -> String {
    let mut decoder = FountainDecoder::new();
    for f in frames {
        decoder.push(f).unwrap();
    }
    let wire = ChallengeWire::decode(&decoder.finish().unwrap().expect("all frames")).unwrap();
    let ciphertext =
        crate::sync::crypto::DeviceIdentity::seal_to(&wire.eph_x25519_pub, &share_to_bytes(share));
    B64.encode(
        ShareResponseWire {
            request_id: wire.request_id,
            ciphertext,
        }
        .encode(),
    )
}

fn wait_for_phone_prompt(r: &mut Ring) -> Vec<Vec<u8>> {
    pump(&mut r.app, "the phone prompt", |a| {
        a.sync.unlock.as_ref().is_some_and(|u| !u.frames.is_empty())
    });
    r.app.sync.unlock.as_ref().unwrap().frames.clone()
}

fn assert_resumed_and_open(r: &mut Ring) {
    pump(&mut r.app, "the action to resume", |a| {
        a.host_notice.as_deref() == Some("resumed")
    });
    assert_eq!(
        r.gate.get(&r.key).as_deref().map(String::as_str),
        Some("trusted")
    );
    assert_eq!(r.app.mode, AppMode::Normal);
}

#[test]
fn the_phones_pasted_reply_opens_the_credential_and_a_bad_paste_is_refused_in_place() {
    let mut r = ring(None);
    ask(&mut r);
    let frames = wait_for_phone_prompt(&mut r);
    use crate::app::sync::UnlockPathKind::*;
    assert!(status(&r.app, PhoneQr).is_some() && status(&r.app, Clipboard).is_some());

    type_and_enter(&mut r.app, "this is not a reply");
    pump(&mut r.app, "the refusal", |a| {
        status(a, Clipboard).is_some_and(|s| s.starts_with("not accepted"))
    });
    assert!(!unlocked(&r));
    assert_eq!(r.app.mode, AppMode::SyncUnlock);

    let reply = phone_reply_text(&frames, &phone_share(&r));
    type_and_enter(&mut r.app, &reply);
    assert_resumed_and_open(&mut r);
}

#[test]
fn the_24_words_open_the_credential() {
    let mut r = ring(None);
    ask(&mut r);
    wait_for_phone_prompt(&mut r);
    let bytes: [u8; 32] = phone_share(&r).bytes.clone().try_into().unwrap();
    let words = crate::sync::crypto::bip39_encode(&bytes);
    type_and_enter(&mut r.app, &words);
    assert_resumed_and_open(&mut r);
}

#[test]
fn the_short_code_opens_the_credential_from_a_stored_snapshot() {
    let mut r = ring(None);
    let code = crate::sync::codes::gen_snapshot_code();
    let blob =
        crate::sync::snapshot::snapshot_seal(&phone_share(&r), &code, crate::store::now_ts())
            .to_bytes();
    crate::sync::unlock_paths::remember_snapshot(&r.app.store, &r.host, &blob).unwrap();
    ask(&mut r);
    wait_for_phone_prompt(&mut r);
    type_and_enter(&mut r.app, &code.to_lowercase().replace('-', " "));
    assert_resumed_and_open(&mut r);
}

/// A fake `ssh`: connects its stdio to a TCP port where an in-process
/// `sync-serve` of device 2 answers.
fn fake_ssh_to_holder(r: &Ring) -> String {
    use std::os::unix::fs::PermissionsExt;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (store, id) = (r.d[2].store.clone(), r.d[2].id.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let closer = stream.try_clone().unwrap();
            let mut conn = Conn::new(
                stream.try_clone().unwrap(),
                stream,
                Limits::default(),
                Arc::new(AtomicBool::new(false)),
            );
            let _ = run_sync_serve(&store, &id, &mut conn);
            // The Conn's reader thread holds a clone: close the socket so the fake ssh ends.
            let _ = closer.shutdown(std::net::Shutdown::Both);
        }
    });
    let script = r._dir.path().join("fake-ssh");
    std::fs::write(
        &script,
        format!("#!/bin/bash\nexec 3<>/dev/tcp/127.0.0.1/{port}\ncat <&3 &\ncat >&3\nwait\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script.to_string_lossy().into_owned()
}

fn certify_me(r: &Ring) {
    let now = crate::store::now_ts();
    append(
        &r.d[0],
        OpBody::CertIssue {
            device_id: r.d[1].did.clone(),
            scope: CertScope::Hosts(vec![r.host]),
            expiry: now + 7 * 86_400,
            max_new_hosts_per_day: 5,
            epoch_lo: 0,
            epoch_hi: 9,
        },
    );
    all_to_all(&r.d);
}

#[test]
fn a_holder_under_a_trust_certificate_releases_a_share_without_the_user_typing() {
    let mut r = ring(None);
    certify_me(&r);
    let program = fake_ssh_to_holder(&r);
    // Rebuild the worker with the fake ssh (the ring fixture is reused).
    r.app
        .attach_sync_worker(spawn_sync_thread(round_fn(RoundDeps {
            paths: r.paths.clone(),
            store: r.app.store.clone(),
            gate: r.gate.clone(),
            argon: FAST,
            open_secrets: file_secrets,
            ssh_program: Some(program),
        })));
    Peers::record(&r.paths, &r.d[2].did, "holder").unwrap();
    ask(&mut r);
    assert_resumed_and_open(&mut r);
    assert!(
        status(&r.app, crate::app::sync::UnlockPathKind::Peer).is_none()
            || r.app.mode == AppMode::Normal,
        "the modal closed once the share arrived"
    );
}

#[test]
fn a_holder_without_a_certificate_defers_and_the_phone_path_takes_over() {
    let mut r = ring(None);
    let program = fake_ssh_to_holder(&r);
    r.app
        .attach_sync_worker(spawn_sync_thread(round_fn(RoundDeps {
            paths: r.paths.clone(),
            store: r.app.store.clone(),
            gate: r.gate.clone(),
            argon: FAST,
            open_secrets: file_secrets,
            ssh_program: Some(program),
        })));
    Peers::record(&r.paths, &r.d[2].did, "holder").unwrap();
    ask(&mut r);
    let frames = wait_for_phone_prompt(&mut r);
    let peer =
        status(&r.app, crate::app::sync::UnlockPathKind::Peer).expect("the peer path reported");
    assert!(
        !peer.starts_with("released"),
        "no certificate, no release: {peer}"
    );
    assert!(!unlocked(&r));
    let reply = phone_reply_text(&frames, &phone_share(&r));
    type_and_enter(&mut r.app, &reply);
    assert_resumed_and_open(&mut r);
}

#[test]
fn auto_sync_on_start_queues_one_sync_per_device_with_a_known_host() {
    use crate::app::sync::SyncIntent;
    let mut r = ring(None);
    r.app.queue_auto_sync();
    assert!(r.app.sync.intents.is_empty(), "no host is known yet");
    Peers::record(&r.paths, &r.d[2].did, "holder").unwrap();
    r.app.config.sync.auto_sync_on_start = false;
    r.app.queue_auto_sync();
    assert!(r.app.sync.intents.is_empty(), "switched off in the config");
    r.app.config.sync.auto_sync_on_start = true;
    r.app.queue_auto_sync();
    assert_eq!(
        r.app.sync.intents,
        vec![SyncIntent::SyncNow {
            device: r.d[2].did.clone()
        }]
    );
}

fn rebuild_worker(r: &mut Ring, program: String) {
    r.app
        .attach_sync_worker(spawn_sync_thread(round_fn(RoundDeps {
            paths: r.paths.clone(),
            store: r.app.store.clone(),
            gate: r.gate.clone(),
            argon: FAST,
            open_secrets: file_secrets,
            ssh_program: Some(program),
        })));
}

/// What a holder's TUI does once its user approved a release request.
fn holder_approves(d: &Dev) {
    for op in d.store.sync_pending().unwrap() {
        if matches!(op.body, OpBody::ShareRelease { .. }) {
            d.store.sync_resolve_pending(&op.hash(), true).unwrap();
            for body in crate::sync::quorum::grant_bodies(&d.store, &d.id, &op).unwrap() {
                d.store.sync_append_local(&d.id, body).unwrap();
            }
        }
    }
}

#[test]
fn an_attended_holder_approving_on_its_screen_releases_the_share() {
    let mut r = ring(None);
    let program = fake_ssh_to_holder(&r);
    rebuild_worker(&mut r, program);
    Peers::record(&r.paths, &r.d[2].did, "holder").unwrap();
    ask(&mut r);
    pump(&mut r.app, "the attended prompt", |a| {
        status(a, crate::app::sync::UnlockPathKind::Peer)
            .is_some_and(|s| s.contains("approve on its screen"))
    });
    let end = Instant::now() + Duration::from_secs(40);
    while r.app.host_notice.as_deref() != Some("resumed") {
        assert!(
            Instant::now() < end,
            "never resumed: {:?}",
            r.app.sync.last_error
        );
        holder_approves(&r.d[2]);
        r.app.poll_sync();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_resumed_and_open(&mut r);
}

/// This app is the holder, device 2 asks: deliver the request, refresh.
fn request_from_device_2(r: &mut Ring) -> crate::sync::quorum::PeerRequest {
    let req = crate::sync::quorum::peer_request(&r.d[2].store, &r.d[2].id, &r.host).unwrap();
    // Ingest only: `all_to_all` would approve what waits, as a user does.
    let mine = r.d[1].store.sync_chain_heads().unwrap();
    let have = mine
        .iter()
        .find(|(d, _, _)| *d == r.d[2].did)
        .map_or(0, |(_, s, _)| *s);
    for op in r.d[2].store.sync_chain_tail(&r.d[2].did, have).unwrap() {
        r.d[1].store.sync_ingest(op).unwrap();
    }
    r.app.refresh_sync_view();
    req
}

#[test]
fn a_release_request_opens_the_holders_modal_and_approving_grants_the_requester() {
    let mut r = ring(None);
    let req = request_from_device_2(&mut r);
    assert_eq!(
        r.app.mode,
        AppMode::SyncReleaseConfirm,
        "pending {:?}, error {:?}",
        r.app
            .store
            .sync_pending()
            .unwrap()
            .iter()
            .map(|o| format!("{:?}", o.body).chars().take(40).collect::<String>())
            .collect::<Vec<_>>(),
        r.app.sync.last_error
    );
    let view = r.app.sync.release.clone().unwrap();
    assert_eq!(view.requester_id, r.d[2].did);
    assert_eq!(view.host, "web");

    r.app.handle_key(key_char('y')).unwrap();
    pump(&mut r.app, "the grants", |a| {
        a.sync.pending == 0 && !a.sync.syncing
    });
    assert!(r.app.store.sync_pending().unwrap().is_empty());
    all_to_all(&r.d);
    let share = crate::sync::quorum::peer_poll(&r.d[2].store, &req, 0)
        .unwrap()
        .expect("the requester finds the grant");
    let mine = own_shares(&r.app.store, &r.d[1].id, &r.host)
        .unwrap()
        .remove(0)
        .1;
    assert_eq!(share, mine, "the requester got exactly the holder's share");
}

#[test]
fn refusing_a_release_request_grants_nothing_and_does_not_ask_again() {
    let mut r = ring(None);
    let req = request_from_device_2(&mut r);
    assert_eq!(r.app.mode, AppMode::SyncReleaseConfirm);
    r.app.handle_key(key_char('n')).unwrap();
    pump(&mut r.app, "the refusal", |a| {
        a.sync.pending == 0 && !a.sync.syncing
    });
    all_to_all(&r.d);
    assert!(crate::sync::quorum::peer_poll(&r.d[2].store, &req, 0)
        .unwrap()
        .is_none());
    r.app.refresh_sync_view();
    assert_eq!(
        r.app.mode,
        AppMode::Normal,
        "the answered request is not asked again"
    );
}

/// Ingest only (`all_to_all` approves what waits, which a Red change must not get).
pub(super) fn ingest_from(from: &Dev, to: &Dev) {
    let have = to
        .store
        .sync_chain_heads()
        .unwrap()
        .into_iter()
        .find(|(d, _, _)| *d == from.did)
        .map_or(0, |(_, s, _)| s);
    for op in from.store.sync_chain_tail(&from.did, have).unwrap() {
        to.store.sync_ingest(op).unwrap();
    }
}

#[test]
fn a_red_change_shows_its_ceremony_and_takes_effect_only_when_all_proofs_are_in() {
    use crate::sync::ceremony::{factor_register_paper, red_phone_sign_body, settle_red_ops};
    let mut r = ring(None);
    let words = crate::sync::crypto::bip39_encode(&[7u8; 32]);
    // The holder proposes registering the paper as a third factor.
    let proposal = append(&r.d[2], factor_register_paper(&words).unwrap());
    ingest_from(&r.d[2], &r.d[1]);
    r.app.refresh_sync_view();
    let red = r.app.sync.review[0]
        .red
        .clone()
        .expect("a Red change shows its ceremony");
    assert!(red.missing.iter().any(|m| m.contains("phone")), "{red:?}");
    assert!(red.missing.iter().any(|m| m.contains("second")), "{red:?}");

    // Approving through the store directly is refused: only the ceremony applies it.
    open_review(&mut r.app);
    r.app.sync_resolve_review(true);
    assert!(r
        .app
        .host_notice
        .as_deref()
        .is_some_and(|n| n.contains("Red change")));
    assert_eq!(r.app.store.sync_pending().unwrap().len(), 1);

    // `a` adds this device's approval; the change still waits for the phone.
    r.app.handle_key(key_char('a')).unwrap();
    pump(&mut r.app, "the approval", |a| {
        a.host_notice
            .as_deref()
            .is_some_and(|n| n.starts_with("proof added"))
    });
    let red = r.app.sync.review[0].red.clone().unwrap();
    assert!(!red.missing.iter().any(|m| m.contains("second")), "{red:?}");
    assert!(red.have.iter().any(|h| h.contains("second")));

    // The phone signs; once the proofs are all in, the ceremony applies it.
    // The phone saw the proposal (its proof must come after it), then signs.
    ingest_from(&r.d[2], &r.d[0]);
    append(&r.d[0], red_phone_sign_body(&proposal));
    ingest_from(&r.d[0], &r.d[1]);
    let st =
        crate::sync::ceremony::red_status(&r.app.store, &proposal, crate::store::now_ts()).unwrap();
    let report = settle_red_ops(&r.app.store, crate::store::now_ts()).unwrap();
    assert_eq!(report.applied, vec![proposal.hash()], "{st:?}");
    r.app.refresh_sync_view();
    assert!(r.app.sync.review.is_empty());
}

#[test]
fn the_paper_phrase_is_typed_without_echo_and_added_as_a_factor_proof() {
    use crate::sync::ceremony::factor_register_paper;
    let mut r = ring(None);
    let words = crate::sync::crypto::bip39_encode(&[9u8; 32]);
    append(&r.d[2], factor_register_paper(&words).unwrap());
    ingest_from(&r.d[2], &r.d[1]);
    r.app.refresh_sync_view();
    open_review(&mut r.app);
    r.app.handle_key(key_char('p')).unwrap();
    assert_eq!(r.app.mode, AppMode::SyncPrompt);
    type_and_enter(&mut r.app, &words);
    pump(&mut r.app, "the factor proof", |a| {
        a.host_notice
            .as_deref()
            .is_some_and(|n| n.starts_with("proof added"))
    });
    // The words are in no file of the profile.
    let mut hits = Vec::new();
    for e in std::fs::read_dir(&r.paths.root).unwrap().flatten() {
        if e.path().is_file() && std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains(&words))
        {
            hits.push(e.path());
        }
    }
    assert!(hits.is_empty(), "{hits:?}");
}

fn open_review(app: &mut App) {
    app.active_tab = 6;
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(app.mode, AppMode::SyncReview);
}

pub(super) fn mark_this_device(r: &mut Ring) {
    r.app.sync.this_device = Some(r.d[1].did.clone());
    r.app.refresh_sync_view();
    assert!(r.app.sync.devices.iter().any(|d| d.this));
}

pub(super) fn enter(app: &mut App) {
    app.handle_key(key(KeyCode::Enter)).unwrap();
}

#[test]
fn the_role_flow_proposes_a_change_for_the_chosen_device_through_the_worker() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    crate::app::sync_flows::role(&mut r.app);
    assert_eq!(r.app.mode, AppMode::SyncMenu);
    enter(&mut r.app); // the first other device
    assert_eq!(r.app.mode, AppMode::SyncMenu, "now the role menu");
    r.app.handle_key(key(KeyCode::Down)).unwrap();
    enter(&mut r.app); // Mirror
    pump(&mut r.app, "the proposal panel", |a| {
        a.mode == AppMode::SyncInfo
    });
    let ops = crate::sync::quorum::ops_of(&r.app.store, "DeviceRoleChange", None).unwrap();
    assert_eq!(ops.len(), 1, "one DeviceRoleChange was appended");
}

#[test]
fn revoking_needs_the_devices_name_typed_back() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    crate::app::sync_flows::revoke(&mut r.app);
    enter(&mut r.app);
    assert_eq!(r.app.mode, AppMode::SyncPrompt);
    type_and_enter(&mut r.app, "not the name");
    assert!(r
        .app
        .host_notice
        .as_deref()
        .is_some_and(|n| n.contains("nothing changed")));
    assert!(
        crate::sync::quorum::ops_of(&r.app.store, "DeviceRevoke", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_paper_phrase_is_shown_once_and_registered_only_when_typed_back_exactly() {
    let mut r = ring(None);
    crate::app::sync_flows::paper_setup(&mut r.app);
    assert_eq!(r.app.mode, AppMode::SyncInfo);
    let words = r.app.sync_paper.clone().expect("held in memory");
    assert_eq!(words.split_whitespace().count(), 24);
    r.app.handle_key(key(KeyCode::Esc)).unwrap();

    // A wrong copy registers nothing and drops the phrase.
    crate::app::sync_flows::paper_register(&mut r.app);
    type_and_enter(&mut r.app, "wrong words");
    assert!(r.app.sync_paper.is_none());
    assert!(
        crate::sync::quorum::ops_of(&r.app.store, "FactorRegister", None)
            .unwrap()
            .is_empty()
    );

    // The right copy registers it as the third factor.
    crate::app::sync_flows::paper_setup(&mut r.app);
    let words = r.app.sync_paper.clone().unwrap();
    r.app.handle_key(key(KeyCode::Esc)).unwrap();
    crate::app::sync_flows::paper_register(&mut r.app);
    type_and_enter(&mut r.app, &words);
    pump(&mut r.app, "the registration", |a| {
        a.mode == AppMode::SyncInfo
    });
    assert_eq!(
        crate::sync::quorum::ops_of(&r.app.store, "FactorRegister", None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_reply_heard_or_seen_opens_the_credential_and_a_reply_for_another_request_is_refused() {
    use crate::app::sync::UnlockPathKind::{Audio, Camera};
    use crate::sync::media::{MediaEvent, MediaKind};
    let mut r = ring(None);
    ask(&mut r);
    let frames = wait_for_phone_prompt(&mut r);
    // Each listener says what it is doing, or why it is not (feature off, no device).
    pump(&mut r.app, "the listener statuses", |a| {
        status(a, Audio).is_some() && status(a, Camera).is_some()
    });
    let tx = loop {
        if let Some(tx) = crate::sync::ui_shares::test_hooks::channel(&r.paths.root) {
            break tx;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    // A reply sealed for some other request is refused and the wait goes on.
    let bytes_of = |text: &str| B64.decode(text).unwrap();
    let mut other = bytes_of(&phone_reply_text(&frames, &phone_share(&r)));
    let at = other.len().min(8) - 1;
    other[at] ^= 0x55; // damages the frame header or request id: not an acceptable reply
    tx.send(MediaEvent::Reply(MediaKind::Audio, other)).unwrap();
    pump(&mut r.app, "the refusal", |a| {
        status(a, Audio).is_some_and(|s| s.starts_with("not accepted"))
    });
    assert!(!unlocked(&r));

    // The phone's real reply, heard by the microphone, opens it.
    let good = bytes_of(&phone_reply_text(&frames, &phone_share(&r)));
    tx.send(MediaEvent::Reply(MediaKind::Audio, good)).unwrap();
    assert_resumed_and_open(&mut r);
}

/// A group with one host in the app's store, for scoping a trust session.
fn group_with_host(r: &Ring) -> (crate::sync::oplog::CertScope, [u8; 16]) {
    let group = r
        .app
        .store
        .create_group(&crate::store::NewHostGroup {
            name: "staging".into(),
            ..Default::default()
        })
        .unwrap();
    let host = r
        .app
        .store
        .create_host(&crate::store::NewHost {
            group_id: Some(group.id),
            ..crate::store::NewHost::launcher("stg-1", "10.0.0.7")
        })
        .unwrap();
    let guid = r.app.store.group_uid(group.id).unwrap().unwrap();
    let huid = r.app.store.host_uid(host.id).unwrap().unwrap();
    (crate::sync::oplog::CertScope::Group(guid), huid)
}

fn frames_of_prompt(app: &App) -> Vec<Vec<u8>> {
    match app.sync_ui.as_ref() {
        Some(crate::app::sync_ui::SyncUi::Prompt { frames, .. }) => frames.clone(),
        _ => panic!("no QR prompt is open"),
    }
}

fn decode_frames(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut d = FountainDecoder::new();
    for f in frames {
        d.push(f).unwrap();
    }
    d.finish().unwrap().expect("all frames")
}

#[test]
fn a_trust_session_is_scoped_to_a_group_or_a_host_never_everything() {
    let mut r = ring(None);
    let (scope, _) = group_with_host(&r);
    r.app.refresh_sync_view();
    let labels = r.app.sync.trust_scopes.clone();
    assert!(
        labels.iter().any(|l| l.starts_with("group staging")),
        "{labels:?}"
    );
    assert!(labels.iter().any(|l| l == "host stg-1"));
    assert!(!labels
        .iter()
        .any(|l| l.to_lowercase().contains("everything")));
    assert!(r.app.sync.trust_scope_values.contains(&scope));
}

#[test]
fn space_marks_hosts_and_the_request_lists_exactly_those() {
    use crate::sync::oplog::CertScope;
    use crate::sync::wire::CertRequestWire;
    let mut r = ring(None);
    mark_this_device(&mut r);
    let (_, h1) = group_with_host(&r);
    let h2 = {
        let h = r
            .app
            .store
            .create_host(&crate::store::NewHost::launcher("other", "10.0.0.8"))
            .unwrap();
        r.app.store.host_uid(h.id).unwrap().unwrap()
    };
    r.app.refresh_sync_view();
    r.app.active_tab = 6;
    r.app.handle_key(key_char('t')).unwrap();
    // A group row refuses the mark.
    let g = r
        .app
        .sync
        .trust_scope_values
        .iter()
        .position(|s| matches!(s, CertScope::Group(_)))
        .unwrap();
    r.app.sync.trust_scope_selected = g;
    r.app.handle_key(key_char(' ')).unwrap();
    assert!(r.app.sync.trust_marked.is_empty());
    for uid in [h1, h2] {
        r.app.sync.trust_scope_selected = r
            .app
            .sync
            .trust_scope_values
            .iter()
            .position(|s| *s == CertScope::Hosts(vec![uid]))
            .unwrap();
        r.app.handle_key(key_char(' ')).unwrap();
    }
    assert_eq!(r.app.sync.trust_marked.len(), 2);
    // Space again unmarks.
    r.app.handle_key(key_char(' ')).unwrap();
    assert_eq!(r.app.sync.trust_marked.len(), 1);
    r.app.handle_key(key_char(' ')).unwrap();
    enter(&mut r.app); // scope
    enter(&mut r.app); // term
    type_and_enter(&mut r.app, "5");
    let wire = CertRequestWire::decode(&decode_frames(&frames_of_prompt(&r.app))).unwrap();
    let mut want = vec![h1, h2];
    want.sort_unstable();
    assert_eq!(wire.scope, CertScope::Hosts(want));
}

#[test]
fn the_trust_flow_shows_the_phone_a_request_and_takes_its_answer_into_the_ceremony() {
    use crate::sync::oplog::{CertScope, OpBody};
    use crate::sync::wire::{CertRequestWire, PhoneOpsWire};
    let mut r = ring(None);
    mark_this_device(&mut r);
    let (_, huid) = group_with_host(&r);
    r.app.refresh_sync_view();
    // The host's scope, chosen through the real menus.
    let pick = r
        .app
        .sync
        .trust_scope_values
        .iter()
        .position(|s| *s == CertScope::Hosts(vec![huid]))
        .unwrap();
    r.app.active_tab = 6;
    r.app.handle_key(key_char('t')).unwrap();
    assert_eq!(r.app.mode, AppMode::TrustSessionScope);
    r.app.sync.trust_scope_selected = pick;
    enter(&mut r.app); // scope
    enter(&mut r.app); // term: the default month
    assert_eq!(r.app.mode, AppMode::SyncPrompt, "the budget is asked");
    type_and_enter(&mut r.app, "5");
    // The request is a looping QR whose bytes say what was asked.
    let wire = CertRequestWire::decode(&decode_frames(&frames_of_prompt(&r.app))).unwrap();
    assert_eq!(wire.scope, CertScope::Hosts(vec![huid]));
    assert_eq!(wire.max_new_hosts_per_day, 5);
    assert_eq!(wire.device_id[..], hex::decode(&r.d[1].did).unwrap()[..8]);

    // The phone approves and returns its signed certificate; the user pastes it.
    ingest_from(&r.d[2], &r.d[0]);
    let now = crate::store::now_ts();
    let issue = append(
        &r.d[0],
        OpBody::CertIssue {
            device_id: r.d[1].did.clone(),
            scope: CertScope::Hosts(vec![huid]),
            expiry: now + 30 * 86_400,
            max_new_hosts_per_day: 5,
            epoch_lo: 0,
            epoch_hi: 9,
        },
    );
    let answer = B64.encode(
        PhoneOpsWire {
            ops: vec![(issue.canonical_bytes(), issue.signature)],
        }
        .encode(),
    );
    type_and_enter(&mut r.app, &answer);
    pump(&mut r.app, "the ceremony status", |a| {
        a.mode == AppMode::SyncInfo
    });
    // The change waits in Review with its ceremony; nothing applied by itself.
    r.app.refresh_sync_view();
    let cert = r
        .app
        .sync
        .review
        .iter()
        .find(|p| p.summary.to_lowercase().contains("cert"))
        .or_else(|| r.app.sync.review.first());
    assert!(
        cert.is_some_and(|c| c.red.is_some()),
        "{:?}",
        r.app.sync.review
    );
}

#[test]
fn a_garbage_answer_from_the_phone_is_refused_and_a_trust_session_can_be_revoked() {
    use crate::sync::oplog::{CertScope, OpBody};
    let mut r = ring(None);
    mark_this_device(&mut r);
    let (_, huid) = group_with_host(&r);
    let now = crate::store::now_ts();
    ingest_from(&r.d[2], &r.d[0]);
    let issue = append(
        &r.d[0],
        OpBody::CertIssue {
            device_id: r.d[1].did.clone(),
            scope: CertScope::Hosts(vec![huid]),
            expiry: now + 30 * 86_400,
            max_new_hosts_per_day: 5,
            epoch_lo: 0,
            epoch_hi: 9,
        },
    );
    // The certificate arrives and is approved (as the ceremony would).
    ingest_from(&r.d[0], &r.d[1]);
    let _ = r.app.store.sync_resolve_pending(&issue.hash(), true);
    r.app.refresh_sync_view();

    // Extending asks the phone: the request shows as a QR; a paste that is not
    // the phone's answer is refused with a notice.
    crate::app::sync_trust::extend(&mut r.app);
    if r.app.mode == AppMode::SyncMenu {
        enter(&mut r.app);
        type_and_enter(&mut r.app, "3");
        let wire = crate::sync::wire::CertExtendRequestWire::decode(&decode_frames(
            &frames_of_prompt(&r.app),
        ))
        .unwrap();
        assert_eq!(wire.extra_hosts, 3);
        assert_eq!(wire.cert_id, issue.hash());
        type_and_enter(&mut r.app, "not the answer");
        assert!(r
            .app
            .host_notice
            .as_deref()
            .is_some_and(|n| n.contains("not the phone's answer")));

        crate::app::sync_trust::revoke(&mut r.app);
        enter(&mut r.app);
        pump(&mut r.app, "the revocation", |a| {
            a.host_notice
                .as_deref()
                .is_some_and(|n| n.contains("revoked"))
        });
        assert_eq!(
            crate::sync::quorum::ops_of(&r.app.store, "CertRevoke", None)
                .unwrap()
                .len(),
            1
        );
    } else {
        panic!("the certificate was not listed: {:?}", r.app.host_notice);
    }
}
