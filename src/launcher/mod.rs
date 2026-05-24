mod custom;
mod ghostty;
mod kitty;
mod r#trait;

pub use custom::{apply_template, CustomLauncher};
pub use ghostty::GhosttyLauncher;
pub use kitty::KittyLauncher;
pub use r#trait::TerminalLauncher;

use crate::config::{AppConfig, TerminalKind};
use anyhow::Result;

pub fn launcher_from_config(config: &AppConfig) -> Result<Box<dyn TerminalLauncher>> {
    match config.terminal {
        TerminalKind::Kitty => Ok(Box::new(KittyLauncher)),
        TerminalKind::Ghostty => Ok(Box::new(GhosttyLauncher)),
        TerminalKind::Custom => {
            let template = config
                .launch_command
                .clone()
                .ok_or_else(|| anyhow::anyhow!("terminal=custom requires launch_command"))?;
            Ok(Box::new(CustomLauncher { template }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SshHost;

    #[test]
    fn apply_template_replaces_placeholders() {
        let mut host = SshHost::new("web");
        host.hostname = Some("10.0.0.1".into());
        host.user = Some("ubuntu".into());
        host.port = Some(2222);
        let out = apply_template("ssh {user}@{hostname} -p {port} -l {host}", &host).unwrap();
        assert_eq!(out, "ssh ubuntu@10.0.0.1 -p 2222 -l web");
    }

    #[test]
    fn apply_template_rejects_unknown_placeholder() {
        let host = SshHost::new("web");
        let err = apply_template("ssh {foo}", &host).unwrap_err();
        assert!(err.to_string().contains("{foo}"));
    }

    #[test]
    fn apply_template_empty_port_when_unset() {
        let host = SshHost::new("web");
        let out = apply_template("ssh -p{port} {host}", &host).unwrap();
        assert_eq!(out, "ssh -p web");
    }

    #[test]
    fn custom_launcher_builds_direct_argv() {
        let host = SshHost::new("prod");
        let argv = custom::build_argv("wezterm start -- ssh {host}", &host).expect("build argv");
        assert_eq!(
            argv,
            vec![
                "wezterm".to_string(),
                "start".to_string(),
                "--".to_string(),
                "ssh".to_string(),
                "prod".to_string(),
            ]
        );
    }

    #[test]
    fn custom_launcher_explicit_sh_c_splits_without_wrapping_again() {
        let host = SshHost::new("prod");
        let argv = custom::build_argv(r#"sh -c 'kitty -e ssh {host}'"#, &host).expect("build");
        assert_eq!(
            argv,
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "kitty -e ssh prod".to_string(),
            ]
        );
    }

    #[test]
    fn custom_launcher_wraps_shell_operators() {
        let host = SshHost::new("prod");
        let argv = custom::build_argv("echo {host} | less", &host).expect("build");
        assert_eq!(argv, vec!["sh", "-c", "echo prod | less"]);
    }

    #[test]
    fn launcher_from_config_custom_requires_launch_command() {
        let config = AppConfig {
            terminal: TerminalKind::Custom,
            launch_command: None,
            ..AppConfig::default()
        };
        assert!(launcher_from_config(&config).is_err());
    }

    #[test]
    fn launcher_from_config_custom_with_template() {
        let config = AppConfig {
            terminal: TerminalKind::Custom,
            launch_command: Some("alacritty -e ssh {host}".into()),
            ..AppConfig::default()
        };
        assert!(launcher_from_config(&config).is_ok());
    }

    #[test]
    fn kitty_launcher_builds_expected_argv() {
        use crate::ssh::build_ssh_alias_argv;

        let host = SshHost::new("edge");
        assert_eq!(
            kitty::build_terminal_argv(&build_ssh_alias_argv(&host)),
            vec![
                "kitty".to_string(),
                "--class".to_string(),
                "sshub-session".to_string(),
                "--title".to_string(),
                "SSH: edge".to_string(),
                "--hold".to_string(),
                "-e".to_string(),
                "ssh".to_string(),
                "edge".to_string(),
            ]
        );
    }

    #[test]
    fn ghostty_launcher_builds_expected_argv() {
        use crate::ssh::build_ssh_alias_argv;

        let host = SshHost::new("edge");
        assert_eq!(
            ghostty::build_terminal_argv(&build_ssh_alias_argv(&host)),
            vec![
                "ghostty".to_string(),
                "-e".to_string(),
                "ssh".to_string(),
                "edge".to_string(),
            ]
        );
    }
}
