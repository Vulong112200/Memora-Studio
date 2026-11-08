//! SQLite database at `<data root>\database\memora.db`.
//! Schema migrations are an append-only list applied in order; `PRAGMA user_version` = number applied.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
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
    // 9: per-clip captions and per-project export options (look, music, titles)
    "ALTER TABLE project_clips ADD COLUMN caption TEXT;
    CREATE TABLE project_settings (
        project_id   INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        export_json  TEXT NOT NULL
    );",
];

pub fn err(e: rusqlite::Error) -> String {
    format!("Database error: {e}")
}

/// Backups kept in `<database folder>\backups\`; older ones are deleted (the database is small, < 1 MB today).
const KEEP_BACKUPS: usize = 10;

pub fn backups_dir(db_path: &Path) -> PathBuf {
    db_path.parent().unwrap_or(Path::new(".")).join("backups")
}

/// Opens (and migrates) the database. An existing database is first copied to `backups\`: at start-up (unless
/// a copy is less than 10 minutes old), and always before a schema migration — if that copy fails, the migration is not run.
/// Restore: close the app, copy a backup over `memora.db`, delete `memora.db-wal` and `memora.db-shm`.
pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    }
    let c = Connection::open(path).map_err(err)?;
    c.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())).map_err(err)?;
    let applied = user_version(&c)?;
    let pending = applied < MIGRATIONS.len();
    // Start-up copies are skipped when one is fresh (the command-line tool opens the database per command).
    if applied > 0 && (pending || !backed_up_within(&backups_dir(path), STARTUP_BACKUP_INTERVAL)) {
        match backup(&c, &backups_dir(path), if pending { "before-upgrade" } else { "startup" }) {
            Ok(p) => tracing::info!(backup = %p.display(), "database backed up"),
            Err(e) if pending => return Err(format!("Cannot back up the database before upgrading it, so it was not changed: {e}")),
            Err(e) => tracing::warn!(error = %e, "database backup failed"),
        }
    }
    init(c)
}

const STARTUP_BACKUP_INTERVAL: Duration = Duration::from_secs(10 * 60);

fn backed_up_within(dir: &Path, interval: Duration) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok()?.elapsed().ok())
        .any(|age| age < interval)
}

fn user_version(c: &Connection) -> Result<usize, String> {
    c.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(err)
}

/// Consistent copy of the open database (`VACUUM INTO` includes committed WAL content), checked with
/// `PRAGMA integrity_check`, named `memora-<UTC time>-v<schema>-<label>.db`. Keeps the newest `KEEP_BACKUPS`.
pub fn backup(c: &Connection, dir: &Path, label: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let stamp: String = c.query_row("SELECT strftime('%Y%m%d-%H%M%f', 'now')", [], |r| r.get(0)).map_err(err)?;
    let base = format!("memora-{}-v{}-{label}", stamp.replace('.', ""), user_version(c)?);
    let path = (1..).map(|i| dir.join(if i == 1 { format!("{base}.db") } else { format!("{base}-{i}.db") })).find(|p| !p.exists()).unwrap();
    c.execute("VACUUM INTO ?1", [path.to_string_lossy()]).map_err(err)?;
    let check = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|b| b.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)));
    if check.as_deref() != Ok("ok") {
        let _ = std::fs::remove_file(&path);
        return Err(format!("The backup copy failed its integrity check: {check:?}"));
    }
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("memora-")) && p.extension().is_some_and(|x| x == "db"))
        .collect();
    old.sort(); // the timestamp follows the prefix, so name order is time order
    for p in old.iter().take(old.len().saturating_sub(KEEP_BACKUPS)) {
        let _ = std::fs::remove_file(p);
    }
    Ok(path)
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

    #[test]
    fn existing_database_is_backed_up_on_open_and_old_backups_pruned() {
        let dir = std::env::temp_dir().join(format!("memora-db-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("database").join("memora.db");
        let c = open(&path).unwrap();
        assert!(!backups_dir(&path).exists(), "a brand-new database has nothing to back up");
        c.execute("INSERT INTO projects (name) VALUES ('Đà Lạt')", []).unwrap(); // stays in the WAL file
        drop(c);

        let c = open(&path).unwrap();
        let backups: Vec<_> = std::fs::read_dir(backups_dir(&path)).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(backups.len(), 1);
        let name = backups[0].file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("memora-") && name.ends_with(&format!("-v{}-startup.db", MIGRATIONS.len())), "{name}");
        let copy = Connection::open(&backups[0]).unwrap();
        let n: String = copy.query_row("SELECT name FROM projects", [], |r| r.get(0)).unwrap();
        assert_eq!(n, "Đà Lạt", "committed WAL content is in the copy");
        drop(copy); // Windows cannot delete an open file
        drop(open(&path).unwrap());
        assert_eq!(std::fs::read_dir(backups_dir(&path)).unwrap().count(), 1, "no second start-up copy within the interval");

        for _ in 0..KEEP_BACKUPS + 2 {
            backup(&c, &backups_dir(&path), "manual").unwrap();
        }
        assert_eq!(std::fs::read_dir(backups_dir(&path)).unwrap().count(), KEEP_BACKUPS);
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
