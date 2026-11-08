//! Trust-session certificates on the phone (contract 4c-2, P1). Port of `assets/companion/src/cert.mjs` and the trust
//! screens of `app.mjs`. The phone AUTHORS and signs `CertIssue`, `CertRevoke` and `CertExtend`; this module decides
//! what it may sign and renders what it was asked, both from the bytes of the request (kind 9, kind 10). Everything the
//! phone knows about the certificates it issued is read back from its own chain, so there is no second record to drift.

use std::collections::BTreeMap;

use sshub_sync_proto::limits::{CERT_NEW_HOSTS_PER_DAY, CERT_TERM_MAX_SECS};
use sshub_sync_proto::oplog::{canonical::decode_op, CertScope, OpBody};
use sshub_sync_proto::uid::uid_hex;
use sshub_sync_proto::wire::{CertExtendRequestWire, CertRequestWire, MAX_CERT_REQUEST_HOSTS};

use crate::chain;
use crate::ctx::Ctx;
use crate::error::{CoreError, Result};
use crate::identity::PhoneIdentity;
use crate::outgoing::ops_out;
use crate::state::{PhoneState, COURIER_TAIL};
use crate::text::{iso, safe_text};
use crate::views::{
    CertExtendView, CertExtensionView, CertRequestView, CertStatus, IssuedCertView, Opened,
    RefusalView, Row,
};

/// A terminal asks for the phone's current epoch plus this much slack (`src/app/sync_trust.rs` passes 1): a certificate
/// then ends with the next reshare that revokes someone. A wider range would outlive it.
pub const CERT_EPOCH_SLACK: u64 = 1;
/// One extension per certificate per rolling 24 hours.
pub const EXTEND_WINDOW_SECS: i64 = 86_400;

#[cfg(test)]
mod tests;

/// A request the person looked at and found acceptable (no problems): what `approve_cert` signs.
#[derive(Clone, Debug)]
pub enum PendingCert {
    Request(CertRequestWire),
    Extend(CertExtendRequestWire),
}

/// `1 day 3 h`, `2 h 5 min`, `7 min`, `45 s`; floor semantics of the page (`Math.floor`), so a negative term reads the
/// same as on the page.
pub fn describe_term(secs: i64) -> String {
    let s = i128::from(secs);
    let d = s.div_euclid(86_400);
    let hours = (s % 86_400).div_euclid(3_600);
    let mins = (s % 3_600).div_euclid(60);
    if d != 0 {
        format!(
            "{d} day{}{}",
            if d == 1 { "" } else { "s" },
            if hours != 0 {
                format!(" {hours} h")
            } else {
                String::new()
            }
        )
    } else if hours != 0 {
        format!(
            "{hours} h{}",
            if mins != 0 {
                format!(" {mins} min")
            } else {
                String::new()
            }
        )
    } else if mins != 0 {
        format!("{mins} min")
    } else {
        format!("{s} s")
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Name of a device the person gave or the ring knows; `None` for an empty name (falsy on the page).
fn alias<'a>(aliases: &'a BTreeMap<String, String>, id: &str) -> Option<&'a str> {
    aliases
        .get(id)
        .map(String::as_str)
        .filter(|n| !n.is_empty())
}

fn scope_hosts_count(scope: &CertScope) -> Option<usize> {
    match scope {
        CertScope::Hosts(h) => Some(h.len()),
        CertScope::Group(_) => None,
    }
}

/// What the screen shows for a kind 9 request, every row from the decoded bytes: (headline, rows, delegate known).
pub fn describe_cert_request(
    req: &CertRequestWire,
    now: i64,
    aliases: &BTreeMap<String, String>,
) -> (String, Vec<Row>, bool) {
    let dev = hex::encode(req.device_id);
    let who = match alias(aliases, &dev) {
        Some(n) => format!("{} [{dev}]", safe_text(n)),
        None => format!("(no name known) [{dev}]"),
    };
    let term = req.expiry.saturating_sub(now);
    let scope_row = match &req.scope {
        CertScope::Group(g) => Row {
            label: "Scope: every host of group".into(),
            value: uid_hex(g),
            danger: true,
        },
        CertScope::Hosts(h) => Row {
            label: format!("Scope: {} host{} only", h.len(), plural(h.len())),
            value: h.iter().map(uid_hex).collect::<Vec<_>>().join("\n"),
            danger: true,
        },
    };
    let what = match scope_hosts_count(&req.scope) {
        None => "the hosts of a group".to_string(),
        Some(n) => format!("{n} listed host{}", plural(n)),
    };
    let rows = vec![
        Row {
            label: "Trusted device".into(),
            value: who.clone(),
            danger: true,
        },
        scope_row,
        Row {
            label: "Term".into(),
            value: format!("{} (until {})", describe_term(term), iso(req.expiry)),
            danger: true,
        },
        Row {
            label: "New hosts per day".into(),
            value: req.max_new_hosts_per_day.to_string(),
            danger: true,
        },
        Row {
            label: "Phone epochs it holds for".into(),
            value: format!("{} to {}", req.epoch_lo, req.epoch_hi),
            danger: true,
        },
    ];
    (
        format!("Let {who} open {what} without asking this phone again"),
        rows,
        alias(aliases, &dev).is_some(),
    )
}

/// The refusals, each a sentence a person can act on. Empty when the phone may sign. `own_epoch` is this phone's epoch:
/// the range must start there (F7).
pub fn cert_request_problems(
    req: &CertRequestWire,
    now: i64,
    own_id: &str,
    own_epoch: u64,
) -> Vec<String> {
    let mut out = Vec::new();
    let term = i128::from(req.expiry) - i128::from(now);
    if term < 1 {
        out.push("The term has already ended: the expiry is not in the future.".to_string());
    } else if term > i128::from(CERT_TERM_MAX_SECS) {
        out.push(format!(
            "The term is longer than 90 days ({term} s, {}). Ask for a shorter one.",
            describe_term(i64::try_from(term).unwrap_or(i64::MAX))
        ));
    }
    if req.max_new_hosts_per_day < 1 || req.max_new_hosts_per_day > CERT_NEW_HOSTS_PER_DAY {
        out.push(format!(
            "The daily budget of new hosts must be between 1 and {CERT_NEW_HOSTS_PER_DAY}, not {}.",
            req.max_new_hosts_per_day
        ));
    }
    if let CertScope::Hosts(h) = &req.scope {
        if h.is_empty() || h.len() > MAX_CERT_REQUEST_HOSTS {
            out.push(format!(
                "A list names 1 to {MAX_CERT_REQUEST_HOSTS} hosts; use a group for more."
            ));
        }
    }
    if req.epoch_lo > req.epoch_hi {
        out.push("The epoch range is inverted.".to_string());
    } else if req.epoch_hi - req.epoch_lo > CERT_EPOCH_SLACK {
        out.push(format!(
            "The epoch range {} to {} is wider than a terminal asks for (this phone's epoch plus {CERT_EPOCH_SLACK}): the certificate would outlive the next reshare.",
            req.epoch_lo, req.epoch_hi
        ));
    }
    if req.epoch_lo != own_epoch {
        out.push(format!(
            "The range starts at epoch {}, but this phone is at epoch {own_epoch}. Deliver this phone's newest ops to the terminal (Export my ops) and ask again.",
            req.epoch_lo
        ));
    }
    if hex::encode(req.device_id) == own_id {
        out.push("The request names this phone as the trusted device. A phone does not delegate to itself.".to_string());
    }
    out
}

/// A certificate this phone issued, read back from its own chain.
#[derive(Clone, Debug)]
pub struct IssuedCert {
    /// Hash of the `CertIssue` op (the certificate id).
    pub id: [u8; 32],
    pub device_id: String,
    pub scope: CertScope,
    /// The signed time of the `CertIssue` op.
    pub issued_at: i64,
    pub expiry: i64,
    pub max_new_hosts_per_day: u32,
    pub epoch_lo: u64,
    pub epoch_hi: u64,
    pub revoked: bool,
    /// `(timestamp of the op, extra hosts)`.
    pub extensions: Vec<(i64, u32)>,
}

impl IssuedCert {
    pub fn live(&self, now: i64) -> bool {
        !self.revoked && now < self.expiry
    }
}

/// The certificates this phone issued, in the order of the chain. The id of a certificate is the hash of its `CertIssue`
/// op; revocations and extensions name it.
pub fn issued_cert_records(state: &PhoneState) -> Vec<IssuedCert> {
    let mut certs: Vec<IssuedCert> = Vec::new();
    for e in &state.chain {
        let Ok(op) = decode_op(&e.canonical, e.signature64()) else {
            continue;
        };
        match op.body {
            OpBody::CertIssue {
                device_id,
                scope,
                expiry,
                max_new_hosts_per_day,
                epoch_lo,
                epoch_hi,
            } => certs.push(IssuedCert {
                id: e.hash32(),
                device_id,
                scope,
                issued_at: op.timestamp,
                expiry,
                max_new_hosts_per_day,
                epoch_lo,
                epoch_hi,
                revoked: false,
                extensions: Vec::new(),
            }),
            OpBody::CertRevoke { cert_id } => {
                if let Some(c) = certs.iter_mut().find(|c| c.id == cert_id) {
                    c.revoked = true;
                }
            }
            OpBody::CertExtend {
                cert_id,
                extra_hosts,
            } => {
                if let Some(c) = certs.iter_mut().find(|c| c.id == cert_id) {
                    c.extensions.push((op.timestamp, extra_hosts));
                }
            }
            _ => {}
        }
    }
    certs
}

/// A kind 10 request against what this phone issued: the refusals (empty = the phone may sign) and the certificate.
pub fn extend_problems(
    state: &PhoneState,
    req: &CertExtendRequestWire,
    now: i64,
) -> (Vec<String>, Option<IssuedCert>) {
    let Some(cert) = issued_cert_records(state)
        .into_iter()
        .find(|c| c.id == req.cert_id)
    else {
        return (
            vec!["This phone did not issue that certificate (or it was issued by an earlier phone). Only the issuing phone extends it.".to_string()],
            None,
        );
    };
    let mut out = Vec::new();
    if cert.revoked {
        out.push("That certificate was revoked; there is nothing to extend.".to_string());
    }
    if now >= cert.expiry {
        out.push("That certificate has expired; an extension never revives it.".to_string());
    }
    if req.extra_hosts < 1 || req.extra_hosts > CERT_NEW_HOSTS_PER_DAY {
        out.push(format!(
            "An extension adds 1 to {CERT_NEW_HOSTS_PER_DAY} hosts."
        ));
    }
    let last = cert.extensions.iter().map(|x| x.0).max().unwrap_or(0);
    if last != 0 && now.saturating_sub(last) < EXTEND_WINDOW_SECS {
        out.push(format!(
            "This certificate was extended at {}; the next extension is possible after {} (one per 24 hours).",
            iso(last),
            iso(last.saturating_add(EXTEND_WINDOW_SECS))
        ));
    }
    (out, Some(cert))
}

/// One line about an issued certificate: device, scope, term, budget, extensions.
pub fn describe_cert(c: &IssuedCert, aliases: &BTreeMap<String, String>) -> String {
    let who = match alias(aliases, &c.device_id) {
        Some(n) => format!("{} [{}]", safe_text(n), c.device_id),
        None => format!("[{}]", c.device_id),
    };
    let scope = match &c.scope {
        CertScope::Group(g) => format!("group {}", uid_hex(g)),
        CertScope::Hosts(h) => format!("{} listed host{}", h.len(), plural(h.len())),
    };
    format!(
        "{who}: {scope}, until {}, {} new hosts per day{}{}",
        iso(c.expiry),
        c.max_new_hosts_per_day,
        if c.extensions.is_empty() {
            String::new()
        } else {
            format!(", extended {}x", c.extensions.len())
        },
        if c.revoked { ", REVOKED" } else { "" }
    )
}

/// The trust screen's list: what this phone issued, with status and whether Revoke is offered.
pub fn issued_certs(
    state: &PhoneState,
    aliases: &BTreeMap<String, String>,
    now: i64,
) -> Vec<IssuedCertView> {
    issued_cert_records(state)
        .iter()
        .map(|c| {
            let (group, hosts) = match &c.scope {
                CertScope::Group(g) => (Some(uid_hex(g)), Vec::new()),
                CertScope::Hosts(h) => (None, h.iter().map(uid_hex).collect()),
            };
            let term_secs = c.expiry.saturating_sub(c.issued_at);
            IssuedCertView {
                cert_id: hex::encode(c.id),
                summary: describe_cert(c, aliases),
                status: if c.revoked {
                    CertStatus::Revoked
                } else if now < c.expiry {
                    CertStatus::Active
                } else {
                    CertStatus::Expired
                },
                can_revoke: c.live(now),
                device_id: c.device_id.clone(),
                device_name: alias(aliases, &c.device_id).map(safe_text),
                group,
                hosts,
                issued_at: c.issued_at,
                expiry: c.expiry,
                term_secs,
                term_text: describe_term(term_secs),
                max_new_hosts_per_day: c.max_new_hosts_per_day,
                epoch_lo: c.epoch_lo,
                epoch_hi: c.epoch_hi,
                revoked: c.revoked,
                extensions: c
                    .extensions
                    .iter()
                    .map(|(at, extra)| CertExtensionView {
                        at: *at,
                        extra_hosts: *extra,
                    })
                    .collect(),
            }
        })
        .collect()
}

fn refused(reason: String) -> Opened {
    Opened::Refused {
        view: RefusalView {
            kind: crate::views::RefusalKind::Other,
            compared: Vec::new(),
            reason,
            checks: Vec::new(),
        },
    }
}

const REQUEST_NOTE: &str = "The certificate takes effect only after a second terminal and a third factor approve it. This signature is the phone's part: it authors the certificate.";
const UNKNOWN_DELEGATE_NOTE: &str = "This phone has not met that device: compare its id with the one on the terminal before you approve, then type it below.";

/// Kind 9: a terminal asks this phone to issue a trust session. With problems the view carries them and nothing can be
/// approved (`None`).
pub fn open_cert_request(frame: &[u8], ctx: &Ctx, token: u64) -> (Opened, Option<PendingCert>) {
    let req = match CertRequestWire::decode(frame) {
        Ok(r) => r,
        Err(e) => return (refused(format!("Not a trust-session request: {e:#}")), None),
    };
    let problems = cert_request_problems(&req, ctx.now, ctx.id.device_id(), ctx.state.epoch);
    let (headline, rows, delegate_known) = describe_cert_request(&req, ctx.now, &ctx.aliases());
    let clean = problems.is_empty();
    let note = if !clean {
        String::new()
    } else if delegate_known {
        REQUEST_NOTE.to_string()
    } else {
        format!("{UNKNOWN_DELEGATE_NOTE} {REQUEST_NOTE}")
    };
    // A delegate this phone has never met cannot be authenticated here: approving needs its device id typed in.
    let type_to_approve = (clean && !delegate_known).then(|| hex::encode(req.device_id));
    let view = CertRequestView {
        token,
        plain: crate::plain::plain_cert_request(&req, &ctx.aliases()),
        headline,
        rows,
        delegate_known,
        problems,
        note,
        type_to_approve,
    };
    (
        Opened::CertRequest { view },
        clean.then_some(PendingCert::Request(req)),
    )
}

/// Kind 10: a terminal asks for more new hosts on a certificate this phone issued.
pub fn open_cert_extend(frame: &[u8], ctx: &Ctx, token: u64) -> (Opened, Option<PendingCert>) {
    let req = match CertExtendRequestWire::decode(frame) {
        Ok(r) => r,
        Err(e) => {
            return (
                refused(format!("Not a budget extension request: {e:#}")),
                None,
            )
        }
    };
    let (problems, cert) = extend_problems(&*ctx.state, &req, ctx.now);
    let aliases = ctx.aliases();
    let mut rows = vec![Row {
        label: "Certificate".into(),
        value: hex::encode(req.cert_id),
        danger: true,
    }];
    if let Some(c) = &cert {
        rows.push(Row {
            label: "Trusted device".into(),
            value: describe_cert(c, &aliases),
            danger: false,
        });
        rows.push(Row {
            label: "Remaining term".into(),
            value: if c.expiry > ctx.now {
                describe_term(c.expiry.saturating_sub(ctx.now))
            } else {
                "expired".into()
            },
            danger: false,
        });
        rows.push(Row {
            label: "Daily budget now".into(),
            value: c.max_new_hosts_per_day.to_string(),
            danger: false,
        });
    }
    rows.push(Row {
        label: "Extra new hosts for today".into(),
        value: req.extra_hosts.to_string(),
        danger: true,
    });
    let clean = problems.is_empty();
    let view = CertExtendView {
        token,
        plain: crate::plain::plain_cert_extend(
            &req,
            cert.as_ref().map(|c| c.device_id.as_str()),
            &aliases,
        ),
        headline: format!(
            "Allow {} more new host{} for the rolling day on a trust session",
            req.extra_hosts,
            if req.extra_hosts == 1 { "" } else { "s" }
        ),
        rows,
        problems,
        note: if clean {
            "It adds to the daily budget for the next 24 hours only. It never changes the scope or the term.".to_string()
        } else {
            String::new()
        },
    };
    (
        Opened::CertExtend { view },
        clean.then_some(PendingCert::Extend(req)),
    )
}

/// Signs what the person approved, re-checking every limit at the moment of signing (the clock moved, the chain may
/// have). A delegate this phone never met needs its device id typed in (`typed_id`).
pub fn approve_cert(
    p: PendingCert,
    typed_id: Option<&str>,
    ctx: &mut Ctx,
) -> Result<crate::views::Outgoing> {
    match p {
        PendingCert::Request(req) => {
            let dev = hex::encode(req.device_id);
            if alias(&ctx.aliases(), &dev).is_none()
                && typed_id.map(|t| t.trim().to_lowercase()).as_deref() != Some(dev.as_str())
            {
                return Err(CoreError::bad(format!("Type the device id of the trusted device ({dev}) to approve: this phone has not met it.")));
            }
            issue_cert(ctx.state, ctx.id, &req, ctx.now)?;
            ops_out(
                &chain::courier_ops(ctx.state, COURIER_TAIL),
                "Carry this to the terminal: it holds the certificate this phone signed. The ring activates it after the second terminal and the third factor approve it.",
            )
        }
        PendingCert::Extend(req) => {
            extend_cert(ctx.state, ctx.id, &req, ctx.now)?;
            ops_out(
                &chain::courier_ops(ctx.state, COURIER_TAIL),
                "Carry this to the terminal that asked.",
            )
        }
    }
}

fn issue_cert(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    req: &CertRequestWire,
    now: i64,
) -> Result<()> {
    let problems = cert_request_problems(req, now, id.device_id(), state.epoch);
    if !problems.is_empty() {
        return Err(CoreError::refused(problems.join(" ")));
    }
    chain::cert_issue(
        state,
        id,
        hex::encode(req.device_id),
        req.scope.clone(),
        req.expiry,
        req.max_new_hosts_per_day,
        req.epoch_lo,
        req.epoch_hi,
        now,
        None,
    )?;
    Ok(())
}

fn extend_cert(
    state: &mut PhoneState,
    id: &PhoneIdentity,
    req: &CertExtendRequestWire,
    now: i64,
) -> Result<()> {
    let (problems, _) = extend_problems(state, req, now);
    if !problems.is_empty() {
        return Err(CoreError::refused(problems.join(" ")));
    }
    chain::cert_extend(state, id, req.cert_id, req.extra_hosts, now, None)?;
    Ok(())
}

/// Revokes a certificate this phone issued (the id is the hex of its `CertIssue` op hash). The list to carry to any
/// terminal comes back.
pub fn revoke_cert(cert_id_hex: &str, ctx: &mut Ctx) -> Result<crate::views::Outgoing> {
    let id: [u8; 32] = hex::decode(cert_id_hex.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| CoreError::bad("A certificate id is 64 hex characters."))?;
    let Some(c) = issued_cert_records(&*ctx.state)
        .into_iter()
        .find(|c| c.id == id)
    else {
        return Err(CoreError::refused(
            "This phone did not issue that certificate.",
        ));
    };
    if c.revoked {
        return Err(CoreError::refused("That certificate is already revoked."));
    }
    chain::cert_revoke(ctx.state, ctx.id, id, ctx.now, None)?;
    ops_out(
        &chain::courier_ops(ctx.state, COURIER_TAIL),
        "Carry this to any terminal: peers stop honouring the certificate on their next sync.",
    )
}
