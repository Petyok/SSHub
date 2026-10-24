//! The terminal side of enrolling the phone (frames kind 6 to 8, QR both ways, no network). The
//! phone is the INITIATOR of the commitments and the terminal the RESPONDER, exactly as the page
//! does it:
//!
//! 1. the terminal shows its `Commit`;
//! 2. it scans the phone's `Commit` and shows its `Reveal`;
//! 3. it scans the phone's `Reveal`, checks it opens the commitment and shows the four words;
//! 4. the user compares the words with the phone's: [`PhoneEnroll::answer`];
//! 5. it scans the phone's `Confirm` (a signature by the phone key over the transcript and the
//!    decision) and shows its own.
//!
//! A mismatch anywhere stops it and shows a declining `Confirm` so the phone stops too. On success
//! [`admit_phone`] registers the phone and proposes its `DeviceAdd { Holder, Phone }`, a Red op: the
//! phone signs the notice it gets, a second terminal approves and the third factor completes it.

use anyhow::{bail, ensure, Result};

use super::crypto::{sas_words, verify, DeviceIdentity, DevicePublic};
use super::enroll::{
    check_reveal, confirm_message, transcript_hash, Party, Reveal, INITIATOR, RESPONDER,
};
use super::oplog::{Attendance, DeviceKind, DeviceRole, Op, OpBody, VerifiedBy};
use super::wire::EnrollFrame;
use crate::store::LauncherStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    WantPhoneCommit,
    WantPhoneReveal,
    Compare,
    WantPhoneConfirm,
    Done,
    Stopped,
}

/// What the UI does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Show this frame as a QR code (or text) for the phone and keep scanning.
    Show(Vec<u8>),
    /// Show the four words; the user answers with [`PhoneEnroll::answer`].
    Compare { words: String, phone_name: String },
    /// Nothing to show yet: scan the next frame.
    Wait,
    /// The phone is verified. Show `frame` (the terminal's confirmation) and call [`admit_phone`].
    Verified(Verified),
    /// Stopped. `frame` tells the phone to stop too, when there is one to show.
    Stopped { why: String, frame: Option<Vec<u8>> },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Verified {
    pub frame: Vec<u8>,
    pub phone: DevicePublic,
    pub name: String,
}

pub struct PhoneEnroll {
    party: Party,
    state: State,
    phone_commit: Option<[u8; 32]>,
    phone: Option<Reveal>,
    th: Option<[u8; 32]>,
    phone_confirm: Option<(bool, [u8; 64])>,
}

impl PhoneEnroll {
    /// Start: the first frame to show is the terminal's commitment.
    pub fn begin(id: &DeviceIdentity, name: &str) -> (Self, Vec<u8>) {
        let party = Party::new(id, name, true);
        let first = EnrollFrame::Commit {
            role: RESPONDER,
            commit: party.commit(RESPONDER),
        }
        .encode();
        (
            Self {
                party,
                state: State::WantPhoneCommit,
                phone_commit: None,
                phone: None,
                th: None,
                phone_confirm: None,
            },
            first,
        )
    }

    fn stop(&mut self, why: &str) -> Step {
        self.state = State::Stopped;
        Step::Stopped {
            why: why.to_string(),
            frame: None,
        }
    }

    /// Feed a frame the camera (or a paste) produced.
    pub fn scan(&mut self, id: &DeviceIdentity, bytes: &[u8]) -> Result<Step> {
        let frame = EnrollFrame::decode(bytes)?;
        match (self.state, frame) {
            (State::WantPhoneCommit, EnrollFrame::Commit { role, commit }) => {
                ensure!(role == INITIATOR, "that is not the phone's commitment");
                self.phone_commit = Some(commit);
                self.state = State::WantPhoneReveal;
                Ok(Step::Show(
                    EnrollFrame::Reveal {
                        role: RESPONDER,
                        reveal: self.party.reveal.bytes(),
                    }
                    .encode(),
                ))
            }
            (State::WantPhoneReveal, EnrollFrame::Reveal { role, reveal }) => {
                ensure!(role == INITIATOR, "that is not the phone's reveal");
                let r = Reveal::from_bytes(&reveal)?;
                let commit = self.phone_commit.expect("commit comes first");
                if check_reveal(INITIATOR, &commit, &r).is_err() {
                    return Ok(self.stop(
                        "the phone's keys do not open the commitment it showed first: someone is interfering",
                    ));
                }
                if r.ed25519_pub == id.public().ed25519_pub {
                    return Ok(self.stop("the phone presents this terminal's own key"));
                }
                let th = transcript_hash(&r, &self.party.reveal);
                let step = Step::Compare {
                    words: sas_words(&th),
                    phone_name: r.name.clone(),
                };
                self.th = Some(th);
                self.phone = Some(r);
                self.state = State::Compare;
                Ok(step)
            }
            (State::Compare | State::WantPhoneConfirm, EnrollFrame::Confirm { role, ok, sig }) => {
                ensure!(role == INITIATOR, "that is not the phone's confirmation");
                self.phone_confirm = Some((ok, sig));
                if self.state == State::WantPhoneConfirm {
                    return self.finish(id, true);
                }
                Ok(Step::Wait)
            }
            (_, f) => bail!("unexpected enrollment frame {f:?} at this point"),
        }
    }

    /// The user compared the words with the phone's. `ok = false` stops everything and returns the
    /// declining confirmation to show.
    pub fn answer(&mut self, id: &DeviceIdentity, ok: bool) -> Result<Step> {
        ensure!(
            self.state == State::Compare,
            "there is nothing to answer yet"
        );
        let th = self.th.expect("words were shown");
        if !ok {
            self.state = State::Stopped;
            return Ok(Step::Stopped {
                why: "the words did not match".into(),
                frame: Some(
                    EnrollFrame::Confirm {
                        role: RESPONDER,
                        ok: false,
                        sig: id.sign(&confirm_message(RESPONDER, false, &th)),
                    }
                    .encode(),
                ),
            });
        }
        self.state = State::WantPhoneConfirm;
        if self.phone_confirm.is_some() {
            return self.finish(id, true);
        }
        Ok(Step::Wait)
    }

    fn finish(&mut self, id: &DeviceIdentity, ours: bool) -> Result<Step> {
        let (ok, sig) = self.phone_confirm.expect("checked by the callers");
        let th = self.th.expect("words were shown");
        let phone = self.phone.as_ref().expect("the phone revealed");
        if !verify(
            &phone.ed25519_pub,
            &confirm_message(INITIATOR, ok, &th),
            &sig,
        ) {
            return Ok(self.stop("the phone's confirmation is not signed by its key"));
        }
        if !ok {
            return Ok(self.stop("the phone says the words differ"));
        }
        self.state = State::Done;
        Ok(Step::Verified(Verified {
            frame: EnrollFrame::Confirm {
                role: RESPONDER,
                ok: ours,
                sig: id.sign(&confirm_message(RESPONDER, ours, &th)),
            }
            .encode(),
            phone: phone.public(),
            name: phone.name.clone(),
        }))
    }
}

/// Register the verified phone on this terminal and propose its admission: `DeviceAdd { Holder, Phone }`
/// is a Red op, so it waits for the ceremony (the phone's own signature on the notice, a second
/// terminal, the third factor) before any other device acts on it.
pub fn admit_phone(store: &LauncherStore, id: &DeviceIdentity, v: &Verified) -> Result<Op> {
    ensure!(
        super::quorum::phone_device(store)?.is_none(),
        "the ring already has a phone: replacing it is a recovery, not an enrollment"
    );
    store.sync_register_device(
        &v.phone,
        &v.name,
        DeviceRole::Holder,
        Attendance::Attended,
        DeviceKind::Phone,
        VerifiedBy::Sas,
    )?;
    store.sync_append_local(
        id,
        OpBody::DeviceAdd {
            device: v.phone.clone(),
            name: v.name.clone(),
            role: DeviceRole::Holder,
            attendance: Attendance::Attended,
            kind: DeviceKind::Phone,
            verified_by: VerifiedBy::Sas,
        },
    )
}

#[cfg(test)]
mod tests;
