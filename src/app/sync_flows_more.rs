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
        return Some("the master phrase exemption exists only in a Quorum ring (switch the passwords to Quorum first)");
    }
    match app.sync.devices.iter().find(|d| d.this) {
        Some(d) if d.role == DeviceRoleView::Holder && !d.unattended => None,
        Some(_) => Some("only a full computer with a person at the keyboard can have the master phrase exemption"),
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
        "password of the ring. No second device is asked, no phone, no other full device.".into(),
        "Whoever has this machine and the six words has all of your credentials.".into(),
        String::new(),
        "Safer ways to open credentials without a second device on hand:".into(),
        "  - a trust session (the phone lets a computer open some hosts without asking),".into(),
        "  - the Vault mode (master phrase and ring secret, no quorum).".into(),
        String::new(),
        "Enabling is a big change made by the exempt computer itself: it runs".into(),
        "here, on THIS machine, and cannot be done for another one. A second".into(),
        "device with a person at the keyboard and the paper phrase or emailed code must approve it. Esc cancels.".into(),
    ];
    app.sync_prompt(
        "Master phrase exemption: turn on for this computer",
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
                        "master phrase exemption of this computer proposed (change {})",
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
        app.host_notice = Some("no device has the master phrase exemption".into());
        return;
    }
    app.sync_menu("Turn the master phrase exemption off for", items);
}

// ------------------------------------------------------------ bulk export

/// This device's export that was proposed and has not expired: the one a second visit continues.
pub(crate) fn open_export(app: &App) -> Option<crate::sync::oplog::Op> {
    let me = app.sync.this_device.clone()?;
    crate::sync::quorum::ops_of(&app.store, "BulkExport", Some(&me))
        .ok()?
        .into_iter()
        .rev()
        .find(|o| {
            ceremony::red_status(&app.store, o, now())
                .is_ok_and(|st| !st.expired && st.blocked.is_none())
                && admin::export_pending(&app.store, o, now())
        })
}

pub(crate) fn export_start(app: &mut App) {
    // An export that was proposed earlier is continued here: no op id to carry back.
    if let Some(op) = open_export(app) {
        return export_path(app, move |app, out| {
            report_job(app, move |env| export_step(env, &op, &out, false));
        });
    }
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
        anyhow::bail!("the export ran out of time before it was approved: propose it again");
    }
    if !st.complete {
        let head = if proposed {
            format!("export of every saved password proposed (change {id})")
        } else {
            format!("export (change {id}) is not approved yet")
        };
        let mut r = admin::op_report(&env.store, op, head, at)?;
        r.lines.push(
            "Approve it from the other devices, then choose \"Export every credential\" again: it continues this export."
                .into(),
        );
        r.lines
            .push("Nothing was exported; the window opens when all approvals are in.".into());
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

// -------------------------------------------------------------------- mail

/// Is a recovery of the phone mailed from this profile? (`[sync.notify]` has a command and a recipient.)
pub(crate) fn mail_configured(cfg: &crate::config::AppConfig) -> bool {
    cfg.sync.notify.command.is_some() && !cfg.sync.notify.to.is_empty()
}

fn config_path(app: &App) -> Option<PathBuf> {
    app.profile.as_ref().map(|p| p.config_file.clone())
}

/// After a phone is enrolled: offer the mail step unless mail is already set up or "Not now" was chosen before.
pub(crate) fn mail_offer_after_enroll(app: &mut App) {
    let Some(cfg) = config_path(app).and_then(|p| crate::config::load_config_at(&p).ok()) else {
        return;
    };
    if !mail_configured(&cfg) && !cfg.sync.notify_declined {
        mail_offer(app);
    }
}

pub(crate) fn mail_offer(app: &mut App) {
    app.sync_menu(
        "Be told when someone tries to replace this phone",
        vec![
            (
                "Set up mail now (the local sendmail, a test message goes out)".to_string(),
                Box::new(mail_setup) as MenuFn,
            ),
            (
                "Not now (a hostile recovery would then only show on terminal screens)".to_string(),
                Box::new(mail_not_now),
            ),
        ],
    );
}

fn mail_not_now(app: &mut App) {
    let Some(path) = config_path(app) else {
        return;
    };
    app.host_notice = Some(match crate::config::decline_notify_at(&path) {
        Ok(()) => "not now: the Sync tab keeps a line about it; M, Phone, Mail sets it up later".into(),
        Err(e) => format!("could not record that: {e:#}"),
    });
}

/// The `[sync.notify]` block shown where the setting lives on another machine.
fn notify_block() -> Vec<String> {
    vec![
        "[sync.notify]".into(),
        format!("command = \"{}\"", crate::config::DEFAULT_MAIL_COMMAND),
        "to = [\"you@example.com\"]".into(),
        "from = \"sshub@example.com\"".into(),
    ]
}

/// The mail step: To, From, the command, then the settings are written and a test message goes out.
pub(crate) fn mail_setup(app: &mut App) {
    let hub = app
        .sync
        .devices
        .iter()
        .find(|d| d.unattended && !d.phone && !d.this)
        .map(|d| d.name.clone());
    let Some(hub) = hub else {
        return mail_prompts(app);
    };
    let mut lines = vec![
        format!("This ring has a Hub ({hub}). A Hub mails you about a recovery it receives, and its mail setting lives in the sshub config file ON THE HUB, not here. Put this block there:"),
        String::new(),
    ];
    lines.extend(notify_block());
    lines.push(String::new());
    lines.push("Enter continues with the same setting for this computer.".into());
    app.sync_info_with(
        "Mail on the Hub",
        lines,
        Vec::new(),
        Some(Box::new(mail_prompts) as MenuFn),
    );
}

fn mail_prompts(app: &mut App) {
    app.sync_prompt(
        "Mail: who is told",
        "Address to send the alert to (several: separate with commas)",
        false,
        |app, to| {
            app.sync_prompt(
                "Mail: sender",
                "Sender address (From)",
                false,
                move |app, from| {
                    let default = crate::config::default_mail_command();
                    app.sync_prompt(
                        "Mail: command",
                        match default {
                            Some(c) => format!("Command that sends mail, empty for {c}"),
                            None => "Command that sends mail (sendmail-compatible, with -t -i); there is no /usr/sbin/sendmail here".into(),
                        },
                        false,
                        move |app, command| mail_write_and_test(app, &to, &from, &command),
                    );
                },
            );
        },
    );
}

fn mail_write_and_test(app: &mut App, to: &str, from: &str, command: &str) {
    let to: Vec<String> = to
        .split(',')
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    let command = match (command.trim(), crate::config::default_mail_command()) {
        ("", Some(d)) => d.to_string(),
        ("", None) => {
            app.host_notice = Some("no mail command given: nothing saved".into());
            return;
        }
        (c, _) => c.to_string(),
    };
    if to.is_empty() || to.iter().any(|a| !a.contains('@')) || !from.contains('@') {
        app.host_notice = Some("both addresses must look like name@host: nothing saved".into());
        return;
    }
    let notify = crate::config::NotifyConfig {
        command: Some(command.clone()),
        to: to.clone(),
        from: Some(from.trim().to_string()),
    };
    app.run_sync_job(move |env| {
        crate::config::write_notify_at(&env.paths.config_file, notify.clone())?;
        let sent = crate::sync::notify::send_notification(
            &notify,
            "SSHub: mail test",
            "This is a test message. If you read it, SSHub can tell you when someone tries to replace your phone.",
        );
        Ok(match sent {
            Ok(()) => format!(
                "Mail settings saved. The test message was handed to `{command}` for {}: check that it arrived. If it did not, the command or the addresses are wrong (M, Phone, Mail changes them).",
                to.join(", ")
            ),
            Err(e) => format!(
                "Mail settings saved, but the test message FAILED: {e:#}. Nothing was delivered: fix the command or the addresses (M, Phone, Mail)."
            ),
        })
    });
}
