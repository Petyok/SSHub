//! Bulk credential export (C9, Red).
//!
//! A `BulkExport { requester, window_secs }` op, once the ceremony approved it,
//! lets the requester open every credential for `window_secs` (at most 3600)
//! from the op's timestamp, through the normal unlock paths. This module is the
//! requester's side: it finds the open window, calls the caller's `unlock` for
//! each host, and writes a Termius style `L00t.csv` (`Label,Host,Port,Username,
//! Password,SSH_Key,OS`), the one importer format of SSHub that carries
//! passwords. The file is created 0600 and nothing is written outside the
//! window: the clock is read again before the write.

use std::io::Write;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use rusqlite::params;

use super::oplog::merge::load_op;
use super::oplog::{DeviceId, HostUid, OpBody};
use crate::store::{HostSource, LauncherStore};

/// An approved export window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportWindow {
    pub op_hash: [u8; 32],
    pub opened_at: i64,
    pub window_secs: u32,
}

impl ExportWindow {
    pub fn contains(&self, now: i64) -> bool {
        now >= self.opened_at && now < self.opened_at + i64::from(self.window_secs)
    }
}

/// `(op hash, requester, timestamp, window_secs)`.
type ExportOp = ([u8; 32], DeviceId, i64, u32);

fn approved_exports(store: &LauncherStore) -> Result<Vec<ExportOp>> {
    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT op_hash FROM sync_ops WHERE kind = 'BulkExport' AND state IN ('approved','applied')
             ORDER BY lamport",
        )?;
        let hashes: Vec<Vec<u8>> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        let mut out = Vec::new();
        for h in hashes {
            let h: [u8; 32] = h.try_into().map_err(|_| anyhow::anyhow!("stored hash"))?;
            if let Some(row) = load_op(conn, &h)? {
                if let OpBody::BulkExport {
                    requester,
                    window_secs,
                } = row.op.body
                {
                    out.push((h, requester, row.op.timestamp, window_secs));
                }
            }
        }
        Ok(out)
    })
}

/// The newest approved window of `requester` that contains `now`.
pub fn active_window(
    store: &LauncherStore,
    requester: &DeviceId,
    now: i64,
) -> Result<Option<ExportWindow>> {
    Ok(approved_exports(store)?
        .into_iter()
        .filter(|(_, r, _, _)| r == requester)
        .map(|(op_hash, _, opened_at, window_secs)| ExportWindow {
            op_hash,
            opened_at,
            window_secs,
        })
        .filter(|w| w.contains(now))
        .max_by_key(|w| w.opened_at))
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExportReport {
    pub written: usize,
    /// Hosts whose credential `unlock` could not open (no secret stored, refused).
    pub skipped: usize,
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Export every launcher host with its opened credential to `dest`.
///
/// `unlock` is the caller's normal unlock path for one credential (peer, phone,
/// paper, ...); `None` means the host has no stored secret. `clock` is read
/// before the window check and again before the file is written.
pub fn export_all(
    store: &LauncherStore,
    requester: &DeviceId,
    clock: &dyn Fn() -> i64,
    unlock: &mut dyn FnMut(&HostUid) -> Result<Option<String>>,
    dest: &Path,
) -> Result<ExportReport> {
    let Some(window) = active_window(store, requester, clock())? else {
        bail!("no approved export window is open for this device");
    };
    let mut csv = String::from("Label,Host,Port,Username,Password,SSH_Key,OS\n");
    let (mut written, mut skipped) = (0, 0);
    for host in store.list_hosts()? {
        if host.source != HostSource::Launcher {
            continue;
        }
        let Some(uid) = store.host_uid(host.id)? else {
            continue;
        };
        ensure!(
            window.contains(clock()),
            "the export window closed before all credentials were opened"
        );
        let Some(password) = unlock(&uid)? else {
            skipped += 1;
            continue;
        };
        csv.push_str(&format!(
            "{},{},{},{},{},,{}\n",
            csv_field(&host.name),
            csv_field(&host.address),
            host.port.unwrap_or(22),
            csv_field(host.username.as_deref().unwrap_or("")),
            csv_field(&password),
            csv_field(host.os_icon.as_deref().unwrap_or("")),
        ));
        written += 1;
    }
    ensure!(
        window.contains(clock()),
        "the export window closed before the file was written"
    );
    write_private(dest, csv.as_bytes())?;
    Ok(ExportReport { written, skipped })
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .with_context(|| format!("create {} (it must not exist yet)", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Tell the user about export ops they have not been told about yet, once per
/// op, through `send(subject, body)` (the `[sync.notify]` hook). A failing send
/// is retried on the next round.
pub fn announce_exports(
    store: &LauncherStore,
    send: &mut dyn FnMut(&str, &str) -> Result<()>,
) -> Result<usize> {
    let mut sent = 0;
    for (hash, requester, ts, secs) in approved_exports(store)? {
        let key = format!(
            "export-notified:{}",
            hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let known = store.with_conn(|c| {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_meta WHERE key = ?1)",
                params![key],
                |r| r.get::<_, bool>(0),
            )?)
        })?;
        if known {
            continue;
        }
        let subject = "SSHub: bulk credential export approved";
        let body = format!(
            "Device {requester} may open every credential for {secs} seconds from unix time {ts}.\n\
             If you did not approve this, revoke the device now."
        );
        if send(subject, &body).is_ok() {
            store.with_conn(|c| {
                c.execute(
                    "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, x'01')",
                    params![key],
                )?;
                Ok(())
            })?;
            sent += 1;
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::termius_csv::parse_loot_csv;
    use crate::store::NewHost;
    use crate::sync::crypto::DeviceIdentity;
    use crate::sync::oplog::{Attendance, DeviceKind, DeviceRole, VerifiedBy};
    use std::cell::Cell;

    fn setup() -> (LauncherStore, DeviceIdentity, DeviceId) {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = DeviceIdentity::generate();
        let did = id.public().device_id();
        store
            .sync_register_device(
                &id.public(),
                "me",
                DeviceRole::Holder,
                Attendance::Attended,
                DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        for (n, user) in [("web", Some("root")), ("db, main", None)] {
            let mut h = NewHost::launcher(n, "10.0.0.1");
            h.username = user.map(String::from);
            h.has_password = true;
            store.create_host(&h).unwrap();
        }
        (store, id, did)
    }

    fn open_window(store: &LauncherStore, id: &DeviceIdentity, did: &DeviceId, at: i64, secs: u32) {
        store
            .sync_append_local_at(
                id,
                OpBody::BulkExport {
                    requester: did.clone(),
                    window_secs: secs,
                },
                at,
            )
            .unwrap();
    }

    fn secrets(store: &LauncherStore) -> impl FnMut(&HostUid) -> Result<Option<String>> + '_ {
        move |uid| {
            let host = store
                .get_host(store.host_id_by_uid(uid)?.unwrap())?
                .unwrap();
            Ok(Some(format!("pw,\"{}\"\nline2", host.name)))
        }
    }

    #[test]
    fn inside_the_window_it_writes_a_private_file_the_importer_reads() {
        let (store, id, did) = setup();
        open_window(&store, &id, &did, 1_000, 60);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("L00t.csv");
        let report = export_all(&store, &did, &|| 1_030, &mut secrets(&store), &dest).unwrap();
        assert_eq!(
            report,
            ExportReport {
                written: 2,
                skipped: 0
            }
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let rows = parse_loot_csv(&std::fs::read_to_string(&dest).unwrap());
        assert_eq!(rows.len(), 2);
        let web = rows.iter().find(|r| r.label == "web").unwrap();
        assert_eq!(
            (web.host.as_str(), web.port, web.username.as_str()),
            ("10.0.0.1", 22, "root")
        );
        assert_eq!(
            web.password, "pw,\"web\"\nline2",
            "commas, quotes and newlines survive"
        );
        assert!(rows.iter().any(|r| r.label == "db, main"));
        // It refuses to overwrite an existing file.
        assert!(export_all(&store, &did, &|| 1_031, &mut secrets(&store), &dest).is_err());
    }

    #[test]
    fn outside_the_window_or_for_another_device_nothing_is_written() {
        let (store, id, did) = setup();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.csv");
        let mut none =
            |_: &HostUid| -> Result<Option<String>> { panic!("no credential may be opened") };
        // No op at all.
        assert!(export_all(&store, &did, &|| 1_000, &mut none, &dest).is_err());
        open_window(&store, &id, &did, 1_000, 60);
        for now in [999, 1_060, 5_000] {
            assert!(
                export_all(&store, &did, &|| now, &mut none, &dest).is_err(),
                "t={now}"
            );
        }
        let other = "0011223344556677".to_string();
        assert!(export_all(&store, &other, &|| 1_010, &mut none, &dest).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn a_window_that_closes_mid_export_writes_nothing() {
        let (store, id, did) = setup();
        open_window(&store, &id, &did, 1_000, 60);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.csv");
        let t = Cell::new(1_000);
        let mut open = |_: &HostUid| -> Result<Option<String>> {
            t.set(t.get() + 40); // each unlock takes 40 s
            Ok(Some("pw".into()))
        };
        let err = export_all(&store, &did, &|| t.get(), &mut open, &dest).unwrap_err();
        assert!(err.to_string().contains("window closed"), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn a_window_over_an_hour_is_refused_and_exports_are_announced_once() {
        let (store, id, did) = setup();
        assert!(store
            .sync_append_local_at(
                &id,
                OpBody::BulkExport {
                    requester: did.clone(),
                    window_secs: 3601
                },
                1
            )
            .is_err());
        open_window(&store, &id, &did, 1_000, 3600);
        let mut subjects = Vec::new();
        let n = announce_exports(&store, &mut |s, b| {
            subjects.push((s.to_string(), b.to_string()));
            Ok(())
        })
        .unwrap();
        assert_eq!(n, 1);
        assert!(subjects[0].1.contains(&did) && subjects[0].1.contains("3600"));
        assert_eq!(announce_exports(&store, &mut |_, _| Ok(())).unwrap(), 0);
        // A failing mail server is retried next time.
        open_window(&store, &id, &did, 2_000, 60);
        assert_eq!(
            announce_exports(&store, &mut |_, _| anyhow::bail!("down")).unwrap(),
            0
        );
        assert_eq!(announce_exports(&store, &mut |_, _| Ok(())).unwrap(), 1);
    }
}
