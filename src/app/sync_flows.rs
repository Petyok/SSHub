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
    match d.role {
        super::sync::DeviceRoleView::Holder => "holder",
        super::sync::DeviceRoleView::Mirror => "mirror",
    }
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
    let expected = name.clone();
    app.sync_prompt(
        "Revoke a device",
        format!(
            "{name} will be cut off and the keys rotated. A holder needs the Red ceremony. Type its name to confirm"
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
                "Holder: holds shares, votes on Red changes".to_string(),
                to(DeviceRole::Holder, id.clone()),
            ),
            (
                "Mirror: syncs the ring, holds no share".to_string(),
                to(DeviceRole::Mirror, id),
            ),
        ],
    );
}

// -------------------------------------------------------------------- mode

/// The two-step switch, as `sshub sync-mode` does it: the first call proposes
/// the Red change (nothing is re-encrypted before the ring agreed), a later one
/// re-wraps the credentials once the ceremony completed.
fn mode_job(app: &mut App, phrase: String) {
    app.run_sync_job(move |env| {
        let ring = read_ring_secret(env)?;
        let r = admin::mode(
            &env.store,
            &env.identity,
            CredMode::Quorum,
            None,
            phrase.trim(),
            &ring,
            now(),
        )?;
        Ok(r.lines.join("\n"))
    });
}

/// This device's own switch to Quorum that is not finished: `Some(false)` while
/// its ceremony is incomplete, `Some(true)` when it is in force but credentials
/// are still wrapped for the Vault.
fn unfinished_switch(app: &App) -> Option<bool> {
    let me = app.sync.this_device.clone()?;
    let mc = crate::sync::modes::own_mode_change(&app.store, &me).ok()??;
    if !matches!(
        mc.body,
        crate::sync::oplog::OpBody::ModeChange {
            to: CredMode::Quorum,
            ..
        }
    ) {
        return None;
    }
    if !crate::sync::ceremony::in_force(&app.store, &mc).ok()? {
        return Some(false);
    }
    let public = app
        .store
        .sync_devices()
        .ok()?
        .into_iter()
        .find(|d| d.id == me)?
        .public;
    // The probe fails where a credential would have to be opened: that one is still wrapped for the Vault.
    let pending =
        crate::sync::modes::mode_migration(&app.store, &public, &|_| anyhow::bail!("pending"))
            .is_err();
    pending.then_some(true)
}

pub(crate) fn mode(app: &mut App) {
    match unfinished_switch(app) {
        Some(false) => return mode_job(app, String::new()), // shows the ceremony status
        Some(true) => {
            return app.sync_prompt(
                "Finish the switch to Quorum",
                "The ring agreed. Type the master phrase to re-wrap the credentials for Quorum (nothing is shown)",
                true,
                mode_job,
            )
        }
        None => {}
    }
    match app.sync.cred_mode {
        Some(super::sync::CredModeView::Vault) => {
            // Proposing needs no secret; the phrase is asked when the switch is finished.
            mode_job(app, String::new());
        }
        Some(super::sync::CredModeView::Quorum) => app.sync_info(
            "Switch to Vault",
            vec![
                "Quorum to Vault re-wraps every credential, which needs a second holder".into(),
                "to open them and a co-signing holder for the Red ceremony.".into(),
                "Start it from a device that holds shares: `sshub sync-mode vault --cosigner <device>`.".into(),
            ],
        ),
        None => app.host_notice = Some("the credential mode is not known yet".into()),
    }
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

fn recovery_by_id(app: &mut App, step: RecoveryStep) {
    app.sync_prompt(
        match step {
            RecoveryStep::Approve => "Approve a recovery",
            RecoveryStep::Cancel => "Cancel a recovery",
        },
        "Recovery id (the first characters shown by the status list)",
        false,
        move |app, prefix| match step {
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
                                admin::FactorKind::Email => {
                                    "Type the emailed code (nothing is shown)"
                                }
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
                    "Cancel with which third factor?",
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
        },
    );
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
                "Start a recovery: third factor",
                if paper {
                    "Type the emailed code (empty: prove it later; nothing is shown)"
                } else {
                    "Type the 24 words of the paper phrase as the third factor (empty: prove it later; nothing is shown)"
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

pub(crate) fn paper_setup(app: &mut App) {
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut *seed);
    let words = zeroize::Zeroizing::new(bip39_encode(&seed));
    let mut lines = vec![
        "Write these 24 words on paper, now. They are shown once and are the third factor".into(),
        "of every Red change. Anyone with them and a phone proof can act for you.".into(),
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
    lines.push(
        "Close this panel, then choose \"register what I wrote down\" and type them back.".into(),
    );
    app.sync_paper = Some(words);
    app.sync_info("Paper phrase", lines);
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
