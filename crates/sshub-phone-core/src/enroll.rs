//! Enrollment, the joining side (role 0): the phone enrolls with a terminal in four steps over any channel that carries
//! frames of kinds 6 to 8. Port of `enroll` .. `enrollShow` of `assets/companion/src/app.mjs`.
//!
//! 1. the terminal commits (kind 6, role 1); the person names this phone and the phone commits (kind 6, role 0);
//! 2. the terminal reveals its keys (kind 7, role 1), which must open its commitment and be usable; the phone reveals
//!    its own (kind 7, role 0) and shows the four SAS words;
//! 3. the person compares the words with the terminal's and answers; the phone sends a confirm (kind 8, role 0)
//!    signed over `enroll_core::confirm_message`, a declined one when they differ;
//! 4. the terminal's confirm (kind 8, role 1) is verified under the key it revealed; then it enters the ring.
//!
//! The phone is the initiator of the transcript (`transcript_hash(me, terminal)`). A frame out of order or hostile bytes
//! is refused with the page's sentence and the flow waits on (a stray frame in the camera must not end it); everything
//! that touches committed or signed content (a reveal that does not open its commitment, unusable keys, a bad or
//! declining confirm) ends the flow: the next frame must be a fresh commitment.

use sshub_sync_proto::crypto::{device_id_of, verify, DeviceIdentity};
use sshub_sync_proto::enroll_core::{commit_of, confirm_message, sas_of, transcript_hash, Reveal};
use sshub_sync_proto::wire::EnrollFrame;
use zeroize::Zeroizing;

use crate::ctx::Ctx;
use crate::error::{CoreError, Result};
use crate::outgoing::render;
use crate::state::{Enrolled, RingDevice};
use crate::text::safe_text;
use crate::views::{
    Decision, EnrollStep, EnrollView, EnrolledView, Opened, Outgoing, RefusalKind, RefusalView,
};

/// Transport roles of the frames (the same numbers as the commitment roles of `enroll_core`).
const ROLE_JOINING: u8 = 0;
const ROLE_EXISTING: u8 = 1;
/// A device name is at most this many bytes (`enroll_core::MAX_NAME_BYTES`; the terminal refuses longer ones).
const MAX_NAME_BYTES: usize = 64;

enum Stage {
    Idle,
    /// The terminal's commitment is in; waiting for the person to name this phone.
    NeedName {
        commit: [u8; 32],
    },
    /// This phone committed; waiting for the terminal's reveal.
    AwaitReveal {
        me: Reveal,
        commit: [u8; 32],
    },
    /// Both revealed; waiting for the person's answer on the SAS words.
    AwaitSas {
        me: Reveal,
        peer: Reveal,
        th: [u8; 32],
    },
    /// This phone confirmed; waiting for the terminal's confirm.
    AwaitConfirm {
        me: Reveal,
        peer: Reveal,
        th: [u8; 32],
    },
}

/// One enrollment at a time.
pub struct EnrollFlow {
    stage: Stage,
}

impl Default for EnrollFlow {
    fn default() -> Self {
        Self::new()
    }
}

fn refused(kind: RefusalKind, reason: impl Into<String>) -> Opened {
    Opened::Refused {
        view: RefusalView {
            kind,
            compared: Vec::new(),
            reason: reason.into(),
            checks: Vec::new(),
        },
    }
}

/// The name rules of the page (`enroll`): not empty, at most 64 bytes, no control characters (the terminal refuses a
/// reveal with one, so a name it would refuse is stopped here).
pub(crate) fn check_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(CoreError::bad("Give this phone a name."));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(CoreError::bad("The name is longer than 64 bytes."));
    }
    if name.chars().any(char::is_control) {
        return Err(CoreError::bad("The name has control characters."));
    }
    Ok(name.to_string())
}

/// This phone's reveal: its keys, a fresh ephemeral key and nonce (the page draws the ephemeral secret first, then the
/// nonce). The ephemeral public key is the X25519 public key of the secret, taken from the sealed box helper (the phone
/// never uses the shared secret, only the terminal does).
fn new_reveal(ctx: &mut Ctx, name: &str) -> Reveal {
    let mut eph_secret = Zeroizing::new([0u8; 32]);
    ctx.rng.fill_bytes(&mut eph_secret[..]);
    let mut nonce = [0u8; 32];
    ctx.rng.fill_bytes(&mut nonce);
    let p = ctx.id.public();
    let sealed = DeviceIdentity::seal_to_with(&p.x25519_pub, &[], &eph_secret, &[0u8; 24]);
    let mut eph = [0u8; 32];
    eph.copy_from_slice(&sealed[..32]);
    Reveal {
        ed25519_pub: p.ed25519_pub,
        x25519_pub: p.x25519_pub,
        mlkem_pub: p.mlkem_pub,
        eph,
        nonce,
        name: name.to_string(),
        attended: true,
    }
}

impl EnrollFlow {
    pub fn new() -> Self {
        Self { stage: Stage::Idle }
    }

    /// A frame from the terminal (kinds 6, 7, 8). The terminal's confirm completes the enrollment: the terminal enters
    /// `ctx.state.ring` and `ctx.state.enrolled` is set (the caller saves the state).
    pub fn on_frame(&mut self, frame: &[u8], ctx: &mut Ctx, token: u64) -> Opened {
        let msg = match EnrollFrame::decode(frame) {
            Ok(m) => m,
            Err(e) => {
                return refused(
                    RefusalKind::StrayFrame,
                    format!("Not an enrollment document: {e:#}"),
                )
            }
        };
        let (next, out) = match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Idle => match msg {
                EnrollFrame::Commit {
                    role: ROLE_EXISTING,
                    commit,
                } => (
                    Stage::NeedName { commit },
                    Opened::Enroll {
                        view: EnrollView {
                            token,
                            step: EnrollStep::Commit,
                            terminal_id: None,
                            terminal_name: None,
                            sas_words: None,
                            outgoing: None,
                        },
                    },
                ),
                _ => (
                    Stage::Idle,
                    refused(
                        RefusalKind::WrongOrder,
                        "Expected the terminal's code first.",
                    ),
                ),
            },
            stage @ Stage::NeedName { .. } => (
                stage,
                refused(RefusalKind::WrongOrder, "Name this phone first."),
            ),
            Stage::AwaitReveal { me, commit } => match msg {
                EnrollFrame::Reveal {
                    role: ROLE_EXISTING,
                    reveal,
                } => self.on_reveal(me, commit, &reveal, token),
                _ => (
                    Stage::AwaitReveal { me, commit },
                    refused(RefusalKind::WrongOrder, "Expected the terminal's keys."),
                ),
            },
            stage @ Stage::AwaitSas { .. } => (
                stage,
                refused(
                    RefusalKind::WrongOrder,
                    "Compare the four words and answer on this phone first.",
                ),
            ),
            Stage::AwaitConfirm { me, peer, th } => match msg {
                EnrollFrame::Confirm {
                    role: ROLE_EXISTING,
                    ok,
                    sig,
                } => {
                    let p = &peer.ed25519_pub;
                    if !verify(p, &confirm_message(ROLE_EXISTING, ok, &th), &sig) {
                        (
                            Stage::Idle,
                            refused(
                                RefusalKind::BadSignature,
                                "Enrollment failed: the terminal confirmation signature is bad",
                            ),
                        )
                    } else if !ok {
                        (
                            Stage::Idle,
                            refused(
                                RefusalKind::Declined,
                                "Enrollment failed: the terminal declined",
                            ),
                        )
                    } else {
                        let id = device_id_of(p);
                        ctx.state.ring.insert(
                            id.clone(),
                            RingDevice {
                                name: peer.name.clone(),
                                ed25519_pub: hex::encode(p),
                                enrolled_at: ctx.now,
                            },
                        );
                        ctx.state.enrolled = Some(Enrolled {
                            device_id: id.clone(),
                            name: me.name.clone(),
                            at: ctx.now,
                        });
                        let view = EnrolledView {
                            terminal_id: id,
                            terminal_name: safe_text(&peer.name),
                            phone_name: safe_text(&me.name),
                        };
                        (Stage::Idle, Opened::Enrolled { view })
                    }
                }
                _ => (
                    Stage::AwaitConfirm { me, peer, th },
                    refused(
                        RefusalKind::WrongOrder,
                        "Enrollment failed: not the terminal confirmation",
                    ),
                ),
            },
        };
        self.stage = next;
        out
    }

    fn on_reveal(&self, me: Reveal, commit: [u8; 32], bytes: &[u8], token: u64) -> (Stage, Opened) {
        let peer = match Reveal::from_bytes(bytes) {
            Ok(r) => r,
            Err(e) => {
                return (
                    Stage::AwaitReveal { me, commit },
                    refused(
                        RefusalKind::StrayFrame,
                        format!("Not an enrollment document: {e:#}"),
                    ),
                )
            }
        };
        let opens_to = commit_of(ROLE_EXISTING, &peer);
        if opens_to != commit {
            let mut o = refused(
                RefusalKind::Mismatch,
                "The terminal's keys do not match the code it showed first. Enrollment stopped: someone may be interfering.",
            );
            if let Opened::Refused { view } = &mut o {
                let show = |b: &[u8; 32]| {
                    hex::encode(&b[..16])
                        .to_uppercase()
                        .as_bytes()
                        .chunks(4)
                        .map(|c| String::from_utf8_lossy(c).into_owned())
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                view.compared = vec![
                    crate::views::Row {
                        label: "Code the terminal showed first".into(),
                        value: show(&commit),
                        danger: false,
                    },
                    crate::views::Row {
                        label: "Code its keys open to".into(),
                        value: show(&opens_to),
                        danger: true,
                    },
                ];
            }
            return (Stage::Idle, o);
        }
        if peer.public().validate().is_err() || peer.ed25519_pub == me.ed25519_pub {
            return (
                Stage::Idle,
                refused(
                    RefusalKind::UnusableKeys,
                    "The terminal presented unusable keys.",
                ),
            );
        }
        let th = transcript_hash(&me, &peer);
        let sas = sas_of(&me, &peer);
        let outgoing = match render(
            EnrollFrame::Reveal {
                role: ROLE_JOINING,
                reveal: me.bytes(),
            }
            .encode(),
            true,
            "Show this to the terminal, then compare the four words on both screens.",
        ) {
            Ok(o) => o,
            Err(e) => return (Stage::Idle, refused(RefusalKind::Other, e.reason())),
        };
        let view = EnrollView {
            token,
            step: EnrollStep::Reveal,
            terminal_id: Some(device_id_of(&peer.ed25519_pub)),
            terminal_name: Some(safe_text(&peer.name)),
            sas_words: Some(sas),
            outgoing: Some(outgoing),
        };
        (Stage::AwaitSas { me, peer, th }, Opened::Enroll { view })
    }

    /// `EnrollName` after the terminal's commitment, `EnrollSas` after its reveal.
    pub fn decide(&mut self, d: &Decision, ctx: &mut Ctx) -> Result<Outgoing> {
        match (d, std::mem::replace(&mut self.stage, Stage::Idle)) {
            (Decision::EnrollName { name }, Stage::NeedName { commit }) => {
                let name = match check_name(name) {
                    Ok(n) => n,
                    Err(e) => {
                        self.stage = Stage::NeedName { commit };
                        return Err(e);
                    }
                };
                let me = new_reveal(ctx, &name);
                let frame = EnrollFrame::Commit {
                    role: ROLE_JOINING,
                    commit: commit_of(ROLE_JOINING, &me),
                }
                .encode();
                self.stage = Stage::AwaitReveal { me, commit };
                render(frame, true, "Hold this to the terminal camera until it confirms, then scan the terminal's keys.")
            }
            (Decision::EnrollSas { matches }, Stage::AwaitSas { me, peer, th }) => {
                let ok = *matches;
                let sig = ctx.id.sign(&confirm_message(ROLE_JOINING, ok, &th));
                let frame = EnrollFrame::Confirm {
                    role: ROLE_JOINING,
                    ok,
                    sig,
                }
                .encode();
                if ok {
                    self.stage = Stage::AwaitConfirm { me, peer, th };
                    render(
                        frame,
                        true,
                        "Hold it to the terminal camera, then scan the terminal confirmation.",
                    )
                } else {
                    render(
                        frame,
                        true,
                        "Enrollment declined. Show this to the terminal so it stops too.",
                    )
                }
            }
            (Decision::EnrollName { .. } | Decision::EnrollSas { .. }, stage) => {
                self.stage = stage;
                Err(CoreError::bad(
                    "That answer does not belong to this step of the enrollment.",
                ))
            }
            (_, stage) => {
                self.stage = stage;
                Err(CoreError::bad("Not an enrollment decision."))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PhoneIdentity;
    use crate::state::PhoneState;
    use rand_chacha::rand_core::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use sshub_sync_proto::enroll_core::{terminal_confirm_message, Party};

    struct Rig {
        id: PhoneIdentity,
        state: PhoneState,
        rng: ChaCha8Rng,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                id: PhoneIdentity::new(&[0x42; 32], 0).unwrap(),
                state: PhoneState::default(),
                rng: ChaCha8Rng::seed_from_u64(7),
            }
        }
        fn ctx(&mut self) -> Ctx<'_> {
            Ctx {
                id: &mut self.id,
                state: &mut self.state,
                rng: &mut self.rng,
                now: 1_700_000_000,
            }
        }
    }

    fn fx() -> serde_json::Value {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/companion/test/fixtures/enroll.json"
        );
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn hx(v: &serde_json::Value) -> Vec<u8> {
        hex::decode(v.as_str().unwrap()).unwrap()
    }

    fn reason(o: &Opened) -> &str {
        match o {
            Opened::Refused { view } => &view.reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn kind(o: &Opened) -> RefusalKind {
        match o {
            Opened::Refused { view } => view.kind,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn enroll_view(o: Opened) -> EnrollView {
        match o {
            Opened::Enroll { view } => view,
            other => panic!("expected an enroll view, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------- the fixture of the terminal side

    #[test]
    fn the_whole_enrollment_against_the_terminal_fixture() {
        let f = fx();
        let frames: Vec<Vec<u8>> = f["frames_hex"].as_array().unwrap().iter().map(hx).collect();
        let joining = Reveal::from_bytes(&hx(&f["joining"]["reveal_hex"])).unwrap();
        let mut rig = Rig::new();
        assert_eq!(
            joining.ed25519_pub,
            rig.id.public().ed25519_pub,
            "the fixture joining side is the phone identity"
        );
        let mut flow = EnrollFlow::new();

        // step 1: the terminal commitment, then the phone names itself
        let v = enroll_view(flow.on_frame(&frames[0], &mut rig.ctx(), 1));
        assert_eq!((v.step, v.token), (EnrollStep::Commit, 1));
        let out = flow
            .decide(
                &Decision::EnrollName {
                    name: joining.name.clone(),
                },
                &mut rig.ctx(),
            )
            .unwrap();
        assert_eq!(out.kind, 6);
        assert!(matches!(
            EnrollFrame::decode(&out.frame).unwrap(),
            EnrollFrame::Commit { role: 0, .. }
        ));
        assert_eq!(
            frames[1],
            EnrollFrame::Commit {
                role: 0,
                commit: commit_of(0, &joining)
            }
            .encode()
        );

        // the fixture's phone reveal has an ephemeral key the test cannot redraw: continue from it
        let Stage::AwaitReveal { commit, .. } = std::mem::replace(&mut flow.stage, Stage::Idle)
        else {
            panic!("stage")
        };
        flow.stage = Stage::AwaitReveal {
            me: joining.clone(),
            commit,
        };

        // step 2: the terminal reveal; the SAS words equal the Rust mirror; the phone reveal is the fixture frame
        let v = enroll_view(flow.on_frame(&frames[2], &mut rig.ctx(), 2));
        assert_eq!(v.step, EnrollStep::Reveal);
        assert_eq!(v.sas_words.as_deref(), f["sas"].as_str());
        assert_eq!(
            v.terminal_id.as_deref(),
            Some(
                device_id_of(
                    &Reveal::from_bytes(&hx(&f["existing"]["reveal_hex"]))
                        .unwrap()
                        .ed25519_pub
                )
                .as_str()
            )
        );
        let o = v.outgoing.unwrap();
        assert_eq!(o.frame, frames[3]);
        assert!(!o.qr.is_empty());

        // step 3: the words match; the confirm is signed over the transcript
        let out = flow
            .decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .unwrap();
        let EnrollFrame::Confirm {
            role: 0,
            ok: true,
            sig,
        } = EnrollFrame::decode(&out.frame).unwrap()
        else {
            panic!("confirm")
        };
        let th: [u8; 32] = hx(&f["transcript_hash_hex"]).try_into().unwrap();
        assert!(verify(
            &joining.ed25519_pub,
            &confirm_message(0, true, &th),
            &sig
        ));
        assert_eq!(
            hx(&f["confirm_message_existing_ok_hex"]),
            confirm_message(1, true, &th)
        );

        // step 4: the terminal confirm (fixture) enters the ring
        match flow.on_frame(&frames[4], &mut rig.ctx(), 3) {
            Opened::Enrolled { view } => {
                assert_eq!(view.phone_name, joining.name);
                assert_eq!(view.terminal_id.len(), 16);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(rig.state.ring.len(), 1);
        let (id, dev) = rig.state.ring.iter().next().unwrap();
        assert_eq!(
            dev.ed25519_pub,
            hex::encode(&hx(&f["existing"]["reveal_hex"])[..32])
        );
        assert_eq!(&hex::encode(&hx(&f["existing"]["reveal_hex"])[..8]), id);
        assert_eq!(rig.state.enrolled.as_ref().unwrap().device_id, *id);
        assert_eq!(rig.state.enrolled.as_ref().unwrap().name, joining.name);
        // finished: a second confirm is out of order
        assert_eq!(
            reason(&flow.on_frame(&frames[4], &mut rig.ctx(), 4)),
            "Expected the terminal's code first."
        );
    }

    #[test]
    fn the_fixture_frames_decode_and_garbage_does_not() {
        let f = fx();
        let mut rig = Rig::new();
        let mut flow = EnrollFlow::new();
        let first = hx(&f["frames_hex"][0]);
        assert!(reason(&flow.on_frame(&first[..20], &mut rig.ctx(), 1))
            .starts_with("Not an enrollment document"));
        assert!(reason(&flow.on_frame(&[], &mut rig.ctx(), 1))
            .starts_with("Not an enrollment document"));
    }

    // ---------------------------------------------------------------- a self-made terminal

    struct Term {
        id: DeviceIdentity,
        party: Party,
    }

    impl Term {
        fn new() -> Self {
            let id = DeviceIdentity::generate();
            let party = Party::new(&id, "lab terminal", true);
            Term { id, party }
        }
        fn commit(&self) -> Vec<u8> {
            EnrollFrame::Commit {
                role: 1,
                commit: self.party.commit(1),
            }
            .encode()
        }
        fn reveal(&self) -> Vec<u8> {
            EnrollFrame::Reveal {
                role: 1,
                reveal: self.party.reveal.bytes(),
            }
            .encode()
        }
    }

    /// Runs the first two steps; returns the flow, the phone's reveal and the SAS view.
    fn up_to_sas(rig: &mut Rig, t: &Term) -> (EnrollFlow, Reveal, EnrollView) {
        let mut flow = EnrollFlow::new();
        enroll_view(flow.on_frame(&t.commit(), &mut rig.ctx(), 1));
        let out = flow
            .decide(
                &Decision::EnrollName {
                    name: "  my pixel ".into(),
                },
                &mut rig.ctx(),
            )
            .unwrap();
        let EnrollFrame::Commit { role: 0, commit } = EnrollFrame::decode(&out.frame).unwrap()
        else {
            panic!()
        };
        let v = enroll_view(flow.on_frame(&t.reveal(), &mut rig.ctx(), 2));
        let o = v.outgoing.clone().unwrap();
        let EnrollFrame::Reveal { role: 0, reveal } = EnrollFrame::decode(&o.frame).unwrap() else {
            panic!()
        };
        let me = Reveal::from_bytes(&reveal).unwrap();
        assert_eq!(
            commit_of(0, &me),
            commit,
            "the reveal opens the commitment the phone showed"
        );
        assert_eq!(me.name, "my pixel", "the name is trimmed");
        assert!(me.attended);
        (flow, me, v)
    }

    #[test]
    fn a_full_run_against_a_self_made_terminal() {
        let mut rig = Rig::new();
        let t = Term::new();
        let (mut flow, me, v) = up_to_sas(&mut rig, &t);
        assert_eq!(v.sas_words.unwrap(), sas_of(&me, &t.party.reveal));
        assert_eq!(v.terminal_name.as_deref(), Some("lab terminal"));
        let out = flow
            .decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .unwrap();
        let th = transcript_hash(&me, &t.party.reveal);
        let EnrollFrame::Confirm {
            role: 0,
            ok: true,
            sig,
        } = EnrollFrame::decode(&out.frame).unwrap()
        else {
            panic!()
        };
        assert!(verify(
            &rig.id.public().ed25519_pub,
            &confirm_message(0, true, &th),
            &sig
        ));
        let tsig = t.id.sign(&confirm_message(1, true, &th));
        let done = flow.on_frame(
            &EnrollFrame::Confirm {
                role: 1,
                ok: true,
                sig: tsig,
            }
            .encode(),
            &mut rig.ctx(),
            3,
        );
        assert!(matches!(done, Opened::Enrolled { .. }), "{done:?}");
        assert_eq!(
            rig.state.ring[&t.id.public().device_id()].name,
            "lab terminal"
        );
    }

    #[test]
    fn a_tampered_reveal_refuses_and_kills_the_flow() {
        let mut rig = Rig::new();
        let t = Term::new();
        let mut flow = EnrollFlow::new();
        enroll_view(flow.on_frame(&t.commit(), &mut rig.ctx(), 1));
        flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
            .unwrap();
        let mut r = t.party.reveal.clone();
        r.nonce[0] ^= 1; // a reveal that is not the one committed to
        let bad = EnrollFrame::Reveal {
            role: 1,
            reveal: r.bytes(),
        }
        .encode();
        let o = flow.on_frame(&bad, &mut rig.ctx(), 2);
        assert!(reason(&o).contains("do not match the code it showed first"));
        assert_eq!(kind(&o), RefusalKind::Mismatch);
        // the two values that differ are carried: the code shown first and the code the keys open to
        let Opened::Refused { view } = &o else {
            unreachable!()
        };
        assert_eq!(view.compared.len(), 2);
        assert_ne!(view.compared[0].value, view.compared[1].value);
        for r in &view.compared {
            assert_eq!(r.value.split(' ').count(), 8, "{}", r.value);
            assert!(r
                .value
                .chars()
                .all(|c| c == ' ' || c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
        }
        // the honest reveal now finds no flow
        assert_eq!(
            reason(&flow.on_frame(&t.reveal(), &mut rig.ctx(), 3)),
            "Expected the terminal's code first."
        );
        assert!(rig.state.ring.is_empty());
    }

    #[test]
    fn unusable_terminal_keys_are_refused() {
        let mut rig = Rig::new();
        // a short ML-KEM key, committed to honestly
        let t = Term::new();
        let mut short = t.party.reveal.clone();
        short.mlkem_pub.truncate(1183);
        let mut flow = EnrollFlow::new();
        flow.on_frame(
            &EnrollFrame::Commit {
                role: 1,
                commit: commit_of(1, &short),
            }
            .encode(),
            &mut rig.ctx(),
            1,
        );
        flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
            .unwrap();
        let o = flow.on_frame(
            &EnrollFrame::Reveal {
                role: 1,
                reveal: short.bytes(),
            }
            .encode(),
            &mut rig.ctx(),
            2,
        );
        assert_eq!(reason(&o), "The terminal presented unusable keys.");
        assert_eq!(kind(&o), RefusalKind::UnusableKeys);
        // the terminal presents this phone's own signing key
        let mut mirror = t.party.reveal.clone();
        let mine = rig.id.public();
        mirror.ed25519_pub = mine.ed25519_pub;
        mirror.x25519_pub = mine.x25519_pub;
        mirror.mlkem_pub = mine.mlkem_pub;
        let mut flow = EnrollFlow::new();
        flow.on_frame(
            &EnrollFrame::Commit {
                role: 1,
                commit: commit_of(1, &mirror),
            }
            .encode(),
            &mut rig.ctx(),
            1,
        );
        flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
            .unwrap();
        let o = flow.on_frame(
            &EnrollFrame::Reveal {
                role: 1,
                reveal: mirror.bytes(),
            }
            .encode(),
            &mut rig.ctx(),
            2,
        );
        assert_eq!(reason(&o), "The terminal presented unusable keys.");
    }

    #[test]
    fn an_attended_flag_other_than_zero_or_one_is_not_a_reveal() {
        // review round 1, S5: the strict reveal decoder
        let mut rig = Rig::new();
        let t = Term::new();
        let mut flow = EnrollFlow::new();
        enroll_view(flow.on_frame(&t.commit(), &mut rig.ctx(), 1));
        flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
            .unwrap();
        let mut bytes = t.party.reveal.bytes();
        *bytes.last_mut().unwrap() = 2;
        let o = flow.on_frame(
            &EnrollFrame::Reveal {
                role: 1,
                reveal: bytes,
            }
            .encode(),
            &mut rig.ctx(),
            2,
        );
        assert!(
            reason(&o).starts_with("Not an enrollment document"),
            "{}",
            reason(&o)
        );
        // a stray frame does not end the flow: the honest reveal still goes through
        assert!(matches!(
            flow.on_frame(&t.reveal(), &mut rig.ctx(), 3),
            Opened::Enroll { .. }
        ));
    }

    #[test]
    fn when_the_words_differ_the_phone_declines_and_stops() {
        let mut rig = Rig::new();
        let t = Term::new();
        let (mut flow, me, _) = up_to_sas(&mut rig, &t);
        let out = flow
            .decide(&Decision::EnrollSas { matches: false }, &mut rig.ctx())
            .unwrap();
        let th = transcript_hash(&me, &t.party.reveal);
        let EnrollFrame::Confirm {
            role: 0,
            ok: false,
            sig,
        } = EnrollFrame::decode(&out.frame).unwrap()
        else {
            panic!()
        };
        assert!(verify(
            &rig.id.public().ed25519_pub,
            &confirm_message(0, false, &th),
            &sig
        ));
        // a terminal confirm now finds no flow and nothing enters the ring
        let tsig = t.id.sign(&confirm_message(1, true, &th));
        let o = flow.on_frame(
            &EnrollFrame::Confirm {
                role: 1,
                ok: true,
                sig: tsig,
            }
            .encode(),
            &mut rig.ctx(),
            3,
        );
        assert_eq!(reason(&o), "Expected the terminal's code first.");
        assert!(rig.state.ring.is_empty() && rig.state.enrolled.is_none());
        assert!(flow
            .decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .is_err());
    }

    #[test]
    fn the_terminal_confirm_is_checked() {
        let mut rig = Rig::new();
        let t = Term::new();
        // bad signature (the other message: a screen byte appended, the terminal-to-terminal form)
        let (mut flow, me, _) = up_to_sas(&mut rig, &t);
        flow.decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .unwrap();
        let th = transcript_hash(&me, &t.party.reveal);
        let wrong = t.id.sign(&terminal_confirm_message(1, true, &th, true));
        let o = flow.on_frame(
            &EnrollFrame::Confirm {
                role: 1,
                ok: true,
                sig: wrong,
            }
            .encode(),
            &mut rig.ctx(),
            3,
        );
        assert_eq!(
            reason(&o),
            "Enrollment failed: the terminal confirmation signature is bad"
        );
        assert_eq!(kind(&o), RefusalKind::BadSignature);
        assert!(rig.state.ring.is_empty());
        // the terminal declines (validly signed)
        let (mut flow, me, _) = up_to_sas(&mut rig, &t);
        flow.decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .unwrap();
        let th = transcript_hash(&me, &t.party.reveal);
        let no = t.id.sign(&confirm_message(1, false, &th));
        let o = flow.on_frame(
            &EnrollFrame::Confirm {
                role: 1,
                ok: false,
                sig: no,
            }
            .encode(),
            &mut rig.ctx(),
            3,
        );
        assert_eq!(reason(&o), "Enrollment failed: the terminal declined");
        assert_eq!(kind(&o), RefusalKind::Declined);
        assert!(rig.state.ring.is_empty() && rig.state.enrolled.is_none());
        // a confirm that claims the joining role is not the terminal's
        let (mut flow, me, _) = up_to_sas(&mut rig, &t);
        flow.decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .unwrap();
        let th = transcript_hash(&me, &t.party.reveal);
        let sig = t.id.sign(&confirm_message(0, true, &th));
        let o = flow.on_frame(
            &EnrollFrame::Confirm {
                role: 0,
                ok: true,
                sig,
            }
            .encode(),
            &mut rig.ctx(),
            3,
        );
        assert_eq!(
            reason(&o),
            "Enrollment failed: not the terminal confirmation"
        );
        // and the flow still takes the real one
        let sig = t.id.sign(&confirm_message(1, true, &th));
        let o = flow.on_frame(
            &EnrollFrame::Confirm {
                role: 1,
                ok: true,
                sig,
            }
            .encode(),
            &mut rig.ctx(),
            4,
        );
        assert!(matches!(o, Opened::Enrolled { .. }));
    }

    #[test]
    fn wrong_role_and_wrong_order_are_refused() {
        let mut rig = Rig::new();
        let t = Term::new();
        let mut flow = EnrollFlow::new();
        // a reveal or a confirm first
        let o = flow.on_frame(&t.reveal(), &mut rig.ctx(), 1);
        assert_eq!(reason(&o), "Expected the terminal's code first.");
        assert_eq!(kind(&o), RefusalKind::WrongOrder);
        let c = EnrollFrame::Confirm {
            role: 1,
            ok: true,
            sig: [0; 64],
        }
        .encode();
        assert_eq!(
            reason(&flow.on_frame(&c, &mut rig.ctx(), 1)),
            "Expected the terminal's code first."
        );
        // a commitment that claims the joining role
        let own = EnrollFrame::Commit {
            role: 0,
            commit: t.party.commit(0),
        }
        .encode();
        assert_eq!(
            reason(&flow.on_frame(&own, &mut rig.ctx(), 1)),
            "Expected the terminal's code first."
        );
        // a frame of another kind
        let ops = sshub_sync_proto::wire::PhoneOpsWire {
            sender: None,
            ops: vec![],
        }
        .encode();
        let o = flow.on_frame(&ops, &mut rig.ctx(), 1);
        assert!(reason(&o).starts_with("Not an enrollment document"));
        assert_eq!(kind(&o), RefusalKind::StrayFrame);
        // the commitment, then a reveal before the phone named itself
        enroll_view(flow.on_frame(&t.commit(), &mut rig.ctx(), 1));
        assert_eq!(
            reason(&flow.on_frame(&t.reveal(), &mut rig.ctx(), 2)),
            "Name this phone first."
        );
        flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
            .unwrap();
        // a reveal that claims the joining role, a commit again, a confirm too early
        let wrong = EnrollFrame::Reveal {
            role: 0,
            reveal: t.party.reveal.bytes(),
        }
        .encode();
        assert_eq!(
            reason(&flow.on_frame(&wrong, &mut rig.ctx(), 2)),
            "Expected the terminal's keys."
        );
        assert_eq!(
            reason(&flow.on_frame(&t.commit(), &mut rig.ctx(), 2)),
            "Expected the terminal's keys."
        );
        assert_eq!(
            reason(&flow.on_frame(&c, &mut rig.ctx(), 2)),
            "Expected the terminal's keys."
        );
        // after the reveal the person must answer first
        assert!(matches!(
            flow.on_frame(&t.reveal(), &mut rig.ctx(), 3),
            Opened::Enroll { .. }
        ));
        assert!(reason(&flow.on_frame(&c, &mut rig.ctx(), 4)).starts_with("Compare the four words"));
        // decisions out of order are errors, not refusals
        assert!(flow
            .decide(&Decision::EnrollName { name: "x".into() }, &mut rig.ctx())
            .is_err());
        assert!(flow.decide(&Decision::Refuse, &mut rig.ctx()).is_err());
        assert!(flow
            .decide(&Decision::EnrollSas { matches: true }, &mut rig.ctx())
            .is_ok());
    }

    #[test]
    fn the_phone_name_rules() {
        let mut rig = Rig::new();
        let t = Term::new();
        let mut flow = EnrollFlow::new();
        assert!(
            flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
                .is_err(),
            "no commitment yet"
        );
        flow.on_frame(&t.commit(), &mut rig.ctx(), 1);
        let (long, wide) = ("x".repeat(65), "\u{e9}".repeat(33));
        for bad in [
            "",
            "   ",
            "tab\there",
            "nul\0",
            "line\nbreak",
            "del\u{7f}",
            long.as_str(),
            wide.as_str(),
        ] {
            let e = flow
                .decide(&Decision::EnrollName { name: bad.into() }, &mut rig.ctx())
                .unwrap_err();
            assert!(matches!(e, CoreError::BadInput { .. }), "{bad:?}");
        }
        // a refused name keeps the flow where it was; 64 bytes and non-ASCII are fine
        assert!(flow
            .decide(
                &Decision::EnrollName {
                    name: "\u{e9}".repeat(32)
                },
                &mut rig.ctx()
            )
            .is_ok());
    }

    #[test]
    fn equal_randomness_gives_the_same_reveal() {
        let t = Term::new();
        let run = |seed: u64| {
            let mut rig = Rig::new();
            rig.rng = ChaCha8Rng::seed_from_u64(seed);
            let mut flow = EnrollFlow::new();
            flow.on_frame(&t.commit(), &mut rig.ctx(), 1);
            flow.decide(&Decision::EnrollName { name: "p".into() }, &mut rig.ctx())
                .unwrap()
                .frame
        };
        assert_eq!(run(1), run(1));
        assert_ne!(run(1), run(2));
    }
}
