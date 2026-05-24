use std::sync::{Arc, Mutex};

use anyhow::Result;
use sshub::launcher::TerminalLauncher;
use sshub::ssh::SshHost;

/// Records the last launched host and ssh argv for e2e tests (no real terminal spawn).
#[derive(Debug, Default, Clone)]
pub struct MockLauncher {
    pub last_host: Arc<Mutex<Option<SshHost>>>,
    pub last_ssh_argv: Arc<Mutex<Option<Vec<String>>>>,
    pub managed_connect: Arc<Mutex<bool>>,
}

impl MockLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_last(&self) -> Option<SshHost> {
        self.last_host.lock().ok()?.take()
    }

    pub fn take_last_ssh_argv(&self) -> Option<Vec<String>> {
        self.last_ssh_argv.lock().ok()?.take()
    }

    pub fn was_managed_connect(&self) -> bool {
        self.managed_connect.lock().ok().is_some_and(|v| *v)
    }
}

impl TerminalLauncher for MockLauncher {
    fn launch(&self, host: &SshHost) -> Result<()> {
        if let Ok(mut guard) = self.last_host.lock() {
            *guard = Some(host.clone());
        }
        if let Ok(mut guard) = self.last_ssh_argv.lock() {
            *guard = Some(vec!["ssh".into(), host.name.clone()]);
        }
        if let Ok(mut guard) = self.managed_connect.lock() {
            *guard = false;
        }
        Ok(())
    }

    fn launch_managed(&self, host: &SshHost) -> Result<()> {
        let ssh_argv = sshub::ssh::build_ssh_argv(host);
        if let Ok(mut guard) = self.last_host.lock() {
            *guard = Some(host.clone());
        }
        if let Ok(mut guard) = self.last_ssh_argv.lock() {
            *guard = Some(ssh_argv);
        }
        if let Ok(mut guard) = self.managed_connect.lock() {
            *guard = true;
        }
        Ok(())
    }

    fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()> {
        if let Ok(mut guard) = self.last_ssh_argv.lock() {
            *guard = Some(ssh_argv.to_vec());
        }
        // Reconstruct a minimal SshHost from the argv for test assertions
        if let Some(host_name) = ssh_argv.last() {
            if let Ok(mut guard) = self.last_host.lock() {
                *guard = Some(SshHost::new(host_name));
            }
        }
        // Determine if this was a managed connect (more than just "ssh <alias>")
        if let Ok(mut guard) = self.managed_connect.lock() {
            *guard = ssh_argv.len() > 2;
        }
        Ok(())
    }
}
