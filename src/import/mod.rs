pub mod mremoteng;
pub mod putty;
pub mod termius_csv;

/// Result of a third-party host import (mRemoteNG, PuTTY). Kept lean: these
/// importers only create hosts (no identities/passwords), so unlike
/// [`termius_csv::CsvImportReport`] there are no credential counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HostImportReport {
    /// Hosts newly inserted into the launcher store.
    pub imported: usize,
    /// SSH hosts skipped because a launcher host with the same name exists.
    pub skipped_existing: usize,
    /// Entries skipped because they are not SSH connections (RDP/VNC/telnet/…).
    pub skipped_non_ssh: usize,
    /// Entries the store refused: a name, address or username starting with
    /// `-`, which ssh would read as an option rather than a host (issue #101).
    /// One poisoned row does not cost the rest of the file.
    pub skipped_invalid: usize,
}
