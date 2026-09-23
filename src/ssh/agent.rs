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

/// The exact `ssh-add` invocation [`add_key_with_cert`] runs, exposed so
/// tests can pin the argv without spawning anything.
pub fn agent_add_argv(key_path: &str, cert_path: Option<&str>) -> Vec<String> {
    let mut argv = vec!["ssh-add".to_string(), key_path.to_string()];
    if let Some(cert) = cert_path {
        argv.push(cert.to_string());
    }
    argv
}

/// `ssh-add <key> [<cert>]`: the cert rides along as a second identity file
/// so non-`<key>-cert.pub` paths load too (the default name ssh-add would
/// pick up on its own is just the common case, not the rule).
pub fn add_key_with_cert(key_path: &str, cert_path: Option<&str>) -> Result<()> {
    let argv = agent_add_argv(key_path, cert_path);
    let output = Command::new(&argv[0]).args(&argv[1..]).output()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_add_argv_lists_key_before_cert() {
        // Oracle: the recorded `ssh-add` invocation shape — key first, cert
        // second, exactly as the tool accepts multiple identity files.
        assert_eq!(
            agent_add_argv("/home/u/.ssh/id_ed25519", Some("/home/u/.ssh/id-cert.pub")),
            vec![
                "ssh-add".to_string(),
                "/home/u/.ssh/id_ed25519".to_string(),
                "/home/u/.ssh/id-cert.pub".to_string(),
            ],
        );
        assert_eq!(
            agent_add_argv("/home/u/.ssh/id_ed25519", None),
            vec!["ssh-add".to_string(), "/home/u/.ssh/id_ed25519".to_string(),]
        );
    }

    #[test]
    fn add_key_with_cert_passes_both_paths_to_ssh_add() {
        // Oracle: a fake `ssh-add` on a private `PATH` records its real argv —
        // the test observes what the tool would actually receive.
        let _lock = crate::test_env::lock_home();
        let dir = tempfile::tempdir().unwrap();
        let recorder = dir.path().join("argv.txt");
        let fake = dir.path().join("ssh-add");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nprintf '%s\\n' '---' >> {}\n",
                recorder.display(),
                recorder.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{old_path}", dir.path().display()));
        add_key_with_cert("/home/u/.ssh/id_ed25519", Some("/home/u/.ssh/id-cert.pub")).unwrap();
        add_key_with_cert("/home/u/.ssh/id_other", None).unwrap();
        std::env::set_var("PATH", old_path);

        let recorded = std::fs::read_to_string(&recorder).unwrap();
        assert_eq!(
            recorded,
            "/home/u/.ssh/id_ed25519\n/home/u/.ssh/id-cert.pub\n---\n/home/u/.ssh/id_other\n---\n"
        );
    }
}
