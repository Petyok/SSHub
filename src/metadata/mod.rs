mod db;

pub use db::{MetadataDb, MetadataStore};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostMetadata {
    pub host_name: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub environment: Option<String>,
    pub favorite: bool,
    pub last_connected: Option<i64>,
}

impl HostMetadata {
    pub fn new(host_name: impl Into<String>) -> Self {
        Self {
            host_name: host_name.into(),
            ..Default::default()
        }
    }
}
