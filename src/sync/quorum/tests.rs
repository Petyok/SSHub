//! Stage 3 end to end over in-process stores: vault and quorum credentials, the "copy of one
//! device is not enough" criterion, challenges, resharing, certificates, mode changes, migration.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use super::*;
use crate::credentials::{ensure_unlocked, PasswordStore, QuorumGatedStore};
use crate::sync::cert::{cert_allows, validate_cert_issue, DelegationCert};
use crate::sync::cred_ops::{cred_put, migrate_local_secrets, remove_local_copies, CredPayload};
use crate::sync::crypto::{bip39_encode, shamir_combine, COMBINE_CALLS};
use crate::sync::modes::validate_mode_change;
use crate::sync::oplog::{CertScope, CredMode};
use crate::sync::protocol::tests::{ring, Dev};
use crate::sync::reshare::{next_epoch, reissue_plan, reissue_receive, unadopted};
use crate::sync::share_release::unwrap_share_response;

const FAST: Argon2Profile = Argon2Profile {
    mem_kib: 64,
    iters: 1,
};
const PHRASE: &str = "alpha bravo charlie delta echo foxtrot";
const RING: [u8; 16] = [3; 16];

pub(crate) fn approve_all(d: &Dev) {
    for op in d.store.sync_pending().unwrap() {
        d.store.sync_resolve_pending(&op.hash(), true).unwrap();
    }
}

/// What a sync round does: ingest every chain tail `to` lacks, then the user approves what waits.
pub(crate) fn deliver(from: &Dev, to: &Dev) {
    for (dev, _, _) in from.store.sync_chain_heads().unwrap() {
        let have = to
            .store
            .sync_chain_heads()
            .unwrap()
            .into_iter()
            .find(|(d, _, _)| *d == dev)
            .map_or(0, |(_, s, _)| s);
        for op in from.store.sync_chain_tail(&dev, have).unwrap() {
            to.store.sync_ingest(op).unwrap();
        }
    }
    approve_all(to);
}

pub(crate) fn all_to_all(devs: &[Dev]) {
    for _ in 0..2 {
        for a in devs {
            for b in devs {
                if a.did != b.did {
                    deliver(a, b);
                }
            }
        }
    }
}

pub(crate) fn append(d: &Dev, body: OpBody) -> Op {
    d.store.sync_append_local(&d.id, body).unwrap()
}

pub(crate) fn quorum_mode(d: &Dev) {
    append(
        d,
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d.did.clone(),
            cosigner: None,
        },
    );
}

pub(crate) fn payload(pw: &str) -> CredPayload {
    CredPayload {
        password: Some(pw.into()),
        private_key: None,
    }
}

fn open_payload(store: &LauncherStore, host: &HostUid, dek: &[u8; 32]) -> CredPayload {
    let (ct, _) = store.sync_cred_ciphertext(host).unwrap().unwrap();
    CredPayload::from_bytes(&dek_open(dek, &ct).unwrap()).unwrap()
}

#[test]
fn a_vault_credential_opens_only_with_the_phrase_and_the_ring_secret() {
    let d = ring(2);
    for x in &d {
        vault_setup_with(&x.store, PHRASE, &RING, FAST).unwrap();
    }
    let host = [7u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("hunter2")).unwrap();
    all_to_all(&d);
    // The other machine opens it with what the kit and the phrase give it.
    let dek = vault_open_dek_with(&d[1].store, &host, PHRASE, &RING, FAST).unwrap();
    assert_eq!(
        open_payload(&d[1].store, &host, &dek).password.as_deref(),
        Some("hunter2")
    );
    // A copy of the data without the ring secret, or with the wrong phrase, opens nothing.
    assert!(vault_open_dek_with(&d[1].store, &host, PHRASE, &[4; 16], FAST).is_err());
    assert!(vault_open_dek_with(
        &d[1].store,
        &host,
        "alpha bravo charlie delta echo golf",
        &RING,
        FAST
    )
    .is_err());
    assert!(
        vault_open_dek_with(&d[1].store, &[8; 16], PHRASE, &RING, FAST).is_err(),
        "another credential"
    );
    // The device key alone is no way in either: the credential is not wrapped to any device.
    assert!(own_shares(&d[1].store, &d[1].id, &host).is_err());
}

#[test]
fn quorum_needs_two_shares_and_a_copy_of_one_device_has_one() {
    let d = ring(3);
    let paper_phrase = bip39_encode(&[0x42; 32]);
    for x in &d {
        paper_setup(&x.store, &paper_phrase).unwrap();
    }
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [9u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("s3cret")).unwrap();
    all_to_all(&d);

    // A copy of device 2: same device key, the whole log, nothing else.
    let copy = Dev::with(d[2].id.clone());
    for x in &d {
        copy.trust(x);
    }
    for x in &d {
        deliver(x, &copy);
    }
    let mine = own_shares(&copy.store, &copy.id, &host).unwrap();
    assert_eq!(mine.len(), 1, "one share");
    let share = mine[0].1.clone();
    assert!(
        assemble_dek(&copy.store, &host, share.clone(), share.clone()).is_err(),
        "the same share twice is not a quorum"
    );
    let junk = Share {
        index: 9,
        bytes: vec![1; 32],
    };
    assert!(assemble_dek(&copy.store, &host, share.clone(), junk).is_err());
    assert!(shamir_combine(std::slice::from_ref(&share)).is_err());
    assert!(
        vault_open_dek_with(&copy.store, &host, PHRASE, &RING, FAST).is_err(),
        "not a vault credential"
    );

    // With a peer's share it opens.
    let (eph_secret, eph_pub) = {
        let s = x25519_dalek::StaticSecret::from([5u8; 32]);
        (s.to_bytes(), x25519_dalek::PublicKey::from(&s).to_bytes())
    };
    let reply = release_share(&d[1].store, &d[1].id, &host, 0, &eph_pub).unwrap();
    let second = unwrap_share_response(&eph_secret, &reply).unwrap();
    let dek = assemble_dek(&copy.store, &host, share.clone(), second).unwrap();
    assert_eq!(
        open_payload(&copy.store, &host, &dek).password.as_deref(),
        Some("s3cret")
    );
    // The release is bound to the requester's key: another key reads nothing.
    assert!(unwrap_share_response(&[6u8; 32], &reply).is_err());

    // With the paper share it opens as well.
    let paper = paper_derive(&paper_phrase, PaperDomain::Unwrap).unwrap();
    let paper_share = own_shares(&copy.store, &paper, &host).unwrap().remove(0).1;
    let dek = assemble_dek(&copy.store, &host, share.clone(), paper_share).unwrap();
    assert_eq!(
        open_payload(&copy.store, &host, &dek).password.as_deref(),
        Some("s3cret")
    );
}

#[test]
fn a_challenge_binds_the_whole_op_and_travels_as_frames() {
    let d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [1u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("x")).unwrap();
    all_to_all(&d);
    let (c, secret) = make_challenge(&d[1].store, &d[1].id, &host, &d[2].did, 1_000, 120).unwrap();
    assert_eq!(c.phone_material.kind, ShareKind::Hybrid);
    assert_eq!(c.binding_hash(), c.to_wire().binding_hash());
    let decoded = ChallengeWire::decode(&c.to_wire().encode()).unwrap();
    assert_eq!(decoded, c.to_wire());
    let mut other = c.clone();
    other.expiry += 1;
    assert_ne!(c.binding_hash(), other.binding_hash());
    let frames = c.to_qr_payload(84).unwrap();
    assert!(frames.len() > 1 && frames.iter().all(|f| f.len() <= 84 + 16));
    assert_eq!(secret.len(), 32);
    assert_eq!(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(*secret)).to_bytes(),
        c.eph_x25519_pub
    );
}

#[test]
fn resharing_never_reconstructs_and_the_new_sharing_opens_while_a_removed_holders_old_share_does_not(
) {
    let mut d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let hosts: Vec<HostUid> = (1..=3u8).map(|n| [n; 16]).collect();
    for h in &hosts {
        cred_put(&d[0].store, &d[0].id, h, &payload(&format!("pw{}", h[0]))).unwrap();
    }
    all_to_all(&d);
    let removed_old = own_shares(&d[2].store, &d[2].id, &hosts[0])
        .unwrap()
        .remove(0)
        .1;

    let newcomer = Dev::new();
    for x in d.iter().chain(std::iter::once(&newcomer)) {
        for y in d.iter().chain(std::iter::once(&newcomer)) {
            if x.did != y.did
                && !x
                    .store
                    .sync_devices()
                    .unwrap()
                    .iter()
                    .any(|r| r.id == y.did)
            {
                x.trust(y);
            }
        }
    }
    d.push(newcomer);
    let (a, b, n) = (0, 1, 3);
    let holders = vec![d[a].did.clone(), d[b].did.clone(), d[n].did.clone()];
    let epoch = next_epoch(&d[a].store).unwrap();
    assert_eq!(epoch, 1);

    let combines = COMBINE_CALLS.with(|c| c.get());
    for who in [a, b] {
        for body in reissue_plan(&d[who].store, &d[who].id, &holders, epoch)
            .unwrap()
            .bodies
        {
            append(&d[who], body);
        }
    }
    all_to_all(&d);
    for who in [a, b, n] {
        for body in reissue_receive(&d[who].store, &d[who].id, epoch)
            .unwrap()
            .bodies
        {
            append(&d[who], body);
        }
    }
    all_to_all(&d);
    assert_eq!(
        COMBINE_CALLS.with(|c| c.get()),
        combines,
        "resharing must never call shamir_combine"
    );

    // Every new holder has a new share per credential, newest first.
    for h in &hosts {
        for who in [a, b, n] {
            let s = own_shares(&d[who].store, &d[who].id, h).unwrap();
            assert_eq!(s[0].0, 1, "the envelope share comes first");
        }
    }
    // Two new shares open the credential.
    let s_n = own_shares(&d[n].store, &d[n].id, &hosts[0])
        .unwrap()
        .remove(0)
        .1;
    let s_b = own_shares(&d[b].store, &d[b].id, &hosts[0])
        .unwrap()
        .remove(0)
        .1;
    let dek = assemble_dek(&d[n].store, &hosts[0], s_n.clone(), s_b.clone()).unwrap();
    assert_eq!(
        open_payload(&d[n].store, &hosts[0], &dek)
            .password
            .as_deref(),
        Some("pw1")
    );
    // The removed holder's old share does not combine with a new one.
    assert!(assemble_dek(&d[n].store, &hosts[0], removed_old, s_b).is_err());

    // Adoption is recorded by a real unlock only.
    assert_eq!(unadopted(&d[n].store, epoch).unwrap().len(), 3);
    let gate = Arc::new(crate::credentials::SecretGate::default());
    let unlocker = QuorumUnlocker::with_profile(
        d[n].store.clone(),
        d[n].id.clone(),
        gate,
        Box::new(FromPeer {
            store: d[b].store.clone(),
            id: d[b].id.clone(),
        }),
        None,
        FAST,
    );
    let plain = unlocker.open_one(&hosts[1]).unwrap();
    assert_eq!(
        CredPayload::from_bytes(&plain).unwrap().password.as_deref(),
        Some("pw2")
    );
    assert_eq!(unadopted(&d[n].store, epoch).unwrap().len(), 2);
}

struct FromPeer {
    store: Arc<LauncherStore>,
    id: Arc<DeviceIdentity>,
}

impl ShareSource for FromPeer {
    fn master_phrase(&self) -> Option<Zeroizing<String>> {
        Some(Zeroizing::new(PHRASE.into()))
    }

    fn second_share(
        &self,
        host: &HostUid,
        epoch: u64,
        _mine: &Share,
    ) -> std::result::Result<Share, UnlockError> {
        let s = x25519_dalek::StaticSecret::from([11u8; 32]);
        let reply = release_share(
            &self.store,
            &self.id,
            host,
            epoch,
            &x25519_dalek::PublicKey::from(&s).to_bytes(),
        )
        .map_err(|e| UnlockError::NeedsShares(e.to_string()))?;
        unwrap_share_response(&s.to_bytes(), &reply).map_err(|e| UnlockError::Failed(e.to_string()))
    }
}

#[test]
fn the_unlocker_fills_the_gate_for_a_real_host_row_and_nothing_else() {
    let d = ring(2);
    // Exactly two holders: the paper is mandatory.
    append(
        &d[0],
        paper_registration(&bip39_encode(&[0x71; 32])).unwrap(),
    );
    quorum_mode(&d[0]);
    all_to_all(&d);
    let row = d[0]
        .store
        .create_host(&crate::store::NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let uid = d[0].store.host_uid(row.id).unwrap().unwrap();
    cred_put(&d[0].store, &d[0].id, &uid, &payload("opened")).unwrap();
    all_to_all(&d);

    let gated = QuorumGatedStore::with_gate(
        Box::new(MemStore(Mutex::new(HashMap::new()))),
        Arc::new(crate::credentials::SecretGate::default()),
    );
    let gate = gated.gate_handle();
    let unlocker = QuorumUnlocker::with_profile(
        d[0].store.clone(),
        d[0].id.clone(),
        gate.clone(),
        Box::new(FromPeer {
            store: d[1].store.clone(),
            id: d[1].id.clone(),
        }),
        None,
        FAST,
    );
    let key = crate::credentials::host_key(row.id);
    assert!(gate.is_synced(&key) && !gate.is_synced("host:99999"));
    assert!(
        gated
            .get(&key)
            .unwrap_err()
            .downcast_ref::<crate::credentials::Locked>()
            .is_some(),
        "locked until opened"
    );
    ensure_unlocked(&[SecretRef::Key(key.clone())], &unlocker).unwrap();
    assert_eq!(gated.get(&key).unwrap().as_deref(), Some("opened"));
    assert!(
        gated.set(&key, "x").is_err(),
        "a synced credential is not written through the local store"
    );

    // Without a peer to release a share the unlock fails cleanly and the gate stays shut.
    let alone = Arc::new(crate::credentials::SecretGate::default());
    struct Nobody;
    impl ShareSource for Nobody {
        fn master_phrase(&self) -> Option<Zeroizing<String>> {
            None
        }
        fn second_share(
            &self,
            _: &HostUid,
            _: u64,
            _: &Share,
        ) -> std::result::Result<Share, UnlockError> {
            Err(UnlockError::NeedsShares("no peer".into()))
        }
    }
    let lonely = QuorumUnlocker::with_profile(
        d[0].store.clone(),
        d[0].id.clone(),
        alone.clone(),
        Box::new(Nobody),
        None,
        FAST,
    );
    assert!(matches!(
        ensure_unlocked(&[SecretRef::Key(key.clone())], &lonely),
        Err(UnlockError::NeedsShares(_))
    ));
    assert!(alone.get(&key).is_none());
}

#[test]
fn certificates_are_enforced_by_the_holders() {
    let d = ring(4); // 0 = phone, 1 = delegated terminal, 2 and 3 enforcing holders
    let phone = &d[0];
    let host_a = [21u8; 16];
    let host_b = [22u8; 16];
    let host_c = [23u8; 16];
    let now = crate::store::now_ts();
    let op = append(
        phone,
        OpBody::CertIssue {
            device_id: d[1].did.clone(),
            scope: CertScope::Hosts(vec![host_a, host_b]),
            expiry: now + 7 * 86_400,
            max_new_hosts_per_day: 2,
            epoch_lo: 0,
            epoch_hi: 10,
        },
    );
    assert!(validate_cert_issue(&d[2].store, &op, &phone.did).is_ok());
    assert!(
        validate_cert_issue(&d[2].store, &op, &d[1].did).is_err(),
        "only the phone issues"
    );
    let cert = DelegationCert::from_op(&op).unwrap();
    let pk = phone.id.public().ed25519_pub;
    let ok = |host: &HostUid, requester: &DeviceId, head_ts: i64, at: i64| {
        cert_allows(&cert, &pk, &d[2].store, requester, host, head_ts, at)
    };
    let at = op.timestamp + 10;
    assert!(ok(&host_a, &d[1].did, at, at).is_ok());
    assert!(ok(&host_c, &d[1].did, at, at)
        .unwrap_err()
        .to_string()
        .contains("scope"));
    assert!(ok(&host_a, &d[2].did, at, at)
        .unwrap_err()
        .to_string()
        .contains("another device"));
    assert!(ok(&host_a, &d[1].did, at - 15 * 86_400, at)
        .unwrap_err()
        .to_string()
        .contains("phone"));
    assert!(ok(&host_a, &d[1].did, at, now + 8 * 86_400)
        .unwrap_err()
        .to_string()
        .contains("expired"));
    assert!(
        cert_allows(
            &cert,
            &d[2].id.public().ed25519_pub,
            &d[2].store,
            &d[1].did,
            &host_a,
            at,
            at
        )
        .is_err(),
        "wrong issuer key"
    );

    // The budget is ring-wide: two different holders open two hosts, a third new host is refused,
    // an already opened one stays free.
    all_to_all(&d);
    append(&d[2], crate::sync::cert::cert_opened_body(&cert, &host_a));
    append(&d[3], crate::sync::cert::cert_opened_body(&cert, &host_b));
    all_to_all(&d);
    let later = at + 10;
    assert!(
        ok(&host_a, &d[1].did, later, later).is_ok(),
        "an opened host is free"
    );
    let cert3 = {
        let op3 = append(
            phone,
            OpBody::CertIssue {
                device_id: d[1].did.clone(),
                scope: CertScope::Hosts(vec![host_a, host_b, host_c]),
                expiry: now + 7 * 86_400,
                max_new_hosts_per_day: 2,
                epoch_lo: 0,
                epoch_hi: 10,
            },
        );
        all_to_all(&d);
        DelegationCert::from_op(&op3).unwrap()
    };
    append(&d[2], crate::sync::cert::cert_opened_body(&cert3, &host_a));
    append(&d[3], crate::sync::cert::cert_opened_body(&cert3, &host_b));
    all_to_all(&d);
    let t = later + 20;
    assert!(
        cert_allows(&cert3, &pk, &d[2].store, &d[1].did, &host_c, t, t)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );

    // R3b: only the issuing phone extends the day, by one extension per 24 hours.
    append(
        &d[2],
        OpBody::CertExtend {
            cert_id: cert3.id,
            extra_hosts: 5,
        },
    );
    all_to_all(&d);
    assert_eq!(
        crate::sync::cert::extension_budget(&d[2].store, &cert3, t).unwrap(),
        0,
        "a holder cannot extend"
    );
    let req = crate::sync::cert::cert_extend_request(cert3.id, 1).unwrap();
    assert_eq!(
        crate::sync::wire::CertExtendRequestWire::decode(&req.encode()).unwrap(),
        req
    );
    append(phone, req.to_body());
    append(
        phone,
        OpBody::CertExtend {
            cert_id: cert3.id,
            extra_hosts: 7,
        },
    );
    all_to_all(&d);
    assert_eq!(
        crate::sync::cert::extension_budget(&d[2].store, &cert3, t).unwrap(),
        1,
        "the second extension inside 24 hours does not count"
    );
    assert!(cert_allows(&cert3, &pk, &d[2].store, &d[1].did, &host_c, t, t).is_ok());
    assert_eq!(
        crate::sync::cert::extension_budget(&d[2].store, &cert3, t + 2 * 86_400).unwrap(),
        0,
        "an extension covers its own day only"
    );

    // Revocation arrives with the next sync.
    append(phone, OpBody::CertRevoke { cert_id: cert.id });
    all_to_all(&d);
    assert!(ok(&host_a, &d[1].did, t, t)
        .unwrap_err()
        .to_string()
        .contains("revoked"));

    // Fewer than three holders: nobody enforces it.
    let small = ring(2);
    let op = append(
        &small[0],
        OpBody::CertIssue {
            device_id: small[1].did.clone(),
            scope: CertScope::Hosts(vec![host_a]),
            expiry: now + 86_400,
            max_new_hosts_per_day: 5,
            epoch_lo: 0,
            epoch_hi: 1,
        },
    );
    assert!(validate_cert_issue(&small[1].store, &op, &small[0].did)
        .unwrap_err()
        .to_string()
        .contains("three holders"));
}

#[test]
fn a_mode_change_re_encrypts_under_new_deks_and_going_back_needs_a_cosigner() {
    use crate::sync::modes::{mode_change_body, mode_migration};
    let d = ring_phone(3); // 0, 1, 2 terminals; 3 the phone
    for x in &d {
        vault_setup_with(&x.store, PHRASE, &RING, FAST).unwrap();
    }
    let paper = bip39_encode(&[0x55; 32]);
    let reg = append(
        &d[0],
        crate::sync::quorum::paper_registration(&paper).unwrap(),
    );
    complete(&d, &reg, 1, None);
    let hosts = [[31u8; 16], [32u8; 16]];
    for h in &hosts {
        cred_put(&d[0].store, &d[0].id, h, &payload(&format!("v{}", h[0]))).unwrap();
    }
    all_to_all(&d);
    let before = d[0].store.sync_cred_ciphertext(&hosts[0]).unwrap().unwrap();

    let open_vault = |h: &HostUid| -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let dek = vault_open_dek_with(&d[0].store, h, PHRASE, &RING, FAST)?;
        let (ct, _) = d[0].store.sync_cred_ciphertext(h)?.unwrap();
        Ok(Zeroizing::new(dek_open(&dek, &ct)?))
    };
    let body = mode_change_body(&d[0].store, &d[0].id.public(), CredMode::Quorum, None).unwrap();
    let mc = append(&d[0], body);
    // Proposed, not complete: nothing is re-encrypted and new credentials stay in the old shape.
    assert!(mode_migration(&d[0].store, &d[0].id.public(), &open_vault)
        .unwrap_err()
        .to_string()
        .contains("ceremony"));
    assert_eq!(
        crate::sync::ceremony::effective_cred_mode(&d[0].store).unwrap(),
        CredMode::Vault
    );
    complete(&d, &mc, 1, Some(&paper));
    assert_eq!(
        crate::sync::ceremony::effective_cred_mode(&d[0].store).unwrap(),
        CredMode::Quorum
    );
    let bodies = mode_migration(&d[0].store, &d[0].id.public(), &open_vault).unwrap();
    assert_eq!(bodies.len(), hosts.len());
    for b in bodies {
        append(&d[0], b);
    }
    all_to_all(&d);
    assert_eq!(d[1].store.sync_cred_mode().unwrap(), CredMode::Quorum);
    let after = d[0].store.sync_cred_ciphertext(&hosts[0]).unwrap().unwrap();
    assert_ne!(before.0, after.0, "a fresh DEK, a fresh ciphertext");
    assert_eq!(
        after.1.len(),
        5,
        "one share per holder, the phone and the paper"
    );
    // The new shape opens with two shares, not with the vault.
    let s0 = own_shares(&d[0].store, &d[0].id, &hosts[0])
        .unwrap()
        .remove(0)
        .1;
    let s1 = own_shares(&d[1].store, &d[1].id, &hosts[0])
        .unwrap()
        .remove(0)
        .1;
    let dek = assemble_dek(&d[0].store, &hosts[0], s0, s1).unwrap();
    assert_eq!(
        open_payload(&d[0].store, &hosts[0], &dek)
            .password
            .as_deref(),
        Some("v31")
    );
    assert!(vault_open_dek_with(&d[0].store, &hosts[0], PHRASE, &RING, FAST).is_err());

    // Back to Vault: a co-signer is required.
    assert!(mode_change_body(&d[0].store, &d[0].id.public(), CredMode::Vault, None).is_err());
    let back = mode_change_body(
        &d[0].store,
        &d[0].id.public(),
        CredMode::Vault,
        Some(d[1].did.clone()),
    )
    .unwrap();
    let mc = append(&d[0], back);
    assert!(validate_mode_change(&d[2].store, &mc).is_ok());
    let alone = append(
        &d[0],
        OpBody::ModeChange {
            to: CredMode::Vault,
            migrated_on: d[0].did.clone(),
            cosigner: Some(d[0].did.clone()),
        },
    );
    assert!(
        validate_mode_change(&d[2].store, &alone).is_err(),
        "cannot co-sign yourself"
    );
    let nobody = append(
        &d[0],
        OpBody::ModeChange {
            to: CredMode::Vault,
            migrated_on: d[0].did.clone(),
            cosigner: None,
        },
    );
    assert!(validate_mode_change(&d[2].store, &nobody).is_err());
}

#[test]
fn payload_is_strict() {
    let p = CredPayload {
        password: Some("p".into()),
        private_key: Some(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----".into(),
        ),
    };
    assert_eq!(CredPayload::from_bytes(&p.to_bytes()).unwrap(), p);
    assert!(CredPayload::from_bytes(br#"{"v":2}"#).is_err());
    assert!(CredPayload::from_bytes(br#"{"v":1,"extra":1}"#).is_err());
    assert!(CredPayload::from_bytes(b"[]").is_err());
}

struct MemStore(Mutex<HashMap<String, String>>);

impl PasswordStore for MemStore {
    fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, v: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().insert(key.into(), v.into());
        Ok(())
    }

    fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn migration_reads_local_secrets_replicates_them_and_removes_the_copies_only_on_request() {
    let d = ring(2);
    vault_setup_with(&d[0].store, PHRASE, &RING, FAST).unwrap();
    let h = d[0]
        .store
        .create_host(&crate::store::NewHost::launcher("db", "10.0.0.9"))
        .unwrap();
    let i = d[0]
        .store
        .create_identity(&crate::store::NewIdentity {
            name: "me".into(),
            ..Default::default()
        })
        .unwrap();
    d[0].store
        .create_host(&crate::store::NewHost::launcher("nopw", "10.0.0.8"))
        .unwrap();
    let keyfile = std::env::temp_dir().join(format!("sshub-key-{}", std::process::id()));
    std::fs::write(
        &keyfile,
        "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----\n",
    )
    .unwrap();
    d[0].store
        .with_conn(|c| {
            c.execute("UPDATE hosts SET has_password = 1 WHERE id = ?1", [h.id])?;
            c.execute(
                "UPDATE identities SET private_key = ?2 WHERE id = ?1",
                rusqlite::params![i.id, keyfile.to_string_lossy()],
            )?;
            Ok(())
        })
        .unwrap();
    let local = MemStore(Mutex::new(HashMap::new()));
    local
        .set(&crate::credentials::host_key(h.id), "dbpass")
        .unwrap();
    let report = migrate_local_secrets(&d[0].store, &d[0].id, &local).unwrap();
    assert_eq!(
        report.migrated,
        vec![
            crate::credentials::host_key(h.id),
            crate::credentials::identity_key(i.id)
        ]
    );
    assert!(
        local
            .get(&crate::credentials::host_key(h.id))
            .unwrap()
            .is_some(),
        "READ, not deleted"
    );

    let uid = d[0].store.host_uid(h.id).unwrap().unwrap();
    let dek = vault_open_dek_with(&d[0].store, &uid, PHRASE, &RING, FAST).unwrap();
    assert_eq!(
        open_payload(&d[0].store, &uid, &dek).password.as_deref(),
        Some("dbpass")
    );
    let iuid = d[0].store.identity_uid(i.id).unwrap().unwrap();
    let dek = vault_open_dek_with(&d[0].store, &iuid, PHRASE, &RING, FAST).unwrap();
    assert!(open_payload(&d[0].store, &iuid, &dek)
        .private_key
        .clone()
        .unwrap()
        .contains("key"));

    remove_local_copies(&local, &report).unwrap();
    assert!(local
        .get(&crate::credentials::host_key(h.id))
        .unwrap()
        .is_none());
    let _ = std::fs::remove_file(keyfile);
}

#[test]
fn the_ring_secret_sheet_payload_parses_exactly() {
    let secret: [u8; 16] = std::array::from_fn(|i| (i * 17) as u8);
    let text = ring_secret_text(&secret);
    assert_eq!(text, "sshub:ring:1:00112233445566778899aabbccddeeff");
    assert_eq!(parse_ring_secret(&text).unwrap(), secret);
    assert_eq!(parse_ring_secret(&format!("  {text}\n")).unwrap(), secret);
    assert_eq!(
        parse_ring_secret("0011 2233 4455 6677 8899 AABB CCDD EEFF").unwrap(),
        secret,
        "typed hex groups"
    );
    assert!(parse_ring_secret("sshub:ring:2:00112233445566778899aabbccddeeff").is_err());
    assert!(parse_ring_secret("sshub:ring:1:00112233445566778899aabbccddee").is_err());
    assert!(parse_ring_secret("sshub:ring:1:zz112233445566778899aabbccddeeff").is_err());
    assert!(parse_ring_secret("").is_err());
    let store = LauncherStore::open_in_memory().unwrap();
    assert!(vault_setup(&store, "not six eff words", &secret).is_err());
}

struct NoSource;

impl ShareSource for NoSource {
    fn master_phrase(&self) -> Option<Zeroizing<String>> {
        None
    }

    fn second_share(
        &self,
        _: &HostUid,
        _: u64,
        _: &Share,
    ) -> std::result::Result<Share, UnlockError> {
        Err(UnlockError::NeedsShares("no peer".into()))
    }
}

fn unlocker(d: &Dev, source: Box<dyn ShareSource>, ring: Option<[u8; 16]>) -> QuorumUnlocker {
    QuorumUnlocker::with_profile(
        d.store.clone(),
        d.id.clone(),
        Arc::new(crate::credentials::SecretGate::default()),
        source,
        ring,
        FAST,
    )
}

fn peer(d: &Dev) -> Box<dyn ShareSource> {
    Box::new(FromPeer {
        store: d.store.clone(),
        id: d.id.clone(),
    })
}

#[test]
fn p0_a_copy_of_one_device_opens_nothing_through_the_unlocker_and_one_more_share_opens_it() {
    // Quorum ring.
    let d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [41u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("quorum-secret")).unwrap();
    all_to_all(&d);
    // The copy: device 2's key and the whole log in a fresh process, no other share.
    let copy = Dev::with(d[2].id.clone());
    for x in &d {
        copy.trust(x);
        deliver(x, &copy);
    }
    assert!(matches!(
        unlocker(&copy, Box::new(NoSource), None).open_one(&host),
        Err(UnlockError::NeedsShares(_))
    ));
    assert!(
        matches!(
            unlocker(&copy, Box::new(NoSource), Some(RING)).open_one(&host),
            Err(UnlockError::NeedsShares(_))
        ),
        "a ring secret does not help a quorum credential"
    );
    let opened = unlocker(&copy, peer(&d[1]), None).open_one(&host).unwrap();
    assert_eq!(
        CredPayload::from_bytes(&opened)
            .unwrap()
            .password
            .as_deref(),
        Some("quorum-secret")
    );

    // Vault ring: the copy of a machine has the device key and the log; the ring secret is not in it.
    let v = ring(2);
    for x in &v {
        vault_setup_with(&x.store, PHRASE, &RING, FAST).unwrap();
    }
    cred_put(&v[0].store, &v[0].id, &host, &payload("vault-secret")).unwrap();
    all_to_all(&v);
    let vcopy = Dev::with(v[1].id.clone());
    for x in &v {
        vcopy.trust(x);
        deliver(x, &vcopy);
    }
    assert!(
        unlocker(&vcopy, Box::new(NoSource), None)
            .open_one(&host)
            .is_err(),
        "no ring secret, no phrase"
    );
    assert!(
        unlocker(&vcopy, peer(&v[0]), None).open_one(&host).is_err(),
        "the phrase without the ring secret"
    );
    assert!(
        matches!(
            unlocker(&vcopy, Box::new(NoSource), Some(RING)).open_one(&host),
            Err(UnlockError::Cancelled)
        ),
        "the ring secret without the phrase"
    );
    let opened = unlocker(&vcopy, peer(&v[0]), Some(RING))
        .open_one(&host)
        .unwrap();
    assert_eq!(
        CredPayload::from_bytes(&opened)
            .unwrap()
            .password
            .as_deref(),
        Some("vault-secret")
    );
}

#[test]
fn paper_survives_a_revocation_reshare_and_opens_with_the_laptop_after_the_phone_is_lost() {
    // {laptop, phone, removed holder} plus the paper phrase.
    let d = ring(3);
    let (laptop, phone, removed) = (&d[0], &d[1], &d[2]);
    let phrase = bip39_encode(&[0x24; 32]);
    for x in &d {
        paper_setup(&x.store, &phrase).unwrap();
    }
    let paper = paper_derive(&phrase, PaperDomain::Unwrap).unwrap();
    quorum_mode(laptop);
    all_to_all(&d);
    let before = [[51u8; 16], [52u8; 16]];
    for h in &before {
        cred_put(
            &laptop.store,
            &laptop.id,
            h,
            &payload(&format!("old{}", h[0])),
        )
        .unwrap();
    }
    all_to_all(&d);
    let removed_old = own_shares(&removed.store, &removed.id, &before[0])
        .unwrap()
        .remove(0)
        .1;
    let paper_old = own_shares(&laptop.store, &paper, &before[0])
        .unwrap()
        .remove(0)
        .1;

    // Revoke the third holder and reshare to the two that remain. Nobody types the paper phrase.
    append(
        laptop,
        OpBody::DeviceRevoke {
            device_id: removed.did.clone(),
        },
    );
    all_to_all(&d[..2]);
    let epoch = next_epoch(&laptop.store).unwrap();
    let holders = vec![laptop.did.clone(), phone.did.clone()];
    let combines = COMBINE_CALLS.with(|c| c.get());
    for who in [laptop, phone] {
        for body in reissue_plan(&who.store, &who.id, &holders, epoch)
            .unwrap()
            .bodies
        {
            append(who, body);
        }
    }
    all_to_all(&d[..2]);
    for who in [laptop, phone] {
        for body in reissue_receive(&who.store, &who.id, epoch).unwrap().bodies {
            append(who, body);
        }
    }
    all_to_all(&d[..2]);
    assert_eq!(
        COMBINE_CALLS.with(|c| c.get()),
        combines,
        "the reshare never reconstructs, paper included"
    );

    // A credential created after the reshare: the removed holder is not a recipient, the paper is.
    let after = [53u8; 16];
    cred_put(&laptop.store, &laptop.id, &after, &payload("new53")).unwrap();
    all_to_all(&d[..2]);
    assert!(own_shares(&removed.store, &removed.id, &after).is_err());

    // The phone is lost. The laptop and the paper still open every credential.
    for h in before.iter().chain(std::iter::once(&after)) {
        let mine = own_shares(&laptop.store, &laptop.id, h).unwrap().remove(0);
        let papers = own_shares(&laptop.store, &paper, h).unwrap().remove(0);
        assert_eq!(
            mine.0, papers.0,
            "both at the newest sharing of that credential"
        );
        let dek = assemble_dek(&laptop.store, h, mine.1, papers.1).unwrap();
        let want = if h[0] == 53 {
            "new53".to_string()
        } else {
            format!("old{}", h[0])
        };
        assert_eq!(
            open_payload(&laptop.store, h, &dek)
                .password
                .clone()
                .unwrap(),
            want
        );
    }
    assert_eq!(
        own_shares(&laptop.store, &paper, &before[0]).unwrap()[0].0,
        epoch,
        "the paper's share of an old credential is the reshared one"
    );

    // The removed holder's old share does not combine with the paper's new share, and has nothing for
    // credentials created after the reshare.
    let paper_new = own_shares(&laptop.store, &paper, &before[0])
        .unwrap()
        .remove(0)
        .1;
    assert!(assemble_dek(&laptop.store, &before[0], removed_old.clone(), paper_new).is_err());
    // Residual (B6, as documented): the old sharing is still there; a revoked device that keeps its old
    // wrap key plus the paper's OLD share opens what existed before the reshare, never what came after.
    assert!(assemble_dek(&laptop.store, &before[0], removed_old, paper_old).is_ok());
    let papers_after = own_shares(&laptop.store, &paper, &after)
        .unwrap()
        .remove(0)
        .1;
    assert!(assemble_dek(
        &laptop.store,
        &after,
        Share {
            index: 3,
            bytes: vec![0; 32]
        },
        papers_after
    )
    .is_err());
}

#[test]
fn the_trust_session_path_grants_under_a_certificate_and_defers_without_one() {
    use crate::sync::cert::cert_opened_body;
    use crate::sync::oplog::{CertScope, DeviceKind};
    use crate::sync::protocol::tests::pipe_pair;
    use crate::sync::protocol::{run_share_request, run_sync_serve, ShareAsk, ShareReply};

    // 0 phone, 1 requester, 2 holder that answers, 3 another holder
    let d: Vec<_> = (0..4).map(|_| Dev::new()).collect();
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
    quorum_mode(&d[2]);
    all_to_all(&d);
    let host = [81u8; 16];
    cred_put(&d[2].store, &d[2].id, &host, &payload("trusted")).unwrap();
    let now = crate::store::now_ts();
    let issue = |who: usize, scope: CertScope| {
        append(
            &d[0],
            OpBody::CertIssue {
                device_id: d[who].did.clone(),
                scope,
                expiry: now + 7 * 86_400,
                max_new_hosts_per_day: 5,
                epoch_lo: 0,
                epoch_hi: 9,
            },
        )
    };
    issue(1, CertScope::Hosts(vec![host]));
    issue(3, CertScope::Hosts(vec![[99u8; 16]]));
    all_to_all(&d);

    let eph_secret = x25519_dalek::StaticSecret::from([13u8; 32]);
    let ask = |rid: u64| ShareAsk {
        host_uid: host,
        epoch: 0,
        eph: x25519_dalek::PublicKey::from(&eph_secret).to_bytes(),
        request_id: rid,
    };
    let run = |requester: usize, rid: u64| {
        let (mut cl, mut sl) = pipe_pair();
        let (hs, hi) = (&d[2].store, &d[2].id);
        std::thread::scope(|sc| {
            let h = sc.spawn(move || run_sync_serve(hs, hi, &mut sl));
            let r = run_share_request(
                &mut cl,
                &d[requester].store,
                &d[requester].id,
                &d[2].did,
                &ask(rid),
            );
            drop(cl);
            (r, h.join().unwrap())
        })
    };

    // The requester with a certificate for this host is granted a share sealed to its key.
    let (r, served) = run(1, 7);
    served.unwrap();
    let ShareReply::Granted(sealed) = r.unwrap() else {
        panic!("expected a grant")
    };
    let second =
        crate::sync::share_release::unwrap_share_response(&eph_secret.to_bytes(), &sealed).unwrap();
    let mine = own_shares(&d[1].store, &d[1].id, &host)
        .unwrap()
        .remove(0)
        .1;
    let dek = assemble_dek(&d[1].store, &host, mine, second).unwrap();
    assert_eq!(
        open_payload(&d[1].store, &host, &dek).password.as_deref(),
        Some("trusted")
    );
    // The first release of the host was recorded in the log by the holder.
    assert_eq!(
        d[2].store
            .sync_cert_hosts_opened(
                &crate::sync::cert::DelegationCert::from_op(
                    &ops_of(&d[2].store, "CertIssue", None).unwrap()[0]
                )
                .unwrap()
                .id
            )
            .unwrap(),
        vec![host]
    );

    // A device whose certificate covers another host, and one with no certificate, are deferred.
    for (who, rid) in [(3usize, 8u64), (0usize, 9u64)] {
        let (r, served) = run(who, rid);
        served.unwrap();
        assert!(
            matches!(r.unwrap(), ShareReply::Deferred(_)),
            "device {who}"
        );
    }
    let _ = cert_opened_body;
}

#[test]
fn the_attended_peer_path_goes_through_the_log_and_the_grant_is_sealed_to_the_ephemeral_key() {
    let d = ring(3); // 0 requester, 1 attended holder, 2 another holder
    quorum_mode(&d[0]);
    all_to_all(&d);
    let host = [91u8; 16];
    cred_put(&d[0].store, &d[0].id, &host, &payload("through the log")).unwrap();
    all_to_all(&d);

    let req = peer_request(&d[0].store, &d[0].id, &host).unwrap();
    assert!(
        peer_poll(&d[0].store, &req, 0).unwrap().is_none(),
        "nothing granted yet"
    );
    // The holder syncs; the release waits in review. Until approved it grants nothing.
    for (dev, _, _) in d[0].store.sync_chain_heads().unwrap() {
        let have = d[1]
            .store
            .sync_chain_heads()
            .unwrap()
            .into_iter()
            .find(|(x, _, _)| *x == dev)
            .map_or(0, |(_, s, _)| s);
        for op in d[0].store.sync_chain_tail(&dev, have).unwrap() {
            d[1].store.sync_ingest(op).unwrap();
        }
    }
    let pending = d[1].store.sync_pending().unwrap();
    let release = pending
        .iter()
        .find(|o| matches!(o.body, OpBody::ShareRelease { .. }))
        .expect("the release is in review")
        .clone();
    assert!(
        grant_bodies(&d[1].store, &d[1].id, &release).is_err(),
        "not approved yet"
    );
    d[1].store
        .sync_resolve_pending(&release.hash(), true)
        .unwrap();
    let release = ops_of(&d[1].store, "ShareRelease", None).unwrap().remove(0);

    let bodies = grant_bodies(&d[1].store, &d[1].id, &release).unwrap();
    assert_eq!(bodies.len(), 1, "one share epoch, one grant");
    for b in bodies {
        append(&d[1], b);
    }
    // The requester picks it up on its next sync and opens the credential with its own share.
    deliver(&d[1], &d[0]);
    let second = peer_poll(&d[0].store, &req, 0)
        .unwrap()
        .expect("the grant arrived");
    let mine = own_shares(&d[0].store, &d[0].id, &host)
        .unwrap()
        .remove(0)
        .1;
    let dek = assemble_dek(&d[0].store, &host, mine, second).unwrap();
    assert_eq!(
        open_payload(&d[0].store, &host, &dek).password.as_deref(),
        Some("through the log")
    );
    assert!(
        peer_poll(&d[0].store, &req, 1).unwrap().is_none(),
        "no grant for another epoch"
    );
    // A bystander that sees the log cannot open the grant, and another request id does not match.
    let other = peer_request(&d[0].store, &d[0].id, &host).unwrap();
    assert!(peer_poll(&d[0].store, &other, 0).unwrap().is_none());

    // An unattended holder never grants on approval.
    d[2].make_unattended();
    assert!(grant_bodies(&d[2].store, &d[2].id, &release)
        .unwrap_err()
        .to_string()
        .contains("unattended"));
}

#[test]
fn revoking_a_holder_runs_rotate_reshare_adopt_and_retire_without_prompts() {
    use crate::sync::reshare::mark_adopted;
    use crate::sync::revocation::{check_revocable, next_actions, perform, RevocationAction};

    let mut d = ring(4); // 0, 1, 2 stay; 3 is revoked
    quorum_mode(&d[0]);
    all_to_all(&d);
    let hosts: Vec<HostUid> = (1..=3u8).map(|n| [100 + n; 16]).collect();
    for h in &hosts {
        cred_put(&d[0].store, &d[0].id, h, &payload(&format!("p{}", h[0]))).unwrap();
    }
    all_to_all(&d);
    let removed_old = own_shares(&d[3].store, &d[3].id, &hosts[0])
        .unwrap()
        .remove(0)
        .1;

    let now = crate::store::now_ts();
    check_revocable(&d[0].store, &d[3].did, now).unwrap();
    append(
        &d[0],
        OpBody::DeviceRevoke {
            device_id: d[3].did.clone(),
        },
    );
    all_to_all(&d[..3]);
    assert!(
        next_actions(&d[3].store, &d[3].id, now).unwrap().is_empty(),
        "the revoked device has no part"
    );

    // Each round every remaining holder does what the log says is next; nobody is asked anything.
    let mut seen_rotate = 0;
    let mut seen_contribute = 0;
    let mut seen_receive = 0;
    for _ in 0..8 {
        let mut progressed = false;
        for dev in d.iter_mut().take(3) {
            let actions = next_actions(&dev.store, &dev.id, now).unwrap();
            for a in actions {
                match &a {
                    RevocationAction::Rotate => seen_rotate += 1,
                    RevocationAction::Contribute { .. } => seen_contribute += 1,
                    RevocationAction::Receive { .. } => seen_receive += 1,
                    RevocationAction::Retire => continue,
                }
                let store = dev.store.clone();
                let bodies = perform(
                    &store,
                    std::sync::Arc::get_mut(&mut dev.id).expect("unique identity"),
                    &a,
                )
                .unwrap();
                for b in bodies {
                    append(dev, b);
                }
                progressed = true;
            }
        }
        all_to_all(&d[..3]);
        if !progressed {
            break;
        }
    }
    assert_eq!(
        (seen_rotate, seen_contribute, seen_receive),
        (3, 3, 3),
        "each remaining holder rotated, contributed and received once"
    );
    // The new sharing opens, the removed holder's old share does not combine with it.
    for h in &hosts {
        let s0 = own_shares(&d[0].store, &d[0].id, h).unwrap().remove(0);
        let s1 = own_shares(&d[1].store, &d[1].id, h).unwrap().remove(0);
        assert_eq!((s0.0, s1.0), (1, 1));
        let dek = assemble_dek(&d[0].store, h, s0.1.clone(), s1.1).unwrap();
        assert_eq!(
            open_payload(&d[0].store, h, &dek).password.clone().unwrap(),
            format!("p{}", h[0])
        );
        if *h == hosts[0] {
            assert!(assemble_dek(&d[0].store, h, removed_old.clone(), s0.1).is_err());
        }
    }

    // The previous wrap keys stay until every credential was adopted by a real unlock.
    assert!(!next_actions(&d[0].store, &d[0].id, now)
        .unwrap()
        .contains(&RevocationAction::Retire));
    for h in &hosts {
        mark_adopted(&d[0].store, h, 1).unwrap();
    }
    assert!(next_actions(&d[0].store, &d[0].id, now)
        .unwrap()
        .contains(&RevocationAction::Retire));
    let store = d[0].store.clone();
    let did0 = d[0].did.clone();
    let did1 = d[1].did.clone();
    let id0 = std::sync::Arc::get_mut(&mut d[0].id).unwrap();
    assert_eq!(id0.wrap_key_generations(), 2);
    perform(&store, id0, &RevocationAction::Retire).unwrap();
    assert_eq!(id0.wrap_key_generations(), 1);
    // And the old wrapped shares are now unreadable by it: forward secrecy.
    let (_, wraps) = store.sync_cred_ciphertext(&hosts[0]).unwrap().unwrap();
    let mine = wraps.iter().find(|w| w.recipient == did0).unwrap();
    assert!(crate::sync::crypto::hybrid_unwrap(id0, &mine.sealed).is_err());

    // With only two holders left, a revocation is refused up front.
    let _ = did1;
    let two = ring(2);
    assert!(
        check_revocable(&two[0].store, &two[1].did, now).is_err(),
        "one remaining holder cannot contribute"
    );
}

struct OnlyPhrase(&'static str);

impl ShareSource for OnlyPhrase {
    fn master_phrase(&self) -> Option<Zeroizing<String>> {
        Some(Zeroizing::new(self.0.into()))
    }

    fn second_share(
        &self,
        _: &HostUid,
        _: u64,
        _: &Share,
    ) -> std::result::Result<Share, UnlockError> {
        Err(UnlockError::NeedsShares("no peer".into()))
    }
}

#[test]
fn the_exempt_device_opens_through_the_unlocker_with_the_phrase_and_no_peer() {
    let d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    append(
        &d[0],
        crate::sync::exemption::enable_body(&d[0].id, PHRASE, FAST),
    );
    all_to_all(&d);
    let host = [91u8; 16];
    cred_put(&d[1].store, &d[1].id, &host, &payload("exempt-secret")).unwrap();
    all_to_all(&d);

    let opened = unlocker(&d[0], Box::new(OnlyPhrase(PHRASE)), None)
        .open_one(&host)
        .unwrap();
    assert_eq!(
        CredPayload::from_bytes(&opened)
            .unwrap()
            .password
            .as_deref(),
        Some("exempt-secret")
    );
    // Wrong phrase on the exempt device, or the right phrase on any other device: nothing opens.
    assert!(unlocker(
        &d[0],
        Box::new(OnlyPhrase("not the right words at all")),
        None
    )
    .open_one(&host)
    .is_err());
    assert!(unlocker(&d[1], Box::new(OnlyPhrase(PHRASE)), None)
        .open_one(&host)
        .is_err());
}

/// `n` terminals plus a phone (the last device): the shape the Red ceremony needs.
pub(crate) fn ring_phone(n: usize) -> Vec<Dev> {
    use crate::sync::oplog::{Attendance, DeviceKind, DeviceRole, VerifiedBy};
    let devs: Vec<Dev> = (0..=n).map(|_| Dev::new()).collect();
    for d in &devs {
        crate::sync::ceremony::test_force(&d.store, false);
    }
    for (i, a) in devs.iter().enumerate() {
        for (j, b) in devs.iter().enumerate() {
            if i != j {
                let kind = if j == n {
                    DeviceKind::Phone
                } else {
                    DeviceKind::Terminal
                };
                a.trust_with(b, Attendance::Attended, kind);
            }
        }
        a.store
            .sync_register_device(
                &a.id.public(),
                "self",
                DeviceRole::Holder,
                Attendance::Attended,
                if i == n {
                    DeviceKind::Phone
                } else {
                    DeviceKind::Terminal
                },
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    devs
}

/// Finish the ceremony of `op` as the ring would: the phone signs (unless it wrote the op), terminal
/// `second` approves, and with `paper` the paper factor signs too. Everyone has the op first.
pub(crate) fn complete(d: &[Dev], op: &Op, second: usize, paper: Option<&str>) {
    use crate::sync::ceremony::{red_approve_body, red_factor_paper, red_phone_sign_body};
    use crate::sync::crypto::{paper_derive, PaperDomain};
    let phone = d.len() - 1;
    all_to_all(d);
    if op.device_id != d[phone].did {
        append(&d[phone], red_phone_sign_body(op));
    }
    append(&d[second], red_approve_body(op));
    if let Some(phrase) = paper {
        let recover = paper_derive(phrase, PaperDomain::Recover).unwrap();
        append(&d[second], red_factor_paper(&recover, op));
    }
    all_to_all(d);
}
