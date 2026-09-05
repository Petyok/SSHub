use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::migrate::now_ts;
use super::types::{NewSnippet, Snippet, SnippetUpdate};
use super::LauncherStore;

impl LauncherStore {
    pub fn create_snippet(&self, snippet: &NewSnippet) -> Result<i64> {
        let now = now_ts();
        let tags = tags_to_json(&snippet.tags)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO snippets (name, command, description, tags, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    snippet.name,
                    snippet.command,
                    snippet.description,
                    tags,
                    now,
                    now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn list_snippets(&self) -> Result<Vec<Snippet>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, command, description, tags, created_at, updated_at
                 FROM snippets ORDER BY name COLLATE NOCASE, created_at",
            )?;
            let rows = stmt.query_map([], row_to_snippet)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    pub fn get_snippet(&self, id: i64) -> Result<Option<Snippet>> {
        self.with_conn(|conn| {
            conn.prepare(
                "SELECT id, name, command, description, tags, created_at, updated_at
                 FROM snippets WHERE id = ?1",
            )?
            .query_row(params![id], row_to_snippet)
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn update_snippet(&self, id: i64, update: &SnippetUpdate) -> Result<Option<Snippet>> {
        let current = match self.get_snippet(id)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let name = update.name.as_ref().unwrap_or(&current.name);
        let command = update.command.as_ref().unwrap_or(&current.command);
        let description = match &update.description {
            Some(v) => v.clone(),
            None => current.description.clone(),
        };
        let tags = match &update.tags {
            Some(v) => v.clone(),
            None => current.tags.clone(),
        };
        let tags_json = tags_to_json(&tags)?;
        let now = now_ts();

        self.with_conn(|conn| {
            conn.execute(
                "UPDATE snippets
                 SET name = ?1, command = ?2, description = ?3, tags = ?4, updated_at = ?5
                 WHERE id = ?6",
                params![name, command, description, tags_json, now, id],
            )?;
            Ok(())
        })?;

        self.get_snippet(id)
    }

    pub fn delete_snippet(&self, id: i64) -> Result<bool> {
        self.with_conn(|conn| {
            let affected = conn.execute("DELETE FROM snippets WHERE id = ?1", params![id])?;
            Ok(affected > 0)
        })
    }
}

fn row_to_snippet(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snippet> {
    let tags = tags_from_json(row.get(4)?).unwrap_or_default();
    Ok(Snippet {
        id: row.get(0)?,
        name: row.get(1)?,
        command: row.get(2)?,
        description: row.get(3)?,
        tags,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn new(name: &str, command: &str) -> NewSnippet {
        NewSnippet {
            name: name.into(),
            command: command.into(),
            description: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn snippet_crud_roundtrip() {
        let store = LauncherStore::open_in_memory().unwrap();

        let id = store
            .create_snippet(&NewSnippet {
                name: "restart nginx".into(),
                command: "sudo systemctl restart nginx".into(),
                description: Some("bounce the web server".into()),
                tags: vec!["ops".into(), "web".into()],
            })
            .unwrap();

        let fetched = store.get_snippet(id).unwrap().unwrap();
        assert_eq!(fetched.name, "restart nginx");
        assert_eq!(fetched.command, "sudo systemctl restart nginx");
        assert_eq!(
            fetched.description.as_deref(),
            Some("bounce the web server")
        );
        assert_eq!(fetched.tags, vec!["ops".to_string(), "web".to_string()]);
        assert!(fetched.created_at > 0);
        assert_eq!(fetched.created_at, fetched.updated_at);

        let updated = store
            .update_snippet(
                id,
                &SnippetUpdate {
                    command: Some("sudo systemctl reload nginx".into()),
                    description: Some(None),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated.command, "sudo systemctl reload nginx");
        assert_eq!(updated.description, None);
        assert_eq!(updated.name, "restart nginx");
        assert_eq!(updated.tags, vec!["ops".to_string(), "web".to_string()]);
        assert!(updated.updated_at >= updated.created_at);

        assert!(store.delete_snippet(id).unwrap());
        assert!(store.get_snippet(id).unwrap().is_none());
        assert!(!store.delete_snippet(id).unwrap());
    }

    #[test]
    fn list_is_sorted_case_insensitively_by_name() {
        let store = LauncherStore::open_in_memory().unwrap();
        store.create_snippet(&new("beta", "b")).unwrap();
        store.create_snippet(&new("Alpha", "a")).unwrap();
        store.create_snippet(&new("gamma", "g")).unwrap();

        let names: Vec<String> = store
            .list_snippets()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["Alpha", "beta", "gamma"]);
    }

    #[test]
    fn update_missing_snippet_returns_none() {
        let store = LauncherStore::open_in_memory().unwrap();
        let result = store
            .update_snippet(999, &SnippetUpdate::default())
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn tags_default_to_empty_when_absent() {
        let store = LauncherStore::open_in_memory().unwrap();
        let id = store.create_snippet(&new("noop", ":")).unwrap();
        let fetched = store.get_snippet(id).unwrap().unwrap();
        assert!(fetched.tags.is_empty());
    }
}
