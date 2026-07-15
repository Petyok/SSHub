//! Per-host session transport (`ssh` vs `mosh`) for embedded sessions and
//! external terminal launchers. Tunnels and SFTP always use `ssh`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionTransport {
    #[default]
    Ssh,
    Mosh,
}

impl SessionTransport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Mosh => "mosh",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Ssh => Self::Mosh,
            Self::Mosh => Self::Ssh,
        }
    }

    pub fn from_db(value: Option<&str>) -> Self {
        match value {
            Some("mosh") => Self::Mosh,
            _ => Self::Ssh,
        }
    }

    pub fn to_db(self) -> &'static str {
        self.label()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_db() {
        assert_eq!(
            SessionTransport::from_db(Some(SessionTransport::Mosh.to_db())),
            SessionTransport::Mosh
        );
        assert_eq!(
            SessionTransport::from_db(Some(SessionTransport::Ssh.to_db())),
            SessionTransport::Ssh
        );
        assert_eq!(SessionTransport::from_db(None), SessionTransport::Ssh);
    }
}
