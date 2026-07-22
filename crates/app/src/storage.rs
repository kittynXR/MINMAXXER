use anyhow::{Context, Result};
use minmaxxer_core::{events_for_run as select_run_events, GameEvent};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct IngestState {
    pub file_size: u64,
    pub modified_millis: i64,
    pub complete: bool,
    pub parser_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageStats {
    pub event_count: u64,
    pub source_count: u64,
    pub unknown_line_count: u64,
    pub database_bytes: u64,
}

pub struct Storage {
    connection: Mutex<Connection>,
    database_path: PathBuf,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed creating database directory {}", parent.display())
            })?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("failed opening database {}", path.display()))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(3))?;
        migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            database_path: path.to_owned(),
        })
    }

    pub fn ingest_state(&self, path: &Path) -> Result<Option<IngestState>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection
            .query_row(
                "SELECT file_size, modified_millis, complete, parser_version
                 FROM ingest_file WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                |row| {
                    Ok(IngestState {
                        file_size: row.get::<_, i64>(0)?.max(0) as u64,
                        modified_millis: row.get(1)?,
                        complete: row.get::<_, i64>(2)? != 0,
                        parser_version: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn insert_events(
        &self,
        path: &Path,
        events: &[(u64, GameEvent)],
        byte_offset: u64,
        file_size: u64,
        modified_millis: i64,
        complete: bool,
    ) -> Result<usize> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let inserted = insert_event_batch(&transaction, path, events)?;
        transaction.execute(
            "INSERT INTO ingest_file
                 (path, byte_offset, file_size, modified_millis, complete, parser_version, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET
                 byte_offset = excluded.byte_offset,
                 file_size = excluded.file_size,
                 modified_millis = excluded.modified_millis,
                 complete = excluded.complete,
                 parser_version = excluded.parser_version,
                 updated_at = excluded.updated_at",
            params![
                path.to_string_lossy().as_ref(),
                byte_offset as i64,
                file_size as i64,
                modified_millis,
                i64::from(complete),
                env!("CARGO_PKG_VERSION")
            ],
        )?;
        transaction.commit()?;
        Ok(inserted)
    }

    pub fn insert_unknown_line(&self, path: &Path, byte_offset: u64, raw: &str) -> Result<()> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT OR IGNORE INTO unknown_line (source_path, source_offset, raw)
             VALUES (?1, ?2, ?3)",
            params![path.to_string_lossy().as_ref(), byte_offset as i64, raw],
        )?;
        Ok(())
    }

    /// Removes all parser output and checkpoint state for a source whose contents were replaced.
    /// This is intentionally path-scoped so a truncated VRChat log can be replayed without
    /// retaining events that used to occupy the same byte offsets.
    pub fn reset_source(&self, path: &Path) -> Result<()> {
        let source = path.to_string_lossy();
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM event WHERE source_path = ?1",
            [source.as_ref()],
        )?;
        transaction.execute(
            "DELETE FROM unknown_line WHERE source_path = ?1",
            [source.as_ref()],
        )?;
        transaction.execute("DELETE FROM ingest_file WHERE path = ?1", [source.as_ref()])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn all_events(&self) -> Result<Vec<GameEvent>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT event_json FROM event ORDER BY timestamp ASC, source_path ASC, source_offset ASC",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut events = Vec::new();
        for row in rows {
            let json = row?;
            match serde_json::from_str(&json) {
                Ok(event) => events.push(event),
                Err(error) => tracing::warn!(%error, "skipping unreadable stored event"),
            }
        }
        Ok(events)
    }

    pub fn events_for_run(&self, run_id: &str) -> Result<Vec<GameEvent>> {
        let events = self.all_events()?;
        Ok(select_run_events(&events, run_id))
    }

    pub fn stats(&self) -> Result<StorageStats> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let event_count = connection
            .query_row("SELECT COUNT(*) FROM event", [], |row| row.get::<_, i64>(0))?
            as u64;
        let source_count = connection.query_row("SELECT COUNT(*) FROM ingest_file", [], |row| {
            row.get::<_, i64>(0)
        })? as u64;
        let unknown_line_count =
            connection.query_row("SELECT COUNT(*) FROM unknown_line", [], |row| {
                row.get::<_, i64>(0)
            })? as u64;
        let database_bytes = std::fs::metadata(&self.database_path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        Ok(StorageStats {
            event_count,
            source_count,
            unknown_line_count,
            database_bytes,
        })
    }
}

fn insert_event_batch(
    transaction: &Transaction<'_>,
    path: &Path,
    events: &[(u64, GameEvent)],
) -> Result<usize> {
    let mut statement = transaction.prepare_cached(
        "INSERT OR IGNORE INTO event
             (source_path, source_offset, timestamp, sequence, kind, player, session_id,
              stage, class_name, boss, amount, damage_type, source, target, entity, event_json)
         VALUES
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut inserted = 0;
    for (source_offset, event) in events {
        inserted += statement.execute(params![
            path.to_string_lossy().as_ref(),
            *source_offset as i64,
            event.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            event.sequence as i64,
            event.kind.as_str(),
            event.player.as_deref(),
            event.session_id.map(i64::from),
            event.stage.as_deref(),
            event.class_name.as_deref(),
            event.boss.as_deref(),
            event.amount,
            event.damage_type.as_deref(),
            event.source.as_deref(),
            event.target.as_deref(),
            event.entity.as_deref(),
            serde_json::to_string(event)?,
        ])?;
    }
    Ok(inserted)
}

fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
             version INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS ingest_file (
             path TEXT PRIMARY KEY,
             byte_offset INTEGER NOT NULL DEFAULT 0,
             file_size INTEGER NOT NULL DEFAULT 0,
             modified_millis INTEGER NOT NULL DEFAULT 0,
             complete INTEGER NOT NULL DEFAULT 0,
             parser_version TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS event (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             source_path TEXT NOT NULL,
             source_offset INTEGER NOT NULL,
             timestamp TEXT NOT NULL,
             sequence INTEGER NOT NULL,
             kind TEXT NOT NULL,
             player TEXT,
             session_id INTEGER,
             stage TEXT,
             class_name TEXT,
             boss TEXT,
             amount REAL,
             damage_type TEXT,
             source TEXT,
             target TEXT,
             entity TEXT,
             event_json TEXT NOT NULL,
             UNIQUE(source_path, source_offset)
         );
         CREATE INDEX IF NOT EXISTS event_session_time ON event(session_id, timestamp);
         CREATE INDEX IF NOT EXISTS event_kind_time ON event(kind, timestamp);
         CREATE INDEX IF NOT EXISTS event_player_time ON event(player, timestamp);
         CREATE TABLE IF NOT EXISTS unknown_line (
             source_path TEXT NOT NULL,
             source_offset INTEGER NOT NULL,
             raw TEXT NOT NULL,
             UNIQUE(source_path, source_offset)
         );",
    )?;
    let version: Option<i64> = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    match version {
        None => {
            connection.execute(
                "INSERT INTO schema_version(version) VALUES (?1)",
                [SCHEMA_VERSION],
            )?;
        }
        Some(version) if version > SCHEMA_VERSION => {
            anyhow::bail!(
                "database schema {version} is newer than supported schema {SCHEMA_VERSION}"
            );
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use minmaxxer_core::EventKind;

    #[test]
    fn event_insert_is_idempotent_by_file_offset() {
        let path = std::env::temp_dir().join(format!(
            "minmaxxer-storage-test-{}-{}.sqlite",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("thread")
                .replace(':', "_")
        ));
        let _ = std::fs::remove_file(&path);
        let storage = Storage::open(&path).unwrap();
        let event = GameEvent {
            sequence: 1,
            timestamp: NaiveDate::from_ymd_opt(2026, 7, 21)
                .unwrap()
                .and_hms_opt(20, 0, 0)
                .unwrap(),
            kind: EventKind::DamageDealt,
            player: Some("Test".to_owned()),
            session_id: Some(1),
            world: Some("Ecliptica".to_owned()),
            instance: Some("wrld_test:1".to_owned()),
            stage: None,
            class_name: None,
            boss: None,
            phase: None,
            amount: Some(10.0),
            damage_type: Some("strike".to_owned()),
            source: None,
            target: None,
            entity: None,
            pool_id: None,
            enemy_id: None,
            user_id: None,
            message: None,
            raw: String::new(),
        };
        let source = Path::new("test.log");
        assert_eq!(
            storage
                .insert_events(source, &[(100, event.clone())], 200, 200, 0, true)
                .unwrap(),
            1
        );
        assert_eq!(
            storage
                .insert_events(source, &[(100, event)], 200, 200, 0, true)
                .unwrap(),
            0
        );
        assert_eq!(storage.all_events().unwrap().len(), 1);
        storage
            .insert_unknown_line(source, 120, "unknown test line")
            .unwrap();
        storage.reset_source(source).unwrap();
        assert!(storage.all_events().unwrap().is_empty());
        assert!(storage.ingest_state(source).unwrap().is_none());
        assert_eq!(storage.stats().unwrap().unknown_line_count, 0);
        drop(storage);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
