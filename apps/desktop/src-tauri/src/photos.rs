//! Photo editor (ADR 0006, slice B1): photos read in place (never modified), a non-destructive edit stack per photo,
//! and rendering by `workers/ai-worker/photo.py` in a dedicated worker process (`Lane`), so slider previews never
//! wait behind a long video job in the job queue. Classical processing only; no model is loaded.

use crate::db::err;
use crate::jobs::{Job, Outcome, Status};
use crate::process::JobObject;
use crate::worker::{self, Worker};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
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
    /// Observations stored (`photo_analysis`), so Smart edit can use the photo.
    pub analyzed: bool,
}

const COLUMNS: &str =
    "id, path, size, width, height, taken_at, edits_json, imported_at, EXISTS(SELECT 1 FROM photo_analysis a WHERE a.photo_id = photos.id)";

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
        analyzed: r.get(8)?,
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

/// What the Editing Core knows about a photo: frame metrics of the edited photo (same scale as video proxies) and
/// OpenCLIP tags / cues as a one-sample `MediaTags` (None when the model was not installed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub metrics: crate::analysis::Sample,
    pub tags: Option<crate::tagging::MediaTags>,
}

pub fn store_analysis(c: &Connection, id: i64, result: &Value) -> Result<(), String> {
    let mut m = result["metrics"].clone();
    m["atMs"] = json!(0);
    let metrics: crate::analysis::Sample = serde_json::from_value(m).map_err(|e| format!("Invalid photo metrics: {e}"))?;
    let tags: Option<crate::tagging::MediaTags> = match result.get("tags") {
        Some(t) if !t.is_null() => Some(serde_json::from_value(t.clone()).map_err(|e| format!("Invalid photo tags: {e}"))?),
        _ => None,
    };
    c.execute(
        "INSERT INTO photo_analysis (photo_id, metrics_json, tags_json, model_id) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (photo_id) DO UPDATE SET metrics_json = excluded.metrics_json, tags_json = excluded.tags_json,
            model_id = excluded.model_id, created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![id, serde_json::to_string(&metrics).unwrap(), tags.as_ref().map(|t| serde_json::to_string(t).unwrap()), tags.as_ref().map(|_| crate::tagging::MODEL_ID)],
    )
    .map_err(err)
    .map(drop)
}

pub fn analyses(c: &Connection) -> Result<std::collections::HashMap<i64, Analysis>, String> {
    let mut s = c.prepare("SELECT photo_id, metrics_json, tags_json FROM photo_analysis").map_err(err)?;
    let rows = s.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))).map_err(err)?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (id, m, t) = row.map_err(err)?;
        if let Ok(metrics) = serde_json::from_str(&m) {
            out.insert(id, Analysis { metrics, tags: t.and_then(|t| serde_json::from_str(&t).ok()) });
        }
    }
    Ok(out)
}

/// Job `media.photo_analyze`, params `{"ids": [..]}`: worker `photo_analyze` on each photo as edited (metrics, and
/// OpenCLIP tags when the model is installed), stored in `photo_analysis`. A photo that fails is reported, the rest go on.
pub fn analyze_job(
    job: &Job,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
    env: &crate::media::MediaEnv,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
) -> Outcome {
    let ids: Vec<i64> = job.params["ids"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
    let model_dir = env.with_db(|c| crate::models::ready_dir(c, crate::tagging::MODEL_ID)).ok().map(|(_, d)| d);
    let (mut done, mut failed) = (vec![], vec![]);
    for (i, &id) in ids.iter().enumerate() {
        if cancelled() {
            return Outcome::Cancelled;
        }
        progress(i as u64, ids.len() as u64);
        let p = match env.with_db(|c| get(c, id)) {
            Ok(p) => p,
            Err(e) => {
                failed.push(json!({"id": id, "error": e}));
                continue;
            }
        };
        let sub = Job {
            id: job.id,
            kind: "photo_analyze".into(),
            params: json!({"path": p.path, "edits": p.edits, "modelDir": model_dir}),
            status: Status::Running, done: 0, total: 0, result: None, error: None,
        };
        match ai(&sub, &|_, _| {}, cancelled) {
            Outcome::Completed(r) => match env.with_db(|c| store_analysis(c, id, &r)) {
                Ok(()) => done.push(id),
                Err(e) => failed.push(json!({"id": id, "error": e})),
            },
            Outcome::Failed(e) => failed.push(json!({"id": id, "error": e})),
            Outcome::Cancelled => return Outcome::Cancelled,
        }
    }
    progress(ids.len() as u64, ids.len() as u64);
    tracing::info!(done = done.len(), failed = failed.len(), tagged = model_dir.is_some(), "photos analysed");
    Outcome::Completed(json!({"done": done, "failed": failed, "tagged": model_dir.is_some()}))
}

/// Long side of each photo rendered for a video: covers a 1920-px frame and a centre crop of a 3:2 photo into 9:16.
const VIDEO_SIDE: i64 = 2880;

/// Job `media.photo_video`, params `{"photoIds": [n…] (in order), "photoMs": n, "preset": "landscape", "options": export::Options, "name": "…"}`:
/// each photo rendered with its edit stack (worker `photo_render`, into the render cache, deleted afterwards), then the
/// stills go through the normal export (`export::finish`: transitions, look, titles, music, loudness, validation) to
/// `<data root>\photos\exports\<name>-<preset>.mp4`. The photo files are never written.
pub fn video_job(
    job: &Job,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
    env: &crate::media::MediaEnv,
    root: &Path,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
) -> Outcome {
    use crate::export::{self, Preset};
    let ids: Vec<i64> = job.params["photoIds"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
    if ids.is_empty() {
        return Outcome::Failed("Pick at least one photo.".into());
    }
    let preset_name = job.params["preset"].as_str().unwrap_or("landscape");
    let preset = match Preset::parse(preset_name) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(e),
    };
    let o: export::Options = match serde_json::from_value(job.params["options"].clone()) {
        Ok(o) => o,
        Err(e) => return Outcome::Failed(format!("Invalid export options: {e}")),
    };
    let (lo, hi) = crate::timeline::PHOTO_MS;
    let photo_ms = job.params["photoMs"].as_i64().unwrap_or(3_000).clamp(lo, hi);
    let photos = match env.with_db(|c| ids.iter().map(|&id| get(c, id)).collect::<Result<Vec<_>, _>>()) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(e),
    };
    if let Some(p) = photos.iter().find(|p| !Path::new(&p.path).exists()) {
        return Outcome::Failed(format!("Photo file is missing: {}", p.path));
    }
    let dir = env.cache.join("render");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Outcome::Failed(format!("Cannot create {}: {e}", dir.display()));
    }
    // Rendering the stills is the first 20 % of the job.
    const TOTAL: u64 = 1000;
    let n = photos.len() as u64;
    let mut stills = vec![]; // deleted when dropped, whatever the outcome
    let mut segs = vec![];
    for (i, p) in photos.iter().enumerate() {
        if cancelled() {
            return Outcome::Cancelled;
        }
        progress(i as u64 * 200 / n, TOTAL);
        match still(job, ai, cancelled, &dir, p, photo_ms, i) {
            Ok((seg, file)) => {
                segs.push(seg);
                stills.push(file);
            }
            Err(outcome) => return outcome,
        }
    }
    let name = job.params["name"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Photo video");
    let safe: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let planned = export::free_name(&root.join("photos").join("exports").join(format!("{safe}-{preset_name}.mp4")));
    let target = preset.target(&segs[0].media);
    tracing::info!(photos = segs.len(), photo_ms, "photo video");
    let outcome = export::finish(env, &segs, target, preset_name, o, planned, &|d, t| progress(200 + d * 800 / t.max(1), TOTAL), cancelled);
    drop(stills);
    outcome
}

/// Media standing for a rendered still (`export::STILL`): id −photo id (`export::timeline_joins` keys), dated by the photo.
pub fn still_media(p: &Photo, file: &Path, width: i64, height: i64, ms: i64) -> crate::media::Media {
    let probe = crate::media::Probe {
        duration_ms: ms, width, height, rotation: 0, fps: None, video_codec: crate::export::STILL.into(), audio_codec: None, audio_streams: 0,
        bitrate: None, source_created_at: p.taken_at.clone(),
    };
    crate::media::Media {
        id: -p.id, path: file.to_string_lossy().into(), size: 0, probe, imported_at: p.imported_at.clone(), thumb: None, proxy: None,
        scene_count: None, transcribed: false, tags: None, mark: Default::default(),
    }
}

/// Renders one photo with its edits (job worker `photo_render`, long side `VIDEO_SIDE`) to `dir\still-<job>-<pid>-<i>.jpg`
/// (deleted when the returned guard drops) and returns it as a still segment of `ms`.
pub fn still(
    job: &Job,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
    cancelled: &dyn Fn() -> bool,
    dir: &Path,
    p: &Photo,
    ms: i64,
    i: usize,
) -> Result<(crate::export::Segment, crate::sounds::TempFile), Outcome> {
    std::fs::create_dir_all(dir).map_err(|e| Outcome::Failed(format!("Cannot create {}: {e}", dir.display())))?;
    let file = crate::sounds::TempFile(dir.join(format!("still-{}-{}-{i}.jpg", job.id, std::process::id())));
    let sub = Job {
        id: job.id,
        kind: "photo_render".into(),
        params: json!({"path": p.path, "edits": p.edits, "maxSide": VIDEO_SIDE, "out": file.0, "quality": 95}),
        status: Status::Running, done: 0, total: 0, result: None, error: None,
    };
    let r = match ai(&sub, &|_, _| {}, cancelled) {
        Outcome::Completed(r) => r,
        Outcome::Failed(e) => return Err(Outcome::Failed(format!("{}: {e}", p.path))),
        Outcome::Cancelled => return Err(Outcome::Cancelled),
    };
    let media = still_media(p, &file.0, r["outWidth"].as_i64().unwrap_or(0), r["outHeight"].as_i64().unwrap_or(0), ms);
    Ok((crate::export::Segment { media, in_ms: 0, out_ms: ms, caption: None, speech: vec![], transition: None }, file))
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

    /// Real worker + FFmpeg: two photos (one edited, one portrait) → photo video, validated, stills removed, photos untouched.
    #[test]
    fn photo_video_with_real_worker() {
        let ffmpeg = crate::media::tool_path(None, "ffmpeg.exe");
        if worker::resolve(None).is_err() || !ffmpeg.exists() {
            eprintln!("skipped: worker venv or FFmpeg not found");
            return;
        }
        let dir = std::env::temp_dir().join(format!("memora-photo-video-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (land, port) = (dir.join("land.jpg"), dir.join("port.png"));
        for (f, size) in [(&land, "640x360"), (&port, "360x640")] {
            let ok = std::process::Command::new(&ffmpeg).args(["-v", "error", "-f", "lavfi", "-i", &format!("testsrc2=s={size}"), "-frames:v", "1"]).arg(f).status().unwrap().success();
            assert!(ok);
        }
        let original = std::fs::read(&land).unwrap();
        let c = db();
        let lane = Lane::new(|| None).unwrap();
        let cache = dir.join("cache");
        let ids = import(&c, &lane, &cache, &[land.to_string_lossy().into(), port.to_string_lossy().into()]).unwrap().ids;
        with(&c, |c| set_edits(c, ids[0], &json!({"exposure": 25, "look": "warm"}))).unwrap();
        let job_object = JobObject::new().unwrap();
        let env = crate::media::MediaEnv {
            ffprobe: crate::media::tool_path(None, "ffprobe.exe"), ffmpeg, cache: cache.clone(), job_object: &job_object, db: &c,
        };
        let mut ai = |j: &Job, _: &dyn Fn(u64, u64), _: &dyn Fn() -> bool| match lane.call(&j.kind, j.params.clone()) {
            Ok(v) => Outcome::Completed(v),
            Err(e) => Outcome::Failed(e),
        };
        let job = Job {
            id: 7, kind: "media.photo_video".into(), status: Status::Running, done: 0, total: 0, result: None, error: None,
            params: json!({"photoIds": [ids[1], ids[0]], "photoMs": 1500, "preset": "landscape", "name": "Thử",
                "options": {"transitionMs": 500, "transition": "fade", "fade": true}}),
        };
        let Outcome::Completed(r) = video_job(&job, &|_, _| {}, &|| false, &env, &dir, &mut ai) else { panic!("photo video failed") };
        assert!(r["validation"]["ok"].as_bool().unwrap(), "{r}");
        assert_eq!((r["durationMs"].as_i64(), r["width"].as_i64()), (Some(2_500), Some(1920)), "two 1.5 s photos joined by 0.5 s");
        assert_eq!(Path::new(r["path"].as_str().unwrap()), dir.join("photos").join("exports").join("Thử-landscape.mp4"));
        assert!(std::fs::read_dir(cache.join("render")).unwrap().next().is_none(), "rendered stills removed");
        assert_eq!(std::fs::read(&land).unwrap(), original, "photo untouched");
        drop(lane);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Real worker + FFmpeg: a project timeline mixing a video clip and a photo with its own transition — export
    /// (validated, photo stills removed) and a join preview.
    #[test]
    fn mixed_timeline_export_and_join_preview() {
        let ffmpeg = crate::media::tool_path(None, "ffmpeg.exe");
        let ffprobe = crate::media::tool_path(None, "ffprobe.exe");
        if worker::resolve(None).is_err() || !ffmpeg.exists() || !ffprobe.exists() {
            eprintln!("skipped: worker venv or FFmpeg not found");
            return;
        }
        let dir = std::env::temp_dir().join(format!("memora-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (video, photo) = (dir.join("v.mp4"), dir.join("p.jpg"));
        let run = |args: &[&str], out: &Path| assert!(std::process::Command::new(&ffmpeg).args(["-v", "error"]).args(args).arg(out).status().unwrap().success());
        run(&["-f", "lavfi", "-i", "testsrc2=s=640x360:d=3:r=30", "-f", "lavfi", "-i", "sine=d=3", "-c:v", "libx264", "-c:a", "aac", "-shortest"], &video);
        run(&["-f", "lavfi", "-i", "testsrc2=s=360x640", "-frames:v", "1"], &photo);
        let c = db();
        let job_object = JobObject::new().unwrap();
        let cache = dir.join("cache");
        let env = crate::media::MediaEnv { ffprobe, ffmpeg, cache: cache.clone(), job_object: &job_object, db: &c };
        let raw = crate::media::run_ffprobe(&env.ffprobe, &video, &job_object).unwrap();
        let probe = crate::media::parse_probe(&serde_json::from_str(&raw).unwrap()).unwrap();
        let lane = Lane::new(|| None).unwrap();
        let photo_id = import(&c, &lane, &cache, &[photo.to_string_lossy().into()]).unwrap().ids[0];
        let project = with(&c, |c| {
            let m = crate::media::upsert(c, &video.to_string_lossy(), 1, None, &probe, &raw)?;
            let p = crate::projects::create(c, "Mixed")?;
            crate::timeline::add(c, p.id, m.id, 0, 3_000)?;
            let clip = crate::timeline::add_photo(c, p.id, photo_id, 2_000)?;
            crate::timeline::set_transition(c, clip.id, Some("fadeblack"), 500)?;
            Ok(p.id)
        })
        .unwrap();
        assert_eq!(with(&c, |c| crate::export::timeline_joins(c, project)).unwrap(), [("fadeblack".to_string(), 500)]);
        let mut ai = |j: &Job, _: &dyn Fn(u64, u64), _: &dyn Fn() -> bool| match lane.call(&j.kind, j.params.clone()) {
            Ok(v) => Outcome::Completed(v),
            Err(e) => Outcome::Failed(e),
        };
        let job = Job {
            id: 9, kind: "media.export".into(), status: Status::Running, done: 0, total: 0, result: None, error: None,
            params: json!({"projectId": project, "preset": "landscape", "options": {"transitionMs": 300, "transition": "fade"}}),
        };
        let Outcome::Completed(r) = crate::export::export_job(&job, &|_, _| {}, &|| false, &env, &dir.join("projects"), &mut ai) else { panic!("export failed") };
        assert!(r["validation"]["ok"].as_bool().unwrap(), "{r}");
        assert_eq!(r["durationMs"].as_i64(), Some(4_500), "3 s video + 2 s photo − 0.5 s fade through black");
        assert!(std::fs::read_dir(cache.join("render")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("still-")), "stills removed");
        let still = |p: &Photo, out: &Path| {
            let r = lane.call("photo_render", json!({"path": p.path, "edits": p.edits, "maxSide": 640, "out": out}))?;
            Ok((r["outWidth"].as_i64().unwrap_or(0), r["outHeight"].as_i64().unwrap_or(0)))
        };
        let preview = crate::export::preview_join(&env, project, 0, crate::export::Preset::Landscape, &still).unwrap();
        assert!(preview.exists() && std::fs::metadata(&preview).unwrap().len() > 0);
        assert!(crate::export::preview_join(&env, project, 1, crate::export::Preset::Landscape, &still).is_err(), "only one join");
        drop(lane);
        let _ = std::fs::remove_dir_all(dir);
    }
}
