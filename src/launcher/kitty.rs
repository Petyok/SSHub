use std::process::Command;

use anyhow::Result;

use super::TerminalLauncher;

#[derive(Debug, Default)]
pub struct KittyLauncher;

pub(crate) fn build_terminal_argv(ssh_argv: &[String]) -> Vec<String> {
    let display_name = ssh_argv.last().map(String::as_str).unwrap_or("ssh");
    let mut argv = vec![
        "kitty".into(),
        "--class".into(),
        "sshub-session".into(),
        "--title".into(),
        format!("SSH: {display_name}"),
        "--hold".into(), // keep window open after command exits (shows errors)
        "-e".into(),
    ];
    argv.extend(ssh_argv.iter().cloned());
    argv
}

impl TerminalLauncher for KittyLauncher {
    fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()> {
        let argv = build_terminal_argv(ssh_argv);
        Command::new(&argv[0]).args(&argv[1..]).spawn()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{build_ssh_alias_argv, SshHost};

    #[test]
    fn build_terminal_argv_wraps_alias_ssh_command() {
        let host = SshHost::new("prod-web");
        assert_eq!(
            build_terminal_argv(&build_ssh_alias_argv(&host)),
            vec![
                "kitty".to_string(),
                "--class".to_string(),
                "sshub-session".to_string(),
                "--title".to_string(),
                "SSH: prod-web".to_string(),
                "--hold".to_string(),
                "-e".to_string(),
                "ssh".to_string(),
                "prod-web".to_string(),
            ]
        );
    }

    #[test]
    fn build_terminal_argv_wraps_managed_ssh_command() {
        assert_eq!(
            build_terminal_argv(&[
                "ssh".into(),
                "-p".into(),
                "2222".into(),
                "deploy@10.0.0.1".into(),
            ]),
            vec![
                "kitty".to_string(),
                "--class".to_string(),
                "sshub-session".to_string(),
                "--title".to_string(),
                "SSH: deploy@10.0.0.1".to_string(),
                "--hold".to_string(),
                "-e".to_string(),
                "ssh".to_string(),
                "-p".to_string(),
                "2222".to_string(),
                "deploy@10.0.0.1".to_string(),
            ]
        );
    }
}
