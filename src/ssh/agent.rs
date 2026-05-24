use anyhow::Result;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct AgentInfo {
    pub socket_path: Option<String>,
    pub keys: Vec<AgentKey>,
    pub forwarding_hosts: usize,
}

#[derive(Debug, Clone)]
pub struct AgentKey {
    pub bits: String,
    pub fingerprint: String,
    pub comment: String,
    pub key_type: String,
}

pub fn detect_agent() -> AgentInfo {
    let socket_path = std::env::var("SSH_AUTH_SOCK").ok();
    let keys = if socket_path.is_some() {
        list_agent_keys().unwrap_or_default()
    } else {
        vec![]
    };
    AgentInfo {
        socket_path,
        keys,
        forwarding_hosts: 0,
    }
}

pub fn remove_key(path: &str) -> Result<()> {
    let output = Command::new("ssh-add").arg("-d").arg(path).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh-add -d failed: {}", stderr.trim());
    }
    Ok(())
}

pub fn add_key(path: &str) -> Result<()> {
    let output = Command::new("ssh-add").arg(path).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh-add failed: {}", stderr.trim());
    }
    Ok(())
}

fn list_agent_keys() -> Result<Vec<AgentKey>> {
    let output = Command::new("ssh-add").arg("-l").output()?;
    if !output.status.success() {
        return Ok(vec![]);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let keys = stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            if parts.len() >= 3 {
                Some(AgentKey {
                    bits: parts[0].to_string(),
                    fingerprint: parts[1].to_string(),
                    comment: parts.get(2).unwrap_or(&"").to_string(),
                    key_type: parts
                        .get(3)
                        .map(|s| s.trim_matches(|c| c == '(' || c == ')').to_string())
                        .unwrap_or_default(),
                })
            } else {
                None
            }
        })
        .collect();
    Ok(keys)
}
