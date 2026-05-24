use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sshub::ssh::{parse_host_aliases, parse_ssh_g_output, HostResolver, SshHost};

/// Resolver backed by `tests/fixtures/ssh_config` and `tests/fixtures/ssh_g/*.txt`.
#[derive(Debug, Clone)]
pub struct FixtureResolver {
    config_path: PathBuf,
    ssh_g_dir: PathBuf,
}

impl FixtureResolver {
    pub fn from_manifest_dir() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Self {
            config_path: root.join("tests/fixtures/ssh_config"),
            ssh_g_dir: root.join("tests/fixtures/ssh_g"),
        }
    }

    pub fn with_paths(config_path: impl Into<PathBuf>, ssh_g_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
            ssh_g_dir: ssh_g_dir.into(),
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

impl HostResolver for FixtureResolver {
    fn list_hosts(&self) -> Result<Vec<String>> {
        let content = fs::read_to_string(&self.config_path).with_context(|| {
            format!("read fixture ssh config at {}", self.config_path.display())
        })?;
        Ok(parse_host_aliases(&content))
    }

    fn resolve_host(&self, name: &str) -> Result<SshHost> {
        let path = self.ssh_g_dir.join(format!("{name}.txt"));
        let output = fs::read_to_string(&path)
            .with_context(|| format!("read ssh -G fixture at {}", path.display()))?;
        Ok(parse_ssh_g_output(name, &output))
    }
}
