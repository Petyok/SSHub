use super::*;
use crate::sync::cred_ops::cred_put;
use crate::sync::crypto::dek_open;
use crate::sync::protocol::tests::{ring, Dev};
use crate::sync::quorum::tests::{all_to_all, append, payload, quorum_mode};
use crate::sync::quorum::{assemble_dek, own_shares};
use crate::sync::revocation::{next_actions, perform};

const TEST: Argon2Profile = Argon2Profile {
    mem_kib: 64,
    iters: 1,
};
const PHRASE: &str = "alpha bravo charlie delta echo foxtrot";

/// Run the membership reshare to completion on all devices.
fn settle_reshare(d: &mut [Dev]) {
    let now = crate::store::now_ts();
    for _ in 0..6 {
        let mut progressed = false;
        for dev in d.iter_mut() {
            for a in next_actions(&dev.store, &dev.id, now).unwrap() {
                if matches!(a, crate::sync::revocation::RevocationAction::Retire) {
                    continue;
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
        all_to_all(d);
        if !progressed {
            break;
        }
    }
}

fn opens(store: &LauncherStore, ct_host: &HostUid, a: Share, b: Share) -> bool {
    let Ok(dek) = assemble_dek(store, ct_host, a, b) else {
        return false;
    };
    let (ct, _) = store.sync_cred_ciphertext(ct_host).unwrap().unwrap();
    dek_open(&dek, &ct).is_ok()
}

#[test]
fn the_exempt_device_opens_with_phrase_and_device_key_and_nothing_less_does() {
    let mut d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    let old = [81u8; 16];
    cred_put(&d[1].store, &d[1].id, &old, &payload("old")).unwrap();
    all_to_all(&d);

    // Enable on d[0] (the ceremony is tested in ceremony/tests.rs); the ring reshares the old credential.
    append(&d[0], enable_body(&d[0].id, PHRASE, TEST));
    all_to_all(&d);
    assert_eq!(active_slots(&d[1].store).unwrap().len(), 1);
    assert!(d[1].store.sync_devices().unwrap().iter().any(|x| x.exempt));
    settle_reshare(&mut d);
    let fresh = [82u8; 16];
    cred_put(&d[1].store, &d[1].id, &fresh, &payload("fresh")).unwrap();
    all_to_all(&d);

    for (host, epoch) in [(old, 1u64), (fresh, 0u64)] {
        let own = own_shares(&d[0].store, &d[0].id, &host).unwrap();
        let (e, mine) = own.into_iter().find(|(e, _)| *e == epoch).unwrap();
        let slot = slot_share(&d[0].store, &d[0].id, PHRASE, &host, e, TEST).unwrap();
        assert!(
            opens(&d[0].store, &host, mine.clone(), slot),
            "device key + phrase open alone at epoch {e}"
        );
        // The device key alone is one share; the phrase alone derives a key nobody wrapped to.
        assert!(slot_share(
            &d[0].store,
            &d[0].id,
            "wrong words here now please",
            &host,
            e,
            TEST
        )
        .is_err());
        assert!(
            slot_share(&d[1].store, &d[1].id, PHRASE, &host, e, TEST).is_err(),
            "the phrase on another device"
        );
        assert!(slot_share(&d[2].store, &d[2].id, PHRASE, &host, e, TEST).is_err());
    }

    // Other devices are unaffected: still two shares from two holders.
    let a = own_shares(&d[1].store, &d[1].id, &fresh)
        .unwrap()
        .remove(0)
        .1;
    let b = own_shares(&d[2].store, &d[2].id, &fresh)
        .unwrap()
        .remove(0)
        .1;
    assert!(opens(&d[1].store, &fresh, a, b));
}

#[test]
fn disabling_removes_the_slot_from_new_credentials_and_reshares_without_it() {
    let mut d = ring(3);
    quorum_mode(&d[0]);
    all_to_all(&d);
    append(&d[0], enable_body(&d[0].id, PHRASE, TEST));
    all_to_all(&d);
    let a = [83u8; 16];
    cred_put(&d[1].store, &d[1].id, &a, &payload("a")).unwrap();
    all_to_all(&d);
    let slot_id = active_slots(&d[1].store).unwrap().remove(0).1.device_id();
    assert!(d[1]
        .store
        .sync_cred_ciphertext(&a)
        .unwrap()
        .unwrap()
        .1
        .iter()
        .any(|w| w.recipient == slot_id));

    let me = d[0].did.clone();
    append(&d[1], disable_body(&me));
    all_to_all(&d);
    assert!(active_slots(&d[1].store).unwrap().is_empty());
    let b = [84u8; 16];
    cred_put(&d[1].store, &d[1].id, &b, &payload("b")).unwrap();
    all_to_all(&d);
    assert!(!d[1]
        .store
        .sync_cred_ciphertext(&b)
        .unwrap()
        .unwrap()
        .1
        .iter()
        .any(|w| w.recipient == slot_id));
    settle_reshare(&mut d);
    // After the reshare the newest sharing of `a` no longer includes the slot.
    let epoch = own_shares(&d[1].store, &d[1].id, &a).unwrap().remove(0).0;
    assert!(epoch >= 1);
    assert!(slot_share(&d[0].store, &d[0].id, PHRASE, &a, epoch, TEST).is_err());
}
