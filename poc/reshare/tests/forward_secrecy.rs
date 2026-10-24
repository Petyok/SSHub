//! PoC B claims B1 to B4 at ring level, and the forward secrecy question B6.
//!
//! B6: resharing keeps the same DEK, so every OLD wrapped share stays in the append-only log
//! forever. If a holder's long-lived wrapping key can still open its old blob, then
//! "revoked device's old share + a later theft of one remaining holder's key" yields the DEK.
//! Rotating wrapping keys per reshare epoch and retiring the old generation closes that for
//! software holders; holders that cannot rotate (paper, a phone that never came online) keep
//! the exposure.

use poc_reshare::*;
use sshub::sync::crypto::*;

fn ring(n: u8) -> Vec<Holder> {
    (1..=n).map(Holder::new).collect()
}

fn refs(v: &[Holder]) -> Vec<&Holder> {
    v.iter().collect()
}

/// The attacker's move: the revoked device's old share plus one old wrapped blob opened with a
/// stolen key. Returns true when the credential opens.
fn attack(old: &Credential, revoked_share: &Share, stolen: &Holder) -> bool {
    let blob = &old
        .wrapped
        .iter()
        .find(|(i, _)| *i == stolen.index)
        .unwrap()
        .1;
    let Ok(bytes) = hybrid_unwrap(&stolen.id, blob) else {
        return false;
    };
    let other = Share {
        index: stolen.index,
        bytes,
    };
    let Ok(dek) = shamir_combine(&[revoked_share.clone(), other]) else {
        return false;
    };
    dek_open(&dek, &old.ciphertext).is_ok()
}

#[test]
fn b1_to_b4_at_ring_level() {
    let mut h = ring(4);
    let (cred, _) = create_credential(&refs(&h), b"hunter2");
    let mut t = Timing::default();
    // revoke holder 4: holders 1 and 2 contribute, 1, 2, 3 are the new set
    let new_set: Vec<&Holder> = h[..3].iter().collect();
    let r = reshare_v1(&cred, [&h[0], &h[1]], &new_set, &mut t).unwrap();
    // B1: every pair of the new holders opens the credential
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert_eq!(unlock(&r.new_credential, &h[a], &h[b]).unwrap(), b"hunter2");
    }
    // B2: the revoked holder's old share mixed with a new share opens nothing
    let old4 = open_share(&h[3], &cred).unwrap();
    for n in &new_set {
        let new_share = open_share(n, &r.new_credential).unwrap();
        let got = shamir_combine(&[old4.clone(), new_share]).unwrap();
        assert!(
            dek_open(&got, &cred.ciphertext).is_err(),
            "old + new share opened the credential"
        );
    }
    // B4: a missing contribution never leaves the ring without an openable credential
    h.truncate(4);
    assert_eq!(
        unlock(&cred, &h[0], &h[1]).unwrap(),
        b"hunter2",
        "old set still opens after an aborted reshare"
    );
}

/// B6a: without rotation the attack SUCCEEDS. This is the finding, asserted so it cannot be forgotten.
#[test]
fn b6a_without_rotation_an_old_blob_plus_a_stolen_key_opens_everything() {
    let h = ring(4);
    let (cred, dek) = create_credential(&refs(&h), b"hunter2");
    let revoked_share = open_share(&h[3], &cred).unwrap(); // the device later revoked
    let new_set: Vec<&Holder> = h[..3].iter().collect();
    let mut t = Timing::default();
    let _after = reshare_v1(&cred, [&h[0], &h[1]], &new_set, &mut t).unwrap();
    // revocation done; later the attacker steals holder 2's wrapping key from its disk
    assert!(
        attack(&cred, &revoked_share, &h[1]),
        "expected the attack to work without key rotation"
    );
    let _ = dek;
}

/// B6b: rotate before the reshare, retire after it: the same attack fails.
#[test]
fn b6b_rotation_and_retirement_close_the_hole_for_software_holders() {
    let mut h = ring(4);
    let (cred, _) = create_credential(&refs(&h), b"hunter2");
    let revoked_share = open_share(&h[3], &cred).unwrap();
    // step 1: every remaining holder rotates its wrapping keys (and publishes the new public half)
    for holder in h[..3].iter_mut() {
        holder.id.rotate_wrap_keys();
    }
    // step 2: reshare to the rotated keys
    let new_set: Vec<&Holder> = h[..3].iter().collect();
    let mut t = Timing::default();
    let after = reshare_v1(&cred, [&h[0], &h[1]], &new_set, &mut t).unwrap();
    // during the window (old keys still held) the old blobs still open, and so does the new set
    for (a, b) in [(0, 1), (1, 2)] {
        assert_eq!(
            unlock(&after.new_credential, &h[a], &h[b]).unwrap(),
            b"hunter2"
        );
    }
    assert!(
        attack(&cred, &revoked_share, &h[1]),
        "inside the retirement window the old key still opens the old blob"
    );
    // step 3: the new set is confirmed, so every holder retires its previous generation
    let image_before_retirement_of_holder_2 = h[1].id.to_encrypted("pw");
    for holder in h[..3].iter_mut() {
        assert_eq!(holder.id.retire_previous_wrap_keys(), 1);
    }
    assert!(
        !attack(&cred, &revoked_share, &h[1]),
        "after retirement the stolen key must not open the old blob"
    );
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert_eq!(
            unlock(&after.new_credential, &h[a], &h[b]).unwrap(),
            b"hunter2",
            "the new set keeps working"
        );
    }
    // a disk image taken BEFORE retirement is the residual: it still holds the old generation
    let stolen_early =
        DeviceIdentity::from_encrypted(&image_before_retirement_of_holder_2, "pw").unwrap();
    assert_eq!(stolen_early.wrap_key_generations(), 2);
    let thief = Holder {
        id: stolen_early,
        index: h[1].index,
    };
    assert!(
        attack(&cred, &revoked_share, &thief),
        "residual: a copy of the key file taken before retirement still opens the old blob"
    );
}

/// B6c: a holder that cannot rotate (paper, a phone that never came online) keeps the exposure.
#[test]
fn b6c_a_holder_that_cannot_rotate_keeps_the_exposure() {
    let mut h = ring(4);
    // holder 3 stands for the paper share (wrapping key derived from a phrase: it cannot rotate)
    let (cred, _) = create_credential(&refs(&h), b"hunter2");
    let revoked_share = open_share(&h[3], &cred).unwrap();
    for i in [0, 1] {
        h[i].id.rotate_wrap_keys();
    }
    let new_set: Vec<&Holder> = h[..3].iter().collect();
    let mut t = Timing::default();
    reshare_v1(&cred, [&h[0], &h[1]], &new_set, &mut t).unwrap();
    for i in [0, 1] {
        h[i].id.retire_previous_wrap_keys();
    }
    assert!(
        !attack(&cred, &revoked_share, &h[1]),
        "rotating holders are protected"
    );
    assert!(
        attack(&cred, &revoked_share, &h[2]),
        "residual: theft of the non-rotating holder's key still combines with the revoked share"
    );
}

/// The V2 envelope construction reaches the same new shares as V1 and opens the credential.
#[test]
fn v2_epoch_envelopes_reshare_correctly_and_bind_their_context() {
    let h = ring(4);
    let (cred, _) = create_credential(&refs(&h), b"hunter2");
    let new_set: Vec<&Holder> = h[..3].iter().collect();
    let mut t = Timing::default();
    let epoch = setup_epoch([&h[0], &h[1]], &new_set, &mut t).unwrap();
    let (contribs, adopts) =
        reshare_v2(&cred, &epoch, 1, [&h[0], &h[1]], &new_set, &mut t).unwrap();
    assert_eq!(contribs.len(), 2);
    let a = open_v2_share(&h[0], &epoch, 1, &cred.host_uid, &adopts[0].sealed_share).unwrap();
    let b = open_v2_share(&h[2], &epoch, 1, &cred.host_uid, &adopts[2].sealed_share).unwrap();
    let dek = shamir_combine(&[a, b]).unwrap();
    assert_eq!(dek_open(&dek, &cred.ciphertext).unwrap(), b"hunter2");
    // a sealed share moved to another credential or epoch does not open
    assert!(open_v2_share(&h[0], &epoch, 2, &cred.host_uid, &adopts[0].sealed_share).is_err());
    assert!(open_v2_share(&h[0], &epoch, 1, &random16(), &adopts[0].sealed_share).is_err());
}
