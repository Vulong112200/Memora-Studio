//! Classical analysis (Phase 4): the AI worker (workers/ai-worker/analysis.py) observes scenes and
//! sample-frame metrics on the preview proxy; this module stores those observations.
//! Times map 1:1 to the original file. Scoring lives in `scoring.rs` and is computed on read.

use crate::db::err;
use crate::jobs::{Job, Outcome, Status};
use crate::media::{self, MediaEnv};
use crate::proxy::{self, RunError};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Raw observations of one sample frame (see analysis.py `frame_metrics`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub at_ms: i64,
    /// Variance of the Laplacian at proxy resolution (540p).
    pub sharpness: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub clip_low: f64,
    pub clip_high: f64,
    pub black: bool,
    pub dhash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub id: i64,
    pub media_id: i64,
    pub index: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub shake: f64,
    pub frame_path: Option<String>,
    pub samples: Vec<Sample>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerScene {
    index: i64,
    start_ms: i64,
    end_ms: i64,
    shake: f64,
    frame_path: Option<String>,
    samples: Vec<Sample>,
}

/// Replaces the stored analysis of `media_id` with a worker result.
pub fn store(c: &Connection, media_id: i64, result: &Value) -> Result<usize, String> {
    let scenes: Vec<WorkerScene> =
        serde_json::from_value(result["scenes"].clone()).map_err(|e| format!("Invalid analysis result: {e}"))?;
    let tx = c.unchecked_transaction().map_err(err)?;
    clear(&tx, media_id)?;
    tx.execute(
        "INSERT INTO media_analysis (media_id, version, quality) VALUES (?1, ?2, ?3)",
        params![media_id, result["version"].as_i64().unwrap_or(0), result["quality"].as_str().unwrap_or("?")],
    )
    .map_err(err)?;
    for s in scenes.iter().filter(|s| s.end_ms > s.start_ms) {
        tx.execute(
            "INSERT INTO scenes (media_id, idx, start_ms, end_ms, shake, frame_path, samples_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![media_id, s.index, s.start_ms, s.end_ms, s.shake, s.frame_path, serde_json::to_string(&s.samples).unwrap()],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(scenes.len())
}

/// Forgets the analysis (e.g. the original file changed).
pub fn clear(c: &Connection, media_id: i64) -> Result<(), String> {
    c.execute("DELETE FROM scenes WHERE media_id = ?1", [media_id]).map_err(err)?;
    c.execute("DELETE FROM media_analysis WHERE media_id = ?1", [media_id]).map_err(err)?;
    Ok(())
}

pub fn scenes(c: &Connection, media_id: i64) -> Result<Vec<Scene>, String> {
    query_scenes(c, "WHERE media_id = ?1", [media_id])
}

/// (media id, scene index, representative dHash) for every analysed scene, for duplicate detection.
pub fn library_hashes(c: &Connection) -> Result<Vec<(i64, i64, String)>, String> {
    Ok(query_scenes(c, "", [])?
        .iter()
        .filter_map(|s| Some((s.media_id, s.index, crate::scoring::representative_hash(s)?.to_string())))
        .collect())
}

fn query_scenes(c: &Connection, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Scene>, String> {
    let mut s = c
        .prepare(&format!(
            "SELECT id, media_id, idx, start_ms, end_ms, shake, frame_path, samples_json FROM scenes {filter} ORDER BY media_id, idx"
        ))
        .map_err(err)?;
    let rows = s
        .query_map(args, |r| {
            let json: String = r.get(7)?;
            Ok(Scene {
                id: r.get(0)?,
                media_id: r.get(1)?,
                index: r.get(2)?,
                start_ms: r.get(3)?,
                end_ms: r.get(4)?,
                shake: r.get(5)?,
                frame_path: r.get(6)?,
                samples: serde_json::from_str(&json).unwrap_or_default(),
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// media id → number of scenes, for analysed media only.
pub fn scene_counts(c: &Connection) -> Result<HashMap<i64, i64>, String> {
    let mut s = c
        .prepare("SELECT a.media_id, (SELECT COUNT(*) FROM scenes s WHERE s.media_id = a.media_id) FROM media_analysis a")
        .map_err(err)?;
    let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// Job `media.analyze`, params `{"ids": [..], "quality": "fast"|"balanced"|"detailed"}`.
/// Creates missing proxies, then runs the worker's `analyze_video` on each.
pub fn analyze_job(
    job: &Job,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
    env: &MediaEnv,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
) -> Outcome {
    let Some(ids) = job.params["ids"].as_array() else {
        return Outcome::Failed("media.analyze needs params.ids".into());
    };
    let quality = job.params["quality"].as_str().unwrap_or("balanced");
    const STEP: u64 = 1000;
    const PROXY_PART: f64 = 300.0; // share of a file's progress spent on a missing proxy
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
            kind: "analyze_video".into(),
            params: json!({
                "path": proxy::proxy_path(&env.cache, id),
                "framesDir": env.cache.join("frames").join(id.to_string()),
                "quality": quality,
            }),
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
            Outcome::Completed(result) => match env.with_db(|c| store(c, id, &result)) {
                Ok(n) => {
                    tracing::info!(media_id = id, scenes = n, quality, "media analysed");
                    done.push(json!({"id": id, "scenes": n}));
                }
                Err(e) => failed.push(json!({"id": id, "error": e})),
            },
            Outcome::Cancelled => return Outcome::Cancelled,
            Outcome::Failed(e) => {
                tracing::warn!(media_id = id, error = %e, "analysis failed");
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

    fn sample(at_ms: i64) -> Value {
        json!({"atMs": at_ms, "sharpness": 100.0, "brightness": 0.5, "contrast": 0.2, "clipLow": 0.0,
               "clipHigh": 0.01, "black": false, "dhash": "00ff00ff00ff00ff"})
    }

    #[test]
    fn store_replaces_and_cascades() {
        let c = open_in_memory();
        let probe = Probe {
            duration_ms: 4000, width: 960, height: 540, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
            audio_codec: None, audio_streams: 0, bitrate: None, source_created_at: None,
        };
        let m = upsert(&c, r"E:\v.mp4", 1, None, &probe, "{}").unwrap();
        let result = json!({"version": 1, "quality": "balanced", "scenes": [
            {"index": 0, "startMs": 0, "endMs": 2000, "shake": 0.001, "framePath": null, "samples": [sample(0), sample(1000)]},
            {"index": 1, "startMs": 2000, "endMs": 4000, "shake": 0.02, "framePath": "x.jpg", "samples": [sample(3000)]}
        ]});
        assert_eq!(store(&c, m.id, &result).unwrap(), 2);
        assert_eq!(store(&c, m.id, &result).unwrap(), 2, "re-analysis replaces, not appends");
        let s = scenes(&c, m.id).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].samples.len(), 2);
        assert_eq!(s[1].frame_path.as_deref(), Some("x.jpg"));
        assert_eq!(scene_counts(&c).unwrap().get(&m.id), Some(&2));
        assert!(store(&c, m.id, &json!({"scenes": "nope"})).is_err());
        media::remove(&c, m.id).unwrap();
        assert!(scenes(&c, m.id).unwrap().is_empty());
        assert!(scene_counts(&c).unwrap().is_empty());
    }

    /// Full path: Rust job → proxy → real Python worker (PySceneDetect/OpenCV) → stored scenes.
    /// Skipped when FFmpeg or the dev worker venv is missing.
    #[test]
    fn analyze_job_with_real_worker() {
        let (ffprobe, ffmpeg) = (media::tool_path(None, "ffprobe.exe"), media::tool_path(None, "ffmpeg.exe"));
        if !ffprobe.exists() || !ffmpeg.exists() || crate::worker::resolve(None).is_err() {
            eprintln!("skipped: FFmpeg or worker venv not found");
            return;
        }
        let dir = std::env::temp_dir().join(format!("memora-analysis-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("cuts.mp4");
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:d=2:r=30", "-f", "lavfi", "-i", "smptebars=s=640x360:d=2:r=30"])
            .args(["-filter_complex", "[0]setsar=1[a];[1]setsar=1[b];[a][b]concat=n=2:v=1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let db = std::sync::Mutex::new(Ok(open_in_memory()));
        let job_object = std::sync::Arc::new(crate::process::JobObject::new().unwrap());
        let env = MediaEnv { ffprobe, ffmpeg, cache: dir.join("cache"), job_object: &job_object, db: &db };
        let raw = media::run_ffprobe(&env.ffprobe, &clip, &job_object).unwrap();
        let p = media::parse_probe(&serde_json::from_str(&raw).unwrap()).unwrap();
        let m = env.with_db(|c| upsert(c, &clip.to_string_lossy(), 1, None, &p, &raw)).unwrap();

        let mut ai = crate::worker::runner(|| None, job_object.clone(), std::sync::Arc::new(std::sync::Mutex::new(None)));
        let job = Job {
            id: 7, kind: "media.analyze".into(), params: json!({"ids": [m.id], "quality": "detailed"}),
            status: Status::Running, done: 0, total: 0, result: None, error: None,
        };
        let Outcome::Completed(r) = analyze_job(&job, &|_, _| {}, &|| false, &env, &mut ai) else { panic!("analysis job failed") };
        assert_eq!(r["failed"].as_array().unwrap().len(), 0, "{r}");
        let s = env.with_db(|c| scenes(c, m.id)).unwrap();
        assert_eq!(s.len(), 2, "one hard cut at 2 s: {:?}", s.iter().map(|x| (x.start_ms, x.end_ms)).collect::<Vec<_>>());
        assert!((s[1].start_ms - 2000).abs() <= 70);
        assert!(s.iter().all(|x| !x.samples.is_empty() && x.frame_path.as_ref().is_some_and(|f| std::path::Path::new(f).exists())));
        drop(ai); // stops the worker
        std::fs::remove_dir_all(dir).unwrap();
    }
}
