use anyhow::Result;

use crate::ssh::{build_ssh_alias_argv, build_ssh_argv, SshHost};

pub trait TerminalLauncher: Send + Sync {
    /// Connect via ssh_config alias (`ssh name`).
    fn launch(&self, host: &SshHost) -> Result<()> {
        self.launch_ssh_argv(&build_ssh_alias_argv(host))
    }

    /// Connect with explicit ssh arguments (launcher-managed hosts).
    fn launch_managed(&self, host: &SshHost) -> Result<()> {
        self.launch_ssh_argv(&build_ssh_argv(host))
    }

    /// Spawn a terminal running the given `ssh` argv (`ssh` binary plus args).
    fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()>;
}
