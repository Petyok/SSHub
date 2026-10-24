//! The paper phrase: 24 BIP39 words that derive TWO independent identities under
//! separate domain tags. Typing the phrase to unlock a credential hands a machine
//! `Unwrap`; it must not also hand it the ability to authorise a recovery (`Recover`).

use anyhow::Result;
use zeroize::Zeroizing;

use super::identity::DeviceIdentity;
use super::words::bip39_decode;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaperDomain {
    Unwrap,
    Recover,
}

impl PaperDomain {
    pub(crate) fn context(self) -> &'static str {
        match self {
            PaperDomain::Unwrap => "sshub/sync/paper/unwrap/v1",
            PaperDomain::Recover => "sshub/sync/paper/recover/v1",
        }
    }
}

/// Derives the identity for one domain from the 24 word phrase.
pub fn paper_derive(phrase: &str, domain: PaperDomain) -> Result<DeviceIdentity> {
    let entropy = Zeroizing::new(bip39_decode(phrase)?);
    Ok(DeviceIdentity::from_seed(&entropy, domain.context()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::crypto::bip39_encode;

    #[test]
    fn domains_are_independent_and_deterministic() {
        let phrase = bip39_encode(&[0x42; 32]);
        let u = paper_derive(&phrase, PaperDomain::Unwrap).unwrap().public();
        let r = paper_derive(&phrase, PaperDomain::Recover)
            .unwrap()
            .public();
        assert_ne!(u.ed25519_pub, r.ed25519_pub);
        assert_ne!(u.x25519_pub, r.x25519_pub);
        assert_ne!(u.mlkem_pub, r.mlkem_pub);
        assert_eq!(
            u,
            paper_derive(&phrase, PaperDomain::Unwrap).unwrap().public()
        );
        assert_eq!(
            u,
            paper_derive(&phrase.to_uppercase(), PaperDomain::Unwrap)
                .unwrap()
                .public()
        );
    }

    #[test]
    fn a_bad_phrase_is_an_error() {
        assert!(paper_derive("abandon abandon abandon", PaperDomain::Unwrap).is_err());
    }
}
