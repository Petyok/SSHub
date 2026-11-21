//! Trust-session certificates: what the share-holding peers enforce before releasing a share to a
//! device that holds a delegation certificate. The constraints live here and in the log, never on
//! the delegated machine.

use anyhow::{bail, ensure, Result};
use rusqlite::params;

use super::crypto::device_id_of;
use super::limits::{
    CERT_LIFETIME_MAX_SECS, CERT_MAX_RENEWALS, CERT_NEW_HOSTS_PER_DAY, CERT_TERM_MAX_SECS,
    PHONE_FRESH_SECS,
};
use super::oplog::{CertScope, DeviceId, DeviceRole, DeviceStatus, GroupUid, HostUid, Op, OpBody};
use crate::store::LauncherStore;

const DAY: i64 = 86_400;

/// A `CertIssue` op read as a certificate. `sig` is the issuer's signature over the op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegationCert {
    pub id: [u8; 32],
    pub device_id: DeviceId,
    pub scope: CertScope,
    pub expiry: i64,
    pub max_new_hosts_per_day: u32,
    pub epoch_range: (u64, u64),
    pub issuer: DeviceId,
    pub sig: [u8; 64],
    op: Op,
}

impl DelegationCert {
    pub fn from_op(op: &Op) -> Result<Self> {
        let OpBody::CertIssue {
            device_id,
            scope,
            expiry,
            max_new_hosts_per_day,
            epoch_lo,
            epoch_hi,
        } = &op.body
        else {
            bail!("not a certificate op");
        };
        Ok(Self {
            id: op.hash(),
            device_id: device_id.clone(),
            scope: scope.clone(),
            expiry: *expiry,
            max_new_hosts_per_day: *max_new_hosts_per_day,
            epoch_range: (*epoch_lo, *epoch_hi),
            issuer: op.device_id.clone(),
            sig: op.signature,
            op: op.clone(),
        })
    }

    pub fn issued_at(&self) -> i64 {
        self.op.timestamp
    }

    /// The renewals that count, from the folded record of the log (`CertRenew` ops judged clock-free in
    /// `(lamport, device, seq)` order: author, term, lifetime and count caps; design 6.1) and cut at the first one written
    /// when the delegate was no holder in force: a renewal cannot revive a session for a device that was removed.
    fn counted_renewals(&self, store: &LauncherStore) -> Result<Vec<super::oplog::Renewal>> {
        let Some(record) = store.sync_cert_record(&self.id)? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for r in record.accepted {
            let holder = super::ceremony::effective_devices(store, Some(r.lamport))?
                .into_iter()
                .any(|d| {
                    d.id == self.device_id
                        && d.role == DeviceRole::Holder
                        && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
                });
            if !holder {
                break;
            }
            out.push(r);
        }
        Ok(out)
    }

    /// The end date that counts: the issue op's, or the newest counted renewal's. The local clock is read only by the
    /// caller, against this date, when the session is used.
    pub fn effective_expiry(&self, store: &LauncherStore) -> Result<i64> {
        Ok(self
            .counted_renewals(store)?
            .last()
            .map_or(self.expiry, |r| r.expiry))
    }

    /// How many renewals count (at most `CERT_MAX_RENEWALS`).
    pub fn renewals(&self, store: &LauncherStore) -> Result<u32> {
        Ok(u32::try_from(self.counted_renewals(store)?.len()).unwrap_or(u32::MAX))
    }
}

fn host_in_group(store: &LauncherStore, host: &HostUid, group: &GroupUid) -> Result<bool> {
    let (Some(h), Some(g)) = (store.host_id_by_uid(host)?, store.group_id_by_uid(group)?) else {
        return Ok(false);
    };
    store.with_conn(|c| {
        Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM host_group_memberships WHERE host_id = ?1 AND group_id = ?2)", params![h, g], |r| r.get(0))?)
    })
}

/// May `requester` have a share for `host` under `cert` right now? `issuer_ed25519` is the phone's
/// key as this device knows it; `phone_head_ts` the time of the newest head seen from the phone.
pub fn cert_allows(
    cert: &DelegationCert,
    issuer_ed25519: &[u8; 32],
    store: &LauncherStore,
    requester: &DeviceId,
    host: &HostUid,
    phone_head_ts: i64,
    now: i64,
) -> Result<()> {
    ensure!(
        cert.op.verify(issuer_ed25519) && device_id_of(issuer_ed25519) == cert.issuer,
        "the trust session is not signed by the phone"
    );
    ensure!(
        cert.id == cert.op.hash(),
        "certificate id does not match its op"
    );
    // The phone's own op counts only once its ceremony completed, on every device.
    ensure!(
        super::ceremony::in_force(store, &cert.op)?,
        "the trust session is not approved yet"
    );
    ensure!(
        cert.device_id == *requester,
        "the trust session was made for another device"
    );
    ensure!(
        !store.sync_cert_revoked(&cert.id)?,
        "the trust session was ended"
    );
    ensure!(
        now < cert.effective_expiry(store)?,
        "the trust session has expired"
    );
    ensure!(
        cert.expiry - cert.issued_at() <= CERT_TERM_MAX_SECS,
        "the trust session lasts longer than allowed (90 days)"
    );
    ensure!(
        now - phone_head_ts <= PHONE_FRESH_SECS,
        "the phone has not touched any of your computers for 14 days, so this trust session stopped: open the phone app and scan something from a terminal"
    );
    let epoch = store.sync_epoch(&cert.issuer)?;
    ensure!(
        (cert.epoch_range.0..=cert.epoch_range.1).contains(&epoch),
        "the phone's sharing round is outside the trust session's range"
    );
    match &cert.scope {
        CertScope::Hosts(list) => ensure!(
            list.contains(host),
            "the host is not covered by the trust session"
        ),
        CertScope::Group(g) => ensure!(
            host_in_group(store, host, g)?,
            "the host is not in the trust session's group"
        ),
    }
    // Hosts already opened under this certificate are free; a new one counts against the
    // ring-wide daily budget, read from the log.
    let openings = recorded_openings(store, cert)?;
    if !openings.iter().any(|(h, _)| h == host) {
        let used = openings.iter().filter(|(_, at)| *at >= now - DAY).count() as u32;
        ensure!(
            used < cert
                .max_new_hosts_per_day
                .saturating_add(extension_budget(store, cert, now)?),
            "the daily budget of new hosts is used up: the phone must extend the day"
        );
    }
    Ok(())
}

/// The hosts opened under `cert` and when, as recorded by the enforcing side: `CertHostOpened` ops by
/// holders in force that are neither the delegate nor the device that is asking. A delegate writing its
/// own "opened" records, or a mirror doing it, buys nothing. The time is the op's stamp, never later
/// than when this device first saw it.
pub fn recorded_openings(
    store: &LauncherStore,
    cert: &DelegationCert,
) -> Result<Vec<(HostUid, i64)>> {
    let holders: Vec<DeviceId> = super::ceremony::effective_devices(store, None)?
        .into_iter()
        .filter(|d| {
            d.role == DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        })
        .map(|d| d.id)
        .collect();
    let mut seen: Vec<(HostUid, i64)> = Vec::new();
    for op in super::quorum::ops_of(store, "CertHostOpened", None)? {
        let OpBody::CertHostOpened { cert_id, host_uid } = &op.body else {
            continue;
        };
        if *cert_id != cert.id || op.device_id == cert.device_id || !holders.contains(&op.device_id)
        {
            continue;
        }
        let at = super::ceremony::base_time(store, &op);
        match seen.iter_mut().find(|(h, _)| h == host_uid) {
            Some(e) => e.1 = e.1.min(at),
            None => seen.push((*host_uid, at)),
        }
    }
    Ok(seen)
}

/// Extra new hosts the phone granted for the rolling day ending at `now` (R3b): `CertExtend` ops by the
/// certificate's issuer before expiry, at most one counted per 24 hours, each valid for the 24 hours
/// after it.
pub fn extension_budget(store: &LauncherStore, cert: &DelegationCert, now: i64) -> Result<u32> {
    let end = cert.effective_expiry(store)?;
    // By the issuer or by the phone that took over its certificates in a handover (design 5.4).
    let mut ops: Vec<(i64, u32)> = Vec::new();
    for o in super::quorum::ops_of(store, "CertExtend", None)? {
        let OpBody::CertExtend {
            cert_id,
            extra_hosts,
        } = &o.body
        else {
            continue;
        };
        if *cert_id != cert.id || o.timestamp >= end {
            continue;
        }
        if store.sync_cert_holder_at(&cert.issuer, (o.lamport, &o.device_id, o.seq))? != o.device_id
        {
            continue;
        }
        ops.push((o.timestamp, *extra_hosts));
    }
    ops.sort();
    let mut last: Option<i64> = None;
    let mut sum = 0u32;
    for (ts, extra) in ops {
        if last.is_some_and(|l| ts - l < DAY) {
            continue;
        }
        last = Some(ts);
        if ts <= now && now - ts < DAY {
            sum = sum.saturating_add(extra);
        }
    }
    Ok(sum)
}

/// The request the terminal shows the phone when the daily budget is used up (kind 10).
pub fn cert_extend_request(
    cert_id: [u8; 32],
    extra_hosts: u32,
) -> Result<super::wire::CertExtendRequestWire> {
    ensure!(
        (1..=CERT_NEW_HOSTS_PER_DAY).contains(&extra_hosts),
        "an extension is 1 to {CERT_NEW_HOSTS_PER_DAY} hosts"
    );
    Ok(super::wire::CertExtendRequestWire {
        cert_id,
        extra_hosts,
    })
}

/// Checks a `CertIssue` op before it is shown for approval: structure, scope, term, budget and the
/// ring size (three holders, otherwise nobody enforces it).
pub fn validate_cert_issue(store: &LauncherStore, op: &Op, phone: &DeviceId) -> Result<()> {
    let cert = DelegationCert::from_op(op)?;
    ensure!(
        cert.issuer == *phone,
        "only the phone can start a trust session"
    );
    ensure!(
        cert.expiry > op.timestamp && cert.expiry - op.timestamp <= CERT_TERM_MAX_SECS,
        "the term must be positive and at most 90 days"
    );
    ensure!(
        (1..=CERT_NEW_HOSTS_PER_DAY).contains(&cert.max_new_hosts_per_day),
        "the daily budget is outside 1..=50"
    );
    ensure!(
        cert.epoch_range.0 <= cert.epoch_range.1,
        "the epoch range is inverted"
    );
    match &cert.scope {
        CertScope::Hosts(h) => ensure!(
            !h.is_empty() && h.len() <= 10_000,
            "an explicit host list must be non-empty"
        ),
        CertScope::Group(_) => {}
    }
    let devices = super::ceremony::effective_devices(store, None)?;
    let holders = devices
        .iter()
        .filter(|d| {
            d.role == DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        })
        .count();
    ensure!(
        holders >= 3,
        "trust sessions need at least three full devices: with fewer nobody enforces the trust session"
    );
    ensure!(
        devices
            .iter()
            .any(|d| d.id == cert.device_id && d.role == DeviceRole::Holder),
        "the trust session names a device that is not a full device"
    );
    Ok(())
}

/// The `CertHostOpened` record a releasing holder appends the first time a host is opened.
pub fn cert_opened_body(cert: &DelegationCert, host: &HostUid) -> OpBody {
    OpBody::CertHostOpened {
        cert_id: cert.id,
        host_uid: *host,
    }
}

/// What a terminal shows the phone to ask for a trust session: the user's choices, with the phone's
/// current epoch as the start of the epoch range (`epoch_slack` more epochs stay valid). The phone
/// builds and signs the op; the terminal then receives it as courier ops or a pasted phone-ops frame.
pub fn cert_request(
    store: &LauncherStore,
    delegate: &DeviceId,
    scope: CertScope,
    term_secs: i64,
    max_new_hosts_per_day: u32,
    epoch_slack: u64,
    now: i64,
) -> Result<super::wire::CertRequestWire> {
    use anyhow::Context;
    let phone = super::quorum::phone_device(store)?.context("the ring has no phone")?;
    ensure!(
        (1..=CERT_TERM_MAX_SECS).contains(&term_secs),
        "the term must be between a second and 90 days"
    );
    ensure!(
        (1..=CERT_NEW_HOSTS_PER_DAY).contains(&max_new_hosts_per_day),
        "the daily budget is outside 1..=50"
    );
    let raw: [u8; 8] = hex::decode(delegate)
        .ok()
        .and_then(|b| b.try_into().ok())
        .context("malformed device id")?;
    let epoch = store.sync_epoch(&phone)?;
    Ok(super::wire::CertRequestWire {
        device_id: raw,
        scope,
        expiry: now + term_secs,
        max_new_hosts_per_day,
        epoch_lo: epoch,
        epoch_hi: epoch.saturating_add(epoch_slack),
    })
}

/// Revoke a certificate (Yellow): any attended holder writes it; peers stop honouring the cert at once.
pub fn cert_revoke_body(cert: &DelegationCert) -> OpBody {
    OpBody::CertRevoke { cert_id: cert.id }
}

/// What a terminal shows the phone to renew a trust session (kind 11, design 6.2). The new end date is `term_secs` after
/// the end that counts, never more than the original term, never past `CERT_LIFETIME_MAX_SECS` after the issue; the
/// evidence is one entry per holder terminal other than the delegate: its newest op seq and that op's author stamp, so the
/// phone can tell whether the list of opened hosts comes from someone who saw the delegate's recent work. The request is
/// built from a terminal that is NOT the delegate; `now` is not a rule (a UI rate hint at most).
pub fn cert_renew_request(
    store: &LauncherStore,
    cert_id: [u8; 32],
    term_secs: i64,
    _now: i64,
) -> Result<super::wire::CertRenewRequestWire> {
    use anyhow::Context;
    let issue = super::quorum::ops_of(store, "CertIssue", None)?
        .into_iter()
        .find(|o| o.hash() == cert_id)
        .context("no such trust session")?;
    let cert = DelegationCert::from_op(&issue)?;
    ensure!(
        !store.sync_cert_revoked(&cert_id)?,
        "the trust session was ended"
    );
    ensure!(
        cert.renewals(store)? < CERT_MAX_RENEWALS,
        "this trust session was renewed {CERT_MAX_RENEWALS} times already: a new one needs the full approval"
    );
    let original_term = cert.expiry - cert.issued_at();
    ensure!(
        term_secs > 0 && term_secs <= original_term,
        "a renewal is at most as long as the first term"
    );
    let so_far = cert.effective_expiry(store)?;
    let expiry = (so_far + term_secs).min(cert.issued_at() + CERT_LIFETIME_MAX_SECS);
    ensure!(
        expiry > so_far,
        "the trust session already reaches the longest lifetime (180 days)"
    );
    let mut evidence = Vec::new();
    for d in super::ceremony::effective_devices(store, None)? {
        if d.role != DeviceRole::Holder
            || d.kind != super::oplog::DeviceKind::Terminal
            || d.id == cert.device_id
            || matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        {
            continue;
        }
        let Some(terminal) = hex::decode(&d.id)
            .ok()
            .and_then(|b| <[u8; 8]>::try_from(b).ok())
        else {
            continue;
        };
        let head_seq = store
            .sync_chain_heads()?
            .into_iter()
            .find(|(id, _, _)| *id == d.id)
            .map_or(0, |(_, seq, _)| seq);
        let head_ts: i64 = store.with_conn(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(timestamp), 0) FROM sync_ops
                 WHERE device_id = ?1 AND state NOT IN ('rejected', 'void')",
                params![d.id],
                |r| r.get(0),
            )?)
        })?;
        evidence.push(super::wire::HeadEvidence {
            terminal,
            head_seq,
            head_ts,
        });
        if evidence.len() == super::wire::MAX_RENEW_EVIDENCE {
            break;
        }
    }
    // The hosts opened under this session as the HOLDERS recorded them (never the delegate's own claim), newest first:
    // up to eight names and the total, so the phone can show what the delegate did before it signs (contract 4l a).
    let mut openings = recorded_openings(store, &cert)?;
    openings.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    let opened_total = u16::try_from(openings.len()).unwrap_or(u16::MAX);
    let mut opened = Vec::new();
    for (uid, _) in openings.iter().take(8) {
        let name: Option<String> = store.with_conn(|c| {
            Ok(c.query_row(
                "SELECT name FROM hosts WHERE uid = ?1",
                params![&uid[..]],
                |r| r.get(0),
            )
            .ok())
        })?;
        let mut name = name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| hex::encode(&uid[..4]));
        while name.len() > 48 {
            name.pop();
        }
        opened.push(name);
    }
    Ok(super::wire::CertRenewRequestWire {
        cert_id,
        expiry,
        evidence,
        opened,
        opened_total,
    })
}

fn renew_mailed_key(op: &Op) -> String {
    format!("renew_mailed:{}", hex::encode(op.hash()))
}

/// The renewals of trust sessions this device has not mailed yet: `(op hash, subject, body)`. Every `CertRenew` ingested
/// is mailed, whether or not it counts, so a quiet renewal by a stolen phone is seen (design 6.1).
pub fn owed_renew_mails(store: &LauncherStore) -> Result<Vec<([u8; 32], String, String)>> {
    let issues = super::quorum::ops_of(store, "CertIssue", None)?;
    let devices = store.sync_devices()?;
    let mut out = Vec::new();
    for op in super::quorum::ops_of(store, "CertRenew", None)? {
        let OpBody::CertRenew { cert_id, expiry } = &op.body else {
            continue;
        };
        if super::quorum::meta_get(store, &renew_mailed_key(&op))?.is_some() {
            continue;
        }
        let Some(cert) = issues
            .iter()
            .find(|i| i.hash() == *cert_id)
            .and_then(|i| DelegationCert::from_op(i).ok())
        else {
            continue;
        };
        let delegate = devices
            .iter()
            .find(|d| d.id == cert.device_id)
            .map_or(cert.device_id.clone(), |d| d.name.clone());
        out.push((
            op.hash(),
            "A trust session was renewed".to_string(),
            format!(
                "The phone renewed the trust session of {delegate} until {}. {} of {CERT_MAX_RENEWALS} renewals are counted so far. If this was not you, end the trust session from a computer and look at your phone.",
                super::hub::utc(*expiry),
                cert.renewals(store)?,
            ),
        ));
    }
    Ok(out)
}

/// Mail what [`owed_renew_mails`] lists through `send`; a renewal is marked mailed only when `send` succeeded. Returns how
/// many went out.
pub fn send_renew_mails(
    store: &LauncherStore,
    send: &mut dyn FnMut(&str, &str) -> Result<()>,
) -> Result<usize> {
    let mut sent = 0;
    for (hash, subject, body) in owed_renew_mails(store)? {
        if send(&subject, &body).is_ok() {
            super::quorum::meta_set(store, &format!("renew_mailed:{}", hex::encode(hash)), b"1")?;
            sent += 1;
        }
    }
    Ok(sent)
}

/// The phones whose certificates the phone `current` honours: itself and every phone it took over from in a handover
/// (their `CertIssue` signatures still verify with the retired phone's key, which stays in the device table).
pub fn issuer_phones(store: &LauncherStore, current: &DeviceId) -> Result<Vec<DeviceId>> {
    let mut out = vec![current.clone()];
    for h in store.sync_handovers()? {
        if store.sync_successor_of(&h.old_phone)?.as_ref() == Some(current)
            && !out.contains(&h.old_phone)
        {
            out.push(h.old_phone);
        }
    }
    Ok(out)
}
