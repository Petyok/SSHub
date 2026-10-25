//! The Sync view fed from the store, review through the store, and the
//! enrollment flow over an in-process peer.

use super::*;
use crate::store::LauncherStore;
use crate::sync::client::spawn_sync_thread;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::enroll::{run_sync_enroll, run_sync_join, SasPrompt};
use crate::sync::oplog::VerifiedBy;
use crate::sync::oplog::{Attendance, DeviceRole, HostFields, Op, OpBody};
use crate::sync::runtime::UiSas;
use crate::sync::transport::{Conn, Limits};
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

const UID: [u8; 16] = [1; 16];

struct Author {
    store: LauncherStore,
    id: DeviceIdentity,
}

impl Author {
    fn append(&self, fields: HostFields) -> Op {
        self.store
            .sync_append_local(
                &self.id,
                OpBody::HostUpsert {
                    host_uid: UID,
                    fields,
                },
            )
            .unwrap()
    }
}

/// An app whose store trusts one other device, `laptop`.
fn app_with_author() -> (App, Author) {
    let app = test_app(vec![]);
    let author = Author {
        store: LauncherStore::open_in_memory().unwrap(),
        id: DeviceIdentity::generate(),
    };
    for store in [&*app.store, &author.store] {
        store
            .sync_register_device(
                &author.id.public(),
                "laptop",
                DeviceRole::Holder,
                Attendance::Attended,
                crate::sync::oplog::DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    (app, author)
}

fn new_host(name: &str, address: &str) -> HostFields {
    HostFields {
        name: Some(name.into()),
        address: Some(address.into()),
        ..Default::default()
    }
}

fn open_review(app: &mut App) {
    app.active_tab = 6;
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(app.mode, AppMode::SyncReview);
}

#[test]
fn the_view_is_filled_from_devices_and_pending_ops_in_the_store() {
    let (mut app, author) = app_with_author();
    assert!(!app.sync.configured);
    app.refresh_sync_view();
    assert!(app.sync.configured, "a known device makes a ring");
    assert_eq!(app.sync.devices.len(), 1);
    assert_eq!(app.sync.devices[0].name, "laptop");
    assert_eq!(app.sync.pending, 0);

    app.store
        .sync_ingest(author.append(new_host("web", "10.0.0.1")))
        .unwrap();
    app.refresh_sync_view();
    assert_eq!(app.sync.pending, 1);
    let op = &app.sync.review[0];
    assert_eq!(op.device, "laptop");
    assert!(op.summary.starts_with("add host web"), "{}", op.summary);
    let address = op.changes.iter().find(|c| c.field == "address").unwrap();
    assert_eq!(
        (address.from.as_str(), address.to.as_str()),
        ("(new host)", "10.0.0.1")
    );
    assert!(address.dangerous);
}

#[test]
fn approving_a_pending_op_applies_it_and_clears_the_review() {
    let (mut app, author) = app_with_author();
    app.store
        .sync_ingest(author.append(new_host("web", "10.0.0.1")))
        .unwrap();
    app.refresh_sync_view();
    assert!(app.store.get_host_by_name("web").unwrap().is_none());

    open_review(&mut app);
    app.handle_key(key_char('a')).unwrap();
    let host = app.store.get_host_by_name("web").unwrap().expect("applied");
    assert_eq!(host.address, "10.0.0.1");
    assert!(app.store.sync_pending().unwrap().is_empty());
    assert_eq!(app.sync.pending, 0);
    assert_eq!(app.mode, AppMode::Normal, "nothing left to review");
    assert!(
        app.hosts.iter().any(|h| h.name() == "web"),
        "the hosts list shows it"
    );
}

#[test]
fn rejecting_a_pending_op_leaves_the_store_untouched() {
    let (mut app, author) = app_with_author();
    app.store
        .sync_ingest(author.append(new_host("web", "10.0.0.1")))
        .unwrap();
    app.refresh_sync_view();
    open_review(&mut app);
    app.handle_key(key_char('r')).unwrap();
    assert!(app.store.get_host_by_name("web").unwrap().is_none());
    assert!(app.store.sync_pending().unwrap().is_empty());
    assert!(app.sync.review.is_empty());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
fn the_diff_shows_the_current_value_and_marks_only_dangerous_fields() {
    let (mut app, author) = app_with_author();
    app.store
        .sync_ingest(author.append(new_host("web", "10.0.0.1")))
        .unwrap();
    app.refresh_sync_view();
    open_review(&mut app);
    app.handle_key(key_char('a')).unwrap();

    app.store
        .sync_ingest(author.append(HostFields {
            address: Some("10.9.9.9".into()),
            notes: Some(Some("moved".into())),
            ..Default::default()
        }))
        .unwrap();
    app.refresh_sync_view();
    assert_eq!(
        app.sync.pending, 1,
        "one dangerous field holds the whole op"
    );
    let op = &app.sync.review[0];
    assert!(op.summary.starts_with("change host web"), "{}", op.summary);
    let by = |f: &str| op.changes.iter().find(|c| c.field == f).unwrap();
    assert_eq!(by("address").from, "10.0.0.1");
    assert_eq!(by("address").to, "10.9.9.9");
    assert!(by("address").dangerous);
    assert_eq!(by("notes").from, "(none)");
    assert!(!by("notes").dangerous);
}

/// The peer of an enrollment.
#[derive(Clone, Copy)]
enum Peer {
    /// A human at a screen, answering the words.
    Screen { agrees: bool },
    /// No tty on the peer (`None` for the confirm).
    NoScreen,
}

/// A worker whose rounds run against an in-process existing device: the real
/// enrollment code on both ends over a socket pair. `rings` gives each side
/// its own ring id (`local`, `peer`); by default the peer founded a ring and
/// this device joins it.
fn attach_enrolling_worker_in(
    app: &mut App,
    peer_kind: Peer,
    rings: Option<(&'static str, &'static str)>,
) -> (Arc<LauncherStore>, String) {
    attach_enrolling_worker_with(app, peer_kind, rings, false)
}

/// [`attach_enrolling_worker_in`]; `hold` keeps the round open after the join
/// until the test sends a command, so the last progress line stays visible.
fn attach_enrolling_worker_with(
    app: &mut App,
    peer_kind: Peer,
    rings: Option<(&'static str, &'static str)>,
    hold: bool,
) -> (Arc<LauncherStore>, String) {
    let store = app.store.clone();
    let id = Arc::new(DeviceIdentity::generate());
    let peer_store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let peer_id = DeviceIdentity::generate();
    let peer_device = peer_id.public().device_id();
    let peer_ring = rings.map_or_else(|| peer_device.clone(), |(_, p)| p.to_string());
    crate::sync::quorum::meta_set(&peer_store, "ring_id", peer_ring.as_bytes()).unwrap();
    if let Some((local, _)) = rings {
        crate::sync::quorum::meta_set(&store, "ring_id", local.as_bytes()).unwrap();
    }
    let peer_for_thread = peer_store.clone();
    let peer_id = Arc::new(peer_id);
    app.attach_sync_worker(spawn_sync_thread(Box::new(move |kind, _host, ctx| {
        if kind != crate::sync::client::Round::Enroll {
            return Ok(Default::default());
        }
        let (a, b) = UnixStream::pair().unwrap();
        let (peer_store, peer_id) = (peer_for_thread.clone(), peer_id.clone());
        let peer = std::thread::spawn(move || {
            let mut conn = Conn::new(
                b.try_clone().unwrap(),
                b,
                Limits::default(),
                Arc::new(AtomicBool::new(false)),
            );
            match peer_kind {
                Peer::Screen { agrees } => {
                    let mut ui = move |_: &SasPrompt<'_>| agrees;
                    run_sync_enroll(&mut conn, &peer_store, &peer_id, "hub", true, Some(&mut ui))
                }
                Peer::NoScreen => {
                    run_sync_enroll(&mut conn, &peer_store, &peer_id, "hub", true, None)
                }
            }
        });
        let mut conn = Conn::new(
            a.try_clone().unwrap(),
            a,
            Limits::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let joined = run_sync_join(&mut conn, &store, &id, "me", true, Some(&mut UiSas(ctx)));
        let _ = peer.join();
        let outcome = joined?;
        if hold {
            ctx.recv_cmd(Duration::from_secs(10));
        }
        Ok(crate::sync::protocol::RoundSummary {
            peer: outcome.peer.device_id(),
            first_contact: true,
            ..Default::default()
        })
    })));
    (peer_store, peer_device)
}

fn attach_enrolling_worker(app: &mut App, peer_agrees: bool) -> (Arc<LauncherStore>, String) {
    attach_enrolling_worker_in(
        app,
        Peer::Screen {
            agrees: peer_agrees,
        },
        None,
    )
}

fn pump(app: &mut App, until: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while !until(app) {
        assert!(
            Instant::now() < end,
            "timed out; error: {:?}",
            app.sync.last_error
        );
        app.poll_sync();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn start_enroll(app: &mut App) {
    app.sync
        .intents
        .push(crate::app::sync::SyncIntent::Enroll { host: "hub".into() });
}

#[test]
fn both_with_screens_compare_words_and_a_yes_joins_verified_by_sas() {
    let mut app = test_app(vec![]);
    let (_peer, peer_device) = attach_enrolling_worker(&mut app, true);
    assert!(!app.sync.configured);
    start_enroll(&mut app);
    pump(&mut app, |a| a.mode == AppMode::SyncSas);
    let sas = app.sync.sas.clone().unwrap();
    assert_eq!(sas.peer, "hub");
    assert_eq!(sas.peer_name, "hub");
    assert!(sas.peer_has_screen);
    assert_eq!(sas.words.split('-').count(), 4, "{}", sas.words);

    app.handle_key(key_char('y')).unwrap();
    pump(&mut app, |a| a.sync.configured && !a.sync.syncing);
    // The ring knows the peer and, since enrollment records it, this device itself.
    assert_eq!(app.sync.devices.len(), 2);
    let peer = app
        .sync
        .devices
        .iter()
        .find(|d| d.id == peer_device)
        .expect("the peer is listed");
    assert_eq!(peer.name, "hub");
    assert_eq!(peer.role, crate::app::sync::DeviceRoleView::Holder);
    assert_eq!(peer.verified_by, Some(VerifiedBy::Sas));
    assert!(app.sync.last_error.is_none(), "{:?}", app.sync.last_error);
}

#[test]
fn a_join_shows_the_ring_being_adopted_as_the_worker_progress_line() {
    let mut app = test_app(vec![]);
    attach_enrolling_worker_with(&mut app, Peer::Screen { agrees: true }, None, true);
    start_enroll(&mut app);
    pump(&mut app, |a| a.mode == AppMode::SyncSas);
    app.handle_key(key_char('y')).unwrap();
    pump(&mut app, |a| {
        a.sync
            .progress
            .as_deref()
            .is_some_and(|p| p.starts_with("adopted the ring ("))
    });
    assert!(app.sync.syncing, "the round is still open");
}

#[test]
fn a_peer_without_a_screen_gets_the_plain_warning_and_an_approval_joins() {
    let mut app = test_app(vec![]);
    let (_peer, peer_device) = attach_enrolling_worker_in(&mut app, Peer::NoScreen, None);
    start_enroll(&mut app);
    pump(&mut app, |a| a.mode == AppMode::SyncSas);
    assert!(!app.sync.sas.as_ref().unwrap().peer_has_screen);

    app.handle_key(key_char('y')).unwrap();
    pump(&mut app, |a| a.sync.configured && !a.sync.syncing);
    let peer = app
        .sync
        .devices
        .iter()
        .find(|d| d.id == peer_device)
        .expect("the peer is listed");
    assert_eq!(peer.verified_by, Some(VerifiedBy::SshHostKey));
    assert!(app.sync.last_error.is_none(), "{:?}", app.sync.last_error);
}

#[test]
fn declining_the_words_joins_nothing_and_says_so() {
    let mut app = test_app(vec![]);
    attach_enrolling_worker(&mut app, true);
    start_enroll(&mut app);
    pump(&mut app, |a| a.mode == AppMode::SyncSas);
    app.handle_key(key_char('n')).unwrap();
    pump(&mut app, |a| a.sync.last_error.is_some());
    assert!(
        app.sync.last_error.as_deref().unwrap().contains("declined"),
        "{:?}",
        app.sync.last_error
    );
    assert!(!app.sync.configured);
    assert!(app.store.sync_devices().unwrap().is_empty());
}

#[test]
fn declining_the_admission_of_a_screenless_peer_joins_nothing() {
    let mut app = test_app(vec![]);
    attach_enrolling_worker_in(&mut app, Peer::NoScreen, None);
    start_enroll(&mut app);
    pump(&mut app, |a| a.mode == AppMode::SyncSas);
    app.handle_key(key_char('n')).unwrap();
    pump(&mut app, |a| a.sync.last_error.is_some());
    assert!(!app.sync.configured);
    assert!(app.store.sync_devices().unwrap().is_empty());
}

#[test]
fn two_different_rings_cannot_be_merged() {
    let mut app = test_app(vec![]);
    attach_enrolling_worker_in(
        &mut app,
        Peer::Screen { agrees: true },
        Some(("ring-local", "ring-peer")),
    );
    start_enroll(&mut app);
    pump(&mut app, |a| a.sync.last_error.is_some());
    assert!(
        app.sync
            .last_error
            .as_deref()
            .unwrap()
            .contains("merging rings is not supported"),
        "{:?}",
        app.sync.last_error
    );
    assert!(app.store.sync_devices().unwrap().is_empty());
}

#[test]
fn a_share_release_names_the_requester_and_the_host_in_the_review() {
    let (app, author) = app_with_author();
    let host = app
        .store
        .create_host(&crate::store::NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let uid = app.store.host_uid(host.id).unwrap().unwrap();
    let op = author
        .store
        .sync_append_local(
            &author.id,
            OpBody::ShareRelease {
                host_uid: uid,
                request_id: 7,
                requester: author.id.public().device_id(),
                eph_x25519_pub: [3; 32],
            },
        )
        .unwrap();
    let rows = app.store.sync_devices().unwrap();
    let view = crate::app::sync_view::pending_view(&app.store, &op, &rows, None);
    // The approver sees who, the requester's full id and how old the request is.
    let want = format!(
        "release a share of web to laptop (device {}, asked ",
        author.id.public().device_id()
    );
    assert!(view.summary.starts_with(&want), "{}", view.summary);
    assert!(view.summary.ends_with(" s ago)"), "{}", view.summary);
}

#[test]
fn enter_on_a_compromised_device_shows_the_proof_and_the_next_steps() {
    let (mut app, author) = app_with_author();
    // The same device signs two different operations at one position.
    let first = author.append(new_host("web", "10.0.0.1"));
    let honest = author.append(HostFields::default());
    let mut other = honest.clone();
    other.body = OpBody::HostUpsert {
        host_uid: UID,
        fields: HostFields {
            notes: Some(Some("different".into())),
            ..Default::default()
        },
    };
    other.signature = author.id.sign(&other.canonical_bytes());
    for op in [first, honest, other] {
        app.store.sync_ingest(op).unwrap();
    }
    app.refresh_sync_view();
    app.active_tab = 6;
    assert_eq!(
        app.sync.devices[0].status,
        crate::app::sync::DeviceStatusView::Compromised
    );
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::SyncForkAlert);
    let fork = app.sync.fork.clone().unwrap();
    assert_eq!(fork.device, "laptop");
    assert!(
        fork.evidence.iter().any(|l| l.starts_with("Proof:")),
        "{:?}",
        fork.evidence
    );
    assert!(fork.evidence.iter().any(|l| l.contains("hash")));
}
