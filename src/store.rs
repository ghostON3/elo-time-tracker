//! SQLite persistence.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

/// One aggregated focus event: a continuous run of polls where the active
/// window did not change.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Event {
    pub id: Option<i64>,
    pub app_class: String,
    pub window_title: String,
    pub monitor: String,
    /// ISO-8601 (RFC 3339) UTC.
    pub started_at: String,
    pub duration_secs: i64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn =
            Connection::open(path).with_context(|| format!("open sqlite at {}", path.display()))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                app_class     TEXT NOT NULL,
                window_title  TEXT NOT NULL,
                monitor       TEXT NOT NULL,
                started_at    TEXT NOT NULL,
                duration_secs INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_started_at
                ON events(started_at);
            CREATE INDEX IF NOT EXISTS idx_events_app_class
                ON events(app_class);
            "#,
        )
        .context("create schema")?;
        Ok(Self { conn })
    }

    pub fn insert(&self, event: &Event) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO events (app_class, window_title, monitor, started_at, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.app_class,
                event.window_title,
                event.monitor,
                event.started_at,
                event.duration_secs,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn all_events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, app_class, window_title, monitor, started_at, duration_secs
             FROM events ORDER BY started_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_event)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Events whose `started_at` is >= `from` (UTC).
    pub fn events_since(&self, from: DateTime<Utc>) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, app_class, window_title, monitor, started_at, duration_secs
             FROM events WHERE started_at >= ?1 ORDER BY started_at ASC",
        )?;
        let from_str = from.to_rfc3339();
        let rows = stmt.query_map(params![from_str], row_to_event)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: Some(row.get(0)?),
        app_class: row.get(1)?,
        window_title: row.get(2)?,
        monitor: row.get(3)?,
        started_at: row.get(4)?,
        duration_secs: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_event(class: &str, dur: i64) -> Event {
        Event {
            id: None,
            app_class: class.to_string(),
            window_title: "t".to_string(),
            monitor: "0".to_string(),
            started_at: "2026-05-16T10:00:00Z".to_string(),
            duration_secs: dur,
        }
    }

    #[test]
    fn insert_and_read_back() {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let id = store.insert(&mk_event("kitty", 30)).unwrap();
        assert!(id > 0);
        let all = store.all_events().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].app_class, "kitty");
        assert_eq!(all[0].duration_secs, 30);
    }

    #[test]
    fn events_since_filters() {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let mut e1 = mk_event("a", 10);
        e1.started_at = "2026-05-15T10:00:00Z".into();
        let mut e2 = mk_event("b", 20);
        e2.started_at = "2026-05-16T10:00:00Z".into();
        store.insert(&e1).unwrap();
        store.insert(&e2).unwrap();
        let from = DateTime::parse_from_rfc3339("2026-05-16T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let got = store.events_since(from).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].app_class, "b");
    }
}
