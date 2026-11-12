//! Photo editor (ADR 0006, slice B1): photos read in place (never modified), a non-destructive edit stack per photo,
//! and rendering by `workers/ai-worker/photo.py` in a dedicated worker process (`Lane`), so slider previews never
//! wait behind a long video job in the job queue. Classical processing only; no model is loaded.

use crate::db::err;
use crate::jobs::{Job, Outcome, Status};
use crate::process::JobObject;
use crate::worker::{self, Worker};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Offered in the file picker and accepted on import (OpenCV decodes them; HEIC / RAW are not in round 1).
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp"];
/// Long side of grid thumbnails and of live previews (photo.py tunes its radii for 1600 px, so previews match exports).
const THUMB_SIDE: i64 = 480;
const PREVIEW_SIDE: i64 = 1600;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub id: i64,
    pub path: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// EXIF DateTimeOriginal (camera local time), else the file's modified time (UTC).
    pub taken_at: Option<String>,
    /// The edit stack (photo.py keys); null when never edited.
    pub edits: Value,
    pub imported_at: String,
    /// `<cache>\photo\<id>\thumb.jpg` when it exists (set by `attach_thumbs`).
    pub thumb: Option<PathBuf>,
}

const COLUMNS: &str = "id, path, size, width, height, taken_at, edits_json, imported_at";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Photo> {
    let edits: Option<String> = r.get(6)?;
    Ok(Photo {
        id: r.get(0)?,
        path: r.get(1)?,
        size: r.get(2)?,
        width: r.get(3)?,
        height: r.get(4)?,
        taken_at: r.get(5)?,
        edits: edits.and_then(|e| serde_json::from_str(&e).ok()).unwrap_or(Value::Null),
        imported_at: r.get(7)?,
        thumb: None,
    })
}

/// Newest photo first (taken date, else import date).
pub fn list(c: &Connection) -> Result<Vec<Photo>, String> {
    let mut s = c.prepare(&format!("SELECT {COLUMNS} FROM photos ORDER BY COALESCE(taken_at, imported_at) DESC, id DESC")).map_err(err)?;
    let rows = s.query_map([], row).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

pub fn get(c: &Connection, id: i64) -> Result<Photo, String> {
    c.query_row(&format!("SELECT {COLUMNS} FROM photos WHERE id = ?1"), [id], row).optional().map_err(err)?.ok_or(format!("Photo {id} not found."))
}

/// Adds or refreshes a photo by path (case-insensitive); an existing edit stack is kept.
pub fn upsert(c: &Connection, path: &str, size: i64, modified_unix: Option<i64>) -> Result<i64, String> {
    c.query_row(
        "INSERT INTO photos (path, size, modified_unix) VALUES (?1, ?2, ?3)
         ON CONFLICT (path) DO UPDATE SET size = excluded.size, modified_unix = excluded.modified_unix
         RETURNING id",
        params![path, size, modified_unix],
        |r| r.get(0),
    )
    .map_err(err)
}

pub fn set_info(c: &Connection, id: i64, width: i64, height: i64, taken_at: Option<&str>) -> Result<(), String> {
    c.execute("UPDATE photos SET width = ?2, height = ?3, taken_at = ?4 WHERE id = ?1", params![id, width, height, taken_at]).map_err(err).map(drop)
}

/// Saves the edit stack (a JSON object, or null to reset). photo.py ignores unknown keys and clamps values.
pub fn set_edits(c: &Connection, id: i64, edits: &Value) -> Result<(), String> {
    if !(edits.is_object() || edits.is_null()) {
        return Err("Photo edits must be an object.".into());
    }
    let json = (!edits.is_null()).then(|| edits.to_string());
    match c.execute("UPDATE photos SET edits_json = ?2 WHERE id = ?1", params![id, json]).map_err(err)? {
        0 => Err(format!("Photo {id} not found.")),
        _ => Ok(()),
    }
}

/// Removes the photo from the library (the file stays on disk) and its cache folder.
pub fn remove(c: &Connection, cache: &Path, id: i64) -> Result<(), String> {
    c.execute("DELETE FROM photos WHERE id = ?1", [id]).map_err(err)?;
    let _ = std::fs::remove_dir_all(cache_dir(cache, id));
    Ok(())
}

pub fn cache_dir(cache: &Path, id: i64) -> PathBuf {
    cache.join("photo").join(id.to_string())
}

pub fn attach_thumbs(cache: &Path, photos: &mut [Photo]) {
    for p in photos {
        let t = cache_dir(cache, p.id).join("thumb.jpg");
        p.thumb = t.exists().then_some(t);
    }
}

/// A worker process used only by the photo editor (protocol and script shared with the job-queue worker).
/// Calls are serialised; a crashed worker is restarted on the next call.
pub struct Lane {
    data_root: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    job_object: JobObject,
    worker: Mutex<Option<Worker>>,
    next: AtomicU64,
}

impl Lane {
    pub fn new(data_root: impl Fn() -> Option<PathBuf> + Send + Sync + 'static) -> Result<Lane, String> {
        Ok(Lane { data_root: Box::new(data_root), job_object: JobObject::new()?, worker: Mutex::new(None), next: AtomicU64::new(1) })
    }

    // ponytail: the photo worker stays up for the session once used (no model in it, mostly OpenCV); stop it on idle if memory matters.
    pub fn call(&self, kind: &str, params: Value) -> Result<Value, String> {
        let mut guard = self.worker.lock().unwrap();
        if guard.is_none() {
            *guard = Some(Worker::spawn(&worker::resolve((self.data_root)().as_deref())?, &self.job_object)?);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let job = Job { id, kind: kind.into(), params, status: Status::Running, done: 0, total: 0, result: None, error: None };
        let (outcome, alive) = worker::run_on(guard.as_mut().unwrap(), &job, &|_, _| {}, &|| false);
        if !alive {
            *guard = None;
        }
        match outcome {
            Outcome::Completed(v) => Ok(v),
            Outcome::Failed(e) => Err(e),
            Outcome::Cancelled => Err("Cancelled.".into()),
        }
    }
}

fn modified_unix(m: &std::fs::Metadata) -> Option<i64> {
    m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

fn iso_utc(unix: i64) -> String {
    // Civil from days (H. Hinnant), UTC.
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub ids: Vec<i64>,
    pub failed: Vec<Value>,
}

/// Reads each file in place: library row, size and date (EXIF, else file time), thumbnail.
pub fn import(c: &Mutex<Result<Connection, String>>, lane: &Lane, cache: &Path, paths: &[String]) -> Result<ImportResult, String> {
    let mut out = ImportResult { ids: vec![], failed: vec![] };
    for path in paths {
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let result = (|| {
            if !EXTENSIONS.contains(&ext.as_str()) {
                return Err(format!("Not a supported photo type (.{ext})."));
            }
            let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
            let modified = modified_unix(&meta);
            let id = with(c, |c| upsert(c, path, meta.len() as i64, modified))?;
            let dir = cache_dir(cache, id);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(dir.join("before.jpg")); // the original may have changed
            let r = lane.call(
                "photo_render",
                json!({"path": path, "edits": {}, "maxSide": THUMB_SIDE, "out": dir.join("thumb.jpg"), "quality": 85, "info": true}),
            );
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    with(c, |c| c.execute("DELETE FROM photos WHERE id = ?1 AND width IS NULL", [id]).map_err(err))?;
                    return Err(e);
                }
            };
            let taken = r["taken"].as_str().map(String::from).or(modified.map(iso_utc));
            with(c, |c| set_info(c, id, r["width"].as_i64().unwrap_or(0), r["height"].as_i64().unwrap_or(0), taken.as_deref()))?;
            Ok(id)
        })();
        match result {
            Ok(id) => out.ids.push(id),
            Err(e) => out.failed.push(json!({"path": path, "error": e})),
        }
    }
    Ok(out)
}

fn with<T>(db: &Mutex<Result<Connection, String>>, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let guard = db.lock().unwrap();
    f(guard.as_ref().map_err(Clone::clone)?)
}

static PREVIEW_SEQ: AtomicU64 = AtomicU64::new(0);

/// Live preview at ≤ 1600 px; `before` = the photo without edits (rendered once). A new file name per preview
/// (the webview caches by URL); older previews of the photo are removed.
pub fn preview(lane: &Lane, cache: &Path, photo: &Photo, edits: &Value, show_mask: bool, before: bool) -> Result<PathBuf, String> {
    let dir = cache_dir(cache, photo.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if before {
        let out = dir.join("before.jpg");
        if !out.exists() {
            lane.call("photo_render", json!({"path": photo.path, "edits": {}, "maxSide": PREVIEW_SIDE, "out": out, "quality": 88}))?;
        }
        return Ok(out);
    }
    let out = dir.join(format!("preview-{}.jpg", PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed)));
    lane.call("photo_render", json!({"path": photo.path, "edits": edits, "maxSide": PREVIEW_SIDE, "out": out, "quality": 88, "showMask": show_mask}))?;
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("preview-") && e.path() != out {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Ok(out)
}

/// Full-size render to `<data root>\photos\exports\<name>-edit.jpg` (never replacing a file; written as `.part`, then renamed).
pub fn export(lane: &Lane, exports: &Path, photo: &Photo) -> Result<PathBuf, String> {
    std::fs::create_dir_all(exports).map_err(|e| format!("Cannot create {}: {e}", exports.display()))?;
    let stem = Path::new(&photo.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| format!("photo-{}", photo.id));
    let out = crate::export::free_name(&exports.join(format!("{stem}-edit.jpg")));
    let part = out.with_extension("jpg.part");
    let result = lane.call("photo_render", json!({"path": photo.path, "edits": photo.edits, "maxSide": 0, "out": part, "quality": 95}));
    let result = result.and_then(|_| std::fs::rename(&part, &out).map_err(|e| format!("Cannot write {}: {e}", out.display())));
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|()| out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Mutex<Result<Connection, String>> {
        let dir = std::env::temp_dir().join(format!("memora-photos-{}-{}", std::process::id(), PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed)));
        Mutex::new(crate::db::open(&dir.join("memora.db")))
    }

    #[test]
    fn rows_edits_and_dates() {
        let c = db();
        let id = with(&c, |c| upsert(c, r"E:\Ảnh\a.jpg", 10, Some(0))).unwrap();
        assert_eq!(with(&c, |c| upsert(c, r"e:\Ảnh\A.JPG", 12, Some(1))).unwrap(), id, "same file: ASCII case ignored (SQLite NOCASE, as for videos)");
        with(&c, |c| set_edits(c, id, &json!({"exposure": 20, "sky": 50}))).unwrap();
        with(&c, |c| set_info(c, id, 4000, 3000, Some("2026-02-20T10:15:30"))).unwrap();
        let p = with(&c, |c| get(c, id)).unwrap();
        assert_eq!((p.size, p.width, p.edits["sky"].as_i64()), (12, Some(4000), Some(50)));
        assert!(with(&c, |c| set_edits(c, id, &json!([1]))).is_err(), "only an object");
        with(&c, |c| set_edits(c, id, &Value::Null)).unwrap();
        assert!(with(&c, |c| get(c, id)).unwrap().edits.is_null());
        assert!(with(&c, |c| set_edits(c, 999, &json!({}))).is_err());
        let cache = std::env::temp_dir().join(format!("memora-photo-cache-{}", std::process::id()));
        std::fs::create_dir_all(cache_dir(&cache, id)).unwrap();
        with(&c, |c| remove(c, &cache, id)).unwrap();
        assert!(with(&c, list).unwrap().is_empty() && !cache_dir(&cache, id).exists());
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_771_582_530), "2026-02-20T10:15:30Z");
    }

    /// Real worker: import, preview (before / edited / sky mask), auto, export with a free name. Skips without the dev venv.
    #[test]
    fn import_preview_export_with_real_worker() {
        if worker::resolve(None).is_err() {
            eprintln!("skipped: workers/ai-worker/.venv not found");
            return;
        }
        let ffmpeg = crate::media::tool_path(None, "ffmpeg.exe");
        let dir = std::env::temp_dir().join(format!("memora-photo-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("ảnh thử.jpg");
        let st = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=640x360", "-frames:v", "1"])
            .arg(&src)
            .status();
        if !st.is_ok_and(|s| s.success()) {
            eprintln!("skipped: no ffmpeg to make a test photo");
            return;
        }
        let original = std::fs::read(&src).unwrap();
        let c = db();
        let lane = Lane::new(|| None).unwrap();
        let cache = dir.join("cache");
        let r = import(&c, &lane, &cache, &[src.to_string_lossy().to_string(), dir.join("x.mp4").to_string_lossy().to_string()]).unwrap();
        assert_eq!((r.ids.len(), r.failed.len()), (1, 1), "{r:?}");
        let mut p = with(&c, |c| get(c, r.ids[0])).unwrap();
        assert_eq!((p.width, p.height), (Some(640), Some(360)));
        assert!(p.taken_at.is_some(), "file time when there is no EXIF");
        assert!(cache_dir(&cache, p.id).join("thumb.jpg").exists());
        let before = preview(&lane, &cache, &p, &Value::Null, false, true).unwrap();
        let a = preview(&lane, &cache, &p, &json!({"exposure": 30, "sky": 60}), false, false).unwrap();
        let b = preview(&lane, &cache, &p, &json!({"exposure": 30}), true, false).unwrap();
        assert!(before.exists() && !a.exists() && b.exists() && a != b, "older preview removed, new name each time");
        let auto = lane.call("photo_auto", json!({"path": p.path})).unwrap();
        assert!(auto["edits"].is_object(), "{auto}");
        p.edits = json!({"exposure": 20, "clarity": 30, "look": "warm"});
        let exports = dir.join("photos").join("exports");
        let e1 = export(&lane, &exports, &p).unwrap();
        let e2 = export(&lane, &exports, &p).unwrap();
        assert_eq!(e1.file_name().unwrap(), "ảnh thử-edit.jpg");
        assert_eq!(e2.file_name().unwrap(), "ảnh thử-edit-2.jpg", "an earlier export is never replaced");
        assert_eq!(std::fs::read(&src).unwrap(), original, "original untouched");
        assert!(std::fs::read_dir(&exports).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
        drop(lane);
        let _ = std::fs::remove_dir_all(dir);
    }
}
