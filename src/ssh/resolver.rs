use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use super::host::SshHost;

/// Resolves SSH config hosts.
pub trait HostResolver: Send + Sync {
    fn list_hosts(&self) -> Result<Vec<String>>;
    fn resolve_host(&self, name: &str) -> Result<SshHost>;
}

/// Default resolver using `~/.ssh/config` (or `SSHUB_SSH_CONFIG` / `SSH_LAUNCHER_SSH_CONFIG`) and `ssh -G`.
#[derive(Debug, Clone)]
pub struct SshConfigResolver {
    config_path: PathBuf,
}

impl Default for SshConfigResolver {
    fn default() -> Self {
        Self {
            config_path: ssh_config_path().unwrap_or_else(|_| expand_tilde("~/.ssh/config")),
        }
    }
}

impl SshConfigResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config_path(path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: path.into(),
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

impl HostResolver for SshConfigResolver {
    fn list_hosts(&self) -> Result<Vec<String>> {
        if !self.config_path.exists() {
            return Ok(Vec::new());
        }
        let mut hosts = Vec::new();
        collect_host_aliases(&self.config_path, &mut hosts, 0);
        Ok(hosts)
    }

    fn resolve_host(&self, name: &str) -> Result<SshHost> {
        let output = Command::new("ssh")
            .arg("-F")
            .arg(&self.config_path)
            .args(["-G", name])
            .output()
            .with_context(|| format!("failed to run ssh -G for host {name}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ssh -G failed for {name}: {stderr}");
        }

        let stdout =
            String::from_utf8(output.stdout).context("ssh -G output is not valid UTF-8")?;
        Ok(parse_ssh_g_output(name, &stdout))
    }
}

/// Expand leading `~` using `$HOME`.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(path));
    }
    PathBuf::from(path)
}

/// Resolve SSH config path from `SSHUB_SSH_CONFIG` (or `SSH_LAUNCHER_SSH_CONFIG`) or `~/.ssh/config`.
pub fn ssh_config_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("SSHUB_SSH_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("SSH_LAUNCHER_SSH_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    Ok(expand_tilde("~/.ssh/config"))
}

/// Parse `Host` aliases from ssh config content.
///
/// Excludes `Host *`, `Host !*`, negated patterns, and wildcard aliases.
pub fn parse_host_aliases(config: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in config.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // ssh_config keywords are case-insensitive and may be separated by
        // any whitespace (spaces or tabs), or '='.
        let mut it = line.splitn(2, |c: char| c.is_whitespace() || c == '=');
        let keyword = it.next().unwrap_or("");
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        let Some(rest) = it.next() else {
            continue;
        };
        for alias in rest.split_whitespace() {
            if is_listable_host_alias(alias) {
                hosts.push(alias.to_string());
            }
        }
    }
    hosts
}

/// Collect `Host` aliases from `path`, following `Include` directives (the
/// plain [`parse_host_aliases`] only sees the top-level file). Bounded recursion
/// guards against Include cycles.
fn collect_host_aliases(path: &Path, out: &mut Vec<String>, depth: u8) {
    const MAX_DEPTH: u8 = 16;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    for raw in content.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(2, |c: char| c.is_whitespace() || c == '=');
        let keyword = it.next().unwrap_or("");
        let Some(rest) = it.next() else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("host") {
            for alias in rest.split_whitespace() {
                if is_listable_host_alias(alias) {
                    out.push(alias.to_string());
                }
            }
        } else if keyword.eq_ignore_ascii_case("include") {
            for token in rest.split_whitespace() {
                for inc in expand_include_token(token, base_dir) {
                    collect_host_aliases(&inc, out, depth + 1);
                }
            }
        }
    }
}

/// Resolve one `Include` token to concrete file paths: expand `~`, resolve a
/// relative path against `base_dir`, and expand a `*`/`?` glob in the final
/// path component (the common `Include config.d/*.conf` shape).
fn expand_include_token(token: &str, base_dir: &Path) -> Vec<PathBuf> {
    let expanded = expand_tilde(token);
    let path = if expanded.is_absolute() {
        expanded
    } else {
        base_dir.join(expanded)
    };

    let has_glob = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains('*') || n.contains('?'));
    if !has_glob {
        return vec![path];
    }

    let (Some(parent), Some(pattern)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| wildcard_match(pattern, n))
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// Minimal shell-style wildcard match supporting `*` (any run) and `?` (one).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // DP over (pattern index, text index).
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=t.len() {
            dp[i][j] = match p[i - 1] {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                c => dp[i - 1][j - 1] && c == t[j - 1],
            };
        }
    }
    dp[p.len()][t.len()]
}

fn is_listable_host_alias(alias: &str) -> bool {
    if alias == "*" || alias == "!*" {
        return false;
    }
    if alias.starts_with('!') {
        return false;
    }
    if alias.contains('*') || alias.contains('?') {
        return false;
    }
    true
}

/// Parse `ssh -G` stdout into [`SshHost`].
pub fn parse_ssh_g_output(name: &str, output: &str) -> SshHost {
    let mut host = SshHost::new(name);
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "hostname" => host.hostname = Some(value.to_string()),
            "user" => host.user = Some(value.to_string()),
            "port" => host.port = value.parse().ok(),
            "proxyjump" if value != "none" => host.proxy_jump = Some(value.to_string()),
            "identityfile" if host.identity_file.is_none() => {
                host.identity_file = Some(value.to_string());
            }
            "forwardagent" => {
                host.forward_agent = Some(value.eq_ignore_ascii_case("yes"));
            }
            "remotecommand" if value != "none" => {
                host.remote_command = Some(value.to_string());
            }
            _ => {}
        }
    }
    host
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    #[test]
    fn expand_tilde_replaces_home_prefix() {
        std::env::set_var("HOME", "/tmp/test-home");
        assert_eq!(
            expand_tilde("~/.ssh/config"),
            PathBuf::from("/tmp/test-home/.ssh/config")
        );
    }

    #[test]
    fn parse_host_aliases_reads_fixture() {
        let config = fs::read_to_string(fixture("tests/fixtures/ssh_config")).unwrap();
        let hosts = parse_host_aliases(&config);
        assert_eq!(
            hosts,
            vec![
                "dev-local".to_string(),
                "staging-app".to_string(),
                "prod-db-01".to_string(),
            ]
        );
    }

    #[test]
    fn list_hosts_follows_include_directives_and_globs() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let inc_dir = dir.path().join("config.d");
        std::fs::create_dir(&inc_dir).unwrap();

        // Top-level config: one direct host + a glob include + a relative include.
        let top = dir.path().join("config");
        write!(
            std::fs::File::create(&top).unwrap(),
            "Host direct\n    HostName 10.0.0.1\nInclude config.d/*.conf\nInclude extra.conf\n"
        )
        .unwrap();
        write!(
            std::fs::File::create(inc_dir.join("work.conf")).unwrap(),
            "Host work\n    HostName 10.0.0.5\n"
        )
        .unwrap();
        write!(
            std::fs::File::create(dir.path().join("extra.conf")).unwrap(),
            "Host extra\n    HostName 10.0.0.9\n"
        )
        .unwrap();

        let resolver = SshConfigResolver::with_config_path(top);
        let mut hosts = resolver.list_hosts().unwrap();
        hosts.sort();
        assert_eq!(hosts, vec!["direct", "extra", "work"]);
    }

    #[test]
    fn parse_host_aliases_skips_wildcards_and_negation() {
        let config = r#"
Host *
    ForwardAgent yes

Host !*
    ProxyJump bastion

Host *.example.com
    User deploy

Host dev-*
    User dev

Host valid-one valid-two
    HostName example.com
"#;
        let hosts = parse_host_aliases(config);
        assert_eq!(
            hosts,
            vec!["valid-one".to_string(), "valid-two".to_string()]
        );
    }

    #[test]
    fn parse_ssh_g_output_reads_fixture() {
        let output = fs::read_to_string(fixture("tests/fixtures/ssh_g/dev-local.txt")).unwrap();
        let host = parse_ssh_g_output("dev-local", &output);
        assert_eq!(
            host,
            SshHost {
                name: "dev-local".to_string(),
                hostname: Some("localhost".to_string()),
                user: Some("dev".to_string()),
                port: Some(22),
                proxy_jump: None,
                identity_file: None,
                forward_agent: None,
                remote_command: None,
                certificate_file: None,
            }
        );
    }

    #[test]
    fn parse_ssh_g_output_extracts_proxyjump_and_identityfile() {
        let output = r#"
host jump-host
hostname jump.example.com
user jumper
port 2222
proxyjump bastion.example.com
identityfile /home/user/.ssh/id_ed25519
identityfile /home/user/.ssh/id_rsa
"#;
        let host = parse_ssh_g_output("jump-host", output);
        assert_eq!(host.hostname.as_deref(), Some("jump.example.com"));
        assert_eq!(host.user.as_deref(), Some("jumper"));
        assert_eq!(host.port, Some(2222));
        assert_eq!(host.proxy_jump.as_deref(), Some("bastion.example.com"));
        assert_eq!(
            host.identity_file.as_deref(),
            Some("/home/user/.ssh/id_ed25519")
        );
    }

    #[test]
    fn parse_ssh_g_output_extracts_forwardagent_and_remotecommand() {
        let output = r#"
hostname example.com
user admin
port 22
forwardagent yes
remotecommand tmux attach
"#;
        let host = parse_ssh_g_output("test", output);
        assert_eq!(host.forward_agent, Some(true));
        assert_eq!(host.remote_command.as_deref(), Some("tmux attach"));
    }

    #[test]
    fn parse_ssh_g_output_forwardagent_no_sets_false() {
        let output = "forwardagent no\n";
        let host = parse_ssh_g_output("test", output);
        assert_eq!(host.forward_agent, Some(false));
    }

    #[test]
    fn parse_ssh_g_output_remotecommand_none_leaves_none() {
        let output = "remotecommand none\n";
        let host = parse_ssh_g_output("test", output);
        assert_eq!(host.remote_command, None);
    }

    #[test]
    fn list_hosts_missing_config_returns_empty() {
        let dir = std::env::temp_dir().join(format!("sshub-missing-{}", std::process::id()));
        let path = dir.join("config");
        let resolver = SshConfigResolver::with_config_path(&path);
        assert!(!path.exists());
        assert!(resolver.list_hosts().unwrap().is_empty());
    }

    #[test]
    fn ssh_config_path_honors_env_override() {
        let path = fixture("tests/fixtures/ssh_config");
        std::env::set_var("SSH_LAUNCHER_SSH_CONFIG", &path);
        assert_eq!(ssh_config_path().unwrap(), path);
    }
}
