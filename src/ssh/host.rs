#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub name: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub proxy_jump: Option<String>,
    pub identity_file: Option<String>,
    pub forward_agent: Option<bool>,
    pub remote_command: Option<String>,
    pub certificate_file: Option<String>,
}

impl SshHost {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            hostname: None,
            user: None,
            port: None,
            proxy_jump: None,
            identity_file: None,
            forward_agent: None,
            remote_command: None,
            certificate_file: None,
        }
    }
}

/// Build argv for `ssh` using explicit connection fields (managed / direct connect).
pub fn build_ssh_argv(host: &SshHost) -> Vec<String> {
    let mut args = vec!["ssh".into()];

    if let Some(port) = host.port {
        if port != 22 {
            args.push("-p".into());
            args.push(port.to_string());
        }
    }

    if let Some(ref identity) = host.identity_file {
        args.push("-i".into());
        args.push(identity.clone());
    }

    if let Some(ref cert) = host.certificate_file {
        args.push("-o".into());
        args.push(format!("CertificateFile={cert}"));
    }

    if let Some(ref jump) = host.proxy_jump {
        args.push("-J".into());
        args.push(jump.clone());
    }

    if let Some(forward) = host.forward_agent {
        args.push("-o".into());
        args.push(format!(
            "ForwardAgent={}",
            if forward { "yes" } else { "no" }
        ));
    }

    let hostname = host.hostname.as_deref().unwrap_or(&host.name);
    let target = if let Some(ref user) = host.user {
        format!("{user}@{hostname}")
    } else {
        hostname.to_string()
    };
    args.push(target);

    // Remote command runs on the SSH host (appended after target)
    if let Some(ref cmd) = host.remote_command {
        if !cmd.is_empty() {
            args.push("--".into());
            args.push(cmd.clone());
        }
    }

    args
}

/// Build argv for alias connect (`ssh name` via ssh_config).
pub fn build_ssh_alias_argv(host: &SshHost) -> Vec<String> {
    vec!["ssh".into(), host.name.clone()]
}

/// Build argv for `mosh` using explicit connection fields (managed / direct connect).
pub fn build_mosh_argv(host: &SshHost) -> Vec<String> {
    build_mosh_from_ssh_argv(&build_ssh_argv(host))
}

/// Build argv for alias connect (`mosh name` via ssh_config).
pub fn build_mosh_alias_argv(host: &SshHost) -> Vec<String> {
    vec!["mosh".into(), host.name.clone()]
}

const ACCEPT_NEW_SSH_OPT: &str = "StrictHostKeyChecking=accept-new";

/// Inject `-o StrictHostKeyChecking=accept-new` into the inner `ssh` command
/// used by a `mosh` argv.
///
/// - Managed (`--ssh=…`): appends the option to the existing `--ssh=` value.
/// - Alias (`mosh name`): inserts `--ssh=ssh -o …` before the hostname.
pub fn inject_mosh_ssh_accept_new(mut argv: Vec<String>) -> Vec<String> {
    if argv.first().map(String::as_str) != Some("mosh") {
        return argv;
    }

    if let Some(idx) = argv.iter().position(|a| a.starts_with("--ssh=")) {
        let ssh_cmd = &argv[idx]["--ssh=".len()..];
        argv[idx] = format!("--ssh={ssh_cmd} -o {ACCEPT_NEW_SSH_OPT}");
        return argv;
    }

    if argv.len() >= 2 {
        argv.insert(1, format!("--ssh=ssh -o {ACCEPT_NEW_SSH_OPT}"));
    }
    argv
}

/// Convert a full `ssh` argv (from [`build_ssh_argv`]) into `mosh` argv.
pub fn build_mosh_from_ssh_argv(ssh_argv: &[String]) -> Vec<String> {
    if ssh_argv.first().map(String::as_str) != Some("ssh") {
        return vec!["mosh".into()];
    }
    if ssh_argv.len() == 2 {
        return vec!["mosh".into(), ssh_argv[1].clone()];
    }

    let (base, remote_cmd) = split_ssh_remote_command(ssh_argv);
    let target = match base.last() {
        Some(t) => t.clone(),
        None => return vec!["mosh".into()],
    };
    let ssh_cmd = if base.len() <= 1 {
        "ssh".to_string()
    } else {
        base[..base.len() - 1].join(" ")
    };

    let mut out = vec!["mosh".into(), format!("--ssh={ssh_cmd}"), target];
    if let Some(cmd) = remote_cmd {
        out.push("--".into());
        out.push(cmd);
    }
    out
}

fn split_ssh_remote_command(ssh_argv: &[String]) -> (Vec<String>, Option<String>) {
    if let Some(pos) = ssh_argv.iter().position(|a| a == "--") {
        let (base, rest) = ssh_argv.split_at(pos);
        let cmd = rest.get(1).cloned();
        (base.to_vec(), cmd)
    } else {
        (ssh_argv.to_vec(), None)
    }
}

/// Format a single OpenSSH config option for `-o key=value` argv.
pub fn format_ssh_config_option(key: &str, value: &str) -> String {
    if value.is_empty() {
        return format!("{key}=");
    }
    if value
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '='))
    {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{key}=\"{escaped}\"")
    } else {
        format!("{key}={value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ssh_argv_includes_identity_port_jump_and_agent() {
        let mut host = SshHost::new("prod-web");
        host.hostname = Some("10.0.0.50".into());
        host.port = Some(2222);
        host.user = Some("deploy".into());
        host.identity_file = Some("~/.ssh/id_ed25519".into());
        host.proxy_jump = Some("bastion".into());
        host.forward_agent = Some(true);

        assert_eq!(
            build_ssh_argv(&host),
            vec![
                "ssh".to_string(),
                "-p".to_string(),
                "2222".to_string(),
                "-i".to_string(),
                "~/.ssh/id_ed25519".to_string(),
                "-J".to_string(),
                "bastion".to_string(),
                "-o".to_string(),
                "ForwardAgent=yes".to_string(),
                "deploy@10.0.0.50".to_string(),
            ]
        );
    }

    #[test]
    fn build_ssh_argv_omits_default_port_and_false_agent() {
        let mut host = SshHost::new("web");
        host.hostname = Some("example.com".into());
        host.port = Some(22);
        host.forward_agent = Some(false);

        assert_eq!(
            build_ssh_argv(&host),
            vec![
                "ssh".to_string(),
                "-o".to_string(),
                "ForwardAgent=no".to_string(),
                "example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_ssh_alias_argv_uses_host_name() {
        let host = SshHost::new("staging");
        assert_eq!(
            build_ssh_alias_argv(&host),
            vec!["ssh".to_string(), "staging".to_string()]
        );
    }

    #[test]
    fn build_ssh_argv_includes_certificate_file() {
        let mut host = SshHost::new("cert-host");
        host.hostname = Some("10.0.0.1".into());
        host.identity_file = Some("~/.ssh/id_ed25519".into());
        host.certificate_file = Some("~/.ssh/id_ed25519-cert.pub".into());

        let argv = build_ssh_argv(&host);
        assert!(argv.contains(&"-o".to_string()));
        assert!(argv.contains(&"CertificateFile=~/.ssh/id_ed25519-cert.pub".to_string()));
        // certificate_file should come after identity_file
        let i_pos = argv.iter().position(|a| a == "-i").unwrap();
        let c_pos = argv
            .iter()
            .position(|a| a.starts_with("CertificateFile"))
            .unwrap();
        assert!(c_pos > i_pos);
    }

    #[test]
    fn build_ssh_argv_appends_remote_command_after_target() {
        let mut host = SshHost::new("web");
        host.hostname = Some("example.com".into());
        host.remote_command = Some("tmux attach".into());

        assert_eq!(
            build_ssh_argv(&host),
            vec![
                "ssh".to_string(),
                "example.com".to_string(),
                "--".to_string(),
                "tmux attach".to_string(),
            ]
        );
    }

    #[test]
    fn build_mosh_alias_argv_uses_host_name() {
        let host = SshHost::new("staging");
        assert_eq!(
            build_mosh_alias_argv(&host),
            vec!["mosh".to_string(), "staging".to_string()]
        );
    }

    #[test]
    fn build_mosh_argv_wraps_ssh_options() {
        let mut host = SshHost::new("prod");
        host.hostname = Some("10.0.0.5".into());
        host.port = Some(2222);
        host.user = Some("deploy".into());
        host.identity_file = Some("~/.ssh/id_ed25519".into());

        assert_eq!(
            build_mosh_argv(&host),
            vec![
                "mosh".to_string(),
                "--ssh=ssh -p 2222 -i ~/.ssh/id_ed25519".to_string(),
                "deploy@10.0.0.5".to_string(),
            ]
        );
    }

    #[test]
    fn build_mosh_argv_passes_remote_command() {
        let mut host = SshHost::new("web");
        host.hostname = Some("example.com".into());
        host.remote_command = Some("htop".into());

        assert_eq!(
            build_mosh_argv(&host),
            vec![
                "mosh".to_string(),
                "--ssh=ssh".to_string(),
                "example.com".to_string(),
                "--".to_string(),
                "htop".to_string(),
            ]
        );
    }
}
