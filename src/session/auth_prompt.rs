//! Classification shared by interactive askpass and stored-credential delivery.
//! Generic challenges deliberately have no credential-storage destination.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptClass {
    Password,
    Passphrase,
    HostKey,
    Generic,
}

impl PromptClass {
    pub fn classify(prompt: &str) -> Self {
        let lower = prompt.to_ascii_lowercase();
        // Trust decisions take precedence over credential needles in host text.
        if super::HOST_VERIFY_NEEDLES
            .iter()
            .any(|needle| lower.contains(needle))
            || lower.contains("please type 'yes', 'no' or the fingerprint")
        {
            Self::HostKey
        } else if super::PASSPHRASE_NEEDLES
            .iter()
            .any(|needle| lower.contains(needle))
        {
            Self::Passphrase
        } else if super::PASSWORD_NEEDLES
            .iter()
            .any(|needle| lower.contains(needle))
        {
            Self::Password
        } else {
            Self::Generic
        }
    }

    pub fn matches_secret(self, secret: &super::PendingSecret) -> bool {
        matches!(
            (self, secret),
            (Self::Password, super::PendingSecret::Password(_))
                | (Self::Passphrase, super::PendingSecret::Passphrase(_))
        )
    }
}

/// Display fields from OpenSSH's unknown-key question, without normalizing the
/// fingerprint. The complete prompt remains available when parsing is partial.
#[derive(Debug, PartialEq, Eq)]
pub struct HostKeyDetails<'a> {
    pub host: Option<&'a str>,
    pub key_type: Option<&'a str>,
    pub fingerprint: Option<&'a str>,
}

impl<'a> HostKeyDetails<'a> {
    pub fn parse(prompt: &'a str) -> Self {
        let host = prompt
            .split_once("The authenticity of host '")
            .and_then(|(_, rest)| rest.split_once("' can't be established."))
            .map(|(host, _)| host);
        let key = prompt.lines().find_map(|line| {
            let (kind, fingerprint) = line.split_once(" key fingerprint is: ")?;
            let fingerprint = fingerprint.strip_suffix('.').unwrap_or(fingerprint);
            Some((kind, fingerprint))
        });
        Self {
            host,
            key_type: key.map(|(kind, _)| kind),
            fingerprint: key.map(|(_, fingerprint)| fingerprint),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::PendingSecret;

    // Captured with OpenSSH 10.4 against a throwaway sshd by the maintainer;
    // copied verbatim from issue #52, not synthesized from this parser.
    const UNKNOWN_KEY: &str = "The authenticity of host '[127.0.0.1]:2222 ([127.0.0.1]:2222)' can't be established.\nED25519 key fingerprint is: SHA256:MXUTTgmDROalky/NE8jlcGeW+758bjK3+hOz3jbY4l4\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])?";

    #[test]
    fn every_existing_needle_maps_to_its_prompt_class() {
        for (needles, expected) in [
            (crate::session::PASSWORD_NEEDLES, PromptClass::Password),
            (crate::session::PASSPHRASE_NEEDLES, PromptClass::Passphrase),
            (crate::session::HOST_VERIFY_NEEDLES, PromptClass::HostKey),
        ] {
            for needle in needles {
                assert_eq!(PromptClass::classify(needle), expected);
                assert_eq!(PromptClass::classify(&needle.to_uppercase()), expected);
            }
        }
    }

    #[test]
    fn stored_credentials_only_answer_their_own_class() {
        let password = PendingSecret::Password("not-a-real-password".into());
        let passphrase = PendingSecret::Passphrase("not-a-real-passphrase".into());
        for class in [
            PromptClass::Password,
            PromptClass::Passphrase,
            PromptClass::HostKey,
            PromptClass::Generic,
        ] {
            assert_eq!(
                class.matches_secret(&password),
                class == PromptClass::Password
            );
            assert_eq!(
                class.matches_secret(&passphrase),
                class == PromptClass::Passphrase
            );
        }
        assert_eq!(
            PromptClass::classify("Verification code:"),
            PromptClass::Generic
        );
        assert!(!PromptClass::classify(UNKNOWN_KEY).matches_secret(&password));
    }

    #[test]
    fn measured_host_key_prompt_preserves_fingerprint() {
        assert_eq!(PromptClass::classify(UNKNOWN_KEY), PromptClass::HostKey);
        assert_eq!(
            HostKeyDetails::parse(UNKNOWN_KEY),
            HostKeyDetails {
                host: Some("[127.0.0.1]:2222 ([127.0.0.1]:2222)"),
                key_type: Some("ED25519"),
                fingerprint: Some("SHA256:MXUTTgmDROalky/NE8jlcGeW+758bjK3+hOz3jbY4l4"),
            }
        );
    }

    #[test]
    fn host_key_reprompt_never_receives_a_credential() {
        assert_eq!(
            PromptClass::classify("Please type 'yes', 'no' or the fingerprint:"),
            PromptClass::HostKey
        );
        assert_eq!(
            PromptClass::classify("The authenticity of host 'password:' can't be established."),
            PromptClass::HostKey
        );
    }
}
