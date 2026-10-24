//! The Sync view fed from the store, review through the store, and the
//! enrollment flow over an in-process peer.

use super::*;
use crate::store::LauncherStore;
use crate::sync::client::spawn_sync_thread;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::enroll::{run_sync_enroll, run_sync_join};
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

/// A worker whose rounds run against an in-process existing device: the real
/// enrollment code on both ends over a socket pair, the peer agreeing.
fn attach_enrolling_worker(app: &mut App, peer_agrees: bool) -> (Arc<LauncherStore>, String) {
    let store = app.store.clone();
    let id = Arc::new(DeviceIdentity::generate());
    let peer_store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let peer_id = DeviceIdentity::generate();
    let peer_device = peer_id.public().device_id();
    let peer_for_thread = peer_store.clone();
    let peer_id = Arc::new(peer_id);
    app.attach_sync_worker(spawn_sync_thread(Box::new(move |_kind, _host, ctx| {
        let (a, b) = UnixStream::pair().unwrap();
        let (peer_store, peer_id) = (peer_for_thread.clone(), peer_id.clone());
        let peer = std::thread::spawn(move || {
            let mut conn = Conn::new(
                b.try_clone().unwrap(),
                b,
                Limits::default(),
                Arc::new(AtomicBool::new(false)),
            );
            run_sync_enroll(
                &mut conn,
                &peer_store,
                &peer_id,
                "hub",
                true,
                &mut |_: &str, _: &str| peer_agrees,
            )
        });
        let mut conn = Conn::new(
            a.try_clone().unwrap(),
            a,
            Limits::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let joined = run_sync_join(&mut conn, &store, &id, "me", true, &mut UiSas(ctx));
        let _ = peer.join();
        let outcome = joined?;
        Ok(crate::sync::protocol::RoundSummary {
            peer: outcome.peer.device_id(),
            first_contact: true,
            ..Default::default()
        })
    })));
    (peer_store, peer_device)
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

#[test]
fn enrolling_a_host_shows_the_words_and_a_yes_joins_the_ring() {
    let mut app = test_app(vec![]);
    let (_peer, peer_device) = attach_enrolling_worker(&mut app, true);
    assert!(!app.sync.configured);
    app.sync
        .intents
        .push(crate::app::sync::SyncIntent::Enroll { host: "hub".into() });
    pump(&mut app, |a| a.mode == AppMode::SyncSas);
    let sas = app.sync.sas.clone().unwrap();
    assert_eq!(sas.peer, "hub");
    assert_eq!(sas.words.split('-').count(), 4, "{}", sas.words);

    app.handle_key(key_char('y')).unwrap();
    pump(&mut app, |a| a.sync.configured && !a.sync.syncing);
    assert_eq!(app.sync.devices.len(), 1);
    assert_eq!(app.sync.devices[0].id, peer_device);
    assert_eq!(app.sync.devices[0].name, "hub");
    assert_eq!(
        app.sync.devices[0].role,
        crate::app::sync::DeviceRoleView::Holder
    );
    assert!(app.sync.last_error.is_none(), "{:?}", app.sync.last_error);
}

#[test]
fn declining_the_words_joins_nothing_and_says_so() {
    let mut app = test_app(vec![]);
    attach_enrolling_worker(&mut app, true);
    app.sync
        .intents
        .push(crate::app::sync::SyncIntent::Enroll { host: "hub".into() });
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
            },
        )
        .unwrap();
    let rows = app.store.sync_devices().unwrap();
    let view = crate::app::sync_view::pending_view(&app.store, &op, &rows, None);
    assert_eq!(view.summary, "release a share of web to laptop");
}
