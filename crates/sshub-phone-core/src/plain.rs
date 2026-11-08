//! One plain sentence per op, built from the received bytes and the names this phone knows, in the words of
//! `docs/host-sync-glossary.md` (full device, view-only copy, the three approvals, trust session, saved password, piece).
//! Nothing here names an op, a lamport, a seq or an epoch: it is what a person decides on; the rows under it are the
//! technical details.

use std::collections::BTreeMap;

use sshub_sync_proto::oplog::{CertScope, CredMode, DeviceRole, Op, OpBody, ThirdFactor};
use sshub_sync_proto::wire::{CertExtendRequestWire, CertRequestWire};

use crate::describe::grouped_host_uid;
use crate::text::{iso, safe_text};

/// A device as a person reads it: its name, or "a device you have not met (id)".
fn dev(aliases: &BTreeMap<String, String>, id: &str) -> String {
    match aliases.get(id).filter(|n| !n.is_empty()) {
        Some(n) => safe_text(n),
        None => format!("a device you have not met ({})", safe_text(id)),
    }
}

fn day(t: i64) -> String {
    iso(t).chars().take(10).collect()
}

fn hosts_phrase(n: usize) -> String {
    format!("{n} host{}", if n == 1 { "" } else { "s" })
}

fn scope_phrase(scope: &CertScope) -> String {
    match scope {
        CertScope::Group(_) => "the hosts of a group".to_string(),
        CertScope::Hosts(h) => hosts_phrase(h.len()),
    }
}

fn minutes(secs: u32) -> String {
    let m = secs / 60;
    if m >= 60 && m.is_multiple_of(60) {
        format!("{} hour{}", m / 60, if m == 60 { "" } else { "s" })
    } else {
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    }
}

/// The sentence for a received op. `by` is the device named as the author, shown as "Asked by".
pub fn plain_op(op: &Op, aliases: &BTreeMap<String, String>) -> String {
    let by = dev(aliases, &op.device_id);
    let asked = |s: String| format!("{s} Asked by {by}.");
    match &op.body {
        OpBody::DeviceAdd { name, role, .. } => match role {
            DeviceRole::Holder => asked(format!(
                "Make {} a full device: it will hold one piece of every saved password in Quorum mode and can approve changes.",
                safe_text(name)
            )),
            DeviceRole::Mirror => asked(format!(
                "Add {} as a view-only copy: it gets your host list and encrypted data, but no piece of any password.",
                safe_text(name)
            )),
        },
        OpBody::DeviceRevoke { device_id } => asked(format!(
            "Remove {} from your devices: it stops syncing and the saved passwords are split again without it.",
            dev(aliases, device_id)
        )),
        OpBody::DeviceRoleChange { device_id, to } => match to {
            DeviceRole::Holder => asked(format!(
                "Make {} a full device: it will hold one piece of every saved password in Quorum mode and can approve changes.",
                dev(aliases, device_id)
            )),
            DeviceRole::Mirror => asked(format!(
                "Make {} a view-only copy: it will no longer hold a piece of any saved password or approve changes.",
                dev(aliases, device_id)
            )),
        },
        OpBody::ModeChange { to, .. } => match to {
            CredMode::Quorum => asked(
                "Switch to Quorum: opening a saved password will need two of your devices to agree, and the master phrase alone will no longer open it."
                    .to_string(),
            ),
            CredMode::Vault => asked(
                "Switch to Vault: the kit and the six-word master phrase will open saved passwords on any computer, and your devices will no longer split them."
                    .to_string(),
            ),
        },
        OpBody::ExemptionSet { device_id, slot } => match slot {
            Some(_) => asked(format!(
                "Let {} open saved passwords with the master phrase in Quorum mode (advanced): anyone with the phrase and that device can open every password alone.",
                dev(aliases, device_id)
            )),
            None => asked(format!(
                "Stop letting {} open saved passwords with the master phrase.",
                dev(aliases, device_id)
            )),
        },
        OpBody::BulkExport { requester, window_secs } => asked(format!(
            "Let {} export your hosts and saved passwords for {}.",
            dev(aliases, requester),
            minutes(*window_secs)
        )),
        OpBody::FactorRegister { factor, .. } => match factor {
            ThirdFactor::Paper => asked(
                "Register a paper phrase: the 24 words on paper become the third approval for big changes.".to_string(),
            ),
            ThirdFactor::EmailCode => asked(
                "Register an emailed code as the third approval for big changes.".to_string(),
            ),
        },
        OpBody::CertIssue {
            device_id,
            scope,
            expiry,
            max_new_hosts_per_day,
            ..
        } => asked(format!(
            "Let {} open {} without asking until {}, up to {} new host{} a day.",
            dev(aliases, device_id),
            scope_phrase(scope),
            day(*expiry),
            max_new_hosts_per_day,
            if *max_new_hosts_per_day == 1 { "" } else { "s" }
        )),
        OpBody::CertRevoke { .. } => asked(
            "End a trust session: that computer will have to ask for every host again.".to_string(),
        ),
        OpBody::CertExtend { extra_hosts, .. } => asked(format!(
            "Allow {} more new host{} today on a trust session.",
            extra_hosts,
            if *extra_hosts == 1 { "" } else { "s" }
        )),
        OpBody::CertRenew { expiry, .. } => asked(format!(
            "Renew a trust session: it will run until {}.",
            day(*expiry)
        )),
        OpBody::PhoneHandover { name, .. } => asked(format!(
            "Hand this phone's place over to {}: this phone stops being a device of yours.",
            safe_text(name)
        )),
        OpBody::CertHostOpened { .. } => "A trust session opened a host.".to_string(),
        OpBody::PhoneReenroll { .. } => asked(
            "Replace this phone with a new one. After a 7 day wait the new phone takes over, unless you cancel from this phone."
                .to_string(),
        ),
        OpBody::PhoneReenrollCancel { .. } => asked("Cancel the replacement of this phone.".to_string()),
        OpBody::RecoveryApprove { .. } => asked("Approve replacing the phone.".to_string()),
        OpBody::RecoveryAck { .. } => asked("Confirm that the phone replacement was seen.".to_string()),
        OpBody::ShareRelease {
            host_uid,
            requester,
            ..
        } => format!(
            "Give {} your piece of the saved password of host {}.",
            dev(aliases, requester),
            grouped_host_uid(&hex::encode(host_uid))
        ),
        OpBody::ShareGrant { host_uid, requester, .. } => format!(
            "Hand a piece of the saved password of host {} to {}.",
            grouped_host_uid(&hex::encode(host_uid)),
            dev(aliases, requester)
        ),
        OpBody::ShareReshareSession { .. }
        | OpBody::ShareReshareBatch { .. }
        | OpBody::ShareReshareConfirmBatch { .. }
        | OpBody::ShareStorageKey { .. } => asked(
            "Update the shares: split the saved passwords again among your current full devices.".to_string(),
        ),
        OpBody::WrapKeyRotate { .. } => asked("Renew this phone's key set.".to_string()),
        OpBody::CredUpdate { .. } => asked("Replace a saved password.".to_string()),
        OpBody::HostUpsert { .. } => asked("Add or change a host entry.".to_string()),
        OpBody::HostDelete { .. } => asked("Delete a host entry.".to_string()),
        OpBody::GroupUpsert { .. } => asked("Add or change a group.".to_string()),
        OpBody::GroupDelete { .. } => asked("Delete a group.".to_string()),
        OpBody::IdentityUpsert { .. } => asked("Add or change a key or login.".to_string()),
        OpBody::IdentityDelete { .. } => asked("Delete a key or login.".to_string()),
        OpBody::HeadAttest { .. } => format!("{by} recorded where its history stands."),
        OpBody::RedApprove { .. } => asked("Approve a big change.".to_string()),
        OpBody::RedPhoneSign { .. } => asked("The phone approved a big change.".to_string()),
        OpBody::RedFactor { .. } => asked("The paper phrase or emailed code approved a big change.".to_string()),
        OpBody::CheckpointPropose { .. } | OpBody::CheckpointSign { .. } => {
            asked("Tidy up the change log.".to_string())
        }
    }
}

/// The sentence for a trust session request (kind 9). The terminal that sends it is the one asking.
pub fn plain_cert_request(req: &CertRequestWire, aliases: &BTreeMap<String, String>) -> String {
    format!(
        "Let {} open {} without asking until {}, up to {} new host{} a day.",
        dev(aliases, &hex::encode(req.device_id)),
        scope_phrase(&req.scope),
        day(req.expiry),
        req.max_new_hosts_per_day,
        if req.max_new_hosts_per_day == 1 {
            ""
        } else {
            "s"
        }
    )
}

/// The sentence for a budget extension (kind 10), with the holder's name when this phone issued the session.
pub fn plain_cert_extend(
    req: &CertExtendRequestWire,
    holder: Option<&str>,
    aliases: &BTreeMap<String, String>,
) -> String {
    let n = req.extra_hosts;
    let s = if n == 1 { "" } else { "s" };
    match holder {
        Some(id) => format!(
            "Allow {n} more new host{s} today for the trust session of {}.",
            dev(aliases, id)
        ),
        None => format!("Allow {n} more new host{s} today on a trust session."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::describe::parse_op;

    fn vectors() -> serde_json::Value {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/companion/test/fixtures/ops.vectors.json"
        );
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn plain_of(id: &str, aliases: &BTreeMap<String, String>) -> String {
        let v = vectors();
        let x = v["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .unwrap_or_else(|| panic!("{id}"));
        let c = hex::decode(x["canonical_hex"].as_str().unwrap()).unwrap();
        let s: [u8; 64] = hex::decode(x["signature_hex"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        plain_op(&parse_op(&c, s).unwrap(), aliases)
    }

    #[test]
    fn every_op_vector_has_a_plain_sentence_without_internals() {
        let v = vectors();
        let none = BTreeMap::new();
        for x in v["vectors"].as_array().unwrap() {
            let id = x["id"].as_str().unwrap();
            let s = plain_of(id, &none);
            assert!(!s.is_empty() && s.ends_with('.'), "{id}: {s}");
            for bad in [
                "lamport",
                "seq",
                "epoch",
                "DeviceAdd",
                "HeadAttest",
                "Upsert",
                "ShareRelease",
                "Red op",
            ] {
                assert!(!s.contains(bad), "{id}: {s}");
            }
        }
    }

    #[test]
    fn the_sentences_name_the_people_and_the_decision() {
        let mut names = BTreeMap::new();
        names.insert("723ed85f0566df0c".to_string(), "laptop-2".to_string());
        let holder = plain_of("device-add-holder", &names);
        assert!(holder.starts_with("Make "), "{holder}");
        assert!(
            holder.contains("full device") && holder.contains("one piece of every saved password"),
            "{holder}"
        );
        assert!(holder.ends_with("Asked by laptop-2."), "{holder}");
        let mirror = plain_of("device-add-mirror", &names);
        assert!(
            mirror.contains("view-only copy") && mirror.contains("no piece of any password"),
            "{mirror}"
        );
        // an unknown device is "a device you have not met" with its id
        let anon = plain_of("device-add-holder", &BTreeMap::new());
        assert!(
            anon.ends_with("Asked by a device you have not met (723ed85f0566df0c)."),
            "{anon}"
        );
        let cert = plain_of("cert-issue-hosts", &names);
        assert!(
            cert.starts_with("Let ")
                && cert.contains("without asking until 20")
                && cert.contains("new host"),
            "{cert}"
        );
        let release = plain_of("share-release-u64-max", &names);
        assert!(release.starts_with("Give "), "{release}");
        assert!(
            release.contains("your piece of the saved password of host "),
            "{release}"
        );
        // the host id is the grouped form
        assert!(
            release.contains(' ')
                && release
                    .matches(|c: char| c.is_ascii_uppercase() || c.is_ascii_digit())
                    .count()
                    >= 32,
            "{release}"
        );
    }

    #[test]
    fn the_mode_change_says_what_changes_for_opening_passwords() {
        let v = vectors();
        let ids: Vec<&str> = v["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap())
            .filter(|i| i.starts_with("mode-change"))
            .collect();
        assert!(!ids.is_empty());
        let texts: Vec<String> = ids.iter().map(|i| plain_of(i, &BTreeMap::new())).collect();
        for t in &texts {
            assert!(
                t.contains("two of your devices to agree")
                    || t.contains("master phrase will open saved passwords"),
                "{t}"
            );
        }
    }
}
