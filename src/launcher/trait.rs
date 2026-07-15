use anyhow::Result;

use crate::session_transport::SessionTransport;
use crate::ssh::{
    build_mosh_alias_argv, build_mosh_argv, build_ssh_alias_argv, build_ssh_argv, SshHost,
};

pub trait TerminalLauncher: Send + Sync {
    /// Connect via ssh_config alias (`ssh name` or `mosh name`).
    fn launch(&self, host: &SshHost) -> Result<()> {
        self.launch_ssh_argv(&build_ssh_alias_argv(host))
    }

    /// Connect with explicit ssh arguments (launcher-managed hosts).
    fn launch_managed(&self, host: &SshHost) -> Result<()> {
        self.launch_ssh_argv(&build_ssh_argv(host))
    }

    /// Connect via ssh_config alias with the chosen session transport.
    fn launch_with_transport(&self, host: &SshHost, transport: SessionTransport) -> Result<()> {
        let argv = match transport {
            SessionTransport::Ssh => build_ssh_alias_argv(host),
            SessionTransport::Mosh => build_mosh_alias_argv(host),
        };
        self.launch_ssh_argv(&argv)
    }

    /// Connect with explicit connection fields and the chosen session transport.
    fn launch_managed_with_transport(
        &self,
        host: &SshHost,
        transport: SessionTransport,
    ) -> Result<()> {
        let argv = match transport {
            SessionTransport::Ssh => build_ssh_argv(host),
            SessionTransport::Mosh => build_mosh_argv(host),
        };
        self.launch_ssh_argv(&argv)
    }

    /// Spawn a terminal running the given session argv (`ssh`/`mosh` plus args).
    fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()>;
}
