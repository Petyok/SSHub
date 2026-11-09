//! Admin verbs of host sync: the Red ceremony, roles, factors, recovery, mode,
//! exemption, bulk export, migration and the Vault kit. Every verb is a thin
//! wrapper: the builders and the rules live in `crate::sync` (ceremony,
//! recovery, revocation, modes, cred_ops, export); this file parses arguments,
//! asks for secrets without echo, appends the op and prints where the ceremony
//! stands.
//!
//! Headless like `sync_cmd`: dispatched from `main.rs` before `CliContext`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::credentials::PasswordStore;
use crate::profile::ProfilePaths;
use crate::store::LauncherStore;
use crate::sync::ceremony::{self, RedProof, RedStatus};
use crate::sync::crypto::{paper_derive, DeviceIdentity, PaperDomain};
use crate::sync::identity_store::{load_or_init_identity, save_identity};
use crate::sync::oplog::{
    CredMode, DeviceId, DeviceRole, DeviceRow, Op, OpBody, OpTier, ThirdFactor,
};

use super::help;

pub const COMMANDS: [&str; 14] = [
    "sync-revoke",
    "sync-role",
    "sync-approve",
    "sync-factor",
    "sync-factor-register",
    "sync-recovery",
    "sync-mode",
    "sync-exemption",
    "sync-export",
    "sync-migrate",
    "sync-import-kit",
    "sync-heal",
    "sync-rekey",
    "sync-replace-terminal",
];

const EMPTY_RING: &str = "No sync ring on this profile yet.";

pub fn is_admin_command(cmd: &str) -> bool {
    COMMANDS.contains(&cmd)
}

// ------------------------------------------------------------------ parsing

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactorKind {
    Paper,
    Email,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryCmd {
    Start { paper: bool },
    Approve(String),
    Ack(String),
    Cancel { op: String, factor: FactorKind },
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Revoke(String),
    Role(String, DeviceRole),
    Approve(String),
    Factor(String, FactorKind),
    FactorRegister(FactorKind, Option<String>),
    Recovery(RecoveryCmd),
    Mode(CredMode, Option<String>),
    Exemption {
        on: bool,
        device: String,
    },
    Export {
        window: u32,
        out: PathBuf,
        wait: u64,
    },
    Migrate {
        remove_local: bool,
    },
    ImportKit,
    Heal,
    Rekey {
        csv: Option<String>,
        stdin: bool,
        new_keys: Vec<String>,
    },
    ReplaceTerminal {
        old: String,
        new: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Opts {
    pub json: bool,
}

/// Arguments after the global profile flags.
pub fn parse(cmd: &str, args: &[String]) -> std::result::Result<(Cmd, Opts), String> {
    let mut opts = Opts::default();
    let mut pos: Vec<&str> = Vec::new();
    let (mut paper, mut email_code, mut remove_local) = (false, false, false);
    let (mut window, mut out, mut wait, mut sender, mut factor, mut cosigner) =
        (None, None, 0u64, None, None, None);
    let (mut csv, mut stdin, mut new_keys, mut new_dev): (
        Option<String>,
        bool,
        Vec<String>,
        Option<String>,
    ) = (None, false, Vec::new(), None);
    let mut i = 0;
    let value = |i: usize, flag: &str| -> std::result::Result<String, String> {
        args.get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .cloned()
            .ok_or_else(|| format!("{flag} requires a value"))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--format" => {
                opts.json = match value(i, a)?.as_str() {
                    "plain" => false,
                    "json" => true,
                    other => return Err(format!("unknown format '{other}'")),
                };
                i += 2;
            }
            "--paper" => {
                paper = true;
                i += 1;
            }
            "--email-code" => {
                email_code = true;
                i += 1;
            }
            "--remove-local" => {
                remove_local = true;
                i += 1;
            }
            "--window" => {
                window = Some(
                    value(i, a)?
                        .parse::<u32>()
                        .map_err(|_| "--window takes a number of seconds".to_string())?,
                );
                i += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(value(i, a)?));
                i += 2;
            }
            "--wait" => {
                wait = value(i, a)?
                    .parse::<u64>()
                    .map_err(|_| "--wait takes a number of seconds".to_string())?;
                i += 2;
            }
            "--sender" => {
                sender = Some(value(i, a)?);
                i += 2;
            }
            "--csv" => {
                csv = Some(value(i, a)?);
                i += 2;
            }
            "--stdin" => {
                stdin = true;
                i += 1;
            }
            "--new-key" => {
                new_keys.push(value(i, a)?);
                i += 2;
            }
            "--new" => {
                new_dev = Some(value(i, a)?);
                i += 2;
            }
            "--cosigner" => {
                cosigner = Some(value(i, a)?);
                i += 2;
            }
            "--factor" => {
                factor = Some(match value(i, a)?.as_str() {
                    "paper" => FactorKind::Paper,
                    "email" => FactorKind::Email,
                    other => return Err(format!("unknown factor '{other}' (paper or email)")),
                });
                i += 2;
            }
            _ if a.starts_with('-') => return Err(format!("unknown option '{a}' for {cmd}")),
            _ => {
                pos.push(a);
                i += 1;
            }
        }
    }
    let exact = |n: usize, what: &str| -> std::result::Result<(), String> {
        if pos.len() == n {
            Ok(())
        } else {
            Err(format!("{cmd} takes {what}"))
        }
    };
    let only = |flags: &[(&str, bool)]| -> std::result::Result<(), String> {
        for (name, set) in flags {
            if *set {
                return Err(format!("{name} does not apply to {cmd}"));
            }
        }
        Ok(())
    };
    let cmd_val = match cmd {
        "sync-revoke" => {
            exact(1, "one device")?;
            Cmd::Revoke(pos[0].into())
        }
        "sync-role" => {
            exact(2, "a device and holder|mirror")?;
            let role = match pos[1] {
                "holder" => DeviceRole::Holder,
                "mirror" => DeviceRole::Mirror,
                other => {
                    return Err(format!(
                        "unknown role '{other}' (holder = full device, mirror = view-only copy)"
                    ))
                }
            };
            Cmd::Role(pos[0].into(), role)
        }
        "sync-approve" => {
            exact(1, "one change id")?;
            Cmd::Approve(pos[0].into())
        }
        "sync-factor" => {
            exact(1, "one change id")?;
            let kind = match (paper, email_code) {
                (true, false) => FactorKind::Paper,
                (false, true) => FactorKind::Email,
                _ => return Err("give exactly one of --paper and --email-code".into()),
            };
            Cmd::Factor(pos[0].into(), kind)
        }
        "sync-factor-register" => {
            exact(1, "paper or email")?;
            let kind = match pos[0] {
                "paper" => FactorKind::Paper,
                "email" => FactorKind::Email,
                other => return Err(format!("unknown factor '{other}' (paper or email)")),
            };
            Cmd::FactorRegister(kind, sender)
        }
        "sync-recovery" => {
            let sub = *pos
                .first()
                .ok_or("sync-recovery needs start, approve, cancel or status")?;
            let rc = match sub {
                "start" => {
                    exact(1, "no argument after start")?;
                    RecoveryCmd::Start { paper }
                }
                "approve" => {
                    exact(2, "a change id after approve")?;
                    RecoveryCmd::Approve(pos[1].into())
                }
                "ack" => {
                    exact(2, "a change id after ack")?;
                    RecoveryCmd::Ack(pos[1].into())
                }
                "cancel" => {
                    exact(2, "a change id after cancel")?;
                    RecoveryCmd::Cancel {
                        op: pos[1].into(),
                        factor: factor.ok_or("cancel needs --factor paper|email")?,
                    }
                }
                "status" => {
                    exact(1, "no argument after status")?;
                    RecoveryCmd::Status
                }
                other => return Err(format!("unknown recovery step '{other}'")),
            };
            Cmd::Recovery(rc)
        }
        "sync-mode" => {
            exact(1, "vault or quorum")?;
            let to = match pos[0] {
                "vault" => CredMode::Vault,
                "quorum" => CredMode::Quorum,
                other => return Err(format!("unknown mode '{other}' (vault or quorum)")),
            };
            Cmd::Mode(to, cosigner)
        }
        "sync-exemption" => {
            exact(2, "on|off and a device")?;
            let on = match pos[0] {
                "on" => true,
                "off" => false,
                other => return Err(format!("unknown state '{other}' (on or off)")),
            };
            Cmd::Exemption {
                on,
                device: pos[1].into(),
            }
        }
        "sync-export" => {
            exact(0, "no positional argument")?;
            let window = window.ok_or("sync-export needs --window <seconds>")?;
            if window == 0 || window > 3600 {
                return Err("--window is 1 to 3600 seconds".into());
            }
            Cmd::Export {
                window,
                out: out.clone().ok_or("sync-export needs --out <path>")?,
                wait,
            }
        }
        "sync-migrate" => {
            exact(0, "no positional argument")?;
            Cmd::Migrate { remove_local }
        }
        "sync-import-kit" => {
            exact(0, "no positional argument")?;
            Cmd::ImportKit
        }
        "sync-heal" => {
            exact(0, "no positional argument")?;
            Cmd::Heal
        }
        "sync-rekey" => {
            exact(
                0,
                "no positional argument (use --csv FILE, --stdin or --new-key HOST)",
            )?;
            Cmd::Rekey {
                csv: csv.clone(),
                stdin,
                new_keys: new_keys.clone(),
            }
        }
        "sync-replace-terminal" => {
            exact(1, "the computer being replaced")?;
            Cmd::ReplaceTerminal {
                old: pos[0].into(),
                new: new_dev.clone(),
            }
        }
        other => return Err(format!("unknown sync admin command {other}")),
    };
    // Flags that belong to other verbs are mistakes, not no-ops.
    let uses_paper = matches!(
        cmd_val,
        Cmd::Recovery(RecoveryCmd::Start { .. }) | Cmd::Factor(..)
    );
    only(&[
        ("--paper", paper && !uses_paper),
        (
            "--email-code",
            email_code && !matches!(cmd_val, Cmd::Factor(..)),
        ),
        (
            "--remove-local",
            remove_local && !matches!(cmd_val, Cmd::Migrate { .. }),
        ),
        (
            "--csv",
            csv.is_some() && !matches!(cmd_val, Cmd::Rekey { .. }),
        ),
        ("--stdin", stdin && !matches!(cmd_val, Cmd::Rekey { .. })),
        (
            "--new-key",
            !new_keys.is_empty() && !matches!(cmd_val, Cmd::Rekey { .. }),
        ),
        (
            "--new",
            new_dev.is_some() && !matches!(cmd_val, Cmd::ReplaceTerminal { .. }),
        ),
        (
            "--window",
            window.is_some() && !matches!(cmd_val, Cmd::Export { .. }),
        ),
        (
            "--out",
            out.is_some() && !matches!(cmd_val, Cmd::Export { .. }),
        ),
    ])?;
    Ok((cmd_val, opts))
}

// --------------------------------------------------------------------- entry

/// Run one of the admin verbs; returns the process exit code.
pub fn run(
    cmd: &str,
    args: &[String],
    resolve: impl FnOnce() -> Result<ProfilePaths>,
) -> Result<i32> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        help::print_command_help(cmd);
        return Ok(0);
    }
    let (parsed, opts) = match parse(cmd, args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("sshub: {msg}");
            eprintln!("       run `sshub {cmd} --help` for usage");
            return Ok(2);
        }
    };
    let paths = match resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("sshub: {e:#}");
            return Ok(1);
        }
    };
    Ok(match execute(&paths, parsed, opts) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("sshub: {cmd}: {e:#}");
            1
        }
    })
}

fn execute(paths: &ProfilePaths, cmd: Cmd, opts: Opts) -> Result<i32> {
    let store = LauncherStore::open(paths.launcher_db()).context("open the launcher database")?;
    let secrets = crate::credentials::open_for_profile(paths);
    // Only the kit import may start from nothing; the others never create a device key.
    if !matches!(cmd, Cmd::ImportKit) && store.sync_devices()?.is_empty() {
        bail!("{EMPTY_RING}");
    }
    let mut id = load_or_init_identity(paths, secrets.as_ref())?;
    let now = super::output::now_ts();
    match cmd {
        Cmd::Revoke(d) => {
            let r = revoke(paths, &store, &mut id, secrets.as_ref(), &d, now)?;
            report(opts, &r)
        }
        Cmd::Role(d, to) => report(opts, &role(&store, &id, &d, to, now)?),
        Cmd::Approve(op) => report(opts, &approve(&store, &id, &op, now)?),
        Cmd::Factor(op, kind) => {
            let secret = match kind {
                FactorKind::Paper => super::unlock::prompt_secret("Paper phrase (24 words): "),
                FactorKind::Email => super::unlock::prompt_secret("Emailed code: "),
            }
            .context("no secret entered")?;
            report(opts, &factor(&store, &id, &op, kind, &secret, now)?)
        }
        Cmd::FactorRegister(kind, sender) => {
            let phrase = match kind {
                FactorKind::Paper => Some(
                    super::unlock::prompt_secret("Paper phrase (24 words): ")
                        .context("no phrase entered")?,
                ),
                FactorKind::Email => None,
            };
            let r = factor_register(
                &store,
                &id,
                kind,
                phrase.as_deref().map(String::as_str),
                sender.as_deref(),
                now,
            )?;
            report(opts, &r)
        }
        Cmd::Recovery(rc) => recovery(&store, &id, rc, opts, now),
        Cmd::Mode(to, cosigner) => {
            // Only the Vault to Quorum re-wrap opens credentials with the phrase and the ring secret.
            let (phrase, ring) = if to == CredMode::Vault {
                (Zeroizing::new(String::new()), [0u8; 16])
            } else {
                let phrase = super::unlock::prompt_secret("Master phrase (six words): ")
                    .context("no phrase entered")?;
                let ring = secrets
                    .get(crate::sync::runtime::RING_SECRET_KEY)?
                    .map(|t| crate::sync::quorum::parse_ring_secret(&t))
                    .transpose()?
                    .context("this profile has no ring secret; import the Vault kit first")?;
                (phrase, ring)
            };
            report(
                opts,
                &mode(&store, &id, to, cosigner.as_deref(), &phrase, &ring, now)?,
            )
        }
        Cmd::Exemption { on, device } => {
            if on {
                bail!("Turning the master phrase exemption on is done in the terminal screen: tab 7, M, Advanced, Master phrase exemption.");
            }
            report(opts, &exemption_off(&store, &id, &device, now)?)
        }
        Cmd::Export { window, out, wait } => export(paths, window, &out, wait, opts),
        Cmd::Migrate { remove_local } => {
            let local = crate::credentials::open_ungated_for_profile(paths);
            let r = migrate(&store, &id, local.as_ref(), remove_local)?;
            report(opts, &r)
        }
        Cmd::Heal => {
            let phrase = super::unlock::prompt_secret("Paper phrase (24 words): ")
                .context("no phrase entered")?;
            report(opts, &super::sync_repair::heal(&store, &id, &phrase)?)
        }
        Cmd::Rekey {
            csv,
            stdin,
            new_keys,
        } => {
            use super::sync_repair as rk;
            let mut text = String::new();
            if let Some(path) = &csv {
                text = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
            } else if stdin {
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            }
            let entries = rk::parse_csv(&text)?;
            let mut r = if entries.is_empty() && new_keys.is_empty() {
                let list = rk::rekey_list(&store)?;
                let mut lines: Vec<String> = list
                    .iter()
                    .map(|(_, n, d)| {
                        format!("{n}: shares a key with the removed device {}", d.join(", "))
                    })
                    .collect();
                if lines.is_empty() {
                    lines.push("no saved password shares a key with a removed device".into());
                } else {
                    lines.push("Change each one on its server, then save the new secret: --csv FILE or --stdin with lines host,newpassword, or --new-key HOST for a new key.".into());
                }
                Report {
                    json: json!({"exposed": list.iter().map(|(_, n, _)| n).collect::<Vec<_>>()}),
                    lines,
                }
            } else {
                rk::rekey_apply(&store, &id, &entries)?
            };
            if !new_keys.is_empty() {
                let mut install = rk::exec_installer(paths);
                let k = rk::rekey_new_key(&store, &id, &new_keys, &mut install)?;
                r.lines.extend(k.lines);
            }
            report(opts, &r)
        }
        Cmd::ReplaceTerminal { old, new } => {
            let phrase = if new.is_some() {
                super::unlock::prompt_secret("Paper phrase (24 words): ")
                    .context("no phrase entered")?
            } else {
                Zeroizing::new(String::new())
            };
            let r = super::sync_repair::replace_terminal(
                &store,
                &mut id,
                &old,
                new.as_deref(),
                &phrase,
                now,
                |id, dev| revoke(paths, &store, id, secrets.as_ref(), dev, now),
            )?;
            report(opts, &r)
        }
        Cmd::ImportKit => {
            let kit = super::unlock::prompt_secret("Ring kit (sshub:ring:1:...): ")
                .context("no kit entered")?;
            let phrase = super::unlock::prompt_secret("Master phrase (six words): ")
                .context("no phrase entered")?;
            let local = crate::credentials::open_ungated_for_profile(paths);
            let r = import_kit(
                &store,
                local.as_ref(),
                &kit,
                &phrase,
                crate::sync::crypto::ARGON2_MASTER,
            )?;
            report(opts, &r)
        }
    }
}

// ------------------------------------------------------------------ reports

/// What a verb did: lines for people, the same facts as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub lines: Vec<String>,
    pub json: Value,
}

fn report(opts: Opts, r: &Report) -> Result<i32> {
    if opts.json {
        println!("{}", serde_json::to_string_pretty(&r.json)?);
    } else {
        for l in &r.lines {
            println!("{l}");
        }
    }
    Ok(0)
}

pub(crate) fn short(h: &[u8; 32]) -> String {
    h[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn proof_name(p: &RedProof) -> String {
    match p {
        RedProof::Phone => "phone".into(),
        RedProof::SecondDevice => "second device".into(),
        RedProof::Factor(None) => "paper phrase or emailed code".into(),
        RedProof::Factor(Some(ThirdFactor::Paper)) => "paper".into(),
        RedProof::Factor(Some(ThirdFactor::EmailCode)) => "emailed code".into(),
    }
}

/// The ceremony lines of a Red op: what is there and what is still missing.
fn ceremony_lines(st: &RedStatus) -> Vec<String> {
    let names = |v: &[RedProof]| v.iter().map(proof_name).collect::<Vec<_>>().join(", ");
    let mut out = vec![format!(
        "approvals: have [{}], missing [{}]{}",
        names(&st.have),
        names(&st.missing),
        if st.complete { " (complete)" } else { "" }
    )];
    if st.expired {
        out.push("the approval round ran out of time: propose the change again".into());
    }
    if let Some(why) = &st.blocked {
        out.push(format!("blocked: {why}"));
    }
    if st.single_machine_ring {
        out.push("single machine ring: the paper phrase is required".into());
    }
    out
}

fn status_json(st: &RedStatus) -> Value {
    json!({
        "have": st.have.iter().map(proof_name).collect::<Vec<_>>(),
        "missing": st.missing.iter().map(proof_name).collect::<Vec<_>>(),
        "complete": st.complete,
        "expired": st.expired,
        "blocked": st.blocked,
    })
}

/// Report for an op we just appended (or looked up): its tier and, for Red
/// ops, where the ceremony stands.
pub(crate) fn op_report(
    store: &LauncherStore,
    op: &Op,
    headline: String,
    now: i64,
) -> Result<Report> {
    let mut lines = vec![headline];
    // A Red kind this ring does not gate (revoking a mirror: it holds no shares) is reviewed like a Yellow change,
    // and is reported as one: the tier shown is the tier that decides.
    let gated = ceremony::is_gated(store, op)?;
    let shown = if !gated && op.tier() == OpTier::Red {
        OpTier::Yellow
    } else {
        op.tier()
    };
    let mut json = json!({ "op": hex(&op.hash()), "tier": format!("{shown:?}") });
    if gated {
        let st = ceremony::red_status(store, op, now)?;
        lines.extend(ceremony_lines(&st));
        json["ceremony"] = status_json(&st);
    } else if op.tier() == OpTier::Red {
        lines.push(
            "this revokes a device that holds no shares: Yellow here, peers apply it after their review".into(),
        );
    } else {
        lines.push("peers apply this change after their review".into());
    }
    Ok(Report { lines, json })
}

fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------------ lookups

pub(crate) fn find_device(store: &LauncherStore, spec: &str) -> Result<DeviceRow> {
    let all = crate::sync::ceremony::effective_devices(store, None)?;
    let hits: Vec<&DeviceRow> = all
        .iter()
        .filter(|d| d.id.to_string().starts_with(spec) || d.name == spec)
        .collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no device matches '{spec}' (see `sshub sync-status`)"),
        _ => bail!("'{spec}' matches several devices; give more of the id"),
    }
}

/// An op of the log by hash prefix (at least 6 hex digits).
pub(crate) fn find_op(store: &LauncherStore, prefix: &str) -> Result<Op> {
    ensure!(
        prefix.len() >= 6 && prefix.bytes().all(|b| b.is_ascii_hexdigit()),
        "a change id is at least 6 hex digits (see `sshub sync-review`)"
    );
    let want = prefix.to_ascii_lowercase();
    store.with_conn(|conn| {
        let mut stmt =
            conn.prepare("SELECT op_hash FROM sync_ops WHERE lower(hex(op_hash)) LIKE ?1 || '%'")?;
        let hashes: Vec<Vec<u8>> = stmt
            .query_map([&want], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        match hashes.as_slice() {
            [h] => {
                let h: [u8; 32] = h
                    .as_slice()
                    .try_into()
                    .map_err(|_| anyhow!("stored hash"))?;
                Ok(crate::sync::oplog::merge::load_op(conn, &h)?
                    .context("change vanished")?
                    .op)
            }
            [] => bail!("no change matches '{prefix}'"),
            _ => bail!("'{prefix}' matches several changes; give more digits"),
        }
    })
}

// -------------------------------------------------------------------- verbs

/// A host's name for a credential list, from the host's uid.
pub(crate) fn host_label(store: &LauncherStore, uid: &crate::sync::oplog::HostUid) -> String {
    store
        .host_id_by_uid(uid)
        .ok()
        .flatten()
        .and_then(|id| store.get_host(id).ok().flatten())
        .map_or_else(|| "(unknown host)".into(), |h| h.name)
}

/// Names of the hosts on either exposure list.
pub(crate) fn exposed_host_names(store: &LauncherStore) -> Result<Vec<String>> {
    let mut names: Vec<String> = crate::sync::quorum::exposed_credentials(store)?
        .iter()
        .map(|e| host_label(store, &e.host_uid))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The exposure of saved passwords as plain lines (design 4.3): credentials that share a key with a removed device
/// (change them) and credentials a current full device does not hold a piece of yet (healed on use).
pub(crate) fn exposure_lines(store: &LauncherStore) -> Result<Vec<String>> {
    use crate::sync::quorum::{exposed_credentials, ExposureKind};
    let mut removed = Vec::new();
    let mut not_held: Vec<String> = Vec::new();
    let mut who = std::collections::BTreeSet::new();
    let names: std::collections::HashMap<_, _> =
        crate::sync::ceremony::effective_devices(store, None)?
            .into_iter()
            .map(|d| (d.id, d.name))
            .collect();
    for e in exposed_credentials(store)? {
        let host = host_label(store, &e.host_uid);
        match e.kind {
            ExposureKind::Removed => removed.push(host),
            ExposureKind::NotHeldBy => {
                not_held.push(host);
                who.extend(
                    e.devices
                        .iter()
                        .map(|d| names.get(d).cloned().unwrap_or_else(|| short_id(d))),
                );
            }
        }
    }
    let mut out = Vec::new();
    if !removed.is_empty() {
        out.push(format!(
            "{} credential(s) share a key with a removed device (change them): {}",
            removed.len(),
            removed.join(", ")
        ));
    }
    if !not_held.is_empty() {
        out.push(format!(
            "{} credential(s) not shared with {} (healing on use): {}",
            not_held.len(),
            who.into_iter().collect::<Vec<_>>().join(", "),
            not_held.join(", ")
        ));
    }
    Ok(out)
}

/// What to do about a removed device's passwords when it was stolen or lost (the answer the TUI asks first).
pub(crate) const STOLEN_NOTE: &str = "It was stolen or lost: change the passwords of the hosts listed above on the servers, remove its key from their authorized_keys, then save the new secrets: M, Passwords, Change exposed credentials (or `sshub sync-rekey`).";

/// What to do about a removed device's passwords when it was destroyed or wiped.
pub(crate) const WIPED_NOTE: &str = "It was destroyed or wiped: nothing of it is left to misuse. Run M, Passwords, Heal credentials (or `sshub sync-heal`) so the other computers hold every saved password.";

/// A registered paper phrase comes first (design 3.1): before a switch to Quorum, and before a change that leaves
/// three or more full devices. Without it a stolen device cannot be cut off and a lost phone cannot be recovered.
pub(crate) fn paper_gate(
    store: &LauncherStore,
    to_quorum: bool,
    holders_after: usize,
) -> Result<()> {
    ensure!(
        !(to_quorum || holders_after >= 3) || crate::sync::quorum::paper_pub(store)?.is_some(),
        "register the paper phrase first (TUI: tab 7, M, Paper phrase; or `sshub sync-factor-register paper`): without it you could not cut off a stolen device, and you could not recover a lost phone"
    );
    Ok(())
}

pub(crate) fn role(
    store: &LauncherStore,
    id: &DeviceIdentity,
    device: &str,
    to: DeviceRole,
    now: i64,
) -> Result<Report> {
    let row = find_device(store, device)?;
    ensure!(row.role != to, "{} is already a {to:?}", short_id(&row.id));
    if to == DeviceRole::Holder {
        let after = crate::sync::quorum::live_holder_count(store)? + 1;
        paper_gate(store, false, after)?;
    }
    let op = store.sync_append_local(
        id,
        OpBody::DeviceRoleChange {
            device_id: row.id.clone(),
            to,
        },
    )?;
    let after = crate::sync::revocation::live_terminal_holders(store)?
        + usize::from(
            to == DeviceRole::Holder && row.kind == crate::sync::oplog::DeviceKind::Terminal,
        );
    let mut r = op_report(
        store,
        &op,
        format!(
            "proposed: {} becomes a {} (change {})",
            short_id(&row.id),
            if to == DeviceRole::Holder {
                "full device"
            } else {
                "view-only copy"
            },
            short(&op.hash())
        ),
        now,
    )?;
    // Say it when a device is made a full device: with fewer than three terminals a stolen one cannot be cut off.
    if to == DeviceRole::Holder && after < crate::sync::revocation::TERMINALS_TO_CUT_OFF {
        let w = crate::sync::revocation::cut_off_sentence(after);
        r.lines.push(format!("warning: {w}"));
        r.json["warning"] = json!(w);
    }
    Ok(r)
}

fn short_id(id: &DeviceId) -> String {
    id.to_string().chars().take(8).collect()
}

pub(crate) fn revoke(
    paths: &ProfilePaths,
    store: &LauncherStore,
    id: &mut DeviceIdentity,
    secrets: &dyn PasswordStore,
    device: &str,
    now: i64,
) -> Result<Report> {
    let row = find_device(store, device)?;
    if row.role == DeviceRole::Holder {
        crate::sync::revocation::check_revocable(store, &row.id, now)?;
    }
    let op = store.sync_append_local(
        id,
        OpBody::DeviceRevoke {
            device_id: row.id.clone(),
        },
    )?;
    let mut r = op_report(
        store,
        &op,
        format!(
            "proposed: remove {} (change {})",
            short_id(&row.id),
            short(&op.hash())
        ),
        now,
    )?;
    // The cryptographic follow-up (rotate, reshare, adopt, retire) needs no
    // prompt; every remaining holder runs it after each round.
    let steps = drive_revocation(paths, store, id, secrets, now)?;
    for s in &steps {
        r.lines.push(format!("revocation step: {s}"));
    }
    r.json["steps"] = json!(steps);
    let exposure = exposure_lines(store)?;
    r.lines.extend(exposure.iter().cloned());
    r.json["exposure"] = json!(exposure);
    Ok(r)
}

/// Do every revocation step this device owes right now.
pub(crate) fn drive_revocation(
    paths: &ProfilePaths,
    store: &LauncherStore,
    id: &mut DeviceIdentity,
    secrets: &dyn PasswordStore,
    now: i64,
) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for _ in 0..8 {
        let actions = crate::sync::revocation::next_actions(store, id, now)?;
        let Some(action) = actions.first() else { break };
        let bodies = crate::sync::revocation::perform(store, id, action)?;
        // The changed key file first, then the announcement.
        save_identity(paths, secrets, id)?;
        for b in bodies {
            store.sync_append_local(id, b)?;
        }
        done.push(format!("{action:?}"));
    }
    Ok(done)
}

pub(crate) fn approve(
    store: &LauncherStore,
    id: &DeviceIdentity,
    op_id: &str,
    now: i64,
) -> Result<Report> {
    let op = find_op(store, op_id)?;
    ceremony::require_gated(store, &op)?;
    ensure!(
        op.device_id != id.public().device_id(),
        "propose from this device, approve from another one"
    );
    let vote = store.sync_append_local(id, ceremony::red_approve_body(&op))?;
    ceremony::settle_red_ops(store, now)?;
    op_report(
        store,
        &op,
        format!(
            "approved change {} as {} (vote {})",
            short(&op.hash()),
            short_id(&id.public().device_id()),
            short(&vote.hash())
        ),
        now,
    )
}

pub(crate) fn factor(
    store: &LauncherStore,
    id: &DeviceIdentity,
    op_id: &str,
    kind: FactorKind,
    secret: &str,
    now: i64,
) -> Result<Report> {
    let op = find_op(store, op_id)?;
    // A recovery (and a device's cancellation of one) has its own path: its third factor is proven
    // the same way, by a `RedFactor` over the op's hash, but the verdict is the recovery's.
    let recovery_op = matches!(
        op.body,
        OpBody::PhoneReenroll { .. } | OpBody::PhoneReenrollCancel { .. }
    );
    if !recovery_op {
        ceremony::require_gated(store, &op)?;
    }
    let body = match kind {
        FactorKind::Paper => {
            let recover = paper_derive(secret, PaperDomain::Recover).context("the paper phrase")?;
            ceremony::red_factor_paper(&recover, &op)
        }
        FactorKind::Email => ceremony::red_factor_email(store, secret.trim(), &op)?,
    };
    let sig = store.sync_append_local(id, body)?;
    if recovery_op {
        let mut lines = vec![format!(
            "paper phrase or emailed code added to change {} (record {})",
            short(&op.hash()),
            short(&sig.hash())
        )];
        lines.extend(recovery_lines(store, &recovery_target(store, &op)?, now));
        return Ok(Report {
            lines,
            json: json!({ "op": hex(&op.hash()), "factor": hex(&sig.hash()) }),
        });
    }
    ceremony::settle_red_ops(store, now)?;
    op_report(
        store,
        &op,
        format!(
            "paper phrase or emailed code added to change {} (record {})",
            short(&op.hash()),
            short(&sig.hash())
        ),
        now,
    )
}

pub(crate) fn factor_register(
    store: &LauncherStore,
    id: &DeviceIdentity,
    kind: FactorKind,
    phrase: Option<&str>,
    sender: Option<&str>,
    now: i64,
) -> Result<Report> {
    let (body, note) = match kind {
        FactorKind::Paper => {
            let phrase = phrase.context("the paper phrase is required")?;
            (
                ceremony::factor_register_paper(phrase).context("the paper phrase")?,
                None,
            )
        }
        FactorKind::Email => {
            let sender_id: DeviceId = match sender {
                Some(s) => find_device(store, s)?.id,
                None => id.public().device_id(),
            };
            // The code lives, sealed, on the device that mails it: register from that device.
            ensure!(
                sender_id == id.public().device_id(),
                "the emailed-code factor is registered ON its sender: run this on the device that will mail the code"
            );
            let (body, code) = ceremony::factor_register_email(&sender_id)?;
            ceremony::remember_email_code(store, id, &code)?;
            (body, Some(code))
        }
    };
    let op = store.sync_append_local(id, body)?;
    let mut r = op_report(
        store,
        &op,
        format!(
            "registered the {} (change {})",
            if kind == FactorKind::Paper {
                "paper phrase"
            } else {
                "emailed code"
            },
            short(&op.hash())
        ),
        now,
    )?;
    if let Some(code) = note {
        // Shown once: it is the factor itself.
        r.lines.push(format!(
            "email factor code (write it down, it is not shown again): {code}"
        ));
        r.json["code"] = json!(code);
    }
    Ok(r)
}

/// The `PhoneReenroll` op a recovery op belongs to (itself, or the one a cancellation names).
fn recovery_target(store: &LauncherStore, op: &Op) -> Result<Op> {
    match &op.body {
        OpBody::PhoneReenrollCancel { reenroll_hash, .. } => {
            let hex_id: String = reenroll_hash.iter().map(|b| format!("{b:02x}")).collect();
            find_op(store, &hex_id)
        }
        _ => Ok(op.clone()),
    }
}

/// What a recovery verdict means, in words the user can act on.
fn verdict_hint(v: &crate::sync::recovery::ReenrollVerdict) -> &'static str {
    use crate::sync::recovery::ReenrollVerdict as V;
    match v {
        V::AwaitingFactor => "the paper phrase or emailed code was not proven yet: run `sshub sync-factor <change-id>` with the paper words or the emailed code",
        V::Lapsed => "it was ready but nobody completed it within 3 days: it authorizes nothing any more, start a new one",
        V::Ready => "ready: the lost phone can now be replaced",
        V::AwaitingApproval { .. } => "waiting for approvals of other devices with a person at the keyboard",
        V::AwaitingAck => "waiting for another device with a person at the keyboard to acknowledge it",
        V::NeedsMoreTime { .. } => "the time lock is still running",
        V::CancelledByPhone | V::CancelledByFactor => "cancelled",
        V::Completed => "completed: the old phone is revoked and the new one enrolled",
        V::Superseded => "a new phone was set up after it started: it authorizes nothing any more, start a new one if the phone is still lost",
    }
}

/// After a phone is admitted (design 7b): is a recovery of it mailed? `(configured, lines)`. Names a shared mailbox when
/// the address that receives the alert is also the sender of the emailed code.
pub(crate) fn notify_hint(cfg: Option<&crate::config::NotifyConfig>) -> (bool, Vec<String>) {
    match cfg.filter(|c| c.command.is_some() && !c.to.is_empty()) {
        None => (
            false,
            vec![
                "Be told when someone tries to replace this phone: no mail is configured, so a hostile recovery would only show on terminal screens.".to_string(),
                "To get mail, add to the sshub config file: [sync.notify] command = \"/usr/sbin/sendmail -t -i\", to = [\"you@example.com\"], from = \"sshub@example.com\".".to_string(),
            ],
        ),
        Some(c) => {
            let mut lines = vec![format!(
                "A recovery of this phone is mailed to {}.",
                c.to.join(", ")
            )];
            if c.from.as_ref().is_some_and(|f| c.to.iter().any(|t| t == f)) {
                lines.push("Shared mailbox: the alert goes to the same address the emailed code is sent from, so whoever reads that mailbox sees both. Do not count the mail as a separate check.".to_string());
            }
            (true, lines)
        }
    }
}

/// The recoveries a person at this terminal is told about, as ONE alert (design 7b): deduped by requester with a
/// count and the earliest end of the time lock. Built from `recovery::open_for_alert`; it never writes an op and has
/// no acknowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertView {
    /// `(requester name, open recoveries of it)`, in the order the requesters first appear.
    pub requesters: Vec<(String, usize)>,
    pub total: usize,
    /// The earliest time a still-locked recovery can complete.
    pub earliest_lock: Option<i64>,
    /// Hashes of the recoveries, so a screen can remember which ones were dismissed.
    pub hashes: Vec<[u8; 32]>,
    /// One JSON object per recovery: `{op, requester, verdict, lock_ends}`.
    pub items: Vec<serde_json::Value>,
}

pub(crate) fn alert_view(store: &LauncherStore, ops: &[Op], now: i64) -> Result<Option<AlertView>> {
    if ops.is_empty() {
        return Ok(None);
    }
    let names: std::collections::HashMap<_, _> =
        crate::sync::ceremony::effective_devices(store, None)?
            .into_iter()
            .map(|d| (d.id, d.name))
            .collect();
    let mut requesters: Vec<(DeviceId, String, usize)> = Vec::new();
    let mut earliest = None;
    let mut items = Vec::new();
    for op in ops {
        match requesters.iter_mut().find(|r| r.0 == op.device_id) {
            Some(r) => r.2 += 1,
            None => requesters.push((
                op.device_id.clone(),
                names
                    .get(&op.device_id)
                    .cloned()
                    .unwrap_or_else(|| short_id(&op.device_id)),
                1,
            )),
        }
        let verdict = crate::sync::recovery::validate_phone_reenroll(store, op, now);
        let lock_ends = match &verdict {
            Ok(crate::sync::recovery::ReenrollVerdict::NeedsMoreTime { ready_at }) => {
                Some(*ready_at)
            }
            _ => None,
        };
        if let Some(t) = lock_ends {
            earliest = Some(earliest.map_or(t, |e: i64| e.min(t)));
        }
        items.push(json!({
            "op": short(&op.hash()),
            "requester": op.device_id,
            "verdict": verdict.map_or_else(|e| format!("invalid: {e:#}"), |v| format!("{v:?}")),
            "lock_ends": lock_ends,
        }));
    }
    Ok(Some(AlertView {
        requesters: requesters.into_iter().map(|(_, n, c)| (n, c)).collect(),
        total: ops.len(),
        earliest_lock: earliest,
        hashes: ops.iter().map(|o| o.hash()).collect(),
        items,
    }))
}

/// The alert's plain lines (CLI `sync-status`, the full-screen notice).
pub(crate) fn alert_lines(a: &AlertView, now: i64) -> Vec<String> {
    let who = a
        .requesters
        .iter()
        .map(|(n, c)| {
            if *c > 1 {
                format!("{n} ({c})")
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = vec![format!(
        "RECOVERY STARTED: a recovery of the phone is under way: {} open, started from {who}.",
        a.total
    )];
    if let Some(t) = a.earliest_lock {
        out.push(format!(
            "The earliest time lock ends in {}.",
            crate::sync::health::human_left((t - now).max(0))
        ));
    }
    out.push(
        "If your phone is not lost, cancel it from the phone now: nothing else is needed.".into(),
    );
    out
}

/// One line per recovery state for `sync-recovery status`, `sync-status` and the TUI.
pub(crate) fn recovery_lines(store: &LauncherStore, op: &Op, now: i64) -> Vec<String> {
    let verdict = crate::sync::recovery::validate_phone_reenroll(store, op, now);
    match verdict {
        Ok(v) => vec![format!(
            "recovery {}: {v:?} ({})",
            short(&op.hash()),
            verdict_hint(&v)
        )],
        Err(e) => vec![format!("recovery {}: invalid: {e:#}", short(&op.hash()))],
    }
}

/// Every recovery in the log, plus those still waiting for review on this device.
pub(crate) fn all_recoveries(store: &LauncherStore) -> Result<Vec<Op>> {
    let mut ops = crate::sync::quorum::ops_of(store, "PhoneReenroll", None)?;
    for p in store.sync_pending()? {
        if matches!(p.body, OpBody::PhoneReenroll { .. })
            && !ops.iter().any(|o| o.hash() == p.hash())
        {
            ops.push(p);
        }
    }
    Ok(ops)
}

/// The factor proof body for `op` from what the user typed: the 24 paper words or the emailed code.
fn factor_proof(store: &LauncherStore, kind: FactorKind, secret: &str, op: &Op) -> Result<OpBody> {
    Ok(match kind {
        FactorKind::Paper => {
            let recover = paper_derive(secret, PaperDomain::Recover).context("the paper phrase")?;
            ceremony::red_factor_paper(&recover, op)
        }
        FactorKind::Email => ceremony::red_factor_email(store, secret.trim(), op)?,
    })
}

/// Start a recovery. `paper_phrase` authorizes it by the paper (then the third factor is the
/// emailed code); without it two other attended devices authorize and the third factor is the
/// paper. The third factor is PROVEN here, over the new op: `third_secret` is the words or the
/// code. Without it the recovery stays `AwaitingFactor` (and says so) until `sync-factor`.
pub(crate) fn recovery_start(
    store: &LauncherStore,
    id: &DeviceIdentity,
    now: i64,
    paper_phrase: Option<&str>,
    third_secret: Option<&str>,
) -> Result<Report> {
    use crate::sync::recovery as rec;
    let recover = paper_phrase
        .map(|p| paper_derive(p, PaperDomain::Recover))
        .transpose()?;
    // Design 5.2: the paper authorizes, so the third factor is the emailed code, except in a ring of one computer
    // whose emailed code could only come from this computer itself: there the paper stands for both.
    let me = id.public().device_id();
    let paper_is_both = recover.is_some()
        && rec::is_solo(store, &me)?
        && crate::sync::ceremony::email_factor_sender(store)?.is_none_or(|s| s == me);
    let (third, kind) = if recover.is_some() && !paper_is_both {
        (ThirdFactor::EmailCode, FactorKind::Email)
    } else {
        (ThirdFactor::Paper, FactorKind::Paper)
    };
    let body = rec::start_body(store, id, now, recover.as_ref(), third)?;
    let op = store.sync_append_local(id, body)?;
    let mut lines = vec![if paper_is_both {
        format!(
            "recovery started (change {}); this is the only computer, so the paper phrase is also the third proof; run `sshub sync-recovery ack {}` here to start the 14 day waiting period",
            short(&op.hash()),
            short(&op.hash())
        )
    } else {
        format!(
            "recovery started (change {}); the 7 day waiting period runs once another device has seen it",
            short(&op.hash())
        )
    }];
    if let Some(secret) = third_secret.filter(|s| !s.trim().is_empty()) {
        let proof = factor_proof(store, kind, secret, &op)?;
        store.sync_append_local(id, proof)?;
    } else {
        lines.push(format!(
            "the {} is not proven yet: run `sshub sync-factor {} --{}`",
            if kind == FactorKind::Paper {
                "paper phrase"
            } else {
                "emailed code"
            },
            short(&op.hash()),
            if kind == FactorKind::Paper {
                "paper"
            } else {
                "email-code"
            }
        ));
    }
    lines.extend(recovery_lines(store, &op, now));
    Ok(Report {
        lines,
        json: json!({ "op": hex(&op.hash()) }),
    })
}

/// Cancel a recovery from a device: the cancellation carries the third factor and the proof of it
/// over ITS OWN hash. A wrong paper phrase or code is refused before anything is written where it
/// can be checked (the code), and the report says whether the recovery really is cancelled.
pub(crate) fn recovery_cancel(
    store: &LauncherStore,
    id: &DeviceIdentity,
    op_prefix: &str,
    kind: FactorKind,
    secret: &str,
    now: i64,
) -> Result<Report> {
    use crate::sync::recovery as rec;
    ensure!(
        !secret.trim().is_empty(),
        "a cancellation from a device needs the paper phrase or the emailed code"
    );
    let target = find_op(store, op_prefix)?;
    let f = match kind {
        FactorKind::Paper => ThirdFactor::Paper,
        FactorKind::Email => ThirdFactor::EmailCode,
    };
    if kind == FactorKind::Email {
        // The code can be checked now: a wrong one writes nothing.
        ceremony::red_factor_email(store, secret.trim(), &target)?;
    }
    let cancel = store.sync_append_local(id, rec::cancel_body(&target, false, Some(f))?)?;
    let proof = factor_proof(store, kind, secret, &cancel)?;
    store.sync_append_local(id, proof)?;
    let verdict = rec::validate_phone_reenroll(store, &target, now)?;
    let cancelled = verdict == rec::ReenrollVerdict::CancelledByFactor;
    let mut lines = vec![if cancelled {
        format!(
            "cancelled recovery {} (change {})",
            short(&target.hash()),
            short(&cancel.hash())
        )
    } else {
        format!(
            "the cancellation (change {}) did not take effect: the recovery is {verdict:?} (a wrong paper phrase, or this device is not a full device with a person at the keyboard that is in sync)",
            short(&cancel.hash())
        )
    }];
    lines.extend(recovery_lines(store, &target, now));
    Ok(Report {
        lines,
        json: json!({ "op": hex(&target.hash()), "cancel": hex(&cancel.hash()), "cancelled": cancelled }),
    })
}

fn recovery(
    store: &LauncherStore,
    id: &DeviceIdentity,
    rc: RecoveryCmd,
    opts: Opts,
    now: i64,
) -> Result<i32> {
    use crate::sync::recovery as rec;
    match rc {
        RecoveryCmd::Start { paper } => {
            let phrase = if paper {
                Some(
                    super::unlock::prompt_secret("Paper phrase (24 words): ")
                        .context("no phrase entered")?,
                )
            } else {
                None
            };
            // The third factor is proven now: the paper when two devices authorize, the emailed
            // code when the paper did. Empty input leaves the recovery awaiting it.
            let third = super::unlock::prompt_secret(if paper {
                "Emailed code, or the paper phrase again when this is the only computer (empty: prove it later with sync-factor): "
            } else {
                "Paper phrase to prove it now (empty: prove it later with sync-factor): "
            });
            let r = recovery_start(
                store,
                id,
                now,
                phrase.as_deref().map(String::as_str),
                third.as_deref().map(String::as_str),
            )?;
            report(opts, &r)
        }
        RecoveryCmd::Approve(op_id) => {
            let op = find_op(store, &op_id)?;
            let v = store.sync_append_local(id, rec::approve_body(&op)?)?;
            report(
                opts,
                &Report {
                    lines: vec![format!(
                        "approved recovery {} (vote {})",
                        short(&op.hash()),
                        short(&v.hash())
                    )],
                    json: json!({ "op": hex(&op.hash()), "vote": hex(&v.hash()) }),
                },
            )
        }
        RecoveryCmd::Ack(op_id) => {
            let op = find_op(store, &op_id)?;
            let a = store.sync_append_local(id, rec::ack_body(&op, now)?)?;
            report(
                opts,
                &Report {
                    lines: vec![format!(
                        "acknowledged recovery {} (record {})",
                        short(&op.hash()),
                        short(&a.hash())
                    )],
                    json: json!({ "op": hex(&op.hash()), "ack": hex(&a.hash()) }),
                },
            )
        }
        RecoveryCmd::Cancel { op, factor } => {
            let secret = super::unlock::prompt_secret(match factor {
                FactorKind::Paper => "Paper phrase (24 words): ",
                FactorKind::Email => "Emailed code: ",
            })
            .context("no secret entered")?;
            let r = recovery_cancel(store, id, &op, factor, &secret, now)?;
            report(opts, &r)
        }
        RecoveryCmd::Status => {
            let ops = all_recoveries(store)?;
            let mut lines = Vec::new();
            let mut js = Vec::new();
            for op in &ops {
                let verdict = rec::validate_phone_reenroll(store, op, now)
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_else(|e| format!("invalid: {e:#}"));
                lines.extend(recovery_lines(store, op, now));
                js.push(json!({ "op": hex(&op.hash()), "verdict": verdict }));
            }
            if lines.is_empty() {
                lines.push("no recovery in the log".into());
            }
            report(
                opts,
                &Report {
                    lines,
                    json: json!({ "recoveries": js }),
                },
            )
        }
    }
}

pub(crate) fn mode(
    store: &LauncherStore,
    id: &DeviceIdentity,
    to: CredMode,
    cosigner: Option<&str>,
    phrase: &str,
    ring: &[u8; 16],
    now: i64,
) -> Result<Report> {
    let cosigner = cosigner
        .map(|c| find_device(store, c).map(|d| d.id))
        .transpose()?;
    let me = id.public();
    // Second call, after the ceremony: re-encrypt. Nothing is re-encrypted before the ring agreed.
    let open = |host: &crate::sync::oplog::HostUid| -> Result<Zeroizing<Vec<u8>>> {
        let (ct, _) = store
            .sync_cred_ciphertext(host)?
            .context("credential has no ciphertext")?;
        let dek = crate::sync::quorum::vault_open_dek(store, host, phrase, ring)?;
        Ok(Zeroizing::new(crate::sync::crypto::dek_open(&dek, &ct)?))
    };
    if let Some(mc) = crate::sync::modes::own_mode_change(store, &me.device_id())? {
        if matches!(&mc.body, OpBody::ModeChange { to: t, .. } if *t == to) {
            if !crate::sync::ceremony::in_force(store, &mc)? {
                return op_report(
                    store,
                    &mc,
                    format!(
                        "the switch to {to:?} is still waiting for its approvals (change {}); run this again once they are in",
                        short(&mc.hash())
                    ),
                    now,
                );
            }
            ensure!(
                to == CredMode::Quorum,
                "the other full devices agreed. Finish the switch back to Vault in the terminal screen: sshub, tab 7, M, Switch the credential mode. It asks a second full device (or the phone) to approve each credential, which this command cannot ask."
            );
            let bodies = crate::sync::modes::mode_migration(store, &me, &open)?;
            let n = bodies.len();
            for b in bodies {
                store.sync_append_local(id, b)?;
            }
            return op_report(
                store,
                &mc,
                format!("{n} credentials re-wrapped for {to:?}"),
                now,
            );
        }
    }
    paper_gate(store, to == CredMode::Quorum, 0)?;
    let body = crate::sync::modes::mode_change_body(store, &me, to, cosigner)?;
    let op = store.sync_append_local(id, body)?;
    op_report(
        store,
        &op,
        format!(
            "switch to {to:?} proposed (change {}); {}",
            short(&op.hash()),
            if to == CredMode::Quorum {
                "once the approvals are in, run this again to re-encrypt the credentials"
            } else {
                "once the approvals are in, finish it in the terminal screen (tab 7, M, Switch the credential mode): it asks a second full device or the phone for each credential"
            }
        ),
        now,
    )
}

pub(crate) fn exemption_off(
    store: &LauncherStore,
    id: &DeviceIdentity,
    device: &str,
    now: i64,
) -> Result<Report> {
    let row = find_device(store, device)?;
    ensure!(
        row.exempt,
        "{} has no master phrase exemption",
        short_id(&row.id)
    );
    let op = store.sync_append_local(
        id,
        OpBody::ExemptionSet {
            device_id: row.id.clone(),
            slot: None,
        },
    )?;
    op_report(
        store,
        &op,
        format!(
            "master phrase exemption of {} switched off (change {})",
            short_id(&row.id),
            short(&op.hash())
        ),
        now,
    )
}

/// Whether an export this device proposed can still be written: it is still being approved, or approved and its
/// window (counted from when the ceremony completed) is still open.
pub(crate) fn export_pending(store: &LauncherStore, op: &crate::sync::oplog::Op, now: i64) -> bool {
    let OpBody::BulkExport {
        requester,
        window_secs,
    } = &op.body
    else {
        return false;
    };
    match ceremony::red_status(store, op, now) {
        // Not complete yet: still being approved. Complete: usable until the window after completion ends.
        Ok(st) if !st.complete => true,
        Ok(st) => now < st.complete_at + i64::from(*window_secs) && requester == &op.device_id,
        Err(_) => false,
    }
}

/// Propose the export, wait (up to `wait` seconds) until its ceremony is
/// complete, then export inside the window. Returns the report.
pub(crate) fn export_flow(
    store: &LauncherStore,
    id: &DeviceIdentity,
    window: u32,
    out: &std::path::Path,
    wait: u64,
    clock: &dyn Fn() -> i64,
    unlock: &mut dyn FnMut(&crate::sync::oplog::HostUid) -> Result<Option<String>>,
) -> Result<Report> {
    let me = id.public().device_id();
    // A repeated command continues the open proposal instead of restarting the ceremony.
    let mut open = None;
    for o in crate::sync::quorum::ops_of(store, "BulkExport", Some(&me))?
        .into_iter()
        .rev()
    {
        let same =
            matches!(&o.body, OpBody::BulkExport { window_secs, .. } if *window_secs == window);
        if same && !ceremony::red_status(store, &o, clock())?.expired {
            open = Some(o);
            break;
        }
    }
    let op = match open {
        Some(o) => o,
        None => store.sync_append_local(
            id,
            OpBody::BulkExport {
                requester: me.clone(),
                window_secs: window,
            },
        )?,
    };
    let started = clock();
    // Our own proposal is applied locally at once; the ceremony is what lets the
    // OTHER holders release shares, so wait for it before opening anything.
    loop {
        ceremony::settle_red_ops(store, clock())?;
        let st = ceremony::red_status(store, &op, clock())?;
        if let Some(why) = st.blocked {
            bail!("the export is blocked: {why}");
        }
        if st.expired {
            bail!("the approval round ran out of time");
        }
        if st.complete {
            break;
        }
        if (clock() - started) as u64 >= wait {
            let lines = ceremony_lines(&st);
            bail!(
                "the export is not approved yet (change {}): {}; approve it from the other devices, then run the command again with --wait",
                short(&op.hash()),
                lines.join("; ")
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let r = crate::sync::export::export_all(store, &me, clock, unlock, out)?;
    Ok(Report {
        lines: vec![format!(
            "exported {} credentials ({} without a stored secret) to {}",
            r.written,
            r.skipped,
            out.display()
        )],
        json: json!({ "op": hex(&op.hash()), "written": r.written, "skipped": r.skipped, "out": out.display().to_string() }),
    })
}

fn export(
    paths: &ProfilePaths,
    window: u32,
    out: &std::path::Path,
    wait: u64,
    opts: Opts,
) -> Result<i32> {
    let ctx = super::context::CliContext::bootstrap_with(paths.clone())?;
    let local = crate::credentials::open_ungated_for_profile(paths);
    let id = load_or_init_identity(paths, local.as_ref())?;
    let mut unlock = |uid: &crate::sync::oplog::HostUid| -> Result<Option<String>> {
        let Some(host) = ctx
            .store
            .list_hosts()?
            .into_iter()
            .find(|h| ctx.store.host_uid(h.id).ok().flatten().as_ref() == Some(uid))
        else {
            return Ok(None);
        };
        ctx.ensure_unlocked_for(&host.name, Some(&host), None, None)?;
        ctx.password_store()
            .get(&crate::credentials::host_key(host.id))
    };
    let r = export_flow(
        &ctx.store,
        &id,
        window,
        out,
        wait,
        &super::output::now_ts,
        &mut unlock,
    )?;
    report(opts, &r)
}

pub(crate) fn migrate(
    store: &LauncherStore,
    id: &DeviceIdentity,
    local: &dyn PasswordStore,
    remove_local: bool,
) -> Result<Report> {
    ensure!(!store.sync_devices()?.is_empty(), "{EMPTY_RING}");
    let rep = crate::sync::cred_ops::migrate_local_secrets(store, id, local)?;
    let mut lines = vec![format!(
        "{} secrets written as replicated credentials, {} skipped (nothing to read)",
        rep.migrated.len(),
        rep.skipped.len()
    )];
    if remove_local {
        crate::sync::cred_ops::remove_local_copies(local, &rep)?;
        lines.push("local copies removed".into());
    } else {
        lines.push(
            "local copies kept; run again with --remove-local once the replicated copies open"
                .into(),
        );
    }
    Ok(Report {
        json: json!({ "migrated": rep.migrated.len(), "skipped": rep.skipped.len(), "removed_local": remove_local }),
        lines,
    })
}

pub(crate) fn import_kit(
    store: &LauncherStore,
    local: &dyn PasswordStore,
    kit: &str,
    phrase: &str,
    argon: crate::sync::crypto::Argon2Profile,
) -> Result<Report> {
    let ring = crate::sync::quorum::parse_ring_secret(kit.trim()).context("the ring kit")?;
    crate::sync::crypto::eff_decode(phrase.trim())
        .context("the master phrase is not 6 words of the EFF list")?;
    crate::sync::quorum::vault_setup_with(store, phrase.trim(), &ring, argon)
        .context("the master phrase")?;
    local
        .set(
            crate::sync::runtime::RING_SECRET_KEY,
            &crate::sync::quorum::ring_secret_text(&ring),
        )
        .context("keep the ring secret in the credential store")?;
    Ok(Report {
        lines: vec!["Vault kit imported: this machine can open the ring's credentials with the master phrase.".into()],
        json: json!({ "imported": true }),
    })
}

// --------------------------------------------------------------------- help

pub fn print_help(cmd: &str) {
    let (synopsis, text) = match cmd {
        "sync-revoke" => ("sync-revoke <device> [--format plain|json]", "Remove a device: one approval for a view-only copy, a big change for a full device. Removing a full device needs two computers or servers you control, the one you remove included, so that one other can approve with the phone and the paper phrase (the passwords the removed device held a piece of then stay on an exposure list until you change them). Prints the ceremony status and runs this device's revocation steps (rotate wrap keys, reshare, retire)."),
        "sync-role" => ("sync-role <device> holder|mirror [--format plain|json]", "Promote a mirror to holder or demote a holder (Red)."),
        "sync-approve" => ("sync-approve <op-id> [--format plain|json]", "Approve a pending Red op as this attended device (the second-device vote)."),
        "sync-factor" => ("sync-factor <op-id> (--paper | --email-code) [--format plain|json]", "Add the third factor to a Red op. The 24 paper words or the emailed code are read without echo."),
        "sync-factor-register" => ("sync-factor-register (paper | email) [--sender <device>] [--format plain|json]", "Register a third factor. Paper reads the 24 words without echo; email creates a code and shows it once. The code is kept, sealed, only on its sender, so register it on the device that will mail it (--sender names only this device; another device is refused)."),
        "sync-recovery" => ("sync-recovery (start [--paper] | approve <op-id> | ack <op-id> | cancel <op-id> --factor paper|email | status) [--format plain|json]", "Re-enroll the phone: start, approve or acknowledge as another attended device, cancel, or show the state of every recovery."),
        "sync-mode" => ("sync-mode (vault | quorum) [--cosigner <device>] [--format plain|json]", "Change the credential mode (Red), in two steps: the first call proposes it and shows the ceremony; for Vault to Quorum run it again once the approvals are in to re-encrypt the credentials (the master phrase is read without echo). The switch back to Vault is proposed here with `--cosigner <device>` (a second full device) and finished in the terminal screen (tab 7, M, Switch the credential mode), which asks a second full device or the phone to approve each credential."),
        "sync-exemption" => ("sync-exemption (on | off) <device> [--format plain|json]", "Switch the master-phrase exemption of a device off (Red). Switching it on needs the TUI."),
        "sync-export" => ("sync-export --window <seconds> --out <path> [--wait <seconds>] [--format plain|json]", "Bulk export (Red, window up to 3600 s): propose it, wait for the ceremony, then write every credential to a new 0600 file in the Termius CSV format (import it from a directory as L00t.csv)."),
        "sync-migrate" => ("sync-migrate [--remove-local] [--format plain|json]", "Write this profile's existing host and identity secrets as replicated credentials; --remove-local deletes the local copies afterwards."),
        "sync-heal" => ("sync-heal [--format plain|json]", "After a device was replaced or retired: re-seal the saved passwords that a current full device does not hold a piece of yet, under a fresh key to every current device. The 24 paper words are read once, without echo; this terminal's own piece plus the paper's piece open each one, no phone needed. Passwords a removed device held a piece of are skipped: change those (sync-rekey)."),
        "sync-rekey" => ("sync-rekey [--csv FILE | --stdin] [--new-key HOST]... [--format plain|json]", "For a stolen device: list the saved passwords that share a key with a removed device (no option), or save new secrets: lines `host,newpassword` from --csv FILE or --stdin, and --new-key HOST to generate a new key, install it on the server with the credential the host has now (through `sshub exec`) and only then save it. You remove the old key's line from authorized_keys yourself. SSHub cannot change a password on a server: change it there first. Each host gets its own result line; a host not saved stays on the list."),
        "sync-replace-terminal" => ("sync-replace-terminal <old computer> [--new <computer>] [--format plain|json]", "Replace a full computer in steps: without --new it says how to enroll the new one (sshub sync-join on it); with --new it proposes the new computer as a full device (needs the phone and the paper phrase) and, run again once that is approved, proposes removing the old one. The paper phrase is read once, without echo, and goes to both. The last line points at sync-heal and sync-rekey."),
        "sync-import-kit" => ("sync-import-kit", "On a new machine: read the ring kit (sshub:ring:1:...) and the six master words without echo and set up the Vault here."),
        _ => return,
    };
    println!(
        "sshub {cmd} - {}",
        text.split(". ").next().unwrap_or("").trim_end_matches('.')
    );
    println!();
    println!("USAGE:");
    println!("    sshub [--profile NAME] {synopsis}");
    println!();
    println!("{text}");
}

#[cfg(test)]
mod tests;
