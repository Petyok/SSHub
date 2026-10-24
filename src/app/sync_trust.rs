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

/// The TTL step of the `t` flow is done: ask for the budget, then show the request.
pub(crate) fn begin(app: &mut App, scope_index: usize, term_days: u32) {
    let (Some(scope), Some(label)) = (
        app.sync.trust_scope_values.get(scope_index).cloned(),
        app.sync.trust_scopes.get(scope_index).cloned(),
    ) else {
        app.host_notice = Some("that scope is gone: open the trust session again".into());
        return;
    };
    let _ = crate::sync::quorum::meta_set(&app.store, LAST_SCOPE_KEY, label.as_bytes());
    app.sync_prompt(
        "Trust session: budget",
        format!(
            "How many NEW hosts of {label} may it open per day without asking? (1 to {CERT_NEW_HOSTS_PER_DAY}, Enter for 10)"
        ),
        false,
        move |app, text| {
            let budget = match text.trim() {
                "" => 10,
                t => match t.parse::<u32>() {
                    Ok(n) if (1..=CERT_NEW_HOSTS_PER_DAY).contains(&n) => n,
                    _ => {
                        app.host_notice =
                            Some(format!("the budget is a number from 1 to {CERT_NEW_HOSTS_PER_DAY}"));
                        return;
                    }
                },
            };
            request(app, scope, term_days, budget);
        },
    );
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
                format!("Show this code to the phone: it asks you to approve a trust session of {days} days."),
                "Paste the answer the phone shows (the signed certificate) below.".into(),
            ],
        ),
        Err(e) => app.host_notice = Some(format!("{e:#}")),
    }
}

/// The request as a looping QR over a field for the phone's answer.
fn show_qr(app: &mut App, title: &str, request: Vec<u8>, lines: Vec<String>) {
    let frames = crate::sync::fountain::frame_bytes_for_area(QR_AREA.0, QR_AREA.1)
        .ok_or_else(|| anyhow::anyhow!("no QR fits this terminal"))
        .and_then(|b| crate::sync::fountain::fountain_encode(&request, b, QR_PARITY_PERCENT));
    match frames {
        Ok(frames) => app.sync_prompt_qr(
            title.to_string(),
            lines,
            frames,
            "Phone's answer (base64)",
            ingest,
        ),
        Err(e) => app.host_notice = Some(format!("{e:#}")),
    }
}

/// The phone's answer: chain its ops, run the ceremony check, say what is next.
fn ingest(app: &mut App, text: String) {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let ops = B64
        .decode(compact)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| PhoneOpsWire::decode(&bytes));
    let ops = match ops {
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
        let mut lines = vec![format!("{} operation(s) taken from the phone", ops.len())];
        for (canonical, sig) in &ops {
            let op = Op::from_canonical(canonical, *sig)?;
            if crate::sync::ceremony::is_gated(&env.store, &op)? {
                let st = crate::sync::ceremony::red_status(&env.store, &op, at)?;
                if st.complete {
                    lines.push("the ceremony is complete: the change took effect".into());
                } else {
                    for m in &st.missing {
                        lines.push(format!("  missing: {}", super::sync_view::proof_label(m)));
                    }
                    lines.push(
                        "next: another attended holder approves it in Review (a), and the paper phrase (p) or the emailed code (e) is typed there".into(),
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
