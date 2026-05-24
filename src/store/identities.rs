use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;

use super::migrate::now_ts;
use super::types::{DeleteIdentityOutcome, Identity, IdentityUpdate, NewIdentity};
use super::LauncherStore;

const DEFAULT_IDENTITY_NAME: &str = "Default";

impl LauncherStore {
    /// Ensure the seeded `Default` identity exists.
    pub fn seed_default_identity(&self) -> Result<()> {
        if self.get_identity_by_name(DEFAULT_IDENTITY_NAME)?.is_some() {
            return Ok(());
        }

        self.create_identity(&NewIdentity {
            name: DEFAULT_IDENTITY_NAME.to_string(),
            username: None,
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: false,
        })?;
        Ok(())
    }

    pub fn create_identity(&self, identity: &NewIdentity) -> Result<Identity> {
        let now = now_ts();
        let private_key = path_to_opt_str(identity.private_key.as_ref());
        let certificate = path_to_opt_str(identity.certificate.as_ref());

        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO identities (name, username, private_key, certificate, sort_order, has_password, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    identity.name,
                    identity.username,
                    private_key,
                    certificate,
                    identity.sort_order,
                    i64::from(identity.has_password),
                    now,
                ],
            )?;
            let id = conn.last_insert_rowid();
            Ok(Identity {
                id,
                name: identity.name.clone(),
                username: identity.username.clone(),
                private_key: identity.private_key.clone(),
                certificate: identity.certificate.clone(),
                has_password: identity.has_password,
            })
        })
    }

    pub fn get_identity(&self, id: i64) -> Result<Option<Identity>> {
        self.with_conn(|conn| {
            conn.prepare(
                "SELECT id, name, username, private_key, certificate, has_password FROM identities WHERE id = ?1",
            )?
            .query_row(params![id], row_to_identity)
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn get_identity_by_name(&self, name: &str) -> Result<Option<Identity>> {
        self.with_conn(|conn| {
            conn.prepare(
                "SELECT id, name, username, private_key, certificate, has_password FROM identities WHERE name = ?1",
            )?
            .query_row(params![name], row_to_identity)
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn list_identities(&self) -> Result<Vec<Identity>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, username, private_key, certificate, has_password
                 FROM identities ORDER BY sort_order, name",
            )?;
            let rows = stmt.query_map([], row_to_identity)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn update_identity(&self, id: i64, update: &IdentityUpdate) -> Result<Option<Identity>> {
        let current = match self.get_identity(id)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let name = update.name.as_ref().unwrap_or(&current.name);
        let username = match &update.username {
            Some(v) => v.clone(),
            None => current.username.clone(),
        };
        let private_key = match &update.private_key {
            Some(v) => v.clone(),
            None => current.private_key.clone(),
        };
        let certificate = match &update.certificate {
            Some(v) => v.clone(),
            None => current.certificate.clone(),
        };
        let sort_order = match update.sort_order {
            Some(v) => v,
            None => self.get_identity_sort_order(id)?.unwrap_or(0),
        };
        let has_password = update.has_password.unwrap_or(current.has_password);

        self.with_conn(|conn| {
            conn.execute(
                "UPDATE identities
                 SET name = ?1, username = ?2, private_key = ?3, certificate = ?4, sort_order = ?5, has_password = ?6
                 WHERE id = ?7",
                params![
                    name,
                    username,
                    path_to_opt_str(private_key.as_ref()),
                    path_to_opt_str(certificate.as_ref()),
                    sort_order,
                    i64::from(has_password),
                    id,
                ],
            )?;
            Ok(())
        })?;

        self.get_identity(id)
    }

    /// Count launcher hosts referencing this identity.
    pub fn count_hosts_using_identity(&self, id: i64) -> Result<usize> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM hosts WHERE identity_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
    }

    pub fn delete_identity(&self, id: i64) -> Result<DeleteIdentityOutcome> {
        if self.get_identity(id)?.is_none() {
            return Ok(DeleteIdentityOutcome::NotFound);
        }

        let host_count = self.count_hosts_using_identity(id)?;
        if host_count > 0 {
            return Ok(DeleteIdentityOutcome::InUse { host_count });
        }

        let deleted = self.with_conn(|conn| {
            conn.execute("DELETE FROM identities WHERE id = ?1", params![id])?;
            Ok(conn.changes() > 0)
        })?;

        if deleted {
            Ok(DeleteIdentityOutcome::Deleted)
        } else {
            Ok(DeleteIdentityOutcome::NotFound)
        }
    }

    fn get_identity_sort_order(&self, id: i64) -> Result<Option<i32>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT sort_order FROM identities WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }
}

fn row_to_identity(row: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    Ok(Identity {
        id: row.get(0)?,
        name: row.get(1)?,
        username: row.get(2)?,
        private_key: str_to_path(row.get(3)?),
        certificate: str_to_path(row.get(4)?),
        has_password: row.get::<_, i64>(5).unwrap_or(0) != 0,
    })
}

fn path_to_opt_str(path: Option<&PathBuf>) -> Option<String> {
    path.map(|p| p.to_string_lossy().into_owned())
}

fn str_to_path(raw: Option<String>) -> Option<PathBuf> {
    raw.filter(|s| !s.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LauncherStore;

    #[test]
    fn default_identity_exists_after_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = LauncherStore::open(dir.path().join("launcher.db")).unwrap();
        let default = store
            .get_identity_by_name(DEFAULT_IDENTITY_NAME)
            .unwrap()
            .expect("Default identity should be seeded");
        assert_eq!(default.name, DEFAULT_IDENTITY_NAME);
    }

    #[test]
    fn identity_crud_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = LauncherStore::open(dir.path().join("launcher.db")).unwrap();

        let created = store
            .create_identity(&NewIdentity {
                name: "dev-vcenter".into(),
                username: Some("admin".into()),
                private_key: Some(PathBuf::from("~/.ssh/id_ed25519")),
                certificate: None,
                sort_order: 1,
                has_password: false,
            })
            .unwrap();

        assert_eq!(created.name, "dev-vcenter");
        assert_eq!(
            store.get_identity(created.id).unwrap(),
            Some(created.clone())
        );

        let all = store.list_identities().unwrap();
        assert!(all.iter().any(|i| i.name == "dev-vcenter"));

        let updated = store
            .update_identity(
                created.id,
                &IdentityUpdate {
                    username: Some(Some("root".into())),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated.username.as_deref(), Some("root"));

        assert_eq!(
            store.delete_identity(created.id).unwrap(),
            DeleteIdentityOutcome::Deleted
        );
        assert!(store.get_identity(created.id).unwrap().is_none());
    }

    #[test]
    fn delete_identity_rejects_when_referenced_by_host() {
        use crate::store::NewHost;

        let dir = tempfile::tempdir().unwrap();
        let store = LauncherStore::open(dir.path().join("launcher.db")).unwrap();

        let identity = store
            .create_identity(&NewIdentity {
                name: "work-key".into(),
                username: Some("deploy".into()),
                private_key: None,
                certificate: None,
                sort_order: 1,
                has_password: false,
            })
            .unwrap();

        store
            .create_host(&NewHost {
                name: "web".into(),
                label: None,
                address: "10.0.0.1".into(),
                port: 22,
                group_id: None,
                identity_id: Some(identity.id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(
            store.delete_identity(identity.id).unwrap(),
            DeleteIdentityOutcome::InUse { host_count: 1 }
        );
        assert!(store.get_identity(identity.id).unwrap().is_some());
    }

    #[test]
    fn list_identities_sorted_by_sort_order_then_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = LauncherStore::open(dir.path().join("launcher.db")).unwrap();

        store
            .create_identity(&NewIdentity {
                name: "zeta".into(),
                username: None,
                private_key: None,
                certificate: None,
                sort_order: 2,
                has_password: false,
            })
            .unwrap();
        store
            .create_identity(&NewIdentity {
                name: "alpha".into(),
                username: None,
                private_key: None,
                certificate: None,
                sort_order: 1,
                has_password: false,
            })
            .unwrap();

        let names: Vec<String> = store
            .list_identities()
            .unwrap()
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(names, vec!["Default", "alpha", "zeta"]);
    }
}
