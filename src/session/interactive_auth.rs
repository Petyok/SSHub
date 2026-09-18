//! One session's authentication state. Answers never travel through the PTY.

use std::path::Path;

use super::askpass_channel::{Channel, Request};
use super::auth_prompt::PromptClass;
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

/// Deliberately has no Debug/Clone implementation: do not expose or duplicate
/// submitted credentials while waiting for the actual authentication marker.
pub struct RememberedSecret {
    pub target: SaveTarget,
    pub value: String,
}

pub struct InteractiveAuth {
    channel: Option<Channel>,
    pub request: Option<Request>,
    pub attempts: u8,
    host_questions: u8,
    pub host_id: Option<i64>,
    pub identity_id: Option<i64>,
    remembered: Vec<RememberedSecret>,
    pub changed_key_shown: bool,
}

impl InteractiveAuth {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            channel: Some(Channel::new()?),
            request: None,
            attempts: 0,
            host_questions: 0,
            host_id: None,
            identity_id: None,
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
        // Host verification is not a password attempt. Bound its own retries,
        // too, so malformed host responses cannot create an infinite modal loop.
        let count = if class == PromptClass::HostKey {
            &mut self.host_questions
        } else {
            &mut self.attempts
        };
        if *count >= 3 {
            self.cancel();
            return;
        }
        *count += 1;
        // A re-prompt invalidates the previous value of that class, even if the
        // user cancels or unchecks remember on the next attempt.
        self.remembered.retain(|secret| {
            !matches!(
                (class, secret.target),
                (PromptClass::Password, SaveTarget::Host(_))
                    | (PromptClass::Passphrase, SaveTarget::Identity(_))
            )
        });
        if stored
            .as_ref()
            .is_some_and(|secret| class.matches_secret(secret))
        {
            if let Some(secret) = stored.take() {
                if request.answer(secret.value()).is_err() {
                    self.cancel();
                }
            }
        } else {
            self.request = Some(request);
        }
    }

    pub fn save_target(&self, class: PromptClass) -> Option<SaveTarget> {
        SaveTarget::for_prompt(class, self.host_id, self.identity_id)
    }

    pub fn answer(&mut self, value: String, remember: bool) -> std::io::Result<()> {
        let Some(request) = self.request.take() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "authentication prompt ended",
            ));
        };
        let class = PromptClass::classify(&request.prompt);
        let target = self.save_target(class);
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
            host_questions: 0,
            host_id: Some(7),
            identity_id: None,
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
            std::time::Duration::from_secs(2),
        )
        .is_err());
        let ready = auth.take_ready_secrets(true);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].value, "private-value");
    }
}
