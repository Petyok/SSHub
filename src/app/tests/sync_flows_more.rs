//! The exemption, bulk export and paper sheet flows of the Sync admin menu,
//! with the real worker on a Quorum ring (`sync_paths::ring`).

use super::sync_paths::{ingest_from, mark_this_device, pump, ring, type_and_enter, Ring};
use super::*;
use crate::credentials::{host_key, FilePasswordStore, PasswordStore};
use crate::sync::ceremony::{
    factor_register_paper, red_approve_body, red_factor_paper, red_phone_sign_body, settle_red_ops,
};
use crate::sync::crypto::{bip39_encode, eff_encode, paper_derive, PaperDomain};
use crate::sync::quorum::ops_of;
use crate::sync::quorum::tests::append;

fn notice(app: &App) -> String {
    app.host_notice.clone().unwrap_or_default()
}

fn info_lines(app: &App) -> Vec<String> {
    match &app.sync_ui {
        Some(crate::app::sync_ui::SyncUi::Info { lines, .. }) => lines.clone(),
        _ => Vec::new(),
    }
}

// --------------------------------------------------------------- exemption

#[test]
fn enabling_the_exemption_warns_validates_the_phrase_and_proposes_a_red_op() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    crate::app::sync_flows_more::exemption_on(&mut r.app);
    assert_eq!(r.app.mode, AppMode::SyncPrompt);
    match &r.app.sync_ui {
        Some(crate::app::sync_ui::SyncUi::Prompt { lines, masked, .. }) => {
            assert!(*masked);
            let text = lines.join(" ");
            assert!(text.contains("ALONE"), "{text}");
            assert!(text.contains("EVERY"), "{text}");
            assert!(text.contains("trust session"), "{text}");
            assert!(text.contains("Vault mode"), "{text}");
        }
        _ => panic!("a prompt with the warning"),
    }
    type_and_enter(&mut r.app, "not a phrase");
    assert!(notice(&r.app).contains("nothing proposed"));
    assert!(ops_of(&r.app.store, "ExemptionSet", None)
        .unwrap()
        .is_empty());

    let phrase = eff_encode(&[3u8; 10]).unwrap();
    crate::app::sync_flows_more::exemption_on(&mut r.app);
    type_and_enter(&mut r.app, &phrase);
    pump(&mut r.app, "the ceremony panel", |a| {
        a.mode == AppMode::SyncInfo
    });
    let lines = info_lines(&r.app).join("\n");
    assert!(lines.contains("ceremony: have"), "{lines}");
    assert!(lines.contains("second device"), "{lines}");
    let ops = ops_of(&r.app.store, "ExemptionSet", None).unwrap();
    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0].body,
        crate::sync::oplog::OpBody::ExemptionSet { slot: Some(_), .. }
    ));
}

#[test]
fn the_exemption_is_refused_outside_a_quorum_ring_and_off_an_unattended_machine() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    r.app.sync.cred_mode = Some(crate::app::sync::CredModeView::Vault);
    crate::app::sync_flows_more::exemption_on(&mut r.app);
    assert!(notice(&r.app).contains("Quorum"));
    assert_ne!(r.app.mode, AppMode::SyncPrompt);

    r.app.sync.cred_mode = Some(crate::app::sync::CredModeView::Quorum);
    for d in r.app.sync.devices.iter_mut().filter(|d| d.this) {
        d.unattended = true;
    }
    r.app.host_notice = None;
    crate::app::sync_flows_more::exemption_on(&mut r.app);
    assert!(notice(&r.app).contains("attended holder terminal"));
    assert_ne!(r.app.mode, AppMode::SyncPrompt);
}

#[test]
fn switching_the_exemption_off_lists_only_exempt_devices() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    crate::app::sync_flows_more::exemption_off(&mut r.app);
    assert!(notice(&r.app).contains("no device has"));
    assert_ne!(r.app.mode, AppMode::SyncMenu);

    let other = r.app.sync.devices.iter_mut().find(|d| !d.this).unwrap();
    other.exempt = true;
    let name = other.name.clone();
    crate::app::sync_flows_more::exemption_off(&mut r.app);
    assert_eq!(r.app.mode, AppMode::SyncMenu);
    match &r.app.sync_ui {
        Some(crate::app::sync_ui::SyncUi::Menu { items, .. }) => {
            assert_eq!(items.len(), 1);
            assert!(items[0].0.contains(&name));
        }
        _ => panic!("a menu"),
    }
}

// ------------------------------------------------------------ bulk export

fn start_export(r: &mut Ring, window: &str, out: &std::path::Path) {
    crate::app::sync_flows_more::export_start(&mut r.app);
    type_and_enter(&mut r.app, window);
    if r.app.mode == AppMode::SyncPrompt {
        type_and_enter(&mut r.app, &out.to_string_lossy());
    }
}

#[test]
fn an_export_proposal_reports_the_missing_proofs_and_writes_nothing() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("export.csv");

    // A window over the cap is refused before anything is proposed.
    start_export(&mut r, "3601", &out);
    assert!(notice(&r.app).contains("1 to 3600"));
    assert!(ops_of(&r.app.store, "BulkExport", None).unwrap().is_empty());

    // So is a path that exists.
    std::fs::write(dir.path().join("taken.csv"), "x").unwrap();
    start_export(&mut r, "", &dir.path().join("taken.csv"));
    assert!(notice(&r.app).contains("already exists"));
    assert!(ops_of(&r.app.store, "BulkExport", None).unwrap().is_empty());

    start_export(&mut r, "", &out);
    pump(&mut r.app, "the ceremony panel", |a| {
        a.mode == AppMode::SyncInfo
    });
    let lines = info_lines(&r.app).join("\n");
    assert!(lines.contains("bulk export proposed"), "{lines}");
    assert!(lines.contains("ceremony: have"), "{lines}");
    assert!(lines.contains("choose \"Export every credential\" again"), "{lines}");
    let ops = ops_of(&r.app.store, "BulkExport", None).unwrap();
    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0].body,
        crate::sync::oplog::OpBody::BulkExport {
            window_secs: 600,
            ..
        }
    ));
    assert!(!out.exists());

    // Continuing before the other devices approved says the same and writes nothing.
    r.app.handle_key(key(KeyCode::Esc)).unwrap();
    crate::app::sync_flows_more::export_start(&mut r.app);
    type_and_enter(&mut r.app, &out.to_string_lossy());
    pump(&mut r.app, "the status panel", |a| {
        a.mode == AppMode::SyncInfo
    });
    assert!(info_lines(&r.app).join("\n").contains("not complete yet"));
    assert!(!out.exists());
}

/// Register a paper factor through a full ceremony (phone, this app as the
/// second device) and return the paper's recover key.
fn register_paper(r: &Ring) -> crate::sync::crypto::DeviceIdentity {
    let words = bip39_encode(&[7u8; 32]);
    let recover = paper_derive(&words, PaperDomain::Recover).unwrap();
    let proposal = append(&r.d[2], factor_register_paper(&words).unwrap());
    ingest_from(&r.d[2], &r.d[1]);
    ingest_from(&r.d[2], &r.d[0]);
    append(&r.d[1], red_approve_body(&proposal));
    append(&r.d[0], red_phone_sign_body(&proposal));
    ingest_from(&r.d[0], &r.d[1]);
    let report = settle_red_ops(&r.app.store, crate::store::now_ts()).unwrap();
    assert_eq!(report.applied, vec![proposal.hash()]);
    recover
}

#[test]
fn a_completed_export_writes_the_open_credentials_and_lists_the_locked_ones() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = ring(None);
    mark_this_device(&mut r);
    let recover = register_paper(&r);

    // `web` is a ring credential and stays locked; `db` has a local password.
    let row = r
        .app
        .store
        .create_host(&crate::store::NewHost::launcher("db", "10.0.0.2"))
        .unwrap();
    FilePasswordStore::new(r.paths.credentials_file())
        .set(&host_key(row.id), "s3cret")
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("export.csv");
    start_export(&mut r, "900", &out);
    pump(&mut r.app, "the ceremony panel", |a| {
        a.mode == AppMode::SyncInfo
    });
    assert!(!out.exists());
    let op = ops_of(&r.app.store, "BulkExport", None).unwrap().remove(0);

    // The other devices approve: the phone signs, the holder approves, the paper proves.
    ingest_from(&r.d[1], &r.d[0]);
    ingest_from(&r.d[1], &r.d[2]);
    append(&r.d[0], red_phone_sign_body(&op));
    append(&r.d[2], red_approve_body(&op));
    append(&r.d[0], red_factor_paper(&recover, &op));
    ingest_from(&r.d[0], &r.d[1]);
    ingest_from(&r.d[2], &r.d[1]);

    r.app.handle_key(key(KeyCode::Esc)).unwrap();
    crate::app::sync_flows_more::export_start(&mut r.app);
    type_and_enter(&mut r.app, &out.to_string_lossy());
    pump(&mut r.app, "the export", |a| out_done(a, &out));
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(csv.contains("db,10.0.0.2,22,,s3cret,"), "{csv}");
    assert!(!csv.contains("trusted"), "{csv}");
    assert_eq!(
        std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let lines = info_lines(&r.app).join("\n");
    assert!(lines.contains("exported 1 credentials"), "{lines}");
    assert!(lines.contains("locked, not exported: web"), "{lines}");

    // The file is never overwritten: a second run to the same path fails.
    r.app.handle_key(key(KeyCode::Esc)).unwrap();
    crate::app::sync_flows_more::export_start(&mut r.app);
    type_and_enter(&mut r.app, &out.to_string_lossy());
    assert!(notice(&r.app).contains("already exists"));
}

fn out_done(app: &App, out: &std::path::Path) -> bool {
    out.exists() && app.mode == AppMode::SyncInfo
}

#[test]
fn an_export_another_device_proposed_is_not_continued_here() {
    let mut r = ring(None);
    mark_this_device(&mut r);
    // The op exists but another device proposed it.
    append(
        &r.d[2],
        crate::sync::oplog::OpBody::BulkExport {
            requester: r.d[2].did.clone(),
            window_secs: 60,
        },
    );
    ingest_from(&r.d[2], &r.d[1]);
    crate::app::sync_flows_more::export_start(&mut r.app);
    // A new export is started (it asks for a window); nothing of the other device's is continued.
    match &r.app.sync_ui {
        Some(crate::app::sync_ui::SyncUi::Prompt { label, .. }) => {
            assert!(label.starts_with("Window in seconds"), "{label}")
        }
        _ => panic!("the window prompt is not open"),
    }
}

// ------------------------------------------------------------- paper sheet

fn all_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            all_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn the_paper_sheet_needs_the_words_in_memory() {
    let mut r = ring(None);
    crate::app::sync_flows_more::paper_sheet(&mut r.app);
    assert!(notice(&r.app).contains("create a paper phrase first"));
    assert_ne!(r.app.mode, AppMode::SyncPrompt);
}

#[test]
fn the_paper_sheet_is_one_private_file_that_is_never_overwritten_and_the_only_copy() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = ring(None);
    crate::app::sync_flows::paper_setup(&mut r.app);
    let words = r.app.sync_paper.clone().unwrap().to_string();
    r.app.handle_key(key(KeyCode::Esc)).unwrap();

    let sheet = r.paths.root.join("paper.html");
    crate::app::sync_flows_more::paper_sheet(&mut r.app);
    assert_eq!(r.app.mode, AppMode::SyncPrompt);
    type_and_enter(&mut r.app, &sheet.to_string_lossy());
    assert!(
        notice(&r.app).contains("sheet written"),
        "{}",
        notice(&r.app)
    );
    assert!(!notice(&r.app).contains(&words));

    let html = std::fs::read_to_string(&sheet).unwrap();
    assert!(html.contains(crate::app::kit::DO_NOT_PHOTOGRAPH));
    assert!(html.contains(crate::app::kit::SHEET_IS_THE_STORE));
    assert!(html.contains("Do not photograph"));
    assert!(html.contains("<ol"));
    for (i, w) in words.split_whitespace().enumerate() {
        assert!(html.contains(&format!("<li>{w}</li>")), "word {i}");
    }
    assert_eq!(html.matches("<li>").count(), 24);
    assert_eq!(
        std::fs::metadata(&sheet).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // A second sheet at the same path is refused and the first stays as it was.
    crate::app::sync_flows_more::paper_sheet(&mut r.app);
    type_and_enter(&mut r.app, &sheet.to_string_lossy());
    assert!(notice(&r.app).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&sheet).unwrap(), html);

    // The words are in no other file of the profile directory.
    let mut files = Vec::new();
    all_files(&r.paths.root, &mut files);
    let first_four = words
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    for f in files.into_iter().filter(|f| *f != sheet) {
        let bytes = std::fs::read(&f).unwrap();
        for needle in [words.as_bytes(), first_four.as_bytes()] {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle),
                "{} holds the paper words",
                f.display()
            );
        }
    }
}
