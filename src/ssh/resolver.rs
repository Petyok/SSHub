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
        let content = fs::read_to_string(&self.config_path).with_context(|| {
            format!(
                "failed to read SSH config at {}",
                self.config_path.display()
            )
        })?;
        Ok(parse_host_aliases(&content))
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
        let Some(rest) = line.strip_prefix("Host ") else {
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
