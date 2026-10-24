//! The TUI's `ShareSource`: where a terminal gets the share it does not hold,
//! asking the user through the unlock modal (contract 4b).
//!
//! Order for a second share: holders that may release under a trust
//! certificate (asked over ssh, no human involved), then the phone: its
//! challenge QR is shown and the user answers with the pasted reply
//! (clipboard), the 16 character short code or the 24 words, whichever the
//! text they typed is. The attended asynchronous peer path waits on two
//! op-log types and is not here.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::app::sync::UnlockPathKind;
use crate::credentials::UnlockError;
use crate::sync::client::SyncEvent;
use crate::sync::codes::normalize_code;
use crate::sync::crypto::Share;
use crate::sync::media::{MediaEvent, MediaKind};
use crate::sync::oplog::{DeviceKind, DeviceRole, DeviceStatus, HostUid};
use crate::sync::protocol::{ShareAsk, ShareReply};
use crate::sync::quorum::{
    make_challenge, peer_poll, peer_request, phone_device, Challenge, PeerRequest, ShareSource,
};
use crate::sync::runtime::{Peers, SyncRuntime};
use crate::sync::share_release::unwrap_share_response;
use crate::sync::unlock_paths::{
    apply_reply_ext, full_entry_share, open_phone_reply, open_phone_reply_clipboard,
    share_from_code,
};

/// How long a phone challenge stays valid.
const CHALLENGE_TTL_SECS: i64 = 600;
/// The terminal area the challenge QR is sized for (columns, rows).
const QR_AREA: (u16, u16) = (76, 44);
/// How long the modal waits for the user before the unlock gives up.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(600);
/// How often the wait looks at the listeners and the typed answers.
const MEDIA_POLL: Duration = Duration::from_millis(200);
/// How often the attended path syncs with the holders while the user may type.
const ATTENDED_POLL: Duration = Duration::from_secs(4);

/// The challenge shown to the phone and what checks its answer.
struct PhoneAsk {
    phone: crate::sync::oplog::DeviceId,
    phone_ed25519: [u8; 32],
    challenge: Challenge,
    eph: Zeroizing<[u8; 32]>,
}

pub(crate) struct UiShares {
    pub events: Sender<SyncEvent>,
    pub answers: Mutex<Receiver<String>>,
    pub runtime: SyncRuntime,
    /// The attended request of this unlock, kept across share epochs.
    pub request: Mutex<Option<PeerRequest>>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn failed(e: anyhow::Error) -> UnlockError {
    UnlockError::Failed(format!("{e:#}"))
}

impl UiShares {
    fn say(&self, kind: UnlockPathKind, status: impl Into<String>) {
        let _ = self.events.send(SyncEvent::UnlockPath(kind, status.into()));
    }

    /// The next thing the user typed or pasted; `None` when they cancelled.
    fn answer(&self) -> Option<String> {
        self.answers.lock().ok()?.recv_timeout(ANSWER_TIMEOUT).ok()
    }

    /// Ask each holder that is reachable and not the phone; `Granted` ends it.
    fn via_trusted_peers(&self, host: &HostUid, epoch: u64) -> Option<Share> {
        let rt = &self.runtime;
        let rows = crate::sync::ceremony::effective_devices(&rt.store, None).ok()?;
        let me = rt.identity.public().device_id();
        let peers = Peers::load(&rt.paths).ok()?;
        for d in rows.iter().filter(|d| {
            d.id != me
                && d.role == DeviceRole::Holder
                && d.kind == DeviceKind::Terminal
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        }) {
            if peers.host_of(&d.id).is_none() {
                continue;
            }
            self.say(UnlockPathKind::Peer, format!("asking {}", d.name));
            let mut secret = Zeroizing::new([0u8; 32]);
            OsRng.fill_bytes(&mut *secret);
            let mut rid = [0u8; 8];
            OsRng.fill_bytes(&mut rid);
            let ask = ShareAsk {
                host_uid: *host,
                epoch,
                eph: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(*secret))
                    .to_bytes(),
                request_id: u64::from_be_bytes(rid) >> 11,
            };
            match rt.share_request(&d.id, &ask) {
                Ok(ShareReply::Granted(sealed)) => match unwrap_share_response(&secret, &sealed) {
                    Ok(share) => {
                        self.say(UnlockPathKind::Peer, format!("{} released a share", d.name));
                        return Some(share);
                    }
                    Err(e) => self.say(UnlockPathKind::Peer, format!("{}: {e:#}", d.name)),
                },
                Ok(ShareReply::Deferred(reason)) => {
                    self.say(UnlockPathKind::Peer, format!("{}: {reason}", d.name));
                }
                Err(e) => self.say(UnlockPathKind::Peer, format!("{}: {e}", d.name)),
            }
        }
        None
    }

    /// Take what the user typed for what it is: 24 words, a short code, or the
    /// pasted reply.
    fn read_reply(
        &self,
        text: &str,
        host: &HostUid,
        epoch: u64,
        ask: &PhoneAsk,
    ) -> std::result::Result<Share, (UnlockPathKind, anyhow::Error)> {
        let store = &self.runtime.store;
        if text.split_whitespace().count() == 24 {
            return full_entry_share(store, host, epoch, &ask.phone, text)
                .map_err(|e| (UnlockPathKind::Words24, e));
        }
        if normalize_code(text).is_ok_and(|c| c.len() == 16) {
            return share_from_code(store, host, text, now())
                .map_err(|e| (UnlockPathKind::ShortCode, e));
        }
        let reply = open_phone_reply_clipboard(text, &ask.challenge, &ask.eph, &ask.phone_ed25519)
            .map_err(|e| (UnlockPathKind::Clipboard, e))?;
        Ok(self.keep_reply(reply, UnlockPathKind::Clipboard))
    }

    /// A verified reply: keep its courier ops and snapshot, return the share.
    /// Failing to keep the extras must not lose a good reply.
    fn keep_reply(
        &self,
        reply: crate::sync::unlock_paths::PhoneReply,
        kind: UnlockPathKind,
    ) -> Share {
        if let Some(ext) = &reply.ext {
            if let Err(e) = apply_reply_ext(&self.runtime.store, ext) {
                self.say(kind, format!("reply accepted; courier ops not kept: {e:#}"));
            }
        }
        reply.share
    }

    /// What a listener (microphone, camera) reports: statuses and progress
    /// into the modal, a reply checked against the challenge.
    fn media_event(&self, event: MediaEvent, ask: Option<&PhoneAsk>) -> Option<Share> {
        let path = |k: MediaKind| match k {
            MediaKind::Audio => UnlockPathKind::Audio,
            MediaKind::Camera => UnlockPathKind::Camera,
        };
        match event {
            MediaEvent::Status(k, line) => self.say(path(k), line),
            MediaEvent::Progress(k, held, needed) => {
                self.say(path(k), format!("reading the code: {held} of {needed}"));
            }
            MediaEvent::Reply(k, bytes) => {
                let ask = ask?;
                match open_phone_reply(&bytes, &ask.challenge, &ask.eph, &ask.phone_ed25519) {
                    Ok(reply) => return Some(self.keep_reply(reply, path(k))),
                    Err(e) => self.say(path(k), format!("not accepted: {e:#}")),
                }
            }
        }
        None
    }

    /// The holders worth asking: other terminals that hold shares, not revoked
    /// or compromised, that this device knows how to reach.
    fn reachable_holders(&self) -> Vec<(crate::sync::oplog::DeviceId, String)> {
        let rt = &self.runtime;
        let (Ok(rows), Ok(peers)) = (
            crate::sync::ceremony::effective_devices(&rt.store, None),
            Peers::load(&rt.paths),
        ) else {
            return Vec::new();
        };
        let me = rt.identity.public().device_id();
        rows.into_iter()
            .filter(|d| {
                d.id != me
                    && d.role == DeviceRole::Holder
                    && d.kind == DeviceKind::Terminal
                    && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
                    && peers.host_of(&d.id).is_some()
            })
            .map(|d| (d.id, d.name))
            .collect()
    }

    /// One look for the attended grant: a sync round with each holder (our
    /// request goes out, their grants come in), then the log.
    fn poll_attended(
        &self,
        holders: &[(crate::sync::oplog::DeviceId, String)],
        req: &PeerRequest,
        epoch: u64,
    ) -> Option<Share> {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        for (id, name) in holders {
            if let Err(e) = self.runtime.sync_with(id, &cancel) {
                self.say(UnlockPathKind::Peer, format!("{name}: {e}"));
            }
        }
        match peer_poll(&self.runtime.store, req, epoch) {
            Ok(Some(share)) => {
                self.say(UnlockPathKind::Peer, "a holder approved on its screen");
                Some(share)
            }
            _ => None,
        }
    }

    /// The phone's challenge, shown and armed; `None` when the ring has no phone.
    fn phone_ask(&self, host: &HostUid) -> std::result::Result<Option<PhoneAsk>, UnlockError> {
        let rt = &self.runtime;
        let Some(phone) = phone_device(&rt.store).map_err(failed)? else {
            return Ok(None);
        };
        let phone_ed25519 = rt
            .store
            .sync_devices()
            .map_err(failed)?
            .into_iter()
            .find(|d| d.id == phone)
            .map(|d| d.public.ed25519_pub)
            .ok_or_else(|| failed(anyhow!("the phone is not in the ring")))?;
        let (challenge, eph) = make_challenge(
            &rt.store,
            &rt.identity,
            host,
            &phone,
            now(),
            CHALLENGE_TTL_SECS,
        )
        .map_err(failed)?;
        let ask = PhoneAsk {
            phone,
            phone_ed25519,
            challenge,
            eph,
        };
        let frame_bytes = crate::sync::fountain::frame_bytes_for_area(QR_AREA.0, QR_AREA.1);
        match frame_bytes.map(|b| ask.challenge.to_qr_payload(b)) {
            Some(Ok(frames)) => {
                let _ = self.events.send(SyncEvent::UnlockChallenge(frames));
                self.say(
                    UnlockPathKind::PhoneQr,
                    "show the code to the phone, then paste or type its answer",
                );
            }
            Some(Err(e)) => self.say(
                UnlockPathKind::PhoneQr,
                format!("no QR for this challenge: {e:#}"),
            ),
            None => self.say(UnlockPathKind::PhoneQr, "no QR fits this terminal"),
        }
        self.say(UnlockPathKind::Clipboard, "paste the phone's reply here");
        self.say(UnlockPathKind::ShortCode, "or type its 16 character code");
        self.say(UnlockPathKind::Words24, "or type the 24 words it shows");
        Ok(Some(ask))
    }

    /// After the trust-session path: ask the attended holders (their screens
    /// show a release request naming this device) and arm the phone, then wait
    /// for whichever answers first.
    fn wait_for_second(
        &self,
        host: &HostUid,
        epoch: u64,
    ) -> std::result::Result<Share, UnlockError> {
        let rt = &self.runtime;
        let holders = self.reachable_holders();
        let request = if holders.is_empty() {
            None
        } else {
            let mut slot = self.request.lock().map_err(|_| failed(anyhow!("lock")))?;
            if slot.as_ref().is_none_or(|r| r.host != *host) {
                *slot = Some(peer_request(&rt.store, &rt.identity, host).map_err(failed)?);
            }
            let names: Vec<&str> = holders.iter().map(|(_, n)| n.as_str()).collect();
            self.say(
                UnlockPathKind::Peer,
                format!("ask {} to approve on its screen", names.join(" or ")),
            );
            slot.take()
        };
        let phone = self.phone_ask(host)?;
        if request.is_none() && phone.is_none() {
            return Err(UnlockError::NeedsShares(
                "no holder released a share, none can be asked from here, and the ring has no phone: unlock on a device that can, or use a Vault ring".into(),
            ));
        }
        // The microphone and the camera listen only while this wait lasts.
        let (media_tx, media_rx) = std::sync::mpsc::channel();
        #[cfg(test)]
        test_hooks::register(&rt.paths.root, media_tx.clone());
        let _listening = phone
            .as_ref()
            .map(|_| crate::sync::media::listen(&[MediaKind::Audio, MediaKind::Camera], media_tx));
        let mut last_poll: Option<std::time::Instant> = None;
        let result = (|| loop {
            if let Some(req) = &request {
                if last_poll.is_none_or(|t| t.elapsed() >= ATTENDED_POLL) {
                    last_poll = Some(std::time::Instant::now());
                    if let Some(share) = self.poll_attended(&holders, req, epoch) {
                        return Ok(share);
                    }
                }
            }
            while let Ok(event) = media_rx.try_recv() {
                if let Some(share) = self.media_event(event, phone.as_ref()) {
                    return Ok(share);
                }
            }
            match self
                .answers
                .lock()
                .map_err(|_| failed(anyhow!("lock")))?
                .recv_timeout(MEDIA_POLL)
            {
                Ok(text) => {
                    let Some(ask) = &phone else {
                        self.say(
                            UnlockPathKind::Peer,
                            "waiting for a holder: nothing to type here",
                        );
                        continue;
                    };
                    match self.read_reply(&text, host, epoch, ask) {
                        Ok(share) => return Ok(share),
                        Err((kind, e)) => self.say(kind, format!("not accepted: {e:#}")),
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(UnlockError::Cancelled)
                }
            }
        })();
        // Keep the request for the next epoch of the same unlock.
        if let (Some(req), Ok(mut slot)) = (request, self.request.lock()) {
            *slot = Some(req);
        }
        result
    }
}

impl ShareSource for UiShares {
    fn master_phrase(&self) -> Option<Zeroizing<String>> {
        let _ = self.events.send(SyncEvent::UnlockPrompt(
            "Type the six words of the master phrase, then Enter".into(),
        ));
        self.answer().map(Zeroizing::new)
    }

    fn second_share(
        &self,
        host: &HostUid,
        epoch: u64,
        _mine: &Share,
    ) -> std::result::Result<Share, UnlockError> {
        if let Some(share) = self.via_trusted_peers(host, epoch) {
            return Ok(share);
        }
        self.wait_for_second(host, epoch)
    }
}

/// Tests reach the running wait's media channel by profile directory.
#[cfg(test)]
pub(crate) mod test_hooks {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::Sender;
    use std::sync::Mutex;

    use crate::sync::media::MediaEvent;

    static CHANNELS: Mutex<Option<HashMap<PathBuf, Sender<MediaEvent>>>> = Mutex::new(None);

    pub(crate) fn register(root: &Path, tx: Sender<MediaEvent>) {
        CHANNELS
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(root.to_path_buf(), tx);
    }

    /// The sender of the wait running for `root`, once it started.
    pub(crate) fn channel(root: &Path) -> Option<Sender<MediaEvent>> {
        CHANNELS.lock().unwrap().as_ref()?.get(root).cloned()
    }
}
