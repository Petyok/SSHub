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
            // Format: "<bits> <fingerprint> <comment with spaces> (TYPE)" —
            // the type is the LAST token in parens; everything between the
            // fingerprint and it is the comment.
            let mut parts = line.split_whitespace();
            let bits = parts.next()?.to_string();
            let fingerprint = parts.next()?.to_string();
            let rest: Vec<&str> = parts.collect();
            let (key_type, comment) = match rest.split_last() {
                Some((last, front)) if last.starts_with('(') && last.ends_with(')') => (
                    last.trim_matches(|c| c == '(' || c == ')').to_string(),
                    front.join(" "),
                ),
                _ => (String::new(), rest.join(" ")),
            };
            Some(AgentKey {
                bits,
                fingerprint,
                comment,
                key_type,
            })
        })
        .collect();
    Ok(keys)
}
