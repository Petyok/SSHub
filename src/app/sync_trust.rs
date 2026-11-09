//! Trust sessions (contract 4c-2): this terminal asks the phone to certify
//! that it may open a group or a list of hosts for a while without asking
//! each time. The request is a looping QR for the phone; the phone's answer
//! (its signed `CertIssue`, a kind-5 list) is pasted back, chained into the
//! log, and the Red ceremony then runs: a second attended holder approves and
//! a third factor is typed, both in Review. Extending a budget and revoking a
//! certificate are the same shape.

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use super::sync_ui::MenuFn;
use super::*;
use crate::sync::cert::{cert_extend_request, cert_request, cert_revoke_body, DelegationCert};
use crate::sync::limits::{CERT_NEW_HOSTS_PER_DAY, QR_PARITY_PERCENT};
use crate::sync::oplog::{CertScope, Op};
use crate::sync::wire::PhoneOpsWire;

/// Meta key remembering the label of the scope used last (the next default).
const LAST_SCOPE_KEY: &str = "trust_last_scope";
/// The terminal area the request QR is sized for (columns, rows).
const QR_AREA: (u16, u16) = (76, 44);

fn now() -> i64 {
    crate::store::now_ts()
}

/// What a trust session may be scoped to: each group that is not the
/// Favorites list, and each saved host one by one. Never "everything".
pub(crate) fn scope_choices(store: &crate::store::LauncherStore) -> (Vec<String>, Vec<CertScope>) {
    let mut labels = Vec::new();
    let mut scopes = Vec::new();
    let hosts = store.list_hosts().unwrap_or_default();
    for g in store.list_groups().unwrap_or_default() {
        let Some(uid) = store.group_uid(g.id).ok().flatten() else {
            continue;
        };
        if uid == crate::store::FAVORITES_GROUP_UID {
            continue;
        }
        let n = hosts
            .iter()
            .filter(|h| h.groups.iter().any(|x| x.id == g.id))
            .count();
        labels.push(format!("group {} ({n} hosts)", g.name));
        scopes.push(CertScope::Group(uid));
    }
    for h in &hosts {
        if let Some(uid) = store.host_uid(h.id).ok().flatten() {
            labels.push(format!("host {}", h.name));
            scopes.push(CertScope::Hosts(vec![uid]));
        }
    }
    (labels, scopes)
}

/// The index to highlight first: the scope used last, else the first.
pub(crate) fn default_scope(store: &crate::store::LauncherStore, labels: &[String]) -> usize {
    crate::sync::quorum::meta_get(store, LAST_SCOPE_KEY)
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|last| labels.iter().position(|l| *l == last))
        .unwrap_or(0)
}

/// Space on a host row marks it for the list scope (at most
/// `MAX_CERT_REQUEST_HOSTS`); on a group row it says why nothing happens.
pub(crate) fn toggle_mark(app: &mut App) {
    let sync = &mut app.sync;
    match sync.trust_scope_values.get(sync.trust_scope_selected) {
        Some(CertScope::Hosts(h)) if h.len() == 1 => {
            let uid = h[0];
            if let Some(i) = sync.trust_marked.iter().position(|m| *m == uid) {
                sync.trust_marked.remove(i);
            } else if sync.trust_marked.len() >= crate::sync::wire::MAX_CERT_REQUEST_HOSTS {
                app.host_notice = Some(format!(
                    "a trust session lists at most {} hosts",
                    crate::sync::wire::MAX_CERT_REQUEST_HOSTS
                ));
            } else {
                sync.trust_marked.push(uid);
            }
        }
        _ => {
            app.host_notice = Some("Space marks single hosts; a group is chosen with Enter".into())
        }
    }
}

/// The scope the flow asks for: the marked hosts if any, else the highlighted row.
pub(crate) fn chosen(sync: &super::sync::SyncView) -> Option<(CertScope, String)> {
    if !sync.trust_marked.is_empty() {
        let mut list = sync.trust_marked.clone();
        list.sort_unstable();
        let label = format!("{} selected hosts", list.len());
        return Some((CertScope::Hosts(list), label));
    }
    Some((
        sync.trust_scope_values
            .get(sync.trust_scope_selected)?
            .clone(),
        sync.trust_scopes.get(sync.trust_scope_selected)?.clone(),
    ))
}

/// The confirm screen was accepted (scope, days and new hosts a day, all on one screen): show the request.
pub(crate) fn begin(app: &mut App, term_days: u32, budget: u32) {
    let Some((scope, label)) = chosen(&app.sync) else {
        app.host_notice = Some("that scope is gone: open the trust session again".into());
        return;
    };
    if app.sync.trust_marked.is_empty() {
        let _ = crate::sync::quorum::meta_set(&app.store, LAST_SCOPE_KEY, label.as_bytes());
    }
    let budget = budget.clamp(1, CERT_NEW_HOSTS_PER_DAY);
    request(app, scope, term_days, budget);
}

fn request(app: &mut App, scope: CertScope, days: u32, budget: u32) {
    let Some(me) = app.sync.this_device.clone() else {
        app.host_notice =
            Some("this device's identity is not loaded yet: try again in a moment".into());
        return;
    };
    match cert_request(&app.store, &me, scope, i64::from(days) * 86_400, budget, 1, now()) {
        Ok(wire) => show_qr(
            app,
            "Trust session",
            wire.encode(),
            vec![
                format!("Show this code to the phone: it asks you to approve a trust session of {days} days, up to {budget} new hosts a day."),
                "The phone then shows its answer as a code: hold it to the camera, or paste its text below.".into(),
            ],
        ),
        Err(e) => app.host_notice = Some(format!("{e:#}")),
    }
}

/// `bytes` as the looping QR frames that fit the QR area.
pub(crate) fn qr_frames(bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let b = crate::sync::fountain::frame_bytes_for_area(QR_AREA.0, QR_AREA.1)
        .ok_or_else(|| anyhow::anyhow!("no QR fits this terminal"))?;
    crate::sync::fountain::fountain_encode(bytes, b, QR_PARITY_PERCENT)
}

/// The request as a looping QR over a field for the phone's answer. A camera (or Bluetooth) of this build
/// reads the answer by itself while the prompt is open.
fn show_qr(app: &mut App, title: &str, request: Vec<u8>, lines: Vec<String>) {
    match qr_frames(&request) {
        Ok(frames) => {
            app.sync_prompt_qr(
                title.to_string(),
                lines,
                frames,
                "Phone's answer (base64)",
                ingest,
            );
            listen_for_answer(app);
        }
        Err(e) => app.host_notice = Some(format!("{e:#}")),
    }
}

/// Start the camera (and Bluetooth, when built) for the phone's answer; nothing to do in a build without them.
fn listen_for_answer(app: &mut App) {
    use crate::sync::media::{listen_with, ListenOpts, MediaKind};
    let (tx, rx) = std::sync::mpsc::channel();
    let mut kinds = vec![MediaKind::Camera];
    if cfg!(feature = "bluetooth") {
        kinds.push(MediaKind::Ble);
    }
    let handle = listen_with(&kinds, tx, ListenOpts::PHONE_OPS);
    app.answer_listener = Some((handle, rx));
}

/// Per tick: an answer read by the camera or over Bluetooth is taken in as if it had been pasted; the listener
/// ends with the prompt it belongs to.
pub(crate) fn tick(app: &mut App) {
    let Some((_, rx)) = app.answer_listener.as_ref() else {
        return;
    };
    let answer = rx.try_iter().find_map(|e| match e {
        crate::sync::media::MediaEvent::Reply(_, bytes) => Some(bytes),
        _ => None,
    });
    if let Some(bytes) = answer {
        app.answer_listener = None;
        app.sync_ui = None;
        app.mode = AppMode::Normal;
        ingest_bytes(app, &bytes);
    } else if app.sync_ui.is_none() {
        app.answer_listener = None;
    }
}

/// The phone's answer, pasted as text.
fn ingest(app: &mut App, text: String) {
    app.answer_listener = None;
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    match B64.decode(compact) {
        Ok(bytes) => ingest_bytes(app, &bytes),
        Err(e) => app.host_notice = Some(format!("that is not the phone's answer: {e}")),
    }
}

/// The phone's answer: chain its ops, run the ceremony check, say what is next.
fn ingest_bytes(app: &mut App, bytes: &[u8]) {
    let ops = match PhoneOpsWire::decode(bytes) {
        Ok(w) => w.ops,
        Err(e) => {
            app.host_notice = Some(format!("that is not the phone's answer: {e:#}"));
            return;
        }
    };
    app.run_sync_job(move |env| {
        crate::sync::hub::courier_ingest(&env.store, &ops)?;
        let at = now();
        crate::sync::ceremony::settle_red_ops(&env.store, at)?;
        let mut lines = vec![format!("{} change(s) taken from the phone", ops.len())];
        for (canonical, sig) in &ops {
            let op = Op::from_canonical(canonical, *sig)?;
            if crate::sync::ceremony::is_gated(&env.store, &op)? {
                let st = crate::sync::ceremony::red_status(&env.store, &op, at)?;
                if st.complete {
                    lines.push("the approvals are complete: the change took effect".into());
                } else {
                    for m in &st.missing {
                        lines.push(format!("  still missing: {}", super::sync_view::proof_label(m)));
                    }
                    lines.push(
                        "next: another full device with a person at the keyboard approves it in Review (a), and the paper phrase (p) or the emailed code (e) is typed there".into(),
                    );
                }
            }
        }
        Ok(lines.join("\n"))
    });
}

/// Certificates this ring knows that are still in force, with a label.
fn live_certs(app: &App) -> Vec<(DelegationCert, String)> {
    let at = now();
    crate::sync::quorum::ops_of(&app.store, "CertIssue", None)
        .unwrap_or_default()
        .iter()
        .filter_map(|op| DelegationCert::from_op(op).ok())
        .filter(|c| c.expiry > at && !app.store.sync_cert_revoked(&c.id).unwrap_or(true))
        .map(|c| {
            let who = app
                .sync
                .devices
                .iter()
                .find(|d| d.id == c.device_id)
                .map_or_else(|| c.device_id.chars().take(8).collect(), |d| d.name.clone());
            let days = (c.expiry - at) / 86_400;
            let label = format!(
                "{who}: {} more days, {} new hosts a day",
                days + 1,
                c.max_new_hosts_per_day
            );
            (c, label)
        })
        .collect()
}

fn pick_cert(app: &mut App, title: &str, next: fn(&mut App, [u8; 32])) {
    let items: Vec<(String, MenuFn)> = live_certs(app)
        .into_iter()
        .map(|(c, label)| {
            let id = c.id;
            (
                label,
                Box::new(move |app: &mut App| next(app, id)) as MenuFn,
            )
        })
        .collect();
    if items.is_empty() {
        app.host_notice = Some("no trust session is in force".into());
        return;
    }
    app.sync_menu(title.to_string(), items);
}

/// Ask the phone for more hosts today on an existing certificate.
pub(crate) fn extend(app: &mut App) {
    pick_cert(app, "Extend which trust session?", |app, cert_id| {
        app.sync_prompt(
            "Extend the budget",
            format!("How many more new hosts today? (1 to {CERT_NEW_HOSTS_PER_DAY})"),
            false,
            move |app, text| match text
                .trim()
                .parse::<u32>()
                .map_err(anyhow::Error::from)
                .and_then(|n| cert_extend_request(cert_id, n))
            {
                Ok(wire) => show_qr(
                    app,
                    "Extend a trust session",
                    wire.encode(),
                    vec![
                        "Show this code to the phone: it asks you to confirm the extension.".into(),
                        "Paste the answer the phone shows below.".into(),
                    ],
                ),
                Err(e) => app.host_notice = Some(format!("{e:#}")),
            },
        );
    });
}

/// End a trust session now (a normal Yellow change; peers stop honouring it
/// on their next sync).
pub(crate) fn revoke(app: &mut App) {
    pick_cert(app, "Revoke which trust session?", |app, cert_id| {
        app.run_sync_job(move |env| {
            let op = crate::sync::quorum::ops_of(&env.store, "CertIssue", None)?
                .into_iter()
                .find(|o| o.hash() == cert_id)
                .ok_or_else(|| anyhow::anyhow!("that trust session is gone"))?;
            let cert = DelegationCert::from_op(&op)?;
            env.store
                .sync_append_local(&env.identity, cert_revoke_body(&cert))?;
            Ok("trust session revoked: peers stop honouring it on their next sync".into())
        });
    });
}

// ------------------------------------------------------------------- renewal

/// One trust session of the ring as the renewal flow and the notice see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustRow {
    pub cert_id: [u8; 32],
    pub delegate_id: String,
    pub delegate: String,
    /// The end that counts (the issue's, or the newest counted renewal's).
    pub expiry: i64,
    pub renewals: u32,
    pub original_term_secs: i64,
}

/// Trust sessions in force (issued, not revoked, not past their end) with their effective end and renewals used.
pub(crate) fn trust_rows(
    store: &crate::store::LauncherStore,
    devices: &[(String, String)],
    now: i64,
) -> Vec<TrustRow> {
    let ops = crate::sync::quorum::ops_of(store, "CertIssue", None).unwrap_or_default();
    ops.iter()
        .filter(|o| crate::sync::ceremony::in_force(store, o).unwrap_or(false))
        .filter_map(|o| DelegationCert::from_op(o).ok())
        .filter(|c| !store.sync_cert_revoked(&c.id).unwrap_or(true))
        .filter_map(|c| {
            let expiry = c.effective_expiry(store).ok()?;
            (expiry > now).then(|| TrustRow {
                cert_id: c.id,
                delegate: devices
                    .iter()
                    .find(|(id, _)| *id == c.device_id)
                    .map_or_else(|| c.device_id.chars().take(8).collect(), |(_, n)| n.clone()),
                delegate_id: c.device_id.clone(),
                expiry,
                renewals: c.renewals(store).unwrap_or(u32::MAX),
                original_term_secs: c.expiry - c.issued_at(),
            })
        })
        .collect()
}

/// How close to its end a trust session is offered for renewal by a sticky line.
const RENEW_NOTICE_SECS: i64 = 3 * 86_400;

/// Sticky lines for trust sessions that end soon and can still be renewed (design 6.4): the renewal is lighter than
/// a new session, so say so where the user will see it.
pub(crate) fn renewal_notices(rows: &[TrustRow], now: i64) -> Vec<String> {
    rows.iter()
        .filter(|r| r.expiry - now <= RENEW_NOTICE_SECS && r.renewals < crate::sync::limits::CERT_MAX_RENEWALS)
        .map(|r| {
            format!(
                "The trust session of {} ends in {}: renew it from another full computer (M, Advanced, Renew a trust session).",
                r.delegate,
                crate::sync::health::human_left(r.expiry - now)
            )
        })
        .collect()
}

/// Renew a trust session: the terminal shows the phone a request that carries what the other terminals saw of the
/// delegate; the phone's answer is pasted back like any other.
pub(crate) fn renew(app: &mut App) {
    let devices: Vec<(String, String)> = app
        .sync
        .devices
        .iter()
        .map(|d| (d.id.clone(), d.name.clone()))
        .collect();
    let me = app.sync.this_device.clone();
    let rows: Vec<TrustRow> = trust_rows(&app.store, &devices, now())
        .into_iter()
        .filter(|r| Some(&r.delegate_id) != me.as_ref())
        .filter(|r| r.renewals < crate::sync::limits::CERT_MAX_RENEWALS)
        .collect();
    if rows.is_empty() {
        app.host_notice = Some(
            "no trust session can be renewed from here: it must be in force, renewed fewer than 2 times, and not be this computer's own (renew it from another full computer)".into(),
        );
        return;
    }
    let items: Vec<(String, MenuFn)> = rows
        .into_iter()
        .map(|r| {
            let label = format!(
                "{}: ends in {}, renewed {} of {} times",
                r.delegate,
                crate::sync::health::human_left(r.expiry - now()),
                r.renewals,
                crate::sync::limits::CERT_MAX_RENEWALS
            );
            let (id, max) = (r.cert_id, r.original_term_secs);
            (
                label,
                Box::new(move |app: &mut App| renew_term(app, id, max)) as MenuFn,
            )
        })
        .collect();
    app.sync_menu("Renew which trust session?", items);
}

fn renew_term(app: &mut App, cert_id: [u8; 32], max_secs: i64) {
    let items: Vec<(String, MenuFn)> = [7i64, 30, 90]
        .into_iter()
        .filter(|d| d * 86_400 <= max_secs)
        .map(|d| {
            (
                format!("{d} more days"),
                Box::new(move |app: &mut App| renew_request(app, cert_id, d * 86_400)) as MenuFn,
            )
        })
        .collect();
    if items.is_empty() {
        app.host_notice = Some("a renewal is at most as long as the first term".into());
        return;
    }
    app.sync_menu("Renew for how long?", items);
}

fn renew_request(app: &mut App, cert_id: [u8; 32], term_secs: i64) {
    match crate::sync::cert::cert_renew_request(&app.store, cert_id, term_secs, now()) {
        Ok(wire) => show_qr(
            app,
            "Renew a trust session",
            wire.encode(),
            vec![
                "Show this code to the phone: it shows which hosts were opened and asks you to confirm the renewal.".into(),
                "A renewal needs no second device and no paper phrase. Paste the answer the phone shows below.".into(),
            ],
        ),
        Err(e) => app.host_notice = Some(format!("{e:#}")),
    }
}

#[cfg(test)]
mod renewal_tests {
    use super::*;

    fn row(expiry: i64, renewals: u32) -> TrustRow {
        TrustRow {
            cert_id: [1; 32],
            delegate_id: "d".into(),
            delegate: "laptop".into(),
            expiry,
            renewals,
            original_term_secs: 30 * 86_400,
        }
    }

    #[test]
    fn the_notice_names_a_session_that_ends_soon_and_can_still_be_renewed() {
        let now = 1_000_000;
        let soon = renewal_notices(&[row(now + 2 * 86_400, 0)], now);
        assert_eq!(soon.len(), 1);
        assert!(
            soon[0].contains("laptop") && soon[0].contains("renew"),
            "{soon:?}"
        );
        assert!(renewal_notices(&[row(now + 20 * 86_400, 0)], now).is_empty());
        assert!(renewal_notices(
            &[row(now + 86_400, crate::sync::limits::CERT_MAX_RENEWALS)],
            now
        )
        .is_empty());
    }
}
