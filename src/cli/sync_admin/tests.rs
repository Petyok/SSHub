use super::*;
use crate::app::tests::MemoryPasswordStore;
use crate::sync::crypto::Argon2Profile;
use crate::sync::oplog::{Attendance, DeviceKind, VerifiedBy};

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn ok(cmd: &str, a: &[&str]) -> Cmd {
    parse(cmd, &args(a)).unwrap().0
}

fn bad(cmd: &str, a: &[&str]) -> String {
    parse(cmd, &args(a)).unwrap_err()
}

struct Dev {
    store: LauncherStore,
    id: DeviceIdentity,
}

/// `n` holders that all know each other; device 0 is the proposer.
fn ring(n: usize) -> Vec<Dev> {
    let ids: Vec<DeviceIdentity> = (0..n).map(|_| DeviceIdentity::generate()).collect();
    let publics: Vec<_> = ids.iter().map(|i| i.public()).collect();
    ids.into_iter()
        .map(|id| {
            let store = LauncherStore::open_in_memory().unwrap();
            crate::sync::ceremony::test_force(&store, true);
            for (i, p) in publics.iter().enumerate() {
                store
                    .sync_register_device(
                        p,
                        &format!("dev{i}"),
                        DeviceRole::Holder,
                        Attendance::Attended,
                        DeviceKind::Terminal,
                        VerifiedBy::Sas,
                    )
                    .unwrap();
            }
            Dev { store, id }
        })
        .collect()
}

fn with_paper(store: &LauncherStore) {
    let phrase = crate::sync::crypto::bip39_encode(&[0x42; 32]);
    crate::sync::quorum::paper_setup(store, &phrase).unwrap();
}

#[test]
fn quorum_mode_needs_a_registered_paper() {
    let r = ring(1);
    let e = mode(
        &r[0].store,
        &r[0].id,
        CredMode::Quorum,
        None,
        "w",
        &[0; 16],
        1_000,
    )
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("register the paper phrase first"),
        "{e:#}"
    );
    assert!(crate::sync::quorum::ops_of(&r[0].store, "ModeChange", None)
        .unwrap()
        .is_empty());
}

#[test]
fn third_holder_needs_a_registered_paper() {
    let r = ring(2);
    let m = DeviceIdentity::generate();
    r[0].store
        .sync_register_device(
            &m.public(),
            "mirror",
            DeviceRole::Mirror,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    let id = m.public().device_id().to_string();
    let e = role(&r[0].store, &r[0].id, &id[..10], DeviceRole::Holder, 1_000).unwrap_err();
    assert!(
        format!("{e:#}").contains("register the paper phrase first"),
        "{e:#}"
    );
    with_paper(&r[0].store);
    role(&r[0].store, &r[0].id, &id[..10], DeviceRole::Holder, 1_000).unwrap();
}

#[test]
fn every_verb_parses_its_synopsis() {
    assert_eq!(ok("sync-revoke", &["abc"]), Cmd::Revoke("abc".into()));
    assert_eq!(
        ok("sync-role", &["abc", "mirror"]),
        Cmd::Role("abc".into(), DeviceRole::Mirror)
    );
    assert_eq!(
        ok("sync-approve", &["abcdef"]),
        Cmd::Approve("abcdef".into())
    );
    assert_eq!(
        ok("sync-factor", &["abcdef", "--paper"]),
        Cmd::Factor("abcdef".into(), FactorKind::Paper)
    );
    assert_eq!(
        ok("sync-factor", &["abcdef", "--email-code"]),
        Cmd::Factor("abcdef".into(), FactorKind::Email)
    );
    assert_eq!(
        ok("sync-factor-register", &["email", "--sender", "x"]),
        Cmd::FactorRegister(FactorKind::Email, Some("x".into()))
    );
    assert_eq!(
        ok("sync-recovery", &["start", "--paper"]),
        Cmd::Recovery(RecoveryCmd::Start { paper: true })
    );
    assert_eq!(
        ok("sync-recovery", &["approve", "abcdef"]),
        Cmd::Recovery(RecoveryCmd::Approve("abcdef".into()))
    );
    assert_eq!(
        ok("sync-recovery", &["ack", "abcdef"]),
        Cmd::Recovery(RecoveryCmd::Ack("abcdef".into()))
    );
    assert_eq!(
        ok("sync-recovery", &["cancel", "abcdef", "--factor", "paper"]),
        Cmd::Recovery(RecoveryCmd::Cancel {
            op: "abcdef".into(),
            factor: FactorKind::Paper
        })
    );
    assert_eq!(
        ok("sync-recovery", &["status", "--format", "json"]),
        Cmd::Recovery(RecoveryCmd::Status)
    );
    assert_eq!(
        ok("sync-mode", &["quorum"]),
        Cmd::Mode(CredMode::Quorum, None)
    );
    assert_eq!(
        ok("sync-exemption", &["off", "d"]),
        Cmd::Exemption {
            on: false,
            device: "d".into()
        }
    );
    assert_eq!(
        ok("sync-export", &["--window", "60", "--out", "/tmp/x"]),
        Cmd::Export {
            window: 60,
            out: "/tmp/x".into(),
            wait: 0
        }
    );
    assert_eq!(
        ok("sync-migrate", &["--remove-local"]),
        Cmd::Migrate { remove_local: true }
    );
    assert_eq!(ok("sync-import-kit", &[]), Cmd::ImportKit);
    assert!(parse("sync-status", &[]).is_err());
}

#[test]
fn bad_arguments_are_named() {
    assert!(bad("sync-revoke", &[]).contains("one device"));
    assert!(bad("sync-role", &["a", "boss"]).contains("holder = full device"));
    assert!(bad("sync-factor", &["abcdef"]).contains("exactly one"));
    assert!(bad("sync-factor", &["abcdef", "--paper", "--email-code"]).contains("exactly one"));
    assert!(bad("sync-recovery", &["cancel", "abcdef"]).contains("--factor"));
    assert!(bad("sync-recovery", &[]).contains("needs"));
    assert!(bad("sync-export", &["--out", "x"]).contains("--window"));
    assert!(bad("sync-export", &["--window", "0", "--out", "x"]).contains("1 to 3600"));
    assert!(bad("sync-export", &["--window", "9999", "--out", "x"]).contains("1 to 3600"));
    assert!(bad("sync-export", &["--window", "5"]).contains("--out"));
    assert!(bad("sync-revoke", &["a", "--format", "xml"]).contains("unknown format"));
    assert!(bad("sync-revoke", &["a", "--paper"]).contains("does not apply"));
    assert!(bad("sync-migrate", &["--bogus"]).contains("unknown option"));
}

#[test]
fn a_role_change_is_proposed_and_a_second_holder_votes() {
    let r = ring(3);
    let target = r[1].id.public().device_id();
    let rep = role(
        &r[0].store,
        &r[0].id,
        &target.to_string()[..10],
        DeviceRole::Mirror,
        1_000,
    )
    .unwrap();
    assert!(
        rep.lines.iter().any(|l| l.starts_with("approvals:")),
        "{rep:?}"
    );
    let op_hex = rep.json["op"].as_str().unwrap().to_string();
    // The proposer cannot be its own second device.
    let own = approve(&r[0].store, &r[0].id, &op_hex[..12], 1_000).unwrap_err();
    assert!(format!("{own:#}").contains("another one"));
    // Another holder receives the op, then votes.
    let proposed = find_op(&r[0].store, &op_hex[..12]).unwrap();
    r[2].store.sync_ingest(proposed).unwrap();
    let rep = approve(&r[2].store, &r[2].id, &op_hex[..12], 1_001).unwrap();
    assert!(
        rep.json["ceremony"]["have"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "second device"),
        "{rep:?}"
    );
}

#[test]
fn lookups_refuse_unknown_and_short_ids() {
    let r = ring(2);
    assert!(find_device(&r[0].store, "zzzz").is_err());
    assert!(find_op(&r[0].store, "abc")
        .unwrap_err()
        .to_string()
        .contains("6 hex"));
    assert!(find_op(&r[0].store, "abcdef").is_err());
}

#[test]
fn the_export_waits_for_its_ceremony_and_writes_nothing_without_it() {
    let r = ring(2);
    let out = tempfile::tempdir().unwrap().keep().join("L00t.csv");
    let err = export_flow(&r[0].store, &r[0].id, 60, &out, 0, &|| 1_000, &mut |_| {
        Ok(Some("pw".into()))
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("not approved yet"), "{err:#}");
    assert!(!out.exists());
}

#[test]
fn migrating_writes_replicated_credentials_and_remove_local_deletes_the_copies() {
    let r = ring(1);
    let mut new = crate::store::NewHost::launcher("web", "web.example");
    new.has_password = true;
    let host = r[0].store.create_host(&new).unwrap().id;
    let key = crate::credentials::host_key(host);
    let cheap = Argon2Profile {
        mem_kib: 8,
        iters: 1,
    };
    crate::sync::quorum::vault_setup_with(
        &r[0].store,
        "abacus abdomen abdominal abide abiding ability",
        &[7u8; 16],
        cheap,
    )
    .unwrap();
    let local = MemoryPasswordStore::default();
    local.set(&key, "pw").unwrap();
    let kept = migrate(&r[0].store, &r[0].id, &local, false).unwrap();
    assert_eq!(kept.json["migrated"], 1);
    assert_eq!(local.get(&key).unwrap().as_deref(), Some("pw"));
    let gone = migrate(&r[0].store, &r[0].id, &local, true).unwrap();
    assert_eq!(gone.json["removed_local"], true);
    assert!(local.get(&key).unwrap().is_none());
}

#[test]
fn the_kit_sets_up_the_vault_and_keeps_the_ring_secret() {
    let r = ring(1);
    let local = MemoryPasswordStore::default();
    let kit = crate::sync::quorum::ring_secret_text(&[7u8; 16]);
    let cheap = Argon2Profile {
        mem_kib: 8,
        iters: 1,
    };
    let words = "abacus abdomen abdominal abide abiding ability";
    assert!(import_kit(&r[0].store, &local, "sshub:ring:9:00", words, cheap).is_err());
    assert!(import_kit(&r[0].store, &local, &kit, "not six words", cheap).is_err());
    import_kit(&r[0].store, &local, &kit, words, cheap).unwrap();
    assert_eq!(
        local
            .get(crate::sync::runtime::RING_SECRET_KEY)
            .unwrap()
            .as_deref(),
        Some(kit.as_str())
    );
}

#[test]
fn quorum_mode_needs_no_credentials_to_open_and_then_flips() {
    let r = ring(1);
    with_paper(&r[0].store);
    let rep = mode(
        &r[0].store,
        &r[0].id,
        CredMode::Quorum,
        None,
        "w",
        &[0; 16],
        1_000,
    )
    .unwrap();
    assert!(rep.lines[0].contains("proposed"), "{rep:?}");
    assert_eq!(r[0].store.sync_cred_mode().unwrap(), CredMode::Quorum);
    // After the ceremony (the fixture counts it as complete) the second call re-encrypts.
    let rep = mode(
        &r[0].store,
        &r[0].id,
        CredMode::Quorum,
        None,
        "w",
        &[0; 16],
        1_000,
    )
    .unwrap();
    assert!(rep.lines[0].contains("0 credentials"), "{rep:?}");
    // From the command line the way back cannot open anything.
    assert!(mode(
        &r[0].store,
        &r[0].id,
        CredMode::Vault,
        None,
        "w",
        &[0; 16],
        1_000
    )
    .is_err());
}

#[test]
fn an_exemption_that_is_not_set_cannot_be_switched_off() {
    let r = ring(2);
    let d = r[1].id.public().device_id().to_string();
    assert!(exemption_off(&r[0].store, &r[0].id, &d[..10], 1_000)
        .unwrap_err()
        .to_string()
        .contains("no master phrase exemption"));
}

#[test]
fn the_emailed_code_factor_is_registered_only_on_its_sender() {
    let d = ring(2);
    // dev1 is named as the sender but dev0 registers: refused, and dev0 keeps no code.
    let err = factor_register(
        &d[0].store,
        &d[0].id,
        FactorKind::Email,
        None,
        Some("dev1"),
        1_000,
    )
    .unwrap_err();
    assert!(err.to_string().contains("ON its sender"), "{err}");
    assert!(
        crate::sync::quorum::meta_get(&d[0].store, "email_factor_code")
            .unwrap()
            .is_none()
    );
    // On the sender itself it works, and the code is not in clear in the database.
    factor_register(&d[0].store, &d[0].id, FactorKind::Email, None, None, 1_000).unwrap();
    assert!(
        crate::sync::quorum::meta_get(&d[0].store, "email_factor_code")
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_repeated_export_continues_the_open_proposal() {
    let r = ring(2);
    let run = || {
        export_flow(
            &r[0].store,
            &r[0].id,
            60,
            std::path::Path::new("/nonexistent/x.csv"),
            0,
            &|| 1_000,
            &mut |_| Ok(None),
        )
        .unwrap_err()
        .to_string()
    };
    run();
    run();
    let n = crate::sync::quorum::ops_of(&r[0].store, "BulkExport", None)
        .unwrap()
        .len();
    assert_eq!(n, 1, "the second command must not start a second ceremony");
}

#[test]
fn the_alert_is_one_per_requester_with_a_count() {
    let r = ring(3);
    let mut ops = Vec::new();
    for (i, n) in [(1usize, 1u8), (1, 2), (2, 3)] {
        ops.push(
            r[i].store
                .sync_append_local(
                    &r[i].id,
                    OpBody::HostUpsert {
                        host_uid: [n; 16],
                        fields: Default::default(),
                    },
                )
                .unwrap(),
        );
    }
    let view = alert_view(&r[0].store, &ops, 1_000).unwrap().unwrap();
    assert_eq!(view.requesters, vec![("dev1".to_string(), 2), ("dev2".to_string(), 1)]);
    assert_eq!(view.total, 3);
    let lines = alert_lines(&view, 1_000).join(" ");
    assert!(lines.contains("3 open") && lines.contains("dev1 (2)"), "{lines}");
    assert!(alert_view(&r[0].store, &[], 1_000).unwrap().is_none());
}
