//! The one error type that crosses the uniffi boundary. Everything a person must read (a refusal with its reason) is a
//! value of the views in `views.rs`, not an error: an error means "this call could not run" (locked, bad input, internal).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum CoreError {
    /// No unlocked identity: `unlock` first.
    Locked,
    /// The caller passed something this call cannot work with (wrong length, unknown token, a typed confirmation that
    /// does not match). `reason` is a sentence for the screen.
    BadInput { reason: String },
    /// A rule refused the action (for example a restored phone asked to sign before it has its chain head).
    Refused { reason: String },
    /// A bug or a damaged state. The app shows "internal error" and locks.
    Internal { reason: String },
}

impl CoreError {
    pub fn bad(reason: impl Into<String>) -> Self {
        Self::BadInput {
            reason: reason.into(),
        }
    }
    pub fn refused(reason: impl Into<String>) -> Self {
        Self::Refused {
            reason: reason.into(),
        }
    }
    pub fn internal(reason: impl Into<String>) -> Self {
        Self::Internal {
            reason: reason.into(),
        }
    }
    /// The sentence for a screen.
    pub fn reason(&self) -> &str {
        match self {
            Self::Locked => "This phone is locked.",
            Self::BadInput { reason } | Self::Refused { reason } | Self::Internal { reason } => {
                reason
            }
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for CoreError {}

impl From<anyhow::Error> for CoreError {
    fn from(e: anyhow::Error) -> Self {
        Self::BadInput {
            reason: format!("{e:#}"),
        }
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;
