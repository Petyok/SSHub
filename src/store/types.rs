use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGroup {
    pub id: i64,
    pub name: String,
    pub sort_order: i32,
    /// Identity new hosts in this group inherit by default. `None` = no default.
    pub default_identity_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
    pub private_key: Option<PathBuf>,
    pub certificate: Option<PathBuf>,
    pub has_password: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostSource {
    #[default]
    Launcher,
    SshConfig,
}

impl HostSource {
    pub fn as_str(self) -> &'static str {
        match self {
            HostSource::Launcher => "launcher",
            HostSource::SshConfig => "ssh_config",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "launcher" => Some(HostSource::Launcher),
            "ssh_config" => Some(HostSource::SshConfig),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedHost {
    pub id: i64,
    pub name: String,
    pub label: Option<String>,
    pub address: String,
    pub port: u16,
    pub group_id: Option<i64>,
    pub identity_id: Option<i64>,
    pub group: Option<HostGroup>,
    pub identity: Option<Identity>,
    pub os_icon: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub proxy_jump: Option<String>,
    pub forward_agent: bool,
    pub remote_command: Option<String>,
    pub environment: Option<String>,
    pub sort_order: i32,
    pub favorite: bool,
    pub last_connected: Option<i64>,
    pub source: HostSource,
    pub ssh_config_hash: Option<String>,
    pub has_password: bool,
    pub username: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Fields required to create a launcher-native host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHost {
    pub name: String,
    pub label: Option<String>,
    pub address: String,
    pub port: u16,
    pub group_id: Option<i64>,
    pub identity_id: Option<i64>,
    pub os_icon: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub proxy_jump: Option<String>,
    pub forward_agent: bool,
    pub remote_command: Option<String>,
    pub source: HostSource,
    pub has_password: bool,
    pub username: Option<String>,
}

impl Default for NewHost {
    fn default() -> Self {
        Self::launcher("", "")
    }
}

impl NewHost {
    pub fn launcher(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            label: None,
            port: 22,
            group_id: None,
            identity_id: None,
            os_icon: None,
            tags: Vec::new(),
            notes: None,
            proxy_jump: None,
            forward_agent: false,
            remote_command: None,
            source: HostSource::Launcher,
            has_password: false,
            username: None,
        }
    }
}

/// Partial host update (launcher rows only in later phases).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostUpdate {
    pub name: Option<String>,
    pub label: Option<Option<String>>,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub group_id: Option<Option<i64>>,
    pub identity_id: Option<Option<i64>>,
    pub os_icon: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<Option<String>>,
    pub proxy_jump: Option<Option<String>>,
    pub forward_agent: Option<bool>,
    pub remote_command: Option<Option<String>>,
    pub environment: Option<Option<String>>,
    pub favorite: Option<bool>,
    pub sort_order: Option<i32>,
    pub has_password: Option<bool>,
    pub username: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewHostGroup {
    pub name: String,
    pub sort_order: i32,
    pub default_identity_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostGroupUpdate {
    pub name: Option<String>,
    pub sort_order: Option<i32>,
    /// Outer `Some` = change the default identity; inner `None` = clear it.
    pub default_identity_id: Option<Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewIdentity {
    pub name: String,
    pub username: Option<String>,
    pub private_key: Option<PathBuf>,
    pub certificate: Option<PathBuf>,
    pub sort_order: i32,
    pub has_password: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityUpdate {
    pub name: Option<String>,
    pub username: Option<Option<String>>,
    pub private_key: Option<Option<PathBuf>>,
    pub certificate: Option<Option<PathBuf>>,
    pub sort_order: Option<i32>,
    pub has_password: Option<bool>,
}

/// Result of attempting to delete an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteIdentityOutcome {
    Deleted,
    NotFound,
    InUse { host_count: usize },
}

/// Result of attempting to delete a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteHostOutcome {
    Deleted,
    NotFound,
    NotLauncher,
}

/// Resolved ssh config host ready for DB upsert.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshConfigHostImport {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub proxy_jump: Option<String>,
    pub forward_agent: bool,
    pub remote_command: Option<String>,
    pub ssh_config_hash: String,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub environment: Option<String>,
    pub favorite: bool,
    pub last_connected: Option<i64>,
}

/// Outcome of upserting one imported ssh config host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertSshConfigOutcome {
    Inserted,
    Updated,
    SkippedLauncher,
}

/// A recorded authentication / connection event.
#[derive(Debug, Clone)]
pub struct AuthEvent {
    pub id: i64,
    pub host_name: String,
    pub username: Option<String>,
    pub via: Option<String>,
    pub status: String,
    pub note: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunnelType {
    #[default]
    Local,
    Remote,
    Dynamic,
}

impl TunnelType {
    pub fn as_str(self) -> &'static str {
        match self {
            TunnelType::Local => "L",
            TunnelType::Remote => "R",
            TunnelType::Dynamic => "D",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "R" => TunnelType::Remote,
            "D" => TunnelType::Dynamic,
            _ => TunnelType::Local,
        }
    }

    pub fn next(self) -> Self {
        match self {
            TunnelType::Local => TunnelType::Remote,
            TunnelType::Remote => TunnelType::Dynamic,
            TunnelType::Dynamic => TunnelType::Local,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TunnelType::Local => "Local (-L)",
            TunnelType::Remote => "Remote (-R)",
            TunnelType::Dynamic => "Dynamic (-D)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tunnel {
    pub id: i64,
    pub host_id: Option<i64>,
    pub tunnel_type: TunnelType,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub label: Option<String>,
    pub auto_connect: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct NewTunnel {
    pub host_id: Option<i64>,
    pub tunnel_type: TunnelType,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub label: Option<String>,
    pub auto_connect: bool,
}
