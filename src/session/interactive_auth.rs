//! One session's authentication state. Answers never travel through the PTY.

use std::path::Path;

use std::collections::{HashMap, HashSet};

use super::askpass_channel::{Channel, Request};
use super::auth_prompt::PromptClass;
use super::hop_auth::{self, HopCred};
use super::PendingSecret;

/// A destination is created only for a recognized, persistable prompt class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    Host(i64),
    Identity(i64),
}

impl SaveTarget {
    pub fn for_prompt(
        class: PromptClass,
        host: Option<i64>,
        identity: Option<i64>,
    ) -> Option<Self> {
        // A config alias has no durable ownership association, including for a
        // key passphrase. Never infer a destination from prompt text alone.
        host?;
        match class {
            PromptClass::Password => host.map(Self::Host),
            PromptClass::Passphrase => identity.map(Self::Identity),
            PromptClass::HostKey | PromptClass::Generic => None,
        }
    }

    pub fn key(self) -> String {
        match self {
            Self::Host(id) => crate::credentials::host_key(id),
            Self::Identity(id) => crate::credentials::identity_key(id),
        }
    }
}

/// Where a hop's remembered answer of `class` is saved: that hop host's row for
/// a password, its identity for a key passphrase.
fn hop_target(hop: &HopCred, class: PromptClass) -> Option<SaveTarget> {
    match class {
        PromptClass::Password => hop.host_id.map(SaveTarget::Host),
        PromptClass::Passphrase => hop.identity_id.map(SaveTarget::Identity),
        PromptClass::HostKey | PromptClass::Generic => None,
    }
}

/// Deliberately has no Debug/Clone implementation: do not expose or duplicate
/// submitted credentials while waiting for the actual authentication marker.
pub struct RememberedSecret {
    pub target: SaveTarget,
    pub value: String,
}

pub struct InteractiveAuth {
    channel: Option<Channel>,
    pub request: Option<Request>,
    /// The attempt number of the open request, for the modal.
    pub attempts: u8,
    /// Prompts seen per asker and kind (credential or host key): each ssh in
    /// the chain gets its own three tries, not one budget for all of them.
    counts: HashMap<(String, bool), u8>,
    /// The jump hosts of the chain with a saved credential of their own.
    pub hops: Vec<HopCred>,
    /// (hop, class) answered from storage already: a second prompt means the
    /// stored secret was wrong, and the user is asked.
    hop_answered: HashSet<(String, PromptClass)>,
    pub host_id: Option<i64>,
    pub identity_id: Option<i64>,
    /// The ssh this session spawned. Only its own prompts belong to the
    /// destination: a ProxyJump hop is another ssh with the same askpass
    /// environment, so it never gets the stored secret or a remember target.
    pub destination_pid: Option<u32>,
    remembered: Vec<RememberedSecret>,
    pub changed_key_shown: bool,
}

impl InteractiveAuth {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            channel: Some(Channel::new()?),
            request: None,
            attempts: 0,
            counts: HashMap::new(),
            hops: Vec::new(),
            hop_answered: HashSet::new(),
            host_id: None,
            identity_id: None,
            destination_pid: None,
            remembered: Vec::new(),
            changed_key_shown: false,
        })
    }

    pub fn channel_env(&self, exe: &Path) -> Vec<(String, String)> {
        self.channel
            .as_ref()
            .map(|channel| channel.env(exe))
            .unwrap_or_default()
    }

    pub fn poll(&mut self, stored: &mut Option<PendingSecret>) {
        if self
            .request
            .as_mut()
            .is_some_and(|request| !request.is_live())
        {
            self.cancel();
            return;
        }
        let Some(channel) = self.channel.as_mut() else {
            return;
        };
        channel.poll();
        if self.request.is_some() {
            return;
        }
        let Some(request) = channel.next_request() else {
            return;
        };
        let class = PromptClass::classify(&request.prompt);
        let from_destination = self.asked_by_destination(&request);
        // A jump hop of this chain whose own credential is saved, if this prompt
        // is one: found by what the prompt says, never by the destination's id.
        let hop = if from_destination {
            None
        } else {
            hop_auth::find_for(&self.hops, &request.prompt, request.ssh_pid).cloned()
        };
        // Who is asking, for the retry budget: the destination, a saved hop, or
        // some other ssh of the chain (by process).
        let asker = match (&hop, from_destination) {
            (Some(h), _) => format!("hop:{}", h.name),
            (None, true) => "destination".to_string(),
            (None, false) => format!("pid:{:?}", request.ssh_pid),
        };
        // Host verification is not a password attempt. Bound its own retries,
        // too, so malformed host responses cannot create an infinite modal loop.
        let count = self
            .counts
            .entry((asker, class == PromptClass::HostKey))
            .or_insert(0);
        if *count >= 3 {
            self.cancel();
            return;
        }
        *count += 1;
        if class != PromptClass::HostKey {
            self.attempts = *count;
        }
        // A re-prompt invalidates the previous value of that class, even if the
        // user cancels or unchecks remember on the next attempt.
        let owner = if from_destination {
            SaveTarget::for_prompt(class, self.host_id, self.identity_id)
        } else {
            hop.as_ref().and_then(|h| hop_target(h, class))
        };
        if let Some(owner) = owner {
            self.remembered.retain(|secret| secret.target != owner);
        }
        if from_destination
            && stored
                .as_ref()
                .is_some_and(|secret| class.matches_secret(secret))
        {
            if let Some(secret) = stored.take() {
                if request.answer(secret.value()).is_err() {
                    self.cancel();
                }
            }
        } else if let Some(hop) = hop.filter(|h| {
            matches!(class, PromptClass::Password | PromptClass::Passphrase)
                && !self.hop_answered.contains(&(h.name.clone(), class))
        }) {
            // The hop's own saved secret, fetched now (not when the session
            // opened) and offered once. Nobody else's: not the destination's,
            // not another hop's.
            self.hop_answered.insert((hop.name.clone(), class));
            match hop.secret(class) {
                Some(secret) => {
                    if request.answer(secret.value()).is_err() {
                        self.cancel();
                    }
                }
                None => self.request = Some(request),
            }
        } else {
            self.request = Some(request);
        }
    }

    fn asked_by_destination(&self, request: &Request) -> bool {
        request.ssh_pid.is_some() && request.ssh_pid == self.destination_pid
    }

    /// Where a remembered answer to the open request would be saved: the
    /// destination's own host or identity, or the hop host the prompt names. A
    /// jump hop's password is never saved on the destination.
    pub fn save_target(&self, class: PromptClass) -> Option<SaveTarget> {
        let request = self.request.as_ref()?;
        if self.asked_by_destination(request) {
            return SaveTarget::for_prompt(class, self.host_id, self.identity_id);
        }
        hop_auth::find_for(&self.hops, &request.prompt, request.ssh_pid)
            .and_then(|hop| hop_target(hop, class))
    }

    /// The saved hop host the open request is from, when it is one.
    pub fn request_hop(&self) -> Option<&HopCred> {
        let request = self.request.as_ref()?;
        if self.asked_by_destination(request) {
            return None;
        }
        hop_auth::find_for(&self.hops, &request.prompt, request.ssh_pid)
    }

    /// The open request is from a jump hop (saved or not), not the destination.
    pub fn request_is_from_a_hop(&self) -> bool {
        self.request
            .as_ref()
            .is_some_and(|request| !self.asked_by_destination(request))
    }

    pub fn answer(&mut self, value: String, remember: bool) -> std::io::Result<()> {
        let target = self
            .request
            .as_ref()
            .and_then(|request| self.save_target(PromptClass::classify(&request.prompt)));
        let Some(request) = self.request.take() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "authentication prompt ended",
            ));
        };
        let result = request.answer(&value);
        if result.is_ok() && remember {
            if let Some(target) = target {
                self.remembered.push(RememberedSecret { target, value });
            }
        }
        if result.is_err() {
            self.cancel();
        }
        result
    }

    pub fn cancelled(&self) -> bool {
        self.channel.is_none()
    }
    /// Whether the askpass listener is still up. A connected session closes it
    /// via `finish`, which also reads as `cancelled`; this tells the two apart
    /// without touching `cancel` semantics.
    pub fn channel_open(&self) -> bool {
        self.channel.is_some()
    }
    pub fn cancel(&mut self) {
        self.request = None;
        self.channel = None;
        self.remembered.clear();
    }

    pub fn finish(&mut self, connected: bool) {
        self.request = None;
        self.channel = None;
        if !connected {
            self.remembered.clear();
        }
    }

    pub fn take_ready_secrets(&mut self, connected: bool) -> Vec<RememberedSecret> {
        if connected {
            std::mem::take(&mut self.remembered)
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_stored_answer_does_not_consume_queued_generic_request() {
        let mut auth = InteractiveAuth::new().unwrap();
        auth.destination_pid = Some(std::process::id());
        let env = auth.channel_env(Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(super::super::askpass_channel::SOCKET_ENV));
        let token = get(super::super::askpass_channel::TOKEN_ENV);
        let mut stored = Some(PendingSecret::Password("stored-answer".into()));
        let first_path = path.clone();
        let first_token = token.clone();
        let first = std::thread::spawn(move || {
            super::super::askpass_channel::request_answer(
                &first_path,
                &first_token,
                "Password:",
                Some(std::process::id()),
                std::time::Duration::from_secs(3),
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while auth.attempts == 0 {
            auth.poll(&mut stored);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(first.join().unwrap().unwrap(), "stored-answer");
        assert!(auth.request.is_none());
        let second = std::thread::spawn(move || {
            super::super::askpass_channel::request_answer(
                &path,
                &token,
                "Verification code:",
                Some(std::process::id()),
                std::time::Duration::from_secs(3),
            )
        });
        while auth.request.is_none() {
            auth.poll(&mut stored);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!second.is_finished());
        assert_eq!(auth.request.as_ref().unwrap().prompt, "Verification code:");
        auth.answer("one-time".into(), true).unwrap();
        assert_eq!(second.join().unwrap().unwrap(), "one-time");
        assert!(auth.take_ready_secrets(true).is_empty());
    }

    #[test]
    fn jump_hop_prompt_gets_neither_the_stored_secret_nor_a_remember_target() {
        // ProxyJump: the jump ssh inherits the session's askpass environment,
        // so its password prompt arrives on the same channel as the
        // destination's. The stored destination password must not be sent to
        // the jump host, and the jump password must not be saved for this host.
        const DESTINATION: u32 = 4242;
        const JUMP: u32 = 4243;
        let mut auth = InteractiveAuth::new().unwrap();
        auth.host_id = Some(7);
        auth.destination_pid = Some(DESTINATION);
        let env = auth.channel_env(Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(super::super::askpass_channel::SOCKET_ENV));
        let token = get(super::super::askpass_channel::TOKEN_ENV);
        let ask = |prompt: &'static str, ssh_pid: u32| {
            let (path, token) = (path.clone(), token.clone());
            std::thread::spawn(move || {
                super::super::askpass_channel::request_answer(
                    &path,
                    &token,
                    prompt,
                    Some(ssh_pid),
                    std::time::Duration::from_secs(3),
                )
            })
        };
        let mut stored = Some(PendingSecret::Password("destination-password".into()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);

        let jump = ask("root@10.193.64.3's password: ", JUMP);
        while auth.request.is_none() {
            auth.poll(&mut stored);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(stored.is_some(), "the jump hop consumed the stored secret");
        assert_eq!(auth.save_target(PromptClass::Password), None);
        auth.answer("jump-password".into(), true).unwrap();
        assert_eq!(jump.join().unwrap().unwrap(), "jump-password");

        let destination = ask("root@10.251.12.234's password: ", DESTINATION);
        while stored.is_some() {
            auth.poll(&mut stored);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(destination.join().unwrap().unwrap(), "destination-password");
        assert!(auth.request.is_none());
        assert!(auth.take_ready_secrets(true).is_empty());
    }

    #[test]
    fn generic_and_unmanaged_prompts_have_no_persistence_destination() {
        assert_eq!(
            SaveTarget::for_prompt(PromptClass::Password, Some(7), Some(9)),
            Some(SaveTarget::Host(7))
        );
        assert_eq!(
            SaveTarget::for_prompt(PromptClass::Passphrase, Some(7), Some(9)),
            Some(SaveTarget::Identity(9))
        );
        assert_eq!(
            SaveTarget::for_prompt(PromptClass::Passphrase, Some(7), None),
            None
        );
        for class in [
            PromptClass::Password,
            PromptClass::Passphrase,
            PromptClass::HostKey,
            PromptClass::Generic,
        ] {
            assert_eq!(SaveTarget::for_prompt(class, None, Some(9)), None);
        }
        for class in [PromptClass::HostKey, PromptClass::Generic] {
            assert_eq!(SaveTarget::for_prompt(class, Some(7), Some(9)), None);
        }
        assert_eq!(SaveTarget::Host(7).key(), crate::credentials::host_key(7));
        assert_eq!(
            SaveTarget::Identity(9).key(),
            crate::credentials::identity_key(9)
        );
    }

    #[test]
    fn persistence_waits_for_success_and_dies_with_failed_session() {
        let mut auth = InteractiveAuth {
            channel: None,
            request: None,
            attempts: 1,
            counts: HashMap::new(),
            hops: Vec::new(),
            hop_answered: HashSet::new(),
            host_id: Some(7),
            identity_id: None,
            destination_pid: None,
            changed_key_shown: false,
            remembered: vec![RememberedSecret {
                target: SaveTarget::Host(7),
                value: "private-value".into(),
            }],
        };
        assert!(auth.take_ready_secrets(false).is_empty());
        let ready = auth.take_ready_secrets(true);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].value, "private-value");
        assert!(auth.take_ready_secrets(true).is_empty());
        auth.remembered = ready;
        auth.finish(false);
        assert!(auth.take_ready_secrets(true).is_empty());
    }

    #[test]
    fn connected_finish_closes_channel_but_keeps_remembered_secrets() {
        // `Session::drain` tears the channel down once connected; the remembered
        // secret must survive for `take_ready_secrets` while the old socket dies.
        let mut auth = InteractiveAuth::new().unwrap();
        auth.host_id = Some(7);
        auth.remembered.push(RememberedSecret {
            target: SaveTarget::Host(7),
            value: "private-value".into(),
        });
        assert!(auth.channel_open());
        let env = auth.channel_env(Path::new("unused"));
        let get = |name: &str| env.iter().find(|(key, _)| key == name).unwrap().1.clone();
        let path = std::path::PathBuf::from(get(super::super::askpass_channel::SOCKET_ENV));
        let token = get(super::super::askpass_channel::TOKEN_ENV);
        auth.finish(true);
        assert!(!auth.channel_open());
        assert!(auth.channel_env(Path::new("unused")).is_empty());
        assert!(super::super::askpass_channel::request_answer(
            &path,
            &token,
            "Password:",
            None,
            std::time::Duration::from_secs(2),
        )
        .is_err());
        let ready = auth.take_ready_secrets(true);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].value, "private-value");
    }
}

#[cfg(test)]
mod hop_tests {
    use super::*;
    use crate::session::askpass_channel as ch;
    use std::sync::{Arc, Mutex};

    const DESTINATION: u32 = 4242;
    const HOP_1: u32 = 4243;
    const HOP_2: u32 = 4244;

    /// A hop with a saved password; `asked` records every fetch.
    fn hop(
        name: &str,
        host: &str,
        password: Option<&str>,
        asked: &Arc<Mutex<Vec<String>>>,
    ) -> HopCred {
        let (asked, name_for_log) = (Arc::clone(asked), name.to_string());
        let password = password.map(String::from);
        HopCred::new(
            name.into(),
            Some("ops".into()),
            host.into(),
            host.into(),
            Some(match name {
                "hop-1" => 11,
                "hop-2" => 12,
                _ => 13,
            }),
            None,
            None,
            Arc::new(move |class| {
                asked
                    .lock()
                    .unwrap()
                    .push(format!("{name_for_log}:{class:?}"));
                match (class, &password) {
                    (PromptClass::Password, Some(p)) => Some(PendingSecret::Password(p.clone())),
                    _ => None,
                }
            }),
        )
    }

    struct Rig {
        auth: InteractiveAuth,
        path: std::path::PathBuf,
        token: String,
    }

    impl Rig {
        fn new(hops: Vec<HopCred>) -> Self {
            let mut auth = InteractiveAuth::new().unwrap();
            auth.host_id = Some(7);
            auth.destination_pid = Some(DESTINATION);
            auth.hops = hops;
            // The asking pids are made up: say which hop each one is running for.
            crate::session::hop_auth::pretend_target(HOP_1, "192.0.2.11");
            crate::session::hop_auth::pretend_target(HOP_2, "192.0.2.12");
            let env = auth.channel_env(Path::new("unused"));
            let get = |name: &str| env.iter().find(|(k, _)| k == name).unwrap().1.clone();
            Self {
                path: get(ch::SOCKET_ENV).into(),
                token: get(ch::TOKEN_ENV),
                auth,
            }
        }

        fn ask(
            &self,
            prompt: &'static str,
            pid: u32,
        ) -> std::thread::JoinHandle<std::io::Result<String>> {
            let (path, token) = (self.path.clone(), self.token.clone());
            std::thread::spawn(move || {
                ch::request_answer(
                    &path,
                    &token,
                    prompt,
                    Some(pid),
                    std::time::Duration::from_secs(5),
                )
            })
        }

        /// Poll until `done` says so.
        fn poll_until(
            &mut self,
            stored: &mut Option<PendingSecret>,
            done: impl Fn(&Self, &Option<PendingSecret>) -> bool,
        ) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done(self, stored) {
                self.auth.poll(stored);
                assert!(std::time::Instant::now() < deadline, "timed out");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn each_hop_prompt_is_answered_with_that_hops_own_stored_password() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut rig = Rig::new(vec![
            hop("hop-1", "192.0.2.11", Some("hop-1-pw"), &asked),
            hop("hop-2", "192.0.2.12", Some("hop-2-pw"), &asked),
        ]);
        let mut stored = Some(PendingSecret::Password("target-pw".into()));

        // One ask at a time: two threads started back to back connect in
        // whichever order the scheduler picks, and the channel queues by that.
        let first = rig.ask("ops@192.0.2.11's password: ", HOP_1);
        rig.poll_until(&mut stored, |_, _| first.is_finished());
        let second = rig.ask("ops@192.0.2.12's password: ", HOP_2);
        rig.poll_until(&mut stored, |_, _| second.is_finished());
        assert_eq!(first.join().unwrap().unwrap(), "hop-1-pw");
        assert_eq!(second.join().unwrap().unwrap(), "hop-2-pw");
        // The target's password went to neither, and no modal was needed.
        assert!(stored.is_some(), "a hop consumed the target's password");
        assert!(rig.auth.request.is_none(), "a hop prompt reached the UI");

        let target = rig.ask("ops@192.0.2.91's password: ", DESTINATION);
        rig.poll_until(&mut stored, |_, s| s.is_none());
        assert_eq!(target.join().unwrap().unwrap(), "target-pw");
        assert!(rig.auth.request.is_none());
        // Fetched when each hop asked, nothing else.
        assert_eq!(*asked.lock().unwrap(), ["hop-1:Password", "hop-2:Password"]);
    }

    #[test]
    fn a_hop_without_a_saved_secret_asks_the_user_and_remembers_to_that_hop() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut rig = Rig::new(vec![hop("hop-1", "192.0.2.11", None, &asked)]);
        let mut stored = Some(PendingSecret::Password("target-pw".into()));
        let asking = rig.ask("ops@192.0.2.11's password: ", HOP_1);
        rig.poll_until(&mut stored, |r, _| r.auth.request.is_some());
        assert!(
            stored.is_some(),
            "the target's password must not answer a hop"
        );
        assert_eq!(
            rig.auth.request_hop().map(|h| h.name.as_str()),
            Some("hop-1")
        );
        // The save offer is for the hop host, not the destination.
        assert_eq!(
            rig.auth.save_target(PromptClass::Password),
            Some(SaveTarget::Host(11))
        );
        rig.auth.answer("typed".into(), true).unwrap();
        assert_eq!(asking.join().unwrap().unwrap(), "typed");
        let ready = rig.auth.take_ready_secrets(true);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].target, SaveTarget::Host(11));
    }

    #[test]
    fn a_prompt_that_names_no_saved_hop_is_never_answered_from_storage() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut rig = Rig::new(vec![hop("hop-1", "192.0.2.11", Some("hop-1-pw"), &asked)]);
        let mut stored = Some(PendingSecret::Password("target-pw".into()));
        // Another host, and another login on the saved host.
        for prompt in ["ops@10.9.9.9's password: ", "root@192.0.2.11's password: "] {
            let asking = rig.ask(prompt, HOP_2);
            rig.poll_until(&mut stored, |r, _| r.auth.request.is_some());
            assert!(stored.is_some());
            assert!(rig.auth.request_is_from_a_hop());
            assert_eq!(rig.auth.save_target(PromptClass::Password), None);
            rig.auth.answer("typed".into(), false).unwrap();
            assert_eq!(asking.join().unwrap().unwrap(), "typed");
        }
        assert!(asked.lock().unwrap().is_empty(), "nothing was fetched");
    }

    #[test]
    fn a_server_chosen_prompt_naming_a_hop_never_fetches_that_hops_password() {
        // A hostile bastion can make ssh show any keyboard-interactive text,
        // but ssh prefixes it `(user@host) `. A prompt that merely names hop-2
        // (or carries a banner line in front) is the user's to answer.
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut rig = Rig::new(vec![hop("hop-2", "192.0.2.12", Some("hop-2-pw"), &asked)]);
        let mut stored = Some(PendingSecret::Password("target-pw".into()));
        for prompt in [
            "(ops@192.0.2.11) ops@192.0.2.12's password: ",
            "Welcome\nops@192.0.2.12's password: ",
        ] {
            let asking = rig.ask(prompt, HOP_1);
            rig.poll_until(&mut stored, |r, _| r.auth.request.is_some());
            assert!(
                rig.auth.request_hop().is_none(),
                "{prompt:?} was taken for hop-2"
            );
            assert_eq!(rig.auth.save_target(PromptClass::Password), None);
            rig.auth.answer("typed".into(), false).unwrap();
            assert_eq!(asking.join().unwrap().unwrap(), "typed");
        }
        assert!(
            asked.lock().unwrap().is_empty(),
            "hop-2's secret was fetched"
        );
        assert!(stored.is_some());
    }

    #[test]
    fn a_wrong_stored_hop_password_is_asked_again_and_each_hop_has_its_own_three_tries() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut rig = Rig::new(vec![
            hop("hop-1", "192.0.2.11", Some("old"), &asked),
            hop("hop-2", "192.0.2.12", Some("hop-2-pw"), &asked),
        ]);
        let mut stored = None;
        // hop-1: the stored (wrong) one first, then the user twice: its 3 tries.
        let first = rig.ask("ops@192.0.2.11's password: ", HOP_1);
        rig.poll_until(&mut stored, |_, _| first.is_finished());
        assert_eq!(first.join().unwrap().unwrap(), "old");
        for n in 2..=3 {
            let asking = rig.ask("ops@192.0.2.11's password: ", HOP_1);
            rig.poll_until(&mut stored, |r, _| r.auth.request.is_some());
            assert_eq!(rig.auth.attempts, n);
            rig.auth.answer(format!("typed-{n}"), false).unwrap();
            assert_eq!(asking.join().unwrap().unwrap(), format!("typed-{n}"));
        }
        // hop-2 starts its own count at 1 (not 4): the stored one, then the user
        // twice, then it gives up on the fourth.
        let first = rig.ask("ops@192.0.2.12's password: ", HOP_2);
        rig.poll_until(&mut stored, |_, _| first.is_finished());
        assert_eq!(first.join().unwrap().unwrap(), "hop-2-pw");
        for n in 2..=3 {
            let asking = rig.ask("ops@192.0.2.12's password: ", HOP_2);
            rig.poll_until(&mut stored, |r, _| r.auth.request.is_some());
            assert_eq!(rig.auth.attempts, n, "hop-2 shares hop-1's budget");
            assert_eq!(
                rig.auth.request_hop().map(|h| h.name.as_str()),
                Some("hop-2")
            );
            rig.auth.answer(format!("typed-{n}"), false).unwrap();
            assert_eq!(asking.join().unwrap().unwrap(), format!("typed-{n}"));
        }
        let fourth = rig.ask("ops@192.0.2.12's password: ", HOP_2);
        rig.poll_until(&mut stored, |r, _| r.auth.cancelled());
        assert!(fourth.join().unwrap().is_err());
        // Each hop's saved password was fetched once, never re-offered.
        assert_eq!(*asked.lock().unwrap(), ["hop-1:Password", "hop-2:Password"]);
    }

    #[test]
    fn three_stored_answers_in_a_chain_do_not_use_up_the_destinations_tries() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut rig = Rig::new(vec![
            hop("hop-1", "192.0.2.11", Some("a"), &asked),
            hop("hop-2", "192.0.2.12", Some("b"), &asked),
        ]);
        let mut stored = Some(PendingSecret::Password("target-pw".into()));
        for (prompt, pid) in [
            ("ops@192.0.2.11's password: ", HOP_1),
            ("ops@192.0.2.12's password: ", HOP_2),
            ("ops@192.0.2.91's password: ", DESTINATION),
        ] {
            let asking = rig.ask(prompt, pid);
            rig.poll_until(&mut stored, |_, _| asking.is_finished());
            asking.join().unwrap().unwrap();
        }
        // The target rejected the stored password: the user still gets a modal.
        let again = rig.ask("ops@192.0.2.91's password: ", DESTINATION);
        rig.poll_until(&mut stored, |r, _| r.auth.request.is_some());
        assert!(
            !rig.auth.cancelled(),
            "one shared budget cancelled the chain"
        );
        assert_eq!(rig.auth.attempts, 2);
        rig.auth.answer("retyped".into(), false).unwrap();
        assert_eq!(again.join().unwrap().unwrap(), "retyped");
    }
}

/// Against real `ssh` and real `sshd`: a key-auth hop, a hop that asks for a
/// password, and a destination that asks for one. An unprivileged sshd refuses
/// every password, so a chain cannot get past the password host; what these
/// check is which prompt ssh raises and what answers it. Real process trees, so
/// the destination-versus-hop test runs on the real `ssh` pids.
#[cfg(test)]
mod real_ssh {
    use super::*;
    use crate::test_support::{
        current_user, tempdir_sshd, tempdir_sshd_asking_for_a_password, tool_missing,
    };
    use std::sync::{Arc, Mutex};

    /// The askpass helper's side of the channel, in Python (a test binary is no
    /// stand-in for `sshub`: libtest prints to stdout).
    const HELPER: &str = r#"#!/usr/bin/env python3
import json, os, socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(30)
s.connect(os.environ["SSHUB_ASKPASS_SOCKET"])
s.sendall((json.dumps({"token": os.environ["SSHUB_ASKPASS_TOKEN"], "prompt": sys.argv[1], "ssh_pid": os.getppid()}) + "\n").encode())
data = b""
while not data.endswith(b"\n"):
    chunk = s.recv(1024)
    if not chunk:
        sys.exit(1)
    data += chunk
reply = json.loads(data)
if reply.get("status") != "Answer":
    sys.exit(1)
print(reply["answer"])
"#;

    struct Ssh(std::process::Child);

    impl Drop for Ssh {
        fn drop(&mut self) {
            crate::cli::exec::kill_group(&self.0);
            let _ = self.0.wait();
        }
    }

    struct Chain {
        _dir: tempfile::TempDir,
        ssh: Ssh,
        auth: InteractiveAuth,
        asked: Arc<Mutex<Vec<String>>>,
    }

    /// `ssh -J <jumps> <target>` for `user@127.0.0.1:<port>` hosts, with the
    /// channel's askpass environment, and an `InteractiveAuth` whose destination
    /// is that ssh and whose saved hop is `hop_secret_for` (a host + password).
    fn chain(
        key: &std::path::Path,
        jumps: &[u16],
        target: u16,
        saved_hop: Option<(u16, &str)>,
    ) -> Chain {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("askpass.py");
        std::fs::write(&helper, HELPER).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let conf = dir.path().join("ssh_config");
        std::fs::write(
            &conf,
            format!(
                "Host *\n  IdentityFile {}\n  IdentitiesOnly yes\n  UserKnownHostsFile /dev/null\n  \
                 StrictHostKeyChecking no\n  LogLevel ERROR\n  ConnectTimeout 10\n",
                key.display()
            ),
        )
        .unwrap();
        let user = current_user();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut auth = InteractiveAuth::new().unwrap();
        auth.host_id = Some(7);
        if let Some((_, password)) = saved_hop {
            let (log, password) = (Arc::clone(&asked), password.to_string());
            auth.hops = vec![HopCred::new(
                "hop-b".into(),
                Some(user.clone()),
                "127.0.0.1".into(),
                "127.0.0.1".into(),
                Some(22),
                None,
                None,
                Arc::new(move |class| {
                    log.lock().unwrap().push(format!("hop-b:{class:?}"));
                    (class == PromptClass::Password)
                        .then(|| PendingSecret::Password(password.clone()))
                }),
            )];
        }
        let mut cmd = std::process::Command::new("ssh");
        cmd.arg("-F").arg(&conf);
        if !jumps.is_empty() {
            let list: Vec<String> = jumps
                .iter()
                .map(|p| format!("{user}@127.0.0.1:{p}"))
                .collect();
            cmd.args(["-J", &list.join(",")]);
        }
        cmd.args([
            "-p",
            &target.to_string(),
            &format!("{user}@127.0.0.1"),
            "true",
        ]);
        cmd.args(["-o", "NumberOfPasswordPrompts=3"]);
        for (k, v) in auth.channel_env(&helper) {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(dir.path().join("ssh.err")).unwrap());
        crate::cli::exec::spawn_group(&mut cmd);
        let child = cmd.spawn().expect("ssh");
        auth.destination_pid = Some(child.id());
        Chain {
            _dir: dir,
            ssh: Ssh(child),
            auth,
            asked,
        }
    }

    impl Chain {
        fn poll_until(
            &mut self,
            stored: &mut Option<PendingSecret>,
            done: impl Fn(&Self, &Option<PendingSecret>) -> bool,
        ) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !done(self, stored) {
                self.auth.poll(stored);
                assert!(
                    std::time::Instant::now() < deadline
                        && self.ssh.0.try_wait().unwrap().is_none(),
                    "ssh ended or timed out before the prompt: asked {:?}: {}",
                    self.asked.lock().unwrap(),
                    std::fs::read_to_string(self._dir.path().join("ssh.err")).unwrap_or_default()
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn have_python() -> bool {
        std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_ok()
            || {
                tool_missing("python3");
                false
            }
    }

    #[test]
    fn a_hop_that_asks_for_a_password_is_answered_from_that_hosts_own_store() {
        if !have_python() {
            return;
        }
        let (Some(key_hop), Some(pw_hop)) = (tempdir_sshd(), tempdir_sshd_asking_for_a_password())
        else {
            return;
        };
        // you -> key hop -> password hop -> target. The target is never reached
        // (an unprivileged sshd cannot accept a password), the password hop's
        // prompt is the event.
        let mut run = chain(
            &key_hop.user_key,
            &[key_hop.port, pw_hop.port],
            key_hop.port,
            Some((pw_hop.port, "hop-b-secret")),
        );
        let mut stored = Some(PendingSecret::Password("target-secret".into()));
        run.poll_until(&mut stored, |r, _| {
            let answered = !r.asked.lock().unwrap().is_empty();
            assert!(
                answered || r.auth.request.is_none(),
                "the hop's prompt reached the UI instead of its own store: {:?}",
                r.auth.request.as_ref().map(|q| q.prompt.as_str())
            );
            answered
        });
        // The hop's own secret was fetched for the hop's prompt, and no modal
        // opened for it: the user is not typing a password SSHub already has.
        assert_eq!(*run.asked.lock().unwrap(), ["hop-b:Password"]);
        assert!(
            run.auth.request.is_none(),
            "the hop's prompt reached the UI"
        );
        // The destination's password stayed where it was.
        assert!(stored.is_some(), "the target's password went to a hop");
        // The pw host refuses it, ssh asks again: now it is the user's turn,
        // with the hop named and the answer saved to that hop.
        run.poll_until(&mut stored, |r, _| r.auth.request.is_some());
        assert_eq!(
            run.auth.request_hop().map(|h| h.name.as_str()),
            Some("hop-b")
        );
        assert_eq!(
            run.auth.save_target(PromptClass::Password),
            Some(SaveTarget::Host(22))
        );
        assert!(stored.is_some());
        assert_eq!(run.auth.attempts, 2);
    }

    #[test]
    fn the_destination_still_gets_its_own_password_and_no_hop_is_asked() {
        if !have_python() {
            return;
        }
        let (Some(key_hop), Some(target)) = (tempdir_sshd(), tempdir_sshd_asking_for_a_password())
        else {
            return;
        };
        let mut run = chain(
            &key_hop.user_key,
            &[key_hop.port],
            target.port,
            // A saved hop with the same login and address: it is not what asks.
            Some((key_hop.port, "hop-b-secret")),
        );
        let mut stored = Some(PendingSecret::Password("target-secret".into()));
        run.poll_until(&mut stored, |_, s| s.is_none());
        assert!(
            run.asked.lock().unwrap().is_empty(),
            "a hop's secret was fetched for the target"
        );
        assert!(run.auth.request.is_none());
    }
}
