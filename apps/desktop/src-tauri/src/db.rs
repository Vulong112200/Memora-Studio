//! SQLite database at `<data root>\database\memora.db`.
//! Schema migrations are an append-only list applied in order; `PRAGMA user_version` = number applied.

use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

/// Append only. Never edit a migration that has shipped; add a new one instead.
const MIGRATIONS: &[&str] = &[
    // 1: projects
    "CREATE TABLE projects (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );",
    // 2: media library (originals referenced by path; Windows paths compare case-insensitively)
    "CREATE TABLE media (
        id                INTEGER PRIMARY KEY,
        path              TEXT NOT NULL COLLATE NOCASE UNIQUE,
        size              INTEGER NOT NULL,
        modified_unix     INTEGER,
        duration_ms       INTEGER NOT NULL,
        width             INTEGER NOT NULL,
        height            INTEGER NOT NULL,
        rotation          INTEGER NOT NULL DEFAULT 0,
        fps               REAL,
        video_codec       TEXT NOT NULL,
        audio_codec       TEXT,
        audio_streams     INTEGER NOT NULL DEFAULT 0,
        bitrate           INTEGER,
        source_created_at TEXT,
        probe_json        TEXT NOT NULL,
        imported_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );",
    // 3: project timeline — ordered trimmed segments of library media
    "CREATE TABLE project_clips (
        id         INTEGER PRIMARY KEY,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        media_id   INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        position   INTEGER NOT NULL,
        in_ms      INTEGER NOT NULL CHECK (in_ms >= 0),
        out_ms     INTEGER NOT NULL CHECK (out_ms > in_ms)
    );
    CREATE INDEX project_clips_project ON project_clips (project_id, position);",
    // 4: classical analysis observations (worker analysis.py); scores are computed from these, not stored
    "CREATE TABLE media_analysis (
        media_id    INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
        version     INTEGER NOT NULL,
        quality     TEXT NOT NULL,
        analyzed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );
    CREATE TABLE scenes (
        id           INTEGER PRIMARY KEY,
        media_id     INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        idx          INTEGER NOT NULL,
        start_ms     INTEGER NOT NULL,
        end_ms       INTEGER NOT NULL CHECK (end_ms > start_ms),
        shake        REAL NOT NULL,
        frame_path   TEXT,
        samples_json TEXT NOT NULL,
        UNIQUE (media_id, idx)
    );",
    // 5: installed AI models (no row = not installed); manifests live in models.json
    "CREATE TABLE models (
        id         TEXT PRIMARY KEY,
        version    TEXT NOT NULL,
        state      TEXT NOT NULL,
        error      TEXT,
        path       TEXT,
        details    TEXT,
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );",
    // 6: speech transcripts (whisper.cpp); segments are ordered {startMs, endMs, text}
    "CREATE TABLE transcripts (
        media_id      INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
        model_id      TEXT NOT NULL,
        language      TEXT,
        segments_json TEXT NOT NULL,
        created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );",
    // 7: OpenCLIP tag observations (worker tagging.py), sampled over time; summaries computed on read
    "CREATE TABLE media_tags (
        media_id   INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
        model_id   TEXT NOT NULL,
        device     TEXT NOT NULL,
        tags_json  TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );",
    // 8: Editing Core — user marks (override automation) and the latest Editing Plan per project
    "CREATE TABLE media_marks (
        media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
        favorite INTEGER NOT NULL DEFAULT 0,
        excluded INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE editing_plans (
        project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        style      TEXT NOT NULL,
        plan_json  TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );",
];

pub fn err(e: rusqlite::Error) -> String {
    format!("Database error: {e}")
}

pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    }
    let c = Connection::open(path).map_err(err)?;
    c.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())).map_err(err)?;
    init(c)
}

#[cfg(test)]
pub fn open_in_memory() -> Connection {
    init(Connection::open_in_memory().unwrap()).unwrap()
}

fn init(mut c: Connection) -> Result<Connection, String> {
    c.pragma_update(None, "foreign_keys", true).map_err(err)?;
    c.busy_timeout(Duration::from_secs(5)).map_err(err)?;
    migrate(&mut c)?;
    Ok(c)
}

fn migrate(c: &mut Connection) -> Result<(), String> {
    let applied: usize = c.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(err)?;
    if applied > MIGRATIONS.len() {
        // Opened by an older app version: refuse rather than risk corrupting newer data.
        return Err(format!(
            "This database was created by a newer version of Memora Studio (schema {applied}, supported {}). Please update the app.",
            MIGRATIONS.len()
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = c.transaction().map_err(err)?;
        tx.execute_batch(sql).map_err(err)?;
        tx.pragma_update(None, "user_version", i + 1).map_err(err)?;
        tx.commit().map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(c: &Connection) -> usize {
        c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn fresh_database_gets_all_migrations() {
        assert_eq!(version(&open_in_memory()), MIGRATIONS.len());
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut c = open_in_memory();
        migrate(&mut c).unwrap();
        assert_eq!(version(&c), MIGRATIONS.len());
    }

    #[test]
    fn refuses_newer_schema() {
        let mut c = open_in_memory();
        c.pragma_update(None, "user_version", MIGRATIONS.len() + 1).unwrap();
        assert!(migrate(&mut c).unwrap_err().contains("newer version"));
    }

    #[test]
    fn opens_file_with_wal() {
        let dir = std::env::temp_dir().join(format!("memora-db-test-{}", std::process::id()));
        let path = dir.join("database").join("memora.db");
        let c = open(&path).unwrap();
        let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
