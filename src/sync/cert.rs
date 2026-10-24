//! Trust-session certificates: what the share-holding peers enforce before releasing a share to a
//! device that holds a delegation certificate. The constraints live here and in the log, never on
//! the delegated machine.

use anyhow::{bail, ensure, Result};
use rusqlite::params;

use super::crypto::device_id_of;
use super::limits::{CERT_NEW_HOSTS_PER_DAY, CERT_TERM_MAX_SECS, PHONE_FRESH_SECS};
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
        "the certificate is not signed by the phone"
    );
    ensure!(
        cert.id == cert.op.hash(),
        "certificate id does not match its op"
    );
    ensure!(
        cert.device_id == *requester,
        "the certificate was issued to another device"
    );
    ensure!(
        !store.sync_cert_revoked(&cert.id)?,
        "the certificate was revoked"
    );
    ensure!(now < cert.expiry, "the certificate has expired");
    ensure!(
        cert.expiry - cert.issued_at() <= CERT_TERM_MAX_SECS,
        "the certificate term exceeds the cap"
    );
    ensure!(
        now - phone_head_ts <= PHONE_FRESH_SECS,
        "the phone has not been seen recently enough to vouch for a trust session"
    );
    let epoch = store.sync_epoch(&cert.issuer)?;
    ensure!(
        (cert.epoch_range.0..=cert.epoch_range.1).contains(&epoch),
        "the phone's epoch is outside the certificate's range"
    );
    match &cert.scope {
        CertScope::Hosts(list) => ensure!(
            list.contains(host),
            "the host is outside the certificate's scope"
        ),
        CertScope::Group(g) => ensure!(
            host_in_group(store, host, g)?,
            "the host is outside the certificate's group"
        ),
    }
    // Hosts already opened under this certificate are free; a new one counts against the
    // ring-wide daily budget, read from the log.
    if !store.sync_cert_hosts_opened(&cert.id)?.contains(host) {
        let used = store.sync_cert_new_hosts_since(&cert.id, now - DAY)?;
        ensure!(
            used < cert.max_new_hosts_per_day,
            "the daily budget of new hosts is used up: the phone must extend the day"
        );
    }
    Ok(())
}

/// Checks a `CertIssue` op before it is shown for approval: structure, scope, term, budget and the
/// ring size (three holders, otherwise nobody enforces it).
pub fn validate_cert_issue(store: &LauncherStore, op: &Op, phone: &DeviceId) -> Result<()> {
    let cert = DelegationCert::from_op(op)?;
    ensure!(cert.issuer == *phone, "only the phone issues certificates");
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
    let holders = store
        .sync_devices()?
        .into_iter()
        .filter(|d| {
            d.role == DeviceRole::Holder
                && !matches!(d.status, DeviceStatus::Revoked | DeviceStatus::Compromised)
        })
        .count();
    ensure!(
        holders >= 3,
        "trust sessions need at least three holders: with fewer nobody enforces the certificate"
    );
    ensure!(
        store
            .sync_devices()?
            .iter()
            .any(|d| d.id == cert.device_id && d.role == DeviceRole::Holder),
        "the certificate names a device that is not a holder"
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
