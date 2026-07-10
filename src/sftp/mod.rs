//! Native SFTP browser: pure UI state ([`model`]), a libssh2 transport
//! ([`transport`]), and a background worker thread ([`worker`]) that mirrors
//! the ping worker pattern. The synchronous UI event loop never blocks — it
//! sends [`SftpCommand`]s and drains [`SftpEvent`]s.

pub mod model;
pub mod transport;
pub mod worker;

// Re-export the message enums so callers can `use crate::sftp::{SftpCommand, SftpEvent}`.
pub use worker::{spawn_sftp_worker, SftpCommand, SftpEvent};
