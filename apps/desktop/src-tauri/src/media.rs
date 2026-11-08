//! Media library (docs/Production.md §5.2, §7). Originals are referenced by path and only ever read.
//! Metadata comes from FFprobe; the raw FFprobe JSON is kept so nothing from the source is lost.

use crate::db::err;
use crate::jobs::{Job, Outcome};
use crate::process::{JobObject, command};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub duration_ms: i64,
    /// Coded size; swap for display when `rotation` is 90 or 270.
    pub width: i64,
    pub height: i64,
    /// Display rotation in degrees counter-clockwise (FFmpeg display-matrix convention), 0..359.
    pub rotation: i64,
    pub fps: Option<f64>,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub audio_streams: i64,
    /// Video stream bitrate, else container bitrate (bits/s).
    pub bitrate: Option<i64>,
    /// Recording time as written by the camera/phone, unmodified.
    pub source_created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: i64,
    pub path: String,
    pub size: i64,
    #[serde(flatten)]
    pub probe: Probe,
    pub imported_at: String,
    /// Cached files, filled by `proxy::attach_cache` (absolute paths, None when not generated yet).
    pub thumb: Option<PathBuf>,
    pub proxy: Option<PathBuf>,
    /// Number of detected scenes; None until analysed. Filled by the list command.
    pub scene_count: Option<i64>,
    /// True once transcribed (`transcribe.rs`). Filled by the list command.
    pub transcribed: bool,
    /// Visual concepts over the whole clip (`tagging.rs`), None until tagged. Filled by the list command.
    pub tags: Option<Vec<crate::tagging::TagScore>>,
    /// User override (`editing.rs`). Filled by the list command.
    pub mark: crate::editing::Mark,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

fn rate(v: &Value) -> Option<f64> {
    let (n, d) = v.as_str()?.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (n > 0.0 && d > 0.0).then(|| n / d)
}

pub fn parse_probe(probe: &Value) -> Result<Probe, String> {
    let streams = probe["streams"].as_array().map(Vec::as_slice).unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .ok_or("No video stream found (not a video file?).")?;
    let audio: Vec<&Value> = streams.iter().filter(|s| s["codec_type"] == "audio").collect();
    let format = &probe["format"];

    let seconds = num(&format["duration"]).or_else(|| num(&video["duration"])).unwrap_or(0.0);
    if seconds <= 0.0 {
        return Err("File has no duration (still image or damaged file?).".into());
    }
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| num(&d["rotation"])))
        // Legacy `rotate` tag is clockwise.
        .or_else(|| num(&video["tags"]["rotate"]).map(|r| -r))
        .unwrap_or(0.0);
    let tag = |k: &str| format["tags"][k].as_str().or_else(|| video["tags"][k].as_str()).map(String::from);

    Ok(Probe {
        duration_ms: (seconds * 1000.0).round() as i64,
        width: video["width"].as_i64().ok_or("Video stream has no width.")?,
        height: video["height"].as_i64().ok_or("Video stream has no height.")?,
        rotation: (rotation.round() as i64).rem_euclid(360),
        fps: rate(&video["avg_frame_rate"]).or_else(|| rate(&video["r_frame_rate"])),
        video_codec: video["codec_name"].as_str().unwrap_or("unknown").into(),
        audio_codec: audio.first().and_then(|a| a["codec_name"].as_str()).map(String::from),
        audio_streams: audio.len() as i64,
        bitrate: num(&video["bit_rate"]).or_else(|| num(&format["bit_rate"])).map(|b| b as i64),
        // Apple's tag carries the local time zone; creation_time is UTC.
        source_created_at: tag("com.apple.quicktime.creationdate").or_else(|| tag("creation_time")),
    })
}

/// Packaged FFmpeg under the data root, else PATH, else the winget link folder (dev machines
/// where the app was started before PATH was refreshed).
/// `exe` is `ffprobe.exe` or `ffmpeg.exe`.
pub fn tool_path(data_root: Option<&Path>, exe: &str) -> PathBuf {
    let bundled = data_root.map(|r| r.join("runtime").join("ffmpeg").join(exe));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join(exe));
    // ponytail: winget fallback is a dev convenience; drop once the installer ships runtime/ffmpeg.
    let winget = std::env::var_os("LOCALAPPDATA")
        .map(|l| PathBuf::from(l).join("Microsoft").join("WinGet").join("Links").join(exe));
    bundled.into_iter().chain(on_path).chain(winget).find(|p| p.exists()).unwrap_or_else(|| exe.into())
}

/// FFprobe reads headers only (seconds at most on local files); assumed limit, not measured on slow network drives.
const FFPROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Runs FFprobe and returns its raw JSON. Killed after `FFPROBE_TIMEOUT` so a hung probe cannot block the queue.
pub fn run_ffprobe(ffprobe: &Path, file: &Path, job_object: &JobObject) -> Result<String, String> {
    let mut cmd = command(ffprobe);
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = crate::process::spawn(&mut cmd, job_object).map_err(|e| format!("Cannot start FFprobe ({}): {e}", ffprobe.display()))?;
    let out = crate::process::wait_output(child, FFPROBE_TIMEOUT, &|| false).map_err(|e| format!("FFprobe failed on {}: {e}", file.display()))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(format!("FFprobe could not read the file: {}", msg.trim()));
    }
    String::from_utf8(out.stdout).map_err(|_| "FFprobe returned invalid output.".into())
}

const COLUMNS: &str = "id, path, size, duration_ms, width, height, rotation, fps, video_codec, audio_codec,
    audio_streams, bitrate, source_created_at, imported_at";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Media> {
    Ok(Media {
        id: r.get(0)?,
        path: r.get(1)?,
        size: r.get(2)?,
        probe: Probe {
            duration_ms: r.get(3)?,
            width: r.get(4)?,
            height: r.get(5)?,
            rotation: r.get(6)?,
            fps: r.get(7)?,
            video_codec: r.get(8)?,
            audio_codec: r.get(9)?,
            audio_streams: r.get(10)?,
            bitrate: r.get(11)?,
            source_created_at: r.get(12)?,
        },
        imported_at: r.get(13)?,
        thumb: None,
        proxy: None,
        scene_count: None,
        transcribed: false,
        tags: None,
        mark: Default::default(),
    })
}

/// Inserts or refreshes (same path, case-insensitive) one library entry.
pub fn upsert(c: &Connection, path: &str, size: i64, modified_unix: Option<i64>, p: &Probe, raw: &str) -> Result<Media, String> {
    c.query_row(
        &format!(
            "INSERT INTO media (path, size, modified_unix, duration_ms, width, height, rotation, fps, video_codec,
                audio_codec, audio_streams, bitrate, source_created_at, probe_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT (path) DO UPDATE SET
                size = excluded.size, modified_unix = excluded.modified_unix, duration_ms = excluded.duration_ms,
                width = excluded.width, height = excluded.height, rotation = excluded.rotation, fps = excluded.fps,
                video_codec = excluded.video_codec, audio_codec = excluded.audio_codec,
                audio_streams = excluded.audio_streams, bitrate = excluded.bitrate,
                source_created_at = excluded.source_created_at, probe_json = excluded.probe_json
             RETURNING {COLUMNS}"
        ),
        params![
            path, size, modified_unix, p.duration_ms, p.width, p.height, p.rotation, p.fps, p.video_codec,
            p.audio_codec, p.audio_streams, p.bitrate, p.source_created_at, raw
        ],
        row,
    )
    .map_err(err)
}

/// Newest recordings first; files without a recording time last.
pub fn list(c: &Connection) -> Result<Vec<Media>, String> {
    let mut s = c
        .prepare(&format!(
            "SELECT {COLUMNS} FROM media ORDER BY source_created_at IS NULL, source_created_at DESC, id DESC"
        ))
        .map_err(err)?;
    s.query_map([], row).map_err(err)?.collect::<Result<_, _>>().map_err(err)
}

/// Removes the library entry only; the original file is not touched.
pub fn remove(c: &Connection, id: i64) -> Result<(), String> {
    match c.execute("DELETE FROM media WHERE id = ?1", [id]).map_err(err)? {
        0 => Err(format!("Media {id} not found.")),
        _ => Ok(()),
    }
}

pub fn get(c: &Connection, id: i64) -> Result<Option<Media>, String> {
    c.query_row(&format!("SELECT {COLUMNS} FROM media WHERE id = ?1"), [id], row).optional().map_err(err)
}

/// What media jobs need: tools, the cache folder (`<data root>\cache`), the job object and the database.
pub struct MediaEnv<'a> {
    pub ffprobe: PathBuf,
    pub ffmpeg: PathBuf,
    pub cache: PathBuf,
    pub job_object: &'a JobObject,
    pub db: &'a Mutex<Result<Connection, String>>,
}

impl MediaEnv<'_> {
    pub fn with_db<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let guard = self.db.lock().unwrap();
        f(guard.as_ref().map_err(Clone::clone)?)
    }
}

/// Probes and stores one file, then refreshes its thumbnail. Returns the media id.
fn import_one(file: &Path, env: &MediaEnv) -> Result<i64, String> {
    let meta = std::fs::metadata(file).map_err(|e| format!("Cannot read file: {e}"))?;
    if !meta.is_file() {
        return Err("Not a file.".into());
    }
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    let raw = run_ffprobe(&env.ffprobe, file, env.job_object)?;
    let probe = parse_probe(&serde_json::from_str(&raw).map_err(|e| format!("Invalid FFprobe output: {e}"))?)?;
    let path = file.to_string_lossy();
    let (media, changed) = env.with_db(|c| {
        let before: Option<(i64, Option<i64>)> = c
            .query_row("SELECT size, modified_unix FROM media WHERE path = ?1", [&path], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(err)?;
        let m = upsert(c, &path, meta.len() as i64, modified, &probe, &raw)?;
        let changed = before.is_some_and(|b| b != (meta.len() as i64, modified));
        if changed {
            crate::analysis::clear(c, m.id)?;
            crate::transcribe::clear(c, m.id)?;
            crate::tagging::clear(c, m.id)?;
        }
        Ok((m, changed))
    })?;
    if changed {
        crate::proxy::remove_cache(&env.cache, media.id);
    }
    if let Err(e) = crate::proxy::make_thumbnail(&media, env) {
        tracing::warn!(media_id = media.id, error = %e, "thumbnail failed");
    }
    Ok(media.id)
}

/// Job `media.import`, params `{"paths": [..]}`. Per-file failures are reported, not fatal.
pub fn import_job(job: &Job, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool, env: &MediaEnv) -> Outcome {
    let Some(paths) = job.params["paths"].as_array() else {
        return Outcome::Failed("media.import needs params.paths".into());
    };
    let total = paths.len() as u64;
    let (mut ids, mut failed) = (vec![], vec![]);
    progress(0, total);
    for (i, p) in paths.iter().enumerate() {
        if cancelled() {
            return Outcome::Cancelled;
        }
        let path = p.as_str().unwrap_or_default();
        match import_one(Path::new(path), env) {
            Ok(id) => ids.push(id),
            Err(e) => {
                tracing::warn!(path, error = %e, "media import failed");
                failed.push(json!({"path": path, "error": e}));
            }
        }
        progress(i as u64 + 1, total);
    }
    tracing::info!(imported = ids.len(), failed = failed.len(), "media import finished");
    Outcome::Completed(json!({"imported": ids.len(), "ids": ids, "failed": failed}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    /// Trimmed real output of ffprobe 9.0.2 for a 320x240 clip remuxed with `-display_rotation 90`.
    fn sample() -> Value {
        json!({
            "streams": [
                {"codec_name": "h264", "codec_type": "video", "width": 320, "height": 240,
                 "r_frame_rate": "25/1", "avg_frame_rate": "25/1", "duration": "1.000000", "bit_rate": "287736",
                 "disposition": {"attached_pic": 0},
                 "side_data_list": [{"side_data_type": "Display Matrix", "rotation": 90}]},
                {"codec_name": "aac", "codec_type": "audio", "r_frame_rate": "0/0", "avg_frame_rate": "0/0"}
            ],
            "format": {"duration": "1.000000", "size": "47239", "bit_rate": "377912", "tags": {"encoder": "Lavf63.1.102"}}
        })
    }

    #[test]
    fn parses_ffprobe_output() {
        let p = parse_probe(&sample()).unwrap();
        assert_eq!(
            p,
            Probe {
                duration_ms: 1000, width: 320, height: 240, rotation: 90, fps: Some(25.0),
                video_codec: "h264".into(), audio_codec: Some("aac".into()), audio_streams: 1,
                bitrate: Some(287736), source_created_at: None,
            }
        );
    }

    #[test]
    fn phone_metadata_rotation_and_dates() {
        let mut v = sample();
        v["streams"][0]["side_data_list"][0]["rotation"] = json!(-90);
        v["streams"][0]["avg_frame_rate"] = json!("30000/1001");
        v["format"]["tags"] = json!({"creation_time": "2026-05-01T08:00:00.000000Z",
                                     "com.apple.quicktime.creationdate": "2026-05-01T15:00:00+0700"});
        let p = parse_probe(&v).unwrap();
        assert_eq!(p.rotation, 270);
        assert!((p.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(p.source_created_at.as_deref(), Some("2026-05-01T15:00:00+0700"));

        v["streams"][0].as_object_mut().unwrap().remove("side_data_list");
        v["streams"][0]["tags"] = json!({"rotate": "90"}); // legacy tag, clockwise
        assert_eq!(parse_probe(&v).unwrap().rotation, 270);
    }

    #[test]
    fn rejects_non_videos() {
        let audio_with_cover = json!({
            "streams": [{"codec_type": "audio", "codec_name": "mp3"},
                        {"codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}}],
            "format": {"duration": "200.0"}
        });
        assert!(parse_probe(&audio_with_cover).unwrap_err().contains("No video stream"));
        let still = json!({"streams": [{"codec_type": "video", "codec_name": "mjpeg", "width": 10, "height": 10}], "format": {}});
        assert!(parse_probe(&still).unwrap_err().contains("no duration"));
    }

    #[test]
    fn upsert_list_remove() {
        let c = open_in_memory();
        let p = parse_probe(&sample()).unwrap();
        let a = upsert(&c, r"D:\clips\a.mp4", 100, Some(1), &p, "{}").unwrap();
        // Same file, different case: refreshed, not duplicated.
        let again = upsert(&c, r"d:\CLIPS\A.mp4", 200, Some(2), &p, "{}").unwrap();
        assert_eq!((again.id, again.size), (a.id, 200));
        let mut newer = p.clone();
        newer.source_created_at = Some("2026-06-01T00:00:00Z".into());
        let b = upsert(&c, r"D:\clips\b.mp4", 1, None, &newer, "{}").unwrap();
        let ids: Vec<i64> = list(&c).unwrap().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![b.id, a.id], "dated recordings first");
        remove(&c, a.id).unwrap();
        assert!(get(&c, a.id).unwrap().is_none());
        assert!(remove(&c, a.id).is_err());
    }

    /// Real FFprobe on a synthetic clip; skipped when FFmpeg is not installed.
    #[test]
    fn import_job_with_real_ffprobe() {
        let ffprobe = tool_path(None, "ffprobe.exe");
        let ffmpeg = ffprobe.with_file_name("ffmpeg.exe");
        if !ffprobe.exists() || !ffmpeg.exists() {
            eprintln!("skipped: FFmpeg not found");
            return;
        }
        let dir = std::env::temp_dir().join(format!("memora-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=320x240:d=2:r=30", "-f", "lavfi", "-i", "sine=d=2"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-shortest"])
            .arg(&clip)
            .status()
            .unwrap()
            .success();
        assert!(ok, "ffmpeg failed to create the test clip");
        let not_video = dir.join("notes.txt");
        std::fs::write(&not_video, "hello").unwrap();

        let db = Mutex::new(Ok(open_in_memory()));
        let job_object = JobObject::new().unwrap();
        let env = MediaEnv { ffprobe, ffmpeg, cache: dir.join("cache"), job_object: &job_object, db: &db };
        let job = Job {
            id: 1, kind: "media.import".into(), status: crate::jobs::Status::Running, done: 0, total: 0,
            result: None, error: None,
            params: json!({"paths": [clip, not_video, dir.join("missing.mp4")]}),
        };
        let Outcome::Completed(r) = import_job(&job, &|_, _| {}, &|| false, &env) else {
            panic!("import job did not complete");
        };
        assert_eq!(r["imported"], 1);
        assert_eq!(r["failed"].as_array().unwrap().len(), 2);
        let media = list(db.lock().unwrap().as_ref().unwrap()).unwrap();
        let m = &media[0];
        assert_eq!((m.probe.width, m.probe.height, m.probe.audio_streams), (320, 240, 1));
        assert!((m.probe.duration_ms - 2000).abs() < 100, "duration {}", m.probe.duration_ms);
        assert_eq!(m.probe.fps, Some(30.0));
        assert!(crate::proxy::thumb_path(&env.cache, m.id).exists(), "thumbnail generated on import");

        // Proxy job: progress reaches the total, output is a decodable small H.264 file.
        let proxy_job = Job { kind: "media.proxy".into(), params: json!({"ids": [m.id]}), ..job };
        let last = Mutex::new((0, 0));
        let Outcome::Completed(r) =
            crate::proxy::proxy_job(&proxy_job, &|d, t| *last.lock().unwrap() = (d, t), &|| false, &env)
        else {
            panic!("proxy job did not complete");
        };
        let (d, t) = *last.lock().unwrap();
        assert_eq!(d, t, "progress complete: {r}");
        let out = crate::proxy::proxy_path(&env.cache, m.id);
        let p = parse_probe(&serde_json::from_str(&run_ffprobe(&env.ffprobe, &out, &job_object).unwrap()).unwrap()).unwrap();
        assert_eq!((p.video_codec.as_str(), p.height, p.audio_streams), ("h264", 240, 1), "small source is not upscaled");
        assert!(clip.exists(), "original untouched");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
