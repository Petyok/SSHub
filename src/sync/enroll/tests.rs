//! SAS enrollment: the honest path, declines, cheating reveals, and the man-in-the-middle harness.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::sync::protocol::tests::{pipe_pair, Dev, PipeLink};
use crate::sync::protocol::{run_round, run_sync_serve};
use crate::sync::ratchet::load_pair;

type Seen = Arc<Mutex<Option<String>>>;

fn recorder() -> (Seen, impl FnMut(&SasPrompt<'_>) -> bool + Send) {
    let cell = Arc::new(Mutex::new(None));
    let c2 = cell.clone();
    (cell, move |p: &SasPrompt<'_>| {
        *c2.lock().unwrap() = Some(p.sas.to_string());
        true
    })
}

/// How the store recorded the admission of `device` (the replicated `DeviceRow.verified_by`).
fn admitted_as(store: &LauncherStore, device: &str) -> VerifiedBy {
    store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == device)
        .unwrap()
        .verified_by
}

fn hello(ring: Option<&str>, screen: bool) -> SyncMsg {
    SyncMsg::EnrollHello(EnrollHello {
        ring: ring.map(String::from),
        screen,
    })
}

/// `existing` is the responder and founds a ring first; `joining` is the blank initiator.
fn enroll(
    existing: &Dev,
    joining: &Dev,
    ui_e: &mut (dyn SasConfirm + Send),
    ui_j: &mut (dyn SasConfirm + Send),
) -> (Result<EnrollOutcome>, Result<EnrollOutcome>) {
    if ring_id(&existing.store).unwrap().is_none() {
        set_ring_id(&existing.store, &existing.did).unwrap();
    }
    enroll_raw(existing, joining, Some(ui_e), Some(ui_j))
}

/// `a` runs the responder end (`sync-enroll`), `b` the initiating end (`sync-join`).
fn enroll_raw(
    a: &Dev,
    b: &Dev,
    ui_a: Option<&mut (dyn SasConfirm + Send)>,
    ui_b: Option<&mut (dyn SasConfirm + Send)>,
) -> (Result<EnrollOutcome>, Result<EnrollOutcome>) {
    let (mut cb, mut ca) = pipe_pair();
    std::thread::scope(|sc| {
        let h = sc.spawn(|| {
            let ui = ui_a.map(|u| -> &mut dyn SasConfirm { u });
            run_sync_enroll(&mut ca, &a.store, &a.id, "existing", true, ui)
        });
        let ui = ui_b.map(|u| -> &mut dyn SasConfirm { u });
        let j = run_sync_join(&mut cb, &b.store, &b.id, "joining", true, ui);
        drop(cb);
        (h.join().unwrap(), j)
    })
}

#[test]
fn honest_enrollment_shows_the_same_words_and_leaves_a_working_pair() {
    let (e, j) = (Dev::new(), Dev::new());
    e.append_host();
    let ((se, mut ue), (sj, mut uj)) = (recorder(), recorder());
    let (re, rj) = enroll(&e, &j, &mut ue, &mut uj);
    let (re, rj) = (re.unwrap(), rj.unwrap());
    assert_eq!(*se.lock().unwrap(), *sj.lock().unwrap());
    assert_eq!(re.sas, rj.sas);
    assert_eq!(re.sas.split('-').count(), 4);
    assert_eq!(re.peer_name, "joining");
    // The joiner starts from the inviter's state: its chain is there and nothing is waiting for review.
    assert!(j
        .store
        .sync_chain_heads()
        .unwrap()
        .iter()
        .any(|(d, _, _)| *d == e.did));
    assert!(j.store.sync_pending().unwrap().is_empty());
    let ids: Vec<_> = j
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert!(ids.contains(&e.did) && ids.contains(&j.did), "{ids:?}");
    let (pe, pj) = (
        load_pair(&e.store, &j.did).unwrap().unwrap(),
        load_pair(&j.store, &e.did).unwrap().unwrap(),
    );
    assert_eq!((pe.epoch, pe.key), (pj.epoch, pj.key));
    // The new pair is at epoch 0, so the very next round already proves it and advances.
    let (cl, sl) = pipe_pair();
    let (c, s) = std::thread::scope(|sc| {
        let (mut cl, mut sl) = (cl, sl);
        let (es, ei) = (&e.store, &e.id);
        let h = sc.spawn(move || run_sync_serve(es, ei, &mut sl));
        let c = run_round(&mut cl, &j.store, &j.id, &e.did);
        drop(cl);
        (c, h.join().unwrap())
    });
    let (c, s) = (c.unwrap(), s.unwrap());
    assert!(!c.first_contact && !s.first_contact && c.epoch_advanced);
    assert_eq!(c.behind.len() + s.behind.len(), 0);
}

#[test]
fn a_declined_comparison_registers_nobody() {
    for decliner in [0, 1] {
        let (e, j) = (Dev::new(), Dev::new());
        let mut yes = |_: &SasPrompt<'_>| true;
        let mut no = |_: &SasPrompt<'_>| false;
        let (re, rj) = if decliner == 0 {
            enroll(&e, &j, &mut no, &mut yes)
        } else {
            enroll(&e, &j, &mut yes, &mut no)
        };
        assert!(re.is_err() && rj.is_err());
        assert!(e
            .store
            .sync_devices()
            .unwrap()
            .iter()
            .all(|d| d.id != j.did));
        assert!(j
            .store
            .sync_devices()
            .unwrap()
            .iter()
            .all(|d| d.id != e.did));
        assert!(load_pair(&e.store, &j.did).unwrap().is_none());
    }
}

#[test]
fn a_silent_peer_times_out() {
    let j = Dev::new();
    let (cj, _ce) = pipe_pair();
    let mut cj: PipeLink = cj.with_timeout(Duration::from_millis(150));
    let mut ui = |_: &SasPrompt<'_>| true;
    let r = run_sync_join(&mut cj, &j.store, &j.id, "j", true, Some(&mut ui));
    assert!(r.unwrap_err().to_string().contains("timeout"));
}

#[test]
fn a_reveal_that_does_not_open_its_commitment_is_refused() {
    let (e, j) = (Dev::new(), Dev::new());
    let (mut cj, mut ce) = pipe_pair();
    let mut ui = |_: &SasPrompt<'_>| true;
    let r = std::thread::scope(|sc| {
        let h = sc.spawn(|| run_sync_enroll(&mut ce, &e.store, &e.id, "e", true, Some(&mut ui)));
        // The joining side commits to one ephemeral key and reveals another.
        let mut s = Session::new(&mut cj, true);
        let _ = s.recv();
        s.send(&hello(None, true)).unwrap();
        let _ = s.recv();
        let committed = Party::new(&j.id, "j", true);
        let other = Party::new(&j.id, "j", true);
        s.send(&SyncMsg::EnrollCommit {
            commit: committed.commit(INITIATOR),
        })
        .unwrap();
        let _ = s.recv();
        s.send(&SyncMsg::EnrollReveal(other.reveal.clone()))
            .unwrap();
        drop(cj);
        h.join().unwrap()
    });
    assert!(r
        .unwrap_err()
        .to_string()
        .contains("does not open the commitment"));
    assert!(e
        .store
        .sync_devices()
        .unwrap()
        .iter()
        .all(|d| d.id != j.did));
}

#[test]
fn a_swapped_identity_key_breaks_the_commitment() {
    let (a, m) = (Dev::new(), Dev::new());
    let honest = Party::new(&a.id, "a", true);
    let c = honest.commit(INITIATOR);
    let mut swapped = honest.reveal.clone();
    swapped.ed25519_pub = m.id.public().ed25519_pub;
    assert!(check_reveal(INITIATOR, &c, &honest.reveal).is_ok());
    assert!(check_reveal(INITIATOR, &c, &swapped).is_err());
    let mut renamed = honest.reveal.clone();
    renamed.name = "someone else".into();
    assert!(
        check_reveal(INITIATOR, &c, &renamed).is_err(),
        "the displayed name is committed too"
    );
    assert!(
        check_reveal(RESPONDER, &c, &honest.reveal).is_err(),
        "roles are domain separated"
    );
}

/// Man in the middle between a joining device `a` (initiator) and an existing device `b`
/// (responder). It commits to its own keys toward each side before seeing either reveal, as it
/// must. Returns the SAS each human sees.
fn mitm_once(a: &Dev, b: &Dev, m: &Dev) -> (String, String) {
    let (mut a_link, mut m_to_a) = pipe_pair();
    let (mut m_to_b, mut b_link) = pipe_pair();
    let (sa, mut ua) = recorder();
    let (sb, mut ub) = recorder();
    std::thread::scope(|sc| {
        let ha = sc.spawn(|| run_sync_join(&mut a_link, &a.store, &a.id, "a", true, Some(&mut ua)));
        let hb =
            sc.spawn(|| run_sync_enroll(&mut b_link, &b.store, &b.id, "b", true, Some(&mut ub)));
        let mut toward_a = Session::new(&mut m_to_a, false);
        let mut toward_b = Session::new(&mut m_to_b, true);
        let m1 = Party::new(&m.id, "b", true); // what A believes is B
        let m2 = Party::new(&m.id, "a", true); // what B believes is A
        toward_a.send(&hello(None, true)).unwrap();
        let _ = toward_a.recv().unwrap();
        let _ = toward_b.recv().unwrap();
        toward_b.send(&hello(None, true)).unwrap();
        let _c_b = toward_b.recv().unwrap();
        toward_a
            .send(&SyncMsg::EnrollCommit {
                commit: m1.commit(RESPONDER),
            })
            .unwrap();
        let _c_a = toward_a.recv().unwrap();
        toward_b
            .send(&SyncMsg::EnrollCommit {
                commit: m2.commit(INITIATOR),
            })
            .unwrap();
        toward_a
            .send(&SyncMsg::EnrollReveal(m1.reveal.clone()))
            .unwrap();
        toward_b
            .send(&SyncMsg::EnrollReveal(m2.reveal.clone()))
            .unwrap();
        // Drain the real reveals; the attacker learns them only now.
        let _ = toward_a.recv();
        let _ = toward_b.recv();
        drop(m_to_a);
        drop(m_to_b);
        let _ = (ha.join(), hb.join());
    });
    let (x, y) = (
        sa.lock().unwrap().clone().unwrap(),
        sb.lock().unwrap().clone().unwrap(),
    );
    (x, y)
}

#[test]
fn key_substitution_never_yields_matching_words() {
    let (a, b, m) = (Dev::new(), Dev::new(), Dev::new());
    let n = 300;
    let mut equal = 0;
    for _ in 0..n {
        let (x, y) = mitm_once(&a, &b, &m);
        if x == y {
            equal += 1;
        }
    }
    eprintln!(
        "MITM key substitution through the full protocol: {equal} matching SAS in {n} attempts"
    );
    assert_eq!(equal, 0);
}

#[test]
fn grinding_after_the_commitment_is_caught_every_time() {
    let (e, j) = (Dev::new(), Dev::new());
    let attempts = 200;
    let mut refused = 0;
    for _ in 0..attempts {
        let (mut cj, mut ce) = pipe_pair();
        let mut ui = |_: &SasPrompt<'_>| true;
        let r = std::thread::scope(|sc| {
            let h =
                sc.spawn(|| run_sync_enroll(&mut ce, &e.store, &e.id, "e", true, Some(&mut ui)));
            let mut s = Session::new(&mut cj, true);
            let _ = s.recv().unwrap();
            s.send(&hello(None, true)).unwrap();
            let first = s.recv().unwrap();
            let SyncMsg::EnrollCommit { .. } = first else {
                panic!()
            };
            let committed = Party::new(&j.id, "j", true);
            s.send(&SyncMsg::EnrollCommit {
                commit: committed.commit(INITIATOR),
            })
            .unwrap();
            let SyncMsg::EnrollReveal(responder) = s.recv().unwrap() else {
                panic!()
            };
            // The attacker now sees the responder's reveal and searches for an ephemeral key and
            // nonce whose SAS it likes better, then reveals the better one.
            let mut best = Party::new(&j.id, "j", true);
            for _ in 0..64 {
                let cand = Party::new(&j.id, "j", true);
                if sas_of(&cand.reveal, &responder) < sas_of(&best.reveal, &responder) {
                    best = cand;
                }
            }
            s.send(&SyncMsg::EnrollReveal(best.reveal.clone())).unwrap();
            drop(cj);
            h.join().unwrap()
        });
        if r.is_err_and(|e| e.to_string().contains("does not open")) {
            refused += 1;
        }
    }
    eprintln!("grinding after the commitment: refused {refused} of {attempts}");
    assert_eq!(refused, attempts);
}

#[test]
fn sas_words_are_uniform_enough_and_never_collide_across_independent_draws() {
    let id = crate::sync::crypto::DeviceIdentity::generate();
    let n = 20_000usize;
    let draws: Vec<String> = (0..n)
        .map(|_| {
            let (i, r) = (Party::new(&id, "x", true), Party::new(&id, "y", true));
            sas_of(&i.reveal, &r.reveal)
        })
        .collect();
    let mut full = 0;
    let mut first_word = 0;
    for pair in draws.chunks(2) {
        full += usize::from(pair[0] == pair[1]);
        first_word += usize::from(pair[0].split('-').next() == pair[1].split('-').next());
    }
    // Two independent random strings of 44 bits agree with probability 2^-44; the first word with 2^-11.
    eprintln!("independent SAS draws: {} pairs, {full} full matches, {first_word} first-word matches (expected about {:.1})", n / 2, (n / 2) as f64 / 2048.0);
    assert_eq!(full, 0);
    assert!(first_word < 30);
}

#[test]
fn roles_follow_ring_membership_not_the_ssh_direction() {
    let ring = |r: &str| Some(r.to_string());
    // (mine, theirs, am_initiator) -> role
    assert_eq!(
        decide_role(&ring("x"), &None, false),
        Ok(EnrollRole::Inviter),
        "the remote end has the ring"
    );
    assert_eq!(
        decide_role(&ring("x"), &None, true),
        Ok(EnrollRole::Inviter)
    );
    assert_eq!(
        decide_role(&None, &ring("x"), true),
        Ok(EnrollRole::Joiner),
        "the initiator is blank and joins the remote ring"
    );
    assert_eq!(
        decide_role(&None, &ring("x"), false),
        Ok(EnrollRole::Joiner)
    );
    assert_eq!(
        decide_role(&None, &None, true),
        Ok(EnrollRole::Inviter),
        "of two blank devices the initiator founds the ring"
    );
    assert_eq!(decide_role(&None, &None, false), Ok(EnrollRole::Joiner));
    assert_eq!(
        decide_role(&ring("x"), &ring("x"), true),
        Ok(EnrollRole::Inviter),
        "same ring: a repeat ceremony"
    );
    assert!(decide_role(&ring("x"), &ring("y"), true)
        .unwrap_err()
        .contains("merging rings"));
}

#[test]
fn the_caller_with_a_ring_invites_the_blank_remote_end() {
    // The hosts-tab case: the local machine (initiator) is in a ring and enrolls a blank host.
    let (local, remote) = (Dev::new(), Dev::new());
    set_ring_id(&local.store, &local.did).unwrap();
    let (mut ul, mut ur) = (recorder().1, recorder().1);
    let (r_remote, r_local) = enroll_raw(&remote, &local, Some(&mut ur), Some(&mut ul));
    let (r_remote, r_local) = (r_remote.unwrap(), r_local.unwrap());
    assert_eq!(
        (r_local.role, r_remote.role),
        (EnrollRole::Inviter, EnrollRole::Joiner)
    );
    assert_eq!(
        ring_id(&remote.store).unwrap().as_deref(),
        Some(local.did.as_str()),
        "the blank end took the caller's ring"
    );
    let rows = local.store.sync_devices().unwrap();
    let row = rows.iter().find(|d| d.id == remote.did).unwrap();
    assert_eq!(row.role, DeviceRole::Mirror, "admitted as a mirror");
    assert_eq!(
        local.store.sync_chain_tail(&local.did, 0).unwrap().len(),
        1,
        "the DeviceAdd is the ring's record"
    );
    let rows = remote.store.sync_devices().unwrap();
    assert_eq!(
        rows.iter().find(|d| d.id == local.did).unwrap().role,
        DeviceRole::Holder
    );
}

#[test]
fn two_blank_devices_found_a_ring_at_the_initiator_and_two_rings_refuse() {
    let (a, b) = (Dev::new(), Dev::new());
    let (mut ua, mut ub) = (recorder().1, recorder().1);
    let (ra, rb) = enroll_raw(&a, &b, Some(&mut ua), Some(&mut ub)); // b initiates
    let (ra, rb) = (ra.unwrap(), rb.unwrap());
    assert_eq!(
        (rb.role, ra.role),
        (EnrollRole::Inviter, EnrollRole::Joiner)
    );
    assert_eq!(ring_id(&b.store).unwrap().as_deref(), Some(b.did.as_str()));
    assert_eq!(ring_id(&a.store).unwrap().as_deref(), Some(b.did.as_str()));
    assert_eq!(ra.ring, rb.ring);

    let (c, d) = (Dev::new(), Dev::new());
    set_ring_id(&c.store, "ring-one").unwrap();
    set_ring_id(&d.store, "ring-two").unwrap();
    let (mut uc, mut ud) = (recorder().1, recorder().1);
    let (rc, rd) = enroll_raw(&c, &d, Some(&mut uc), Some(&mut ud));
    assert!(rc.unwrap_err().to_string().contains("merging rings") && rd.is_err());
    assert!(c
        .store
        .sync_devices()
        .unwrap()
        .iter()
        .all(|x| x.id != d.did));
}

#[test]
fn a_side_without_a_screen_never_confirms_as_a_human_and_the_record_says_so() {
    // The remote end has no tty: the attended side is told the words prove nothing.
    let (remote, local) = (Dev::new(), Dev::new());
    set_ring_id(&local.store, &local.did).unwrap();
    let seen = Arc::new(Mutex::new(None));
    let s2 = seen.clone();
    let mut ul = move |p: &SasPrompt<'_>| {
        *s2.lock().unwrap() = Some(p.peer_has_screen);
        true
    };
    let (rr, rl) = enroll_raw(&remote, &local, None, Some(&mut ul));
    let (rr, rl) = (rr.unwrap(), rl.unwrap());
    assert_eq!(
        *seen.lock().unwrap(),
        Some(false),
        "the prompt knows the other side has no screen"
    );
    assert_eq!(
        (rl.verified_by, rr.verified_by),
        (VerifiedBy::SshHostKey, VerifiedBy::SshHostKey)
    );
    assert_eq!(
        admitted_as(&local.store, &remote.did),
        VerifiedBy::SshHostKey
    );

    // The attended user can still refuse, and nobody is registered.
    let (remote2, local2) = (Dev::new(), Dev::new());
    set_ring_id(&local2.store, &local2.did).unwrap();
    let mut no = |_: &SasPrompt<'_>| false;
    let (rr2, rl2) = enroll_raw(&remote2, &local2, None, Some(&mut no));
    assert!(rr2.is_err() && rl2.is_err());
    assert!(local2
        .store
        .sync_devices()
        .unwrap()
        .iter()
        .all(|x| x.id != remote2.did));

    // Two screens: a real SAS, recorded as such on both.
    let (e, j) = (Dev::new(), Dev::new());
    let ((_, mut ue), (_, mut uj)) = (recorder(), recorder());
    let (re, rj) = enroll(&e, &j, &mut ue, &mut uj);
    assert_eq!(
        (re.unwrap().verified_by, rj.unwrap().verified_by),
        (VerifiedBy::Sas, VerifiedBy::Sas)
    );
    assert_eq!(admitted_as(&e.store, &j.did), VerifiedBy::Sas);
}

#[test]
fn an_unattended_inviter_admits_a_mirror_the_ring_still_has_to_approve() {
    // The hub: it has a ring, no tty, and is unattended. The attended joiner connects to it.
    let (hub, joiner) = (Dev::new(), Dev::new());
    set_ring_id(&hub.store, &hub.did).unwrap();
    let mut uj = recorder().1;
    let (ra, rb) = {
        let (mut cb, mut ca) = pipe_pair();
        std::thread::scope(|sc| {
            let h = sc.spawn(|| run_sync_enroll(&mut ca, &hub.store, &hub.id, "hub", false, None));
            let j = run_sync_join(
                &mut cb,
                &joiner.store,
                &joiner.id,
                "laptop",
                true,
                Some(&mut uj),
            );
            drop(cb);
            (h.join().unwrap(), j)
        })
    };
    let (ra, rb) = (ra.unwrap(), rb.unwrap());
    assert_eq!(ra.role, EnrollRole::Inviter);
    assert_eq!(rb.verified_by, VerifiedBy::SshHostKey);
    let row = hub
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == joiner.did)
        .unwrap();
    assert_eq!(row.role, DeviceRole::Mirror, "never a holder by itself");
    // An attended ring member that receives the hub's DeviceAdd holds it as pending until approved.
    let member = Dev::new();
    member.trust(&hub);
    for op in hub.store.sync_chain_tail(&hub.did, 0).unwrap() {
        member.store.sync_ingest(op).unwrap();
    }
    assert_eq!(member.store.sync_pending().unwrap().len(), 1);
}
