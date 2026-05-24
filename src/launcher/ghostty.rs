use std::process::Command;

use anyhow::Result;

use super::TerminalLauncher;

#[derive(Debug, Default)]
pub struct GhosttyLauncher;

pub(crate) fn build_terminal_argv(ssh_argv: &[String]) -> Vec<String> {
    let mut argv = vec!["ghostty".into(), "-e".into()];
    argv.extend(ssh_argv.iter().cloned());
    argv
}

impl TerminalLauncher for GhosttyLauncher {
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
        let host = SshHost::new("staging");
        assert_eq!(
            build_terminal_argv(&build_ssh_alias_argv(&host)),
            vec![
                "ghostty".to_string(),
                "-e".to_string(),
                "ssh".to_string(),
                "staging".to_string(),
            ]
        );
    }
}
