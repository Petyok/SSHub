//! Stage 3 end to end over in-process stores: vault and quorum credentials, the "copy of one
//! device is not enough" criterion, challenges, resharing, certificates, mode changes, migration.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use super::*;
use crate::credentials::{ensure_unlocked, PasswordStore, QuorumGatedStore};
use crate::sync::cert::{cert_allows, validate_cert_issue, DelegationCert};
use crate::sync::cred_ops::{cred_put, materialise_key, migrate_local_secrets, remove_local_copies, CredPayload};
use crate::sync::crypto::{bip39_encode, shamir_combine, COMBINE_CALLS};
use crate::sync::modes::{mode_change_plan, validate_mode_change};
use crate::sync::oplog::{CertScope, CredMode};
use crate::sync::protocol::tests::{ring, Dev};
use crate::sync::reshare::{next_epoch, reissue_plan, reissue_receive, unadopted};
use crate::sync::share_release::unwrap_share_response;

const FAST: Argon2Profile = Argon2Profile { mem_kib: 64, iters: 1 };
const PHRASE: &str = "alpha bravo charlie delta echo foxtrot";
const RING: [u8; 16] = [3; 16];

fn approve_all(d: &Dev) {
    for op in d.store.sync_pending().unwrap() {
        d.store.sync_resolve_pending(&op.hash(), true).unwrap();
    }
}

/// What a sync round does: ingest every chain tail `to` lacks, then the user approves what waits.
fn deliver(from: &Dev, to: &Dev) {
    for (dev, _, _) in from.store.sync_chain_heads().unwrap() {
        let have = to.store.sync_chain_heads().unwrap().into_iter().find(|(d, _, _)| *d == dev).map_or(0, |(_, s, _)| s);
        for op in from.store.sync_chain_tail(&dev, have).unwrap() {
            to.store.sync_ingest(op).unwrap();
        }
    }
    approve_all(to);
}

fn all_to_all(devs: &[Dev]) {
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

fn append(d: &Dev, body: OpBody) -> Op {
    d.store.sync_append_local(&d.id, body).unwrap()
}

fn quorum_mode(d: &Dev) {
    append(d, OpBody::ModeChange { to: CredMode::Quorum, migrated_on: d.did.clone(), cosigner: None });
}

fn payload(pw: &str) -> CredPayload {
    CredPayload { password: Some(pw.into()), private_key: None }
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
    assert_eq!(open_payload(&d[1].store, &host, &dek).password.as_deref(), Some("hunter2"));
    // A copy of the data without the ring secret, or with the wrong phrase, opens nothing.
    assert!(vault_open_dek_with(&d[1].store, &host, PHRASE, &[4; 16], FAST).is_err());
    assert!(vault_open_dek_with(&d[1].store, &host, "alpha bravo charlie delta echo golf", &RING, FAST).is_err());
    assert!(vault_open_dek_with(&d[1].store, &[8; 16], PHRASE, &RING, FAST).is_err(), "another credential");
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
    assert!(assemble_dek(&copy.store, &host, share.clone(), share.clone()).is_err(), "the same share twice is not a quorum");
    let junk = Share { index: 9, bytes: vec![1; 32] };
    assert!(assemble_dek(&copy.store, &host, share.clone(), junk).is_err());
    assert!(shamir_combine(&[share.clone()]).is_err());
    assert!(vault_open_dek_with(&copy.store, &host, PHRASE, &RING, FAST).is_err(), "not a vault credential");

    // With a peer's share it opens.
    let (eph_secret, eph_pub) = {
        let s = x25519_dalek::StaticSecret::from([5u8; 32]);
        (s.to_bytes(), x25519_dalek::PublicKey::from(&s).to_bytes())
    };
    let reply = release_share(&d[1].store, &d[1].id, &host, 0, &eph_pub).unwrap();
    let second = unwrap_share_response(&eph_secret, &reply).unwrap();
    let dek = assemble_dek(&copy.store, &host, share.clone(), second).unwrap();
    assert_eq!(open_payload(&copy.store, &host, &dek).password.as_deref(), Some("s3cret"));
    // The release is bound to the requester's key: another key reads nothing.
    assert!(unwrap_share_response(&[6u8; 32], &reply).is_err());

    // With the paper share it opens as well.
    let paper = paper_derive(&paper_phrase, PaperDomain::Unwrap).unwrap();
    let paper_share = own_shares(&copy.store, &paper, &host).unwrap().remove(0).1;
    let dek = assemble_dek(&copy.store, &host, share.clone(), paper_share).unwrap();
    assert_eq!(open_payload(&copy.store, &host, &dek).password.as_deref(), Some("s3cret"));
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
    assert_eq!(x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(*secret)).to_bytes(), c.eph_x25519_pub);
}

#[test]
fn resharing_never_reconstructs_and_the_new_sharing_opens_while_a_removed_holders_old_share_does_not() {
    let mut d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let hosts: Vec<HostUid> = (1..=3u8).map(|n| [n; 16]).collect();
    for h in &hosts {
        cred_put(&d[0].store, &d[0].id, h, &payload(&format!("pw{}", h[0]))).unwrap();
    }
    all_to_all(&d);
    let removed_old = own_shares(&d[2].store, &d[2].id, &hosts[0]).unwrap().remove(0).1;

    let newcomer = Dev::new();
    for x in d.iter().chain(std::iter::once(&newcomer)) {
        for y in d.iter().chain(std::iter::once(&newcomer)) {
            if x.did != y.did && !x.store.sync_devices().unwrap().iter().any(|r| r.id == y.did) {
                x.trust(y);
            }
        }
    }
    d.push(newcomer);
    let (a, b, n) = (0, 1, 3);
    let holders = vec![d[a].did.clone(), d[b].did.clone(), d[n].did.clone()];
    let epoch = next_epoch(&d[a].store).unwrap();
    assert_eq!(epoch, 1);

    let combines = COMBINE_CALLS.load(Ordering::SeqCst);
    for who in [a, b] {
        for body in reissue_plan(&d[who].store, &d[who].id, &holders, epoch).unwrap().bodies {
            append(&d[who], body);
        }
    }
    all_to_all(&d);
    for who in [a, b, n] {
        for body in reissue_receive(&d[who].store, &d[who].id, epoch).unwrap().bodies {
            append(&d[who], body);
        }
    }
    all_to_all(&d);
    assert_eq!(COMBINE_CALLS.load(Ordering::SeqCst), combines, "resharing must never call shamir_combine");

    // Every new holder has a new share per credential, newest first.
    for h in &hosts {
        for who in [a, b, n] {
            let s = own_shares(&d[who].store, &d[who].id, h).unwrap();
            assert_eq!(s[0].0, 1, "the envelope share comes first");
        }
    }
    // Two new shares open the credential.
    let s_n = own_shares(&d[n].store, &d[n].id, &hosts[0]).unwrap().remove(0).1;
    let s_b = own_shares(&d[b].store, &d[b].id, &hosts[0]).unwrap().remove(0).1;
    let dek = assemble_dek(&d[n].store, &hosts[0], s_n.clone(), s_b.clone()).unwrap();
    assert_eq!(open_payload(&d[n].store, &hosts[0], &dek).password.as_deref(), Some("pw1"));
    // The removed holder's old share does not combine with a new one.
    assert!(assemble_dek(&d[n].store, &hosts[0], removed_old, s_b).is_err());

    // Adoption is recorded by a real unlock only.
    assert_eq!(unadopted(&d[n].store, epoch).unwrap().len(), 3);
    let gate = Arc::new(crate::credentials::SecretGate::default());
    let unlocker = QuorumUnlocker::with_profile(d[n].store.clone(), d[n].id.clone(), gate, Box::new(FromPeer { store: d[b].store.clone(), id: d[b].id.clone() }), None, FAST);
    let plain = unlocker.open_one(&hosts[1]).unwrap();
    assert_eq!(CredPayload::from_bytes(&plain).unwrap().password.as_deref(), Some("pw2"));
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

    fn second_share(&self, host: &HostUid, epoch: u64, _mine: &Share) -> std::result::Result<Share, UnlockError> {
        let s = x25519_dalek::StaticSecret::from([11u8; 32]);
        let reply = release_share(&self.store, &self.id, host, epoch, &x25519_dalek::PublicKey::from(&s).to_bytes())
            .map_err(|e| UnlockError::NeedsShares(e.to_string()))?;
        unwrap_share_response(&s.to_bytes(), &reply).map_err(|e| UnlockError::Failed(e.to_string()))
    }
}

#[test]
fn the_unlocker_fills_the_gate_for_a_real_host_row_and_nothing_else() {
    let d = ring(2);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let row = d[0].store.create_host(&crate::store::NewHost::launcher("web", "10.0.0.1")).unwrap();
    let uid = d[0].store.host_uid(row.id).unwrap().unwrap();
    cred_put(&d[0].store, &d[0].id, &uid, &payload("opened")).unwrap();
    all_to_all(&d);

    let gated = QuorumGatedStore::with_gate(Box::new(MemStore(Mutex::new(HashMap::new()))), Arc::new(crate::credentials::SecretGate::default()));
    let gate = gated.gate_handle();
    let unlocker = QuorumUnlocker::with_profile(d[0].store.clone(), d[0].id.clone(), gate.clone(), Box::new(FromPeer { store: d[1].store.clone(), id: d[1].id.clone() }), None, FAST);
    let key = crate::credentials::host_key(row.id);
    assert!(gate.is_synced(&key) && !gate.is_synced("host:99999"));
    assert!(gated.get(&key).unwrap_err().downcast_ref::<crate::credentials::Locked>().is_some(), "locked until opened");
    ensure_unlocked(&[SecretRef::Key(key.clone())], &unlocker).unwrap();
    assert_eq!(gated.get(&key).unwrap().as_deref(), Some("opened"));
    assert!(gated.set(&key, "x").is_err(), "a synced credential is not written through the local store");

    // Without a peer to release a share the unlock fails cleanly and the gate stays shut.
    let alone = Arc::new(crate::credentials::SecretGate::default());
    struct Nobody;
    impl ShareSource for Nobody {
        fn master_phrase(&self) -> Option<Zeroizing<String>> {
            None
        }
        fn second_share(&self, _: &HostUid, _: u64, _: &Share) -> std::result::Result<Share, UnlockError> {
            Err(UnlockError::NeedsShares("no peer".into()))
        }
    }
    let lonely = QuorumUnlocker::with_profile(d[0].store.clone(), d[0].id.clone(), alone.clone(), Box::new(Nobody), None, FAST);
    assert!(matches!(ensure_unlocked(&[SecretRef::Key(key.clone())], &lonely), Err(UnlockError::NeedsShares(_))));
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
    assert!(validate_cert_issue(&d[2].store, &op, &d[1].did).is_err(), "only the phone issues");
    let cert = DelegationCert::from_op(&op).unwrap();
    let pk = phone.id.public().ed25519_pub;
    let ok = |host: &HostUid, requester: &DeviceId, head_ts: i64, at: i64| cert_allows(&cert, &pk, &d[2].store, requester, host, head_ts, at);
    let at = op.timestamp + 10;
    assert!(ok(&host_a, &d[1].did, at, at).is_ok());
    assert!(ok(&host_c, &d[1].did, at, at).unwrap_err().to_string().contains("scope"));
    assert!(ok(&host_a, &d[2].did, at, at).unwrap_err().to_string().contains("another device"));
    assert!(ok(&host_a, &d[1].did, at - 15 * 86_400, at).unwrap_err().to_string().contains("phone"));
    assert!(ok(&host_a, &d[1].did, at, now + 8 * 86_400).unwrap_err().to_string().contains("expired"));
    assert!(cert_allows(&cert, &d[2].id.public().ed25519_pub, &d[2].store, &d[1].did, &host_a, at, at).is_err(), "wrong issuer key");

    // The budget is ring-wide: two different holders open two hosts, a third new host is refused,
    // an already opened one stays free.
    all_to_all(&d);
    append(&d[2], crate::sync::cert::cert_opened_body(&cert, &host_a));
    append(&d[3], crate::sync::cert::cert_opened_body(&cert, &host_b));
    all_to_all(&d);
    let later = at + 10;
    assert!(ok(&host_a, &d[1].did, later, later).is_ok(), "an opened host is free");
    let cert3 = {
        let op3 = append(
            phone,
            OpBody::CertIssue { device_id: d[1].did.clone(), scope: CertScope::Hosts(vec![host_a, host_b, host_c]), expiry: now + 7 * 86_400, max_new_hosts_per_day: 2, epoch_lo: 0, epoch_hi: 10 },
        );
        all_to_all(&d);
        DelegationCert::from_op(&op3).unwrap()
    };
    append(&d[2], crate::sync::cert::cert_opened_body(&cert3, &host_a));
    append(&d[3], crate::sync::cert::cert_opened_body(&cert3, &host_b));
    all_to_all(&d);
    let t = later + 20;
    assert!(cert_allows(&cert3, &pk, &d[2].store, &d[1].did, &host_c, t, t).unwrap_err().to_string().contains("budget"));

    // Revocation arrives with the next sync.
    append(phone, OpBody::CertRevoke { cert_id: cert.id });
    all_to_all(&d);
    assert!(ok(&host_a, &d[1].did, t, t).unwrap_err().to_string().contains("revoked"));

    // Fewer than three holders: nobody enforces it.
    let small = ring(2);
    let op = append(
        &small[0],
        OpBody::CertIssue { device_id: small[1].did.clone(), scope: CertScope::Hosts(vec![host_a]), expiry: now + 86_400, max_new_hosts_per_day: 5, epoch_lo: 0, epoch_hi: 1 },
    );
    assert!(validate_cert_issue(&small[1].store, &op, &small[0].did).unwrap_err().to_string().contains("three holders"));
}

#[test]
fn a_mode_change_re_encrypts_under_new_deks_and_going_back_needs_a_cosigner() {
    let d = ring(3);
    for x in &d {
        vault_setup_with(&x.store, PHRASE, &RING, FAST).unwrap();
    }
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
    let bodies = mode_change_plan(&d[0].store, &d[0].did, CredMode::Quorum, None, &open_vault).unwrap();
    assert_eq!(bodies.len(), 1 + hosts.len());
    for b in bodies {
        append(&d[0], b);
    }
    all_to_all(&d);
    assert_eq!(d[1].store.sync_cred_mode().unwrap(), CredMode::Quorum);
    let after = d[0].store.sync_cred_ciphertext(&hosts[0]).unwrap().unwrap();
    assert_ne!(before.0, after.0, "a fresh DEK, a fresh ciphertext");
    assert_eq!(after.1.len(), 3, "one share per holder");
    // The new shape opens with two shares, not with the vault.
    let s0 = own_shares(&d[0].store, &d[0].id, &hosts[0]).unwrap().remove(0).1;
    let s1 = own_shares(&d[1].store, &d[1].id, &hosts[0]).unwrap().remove(0).1;
    let dek = assemble_dek(&d[0].store, &hosts[0], s0, s1).unwrap();
    assert_eq!(open_payload(&d[0].store, &hosts[0], &dek).password.as_deref(), Some("v31"));
    assert!(vault_open_dek_with(&d[0].store, &hosts[0], PHRASE, &RING, FAST).is_err());

    // Back to Vault: a co-signer is required.
    let open_quorum = |h: &HostUid| -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let s0 = own_shares(&d[0].store, &d[0].id, h)?.remove(0).1;
        let s1 = own_shares(&d[1].store, &d[1].id, h)?.remove(0).1;
        let dek = assemble_dek(&d[0].store, h, s0, s1)?;
        let (ct, _) = d[0].store.sync_cred_ciphertext(h)?.unwrap();
        Ok(Zeroizing::new(dek_open(&dek, &ct)?))
    };
    assert!(mode_change_plan(&d[0].store, &d[0].did, CredMode::Vault, None, &open_quorum).is_err());
    let bodies = mode_change_plan(&d[0].store, &d[0].did, CredMode::Vault, Some(d[1].did.clone()), &open_quorum).unwrap();
    let mc = append(&d[0], bodies[0].clone());
    assert!(validate_mode_change(&d[1].store, &mc).is_ok());
    let alone = append(&d[0], OpBody::ModeChange { to: CredMode::Vault, migrated_on: d[0].did.clone(), cosigner: Some(d[0].did.clone()) });
    assert!(validate_mode_change(&d[1].store, &alone).is_err(), "cannot co-sign yourself");
    let nobody = append(&d[0], OpBody::ModeChange { to: CredMode::Vault, migrated_on: d[0].did.clone(), cosigner: None });
    assert!(validate_mode_change(&d[1].store, &nobody).is_err());
}

#[test]
fn payload_is_strict_keys_materialise_with_a_private_mode_and_vanish() {
    let p = CredPayload { password: Some("p".into()), private_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----".into()) };
    assert_eq!(CredPayload::from_bytes(&p.to_bytes()).unwrap(), p);
    assert!(CredPayload::from_bytes(br#"{"v":2}"#).is_err());
    assert!(CredPayload::from_bytes(br#"{"v":1,"extra":1}"#).is_err());
    assert!(CredPayload::from_bytes(b"[]").is_err());
    let dir = std::env::temp_dir().join(format!("sshub-mat-{}", std::process::id()));
    let uid = [5u8; 16];
    let path = {
        let k = materialise_key(&p, &dir, &uid).unwrap().unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(k.path()).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_to_string(k.path()).unwrap().ends_with("KEY-----\n"));
        k.path().to_path_buf()
    };
    assert!(!path.exists(), "the file goes with the guard");
    assert!(materialise_key(&payload("only a password"), &dir, &uid).unwrap().is_none());
    let _ = std::fs::remove_dir_all(&dir);
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
    let h = d[0].store.create_host(&crate::store::NewHost::launcher("db", "10.0.0.9")).unwrap();
    let i = d[0].store.create_identity(&crate::store::NewIdentity { name: "me".into(), ..Default::default() }).unwrap();
    let other = d[0].store.create_host(&crate::store::NewHost::launcher("nopw", "10.0.0.8")).unwrap();
    let keyfile = std::env::temp_dir().join(format!("sshub-key-{}", std::process::id()));
    std::fs::write(&keyfile, "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----\n").unwrap();
    d[0].store
        .with_conn(|c| {
            c.execute("UPDATE hosts SET has_password = 1 WHERE id = ?1", [h.id])?;
            c.execute("UPDATE identities SET private_key = ?2 WHERE id = ?1", rusqlite::params![i.id, keyfile.to_string_lossy()])?;
            Ok(())
        })
        .unwrap();
    let local = MemStore(Mutex::new(HashMap::new()));
    local.set(&crate::credentials::host_key(h.id), "dbpass").unwrap();
    let report = migrate_local_secrets(&d[0].store, &d[0].id, &local).unwrap();
    assert_eq!(report.migrated, vec![crate::credentials::host_key(h.id), crate::credentials::identity_key(i.id)]);
    assert!(local.get(&crate::credentials::host_key(h.id)).unwrap().is_some(), "READ, not deleted");

    let uid = d[0].store.host_uid(h.id).unwrap().unwrap();
    let dek = vault_open_dek_with(&d[0].store, &uid, PHRASE, &RING, FAST).unwrap();
    assert_eq!(open_payload(&d[0].store, &uid, &dek).password.as_deref(), Some("dbpass"));
    let iuid = d[0].store.identity_uid(i.id).unwrap().unwrap();
    let dek = vault_open_dek_with(&d[0].store, &iuid, PHRASE, &RING, FAST).unwrap();
    assert!(open_payload(&d[0].store, &iuid, &dek).private_key.unwrap().contains("key"));

    remove_local_copies(&local, &report).unwrap();
    assert!(local.get(&crate::credentials::host_key(h.id)).unwrap().is_none());
    let _ = std::fs::remove_file(keyfile);
}
