//! `sshub audit` subcommands: auth-event audit log listing and stats.

use anyhow::Result;
use serde::Serialize;

use super::CliContext;
use crate::cli::output::{audit_event_json, format_audit_plain, now_ts};
use crate::cli::parse::{
    parse_days, parse_format, parse_limit, take_flag, take_opt, usage, OutputFormat,
};

pub fn run(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(ctx, &args[1..]),
        Some("stats") => cmd_stats(ctx, &args[1..]),
        Some(other) => {
            eprintln!("sshub: unknown audit subcommand '{other}'");
            eprintln!("sshub: try: audit list, audit stats");
            Ok(2)
        }
        None => usage("audit needs a subcommand (list|stats)"),
    }
}

fn cmd_list(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let limit = parse_limit(args, 50).map_err(anyhow::Error::msg)?;

    let mut a = args.to_vec();
    let status = take_opt(&mut a, "--status").unwrap_or_else(|| "all".into());
    let via = take_opt(&mut a, "--via").unwrap_or_else(|| "all".into());
    let host = take_opt(&mut a, "--host");
    let days = take_opt(&mut a, "--days");

    if !valid_status(&status) {
        usage("audit list --status must be one of: all, ok, fail, retry");
    }
    if !valid_via(&via) {
        usage("audit list --via must be one of: all, connect, tunnel, agent");
    }

    let since = match days {
        Some(d) => {
            let n: i64 = d
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid --days '{d}'"))?;
            Some(now_ts() - n * 86400)
        }
        None => None,
    };

    // Pass None for "all" so the store applies no filter (cleaner than the
    // sentinel string, which the store also treats as "all").
    let status_opt = (status != "all").then_some(status.as_str());
    let via_opt = (via != "all").then_some(via.as_str());

    let events =
        ctx.store
            .list_auth_events_cli(status_opt, since, via_opt, host.as_deref(), limit)?;

    match fmt {
        OutputFormat::Plain => {
            for event in &events {
                println!("{}", format_audit_plain(&audit_event_json(event)));
            }
        }
        OutputFormat::Json => {
            let rows: Vec<_> = events.iter().map(audit_event_json).collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
    }
    Ok(0)
}

fn cmd_stats(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let days = parse_days(args, 30).map_err(anyhow::Error::msg)?;

    let mut a = args.to_vec();
    let via = take_opt(&mut a, "--via").unwrap_or_else(|| "all".into());
    let include_retry = take_flag(&mut a, "--include-retry");

    if !valid_via(&via) {
        usage("audit stats --via must be one of: all, connect, tunnel, agent");
    }

    let via_opt = (via != "all").then_some(via.as_str());
    let (ok, fail, retry) = ctx
        .store
        .auth_event_stats_filtered(days, via_opt, include_retry)?;

    match fmt {
        OutputFormat::Plain => {
            println!("ok: {ok}");
            println!("fail: {fail}");
            if include_retry {
                println!("retry: {retry}");
            }
            println!("window: last {days} days");
        }
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct StatsJson {
                ok: i64,
                fail: i64,
                retry: Option<i64>,
                days: i64,
            }
            let out = StatsJson {
                ok,
                fail,
                retry: include_retry.then_some(retry),
                days,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }
    Ok(0)
}

fn valid_status(s: &str) -> bool {
    matches!(s, "all" | "ok" | "fail" | "retry")
}

fn valid_via(s: &str) -> bool {
    matches!(s, "all" | "connect" | "tunnel" | "agent")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_are_validated() {
        for s in ["all", "ok", "fail", "retry"] {
            assert!(valid_status(s));
        }
        for s in ["", "bogus", "launched", "success"] {
            assert!(!valid_status(s));
        }
    }

    #[test]
    fn via_values_are_validated() {
        for s in ["all", "connect", "tunnel", "agent"] {
            assert!(valid_via(s));
        }
        for s in ["", "bogus", "local", "remote"] {
            assert!(!valid_via(s));
        }
    }
}
