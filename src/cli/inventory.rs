//! Inventory and config commands: tags, sync, import, export.

use anyhow::{Context, Result};

use crate::secure_fs;
use crate::ssh::{
    export_launcher_hosts_to, import_ssh_config, render_launcher_hosts, sync_ssh_config_hosts,
    ImportReport,
};
use std::collections::BTreeSet;

use std::path::PathBuf;

use super::context::CliContext;
use super::parse::{fail, parse_format, positional, take_flag, take_opt, usage, OutputFormat};

pub fn run_tags(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let mut tags: BTreeSet<String> = BTreeSet::new();
    for entry in &ctx.hosts {
        for t in entry.tags() {
            tags.insert(t.clone());
        }
    }

    match fmt {
        OutputFormat::Plain => {
            for t in &tags {
                println!("{t}");
            }
        }
        OutputFormat::Json => {
            let list: Vec<&str> = tags.iter().map(String::as_str).collect();
            println!("{}", serde_json::to_string(&list)?);
        }
    }
    Ok(0)
}

pub fn run_sync(ctx: &mut CliContext, _args: &[String]) -> Result<i32> {
    let updated = sync_ssh_config_hosts(&ctx.resolver, &ctx.store)?;
    println!("updated {updated} ssh_config host(s)");
    ctx.reload_hosts()?;
    Ok(0)
}

pub fn run_import(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let dry_run = take_flag(&mut rest, "--dry-run");
    let from = take_opt(&mut rest, "--from").unwrap_or_else(|| "ssh".into());
    let path = positional(&rest).first().map(|s| s.to_string());

    match from.as_str() {
        "ssh" => {
            if dry_run {
                fail("import: --dry-run is not supported for --from ssh");
            }
            let report = import_ssh_config(&ctx.resolver, &ctx.store, ctx.metadata.as_ref())?;
            print_import_report(&report);
            ctx.reload_hosts()?;
        }
        "termius" => {
            let dir = match path {
                Some(p) => PathBuf::from(p),
                None => match crate::import::termius_csv::default_export_dir() {
                    Some(d) => d,
                    None => fail(
                        "import: termius needs a PATH to the export directory (containing L00t.csv)",
                    ),
                },
            };
            if dry_run {
                let loot = dir.join("L00t.csv");
                let content = std::fs::read_to_string(&loot)
                    .with_context(|| format!("reading {}", loot.display()))?;
                let preview: Vec<(String, String, String, u16)> =
                    crate::import::termius_csv::parse_loot_csv(&content)
                        .into_iter()
                        .filter_map(|r| {
                            let name = if r.label.is_empty() {
                                r.host.clone()
                            } else {
                                r.label.clone()
                            };
                            // Mirror import_csv_export: it skips only when the
                            // computed name is empty (never on an empty host
                            // alone), so the preview must not either.
                            if name.is_empty() {
                                return None;
                            }
                            Some((name, r.username, r.host, r.port))
                        })
                        .collect();
                print_dry_run_preview(&preview);
            } else {
                let report = crate::import::termius_csv::import_csv_export(
                    &dir,
                    &ctx.store,
                    ctx.password_store.as_ref(),
                )?;
                print_csv_import_report(&report);
                ctx.reload_hosts()?;
            }
        }
        "putty" => {
            let path = match path {
                Some(p) => PathBuf::from(p),
                None => {
                    let default = crate::ssh::expand_tilde("~/.putty/sessions");
                    if default.is_dir() {
                        default
                    } else {
                        fail(
                            "import: putty needs a PATH to a .reg file or sessions directory \
                             (~/.putty/sessions not found)",
                        );
                    }
                }
            };
            if dry_run {
                let parse = if path.is_dir() {
                    crate::import::putty::parse_sessions_dir(&path)
                } else {
                    let bytes = std::fs::read(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    crate::import::putty::parse_reg(&crate::import::putty::decode_reg_bytes(&bytes))
                };
                let preview: Vec<(String, String, String, u16)> = parse
                    .hosts
                    .into_iter()
                    .map(|h| (h.name, h.username, h.hostname, h.port))
                    .collect();
                print_dry_run_preview(&preview);
            } else {
                let report = crate::import::putty::import_putty(&path, &ctx.store)?;
                print_host_import_report(&report);
                ctx.reload_hosts()?;
            }
        }
        "mremoteng" => {
            let path = match path {
                Some(p) => PathBuf::from(p),
                None => fail("import: mremoteng needs a PATH to confCons.xml"),
            };
            if dry_run {
                let xml = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let preview: Vec<(String, String, String, u16)> =
                    crate::import::mremoteng::parse_conf_cons(&xml)
                        .hosts
                        .into_iter()
                        .map(|h| (h.name, h.username, h.hostname, h.port))
                        .collect();
                print_dry_run_preview(&preview);
            } else {
                let report = crate::import::mremoteng::import_mremoteng(&path, &ctx.store)?;
                print_host_import_report(&report);
                ctx.reload_hosts()?;
            }
        }
        _ => {
            usage(&format!(
                "import: unknown source '{from}' (use ssh|termius|putty|mremoteng)"
            ));
        }
    }
    Ok(0)
}

pub fn run_export(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let stdout = take_flag(&mut rest, "--stdout");
    let out_path = take_opt(&mut rest, "-o").or_else(|| take_opt(&mut rest, "--output"));

    if stdout && out_path.is_some() {
        fail("export: use either --stdout or -o PATH, not both");
    }

    if stdout {
        let content = render_launcher_hosts(&ctx.store)?;
        print!("{content}");
        return Ok(0);
    }

    let path = match out_path {
        Some(p) => std::path::PathBuf::from(p),
        None => crate::ssh::exported_conf_path()?,
    };

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create export directory {}", parent.display()))?;
            secure_fs::restrict_dir(parent);
        }
    }

    let written = export_launcher_hosts_to(&ctx.store, &path)?;
    println!("exported launcher hosts to {}", written.display());
    Ok(0)
}

fn print_import_report(report: &ImportReport) {
    println!(
        "imported: {} inserted, {} updated, {} skipped (launcher), {} failed",
        report.inserted, report.updated, report.skipped_launcher, report.failed
    );
}

/// Summary line for a third-party host import (PuTTY / mRemoteNG).
fn print_host_import_report(r: &crate::import::HostImportReport) {
    println!(
        "imported: {} host(s), {} skipped (already exist), {} skipped (non-ssh)",
        r.imported, r.skipped_existing, r.skipped_non_ssh
    );
}

/// Summary line for a Termius CSV export import.
fn print_csv_import_report(r: &crate::import::termius_csv::CsvImportReport) {
    println!(
        "imported: {} host(s), {} identity(ies) created, {} skipped (already exist)",
        r.hosts_imported, r.identities_created, r.skipped
    );
}

/// Preview printer for `--dry-run`: one `  name  user@host:port` line per host
/// plus a trailing count. Writes nothing to the store.
fn print_dry_run_preview(hosts: &[(String, String, String, u16)]) {
    for (name, user, host, port) in hosts {
        if user.is_empty() {
            println!("  {name}  {host}:{port}");
        } else {
            println!("  {name}  {user}@{host}:{port}");
        }
    }
    println!(
        "{} host(s) would be imported (dry run, nothing written)",
        hosts.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_report_format() {
        let report = ImportReport {
            inserted: 2,
            updated: 1,
            skipped_launcher: 5,
            failed: 0,
        };
        // smoke: formatting doesn't panic
        print_import_report(&report);
    }

    #[test]
    fn host_import_report_format() {
        // smoke: PuTTY / mRemoteNG summary formatting doesn't panic.
        let report = crate::import::HostImportReport {
            imported: 3,
            skipped_existing: 1,
            skipped_non_ssh: 2,
        };
        print_host_import_report(&report);
    }

    #[test]
    fn csv_import_report_format() {
        // smoke: Termius CSV summary formatting doesn't panic.
        let report = crate::import::termius_csv::CsvImportReport {
            hosts_imported: 4,
            identities_created: 2,
            skipped: 1,
            ..Default::default()
        };
        print_csv_import_report(&report);
    }

    #[test]
    fn dry_run_preview_format() {
        // A host with and without a username exercise both preview branches.
        // Unknown `--from` is intentionally not tested here: the dispatch calls
        // `usage()`, which exits the process and would abort the test runner.
        let hosts = vec![
            (
                "web".to_string(),
                "admin".to_string(),
                "10.0.0.1".to_string(),
                22u16,
            ),
            (
                "db".to_string(),
                String::new(),
                "10.0.0.2".to_string(),
                2222u16,
            ),
        ];
        print_dry_run_preview(&hosts);
    }
}
