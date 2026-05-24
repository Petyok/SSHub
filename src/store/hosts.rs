use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use super::migrate::now_ts;
use super::types::{
    AuthEvent, DeleteHostOutcome, HostGroup, HostGroupUpdate, HostSource, HostUpdate, ManagedHost,
    NewHost, NewHostGroup, SshConfigHostImport, UpsertSshConfigOutcome,
};
use super::LauncherStore;

impl LauncherStore {
    // --- host groups ---

    pub fn create_group(&self, group: &NewHostGroup) -> Result<HostGroup> {
        let now = now_ts();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO host_groups (name, sort_order, created_at) VALUES (?1, ?2, ?3)",
                params![group.name, group.sort_order, now],
            )?;
            Ok(HostGroup {
                id: conn.last_insert_rowid(),
                name: group.name.clone(),
                sort_order: group.sort_order,
            })
        })
    }

    pub fn get_group(&self, id: i64) -> Result<Option<HostGroup>> {
        self.with_conn(|conn| {
            conn.prepare("SELECT id, name, sort_order FROM host_groups WHERE id = ?1")?
                .query_row(params![id], row_to_group)
                .optional()
                .map_err(Into::into)
        })
    }

    pub fn list_groups(&self) -> Result<Vec<HostGroup>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, sort_order FROM host_groups ORDER BY sort_order, name",
            )?;
            let rows = stmt.query_map([], row_to_group)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn update_group(&self, id: i64, update: &HostGroupUpdate) -> Result<Option<HostGroup>> {
        let current = match self.get_group(id)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let name = update.name.as_ref().unwrap_or(&current.name);
        let sort_order = update.sort_order.unwrap_or(current.sort_order);

        self.with_conn(|conn| {
            conn.execute(
                "UPDATE host_groups SET name = ?1, sort_order = ?2 WHERE id = ?3",
                params![name, sort_order, id],
            )?;
            Ok(())
        })?;

        self.get_group(id)
    }

    pub fn delete_group(&self, id: i64) -> Result<bool> {
        let deleted = self.with_conn(|conn| {
            conn.execute("DELETE FROM host_groups WHERE id = ?1", params![id])?;
            Ok(conn.changes() > 0)
        })?;
        Ok(deleted)
    }

    // --- hosts ---

    pub fn create_host(&self, host: &NewHost) -> Result<ManagedHost> {
        let now = now_ts();
        let tags_json = tags_to_json(&host.tags)?;
        let source = host.source.as_str();

        let id = self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO hosts
                    (name, label, address, port, group_id, identity_id, os_icon, tags, notes,
                     proxy_jump, forward_agent, remote_command, source, has_password, username, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)",
                params![
                    host.name,
                    host.label,
                    host.address,
                    i64::from(host.port),
                    host.group_id,
                    host.identity_id,
                    host.os_icon,
                    tags_json,
                    host.notes,
                    host.proxy_jump,
                    i64::from(host.forward_agent),
                    host.remote_command,
                    source,
                    i64::from(host.has_password),
                    host.username,
                    now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        self.get_host(id)?.context("inserted host missing")
    }

    pub fn get_host(&self, id: i64) -> Result<Option<ManagedHost>> {
        self.with_conn(|conn| load_host_by_id(conn, id))
    }

    pub fn get_host_by_name(&self, name: &str) -> Result<Option<ManagedHost>> {
        self.with_conn(|conn| {
            let id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM hosts WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .optional()?;
            match id {
                Some(id) => load_host_by_id(conn, id),
                None => Ok(None),
            }
        })
    }

    pub fn list_hosts(&self) -> Result<Vec<ManagedHost>> {
        self.list_hosts_filtered(None)
    }

    pub fn list_hosts_filtered(&self, source: Option<HostSource>) -> Result<Vec<ManagedHost>> {
        self.with_conn(|conn| {
            let ids: Vec<i64> = match source {
                None => {
                    let mut stmt =
                        conn.prepare("SELECT id FROM hosts ORDER BY sort_order, name")?;
                    let rows = stmt.query_map([], |row| row.get(0))?;
                    rows.collect::<Result<Vec<_>, _>>()?
                }
                Some(src) => {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM hosts WHERE source = ?1 ORDER BY sort_order, name",
                    )?;
                    let rows = stmt.query_map(params![src.as_str()], |row| row.get(0))?;
                    rows.collect::<Result<Vec<_>, _>>()?
                }
            };

            ids.into_iter()
                .map(|id| load_host_by_id(conn, id)?.context("host row missing after list query"))
                .collect()
        })
    }

    pub fn update_host(&self, id: i64, update: &HostUpdate) -> Result<Option<ManagedHost>> {
        let current = match self.get_host(id)? {
            Some(v) => v,
            None => return Ok(None),
        };

        if current.source == HostSource::SshConfig && update_changes_connection_fields(update) {
            anyhow::bail!("connection fields are read-only for ssh_config hosts");
        }

        let label = match &update.label {
            Some(v) => v.clone(),
            None => current.label.clone(),
        };
        let name = update.name.as_ref().unwrap_or(&current.name).clone();
        let address = update.address.as_ref().unwrap_or(&current.address).clone();
        let port = update.port.unwrap_or(current.port);
        let group_id = match &update.group_id {
            Some(v) => *v,
            None => current.group_id,
        };
        let identity_id = match &update.identity_id {
            Some(v) => *v,
            None => current.identity_id,
        };
        let tags = update.tags.as_ref().unwrap_or(&current.tags).clone();
        let notes = match &update.notes {
            Some(v) => v.clone(),
            None => current.notes.clone(),
        };
        let os_icon = match &update.os_icon {
            Some(v) => v.clone(),
            None => current.os_icon.clone(),
        };
        let proxy_jump = match &update.proxy_jump {
            Some(v) => v.clone(),
            None => current.proxy_jump.clone(),
        };
        let forward_agent = update.forward_agent.unwrap_or(current.forward_agent);
        let remote_command = match &update.remote_command {
            Some(v) => v.clone(),
            None => current.remote_command.clone(),
        };
        let favorite = update.favorite.unwrap_or(current.favorite);
        let sort_order = update.sort_order.unwrap_or(current.sort_order);
        let has_password = update.has_password.unwrap_or(current.has_password);
        let username = match &update.username {
            Some(v) => v.clone(),
            None => current.username.clone(),
        };
        let tags_json = tags_to_json(&tags)?;
        let now = now_ts();

        self.with_conn(|conn| {
            conn.execute(
                "UPDATE hosts
                 SET name = ?1, label = ?2, address = ?3, port = ?4, group_id = ?5, identity_id = ?6,
                     os_icon = ?7, tags = ?8, notes = ?9, proxy_jump = ?10, forward_agent = ?11,
                     remote_command = ?12, favorite = ?13, sort_order = ?14, has_password = ?15, username = ?16, updated_at = ?17
                 WHERE id = ?18",
                params![
                    name,
                    label,
                    address,
                    i64::from(port),
                    group_id,
                    identity_id,
                    os_icon,
                    tags_json,
                    notes,
                    proxy_jump,
                    i64::from(forward_agent),
                    remote_command,
                    i64::from(favorite),
                    sort_order,
                    i64::from(has_password),
                    username,
                    now,
                    id,
                ],
            )?;
            Ok(())
        })?;

        self.get_host(id)
    }

    pub fn set_host_last_connected(&self, id: i64, ts: i64) -> Result<()> {
        let now = now_ts();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE hosts SET last_connected = ?1, updated_at = ?2 WHERE id = ?3",
                params![ts, now, id],
            )?;
            Ok(())
        })
    }

    pub fn delete_host(&self, id: i64) -> Result<DeleteHostOutcome> {
        let Some(current) = self.get_host(id)? else {
            return Ok(DeleteHostOutcome::NotFound);
        };
        if current.source != HostSource::Launcher {
            return Ok(DeleteHostOutcome::NotLauncher);
        }

        let deleted = self.with_conn(|conn| {
            conn.execute("DELETE FROM hosts WHERE id = ?1", params![id])?;
            Ok(conn.changes() > 0)
        })?;
        if deleted {
            Ok(DeleteHostOutcome::Deleted)
        } else {
            Ok(DeleteHostOutcome::NotFound)
        }
    }

    /// Swap `sort_order` values between two launcher hosts.
    pub fn swap_host_sort_orders(&self, id_a: i64, id_b: i64) -> Result<()> {
        let a = self
            .get_host(id_a)?
            .context("host a missing for sort swap")?;
        let b = self
            .get_host(id_b)?
            .context("host b missing for sort swap")?;

        self.update_host(
            id_a,
            &HostUpdate {
                sort_order: Some(b.sort_order),
                ..Default::default()
            },
        )?;
        self.update_host(
            id_b,
            &HostUpdate {
                sort_order: Some(a.sort_order),
                ..Default::default()
            },
        )?;
        Ok(())
    }

    /// Insert or update a host imported from ssh config. Never overwrites launcher rows.
    pub fn upsert_ssh_config_host(
        &self,
        import: &SshConfigHostImport,
    ) -> Result<UpsertSshConfigOutcome> {
        if let Some(existing) = self.get_host_by_name(&import.name)? {
            if existing.source == HostSource::Launcher {
                return Ok(UpsertSshConfigOutcome::SkippedLauncher);
            }

            let tags_json = tags_to_json(&existing.tags)?;
            let now = now_ts();
            self.with_conn(|conn| {
                conn.execute(
                    "UPDATE hosts
                     SET address = ?1, port = ?2, proxy_jump = ?3, forward_agent = ?4,
                         remote_command = ?5, ssh_config_hash = ?6, tags = ?7, notes = ?8,
                         favorite = ?9, last_connected = ?10, updated_at = ?11
                     WHERE id = ?12",
                    params![
                        import.address,
                        i64::from(import.port),
                        import.proxy_jump,
                        i64::from(import.forward_agent),
                        import.remote_command,
                        import.ssh_config_hash,
                        tags_json,
                        existing.notes,
                        i64::from(existing.favorite),
                        existing.last_connected,
                        now,
                        existing.id,
                    ],
                )?;
                Ok(())
            })?;
            return Ok(UpsertSshConfigOutcome::Updated);
        }

        let now = now_ts();
        let tags_json = tags_to_json(&import.tags)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO hosts
                    (name, label, address, port, tags, notes, proxy_jump, forward_agent,
                     remote_command, favorite, last_connected, source, ssh_config_hash,
                     created_at, updated_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'ssh_config', ?11, ?12, ?12)",
                params![
                    import.name,
                    import.address,
                    i64::from(import.port),
                    tags_json,
                    import.notes,
                    import.proxy_jump,
                    i64::from(import.forward_agent),
                    import.remote_command,
                    i64::from(import.favorite),
                    import.last_connected,
                    import.ssh_config_hash,
                    now,
                ],
            )?;
            Ok(())
        })?;
        Ok(UpsertSshConfigOutcome::Inserted)
    }

    pub fn duplicate_host(&self, id: i64) -> Result<Option<ManagedHost>> {
        let current = match self.get_host(id)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let mut name = format!("{}-copy", current.name);
        let mut suffix = 2u32;
        while self.get_host_by_name(&name)?.is_some() {
            name = format!("{}-copy-{}", current.name, suffix);
            suffix += 1;
        }

        self.create_host(&NewHost {
            name,
            label: current.label.clone(),
            address: current.address.clone(),
            port: current.port,
            group_id: current.group_id,
            identity_id: current.identity_id,
            os_icon: current.os_icon.clone(),
            tags: current.tags.clone(),
            notes: current.notes.clone(),
            proxy_jump: current.proxy_jump.clone(),
            forward_agent: current.forward_agent,
            remote_command: current.remote_command.clone(),
            source: HostSource::Launcher,
            has_password: false,
            username: current.username.clone(),
        })
        .map(Some)
    }

    // --- auth events ---

    pub fn log_auth_event(
        &self,
        host_name: &str,
        username: Option<&str>,
        via: &str,
        status: &str,
        note: &str,
    ) -> Result<()> {
        let now = now_ts();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO auth_events (host_name, username, via, status, note, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![host_name, username, via, status, note, now],
            )?;
            Ok(())
        })
    }

    pub fn list_auth_events(&self, limit: usize) -> Result<Vec<AuthEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, host_name, username, via, status, note, created_at
                 FROM auth_events ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |row| {
                Ok(AuthEvent {
                    id: row.get(0)?,
                    host_name: row.get(1)?,
                    username: row.get(2)?,
                    via: row.get(3)?,
                    status: row.get(4)?,
                    note: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    pub fn list_auth_events_filtered(
        &self,
        status_filter: Option<&str>,
        since: Option<i64>,
        limit: usize,
    ) -> Result<Vec<AuthEvent>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT id, host_name, username, via, status, note, created_at FROM auth_events",
            );
            let mut conditions = Vec::new();
            if status_filter.is_some() {
                conditions.push("status = ?1");
            }
            if since.is_some() {
                conditions.push(if status_filter.is_some() {
                    "created_at >= ?2"
                } else {
                    "created_at >= ?1"
                });
            }
            if !conditions.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&conditions.join(" AND "));
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            // param index for limit
            let limit_idx = 1 + status_filter.is_some() as u8 + since.is_some() as u8;
            sql.push_str(&limit_idx.to_string());

            let mut stmt = conn.prepare(&sql)?;

            let mut param_idx = 1;
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if let Some(s) = status_filter {
                params_vec.push(Box::new(s.to_string()));
                param_idx += 1;
            }
            if let Some(ts) = since {
                params_vec.push(Box::new(ts));
                param_idx += 1;
            }
            let _ = param_idx;
            params_vec.push(Box::new(limit as i64));

            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();

            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(AuthEvent {
                    id: row.get(0)?,
                    host_name: row.get(1)?,
                    username: row.get(2)?,
                    via: row.get(3)?,
                    status: row.get(4)?,
                    note: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    /// Count events by status for the last N days.
    pub fn auth_event_stats(&self, days: i64) -> Result<(i64, i64)> {
        let cutoff = now_ts() - days * 86400;
        self.with_conn(|conn| {
            let ok: i64 = conn.query_row(
                "SELECT COUNT(*) FROM auth_events WHERE status = 'ok' AND created_at >= ?1",
                params![cutoff],
                |row| row.get(0),
            )?;
            let fail: i64 = conn.query_row(
                "SELECT COUNT(*) FROM auth_events WHERE status != 'ok' AND created_at >= ?1",
                params![cutoff],
                |row| row.get(0),
            )?;
            Ok((ok, fail))
        })
    }
}

fn load_host_by_id(conn: &rusqlite::Connection, id: i64) -> Result<Option<ManagedHost>> {
    conn.prepare(
        "SELECT h.id, h.name, h.label, h.address, h.port, h.group_id, h.identity_id,
                h.os_icon, h.tags, h.notes, h.proxy_jump, h.forward_agent, h.remote_command,
                h.sort_order, h.favorite, h.last_connected, h.source, h.ssh_config_hash,
                h.has_password, h.created_at, h.updated_at, h.username,
                g.id, g.name, g.sort_order,
                i.id, i.name, i.username, i.private_key, i.certificate, i.has_password
         FROM hosts h
         LEFT JOIN host_groups g ON g.id = h.group_id
         LEFT JOIN identities i ON i.id = h.identity_id
         WHERE h.id = ?1",
    )?
    .query_row(params![id], row_to_managed_host)
    .optional()
    .map_err(Into::into)
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostGroup> {
    Ok(HostGroup {
        id: row.get(0)?,
        name: row.get(1)?,
        sort_order: row.get(2)?,
    })
}

fn row_to_managed_host(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManagedHost> {
    let tags_raw: String = row.get(8)?;
    let tags = tags_from_json(Some(tags_raw)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;

    let source_raw: String = row.get(16)?;
    let source = HostSource::from_str(&source_raw).unwrap_or(HostSource::Launcher);

    let group = match row.get::<_, Option<i64>>(22)? {
        Some(_) => Some(HostGroup {
            id: row.get(22)?,
            name: row.get(23)?,
            sort_order: row.get(24)?,
        }),
        None => None,
    };

    let identity = match row.get::<_, Option<i64>>(25)? {
        Some(_) => Some(super::types::Identity {
            id: row.get(25)?,
            name: row.get(26)?,
            username: row.get(27)?,
            private_key: str_to_path(row.get(28)?),
            certificate: str_to_path(row.get(29)?),
            has_password: row.get::<_, i64>(30).unwrap_or(0) != 0,
        }),
        None => None,
    };

    Ok(ManagedHost {
        id: row.get(0)?,
        name: row.get(1)?,
        label: row.get(2)?,
        address: row.get(3)?,
        port: row.get::<_, i64>(4)? as u16,
        group_id: row.get(5)?,
        identity_id: row.get(6)?,
        group,
        identity,
        os_icon: row.get(7)?,
        tags,
        notes: row.get(9)?,
        proxy_jump: row.get(10)?,
        forward_agent: row.get::<_, i64>(11)? != 0,
        remote_command: row.get(12)?,
        sort_order: row.get(13)?,
        favorite: row.get::<_, i64>(14)? != 0,
        last_connected: row.get(15)?,
        source,
        ssh_config_hash: row.get(17)?,
        has_password: row.get::<_, i64>(18).unwrap_or(0) != 0,
        username: row.get(21)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn tags_to_json(tags: &[String]) -> Result<String> {
    Ok(serde_json::to_string(tags)?)
}

fn tags_from_json(raw: Option<String>) -> Result<Vec<String>> {
    match raw {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => Ok(serde_json::from_str(&s)?),
    }
}

fn str_to_path(raw: Option<String>) -> Option<std::path::PathBuf> {
    raw.filter(|s| !s.is_empty()).map(std::path::PathBuf::from)
}

fn update_changes_connection_fields(update: &HostUpdate) -> bool {
    update.name.is_some()
        || update.address.is_some()
        || update.port.is_some()
        || update.proxy_jump.is_some()
        || update.forward_agent.is_some()
        || update.remote_command.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{HostSource, LauncherStore, NewHost, NewHostGroup};

    #[test]
    fn group_crud_roundtrip() {
        let store = LauncherStore::open_in_memory().unwrap();

        let a = store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                sort_order: 10,
                ..Default::default()
            })
            .unwrap();
        let b = store
            .create_group(&NewHostGroup {
                name: "dev".into(),
                sort_order: 0,
            })
            .unwrap();

        let listed = store.list_groups().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, b.id);
        assert_eq!(listed[0].name, "dev");
        assert_eq!(listed[1].id, a.id);

        let updated = store
            .update_group(
                a.id,
                &HostGroupUpdate {
                    name: Some("production".into()),
                    sort_order: Some(5),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "production");
        assert_eq!(updated.sort_order, 5);

        assert!(store.delete_group(b.id).unwrap());
        assert!(store.get_group(b.id).unwrap().is_none());
        assert_eq!(store.list_groups().unwrap().len(), 1);
    }

    #[test]
    fn delete_group_reassigns_hosts_to_null() {
        let store = LauncherStore::open_in_memory().unwrap();
        let default_id = store
            .get_identity_by_name("Default")
            .unwrap()
            .expect("Default identity")
            .id;

        let group = store
            .create_group(&NewHostGroup {
                name: "staging".into(),
                sort_order: 0,
            })
            .unwrap();

        let host = store
            .create_host(&NewHost {
                name: "app-1".into(),
                label: None,
                address: "10.0.0.1".into(),
                port: 22,
                group_id: Some(group.id),
                identity_id: Some(default_id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(host.group_id, Some(group.id));

        assert!(store.delete_group(group.id).unwrap());

        let fetched = store.get_host(host.id).unwrap().unwrap();
        assert_eq!(fetched.group_id, None);
        assert!(fetched.group.is_none());
    }

    #[test]
    fn insert_and_list_host_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = LauncherStore::open(dir.path().join("launcher.db")).unwrap();

        let default_id = store
            .get_identity_by_name("Default")
            .unwrap()
            .expect("Default identity")
            .id;

        let group = store
            .create_group(&NewHostGroup {
                name: "dev-vcenter".into(),
                sort_order: 0,
            })
            .unwrap();

        let created = store
            .create_host(&NewHost {
                name: "dev-partners".into(),
                label: Some("Dev Partners".into()),
                address: "10.100.19.123".into(),
                port: 22,
                group_id: Some(group.id),
                identity_id: Some(default_id),
                tags: vec!["dev".into()],
                notes: Some("staging".into()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(created.name, "dev-partners");
        assert_eq!(created.address, "10.100.19.123");
        assert_eq!(created.source, HostSource::Launcher);
        assert_eq!(
            created.group.as_ref().map(|g| g.name.as_str()),
            Some("dev-vcenter")
        );
        assert_eq!(
            created.identity.as_ref().map(|i| i.name.as_str()),
            Some("Default")
        );

        let listed = store.list_hosts().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let fetched = store.get_host(created.id).unwrap().unwrap();
        assert_eq!(fetched.label, Some("Dev Partners".into()));
        assert_eq!(fetched.tags, vec!["dev"]);

        let updated = store
            .update_host(
                created.id,
                &HostUpdate {
                    favorite: Some(true),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert!(updated.favorite);

        assert!(store.delete_host(created.id).unwrap() == DeleteHostOutcome::Deleted);
        assert!(store.list_hosts().unwrap().is_empty());
    }

    #[test]
    fn swap_host_sort_orders_exchanges_values() {
        let store = LauncherStore::open_in_memory().unwrap();
        let default_id = store.get_identity_by_name("Default").unwrap().unwrap().id;

        let a = store
            .create_host(&NewHost {
                name: "a".into(),
                label: None,
                address: "10.0.0.1".into(),
                port: 22,
                group_id: None,
                identity_id: Some(default_id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();
        let b = store
            .create_host(&NewHost {
                name: "b".into(),
                label: None,
                address: "10.0.0.2".into(),
                port: 22,
                group_id: None,
                identity_id: Some(default_id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();

        store
            .update_host(
                a.id,
                &HostUpdate {
                    sort_order: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .update_host(
                b.id,
                &HostUpdate {
                    sort_order: Some(99),
                    ..Default::default()
                },
            )
            .unwrap();

        store.swap_host_sort_orders(a.id, b.id).unwrap();

        assert_eq!(store.get_host(a.id).unwrap().unwrap().sort_order, 99);
        assert_eq!(store.get_host(b.id).unwrap().unwrap().sort_order, 1);
    }

    #[test]
    fn ssh_config_reimport_preserves_user_metadata() {
        let store = LauncherStore::open_in_memory().unwrap();
        store
            .upsert_ssh_config_host(&SshConfigHostImport {
                name: "web".into(),
                address: "1.2.3.4".into(),
                port: 22,
                tags: vec!["from-import".into()],
                ssh_config_hash: "hash-v1".into(),
                ..Default::default()
            })
            .unwrap();

        let id = store.get_host_by_name("web").unwrap().unwrap().id;
        store
            .update_host(
                id,
                &HostUpdate {
                    tags: Some(vec!["user-tagged".into()]),
                    notes: Some(Some("keep me".into())),
                    favorite: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();

        store
            .upsert_ssh_config_host(&SshConfigHostImport {
                name: "web".into(),
                address: "5.6.7.8".into(),
                port: 22,
                tags: vec!["from-import".into()],
                ssh_config_hash: "hash-v2".into(),
                ..Default::default()
            })
            .unwrap();

        let host = store.get_host_by_name("web").unwrap().unwrap();
        assert_eq!(host.address, "5.6.7.8");
        assert_eq!(host.tags, vec!["user-tagged"]);
        assert_eq!(host.notes.as_deref(), Some("keep me"));
        assert!(host.favorite);
    }

    #[test]
    fn ssh_config_update_rejects_connection_fields() {
        let store = LauncherStore::open_in_memory().unwrap();
        store
            .upsert_ssh_config_host(&SshConfigHostImport {
                name: "web".into(),
                address: "1.2.3.4".into(),
                port: 22,
                ssh_config_hash: "hash".into(),
                ..Default::default()
            })
            .unwrap();
        let id = store.get_host_by_name("web").unwrap().unwrap().id;

        let err = store
            .update_host(
                id,
                &HostUpdate {
                    address: Some("evil".into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("read-only"));
    }
}
