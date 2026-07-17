//! Shell completion generators (bash/zsh/fish).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::secure_fs;

use super::context::CliContext;
use super::parse::{take_opt, usage};

const CACHE_HEADER: &str = "# sshub-completion-cache v1";

/// Top-level subcommands (static completion tree).
const TOP_LEVEL: &[&str] = &[
    "host",
    "connect",
    "list",
    "groups",
    "group",
    "identity",
    "tunnel",
    "sftp",
    "audit",
    "tags",
    "sync",
    "import",
    "export",
    "completions",
    "db",
];

const HOST_SUB: &[&str] = &[
    "list",
    "show",
    "connect",
    "resolve",
    "search",
    "add",
    "edit",
    "rename",
    "delete",
    "duplicate",
];

const GROUP_SUB: &[&str] = &["list", "show", "add", "edit", "delete"];

const IDENTITY_SUB: &[&str] = &["list", "show", "add", "edit", "delete", "agent-remove"];

const TUNNEL_SUB: &[&str] = &["list", "show", "create", "delete", "start", "stop"];

const SFTP_SUB: &[&str] = &["ls", "get", "put", "rm", "mkdir", "rename", "chmod"];

const AUDIT_SUB: &[&str] = &["list", "stats"];

const DB_SUB: &[&str] = &["purge"];

const COMPLETIONS_SUB: &[&str] = &["bash", "zsh", "fish"];

pub fn run(_ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let shell = match rest.first() {
        Some(s) => s.to_string(),
        None => usage("completions requires a shell (bash|zsh|fish)"),
    };

    if !matches!(shell.as_str(), "bash" | "zsh" | "fish") {
        eprintln!("sshub: unknown completions shell '{shell}'");
        eprintln!("       try: sshub completions bash|zsh|fish");
        return Ok(2);
    }
    rest.remove(0);

    let cache_path = take_opt(&mut rest, "--cache");
    let host_names = load_host_names(cache_path.as_deref())?;

    if let Some(path) = cache_path {
        write_cache(&path, &host_names)?;
    }

    let script = match shell.as_str() {
        "bash" => render_bash(&host_names),
        "zsh" => render_zsh(&host_names),
        "fish" => render_fish(&host_names),
        _ => unreachable!(),
    };
    print!("{script}");
    Ok(0)
}

fn load_host_names(cache_path: Option<&str>) -> Result<Vec<String>> {
    if let Some(path) = cache_path {
        if Path::new(path).exists() {
            if let Ok(names) = read_cache(path) {
                return Ok(names);
            }
        }
    }
    fetch_host_names_via_subprocess()
}

fn fetch_host_names_via_subprocess() -> Result<Vec<String>> {
    let exe = std::env::current_exe().context("resolve sshub executable path")?;
    let output = Command::new(&exe)
        .args(["host", "list", "--format", "json"])
        .output()
        .context("spawn sshub host list for completions")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_host_list_json(&stdout)
}

fn parse_host_list_json(stdout: &str) -> Result<Vec<String>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).unwrap_or(serde_json::Value::Null);
    let mut names = Vec::new();
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(n) = item.get("name").and_then(|v| v.as_str()) {
                names.push(n.to_string());
            } else if let Some(n) = item.as_str() {
                names.push(n.to_string());
            }
        }
    } else if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                names.push(s.to_string());
            }
        }
    }
    Ok(names)
}

fn write_cache(path: &str, names: &[String]) -> Result<()> {
    let p = PathBuf::from(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create cache directory {}", parent.display()))?;
            secure_fs::restrict_dir(parent);
        }
    }
    let mut body = String::from(CACHE_HEADER);
    body.push('\n');
    for n in names {
        body.push_str(n);
        body.push('\n');
    }
    fs::write(&p, &body).with_context(|| format!("write completion cache {}", p.display()))?;
    secure_fs::restrict_file(&p);
    Ok(())
}

fn read_cache(path: &str) -> Result<Vec<String>> {
    let content = fs::read_to_string(path).with_context(|| format!("read cache {path}"))?;
    Ok(content
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

fn words_csv(items: &[&str]) -> String {
    items.join(" ")
}

fn names_csv(names: &[String]) -> String {
    names.join(" ")
}

fn render_bash(host_names: &[String]) -> String {
    format!(
        r#"# sshub bash completion
_sshub_completions() {{
    local cur prev
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    prev="${{COMP_WORDS[COMP_CWORD-1]}}"

    local top="{top}"
    local host_sub="{host_sub}"
    local group_sub="{group_sub}"
    local identity_sub="{identity_sub}"
    local tunnel_sub="{tunnel_sub}"
    local sftp_sub="{sftp_sub}"
    local audit_sub="{audit_sub}"
    local db_sub="{db_sub}"
    local completions_sub="{completions_sub}"
    local hosts="{hosts}"

    if (( COMP_CWORD == 1 )); then
        COMPREPLY=( $(compgen -W "$top" -- "$cur") )
        return
    fi

    case "${{COMP_WORDS[1]}}" in
        host|connect)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$host_sub" -- "$cur") )
            elif (( COMP_CWORD >= 3 )) && [[ "${{COMP_WORDS[2]}}" == @(connect|show|resolve|delete|duplicate) ]]; then
                COMPREPLY=( $(compgen -W "$hosts" -- "$cur") )
            fi
            ;;
        list)
            COMPREPLY=( $(compgen -W "--tag --group --sort --format plain json" -- "$cur") )
            ;;
        groups|group)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$group_sub" -- "$cur") )
            fi
            ;;
        identity)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$identity_sub" -- "$cur") )
            fi
            ;;
        tunnel)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$tunnel_sub" -- "$cur") )
            fi
            ;;
        sftp)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$sftp_sub" -- "$cur") )
            elif (( COMP_CWORD == 3 )); then
                COMPREPLY=( $(compgen -W "$hosts" -- "$cur") )
            fi
            ;;
        audit)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$audit_sub" -- "$cur") )
            fi
            ;;
        db)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$db_sub" -- "$cur") )
            fi
            ;;
        completions)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=( $(compgen -W "$completions_sub" -- "$cur") )
            fi
            ;;
        tags|sync|import|export)
            COMPREPLY=( $(compgen -W "--format plain json --stdout -o --cache" -- "$cur") )
            ;;
    esac
}}
complete -F _sshub_completions sshub
"#,
        top = words_csv(TOP_LEVEL),
        host_sub = words_csv(HOST_SUB),
        group_sub = words_csv(GROUP_SUB),
        identity_sub = words_csv(IDENTITY_SUB),
        tunnel_sub = words_csv(TUNNEL_SUB),
        sftp_sub = words_csv(SFTP_SUB),
        audit_sub = words_csv(AUDIT_SUB),
        db_sub = words_csv(DB_SUB),
        completions_sub = words_csv(COMPLETIONS_SUB),
        hosts = names_csv(host_names),
    )
}

fn render_zsh(host_names: &[String]) -> String {
    format!(
        r#"#compdef sshub
# sshub zsh completion

local -a top_cmds host_cmds group_cmds identity_cmds tunnel_cmds sftp_cmds audit_cmds db_cmds completion_cmds hosts

top_cmds=({top})
host_cmds=({host_sub})
group_cmds=({group_sub})
identity_cmds=({identity_sub})
tunnel_cmds=({tunnel_sub})
sftp_cmds=({sftp_sub})
audit_cmds=({audit_sub})
db_cmds=({db_sub})
completion_cmds=({completions_sub})
hosts=({hosts})

_sshub() {{
    local curcontext="$curcontext" state line
    typeset -A opt_args

    _arguments -C \
        '1:command:->cmd' \
        '*::arg:->args'

    case $state in
        cmd)
            _describe 'command' top_cmds
            ;;
        args)
            case $words[2] in
                host|connect)
                    if (( CURRENT == 3 )); then
                        _describe 'subcommand' host_cmds
                    elif (( CURRENT >= 4 )) && [[ $words[3] == (connect|show|resolve|delete|duplicate) ]]; then
                        _describe 'host' hosts
                    fi
                    ;;
                groups|group)
                    (( CURRENT == 3 )) && _describe 'subcommand' group_cmds
                    ;;
                identity)
                    (( CURRENT == 3 )) && _describe 'subcommand' identity_cmds
                    ;;
                tunnel)
                    (( CURRENT == 3 )) && _describe 'subcommand' tunnel_cmds
                    ;;
                sftp)
                    if (( CURRENT == 3 )); then
                        _describe 'subcommand' sftp_cmds
                    elif (( CURRENT == 4 )); then
                        _describe 'host' hosts
                    fi
                    ;;
                audit)
                    (( CURRENT == 3 )) && _describe 'subcommand' audit_cmds
                    ;;
                db)
                    (( CURRENT == 3 )) && _describe 'subcommand' db_cmds
                    ;;
                completions)
                    (( CURRENT == 3 )) && _describe 'shell' completion_cmds
                    ;;
            esac
            ;;
    esac
}}

_sshub "$@"
"#,
        top = TOP_LEVEL.join(" "),
        host_sub = HOST_SUB.join(" "),
        group_sub = GROUP_SUB.join(" "),
        identity_sub = IDENTITY_SUB.join(" "),
        tunnel_sub = TUNNEL_SUB.join(" "),
        sftp_sub = SFTP_SUB.join(" "),
        audit_sub = AUDIT_SUB.join(" "),
        db_sub = DB_SUB.join(" "),
        completions_sub = COMPLETIONS_SUB.join(" "),
        hosts = host_names.join(" "),
    )
}

fn render_fish(host_names: &[String]) -> String {
    let mut out = String::from("# sshub fish completion\n\n");
    out.push_str("complete -c sshub -f\n\n");

    for cmd in TOP_LEVEL {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_use_subcommand' -a '{cmd}'\n"
        ));
    }

    for sub in HOST_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from host connect' -a '{sub}'\n"
        ));
    }
    for sub in GROUP_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from groups group' -a '{sub}'\n"
        ));
    }
    for sub in IDENTITY_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from identity' -a '{sub}'\n"
        ));
    }
    for sub in TUNNEL_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from tunnel' -a '{sub}'\n"
        ));
    }
    for sub in SFTP_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from sftp' -a '{sub}'\n"
        ));
    }
    for sub in AUDIT_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from audit' -a '{sub}'\n"
        ));
    }
    for sub in DB_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from db' -a '{sub}'\n"
        ));
    }
    for sub in COMPLETIONS_SUB {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from completions' -a '{sub}'\n"
        ));
    }

    for name in host_names {
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from host connect; and __fish_seen_subcommand_from connect show resolve delete duplicate' -a '{name}'\n"
        ));
        out.push_str(&format!(
            "complete -c sshub -n '__fish_seen_subcommand_from sftp; and not __fish_seen_subcommand_from ls get put rm mkdir rename chmod' -a '{name}'\n"
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.cache");
        let names = vec!["web".into(), "db".into()];
        write_cache(path.to_str().unwrap(), &names).unwrap();
        let read = read_cache(path.to_str().unwrap()).unwrap();
        assert_eq!(read, names);
    }

    #[test]
    fn bash_script_contains_top_level() {
        let script = render_bash(&["prod".into()]);
        assert!(script.contains("host"));
        assert!(script.contains("group"));
        assert!(script.contains("prod"));
    }
}
