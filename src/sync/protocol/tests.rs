//! Sync rounds between in-process stores over a pipe pair: honest, reordered, duplicated,
//! malformed and malicious peers; the ratchet and the transcript MAC.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::*;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::oplog::{Attendance, DeviceKind, DeviceRole, HostFields, OpBody, VerifiedBy};
use crate::sync::ratchet::{load_pair, save_pair};

type Hook = Box<dyn FnMut(&mut Value) -> bool + Send>;

/// One end of an in-memory frame pipe. `hook` sees every outgoing frame and may change or drop it.
pub(crate) struct PipeLink {
    tx: Sender<Value>,
    rx: Receiver<Value>,
    timeout: Duration,
    hook: Option<Hook>,
    sent: Arc<AtomicUsize>,
}

pub(crate) fn pipe_pair() -> (PipeLink, PipeLink) {
    let (t1, r1) = channel();
    let (t2, r2) = channel();
    let mk = |tx, rx| PipeLink {
        tx,
        rx,
        timeout: Duration::from_secs(20),
        hook: None,
        sent: Arc::new(AtomicUsize::new(0)),
    };
    (mk(t1, r2), mk(t2, r1))
}

impl PipeLink {
    pub(crate) fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    fn with_hook(mut self, h: impl FnMut(&mut Value) -> bool + Send + 'static) -> Self {
        self.hook = Some(Box::new(h));
        self
    }

    fn counter(&self) -> Arc<AtomicUsize> {
        self.sent.clone()
    }
}

impl SyncLink for PipeLink {
    fn send(&mut self, v: &Value) -> Result<()> {
        let mut v = v.clone();
        self.sent.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = &mut self.hook {
            if !h(&mut v) {
                return Ok(());
            }
        }
        self.tx.send(v).map_err(|_| anyhow!("pipe closed"))
    }

    fn recv(&mut self) -> Result<Value> {
        self.rx.recv_timeout(self.timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => anyhow!("pipe timeout"),
            RecvTimeoutError::Disconnected => anyhow!("pipe closed"),
        })
    }
}

pub(crate) struct Dev {
    pub store: Arc<LauncherStore>,
    pub id: Arc<DeviceIdentity>,
    pub did: DeviceId,
}

impl Dev {
    pub(crate) fn new() -> Self {
        Self::with(Arc::new(DeviceIdentity::generate()))
    }

    pub(crate) fn with(id: Arc<DeviceIdentity>) -> Self {
        let did = id.public().device_id();
        Self {
            store: Arc::new(LauncherStore::open_in_memory().unwrap()),
            id,
            did,
        }
    }

    pub(crate) fn trust(&self, other: &Dev) {
        self.trust_as(other, DeviceKind::Terminal);
    }

    /// This device's own row says it is unattended (what enrollment records for a VPS).
    pub(crate) fn make_unattended(&self) {
        self.store
            .with_conn(|c| {
                c.execute(
                    "UPDATE sync_devices SET attendance = 'unattended' WHERE device_id = ?1",
                    [&self.did],
                )?;
                Ok(())
            })
            .unwrap();
    }

    pub(crate) fn trust_as(&self, other: &Dev, kind: DeviceKind) {
        self.trust_with(other, Attendance::Attended, kind);
    }

    pub(crate) fn trust_with(&self, other: &Dev, attendance: Attendance, kind: DeviceKind) {
        self.store
            .sync_register_device(
                &other.id.public(),
                "peer",
                DeviceRole::Holder,
                attendance,
                kind,
                VerifiedBy::Sas,
            )
            .unwrap();
    }

    fn append(&self, n: u8, name: &str) -> Op {
        let fields = HostFields {
            name: Some(name.into()),
            ..Default::default()
        };
        self.store
            .sync_append_local(
                &self.id,
                OpBody::HostUpsert {
                    host_uid: [n; 16],
                    fields,
                },
            )
            .unwrap()
    }

    pub(crate) fn append_host(&self) -> Op {
        self.append(1, "h")
    }

    fn hash(&self) -> [u8; 32] {
        self.store.sync_state_hash().unwrap()
    }

    fn pair(&self, peer: &Dev) -> Option<PairState> {
        load_pair(&self.store, &peer.did).unwrap()
    }
}

pub(crate) fn ring(n: usize) -> Vec<Dev> {
    let devs: Vec<Dev> = (0..n).map(|_| Dev::new()).collect();
    for a in &devs {
        for b in &devs {
            if a.did != b.did {
                a.trust(b);
            }
        }
        // A device records itself when it enrolls into a ring.
        a.store
            .sync_register_device(
                &a.id.public(),
                "self",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    devs
}

type Pair = (Result<RoundSummary>, Result<RoundSummary>);

fn round_with<S>(c: &Dev, peer: &DeviceId, cl: PipeLink, sl: PipeLink, server: S) -> Pair
where
    S: FnOnce(&mut PipeLink) -> Result<RoundSummary> + Send,
{
    let mut cl = cl;
    let mut sl = sl;
    std::thread::scope(|sc| {
        let h = sc.spawn(move || server(&mut sl));
        let r = run_round(&mut cl, &c.store, &c.id, peer);
        drop(cl);
        (r, h.join().unwrap())
    })
}

fn round(c: &Dev, s: &Dev) -> Pair {
    let (cl, sl) = pipe_pair();
    round_with(c, &s.did, cl, sl, |l| run_sync_serve(&s.store, &s.id, l))
}

fn ok(p: Pair) -> (RoundSummary, RoundSummary) {
    (p.0.expect("client"), p.1.expect("server"))
}

/// A server that speaks the real protocol but lets the test rewrite the tails (and heads) it sends.
fn scripted_server(
    link: &mut PipeLink,
    s: &Dev,
    mut heads: impl FnMut(&mut HeadsMsg),
    mut tails: impl FnMut(&DeviceId, Vec<WireOp>) -> Vec<WireOp>,
) -> Result<RoundSummary> {
    let mut sum = RoundSummary::default();
    let mut sess = Session::new(link, false);
    let ctx = handshake(&mut sess, &s.store, &s.id, None, &mut None)?;
    sum.peer = ctx.peer.id.clone();
    let mut mine = heads_msg(&s.store, &s.id)?;
    heads(&mut mine);
    let theirs = match sess.recv()? {
        SyncMsg::Heads(h) => h,
        o => return sess.unexpected("heads", &o),
    };
    sess.send(&SyncMsg::Heads(mine))?;
    absorb_heads(&s.store, &theirs, &mut sum)?;
    loop {
        match sess.recv()? {
            SyncMsg::WantTail {
                device_id,
                after_seq,
            } => {
                let ops = s
                    .store
                    .sync_chain_tail(&device_id, after_seq)?
                    .iter()
                    .map(WireOp::of)
                    .collect();
                let ops = tails(&device_id, ops);
                sess.send(&SyncMsg::Tail { device_id, ops })?;
            }
            SyncMsg::EndWants => break,
            o => return sess.unexpected("want", &o),
        }
    }
    pull(&mut sess, &s.store, &theirs.heads, &mut sum)?;
    absorb_forks(&s.store, &theirs, &mut sum)?;
    finish(&mut sess, &s.store, ctx, &mut sum)?;
    Ok(sum)
}

fn scripted(
    c: &Dev,
    s: &Dev,
    heads: impl FnMut(&mut HeadsMsg) + Send,
    tails: impl FnMut(&DeviceId, Vec<WireOp>) -> Vec<WireOp> + Send,
) -> Pair {
    let (cl, sl) = pipe_pair();
    round_with(c, &s.did, cl, sl, move |l| {
        scripted_server(l, s, heads, tails)
    })
}

fn flaw(devs: &[Dev]) -> Vec<[u8; 32]> {
    devs.iter().map(Dev::hash).collect()
}

#[test]
fn two_stores_converge_and_the_ratchet_advances_every_round() {
    let d = ring(2);
    let (a, b) = (&d[0], &d[1]);
    for i in 0..5 {
        a.append(i, &format!("a{i}"));
        b.append(100 + i, &format!("b{i}"));
    }
    let (c, s) = ok(round(a, b));
    assert_eq!(a.hash(), b.hash());
    assert_eq!(
        (c.applied, c.received, s.applied, s.received),
        (5, 5, 5, 5),
        "{:?} {:?}",
        c.reject_reasons,
        s.reject_reasons
    );
    assert!(c.first_contact && s.first_contact && !c.epoch_advanced);
    assert_eq!(a.pair(b).unwrap().epoch, 0);
    assert_eq!(
        a.pair(b).unwrap().key,
        b.pair(a).unwrap().key,
        "both sides derived the same key"
    );

    a.append(50, "later");
    let (c, s) = ok(round(a, b));
    assert!(!c.first_contact && c.epoch_advanced && s.epoch_advanced);
    assert_eq!(a.hash(), b.hash());
    ok(round(b, a));
    let (pa, pb) = (a.pair(b).unwrap(), b.pair(a).unwrap());
    assert_eq!((pa.epoch, pb.epoch), (2, 2));
    assert_eq!(pa.key, pb.key);
    assert!(
        pb.next.is_none(),
        "the server committed after the acknowledgement"
    );
}

#[test]
fn a_third_device_learns_through_a_relay_and_all_converge() {
    let d = ring(3);
    for (i, x) in d.iter().enumerate() {
        x.append(i as u8 + 1, &format!("h{i}"));
    }
    ok(round(&d[0], &d[1]));
    ok(round(&d[1], &d[2]));
    ok(round(&d[0], &d[1]));
    assert_eq!(flaw(&d)[0], flaw(&d)[1]);
    assert_eq!(flaw(&d)[1], flaw(&d)[2]);
}

#[test]
fn a_long_tail_travels_in_several_frames() {
    let d = ring(2);
    for i in 0..(MAX_OPS_PER_TAIL + 300) {
        d[0].append((i % 200) as u8, &format!("n{i}"));
    }
    let (c, _) = ok(round(&d[1], &d[0]));
    assert_eq!(c.applied, MAX_OPS_PER_TAIL + 300);
    assert_eq!(d[0].hash(), d[1].hash());
}

#[test]
fn reordered_and_duplicated_tails_still_converge() {
    let d = ring(2);
    for i in 0..6 {
        d[0].append(i, &format!("x{i}"));
    }
    let (c, _) = ok(scripted(
        &d[1],
        &d[0],
        |_| {},
        |_, mut ops| {
            ops.reverse();
            let dup = ops.clone();
            ops.extend(dup);
            ops
        },
    ));
    assert_eq!(c.applied, 6, "{c:?}");
    assert_eq!(
        c.duplicates, 6,
        "the second copies are recognised, not applied twice"
    );
    assert_eq!(d[0].hash(), d[1].hash());
}

#[test]
fn malformed_input_is_an_error_and_changes_nothing() {
    type Mangle = Box<dyn Fn(&mut Vec<WireOp>) + Send + Sync>;
    let cases: Vec<(&str, Mangle)> = vec![
        (
            "not json",
            Box::new(|ops| ops[0].canonical = b"{{{{".to_vec()),
        ),
        ("empty", Box::new(|ops| ops[0].canonical.clear())),
        (
            "trailing",
            Box::new(|ops| ops[0].canonical.extend_from_slice(b" ")),
        ),
    ];
    for (name, f) in cases {
        let d = ring(2);
        d[0].append(1, "x");
        let before = d[1].hash();
        let (c, _) = scripted(
            &d[1],
            &d[0],
            |_| {},
            |_, mut ops| {
                f(&mut ops);
                ops
            },
        );
        let e = c.expect_err(name).to_string();
        assert!(e.contains("malformed op"), "{name}: {e}");
        assert_eq!(d[1].hash(), before, "{name}");
        assert!(d[1].pair(&d[0]).is_none(), "{name}: nothing committed");
    }
}

#[test]
fn caps_are_enforced_per_message() {
    let d = ring(2);
    d[0].append(1, "x");
    let (c, _) = scripted(
        &d[1],
        &d[0],
        |_| {},
        |_, ops| std::iter::repeat_n(ops[0].clone(), MAX_OPS_PER_TAIL + 1).collect(),
    );
    assert!(c.unwrap_err().to_string().contains("cap"));

    let d = ring(2);
    let bogus = HeadRef {
        device_id: "0123456789abcdef".into(),
        seq: 1,
        hash: [0; 32],
    };
    let (c, _) = scripted(
        &d[1],
        &d[0],
        |h| h.heads = vec![bogus.clone(); MAX_HEADS + 1],
        |_, o| o,
    );
    assert!(c.unwrap_err().to_string().contains("cap"));

    let d = ring(2);
    d[0].append(1, "x");
    let (c, _) = scripted(
        &d[1],
        &d[0],
        |_| {},
        |dev, ops| {
            if dev.is_empty() {
                ops
            } else {
                ops.into_iter()
                    .chain(std::iter::once(WireOp {
                        canonical: vec![],
                        sig: [0; 64],
                    }))
                    .collect()
            }
        },
    );
    assert!(c.is_err());
}

#[test]
fn garbage_frames_and_wrong_order_end_the_round() {
    let d = ring(2);
    for frame in [
        json!({"t": "nope"}),
        json!([1, 2]),
        json!({"t": "heads"}),
        json!({"t": "finish", "mac": "zz"}),
    ] {
        let (cl, sl) = pipe_pair();
        let f = frame.clone();
        let (c, _) = round_with(&d[0], &d[1].did, cl, sl, move |l| {
            l.send(&f)?;
            Err(anyhow!("sent garbage"))
        });
        let e = c.unwrap_err().to_string();
        assert!(
            e.contains("malformed frame") || e.contains("protocol violation"),
            "{frame}: {e}"
        );
    }

    // A valid message at the wrong point.
    let (cl, sl) = pipe_pair();
    let (c, _) = round_with(&d[0], &d[1].did, cl, sl, |l| {
        l.send(&serde_json::to_value(SyncMsg::EndWants)?)?;
        Err(anyhow!("x"))
    });
    assert!(c.unwrap_err().to_string().contains("protocol violation"));
}

#[test]
fn ops_of_a_device_only_the_peer_knows_are_refused_without_harm() {
    let d = ring(2);
    let x = Dev::new();
    d[0].trust(&x);
    x.append(9, "from a device only the server knows");
    d[0].store
        .sync_ingest(x.store.sync_chain_tail(&x.did, 0).unwrap().remove(0))
        .unwrap();
    d[0].append(1, "real");
    let (c, _) = ok(round(&d[1], &d[0]));
    assert_eq!(c.applied, 1);
    assert!(c.rejected >= 1);
    assert!(d[1]
        .store
        .sync_chain_heads()
        .unwrap()
        .iter()
        .all(|(dev, _, _)| *dev != x.did));
}

#[test]
fn a_forged_op_is_counted_as_rejected() {
    let d = ring(2);
    d[0].append(1, "real");
    let (c, _) = ok(scripted(
        &d[1],
        &d[0],
        |_| {},
        |_, ops| {
            ops.into_iter()
                .map(|mut w| {
                    w.sig[0] ^= 1;
                    w
                })
                .collect()
        },
    ));
    assert_eq!((c.applied, c.rejected), (0, 1));
    assert!(d[1].store.sync_chain_heads().unwrap().is_empty());
}

#[test]
fn equivocation_is_proven_between_peers_and_forwarded_to_a_third() {
    let d = ring(4);
    let (a, b, c, e) = (&d[0], &d[1], &d[2], &d[3]);
    a.append(1, "one");
    let honest = a.append(1, "two");
    let mut other = honest.clone();
    other.body = OpBody::HostUpsert {
        host_uid: [1; 16],
        fields: HostFields {
            notes: Some(Some("forked".into())),
            ..Default::default()
        },
    };
    other.signature = a.id.sign(&other.canonical_bytes());
    for dev in [b, c] {
        for op in a
            .store
            .sync_chain_tail(&a.did, 0)
            .unwrap()
            .into_iter()
            .take(1)
        {
            dev.store.sync_ingest(op).unwrap();
        }
    }
    b.store.sync_ingest(honest).unwrap();
    c.store.sync_ingest(other).unwrap();

    let (cs, ss) = ok(round(b, c));
    assert_eq!(cs.forks, vec![a.did.clone()]);
    assert_eq!(ss.forks, vec![a.did.clone()]);

    let (es, _) = ok(round(e, b));
    assert_eq!(
        es.forks,
        vec![a.did.clone()],
        "the proof travels and is checked by the receiver itself"
    );
    let row = e
        .store
        .sync_devices()
        .unwrap()
        .into_iter()
        .find(|r| r.id == a.did)
        .unwrap();
    assert_eq!(row.status, DeviceStatus::Compromised);
    // Compromised devices are not served any more.
    assert!(round(a, e).0.is_err());
}

#[test]
fn version_unknown_peer_and_impersonation_are_refused() {
    let d = ring(2);
    let stranger = Dev::new();

    // A stranger that only the client knows.
    let c = Dev::with(stranger.id.clone());
    c.trust(&d[0]);
    let (cr, sr) = round(&c, &d[0]);
    assert!(sr.unwrap_err().to_string().contains("not enrolled"));
    assert!(cr.is_err());

    // No common version.
    let (cl, sl) = pipe_pair();
    let (cr, sr) = {
        let mut sl = sl;
        let mut cl = cl;
        std::thread::scope(|sc| {
            let h = sc.spawn(|| run_sync_serve(&d[0].store, &d[0].id, &mut sl));
            let mut s = Session::new(&mut cl, true);
            let _ = s.recv();
            let hello = Hello {
                proto: 9,
                min_proto: 9,
                device_id: d[1].did.clone(),
                nonce: [1; 32],
                eph: [2; 32],
                checkpoint: None,
            };
            let r = s
                .send(&SyncMsg::Hello(hello))
                .and_then(|_| s.recv().map(|_| ()));
            drop(cl);
            (r, h.join().unwrap())
        })
    };
    assert!(sr
        .unwrap_err()
        .to_string()
        .contains("no common protocol version"));
    assert!(cr.is_err());

    // Claiming another device's id with a different key.
    let (mut cl, mut sl) = pipe_pair();
    let imp = Dev::new();
    let sr = std::thread::scope(|sc| {
        let h = sc.spawn(|| run_sync_serve(&d[0].store, &d[0].id, &mut sl));
        let mut s = Session::new(&mut cl, true);
        let _ = s.recv();
        let hello = Hello {
            proto: 1,
            min_proto: 1,
            device_id: d[1].did.clone(),
            nonce: [1; 32],
            eph: [2; 32],
            checkpoint: None,
        };
        let _ = s.send(&SyncMsg::Hello(hello));
        let msg = s.tr.labelled("auth", CLIENT);
        let _ = s.send(&SyncMsg::Auth {
            sig: imp.id.sign(&msg),
        });
        let _ = s.recv();
        drop(cl);
        h.join().unwrap()
    });
    assert!(sr.unwrap_err().to_string().contains("possession"));
}

/// Clone `a`'s pair state with `peer` onto a fresh store with the same identity: a disk copy.
fn disk_clone(a: &Dev, peer: &Dev) -> Dev {
    let c = Dev::with(a.id.clone());
    c.trust(peer);
    save_pair(&c.store, &peer.did, &a.pair(peer).unwrap()).unwrap();
    c
}

#[test]
fn a_clone_that_missed_one_round_is_behind_and_moves_nothing() {
    let d = ring(2);
    let (a, p) = (&d[0], &d[1]);
    ok(round(a, p));
    ok(round(a, p));
    let clone = disk_clone(a, p);
    clone.append(7, "written on the clone");
    ok(round(a, p)); // the original goes on: both are at epoch 3 now, the clone at 1
    let before = p.hash();
    let (cs, ss) = ok(round(&clone, p));
    assert_eq!(cs.behind, vec![p.did.clone()]);
    assert_eq!(ss.behind, vec![a.did.clone()]);
    assert_eq!(
        (cs.sent + cs.received + cs.applied, ss.applied),
        (0, 0),
        "no op moved"
    );
    assert_eq!(p.hash(), before);
    // The original is unaffected.
    let (c, _) = ok(round(a, p));
    assert!(c.epoch_advanced && c.behind.is_empty());
}

#[test]
fn whichever_copy_syncs_first_wins_and_the_other_is_behind() {
    let d = ring(2);
    let (a, p) = (&d[0], &d[1]);
    ok(round(a, p));
    ok(round(a, p));
    let clone = disk_clone(a, p);
    let (c, _) = ok(round(&clone, p));
    assert!(
        c.behind.is_empty() && c.epoch_advanced,
        "the first copy to arrive is accepted"
    );
    let (c, _) = ok(round(a, p));
    assert_eq!(
        c.behind,
        vec![p.did.clone()],
        "the original now looks stale: rehabilitate by enrolling again"
    );
}

#[test]
fn a_lost_acknowledgement_does_not_strand_an_honest_pair() {
    let d = ring(2);
    let (a, p) = (&d[0], &d[1]);
    ok(round(a, p));
    let (cl, sl) = pipe_pair();
    let cl = cl.with_hook(|v| v["t"] != "ack");
    let (cr, sr) = round_with(a, &p.did, cl, sl, |l| run_sync_serve(&p.store, &p.id, l));
    assert!(
        cr.is_ok() && sr.is_err(),
        "the client committed, the server never saw the acknowledgement"
    );
    assert_eq!((a.pair(p).unwrap().epoch, p.pair(a).unwrap().epoch), (1, 0));
    assert!(p.pair(a).unwrap().next.is_some());
    let (c, s) = ok(round(a, p));
    assert!(c.behind.is_empty() && s.behind.is_empty());
    assert_eq!((a.pair(p).unwrap().epoch, p.pair(a).unwrap().epoch), (2, 2));
}

#[test]
fn any_modified_frame_fails_the_round_and_commits_nothing() {
    let ids = [
        Arc::new(DeviceIdentity::generate()),
        Arc::new(DeviceIdentity::generate()),
    ];
    let fresh = || {
        let (a, p) = (Dev::with(ids[0].clone()), Dev::with(ids[1].clone()));
        a.trust(&p);
        p.trust(&a);
        a.append(1, "a");
        p.append(2, "p");
        ok(round(&a, &p));
        a.append(3, "more");
        p.append(4, "more");
        (a, p)
    };
    // Count the frames of an honest second round.
    let (a, p) = fresh();
    let (cl, sl) = pipe_pair();
    let (cn, sn) = (cl.counter(), sl.counter());
    ok(round_with(&a, &p.did, cl, sl, |l| {
        run_sync_serve(&p.store, &p.id, l)
    }));
    let (nc, ns) = (cn.load(Ordering::SeqCst), sn.load(Ordering::SeqCst));
    assert!(nc >= 8 && ns >= 8, "{nc} {ns}");

    for (client_side, n) in [(true, nc), (false, ns)] {
        for k in 0..n {
            if client_side && k == n - 1 {
                continue; // the acknowledgement closes the round; nothing follows it
            }
            let (a, p) = fresh();
            let (before_a, before_p) = (a.pair(&p).unwrap(), p.pair(&a).unwrap());
            let mut i = 0usize;
            let tamper = move |v: &mut Value| {
                if i == k {
                    // Semantically inert (unknown fields are ignored): only the transcript sees it.
                    v["x"] = json!(1);
                }
                i += 1;
                true
            };
            let (mut cl, mut sl) = pipe_pair();
            if client_side {
                cl = cl.with_hook(tamper);
            } else {
                sl = sl.with_hook(tamper);
            }
            let (cr, sr) = round_with(&a, &p.did, cl, sl, |l| run_sync_serve(&p.store, &p.id, l));
            let seen = |r: &Result<RoundSummary>| r.as_ref().map_or(true, |s| !s.behind.is_empty());
            assert!(
                seen(&cr) || seen(&sr),
                "frame {k} from the {} went unnoticed",
                if client_side { "client" } else { "server" }
            );
            // Neither side advanced past what the other can still verify.
            let (aa, pp) = (a.pair(&p).unwrap(), p.pair(&a).unwrap());
            assert!(
                aa.epoch <= before_a.epoch + 1 && pp.epoch == before_p.epoch,
                "frame {k}"
            );
        }
    }
}

// ------------------------------------------------- checkpoints over the wire

fn approve_all(d: &Dev) {
    loop {
        let pending = d.store.sync_pending().unwrap();
        if pending.is_empty() {
            return;
        }
        for op in pending {
            let _ = d.store.sync_resolve_pending(&op.hash(), true);
        }
    }
}

fn converge(devs: &[&Dev]) {
    for _ in 0..2 {
        for a in devs {
            for b in devs {
                if a.did != b.did {
                    ok(round(a, b));
                }
            }
        }
        for d in devs {
            approve_all(d);
        }
    }
}

fn many_ops(devs: &[&Dev], base: u8, n: u8) {
    for (i, d) in devs.iter().enumerate() {
        for k in 0..n {
            d.append(base + (i as u8) * 20 + k, &format!("h{base}-{i}-{k}"));
        }
    }
}

/// `proposer` proposes; the rounds carry the blob, the signs and the
/// acceptance (the driver runs at the end of every round).
fn checkpoint(devs: &[&Dev], proposer: usize, now: i64) -> [u8; 32] {
    converge(devs);
    let p = devs[proposer];
    let (op, _) = p.store.sync_checkpoint_propose(&p.id, None, now).unwrap();
    for _ in 0..3 {
        converge(devs);
    }
    for d in devs {
        let row = d
            .store
            .sync_checkpoints()
            .unwrap()
            .into_iter()
            .find(|c| c.proposal_hash == op.hash())
            .expect("every device learned the proposal");
        assert_eq!(
            row.status,
            crate::sync::oplog::CheckpointStatus::Accepted,
            "{} signers {:?} required {:?}; ops: {:?}",
            d.did,
            row.signers,
            row.required,
            d.store
                .with_conn(|c| {
                    let mut st = c.prepare(
                        "SELECT device_id, kind, state FROM sync_ops WHERE kind LIKE 'Checkpoint%'",
                    )?;
                    let rows = st.query_map([], |r| {
                        Ok(format!(
                            "{} {} {}",
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?
                        ))
                    })?;
                    Ok(rows.map(Result::unwrap).collect::<Vec<_>>())
                })
                .unwrap()
        );
    }
    op.hash()
}

/// Ring of `n` devices with two accepted checkpoints and a prune on device 0.
fn pruned_ring(n: usize, now: i64) -> (Vec<Dev>, crate::sync::oplog::PruneReport) {
    let devs = ring(n);
    let refs: Vec<&Dev> = devs.iter().collect();
    many_ops(&refs, 1, 4);
    checkpoint(&refs, 0, now);
    many_ops(&refs, 100, 4);
    checkpoint(&refs, 1, now + 100);
    many_ops(&refs, 150, 2);
    converge(&refs);
    let report = devs[0].store.sync_prune(now + 8 * 86_400 + 1_000).unwrap();
    (devs, report)
}

#[test]
fn a_fresh_store_bootstraps_over_the_wire_from_a_pruned_peer() {
    let now = crate::store::now_ts();
    let (devs, report) = pruned_ring(3, now);
    assert!(report.ops_deleted > 0, "{report:?}");
    let (a, b) = (&devs[0], &devs[1]);
    // The pruned peer cannot serve the old tails any more.
    let first = a.store.sync_chain_tail(&b.did, 0).unwrap_err();
    assert!(first.downcast_ref::<crate::sync::oplog::Pruned>().is_some());

    let f = Dev::new();
    f.trust(a);
    a.trust(&f);
    let (fs, _) = ok(round(&f, a));
    assert!(fs.adopted.is_some(), "{fs:?}");
    assert!(!fs.needs_checkpoint.is_empty());
    // Work after the adoption reaches the newcomer by tail.
    b.append(200, "after");
    converge(&[&devs[0], &devs[1], &devs[2]]);
    for _ in 0..2 {
        ok(round(&f, a));
        approve_all(&f);
    }
    assert_eq!(
        f.hash(),
        b.hash(),
        "same state as a store with the full history"
    );
    assert_eq!(
        f.store.sync_projection_hash().unwrap(),
        b.store.sync_projection_hash().unwrap()
    );
}

#[test]
fn a_peer_behind_past_the_floor_is_told_pruned_and_adopts_the_checkpoint() {
    let now = crate::store::now_ts();
    let (a, b, c) = (Dev::new(), Dev::new(), Dev::new());
    for x in [&a, &b, &c] {
        for y in [&a, &b, &c] {
            if x.did != y.did {
                x.trust(y);
            }
        }
    }
    // c writes once, long ago, and then goes quiet.
    c.store
        .sync_append_local_at(
            &c.id,
            OpBody::HostUpsert {
                host_uid: [90; 16],
                fields: HostFields {
                    name: Some("old".into()),
                    ..Default::default()
                },
            },
            now - 40 * 86_400,
        )
        .unwrap();
    converge(&[&a, &b, &c]);
    let live = [&a, &b];
    many_ops(&live, 1, 4);
    checkpoint(&live, 0, now);
    many_ops(&live, 100, 4);
    checkpoint(&live, 1, now + 100);
    many_ops(&live, 150, 2);
    converge(&live);
    assert!(
        a.store
            .sync_prune(now + 8 * 86_400 + 1_000)
            .unwrap()
            .ops_deleted
            > 0
    );

    let (cs, _) = ok(round(&c, &a));
    assert!(cs.adopted.is_some(), "{cs:?}");
    approve_all(&c);
    ok(round(&c, &a));
    approve_all(&c);
    assert_eq!(c.hash(), a.hash());
}

#[test]
fn a_lying_peers_checkpoint_chunk_fails_verification_and_changes_nothing() {
    let now = crate::store::now_ts();
    let (devs, _) = pruned_ring(3, now);
    let a = &devs[0];
    let f = Dev::new();
    f.trust(a);
    a.trust(&f);
    let (cl, sl) = pipe_pair();
    let mut flipped = false;
    let sl = sl.with_hook(move |v| {
        if v["t"] == "checkpoint_chunk" && !flipped {
            flipped = true;
            let data = v["data"].as_str().unwrap().to_string();
            let mut bytes = data.into_bytes();
            let i = bytes.len() / 2;
            bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
            v["data"] = Value::String(String::from_utf8(bytes).unwrap());
        }
        true
    });
    let pair = round_with(&f, &a.did, cl, sl, |l| run_sync_serve(&a.store, &a.id, l));
    assert!(
        pair.0.is_err(),
        "the client must refuse the altered checkpoint"
    );
    assert!(f.store.sync_chain_heads().unwrap().is_empty());
    assert!(f.store.sync_checkpoints().unwrap().is_empty());
}

#[test]
fn an_offline_device_adopts_a_checkpoint_and_keeps_its_own_unsynced_ops_on_top() {
    let now = crate::store::now_ts();
    let (a, b, c) = (Dev::new(), Dev::new(), Dev::new());
    for x in [&a, &b, &c] {
        for y in [&a, &b, &c] {
            if x.did != y.did {
                x.trust(y);
            }
        }
    }
    c.store
        .sync_append_local_at(
            &c.id,
            OpBody::HostUpsert {
                host_uid: [90; 16],
                fields: HostFields {
                    name: Some("old".into()),
                    ..Default::default()
                },
            },
            now - 40 * 86_400,
        )
        .unwrap();
    converge(&[&a, &b, &c]);
    // c writes three more ops while it is offline.
    let mine: Vec<[u8; 32]> = (0..3)
        .map(|k| c.append(80 + k, &format!("offline{k}")).hash())
        .collect();
    let live = [&a, &b];
    many_ops(&live, 1, 4);
    checkpoint(&live, 0, now);
    many_ops(&live, 100, 4);
    checkpoint(&live, 1, now + 100);
    many_ops(&live, 150, 2);
    converge(&live);
    assert!(
        a.store
            .sync_prune(now + 8 * 86_400 + 1_000)
            .unwrap()
            .ops_deleted
            > 0
    );

    let (cs, _) = ok(round(&c, &a));
    assert!(cs.adopted.is_some(), "{cs:?}");
    // Its own ops are on top of the adopted base, unchanged.
    let tail: Vec<[u8; 32]> = c
        .store
        .sync_chain_tail(&c.did, 1)
        .unwrap()
        .iter()
        .map(|o| o.hash())
        .collect();
    assert_eq!(tail, mine);
    // Everyone ends in the same state as a peer that got them normally.
    for _ in 0..2 {
        for (x, y) in [(&c, &a), (&a, &b), (&b, &a), (&c, &a)] {
            ok(round(x, y));
        }
        for d in [&a, &b, &c] {
            approve_all(d);
        }
    }
    assert_eq!(c.hash(), b.hash());
    assert_eq!(a.hash(), b.hash());
    assert_eq!(
        c.store.sync_projection_hash().unwrap(),
        b.store.sync_projection_hash().unwrap()
    );
}

#[test]
fn an_own_chain_that_diverges_from_the_checkpoint_is_refused_naming_the_seq() {
    let now = crate::store::now_ts();
    let (devs, _) = pruned_ring(3, now);
    let a = &devs[0];
    // Another copy of device 2's key with a different first op: a fork of its own chain.
    let twin = Dev::with(devs[2].id.clone());
    twin.trust(a);
    let head = a
        .store
        .sync_checkpoint_head()
        .unwrap()
        .expect("a serves a checkpoint");
    let bytes = a.store.sync_checkpoint_bytes(&head).unwrap().unwrap();
    let bundle = crate::sync::oplog::checkpoint::bundle_from_bytes(&bytes).unwrap();
    let OpBody::CheckpointPropose { cut, .. } = &bundle.proposal.body else {
        unreachable!()
    };
    let cut_seq = cut.iter().find(|c| c.device_id == twin.did).unwrap().seq;
    // The twin wrote its own chain up to the cut: other ops, same seqs.
    for k in 0..cut_seq {
        twin.append(70 + (k as u8), &format!("twin{k}"));
    }
    let err = twin
        .store
        .sync_checkpoint_accept(&bytes, &a.did, true)
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("diverges") && msg.contains(&format!("seq {cut_seq}")),
        "{msg}"
    );
    // Nothing changed: the twin's own op is still there.
    assert_eq!(
        twin.store.sync_chain_tail(&twin.did, 0).unwrap().len() as u64,
        cut_seq
    );
}
