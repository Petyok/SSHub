//! SAS enrollment: the honest path, declines, cheating reveals, and the man-in-the-middle harness.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::sync::protocol::tests::{pipe_pair, Dev, PipeLink};
use crate::sync::protocol::{run_round, run_sync_serve};
use crate::sync::ratchet::load_pair;

type Seen = Arc<Mutex<Option<String>>>;

fn recorder() -> (Seen, impl FnMut(&str, &str) -> bool + Send) {
    let cell = Arc::new(Mutex::new(None));
    let c2 = cell.clone();
    (cell, move |sas: &str, _name: &str| {
        *c2.lock().unwrap() = Some(sas.to_string());
        true
    })
}

fn enroll(
    existing: &Dev,
    joining: &Dev,
    ui_e: &mut (dyn SasConfirm + Send),
    ui_j: &mut (dyn SasConfirm + Send),
) -> (Result<EnrollOutcome>, Result<EnrollOutcome>) {
    let (mut cj, mut ce) = pipe_pair();
    std::thread::scope(|sc| {
        let h = sc.spawn(|| {
            run_sync_enroll(
                &mut ce,
                &existing.store,
                &existing.id,
                "existing",
                true,
                ui_e,
            )
        });
        let j = run_sync_join(&mut cj, &joining.store, &joining.id, "joining", true, ui_j);
        drop(cj);
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
        let mut yes = |_: &str, _: &str| true;
        let mut no = |_: &str, _: &str| false;
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
    let mut ui = |_: &str, _: &str| true;
    let r = run_sync_join(&mut cj, &j.store, &j.id, "j", true, &mut ui);
    assert!(r.unwrap_err().to_string().contains("timeout"));
}

#[test]
fn a_reveal_that_does_not_open_its_commitment_is_refused() {
    let (e, j) = (Dev::new(), Dev::new());
    let (mut cj, mut ce) = pipe_pair();
    let mut ui = |_: &str, _: &str| true;
    let r = std::thread::scope(|sc| {
        let h = sc.spawn(|| run_sync_enroll(&mut ce, &e.store, &e.id, "e", true, &mut ui));
        // The joining side commits to one ephemeral key and reveals another.
        let mut s = Session::new(&mut cj, true);
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
        let ha = sc.spawn(|| run_sync_join(&mut a_link, &a.store, &a.id, "a", true, &mut ua));
        let hb = sc.spawn(|| run_sync_enroll(&mut b_link, &b.store, &b.id, "b", true, &mut ub));
        let mut toward_a = Session::new(&mut m_to_a, false);
        let mut toward_b = Session::new(&mut m_to_b, true);
        let m1 = Party::new(&m.id, "b", true); // what A believes is B
        let m2 = Party::new(&m.id, "a", true); // what B believes is A
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
        let mut ui = |_: &str, _: &str| true;
        let r = std::thread::scope(|sc| {
            let h = sc.spawn(|| run_sync_enroll(&mut ce, &e.store, &e.id, "e", true, &mut ui));
            let mut s = Session::new(&mut cj, true);
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
