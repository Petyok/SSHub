//! Stable replicated ids (host sync, contract C1).
//!
//! A [`Uid`] is 16 random bytes that never changes and never depends on the
//! device: it is what ops, frames and challenges name an entity by. The local
//! integer `id` stays the row key and the foreign-key target. Rows get their uid
//! from the v22 migration (existing rows) or an `AFTER INSERT` trigger (new
//! rows); this module only reads and resolves them.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::LauncherStore;

pub use sshub_sync_proto::uid::*;

fn uid_of(raw: Vec<u8>) -> Result<Uid> {
    raw.try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("stored uid has {} bytes, expected 16", v.len()))
}

macro_rules! uid_lookups {
    ($table:literal, $uid_fn:ident, $id_fn:ident, $uid_on:ident, $id_on:ident) => {
        pub(crate) fn $uid_on(conn: &Connection, id: i64) -> Result<Option<Uid>> {
            conn.query_row(
                concat!("SELECT uid FROM ", $table, " WHERE id = ?1"),
                params![id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(uid_of)
            .transpose()
        }

        pub(crate) fn $id_on(conn: &Connection, uid: &Uid) -> Result<Option<i64>> {
            conn.query_row(
                concat!("SELECT id FROM ", $table, " WHERE uid = ?1"),
                params![&uid[..]],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        }

        impl LauncherStore {
            /// Stable uid of the local row `id`, `None` when the row is gone.
            pub fn $uid_fn(&self, id: i64) -> Result<Option<Uid>> {
                self.with_conn(|conn| $uid_on(conn, id))
            }

            /// Local row id of `uid`, `None` when this store has no such row.
            pub fn $id_fn(&self, uid: &Uid) -> Result<Option<i64>> {
                self.with_conn(|conn| $id_on(conn, uid))
            }
        }
    };
}

uid_lookups!(
    "hosts",
    host_uid,
    host_id_by_uid,
    host_uid_on,
    host_id_by_uid_on
);
uid_lookups!(
    "host_groups",
    group_uid,
    group_id_by_uid,
    group_uid_on,
    group_id_by_uid_on
);
uid_lookups!(
    "identities",
    identity_uid,
    identity_id_by_uid,
    identity_uid_on,
    identity_id_by_uid_on
);

impl LauncherStore {
    /// The reserved Favorites group's uid (the same on every device).
    pub fn favorites_group_uid(&self) -> Result<Uid> {
        let id = self.favorites_group_id()?;
        self.group_uid(id)?.context("Favorites group has no uid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewHost, NewHostGroup, NewIdentity};

    #[test]
    fn well_known_uids_are_sixteen_bytes_and_distinct() {
        assert_ne!(FAVORITES_GROUP_UID, DEFAULT_IDENTITY_UID);
        assert_eq!(uid_hex(&FAVORITES_GROUP_UID).len(), 32);
        assert_eq!(
            uid_from_hex(&uid_hex(&DEFAULT_IDENTITY_UID)),
            Some(DEFAULT_IDENTITY_UID)
        );
        assert_eq!(uid_from_hex("AB"), None);
        assert_eq!(uid_from_hex(&"A".repeat(32)), None);
    }

    #[test]
    fn seeded_rows_get_the_well_known_uids() {
        let a = LauncherStore::open_in_memory().unwrap();
        let b = LauncherStore::open_in_memory().unwrap();
        assert_eq!(a.favorites_group_uid().unwrap(), FAVORITES_GROUP_UID);
        assert_eq!(b.favorites_group_uid().unwrap(), FAVORITES_GROUP_UID);
        for s in [&a, &b] {
            let default = s.get_identity_by_name("Default").unwrap().unwrap();
            assert_eq!(
                s.identity_uid(default.id).unwrap(),
                Some(DEFAULT_IDENTITY_UID)
            );
        }
    }

    #[test]
    fn new_rows_get_distinct_uids_that_resolve_back() {
        let s = LauncherStore::open_in_memory().unwrap();
        let h1 = s.create_host(&NewHost::launcher("a", "10.0.0.1")).unwrap();
        let h2 = s.create_host(&NewHost::launcher("b", "10.0.0.2")).unwrap();
        let g = s
            .create_group(&NewHostGroup {
                name: "g".into(),
                ..Default::default()
            })
            .unwrap();
        let i = s
            .create_identity(&NewIdentity {
                name: "i".into(),
                ..Default::default()
            })
            .unwrap();
        let (u1, u2) = (
            s.host_uid(h1.id).unwrap().unwrap(),
            s.host_uid(h2.id).unwrap().unwrap(),
        );
        assert_ne!(u1, u2);
        assert_eq!(s.host_id_by_uid(&u1).unwrap(), Some(h1.id));
        assert_eq!(s.host_id_by_uid(&[9; 16]).unwrap(), None);
        let gu = s.group_uid(g.id).unwrap().unwrap();
        assert_eq!(s.group_id_by_uid(&gu).unwrap(), Some(g.id));
        let iu = s.identity_uid(i.id).unwrap().unwrap();
        assert_eq!(s.identity_id_by_uid(&iu).unwrap(), Some(i.id));
        assert_eq!(s.host_uid(9999).unwrap(), None);
    }
}
