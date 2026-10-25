//! More admin flows of the Sync tab: the master-phrase exemption (contract 4e),
//! the bulk credential export (C9) and the printable paper-phrase sheet. Same
//! shape as `sync_flows`: modal chains ending in a job on the sync worker that
//! calls the helpers the `sshub sync-*` verbs use.

use std::path::PathBuf;

use super::sync::{CredModeView, DeviceRoleView};
use super::sync_flows::{now, report_job};
use super::sync_ui::MenuFn;
use super::*;
use crate::cli::sync_admin as admin;
use crate::sync::oplog::{HostUid, OpBody};
use crate::sync::runtime::JobEnv;
use crate::sync::{ceremony, exemption, export};

const EXPORT_DEFAULT_SECS: u32 = 600;
const EXPORT_MAX_SECS: u32 = 3600;

/// A path for a file that must not exist yet: empty means `default_name` in the
/// runtime directory (memory-backed, owner-only, gone at logout; the home directory
/// without one), `~/` expands to the home directory, an existing file or a missing
/// folder is refused. These files hold the credentials in clear or the paper phrase.
fn new_file_path(raw: &str, default_name: &str) -> std::result::Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let raw = raw.trim();
    let path = match (raw, raw.strip_prefix("~/"), &home) {
        ("", _, _) => super::kit::default_dir().join(default_name),
        (_, Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(raw),
    };
    if path.exists() {
        return Err(format!(
            "{} already exists: choose another path",
            path.display()
        ));
    }
    if path
        .parent()
        .is_some_and(|d| !d.as_os_str().is_empty() && !d.is_dir())
    {
        return Err(format!("the folder of {} does not exist", path.display()));
    }
    Ok(path)
}

// --------------------------------------------------------------- exemption

/// Why this machine cannot take the exemption, if it cannot.
fn exemption_refusal(app: &App) -> Option<&'static str> {
    if app.sync.cred_mode != Some(CredModeView::Quorum) {
        return Some("the exemption exists only in a Quorum ring (switch the mode first)");
    }
    match app.sync.devices.iter().find(|d| d.this) {
        Some(d) if d.role == DeviceRoleView::Holder && !d.unattended => None,
        Some(_) => Some("only an attended holder terminal can be exempt"),
        None => Some("this device is not in the ring yet"),
    }
}

pub(crate) fn exemption_on(app: &mut App) {
    if let Some(why) = exemption_refusal(app) {
        app.host_notice = Some(why.into());
        return;
    }
    let lines = vec![
        "With the exemption this machine ALONE plus the master phrase opens EVERY".to_string(),
        "credential of the ring. No second device is asked, no phone, no holder.".into(),
        "Whoever has this machine and the six words has all of your credentials.".into(),
        String::new(),
        "Safer ways to open credentials without a second device on hand:".into(),
        "  - a trust session (a certificate that lets a holder release shares),".into(),
        "  - the Vault mode (master phrase and ring secret, no quorum).".into(),
        String::new(),
        "Enabling is a Red change authored by the exempt machine itself: it runs".into(),
        "here, on THIS machine, and cannot be done for another one. A second".into(),
        "attended device and a third factor must approve it. Esc cancels.".into(),
    ];
    app.sync_prompt(
        "Master-phrase exemption: enable on this machine",
        "Type the six words of the master phrase (nothing is shown)",
        true,
        |app, phrase| {
            let phrase = zeroize::Zeroizing::new(phrase.trim().to_string());
            if crate::sync::crypto::eff_decode(&phrase).is_err() {
                app.host_notice =
                    Some("that is not a master phrase (six words): nothing proposed".into());
                return;
            }
            report_job(app, move |env| {
                let body = exemption::enable_body(&env.identity, &phrase, env.argon);
                let op = env.store.sync_append_local(&env.identity, body)?;
                admin::op_report(
                    &env.store,
                    &op,
                    format!(
                        "master-phrase exemption of this machine proposed (op {})",
                        admin::short(&op.hash())
                    ),
                    now(),
                )
            });
        },
    );
    // The panel text rides above the field of the same modal.
    if let Some(super::sync_ui::SyncUi::Prompt { lines: l, .. }) = app.sync_ui.as_mut() {
        *l = lines;
    }
}

pub(crate) fn exemption_off(app: &mut App) {
    let items: Vec<(String, MenuFn)> = app
        .sync
        .devices
        .iter()
        .filter(|d| d.exempt)
        .map(|d| {
            let id = d.id.clone();
            let label = format!("{}{}", d.name, if d.this { " (this device)" } else { "" });
            let run = move |app: &mut App| {
                report_job(app, move |env| {
                    admin::exemption_off(&env.store, &env.identity, &id, now())
                })
            };
            (label, Box::new(run) as MenuFn)
        })
        .collect();
    if items.is_empty() {
        app.host_notice = Some("no device has the master-phrase exemption".into());
        return;
    }
    app.sync_menu("Switch the master-phrase exemption off for", items);
}

// ------------------------------------------------------------ bulk export

pub(crate) fn export_start(app: &mut App) {
    app.sync_prompt(
        "Export every credential",
        format!("Window in seconds, empty for {EXPORT_DEFAULT_SECS} (at most {EXPORT_MAX_SECS})"),
        false,
        |app, raw| {
            let raw = raw.trim();
            let window = if raw.is_empty() {
                Some(EXPORT_DEFAULT_SECS)
            } else {
                raw.parse::<u32>()
                    .ok()
                    .filter(|w| (1..=EXPORT_MAX_SECS).contains(w))
            };
            let Some(window) = window else {
                app.host_notice = Some(format!(
                    "the window is 1 to {EXPORT_MAX_SECS} seconds: nothing proposed"
                ));
                return;
            };
            export_path(app, move |app, out| {
                report_job(app, move |env| {
                    let me = env.identity.public().device_id();
                    let op = env.store.sync_append_local(
                        &env.identity,
                        OpBody::BulkExport {
                            requester: me,
                            window_secs: window,
                        },
                    )?;
                    export_step(env, &op, &out, true)
                });
            });
        },
    );
}

pub(crate) fn export_continue(app: &mut App) {
    app.sync_prompt(
        "Export: continue",
        "The op id of the export (the 8 hex digits shown when it was proposed)",
        false,
        |app, id| {
            let id = id.trim().to_string();
            export_path(app, move |app, out| {
                report_job(app, move |env| {
                    let op = admin::find_op(&env.store, &id)?;
                    let mine = env.identity.public().device_id();
                    anyhow::ensure!(
                        matches!(&op.body, OpBody::BulkExport { requester, .. } if *requester == mine),
                        "that op is not a bulk export proposed by this device"
                    );
                    export_step(env, &op, &out, false)
                });
            });
        },
    );
}

fn export_path(app: &mut App, next: impl FnOnce(&mut App, PathBuf) + 'static) {
    app.sync_prompt(
        "Export every credential",
        "File for the export, empty for sshub-export.csv in the runtime directory (must not exist; written owner-only)",
        false,
        move |app, raw| match new_file_path(&raw, "sshub-export.csv") {
            Ok(out) => next(app, out),
            Err(why) => app.host_notice = Some(format!("{why}: nothing exported")),
        },
    );
}

/// Where the ceremony of the export stands; once it is complete (and the
/// window therefore open) write every credential that opens without a modal.
fn export_step(
    env: &JobEnv,
    op: &crate::sync::oplog::Op,
    out: &std::path::Path,
    proposed: bool,
) -> Result<admin::Report> {
    let at = now();
    ceremony::settle_red_ops(&env.store, at)?;
    let st = ceremony::red_status(&env.store, op, at)?;
    let id = admin::short(&op.hash());
    if let Some(why) = &st.blocked {
        anyhow::bail!("the export is blocked: {why}");
    }
    if st.expired {
        anyhow::bail!("the export ceremony expired: propose it again");
    }
    if !st.complete {
        let head = if proposed {
            format!("bulk export proposed (op {id})")
        } else {
            format!("bulk export (op {id}) is not complete yet")
        };
        let mut r = admin::op_report(&env.store, op, head, at)?;
        r.lines.push(
            "Approve it from the other devices, then choose \"Export: continue\" and give the op id."
                .into(),
        );
        r.lines
            .push("Nothing was exported; the window opens when the ceremony completes.".into());
        return Ok(r);
    }
    let hosts = env.store.list_hosts()?;
    let mut locked: Vec<String> = Vec::new();
    let mut unlock = |uid: &HostUid| -> Result<Option<String>> {
        let Some(host) = hosts
            .iter()
            .find(|h| env.store.host_uid(h.id).ok().flatten().as_ref() == Some(uid))
        else {
            return Ok(None);
        };
        let key = crate::credentials::host_key(host.id);
        if let Some(open) = env.gate.get(&key) {
            return Ok((!open.is_empty()).then(|| open.to_string()));
        }
        // A ring credential that is not open needs shares from other devices:
        // that is a modal of its own, not something the export waits for.
        if env.store.sync_cred_ciphertext(uid)?.is_some() {
            locked.push(host.name.clone());
            return Ok(None);
        }
        (env.open_secrets)(&env.paths).get(&key)
    };
    let me = env.identity.public().device_id();
    let r = export::export_all(&env.store, &me, &now, &mut unlock, out)?;
    // The file holds every credential in clear: the quit reminder names it.
    super::kit::note_written(out);
    let mut lines = vec![format!(
        "exported {} credentials ({} skipped) to {}",
        r.written,
        r.skipped,
        out.display()
    )];
    if !locked.is_empty() {
        lines.push(format!(
            "locked, not exported: {}. Open them first (connect to each host), then export again to a new file.",
            locked.join(", ")
        ));
    }
    Ok(admin::Report {
        lines,
        json: serde_json::Value::Null,
    })
}

// ------------------------------------------------------------- paper sheet

pub(crate) fn paper_sheet(app: &mut App) {
    if app.sync_paper.is_none() {
        app.host_notice = Some("create a paper phrase first".into());
        return;
    }
    app.sync_prompt(
        "Paper phrase: printable sheet",
        "File for the sheet, empty for sshub-paper-phrase.html in the runtime directory (must not exist)",
        false,
        |app, raw| {
            let path = match new_file_path(&raw, "sshub-paper-phrase.html") {
                Ok(p) => p,
                Err(why) => {
                    app.host_notice = Some(format!("{why}: no sheet written"));
                    return;
                }
            };
            let Some(words) = app.sync_paper.as_ref() else {
                app.host_notice = Some("create a paper phrase first".into());
                return;
            };
            let html = zeroize::Zeroizing::new(super::kit::paper_sheet(words));
            app.host_notice = Some(match super::kit::write_sheet(&path, &html) {
                Ok(()) => format!(
                    "sheet written to {}: print it, then delete the file and keep only the paper",
                    path.display()
                ),
                Err(e) => format!("{}: {e}", path.display()),
            });
        },
    );
}
