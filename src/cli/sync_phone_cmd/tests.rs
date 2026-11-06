//! Each verb against the library it wraps: the notice decodes to the op, the phone's signature
//! arrives through `sync-phone-ops` and counts in `red_status`, the enrollment session verifies a
//! phone written the way the page does it, and the share reply opens the credential.

use super::*;
use crate::sync::ceremony::{red_phone_sign_body, red_status, RedProof};
use crate::sync::enroll::{confirm_message, transcript_hash, Party, Reveal, INITIATOR, RESPONDER};
use crate::sync::oplog::{Attendance, CredMode, DeviceKind, DeviceRole, OpBody, VerifiedBy};
use crate::sync::protocol::tests::Dev;
use crate::sync::quorum::tests::{all_to_all, append, payload, quorum_mode};
use crate::sync::wire::{ChallengeWire, EnrollFrame, RecoveryNoticeWire, ShareResponseWire};

/// 0 and 1 attended terminals, 2 the phone; every store knows all three.
fn world() -> Vec<Dev> {
    let d: Vec<Dev> = (0..3).map(|_| Dev::new()).collect();
    for x in &d {
        crate::sync::ceremony::test_force(&x.store, false);
    }
    for (i, x) in d.iter().enumerate() {
        for (j, y) in d.iter().enumerate() {
            if i != j {
                x.trust_as(
                    y,
                    if j == 2 {
                        DeviceKind::Phone
                    } else {
                        DeviceKind::Terminal
                    },
                );
            }
        }
        x.store
            .sync_register_device(
                &x.id.public(),
                "self",
                DeviceRole::Holder,
                Attendance::Attended,
                if i == 2 {
                    DeviceKind::Phone
                } else {
                    DeviceKind::Terminal
                },
                VerifiedBy::Sas,
            )
            .unwrap();
    }
    d
}

fn mode_change(d: &[Dev]) -> crate::sync::oplog::Op {
    append(
        &d[0],
        OpBody::ModeChange {
            to: CredMode::Quorum,
            migrated_on: d[0].did.clone(),
            cosigner: None,
        },
    )
}

fn ingest_all(from: &Dev, to: &Dev) {
    for (dev, _, _) in from.store.sync_chain_heads().unwrap() {
        let have = to
            .store
            .sync_chain_heads()
            .unwrap()
            .into_iter()
            .find(|(x, _, _)| *x == dev)
            .map_or(0, |(_, s, _)| s);
        for op in from.store.sync_chain_tail(&dev, have).unwrap() {
            to.store.sync_ingest(op).unwrap();
        }
    }
}

#[test]
fn sync_notice_decodes_to_the_op_and_refuses_what_is_not_red() {
    let d = world();
    let op = mode_change(&d);
    let prefix: String = op
        .hash()
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect();
    let (hash, text) = notice_text(&d[0].store, &prefix).unwrap();
    assert_eq!(
        hash,
        op.hash()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let wire = RecoveryNoticeWire::decode(&B64.decode(text).unwrap()).unwrap();
    let back =
        crate::sync::oplog::Op::from_canonical(&wire.op_canonical, wire.op_signature).unwrap();
    assert_eq!(
        back.hash(),
        op.hash(),
        "the notice carries the op as signed"
    );
    // A green op is not for the phone.
    let green = append(
        &d[0],
        OpBody::HostUpsert {
            host_uid: [1; 16],
            fields: Default::default(),
        },
    );
    let green_prefix: String = green
        .hash()
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(notice_text(&d[0].store, &green_prefix).is_err());
    assert!(notice_text(&d[0].store, "zz").is_err());
}

#[test]
fn sync_phone_ops_takes_the_phones_signature_in_and_it_counts_in_red_status() {
    let d = world();
    let op = mode_change(&d);
    ingest_all(&d[0], &d[1]);
    ingest_all(&d[0], &d[2]);
    // The phone signs on its own device and copies its ops (kind 5, base64) out.
    let sign = append(&d[2], red_phone_sign_body(&op));
    let text = B64.encode(
        crate::sync::wire::PhoneOpsWire {
            ops: vec![(sign.canonical_bytes(), sign.signature)],
        }
        .encode(),
    );
    let before = red_status(&d[1].store, &op, crate::store::now_ts()).unwrap();
    assert!(!before.have.contains(&RedProof::Phone));
    let (summary, _) = phone_ops(&d[1].store, &text).unwrap();
    assert_eq!(summary.received, 1);
    let after = red_status(&d[1].store, &op, crate::store::now_ts()).unwrap();
    assert!(after.have.contains(&RedProof::Phone), "{after:?}");
    // Garbage changes nothing and says so.
    assert!(phone_ops(&d[1].store, "AAAA").is_err());
}

fn ops_text(ops: &[&crate::sync::oplog::Op]) -> String {
    B64.encode(
        crate::sync::wire::PhoneOpsWire {
            ops: ops
                .iter()
                .map(|o| (o.canonical_bytes(), o.signature))
                .collect(),
        }
        .encode(),
    )
}

#[test]
fn sync_phone_ops_says_why_ops_were_skipped() {
    let d = world();
    ingest_all(&d[0], &d[1]);
    let attest = |n: u8| {
        append(
            &d[2],
            OpBody::HeadAttest {
                device_id: d[2].did.clone(),
                seq: u64::from(n),
                chain_hash: [n; 32],
            },
        )
    };
    let (a, b, c) = (attest(1), attest(2), attest(3));
    // A list that starts mid-chain: the ring has none of ops before `b`.
    let (s, _) = phone_ops(&d[1].store, &ops_text(&[&b, &c])).unwrap();
    assert_eq!((s.received, s.applied), (2, 0));
    let k = skipped_ops(&d[1].store, &s).unwrap();
    assert_eq!(k.len(), 1, "{k:?}");
    assert_eq!(k[0]["reason"], "chain-gap");
    assert_eq!(k[0]["first_seq"], b.seq);
    assert_eq!(k[0]["count"], 2);
    assert!(k[0]["note"].as_str().unwrap().contains("full chain"));
    assert!(skipped_line(&k[0]).starts_with("skipped 2 chain-gap (first seq"));
    // The whole chain applies, and a second paste is already known.
    let (s, _) = phone_ops(&d[1].store, &ops_text(&[&a, &b, &c])).unwrap();
    assert_eq!(s.applied, 3);
    assert!(skipped_ops(&d[1].store, &s).unwrap().is_empty());
    let (s, _) = phone_ops(&d[1].store, &ops_text(&[&a, &b, &c])).unwrap();
    let k = skipped_ops(&d[1].store, &s).unwrap();
    assert_eq!(
        (k[0]["reason"].as_str(), k[0]["count"].as_u64()),
        (Some("already-known"), Some(3))
    );
}

fn lines(out: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(out)
        .lines()
        .map(str::to_string)
        .collect()
}

fn value_of(l: &[String], key: &str) -> String {
    l.iter()
        .rev()
        .find_map(|x| x.strip_prefix(&format!("{key}: ")).map(str::to_string))
        .unwrap_or_else(|| panic!("no {key} line in {l:?}"))
}

#[test]
fn sync_phone_enroll_verifies_a_phone_and_proposes_it_as_a_red_holder() {
    // A ring of one terminal: the verb has a ring to enroll the phone into.
    let t = Dev::new();
    t.store
        .sync_register_device(
            &t.id.public(),
            "me",
            DeviceRole::Holder,
            Attendance::Attended,
            DeviceKind::Terminal,
            VerifiedBy::Sas,
        )
        .unwrap();
    crate::sync::ceremony::test_force(&t.store, false);
    let phone = crate::sync::crypto::DeviceIdentity::generate();
    let party = Party::new(&phone, "my phone", true);
    let mut s = EnrollSession::new(&t.store, &t.id, "terminal", false);
    let mut out = Vec::new();
    s.start(&mut out);
    let first_lines = lines(&out);
    assert!(
        first_lines[0].starts_with("companion: ")
            && first_lines[0].contains(crate::sync::companion::COMPANION_FINGERPRINT),
        "the page fingerprint comes first: {first_lines:?}"
    );
    let first = value_of(&lines(&out), "frame");
    assert!(matches!(
        EnrollFrame::decode(&B64.decode(first).unwrap()).unwrap(),
        EnrollFrame::Commit {
            role: RESPONDER,
            ..
        }
    ));
    // The phone's commitment in; the terminal's reveal out.
    out.clear();
    s.feed(
        &B64.encode(
            EnrollFrame::Commit {
                role: INITIATOR,
                commit: party.commit(INITIATOR),
            }
            .encode(),
        ),
        &mut out,
    );
    let reveal_frame = value_of(&lines(&out), "frame");
    let EnrollFrame::Reveal { reveal, .. } =
        EnrollFrame::decode(&B64.decode(reveal_frame).unwrap()).unwrap()
    else {
        panic!("the terminal reveals after the phone's commitment")
    };
    let term_reveal = Reveal::from_bytes(&reveal).unwrap();
    // The phone's reveal in; the four words out, the same ones the phone derives.
    out.clear();
    s.feed(
        &B64.encode(
            EnrollFrame::Reveal {
                role: INITIATOR,
                reveal: party.reveal.bytes(),
            }
            .encode(),
        ),
        &mut out,
    );
    let th = transcript_hash(&party.reveal, &term_reveal);
    let words = value_of(&lines(&out), "words");
    assert!(
        words.starts_with(&crate::sync::crypto::sas_words(&th)),
        "{words}"
    );
    // A bad frame is refused in place; the session goes on.
    out.clear();
    s.feed("***", &mut out);
    assert!(value_of(&lines(&out), "error").contains("not base64"));
    // The explicit answer, then the phone's confirmation: verified, admitted, Red.
    out.clear();
    s.feed("y", &mut out);
    out.clear();
    s.feed(
        &B64.encode(
            EnrollFrame::Confirm {
                role: INITIATOR,
                ok: true,
                sig: phone.sign(&confirm_message(INITIATOR, true, &th)),
            }
            .encode(),
        ),
        &mut out,
    );
    let l = lines(&out);
    assert_eq!(s.done, Some(true), "{l:?}");
    let last = value_of(&l, "frame");
    assert!(matches!(
        EnrollFrame::decode(&B64.decode(last).unwrap()).unwrap(),
        EnrollFrame::Confirm {
            role: RESPONDER,
            ok: true,
            ..
        }
    ));
    assert!(l
        .iter()
        .any(|x| x.starts_with("admitted: ") && x.contains("is enrolled")));
    assert!(t
        .store
        .sync_devices()
        .unwrap()
        .iter()
        .any(|d| d.id == phone.public().device_id() && d.kind == DeviceKind::Phone));
}

#[test]
fn sync_phone_enroll_stops_on_no_and_the_json_form_is_one_object_per_line() {
    let t = Dev::new();
    let phone = crate::sync::crypto::DeviceIdentity::generate();
    let party = Party::new(&phone, "my phone", true);
    let mut s = EnrollSession::new(&t.store, &t.id, "terminal", true);
    let mut out = Vec::new();
    s.start(&mut out);
    s.feed(
        &B64.encode(
            EnrollFrame::Commit {
                role: INITIATOR,
                commit: party.commit(INITIATOR),
            }
            .encode(),
        ),
        &mut out,
    );
    s.feed(
        &B64.encode(
            EnrollFrame::Reveal {
                role: INITIATOR,
                reveal: party.reveal.bytes(),
            }
            .encode(),
        ),
        &mut out,
    );
    s.feed("n", &mut out);
    assert_eq!(s.done, Some(false));
    for l in lines(&out) {
        let v: Value = serde_json::from_str(&l).unwrap_or_else(|e| panic!("{l}: {e}"));
        assert!(v["event"].is_string());
    }
    let last = lines(&out).pop().unwrap();
    assert!(last.contains("\"stopped\""), "{last}");
}

#[test]
fn sync_share_request_challenge_decodes_and_the_phones_reply_opens_the_credential() {
    use crate::sync::cred_ops::cred_put;
    use crate::sync::quorum::{own_shares, QuorumUnlocker};
    let d = world();
    // No ceremony to run here: every Red change counts as in force.
    for x in &d {
        crate::sync::ceremony::test_force(&x.store, true);
    }
    quorum_mode(&d[0]);
    all_to_all(&d);
    let row = d[0]
        .store
        .create_host(&crate::store::NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    let host = d[0].store.host_uid(row.id).unwrap().unwrap();
    cred_put(&d[0].store, &d[0].id, &host, &payload("phone-secret")).unwrap();
    all_to_all(&d);
    let ch = ShareChallenge::new(&d[0].store, &d[0].id, &host).unwrap();
    // The kind 1 text decodes and names a challenge for this host.
    let wire = ChallengeWire::decode(&B64.decode(&ch.text).unwrap()).unwrap();
    // What the verb prints next to the host name is the phone's grouping of this host's uid.
    let shown = ch.challenge.shown().unwrap();
    assert_eq!(shown, crate::sync::quorum::challenge_shown(&wire).unwrap());
    assert_eq!(shown.host_id, crate::sync::quorum::grouped_host_uid(&host));
    assert_eq!(
        shown.words,
        crate::sync::crypto::sas_words(&wire.binding_hash())
    );
    // Bluetooth carries the same bytes, as a frame its channel accepts.
    assert_eq!(ch.frame, B64.decode(&ch.text).unwrap());
    assert!(crate::sync::media::ble_link::valid_frame(&ch.frame));
    // The phone answers with its share, sealed to the challenge's ephemeral key.
    let share = own_shares(&d[2].store, &d[2].id, &host)
        .unwrap()
        .remove(0)
        .1;
    let reply = ShareResponseWire {
        request_id: wire.request_id,
        ciphertext: crate::sync::crypto::DeviceIdentity::seal_to(
            &wire.eph_x25519_pub,
            &crate::sync::share_release::share_to_bytes(&share),
        ),
    };
    let (got, warning) = ch.open(&d[0].store, &B64.encode(reply.encode())).unwrap();
    assert_eq!(got, share);
    assert!(warning.is_none());
    // A reply whose courier ops cannot be taken in still yields its share, with a warning.
    let ext = crate::sync::share_release::ReplyExt {
        approval_sig: d[2].id.sign(&ch.challenge.binding_preimage()),
        ops: vec![(vec![1, 2, 3], [0; 64])],
        snapshot: None,
    };
    let mut plain_bytes = crate::sync::share_release::share_to_bytes(&share);
    plain_bytes.extend_from_slice(&ext.encode());
    let with_ext = ShareResponseWire {
        request_id: wire.request_id,
        ciphertext: crate::sync::crypto::DeviceIdentity::seal_to(
            &wire.eph_x25519_pub,
            &plain_bytes,
        ),
    };
    let (kept, warning) = ch
        .open(&d[0].store, &B64.encode(with_ext.encode()))
        .unwrap();
    assert_eq!(kept, share, "the valid share survives a failing extension");
    assert!(warning.unwrap().contains("courier ops not kept"));
    // A reply for another request is refused.
    let other = ShareResponseWire {
        request_id: wire.request_id + 1,
        ciphertext: vec![0; 40],
    };
    assert!(ch.open(&d[0].store, &B64.encode(other.encode())).is_err());
    // The share finishes the unlock the way the verb does it.
    let gate = std::sync::Arc::new(crate::credentials::SecretGate::default());
    let unlocker = QuorumUnlocker::new(
        d[0].store.clone(),
        d[0].id.clone(),
        gate.clone(),
        Box::new(GivenShare(got)),
        None,
    );
    let key = crate::credentials::host_key(row.id);
    crate::credentials::ensure_unlocked(
        &[crate::credentials::SecretRef::Key(key.clone())],
        &unlocker,
    )
    .unwrap();
    assert_eq!(
        gate.get(&key).as_deref().map(String::as_str),
        Some("phone-secret")
    );
}

#[test]
fn sync_cert_request_names_the_chosen_scope_and_never_everything() {
    let d = world();
    let g = d[0]
        .store
        .create_group(&crate::store::NewHostGroup {
            name: "staging".into(),
            ..Default::default()
        })
        .unwrap();
    let h = d[0]
        .store
        .create_host(&crate::store::NewHost {
            group_id: Some(g.id),
            ..crate::store::NewHost::launcher("stg-1", "10.0.0.7")
        })
        .unwrap();
    let (labels, scopes) = crate::app::sync_trust::scope_choices(&d[0].store);
    let mut a = parse(&[
        "--host".into(),
        "stg-1".into(),
        "--budget".into(),
        "5".into(),
    ])
    .unwrap();
    let scope = pick_scope(&labels, &scopes, &a).unwrap();
    let huid = d[0].store.host_uid(h.id).unwrap().unwrap();
    assert_eq!(scope, crate::sync::oplog::CertScope::Hosts(vec![huid]));
    let wire = crate::sync::cert::cert_request(
        &d[0].store,
        &d[0].did,
        scope,
        30 * 86_400,
        a.budget,
        1,
        crate::store::now_ts(),
    )
    .unwrap();
    let back = crate::sync::wire::CertRequestWire::decode(&wire.encode()).unwrap();
    assert_eq!(back.max_new_hosts_per_day, 5);
    a = parse(&["--group".into(), "staging".into()]).unwrap();
    assert!(matches!(
        pick_scope(&labels, &scopes, &a).unwrap(),
        crate::sync::oplog::CertScope::Group(_)
    ));
    assert!(
        pick_scope(&labels, &scopes, &parse(&[]).unwrap()).is_err(),
        "never everything"
    );
    assert!(pick_scope(
        &labels,
        &scopes,
        &parse(&["--host".into(), "nope".into()]).unwrap()
    )
    .is_err());
}

#[test]
fn the_ble_flag_is_parsed_and_a_build_without_bluetooth_says_so() {
    assert!(parse(&["--ble".into()]).unwrap().ble);
    assert!(!parse(&[]).unwrap().ble);
    #[cfg(not(feature = "bluetooth"))]
    {
        let err = BleLink::new(false, crate::sync::media::ListenOpts::SHARE_RESPONSE)
            .err()
            .expect("no Bluetooth channel in this build");
        assert!(
            format!("{err:#}").contains("--features bluetooth"),
            "{err:#}"
        );
    }
}
