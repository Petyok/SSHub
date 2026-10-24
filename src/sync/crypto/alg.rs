//! Algorithm tags. Mandatory in every op and every wrapped blob from the first
//! release: without them a post-quantum transition needs a new protocol.

use anyhow::{bail, Result};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SigSuite {
    Ed25519 = 1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum WrapSuite {
    X25519MlKem768 = 1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AlgTag {
    pub sig: SigSuite,
    pub wrap: WrapSuite,
}

pub const ALG_CURRENT: AlgTag = AlgTag {
    sig: SigSuite::Ed25519,
    wrap: WrapSuite::X25519MlKem768,
};

impl AlgTag {
    /// Two-byte wire form: signature suite, wrap suite.
    pub const fn to_bytes(self) -> [u8; 2] {
        [self.sig as u8, self.wrap as u8]
    }

    /// Parses the two-byte wire form; unknown suites are rejected with a legible error.
    pub fn from_bytes(b: [u8; 2]) -> Result<Self> {
        let sig = match b[0] {
            1 => SigSuite::Ed25519,
            v => bail!("unknown signature suite {v}"),
        };
        let wrap = match b[1] {
            1 => WrapSuite::X25519MlKem768,
            v => bail!("unknown wrap suite {v}"),
        };
        Ok(Self { sig, wrap })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_form_round_trips_and_rejects_unknown_suites() {
        assert_eq!(
            AlgTag::from_bytes(ALG_CURRENT.to_bytes()).unwrap(),
            ALG_CURRENT
        );
        assert_eq!(ALG_CURRENT.to_bytes(), [1, 1]);
        assert!(AlgTag::from_bytes([2, 1]).is_err());
        assert!(AlgTag::from_bytes([1, 0]).is_err());
    }
}
