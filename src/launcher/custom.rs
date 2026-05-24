use std::process::Command;

use anyhow::{bail, Context, Result};

use super::TerminalLauncher;
use crate::ssh::{build_ssh_alias_argv, build_ssh_argv, SshHost};

const ALLOWED_PLACEHOLDERS: &[&str] = &[
    "host",
    "user",
    "hostname",
    "port",
    "ssh_command",
    "ssh_args",
];

#[derive(Debug, Clone)]
pub struct CustomLauncher {
    pub template: String,
}

impl TerminalLauncher for CustomLauncher {
    fn launch(&self, host: &SshHost) -> Result<()> {
        let argv = build_argv(&self.template, host)?;
        spawn_argv(&argv)
    }

    fn launch_managed(&self, host: &SshHost) -> Result<()> {
        let ssh_argv = build_ssh_argv(host);
        let argv = if template_supports_managed(&self.template) {
            build_argv_managed(&self.template, host, &ssh_argv)?
        } else {
            replace_trailing_ssh_alias(build_argv(&self.template, host)?, host, &ssh_argv)
        };
        spawn_argv(&argv)
    }

    fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()> {
        let argv = build_argv_from_expanded(&join_shell_words(ssh_argv))?;
        spawn_argv(&argv)
    }
}

fn spawn_argv(argv: &[String]) -> Result<()> {
    if argv.is_empty() {
        bail!("launch command resolved to empty argv");
    }
    Command::new(&argv[0]).args(&argv[1..]).spawn()?;
    Ok(())
}

fn template_supports_managed(template: &str) -> bool {
    template.contains("{ssh_command}") || template.contains("{ssh_args}")
}

fn join_shell_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| {
            if word
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '\\'))
            {
                format!("'{word}'")
            } else {
                word.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn replace_trailing_ssh_alias(
    mut argv: Vec<String>,
    host: &SshHost,
    ssh_argv: &[String],
) -> Vec<String> {
    if argv.len() >= 2 && argv[argv.len() - 2] == "ssh" && argv[argv.len() - 1] == host.name {
        argv.truncate(argv.len() - 2);
        argv.extend(ssh_argv.iter().cloned());
    }
    argv
}

/// Substitute whitelisted placeholders in `template`. Rejects unknown `{name}` tokens.
pub fn apply_template(template: &str, host: &SshHost) -> Result<String> {
    apply_template_with_ssh(template, host, None)
}

fn apply_template_with_ssh(
    template: &str,
    host: &SshHost,
    ssh_argv: Option<&[String]>,
) -> Result<String> {
    validate_placeholders(template)?;

    let hostname = host.hostname.as_deref().unwrap_or(&host.name);
    let user = host.user.as_deref().unwrap_or("");
    let port = host.port.map(|p| p.to_string()).unwrap_or_default();
    let ssh_command = ssh_argv
        .map(join_shell_words)
        .unwrap_or_else(|| join_shell_words(&build_ssh_alias_argv(host)));
    let ssh_args = ssh_argv
        .map(|argv| join_shell_words(&argv[1..]))
        .unwrap_or_else(|| host.name.clone());

    Ok(template
        .replace("{host}", &host.name)
        .replace("{hostname}", hostname)
        .replace("{user}", user)
        .replace("{port}", &port)
        .replace("{ssh_command}", &ssh_command)
        .replace("{ssh_args}", &ssh_args))
}

pub(crate) fn build_argv_managed(
    template: &str,
    host: &SshHost,
    ssh_argv: &[String],
) -> Result<Vec<String>> {
    let expanded = apply_template_with_ssh(template, host, Some(ssh_argv))?;
    if needs_shell(expanded.trim()) {
        validate_shell_safe_host(host)?;
    }
    build_argv_from_expanded(&expanded)
}

pub(crate) fn build_argv(template: &str, host: &SshHost) -> Result<Vec<String>> {
    let expanded = apply_template(template, host)?;
    if needs_shell(expanded.trim()) {
        validate_shell_safe_host(host)?;
    }
    build_argv_from_expanded(&expanded)
}

pub(crate) fn build_argv_from_expanded(expanded: &str) -> Result<Vec<String>> {
    let trimmed = expanded.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if needs_shell(trimmed) {
        return Ok(vec!["sh".into(), "-c".into(), trimmed.into()]);
    }

    split_command_line(trimmed).context("parse launch command")
}

fn validate_placeholders(template: &str) -> Result<()> {
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| anyhow::anyhow!("unclosed placeholder in launch command"))?;
        let name = &after[..end];
        if name.is_empty() {
            bail!("empty placeholder in launch command");
        }
        if !ALLOWED_PLACEHOLDERS.contains(&name) {
            bail!(
                "unknown placeholder {{{name}}} in launch command (allowed: host, user, hostname, port, ssh_command, ssh_args)"
            );
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

fn validate_shell_safe_host(host: &SshHost) -> Result<()> {
    let hostname = host.hostname.as_deref().unwrap_or(&host.name);
    let user = host.user.as_deref().unwrap_or("");
    let port = host.port.map(|p| p.to_string()).unwrap_or_default();

    for (field, value) in [
        ("host", host.name.as_str()),
        ("user", user),
        ("hostname", hostname),
        ("port", port.as_str()),
    ] {
        if value
            .chars()
            .any(|c| matches!(c, ';' | '|' | '&' | '$' | '`' | '\n'))
        {
            bail!("{field} value contains shell metacharacters: {value:?}");
        }
    }
    Ok(())
}

fn needs_shell(line: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                i += 1;
            }
            '"' if !in_single => {
                in_double = !in_double;
                i += 1;
            }
            _ if in_single || in_double => i += 1,
            '&' if i + 1 < bytes.len() && bytes[i + 1] == b'&' => return true,
            '|' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => return true,
            '|' | ';' | '>' | '<' | '`' | '$' => return true,
            _ => i += 1,
        }
    }

    false
}

fn split_command_line(line: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut chars = line.char_indices().peekable();

    while chars.peek().is_some() {
        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let (_, first) = chars.next().expect("peeked char");
        let mut arg = String::new();

        match first {
            '\'' => {
                for (_, c) in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    arg.push(c);
                }
            }
            '"' => {
                while let Some((_, c)) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' if chars.peek().is_some_and(|(_, next)| *next == '"') => {
                            chars.next();
                            arg.push('"');
                        }
                        c => arg.push(c),
                    }
                }
            }
            c => {
                arg.push(c);
                while let Some((_, c)) = chars.peek().copied() {
                    if c.is_whitespace() {
                        break;
                    }
                    chars.next();
                    arg.push(c);
                }
            }
        }

        if !arg.is_empty() {
            args.push(arg);
        }
    }

    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_command_line_respects_quotes() {
        assert_eq!(
            split_command_line(r#"wezterm start -- ssh prod"#).unwrap(),
            vec!["wezterm", "start", "--", "ssh", "prod"]
        );
        assert_eq!(
            split_command_line(r#"sh -c 'kitty -e ssh web'"#).unwrap(),
            vec!["sh", "-c", "kitty -e ssh web"]
        );
    }

    #[test]
    fn needs_shell_detects_operators() {
        assert!(!needs_shell("wezterm start -- ssh host"));
        assert!(needs_shell("echo hi | less"));
        assert!(!needs_shell("sh -c 'echo hi | less'"));
    }

    #[test]
    fn rejects_shell_metacharacters_in_host_fields_when_needs_shell() {
        let mut host = SshHost::new("prod;rm");
        host.hostname = Some("example.com".into());
        let err = build_argv("echo {host} | less", &host).unwrap_err();
        assert!(err.to_string().contains("shell metacharacters"));

        let host = SshHost::new("prod");
        assert!(build_argv("echo {host} | less", &host).is_ok());
    }
}
