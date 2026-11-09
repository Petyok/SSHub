//! Old credentials after a device is removed or replaced (design 4) and the guided terminal replacement (design 3.3).
//!
//! * `heal`: credentials whose newest sharing lacks a current holder and that no removed device holds a share of are
//!   re-sealed under a fresh key to every current holder. The paper phrase is typed once; each credential opens with this
//!   terminal's own share plus the paper's share, so no phone round trip is needed.
//! * `rekey`: credentials a removed device still holds a share of cannot be healed (re-sealing leaves the old ciphertext in
//!   the log, which opens to the same secret). The secret itself must change; this saves the new one. The product cannot
//!   change a password on a server, and says so.
//! * `replace_terminal`: enroll the new computer, promote it, remove the old one, with the paper typed once.

use anyhow::{bail, ensure, Context, Result};
use serde_json::json;

use super::sync_admin::{self as admin, FactorKind, Report};
use crate::store::LauncherStore;
use crate::sync::cred_ops::{cred_put, heal_credential, heal_plan, CredPayload};
use crate::sync::crypto::DeviceIdentity;
use crate::sync::oplog::{DeviceRole, HostUid};
use crate::sync::quorum::{assemble_dek, exposed_credentials, own_shares, ExposureKind};
use crate::sync::unlock_paths::paper_second_share;

fn host_name(store: &LauncherStore, uid: &HostUid) -> String {
    admin::host_label(store, uid)
}

/// Open the credential of `host` on this terminal: its own share plus the paper's share, newest sharing first.
fn open_with_paper(
    store: &LauncherStore,
    id: &DeviceIdentity,
    host: &HostUid,
    phrase: &str,
) -> Result<CredPayload> {
    let (ciphertext, _) = store
        .sync_cred_ciphertext(host)?
        .context("the credential has no ciphertext")?;
    for (epoch, mine) in own_shares(store, id, host)? {
        let Ok(second) = paper_second_share(store, host, epoch, phrase) else {
            continue;
        };
        if let Ok(dek) = assemble_dek(store, host, mine, second) {
            let plain = crate::sync::crypto::dek_open(&dek, &ciphertext)?;
            return CredPayload::from_bytes(&plain);
        }
    }
    bail!("this computer holds no piece of it that opens with the paper phrase (run it on a computer that does, or check the words)")
}

/// Bulk heal, with the paper phrase typed once.
pub(crate) fn heal(store: &LauncherStore, id: &DeviceIdentity, phrase: &str) -> Result<Report> {
    let plan = heal_plan(store)?;
    let (mut lines, mut healed, mut skipped) = (Vec::new(), 0usize, Vec::new());
    for host in &plan {
        let name = host_name(store, host);
        match open_with_paper(store, id, host, phrase)
            .and_then(|p| heal_credential(store, id, host, &p))
        {
            Ok(_) => {
                healed += 1;
                lines.push(format!("healed: {name}"));
            }
            Err(e) => {
                skipped.push(name.clone());
                lines.push(format!("skipped: {name}: {e:#}"));
            }
        }
    }
    if plan.is_empty() {
        lines.push("nothing to heal: every saved password is shared with every current full device, or is on the list to change (sshub sync-rekey)".into());
    }
    Ok(Report {
        lines,
        json: json!({"healed": healed, "skipped": skipped, "planned": plan.len()}),
    })
}

/// The credentials that share a key with a removed device: `(host uid, host name, device names)`.
pub(crate) fn rekey_list(store: &LauncherStore) -> Result<Vec<(HostUid, String, Vec<String>)>> {
    let names: std::collections::HashMap<_, _> =
        crate::sync::ceremony::effective_devices(store, None)?
            .into_iter()
            .map(|d| (d.id, d.name))
            .collect();
    Ok(exposed_credentials(store)?
        .into_iter()
        .filter(|e| e.kind == ExposureKind::Removed)
        .map(|e| {
            let devs = e
                .devices
                .iter()
                .map(|d| {
                    names
                        .get(d)
                        .cloned()
                        .unwrap_or_else(|| d.chars().take(8).collect())
                })
                .collect();
            (e.host_uid, host_name(store, &e.host_uid), devs)
        })
        .collect())
}

/// The listed credential a `host` spec (name or hex prefix of the uid) names.
fn find_exposed(list: &[(HostUid, String, Vec<String>)], spec: &str) -> Result<HostUid> {
    let hits: Vec<_> = list
        .iter()
        .filter(|(uid, name, _)| {
            name == spec
                || (spec.len() >= 4 && crate::store::uid_hex(uid).starts_with(&spec.to_lowercase()))
        })
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.0),
        [] => bail!("not on the list of credentials to change"),
        _ => bail!("matches several credentials: use the id"),
    }
}

/// Save new secrets: `entries` are `(host, new password)`. Each host gets its own result line; a host that is not on the
/// list, or whose save fails, stays on the list. Passwords are never printed.
pub(crate) fn rekey_apply(
    store: &LauncherStore,
    id: &DeviceIdentity,
    entries: &[(String, String)],
) -> Result<Report> {
    let list = rekey_list(store)?;
    let (mut lines, mut saved) = (Vec::new(), 0usize);
    for (host, password) in entries {
        let result = find_exposed(&list, host).and_then(|uid| {
            ensure!(!password.is_empty(), "the new password is empty");
            let payload = CredPayload {
                password: Some(password.clone()),
                private_key: None,
            };
            cred_put(store, id, &uid, &payload).map(|_| ())
        });
        match result {
            Ok(()) => {
                saved += 1;
                lines.push(format!("saved: {host}"));
            }
            Err(e) => lines.push(format!("not saved: {host}: {e:#}")),
        }
    }
    lines.push("The new secret is saved in the ring. Passwords cannot be changed on a server by SSHub: change each one there too.".into());
    Ok(Report {
        lines,
        json: json!({"saved": saved, "requested": entries.len()}),
    })
}

/// `host,newpassword` lines (blank lines and `#` comments skipped; the password may contain commas).
pub(crate) fn parse_csv(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (host, password) = line
            .split_once(',')
            .with_context(|| format!("line {}: expected host,newpassword", n + 1))?;
        out.push((host.trim().to_string(), password.to_string()));
    }
    Ok(out)
}

/// Generate a new key for each listed host and save it as the credential. The public key is printed: this command does
/// not log in to servers, so installing it (and removing the old line from `authorized_keys`) is yours.
pub(crate) fn rekey_new_key(
    store: &LauncherStore,
    id: &DeviceIdentity,
    hosts: &[String],
) -> Result<Report> {
    let list = rekey_list(store)?;
    let dir = tempfile::tempdir().context("a private scratch directory")?;
    let (mut lines, mut saved) = (Vec::new(), 0usize);
    for (i, host) in hosts.iter().enumerate() {
        let result = (|| -> Result<String> {
            let uid = find_exposed(&list, host)?;
            let path = dir.path().join(format!("key{i}"));
            crate::ssh::keyfile::generate_key_pair("ed25519", None, "", "sshub-rekey", &path)?;
            let private = std::fs::read_to_string(&path)?;
            let public = crate::ssh::read_public_key(&path, None)?;
            let _ = std::fs::remove_file(&path);
            cred_put(
                store,
                id,
                &uid,
                &CredPayload {
                    password: None,
                    private_key: Some(private),
                },
            )?;
            Ok(public)
        })();
        match result {
            Ok(public) => {
                saved += 1;
                lines.push(format!("new key saved: {host}"));
                lines.push(format!(
                    "  install on the server, and remove the old line: {}",
                    public.trim()
                ));
            }
            Err(e) => lines.push(format!("no new key: {host}: {e:#}")),
        }
    }
    Ok(Report {
        lines,
        json: json!({"saved": saved, "requested": hosts.len()}),
    })
}

/// One step of "Replace this terminal" (design 3.3). The new computer must be enrolled first. Run again after the phone
/// confirmed the promotion to propose removing the old one. The paper phrase is typed once by the caller and goes to both
/// ceremonies. `revoke` writes the removal (the real one rotates keys, so the caller owns it).
pub(crate) fn replace_terminal(
    store: &LauncherStore,
    id: &mut DeviceIdentity,
    old: &str,
    new: Option<&str>,
    phrase: &str,
    now: i64,
    revoke: impl FnOnce(&mut DeviceIdentity, &str) -> Result<Report>,
) -> Result<Report> {
    let old_row = admin::find_device(store, old)?;
    ensure!(
        old_row.kind == crate::sync::oplog::DeviceKind::Terminal
            && old_row.role == DeviceRole::Holder,
        "{} is not a full computer: remove it with `sshub sync-revoke`",
        old_row.name
    );
    let Some(new) = new else {
        let candidates: Vec<String> = crate::sync::ceremony::effective_devices(store, None)?
            .into_iter()
            .filter(|d| d.id != old_row.id && d.kind == crate::sync::oplog::DeviceKind::Terminal)
            .filter(|d| {
                !matches!(
                    d.status,
                    crate::sync::oplog::DeviceStatus::Revoked
                        | crate::sync::oplog::DeviceStatus::Compromised
                )
            })
            .map(|d| d.name)
            .collect();
        return Ok(Report {
            lines: vec![
                "Step 1: enroll the new computer: on it run `sshub sync-join <a reachable computer of yours>`.".into(),
                format!("Enrolled computers besides {}: {}", old_row.name, if candidates.is_empty() { "none yet".into() } else { candidates.join(", ") }),
                "Then run this again with --new <that computer>.".into(),
            ],
            json: json!({"step": "enroll", "candidates": candidates}),
        });
    };
    let new_row = admin::find_device(store, new)?;
    ensure!(
        new_row.id != old_row.id,
        "the new computer is the one being replaced"
    );
    let mut lines = Vec::new();
    let (step, op);
    if new_row.role != DeviceRole::Holder {
        let r = admin::role(store, &*id, &new_row.id, DeviceRole::Holder, now)?;
        let hash = r.json["op"].as_str().context("no change id")?.to_string();
        lines.push(format!(
            "Step 2: {} is proposed as a full computer.",
            new_row.name
        ));
        lines.extend(r.lines);
        let f = admin::factor(store, &*id, &hash[..16], FactorKind::Paper, phrase, now)?;
        lines.push("The paper phrase is added. The phone's confirmation is still needed; when it is in, run this again.".into());
        lines.extend(f.lines);
        (step, op) = ("promote", hash);
    } else {
        let r = revoke(id, &old_row.id)?;
        let hash = r.json["op"].as_str().context("no change id")?.to_string();
        lines.push(format!("Step 3: removing {} is proposed.", old_row.name));
        lines.extend(r.lines);
        let f = admin::factor(store, &*id, &hash[..16], FactorKind::Paper, phrase, now)?;
        lines.push("The paper phrase is added. Confirm on the phone.".into());
        lines.extend(f.lines);
        lines.push("Step 4: afterwards run `sshub sync-heal` (the old computer is gone or retired). If it was stolen or lost, run `sshub sync-rekey` for the passwords it could read.".into());
        (step, op) = ("revoke", hash);
    }
    Ok(Report {
        lines,
        json: json!({"step": step, "op": op}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::crypto::bip39_encode;
    use crate::sync::oplog::{Attendance, DeviceKind, OpBody};
    use crate::sync::protocol::tests::{ring, Dev};
    use crate::sync::quorum::tests::{all_to_all, append, payload, quorum_mode};

    /// A Quorum ring of `n` holders with the paper registered everywhere and one saved credential `h`.
    fn world(n: usize, phrase: &str) -> (Vec<Dev>, HostUid) {
        let d = ring(n);
        quorum_mode(&d[0]);
        all_to_all(&d);
        for x in &d {
            crate::sync::quorum::paper_setup(&x.store, phrase).unwrap();
        }
        let h = [7u8; 16];
        cred_put(&d[0].store, &d[0].id, &h, &payload("old")).unwrap();
        all_to_all(&d);
        (d, h)
    }

    #[test]
    fn heal_re_seals_to_a_new_holder_with_the_paper_typed_once() {
        let phrase = bip39_encode(&[0x71; 32]);
        let (d, h) = world(4, &phrase);
        let fresh = Dev::new();
        d[0].trust_with(&fresh, Attendance::Attended, DeviceKind::Terminal);
        assert_eq!(heal_plan(&d[0].store).unwrap(), vec![h]);
        // Wrong words heal nothing and say why.
        let wrong = heal(&d[0].store, &d[0].id, &bip39_encode(&[0x72; 32])).unwrap();
        assert!(
            wrong.lines.iter().any(|l| l.starts_with("skipped")),
            "{wrong:?}"
        );
        assert_eq!(heal_plan(&d[0].store).unwrap(), vec![h]);
        let r = heal(&d[0].store, &d[0].id, &phrase).unwrap();
        assert!(r.lines.iter().any(|l| l.starts_with("healed")), "{r:?}");
        assert!(heal_plan(&d[0].store).unwrap().is_empty());
        let (_, wraps) = d[0].store.sync_cred_ciphertext(&h).unwrap().unwrap();
        assert!(wraps.iter().any(|w| w.recipient == fresh.did));
    }

    #[test]
    fn rekey_saves_a_new_secret_for_a_credential_a_removed_device_held_and_keeps_the_rest_listed() {
        let phrase = bip39_encode(&[0x73; 32]);
        let (d, h) = world(4, &phrase);
        append(
            &d[0],
            OpBody::DeviceRevoke {
                device_id: d[3].did.clone(),
            },
        );
        let list = rekey_list(&d[0].store).unwrap();
        assert_eq!(list.len(), 1, "{list:?}");
        // Heal refuses it: the secret itself must change.
        let healed = heal(&d[0].store, &d[0].id, &phrase).unwrap();
        assert!(
            healed.lines.iter().any(|l| l.contains("nothing to heal")),
            "{healed:?}"
        );
        let name = list[0].1.clone();
        let entries = vec![
            (name.clone(), "new,secret".to_string()),
            ("nope".to_string(), "x".to_string()),
        ];
        let r = rekey_apply(&d[0].store, &d[0].id, &entries).unwrap();
        assert!(
            r.lines.iter().any(|l| l == &format!("saved: {name}")),
            "{r:?}"
        );
        assert!(
            r.lines.iter().any(|l| l.starts_with("not saved: nope")),
            "{r:?}"
        );
        assert!(
            r.lines.iter().all(|l| !l.contains("new,secret")),
            "the secret is never printed"
        );
        assert!(
            rekey_list(&d[0].store).unwrap().is_empty(),
            "no longer shares a key with the removed device"
        );
        let _ = h;
    }

    #[test]
    fn the_csv_keeps_commas_in_the_password_and_skips_comments() {
        let rows = parse_csv("# my hosts\nweb,pa,ss\r\n\ndb , x\n").unwrap();
        assert_eq!(
            rows,
            vec![("web".into(), "pa,ss".into()), ("db".into(), " x".into())]
        );
        assert!(parse_csv("nocomma").is_err());
    }

    #[test]
    fn replace_terminal_goes_enroll_then_promote_then_remove_with_the_paper_added_to_both() {
        let phrase = bip39_encode(&[0x74; 32]);
        let mut d = ring(3);
        let store = d[0].store.clone();
        crate::sync::ceremony::test_force(&store, true);
        for x in &d {
            crate::sync::quorum::paper_setup(&x.store, &phrase).unwrap();
        }
        let fresh = Dev::new();
        store
            .sync_register_device(
                &fresh.id.public(),
                "new-laptop",
                DeviceRole::Mirror,
                Attendance::Attended,
                DeviceKind::Terminal,
                crate::sync::oplog::VerifiedBy::Sas,
            )
            .unwrap();
        let old = d[1].did.to_string()[..10].to_string();
        let now = crate::store::now_ts();
        let id = std::sync::Arc::get_mut(&mut d[0].id).unwrap();
        let revoke = |id: &mut DeviceIdentity, dev: &str| -> Result<Report> {
            let row = admin::find_device(&store, dev)?;
            let op = store.sync_append_local(
                id,
                OpBody::DeviceRevoke {
                    device_id: row.id.clone(),
                },
            )?;
            admin::op_report(&store, &op, "removal proposed".into(), now)
        };
        // Step 1: without --new it says how to enroll, and names the enrolled computers.
        let r = replace_terminal(&store, id, &old, None, &phrase, now, revoke).unwrap();
        assert_eq!(r.json["step"], "enroll");
        assert!(r.lines.iter().any(|l| l.contains("new-laptop")), "{r:?}");
        // Step 2: the new computer is promoted; the paper goes into that change.
        let r =
            replace_terminal(&store, id, &old, Some("new-laptop"), &phrase, now, revoke).unwrap();
        assert_eq!(r.json["step"], "promote", "{r:?}");
        assert!(
            r.lines.iter().any(|l| l.contains("paper phrase is added")),
            "{r:?}"
        );
        assert_eq!(
            crate::sync::quorum::ops_of(&store, "RedFactor", None)
                .unwrap()
                .len(),
            1
        );
        // Step 3, once the promotion is in force: the old computer's removal gets the paper too.
        let r =
            replace_terminal(&store, id, &old, Some("new-laptop"), &phrase, now, revoke).unwrap();
        assert_eq!(r.json["step"], "revoke", "{r:?}");
        assert!(r.lines.iter().any(|l| l.contains("sync-heal")), "{r:?}");
        assert_eq!(
            crate::sync::quorum::ops_of(&store, "RedFactor", None)
                .unwrap()
                .len(),
            2
        );
    }
}
