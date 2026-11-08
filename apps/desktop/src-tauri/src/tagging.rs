//! Visual concept tags (Phase 5): the AI worker (workers/ai-worker/tagging.py) runs the OpenCLIP image
//! encoder (OpenVINO IR, GPU → CPU fallback) on the proxy every `EVERY_MS` and returns the top tags of a
//! fixed bilingual taxonomy. This module stores those observations and summarises them per time range.

use crate::db::err;
use crate::jobs::{Job, Outcome, Status};
use crate::media::{self, MediaEnv};
use crate::proxy::{self, RunError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const MODEL_ID: &str = "openclip-vit-b32";
const EVERY_MS: i64 = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagName {
    pub en: String,
    pub vi: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSample {
    pub at_ms: i64,
    /// (index into the tag list, probability), best first.
    pub tags: Vec<(usize, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTags {
    pub device: String,
    pub tags: Vec<TagName>,
    pub samples: Vec<TagSample>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagScore {
    pub en: String,
    pub vi: String,
    /// Mean probability over the samples in the range (0..1).
    pub score: f64,
}

/// Mean probability of each tag over samples in [start_ms, end_ms), best first, scores ≥ `min`.
/// A tag missing from a sample's top list counts as 0 there.
pub fn summarize(t: &MediaTags, start_ms: i64, end_ms: i64, min: f64) -> Vec<TagScore> {
    let inside: Vec<&TagSample> = t.samples.iter().filter(|s| s.at_ms >= start_ms && s.at_ms < end_ms).collect();
    if inside.is_empty() {
        return vec![];
    }
    let mut sum: HashMap<usize, f64> = HashMap::new();
    for s in &inside {
        for &(k, p) in &s.tags {
            *sum.entry(k).or_default() += p;
        }
    }
    let mut out: Vec<TagScore> = sum
        .into_iter()
        .filter_map(|(k, p)| {
            let name = t.tags.get(k)?;
            let score = p / inside.len() as f64;
            (score >= min).then(|| TagScore { en: name.en.clone(), vi: name.vi.clone(), score })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.en.cmp(&b.en)));
    out
}

pub fn store(c: &Connection, media_id: i64, result: &Value) -> Result<usize, String> {
    let t: MediaTags = serde_json::from_value(result.clone()).map_err(|e| format!("Invalid tagging result: {e}"))?;
    c.execute(
        "INSERT INTO media_tags (media_id, model_id, device, tags_json) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (media_id) DO UPDATE SET model_id = excluded.model_id, device = excluded.device,
            tags_json = excluded.tags_json, created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![media_id, MODEL_ID, t.device, serde_json::to_string(&t).unwrap()],
    )
    .map_err(err)?;
    Ok(t.samples.len())
}

pub fn get(c: &Connection, media_id: i64) -> Result<Option<MediaTags>, String> {
    let json: Option<String> = c
        .query_row("SELECT tags_json FROM media_tags WHERE media_id = ?1", [media_id], |r| r.get(0))
        .optional()
        .map_err(err)?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

pub fn all(c: &Connection) -> Result<HashMap<i64, MediaTags>, String> {
    let mut s = c.prepare("SELECT media_id, tags_json FROM media_tags").map_err(err)?;
    let rows = s.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map_err(err)?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, j) = row.map_err(err)?;
        if let Ok(t) = serde_json::from_str(&j) {
            out.insert(id, t);
        }
    }
    Ok(out)
}

pub fn clear(c: &Connection, media_id: i64) -> Result<(), String> {
    c.execute("DELETE FROM media_tags WHERE media_id = ?1", [media_id]).map_err(err).map(drop)
}

/// Job `media.tag`, params `{"ids": [..]}`. Creates missing proxies, then runs the worker's `tag_video`.
pub fn tag_job(
    job: &Job,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
    env: &MediaEnv,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
) -> Outcome {
    let Some(ids) = job.params["ids"].as_array() else {
        return Outcome::Failed("media.tag needs params.ids".into());
    };
    let model_dir = match env.with_db(|c| crate::models::ready_dir(c, MODEL_ID)) {
        Ok((_, dir)) => dir,
        Err(e) => return Outcome::Failed(e),
    };
    const STEP: u64 = 1000;
    const PROXY_PART: f64 = 300.0;
    let total = ids.len() as u64 * STEP;
    let (mut done, mut failed) = (vec![], vec![]);
    progress(0, total);
    for (i, id) in ids.iter().enumerate() {
        let base = i as u64 * STEP;
        let id = id.as_i64().unwrap_or_default();
        let m = match env.with_db(|c| media::get(c, id)) {
            Ok(Some(m)) => m,
            Ok(None) => {
                failed.push(json!({"id": id, "error": "Not in the library."}));
                continue;
            }
            Err(e) => return Outcome::Failed(e),
        };
        if let Err(e) = proxy::ensure_proxy(&m, env, &|f| progress(base + (f * PROXY_PART) as u64, total), cancelled) {
            match e {
                RunError::Cancelled => return Outcome::Cancelled,
                RunError::Failed(e) => {
                    failed.push(json!({"id": id, "error": format!("Proxy failed: {e}")}));
                    continue;
                }
            }
        }
        let sub = Job {
            id: job.id,
            kind: "tag_video".into(),
            params: json!({"path": proxy::proxy_path(&env.cache, id), "modelDir": model_dir, "everyMs": EVERY_MS}),
            status: Status::Running,
            done: 0,
            total: 0,
            result: None,
            error: None,
        };
        let step = |d: u64, t: u64| {
            let f = if t > 0 { d as f64 / t as f64 } else { 0.0 };
            progress(base + PROXY_PART as u64 + (f * (STEP as f64 - PROXY_PART)) as u64, total)
        };
        match ai(&sub, &step, cancelled) {
            Outcome::Completed(r) => match env.with_db(|c| store(c, id, &r)) {
                Ok(n) => {
                    tracing::info!(media_id = id, samples = n, device = r["device"].as_str(), seconds = r["seconds"].as_f64(), "media tagged");
                    done.push(json!({"id": id, "samples": n, "device": r["device"]}));
                }
                Err(e) => failed.push(json!({"id": id, "error": e})),
            },
            Outcome::Cancelled => return Outcome::Cancelled,
            Outcome::Failed(e) => {
                tracing::warn!(media_id = id, error = %e, "tagging failed");
                failed.push(json!({"id": id, "error": e}));
            }
        }
        progress(base + STEP, total);
    }
    Outcome::Completed(json!({"done": done, "failed": failed}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::media::{Probe, upsert};

    fn sample_result() -> Value {
        json!({"device": "GPU", "tags": [{"en": "bicycle", "vi": "xe đạp"}, {"en": "park", "vi": "công viên"}, {"en": "beach", "vi": "bãi biển"}],
               "samples": [{"atMs": 0, "tags": [[0, 0.9], [1, 0.1]]}, {"atMs": 2000, "tags": [[1, 0.6], [0, 0.3]]},
                           {"atMs": 4000, "tags": [[2, 0.8], [1, 0.2]]}]})
    }

    #[test]
    fn summarize_ranges() {
        let t: MediaTags = serde_json::from_value(sample_result()).unwrap();
        let s = summarize(&t, 0, 4000, 0.0);
        assert_eq!(s.iter().map(|x| x.en.as_str()).collect::<Vec<_>>(), ["bicycle", "park"]);
        assert!((s[0].score - 0.6).abs() < 1e-9 && (s[1].score - 0.35).abs() < 1e-9);
        assert_eq!(summarize(&t, 4000, 6000, 0.5)[0].vi, "bãi biển");
        assert!(summarize(&t, 0, 4000, 0.5).len() == 1, "min filters");
        assert!(summarize(&t, 9000, 9999, 0.0).is_empty());
    }

    #[test]
    fn store_get_cascade() {
        let c = open_in_memory();
        let probe = Probe {
            duration_ms: 6000, width: 960, height: 540, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
            audio_codec: None, audio_streams: 0, bitrate: None, source_created_at: None,
        };
        let m = upsert(&c, r"E:\v.mp4", 1, None, &probe, "{}").unwrap();
        assert_eq!(store(&c, m.id, &sample_result()).unwrap(), 3);
        assert_eq!(store(&c, m.id, &sample_result()).unwrap(), 3, "re-tagging replaces");
        assert_eq!(get(&c, m.id).unwrap().unwrap().device, "GPU");
        assert_eq!(all(&c).unwrap().len(), 1);
        assert!(store(&c, m.id, &json!({"samples": 1})).is_err());
        media::remove(&c, m.id).unwrap();
        assert!(get(&c, m.id).unwrap().is_none());
    }

    #[test]
    fn job_needs_installed_model() {
        let db = std::sync::Mutex::new(Ok(open_in_memory()));
        let jo = crate::process::JobObject::new().unwrap();
        let env = MediaEnv { ffprobe: "x".into(), ffmpeg: "x".into(), cache: std::env::temp_dir(), job_object: &jo, db: &db };
        let job = Job { id: 1, kind: "media.tag".into(), params: json!({"ids": [1]}), status: Status::Running, done: 0, total: 0, result: None, error: None };
        let mut ai = |_: &Job, _: &dyn Fn(u64, u64), _: &dyn Fn() -> bool| Outcome::Failed("unused".into());
        assert!(matches!(tag_job(&job, &|_, _| {}, &|| false, &env, &mut ai), Outcome::Failed(e) if e.contains("Install OpenCLIP")));
    }

    /// Real worker + real exported IR (skipped when the export or FFmpeg is missing).
    #[test]
    fn tag_job_with_real_worker_and_ir() {
        let ir = std::path::PathBuf::from(r"E:\Memora Data\downloads\openclip-export");
        let (ffprobe, ffmpeg) = (media::tool_path(None, "ffprobe.exe"), media::tool_path(None, "ffmpeg.exe"));
        if !ir.join("openclip_image.xml").exists() || !ffmpeg.exists() || crate::worker::resolve(None).is_err() {
            eprintln!("skipped: exported IR, FFmpeg or worker venv not found");
            return;
        }
        let dir = std::env::temp_dir().join(format!("memora-tag-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("bars.mp4");
        assert!(std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:d=5:r=30", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&clip).status().unwrap().success());
        let db = std::sync::Mutex::new(Ok(open_in_memory()));
        let jo = std::sync::Arc::new(crate::process::JobObject::new().unwrap());
        let env = MediaEnv { ffprobe, ffmpeg, cache: dir.join("cache"), job_object: &jo, db: &db };
        let raw = media::run_ffprobe(&env.ffprobe, &clip, &jo).unwrap();
        let p = media::parse_probe(&serde_json::from_str(&raw).unwrap()).unwrap();
        let m = env.with_db(|c| upsert(c, &clip.to_string_lossy(), 1, None, &p, &raw)).unwrap();
        env.with_db(|c| {
            c.execute("INSERT INTO models (id, version, state, path) VALUES (?1, '1', 'ready', ?2)",
                params![MODEL_ID, ir.to_string_lossy()]).map_err(err)
        }).unwrap();
        let mut ai = crate::worker::runner(|| None, jo.clone(), std::sync::Arc::new(std::sync::Mutex::new(None)));
        let job = Job { id: 9, kind: "media.tag".into(), params: json!({"ids": [m.id]}), status: Status::Running, done: 0, total: 0, result: None, error: None };
        let Outcome::Completed(r) = tag_job(&job, &|_, _| {}, &|| false, &env, &mut ai) else { panic!("tag job failed") };
        assert_eq!(r["failed"].as_array().unwrap().len(), 0, "{r}");
        let t = env.with_db(|c| get(c, m.id)).unwrap().unwrap();
        assert_eq!(t.samples.len(), 3, "samples at 0, 2, 4 s");
        assert!(t.samples.iter().all(|s| s.tags.len() == 5 && s.tags.iter().all(|&(k, p)| k < t.tags.len() && (0.0..=1.0).contains(&p))));
        drop(ai);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
