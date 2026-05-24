use anyhow::Result;
use rusqlite::params;

use super::migrate::now_ts;
use super::types::{NewTunnel, Tunnel, TunnelType};
use super::LauncherStore;

impl LauncherStore {
    pub fn create_tunnel(&self, t: &NewTunnel) -> Result<i64> {
        let now = now_ts();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tunnels (host_id, tunnel_type, local_port, remote_host, remote_port, label, auto_connect, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    t.host_id,
                    t.tunnel_type.as_str(),
                    t.local_port as i64,
                    t.remote_host,
                    t.remote_port as i64,
                    t.label,
                    t.auto_connect as i64,
                    now,
                    now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn list_tunnels(&self) -> Result<Vec<Tunnel>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, host_id, tunnel_type, local_port, remote_host, remote_port, label, auto_connect, created_at, updated_at
                 FROM tunnels ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(Tunnel {
                    id: row.get(0)?,
                    host_id: row.get(1)?,
                    tunnel_type: TunnelType::from_str(row.get::<_, String>(2)?.as_str()),
                    local_port: row.get::<_, i64>(3)? as u16,
                    remote_host: row.get(4)?,
                    remote_port: row.get::<_, i64>(5)? as u16,
                    label: row.get(6)?,
                    auto_connect: row.get::<_, i64>(7)? != 0,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    pub fn delete_tunnel(&self, id: i64) -> Result<bool> {
        self.with_conn(|conn| {
            let affected = conn.execute("DELETE FROM tunnels WHERE id = ?1", params![id])?;
            Ok(affected > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_crud() {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = store
            .create_tunnel(&NewTunnel {
                host_id: None,
                tunnel_type: TunnelType::Local,
                local_port: 5432,
                remote_host: "localhost".into(),
                remote_port: 5432,
                label: Some("postgres".into()),
                auto_connect: false,
            })
            .unwrap();

        let tunnels = store.list_tunnels().unwrap();
        assert_eq!(tunnels.len(), 1);
        assert_eq!(tunnels[0].id, id);
        assert_eq!(tunnels[0].local_port, 5432);
        assert_eq!(tunnels[0].label.as_deref(), Some("postgres"));

        store.delete_tunnel(id).unwrap();
        assert!(store.list_tunnels().unwrap().is_empty());
    }
}
