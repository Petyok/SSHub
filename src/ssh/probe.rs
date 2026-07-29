//! SSH log entry types used by the connect/session logging paths.

#[derive(Debug, Clone)]
pub struct SshLogEntry {
    pub host_name: String,
    pub line: String,
    pub level: LogLevel,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Error,
}
