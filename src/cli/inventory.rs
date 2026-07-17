//! Inventory and config commands: tags, sync, import, export.

use anyhow::{Context, Result};

use crate::secure_fs;
use crate::ssh::{
    export_launcher_hosts_to, import_ssh_config, render_launcher_hosts, sync_ssh_config_hosts,
    ImportReport,
};
use std::collections::BTreeSet;

use super::context::CliContext;
use super::parse::{fail, parse_format, take_flag, take_opt, OutputFormat};

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

pub fn run_import(ctx: &mut CliContext, _args: &[String]) -> Result<i32> {
    let report = import_ssh_config(&ctx.resolver, &ctx.store, ctx.metadata.as_ref())?;
    print_import_report(&report);
    ctx.reload_hosts()?;
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
}
