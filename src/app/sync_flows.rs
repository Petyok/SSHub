//! The admin flows of the Sync tab (`M`): each is a chain of the generic
//! prompt, menu and panel modals ending in a job on the sync worker. The jobs
//! call the same helpers as the `sshub sync-*` verbs (`cli::sync_admin`), so a
//! change made here and one made there are the same change.

use super::sync::{DeviceStatusView, DeviceView};
use super::sync_ui::MenuFn;
use super::*;
use crate::cli::sync_admin as admin;
use crate::sync::crypto::bip39_encode;
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::{CredMode, DeviceRole};
use crate::sync::runtime::{JobEnv, RING_SECRET_KEY};

pub(super) fn now() -> i64 {
    crate::store::now_ts()
}

/// A job whose result is a report: its lines become the panel or notice.
pub(super) fn report_job(
    app: &mut App,
    f: impl FnOnce(&JobEnv) -> Result<admin::Report> + Send + 'static,
) {
    app.run_sync_job(move |env| Ok(f(env)?.lines.join("\n")));
}

/// A menu of the ring's other devices; `next` gets the chosen one's id and name.
pub(super) fn pick_device(
    app: &mut App,
    title: &str,
    keep: fn(&DeviceView) -> bool,
    next: fn(&mut App, String, String),
) {
    let items: Vec<(String, MenuFn)> = app
        .sync
        .devices
        .iter()
        .filter(|d| !d.this && keep(d))
        .map(|d| {
            let (id, name) = (d.id.clone(), d.name.clone());
            let label = format!("{}  ({}, {})", d.name, role_label(d), d.status.label());
            (
                label,
                Box::new(move |app: &mut App| next(app, id, name)) as MenuFn,
            )
        })
        .collect();
    if items.is_empty() {
        app.host_notice = Some("no other device in the ring fits".into());
        return;
    }
    app.sync_menu(title.to_string(), items);
}

fn role_label(d: &DeviceView) -> &'static str {
    d.role_word()
}

fn read_ring_secret(env: &JobEnv) -> Result<[u8; 16]> {
    let text = (env.open_secrets)(&env.paths)
        .get(RING_SECRET_KEY)?
        .ok_or_else(|| anyhow::anyhow!("this device has no ring secret: import the kit first"))?;
    crate::sync::quorum::parse_ring_secret(&text)
}

// ------------------------------------------------------------------ revoke

pub(crate) fn revoke(app: &mut App) {
    pick_device(
        app,
        "Revoke which device?",
        |d| d.status != DeviceStatusView::Revoked,
        confirm_revoke,
    );
}

fn confirm_revoke(app: &mut App, id: String, name: String) {
    // Say at once, before anything is typed, when the ring cannot do it.
    let holder = app
        .sync
        .devices
        .iter()
        .find(|d| d.id == id)
        .is_some_and(|d| d.role == super::sync::DeviceRoleView::Holder);
    if holder {
        if let Err(why) = crate::sync::revocation::check_revocable(&app.store, &id, now()) {
            return app.sync_info(
                "This device cannot be removed yet",
                vec![format!("{why:#}")],
            );
        }
    }
    let expected = name.clone();
    app.sync_prompt(
        "Revoke a device",
        format!(
            "{name} will be cut off and the keys rotated. A full device needs the three approvals. Type its name to confirm"
        ),
        false,
        move |app, typed| {
            if typed.trim() != expected {
                app.host_notice = Some("not confirmed: nothing changed".into());
                return;
            }
            app.run_sync_job(move |env| {
                // Revocation rotates this device's keys: work on a fresh copy and
                // let the worker reload after the job.
                let secrets = (env.open_secrets)(&env.paths);
                let mut identity = load_or_init_identity(&env.paths, secrets.as_ref())?;
                let r = admin::revoke(
                    &env.paths,
                    &env.store,
                    &mut identity,
                    secrets.as_ref(),
                    &id,
                    now(),
                )?;
                Ok(r.lines.join("\n"))
            });
        },
    );
}

// -------------------------------------------------------------------- role

pub(crate) fn role(app: &mut App) {
    pick_device(
        app,
        "Change the role of which device?",
        |_| true,
        choose_role,
    );
}

fn choose_role(app: &mut App, id: String, name: String) {
    let to = |role: DeviceRole, id: String| -> MenuFn {
        Box::new(move |app: &mut App| {
            report_job(app, move |env| {
                admin::role(&env.store, &env.identity, &id, role, now())
            });
        })
    };
    app.sync_menu(
        format!("Role of {name}"),
        vec![
            (
                "Full device: holds a piece of every password, approves big changes".to_string(),
                to(DeviceRole::Holder, id.clone()),
            ),
            (
                "View-only copy: gets your hosts, holds no piece of any password".to_string(),
                to(DeviceRole::Mirror, id),
            ),
        ],
    );
}

// -------------------------------------------------------------------- mode

/// The two-step switch, as `sshub sync-mode` does it: the first call proposes
/// the Red change (nothing is re-encrypted before the ring agreed), a later one
/// re-wraps the credentials once the ceremony completed.
fn mode_job(app: &mut App, phrase: String, to: CredMode) {
    app.run_sync_job(move |env| {
        // Only the Vault direction opens credentials with the ring secret.
        let ring = if to == CredMode::Quorum {
            read_ring_secret(env)?
        } else {
            [0u8; 16]
        };
        let r = admin::mode(
            &env.store,
            &env.identity,
            to,
            None,
            phrase.trim(),
            &ring,
            now(),
        )?;
        Ok(r.lines.join("\n"))
    });
}

/// This device's own mode switch that is not finished: `(to, false)` while its ceremony is incomplete,
/// `(to, true)` when it is in force but credentials are still wrapped for the old mode.
pub(crate) fn unfinished_switch(app: &App) -> Option<(CredMode, bool)> {
    let me = app.sync.this_device.clone()?;
    let mc = crate::sync::modes::own_mode_change(&app.store, &me).ok()??;
    let crate::sync::oplog::OpBody::ModeChange { to, .. } = &mc.body else {
        return None;
    };
    let to = *to;
    if !crate::sync::ceremony::in_force(&app.store, &mc).ok()? {
        return Some((to, false));
    }
    let public = app
        .store
        .sync_devices()
        .ok()?
        .into_iter()
        .find(|d| d.id == me)?
        .public;
    // The probe fails where a credential would have to be opened: that one is still wrapped the old way.
    let pending =
        crate::sync::modes::mode_migration(&app.store, &public, &|_| anyhow::bail!("pending"))
            .is_err();
    pending.then_some((to, true))
}

/// The persistent row of the Sync tab for a switch or an export that was approved and waits to be finished.
pub(crate) fn finish_row(app: &App) -> Option<String> {
    if let Some((to, true)) = unfinished_switch(app) {
        return Some(format!(
            "Finish: the switch to {} is approved. Press M, Passwords, then Switch to finish it.",
            if to == CredMode::Quorum {
                "Quorum"
            } else {
                "Vault"
            }
        ));
    }
    let op = super::sync_flows_more::open_export(app)?;
    let st = crate::sync::ceremony::red_status(&app.store, &op, now()).ok()?;
    st.complete.then(|| {
        "Finish: the export is approved. Press M, Advanced, Export every saved password.".into()
    })
}

pub(crate) fn mode(app: &mut App) {
    match unfinished_switch(app) {
        Some((to, false)) => return mode_job(app, String::new(), to), // shows the ceremony status
        Some((CredMode::Quorum, true)) => {
            return app.sync_prompt(
                "Finish the switch to Quorum",
                "The ring agreed. Type the master phrase to re-wrap the credentials for Quorum (nothing is shown)",
                true,
                |app, phrase| mode_job(app, phrase, CredMode::Quorum),
            )
        }
        Some((CredMode::Vault, true)) => return finish_vault(app),
        None => {}
    }
    match app.sync.cred_mode {
        Some(super::sync::CredModeView::Vault) => {
            // Proposing needs no secret; the phrase is asked when the switch is finished.
            mode_job(app, String::new(), CredMode::Quorum);
        }
        Some(super::sync::CredModeView::Quorum) => pick_device(
            app,
            "Switch back to Vault: which full device is the second signer?",
            |d| {
                d.role == super::sync::DeviceRoleView::Holder
                    && !d.phone
                    && !d.unattended
                    && d.status != DeviceStatusView::Revoked
            },
            propose_vault,
        ),
        None => app.host_notice = Some("the credential mode is not known yet".into()),
    }
}

/// Quorum to Vault, step one: propose the change with a co-signing full device (a big change: the approvals are
/// collected like any other). Nothing is re-wrapped before the ring agreed.
fn propose_vault(app: &mut App, id: String, _name: String) {
    app.run_sync_job(move |env| {
        let r = admin::mode(
            &env.store,
            &env.identity,
            CredMode::Vault,
            Some(&id),
            "",
            &[0u8; 16],
            now(),
        )?;
        let mut lines = r.lines;
        // What the design asks the screen to say plainly about a downgrade (design: Mode transitions).
        lines.push(
            "Going back to Vault lowers the protection: afterwards the master phrase and the kit open every saved password, so a stolen computer is no longer stopped by needing a second device."
                .into(),
        );
        lines.push(
            "The computer that finishes the switch sees every password once. Passwords already copied while they were Quorum stay openable by whoever held a share then."
                .into(),
        );
        lines.push("The second signer you picked must approve it, with the phone and the paper phrase or emailed code.".into());
        Ok(lines.join("\n"))
    });
}

/// Quorum to Vault, step two, once the approvals are in: every credential is opened with the same unlock
/// dialog a connect uses (a second full device approves on its screen, or the phone answers) and written again
/// for the Vault. One credential after another; nothing changes for a credential that was not opened.
pub(crate) fn finish_vault(app: &mut App) {
    if !app.sync.worker {
        app.host_notice = Some("the sync worker is not running: nothing was changed".into());
        return;
    }
    app.sync.migrating = true;
    app.open_sync_unlock(super::sync::UnlockView {
        host: "all credentials (switching back to Vault)".into(),
        have: 0,
        need: 0,
        paths: Vec::new(),
        frames: Vec::new(),
        host_id: String::new(),
        words: String::new(),
        code: String::new(),
    });
    app.sync.intents.push(super::sync::SyncIntent::Unlock {
        secrets: vec![crate::sync::runtime::MIGRATE_KEY.into()],
    });
}

// ---------------------------------------------------------------- recovery

pub(crate) fn recovery(app: &mut App) {
    app.sync_menu(
        "Recovery of a lost phone",
        vec![
            (
                "Status of recoveries in the log".to_string(),
                Box::new(recovery_status) as MenuFn,
            ),
            (
                "Acknowledge open recoveries (I have seen them)".to_string(),
                Box::new(recovery_ack) as MenuFn,
            ),
            (
                "Approve a recovery".to_string(),
                Box::new(|app: &mut App| recovery_by_id(app, RecoveryStep::Approve)) as MenuFn,
            ),
            (
                "Cancel a recovery".to_string(),
                Box::new(|app: &mut App| recovery_by_id(app, RecoveryStep::Cancel)) as MenuFn,
            ),
            (
                "Start a recovery (the phone is lost)".to_string(),
                Box::new(recovery_start) as MenuFn,
            ),
        ],
    );
}

fn recovery_status(app: &mut App) {
    app.run_sync_job(|env| {
        let ops = admin::all_recoveries(&env.store)?;
        if ops.is_empty() {
            return Ok("no recovery in the log".into());
        }
        let mut lines = Vec::new();
        for op in &ops {
            lines.extend(admin::recovery_lines(&env.store, op, now()));
        }
        Ok(lines.join("\n"))
    });
}

fn hex8(h: &[u8; 32]) -> String {
    h[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn recovery_ack(app: &mut App) {
    app.run_sync_job(|env| {
        let open = crate::sync::recovery::unacknowledged(&env.store, &env.identity, now())?;
        if open.is_empty() {
            return Ok("no open recovery waits for your acknowledgement".into());
        }
        let mut lines = Vec::new();
        for op in &open {
            env.store
                .sync_append_local(&env.identity, crate::sync::recovery::ack_body(op, now())?)?;
            lines.push(format!("acknowledged recovery {}", hex8(&op.hash())));
        }
        Ok(lines.join("\n"))
    });
}

#[derive(Clone, Copy)]
enum RecoveryStep {
    Approve,
    Cancel,
}

/// Pick the recovery from a list (no id to type), then approve or cancel it.
fn recovery_by_id(app: &mut App, step: RecoveryStep) {
    let ops = admin::all_recoveries(&app.store).unwrap_or_default();
    if ops.is_empty() {
        app.host_notice = Some("no recovery was started".into());
        return;
    }
    let items: Vec<(String, MenuFn)> = ops
        .iter()
        .map(|op| {
            let who = app
                .sync
                .devices
                .iter()
                .find(|d| d.id == op.device_id)
                .map_or_else(|| op.device_id.clone(), |d| d.name.clone());
            let age_h = (now() - op.timestamp).max(0) / 3_600;
            let label = format!(
                "Recovery started by {who}, {age_h} h ago ({})",
                hex8(&op.hash())
            );
            let id: String = hex8(&op.hash());
            (
                label,
                Box::new(move |app: &mut App| recovery_chosen(app, step, id)) as MenuFn,
            )
        })
        .collect();
    app.sync_menu(
        match step {
            RecoveryStep::Approve => "Approve which recovery?",
            RecoveryStep::Cancel => "Cancel which recovery?",
        },
        items,
    );
}

fn recovery_chosen(app: &mut App, step: RecoveryStep, prefix: String) {
    match step {
        RecoveryStep::Approve => app.run_sync_job(move |env| {
            let op = admin::find_op(&env.store, prefix.trim())?;
            env.store
                .sync_append_local(&env.identity, crate::sync::recovery::approve_body(&op)?)?;
            Ok(format!("approved recovery {}", hex8(&op.hash())))
        }),
        RecoveryStep::Cancel => {
            let pick = |kind: admin::FactorKind, prefix: String| -> MenuFn {
                Box::new(move |app: &mut App| {
                    app.sync_prompt(
                        "Cancel a recovery",
                        match kind {
                            admin::FactorKind::Paper => {
                                "Type the 24 words of the paper phrase (nothing is shown)"
                            }
                            admin::FactorKind::Email => "Type the emailed code (nothing is shown)",
                        },
                        true,
                        move |app, secret| {
                            app.run_sync_job(move |env| {
                                Ok(admin::recovery_cancel(
                                    &env.store,
                                    &env.identity,
                                    prefix.trim(),
                                    kind,
                                    &secret,
                                    now(),
                                )?
                                .lines
                                .join("\n"))
                            });
                        },
                    );
                })
            };
            app.sync_menu(
                "Cancel with which third approval?",
                vec![
                    (
                        "Paper phrase".to_string(),
                        pick(admin::FactorKind::Paper, prefix.clone()),
                    ),
                    (
                        "Emailed code".to_string(),
                        pick(admin::FactorKind::Email, prefix),
                    ),
                ],
            );
        }
    }
}

fn recovery_start(app: &mut App) {
    app.sync_prompt(
        "Start a recovery",
        "Type the 24 words of the paper phrase to start with the paper, or leave empty to start with two other devices' approval (nothing is shown)",
        true,
        |app, words| {
            let words = words.trim().to_string();
            let paper = !words.is_empty();
            // The third factor is proven over the new op: the emailed code when the paper
            // authorized it, the paper words otherwise. Empty leaves it awaiting the factor.
            app.sync_prompt(
                "Start a recovery: paper phrase or emailed code",
                if paper {
                    "Type the emailed code (empty: prove it later; nothing is shown)"
                } else {
                    "Type the 24 words of the paper phrase to prove it now (empty: prove it later; nothing is shown)"
                },
                true,
                move |app, third| {
                    app.run_sync_job(move |env| {
                        let third = third.trim().to_string();
                        Ok(admin::recovery_start(
                            &env.store,
                            &env.identity,
                            now(),
                            paper.then_some(words.as_str()),
                            (!third.is_empty()).then_some(third.as_str()),
                        )?
                        .lines
                        .join("\n"))
                    });
                },
            );
        },
    );
}

// ------------------------------------------------------------------- paper

/// The paper phrase as one wizard: the words are shown, you write them down and press Enter, and the type-back
/// check opens by itself. Nothing is registered until you typed them back exactly.
pub(crate) fn paper_wizard(app: &mut App) {
    paper_panel(app, true);
}

fn paper_panel(app: &mut App, wizard: bool) {
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut *seed);
    let words = zeroize::Zeroizing::new(bip39_encode(&seed));
    let mut lines = vec![
        "Write these 24 words on paper, now. They are shown once. They are the paper phrase: the"
            .into(),
        "third approval of every big change. Keep the paper offline and never photograph it."
            .into(),
        String::new(),
    ];
    let all: Vec<&str> = words.split_whitespace().collect();
    for (i, row) in all.chunks(6).enumerate() {
        lines.push(format!(
            "  {:>2}-{:>2}  {}",
            i * 6 + 1,
            i * 6 + row.len(),
            row.join(" ")
        ));
    }
    lines.push(String::new());
    lines.push(if wizard {
        "Step 1 of 2: write the words on paper. Step 2: press Enter and type them back to check them. They are kept only until this program closes, so do both now.".into()
    } else {
        "Close this panel, then choose \"register what I wrote down\" and type them back.".into()
    });
    app.sync_paper = Some(words);
    let then: Option<MenuFn> = wizard.then(|| Box::new(paper_register) as MenuFn);
    app.sync_info_with("Paper phrase", lines, Vec::new(), then);
}

/// The second half: the user types the words back from the paper; only an exact
/// match registers them as the third factor.
pub(crate) fn paper_register(app: &mut App) {
    if app.sync_paper.is_none() {
        app.host_notice = Some("create a paper phrase first".into());
        return;
    }
    app.sync_prompt(
        "Register the paper phrase",
        "Type the 24 words from your paper (nothing is shown)",
        true,
        |app, typed| {
            let expected = app.sync_paper.take();
            if expected.as_deref().map(|e| e.as_str()) != Some(typed.trim()) {
                app.host_notice =
                    Some("the words do not match: nothing registered, create a new phrase".into());
                return;
            }
            let phrase = typed.trim().to_string();
            report_job(app, move |env| {
                admin::factor_register(
                    &env.store,
                    &env.identity,
                    admin::FactorKind::Paper,
                    Some(&phrase),
                    None,
                    now(),
                )
            });
        },
    );
}

pub(crate) fn email_factor(app: &mut App) {
    report_job(app, |env| {
        admin::factor_register(
            &env.store,
            &env.identity,
            admin::FactorKind::Email,
            None,
            None,
            now(),
        )
    });
}

// ----------------------------------------------------------- migrate, kit

pub(crate) fn migrate(app: &mut App, remove_local: bool) {
    report_job(app, move |env| {
        let local = (env.open_secrets)(&env.paths);
        admin::migrate(&env.store, &env.identity, local.as_ref(), remove_local)
    });
}

pub(crate) fn import_kit(app: &mut App) {
    app.sync_prompt(
        "Import the Vault kit",
        "Type or paste the ring secret from the kit sheet (sshub:ring:1:... or the hex groups)",
        true,
        |app, kit| {
            app.sync_prompt(
                "Import the Vault kit",
                "Type the six words of the master phrase (nothing is shown)",
                true,
                move |app, phrase| {
                    report_job(app, move |env| {
                        let local = (env.open_secrets)(&env.paths);
                        admin::import_kit(&env.store, local.as_ref(), &kit, &phrase, env.argon)
                    });
                },
            );
        },
    );
}
