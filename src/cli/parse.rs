//! Shared CLI argument parsing helpers (hand-rolled, no clap).

use crate::app::SortMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Plain,
    Json,
}

pub fn parse_format(args: &[String]) -> Result<OutputFormat, String> {
    let mut fmt = OutputFormat::Plain;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                let v = args.get(i + 1).ok_or("--format requires a value")?;
                fmt = match v.as_str() {
                    "plain" => OutputFormat::Plain,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unknown format '{other}'")),
                };
                i += 2;
            }
            _ => i += 1,
        }
    }
    Ok(fmt)
}

pub fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(pos) = args.iter().position(|a| a == flag) {
        args.remove(pos);
        true
    } else {
        false
    }
}

pub fn take_opt(args: &mut Vec<String>, flag: &str) -> Option<String> {
    if let Some(pos) = args.iter().position(|a| a == flag) {
        args.remove(pos);
        args.get(pos).cloned().inspect(|_| {
            args.remove(pos);
        })
    } else {
        None
    }
}

pub fn take_all(args: &mut Vec<String>, flag: &str) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(pos) = args.iter().position(|a| a == flag) {
        args.remove(pos);
        if args.get(pos).is_some() {
            out.push(args.remove(pos));
        }
    }
    out
}

pub fn parse_sort(args: &[String]) -> Result<Option<SortMode>, String> {
    for (i, a) in args.iter().enumerate() {
        if a == "--sort" {
            let v = args.get(i + 1).ok_or("--sort requires a value")?;
            return SortMode::from_cli_str(v)
                .ok_or_else(|| format!("unknown sort mode '{v}'"))
                .map(Some);
        }
    }
    Ok(None)
}

pub fn parse_limit(args: &[String], default: usize) -> Result<usize, String> {
    for (i, a) in args.iter().enumerate() {
        if a == "--limit" {
            let v = args.get(i + 1).ok_or("--limit requires a value")?;
            return v.parse().map_err(|_| format!("invalid limit '{v}'"));
        }
    }
    Ok(default)
}

pub fn parse_days(args: &[String], default: i64) -> Result<i64, String> {
    for (i, a) in args.iter().enumerate() {
        if a == "--days" {
            let v = args.get(i + 1).ok_or("--days requires a value")?;
            return v.parse().map_err(|_| format!("invalid days '{v}'"));
        }
    }
    Ok(default)
}

pub fn positional(args: &[String]) -> Vec<&str> {
    args.iter()
        .filter(|a| !a.starts_with('-'))
        .map(String::as_str)
        .collect()
}

/// Confirmation flag for destructive subcommands (delete, etc.).
pub const CONFIRM_YES: &str = "--yes";

pub fn usage(msg: &str) -> ! {
    eprintln!("sshub: {msg}");
    std::process::exit(2);
}

pub fn fail(msg: &str) -> ! {
    eprintln!("sshub: {msg}");
    std::process::exit(1);
}

pub fn fail_code(msg: &str, code: i32) -> ! {
    eprintln!("sshub: {msg}");
    std::process::exit(code);
}
