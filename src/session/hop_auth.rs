//! The jump hosts of a ProxyJump chain and their own stored credentials.
//!
//! ssh runs every `-J` hop as a separate `ssh` that inherits the destination's
//! askpass environment, so a hop's `user@192.0.2.11's password:` arrives at the
//! same helper as the destination's prompt. The destination's secret must never
//! answer it (#141), and nothing used to answer it at all: the user typed the
//! password of a host whose password SSHub already holds. A hop prompt is now
//! answered with the stored secret of the hop host it names, and only that one.
//!
//! The prompt is the only thing ssh tells us about which hop is asking, so it is
//! matched by what it prints: `user@host's password:` (host as ssh resolved it,
//! the saved host's address) or `Enter passphrase for key '<path>':`. A prompt
//! that matches no hop, or more than one, is answered by nobody: it goes to the
//! user, as before.
//!
//! The prompt text is not enough: a hostile hop can make ssh print any
//! `user@host's password:` it likes. So the asking ssh is bound to the hop by
//! its command line too: ssh runs a hop as `ssh [-l user] … -W [target]:port
//! <hop>`, and `<hop>` (the last argument, as `-J` named it) must be the hop
//! whose credential would answer. A prompt from one hop never gets another's
//! secret.

use std::sync::Arc;

use super::auth_prompt::PromptClass;
use super::PendingSecret;
use crate::credentials::{SecretRef, StoreLocator};

/// Fetches a hop's stored secret of the asked class, when the hop asks.
pub type Fetch = Arc<dyn Fn(PromptClass) -> Option<PendingSecret> + Send + Sync>;

/// A saved host used as a jump hop, with a lazy way to get its secret. No
/// `Debug`: nothing here may reach diagnostic formatting.
#[derive(Clone)]
pub struct HopCred {
    /// The saved host's name, for the modal ("hop hop-1").
    pub name: String,
    /// The login ssh prints in the prompt; `None` = whatever ssh defaults to.
    pub user: Option<String>,
    /// The address ssh prints in the prompt.
    pub host: String,
    /// The host as `-J` names it (a saved name or an address): the last
    /// argument of the hop's own ssh process.
    pub via: String,
    /// Where a remembered password for this hop is saved.
    pub host_id: Option<i64>,
    /// Where a remembered key passphrase is saved.
    pub identity_id: Option<i64>,
    /// The hop identity's private key, to recognise its passphrase prompt.
    pub key_path: Option<String>,
    fetch: Fetch,
}

impl HopCred {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        user: Option<String>,
        host: String,
        via: String,
        host_id: Option<i64>,
        identity_id: Option<i64>,
        key_path: Option<String>,
        fetch: Fetch,
    ) -> Self {
        Self {
            name,
            user,
            host,
            via,
            host_id,
            identity_id,
            key_path,
            fetch,
        }
    }

    /// The stored secret for a prompt of `class`, fetched now.
    pub fn secret(&self, class: PromptClass) -> Option<PendingSecret> {
        (self.fetch)(class).filter(|s| class.matches_secret(s))
    }
}

/// Where a hop's secret is kept, for ssh children that read it from a staged
/// file (tunnels, exec, broadcast, key push). Holds a reference, never the
/// secret: the helper reads the credential store when this hop actually asks.
#[derive(Clone)]
pub struct HopSecret {
    pub user: Option<String>,
    pub host: String,
    /// See [`HopCred::via`].
    pub via: String,
    pub key_path: Option<String>,
    pub class: PromptClass,
    /// Store entries to try, in order; the first that holds a secret wins.
    pub refs: Vec<SecretRef>,
    /// The store `refs` live in; `None` when no other process can open it, in
    /// which case this hop is not staged and its prompt is left to the user.
    pub store: Option<StoreLocator>,
}

impl std::fmt::Debug for HopSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HopSecret")
            .field("user", &self.user)
            .field("host", &self.host)
            .field("class", &self.class)
            .field("refs", &self.refs)
            .finish_non_exhaustive()
    }
}

/// Who a prompt says is asking.
#[derive(Debug, PartialEq, Eq)]
pub enum Asker<'a> {
    /// `user@host's password:`
    Login { user: &'a str, host: &'a str },
    /// `Enter passphrase for key '<path>':`
    Key(&'a str),
}

/// Read the asker out of an ssh prompt. Only ssh's own two prompts count, and
/// only whole: `user@host's password: ` and `Enter passphrase for key '<path>': `
/// with nothing before them. A hostile jump host can make ssh show any
/// keyboard-interactive text, but ssh puts `(user@host) ` in front of it.
pub fn asker(prompt: &str) -> Option<Asker<'_>> {
    let prompt = prompt.trim_end();
    if let Some(login) = prompt.strip_suffix("'s password:") {
        if login.contains(char::is_whitespace) {
            return None;
        }
        let (user, host) = login.rsplit_once('@')?;
        return (!user.is_empty() && !host.is_empty()).then_some(Asker::Login { user, host });
    }
    let path = prompt
        .strip_prefix("Enter passphrase for key '")?
        .strip_suffix("':")?;
    (!path.is_empty()).then_some(Asker::Key(path))
}

/// The one hop `prompt` names, `None` when it names none or several.
pub fn find<'a>(hops: &'a [HopCred], prompt: &str) -> Option<&'a HopCred> {
    unique(
        hops,
        &asker(prompt)?,
        |h| (h.user.as_deref(), h.host.as_str()),
        |h| h.key_path.as_deref(),
    )
}

/// [`find`], and the hop is the one the asking ssh `pid` is running for. A
/// prompt naming a hop that this process is not connecting to is a lie.
pub fn find_for<'a>(hops: &'a [HopCred], prompt: &str, pid: Option<u32>) -> Option<&'a HopCred> {
    find(hops, prompt).filter(|h| runs_for(pid, &h.via))
}

/// The one staged secret whose hop and class `prompt` names, asked by the ssh
/// `pid` that is connecting to that hop.
pub fn find_secret<'a>(
    hops: &'a [HopSecret],
    prompt: &str,
    pid: Option<u32>,
) -> Option<&'a HopSecret> {
    let class = PromptClass::classify(prompt);
    let who = asker(prompt)?;
    let same_class: Vec<&HopSecret> = hops.iter().filter(|h| h.class == class).collect();
    unique(
        &same_class,
        &who,
        |h| (h.user.as_deref(), h.host.as_str()),
        |h| h.key_path.as_deref(),
    )
    .copied()
    .filter(|h| runs_for(pid, &h.via))
}

/// Whether process `pid` is an ssh connecting to `via`. Unknown pid or an
/// unreadable process: no.
fn runs_for(pid: Option<u32>, via: &str) -> bool {
    pid.and_then(target_of_process)
        .is_some_and(|target| target.eq_ignore_ascii_case(via))
}

/// Tests ask on behalf of made-up pids: let them say what each one runs for.
#[cfg(test)]
static PRETEND: std::sync::Mutex<Vec<(u32, String)>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn pretend_target(pid: u32, target: &str) {
    PRETEND.lock().unwrap().push((pid, target.to_owned()));
}

/// The host a running ssh connects to: its last command-line argument.
fn target_of_process(pid: u32) -> Option<String> {
    #[cfg(test)]
    if let Some((_, target)) = PRETEND.lock().unwrap().iter().find(|(p, _)| *p == pid) {
        return Some(target.clone());
    }
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let last = raw.split(|b| *b == 0).rfind(|arg| !arg.is_empty())?;
        Some(String::from_utf8_lossy(last).into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        // A host or user name has no whitespace (validated on the way in), so
        // `ps`'s space-joined args still end in the host.
        let out = std::process::Command::new("ps")
            .args(["-ww", "-o", "args=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next_back()
            .map(str::to_owned)
    }
}

fn unique<'a, T>(
    items: &'a [T],
    who: &Asker<'_>,
    login: impl Fn(&T) -> (Option<&str>, &str),
    key: impl Fn(&T) -> Option<&str>,
) -> Option<&'a T> {
    let mut found = None;
    for item in items {
        let hit = match who {
            Asker::Login { user, host } => {
                let (u, h) = login(item);
                h.eq_ignore_ascii_case(host) && u.is_none_or(|u| u == *user)
            }
            Asker::Key(path) => key(item) == Some(*path),
        };
        if hit {
            if found.is_some() {
                // Two hops this prompt could be: answer neither.
                return None;
            }
            found = Some(item);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(name: &str, user: Option<&str>, host: &str) -> HopCred {
        let secret = format!("{name}-pw");
        HopCred::new(
            name.into(),
            user.map(String::from),
            host.into(),
            host.into(),
            Some(1),
            None,
            None,
            Arc::new(move |class| {
                (class == PromptClass::Password).then(|| PendingSecret::Password(secret.clone()))
            }),
        )
    }

    #[test]
    fn the_asker_is_read_from_what_ssh_prints() {
        assert_eq!(
            asker("ops@192.0.2.11's password: "),
            Some(Asker::Login {
                user: "ops",
                host: "192.0.2.11"
            })
        );
        // Anything around the prompt makes it not ssh's own: a banner line in
        // front, or the `(user@host) ` prefix every keyboard-interactive
        // prompt carries (a server picks that text, so it can name any hop).
        assert_eq!(
            asker("Warning: x\r\njohn@corp.com@192.0.2.12's password: "),
            None
        );
        assert_eq!(asker("(ops@bastion) ops@192.0.2.12's password: "), None);
        assert_eq!(
            asker("john@corp.com@192.0.2.12's password: "),
            Some(Asker::Login {
                user: "john@corp.com",
                host: "192.0.2.12"
            })
        );
        assert_eq!(
            asker("(ops@bastion) Enter passphrase for key '/home/u/.ssh/id': "),
            None
        );
        assert_eq!(asker("Please Enter passphrase for key '/k': "), None);
        assert_eq!(
            asker("Enter passphrase for key '/home/u/.ssh/id_ed25519': "),
            Some(Asker::Key("/home/u/.ssh/id_ed25519"))
        );
        assert_eq!(asker("Verification code:"), None);
        assert_eq!(asker("'s password"), None);
    }

    #[test]
    fn a_prompt_finds_exactly_the_hop_it_names() {
        let hops = [
            hop("hop-1", Some("ops"), "192.0.2.11"),
            hop("hop-2", Some("ops"), "192.0.2.12"),
        ];
        assert_eq!(
            find(&hops, "ops@192.0.2.12's password: ").map(|h| h.name.as_str()),
            Some("hop-2")
        );
        // Another user on a known host, or an unknown host: nobody answers.
        assert!(find(&hops, "root@192.0.2.12's password: ").is_none());
        assert!(find(&hops, "ops@192.0.2.99's password: ").is_none());
        // A login-less hop answers for whichever user ssh defaulted to.
        let hops = [hop("hop-3", None, "Hop-3.example")];
        assert!(find(&hops, "anyone@hop-3.example's password: ").is_some());
    }

    #[test]
    fn two_hops_the_prompt_could_be_are_answered_by_neither() {
        let hops = [
            hop("a", Some("ops"), "192.0.2.11"),
            hop("b", Some("ops"), "192.0.2.11"),
        ];
        assert!(find(&hops, "ops@192.0.2.11's password: ").is_none());
    }

    #[test]
    fn a_hop_only_ever_answers_its_own_class() {
        let h = hop("hop-1", Some("ops"), "192.0.2.11");
        assert!(h.secret(PromptClass::Password).is_some());
        assert!(h.secret(PromptClass::Passphrase).is_none());
        assert!(h.secret(PromptClass::HostKey).is_none());
        let staged = [HopSecret {
            user: h.user.clone(),
            host: h.host.clone(),
            via: h.via.clone(),
            key_path: h.key_path.clone(),
            class: PromptClass::Password,
            refs: vec![SecretRef::Key("host:1".into())],
            store: None,
        }];
        let me = Some(std::process::id());
        let (_child, pid) = fake_hop_ssh("192.0.2.11");
        assert!(find_secret(&staged, "ops@192.0.2.11's password: ", Some(pid)).is_some());
        assert!(find_secret(&staged, "ops@192.0.2.12's password: ", Some(pid)).is_none());
        assert!(find_secret(&staged, "Enter passphrase for key '/k': ", Some(pid)).is_none());
        // This test process is not connecting to the hop.
        assert!(find_secret(&staged, "ops@192.0.2.11's password: ", me).is_none());
        assert!(find_secret(&staged, "ops@192.0.2.11's password: ", None).is_none());
    }

    /// A process whose last argument is `host`, like a hop's `ssh … <hop>`.
    fn fake_hop_ssh(host: &str) -> (ChildGuard, u32) {
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & wait", "-W", host])
            .spawn()
            .unwrap();
        let pid = child.id();
        // Between fork and exec the command line is still the test binary's.
        for _ in 0..500 {
            if target_of_process(pid).as_deref() == Some(host) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        (ChildGuard(child), pid)
    }

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn a_hostile_hop_cannot_borrow_another_hops_secret() {
        // Chain bastion -> db-gw. The bastion's ssh shows a prompt that names
        // db-gw; the text is perfect, but the asking process is the bastion's.
        let hops = [
            hop("bastion", Some("ops"), "192.0.2.1"),
            hop("db-gw", Some("ops"), "192.0.2.12"),
        ];
        let (_bastion, bastion_pid) = fake_hop_ssh("192.0.2.1");
        let (_gw, gw_pid) = fake_hop_ssh("192.0.2.12");
        let lie = "ops@192.0.2.12's password: ";
        assert!(find_for(&hops, lie, Some(bastion_pid)).is_none());
        assert_eq!(
            find_for(&hops, lie, Some(gw_pid)).map(|h| h.name.as_str()),
            Some("db-gw")
        );
        assert!(find_for(&hops, lie, None).is_none());
    }
}
