//! Speech transcription (Phase 5): whisper.cpp `whisper-cli.exe` as a separate native process (ADR 0001).
//! Audio is extracted from the ORIGINAL (16 kHz mono WAV, temporary), transcribed with an installed Whisper
//! model from the Model Manager, and stored as timed segments. Times map 1:1 to the original file.

use crate::db::err;
use crate::jobs::{Job, Outcome};
use crate::media::{self, MediaEnv};
use crate::proxy::{self, RunError, os};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Pinned runtime (docs/MODEL_PROPOSAL.md). ponytail: one fixed version dir; resolve by manifest once a second version ships.
const WHISPER_DIR: &str = "b5454";
const VAD_ID: &str = "silero-vad";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub media_id: i64,
    pub model_id: String,
    /// Detected or requested language code ("vi", "en" …); None when the file has no audio.
    pub language: Option<String>,
    pub segments: Vec<Segment>,
    pub created_at: String,
}

pub fn whisper_cli(data_root: &Path) -> PathBuf {
    data_root.join("runtime").join("whisper.cpp").join(WHISPER_DIR).join("whisper-cli.exe")
}

/// Parses whisper-cli `-oj` output into (language, segments). Empty segments are dropped.
/// Hallucination loops are dropped (see `drop_loops`).
pub fn parse_output(v: &Value) -> Result<(Option<String>, Vec<Segment>), String> {
    let items = v["transcription"].as_array().ok_or("whisper.cpp output has no transcription")?;
    let segments = items
        .iter()
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim();
            let (start_ms, end_ms) = (s["offsets"]["from"].as_i64()?, s["offsets"]["to"].as_i64()?);
            (!text.is_empty() && end_ms >= start_ms).then(|| Segment { start_ms, end_ms, text: text.to_string() })
        })
        .collect();
    Ok((v["result"]["language"].as_str().map(String::from), drop_loops(segments)))
}

/// Runs of this many identical consecutive segments or more are a decoder loop on noise (seen on
/// wind/bike footage: "Cảm ơn các bạn." ×14) and are removed entirely.
// ponytail: text heuristic; VAD (silero) before decoding is the real fix, pending model approval.
const LOOP_RUN: usize = 4;

fn drop_loops(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out = vec![];
    let mut i = 0;
    while i < segments.len() {
        let run = segments[i..].iter().take_while(|s| s.text == segments[i].text).count();
        if run < LOOP_RUN {
            out.extend_from_slice(&segments[i..i + run]);
        }
        i += run;
    }
    out
}

pub fn store(c: &Connection, media_id: i64, model_id: &str, language: Option<&str>, segments: &[Segment]) -> Result<(), String> {
    c.execute(
        "INSERT INTO transcripts (media_id, model_id, language, segments_json) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (media_id) DO UPDATE SET model_id = excluded.model_id, language = excluded.language,
            segments_json = excluded.segments_json, created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![media_id, model_id, language, serde_json::to_string(segments).unwrap()],
    )
    .map_err(err)
    .map(drop)
}

pub fn get(c: &Connection, media_id: i64) -> Result<Option<Transcript>, String> {
    c.query_row(
        "SELECT model_id, language, segments_json, created_at FROM transcripts WHERE media_id = ?1",
        [media_id],
        |r| {
            let json: String = r.get(2)?;
            Ok(Transcript {
                media_id,
                model_id: r.get(0)?,
                language: r.get(1)?,
                segments: serde_json::from_str(&json).unwrap_or_default(),
                created_at: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(err)
}

/// Forgets the transcript (e.g. the original file changed).
pub fn clear(c: &Connection, media_id: i64) -> Result<(), String> {
    c.execute("DELETE FROM transcripts WHERE media_id = ?1", [media_id]).map_err(err).map(drop)
}

/// media ids that have a transcript, for the library list.
pub fn transcribed(c: &Connection) -> Result<std::collections::HashSet<i64>, String> {
    let mut s = c.prepare("SELECT media_id FROM transcripts").map_err(err)?;
    let rows = s.query_map([], |r| r.get(0)).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// Path of the installed (state "ready") model file, or a message telling the user what to install.
fn model_file(c: &Connection, model_id: &str) -> Result<PathBuf, String> {
    let (m, dir) = crate::models::ready_dir(c, model_id)?;
    Ok(dir.join(&m.artifacts.first().ok_or("manifest has no artifact")?.file))
}

/// Runs whisper-cli; `progress` gets 0..1 from its `-pp` output. Killed on cancel.
#[allow(clippy::too_many_arguments)]
fn run_whisper(
    cli: &Path,
    model: &Path,
    vad: Option<&Path>,
    wav: &Path,
    out_base: &Path,
    language: &str,
    job_object: &crate::process::JobObject,
    progress: &dyn Fn(f64),
    cancelled: &dyn Fn() -> bool,
) -> Result<(), RunError> {
    // ponytail: half the logical CPUs, max 8; measure before tuning further (render must keep running alongside).
    let threads = std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).clamp(1, 8));
    let mut cmd = crate::process::command(cli);
    if let Some(v) = vad {
        // Silero VAD: decode speech regions only (timestamps stay in the original timeline).
        cmd.args(["--vad".as_ref(), "-vm".as_ref(), v.as_os_str()]);
    }
    let mut child = cmd
        .args(["-m".as_ref(), model.as_os_str(), "-f".as_ref(), wav.as_os_str(), "-of".as_ref(), out_base.as_os_str()])
        // -mc 0: no text context between windows, -sns: suppress non-speech tokens. Both cut noise
        // hallucination loops sharply on the owner's DJI footage (docs/MODEL_PROPOSAL.md §4).
        .args(["-oj", "-np", "-pp", "-mc", "0", "-sns", "-l", language, "-t", &threads.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Failed(format!("Cannot start whisper.cpp ({}): {e}", cli.display())))?;
    if let Err(e) = job_object.assign(&child) {
        let _ = child.kill();
        return Err(RunError::Failed(e));
    }
    // stderr on a thread so cancellation is checked even while whisper prints nothing (model loading).
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let mut tail = std::collections::VecDeque::new();
    let status = loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError::Cancelled);
        }
        while let Ok(line) = rx.try_recv() {
            if let Some(p) = line.split("progress =").nth(1).and_then(|p| p.trim().trim_end_matches('%').trim().parse::<f64>().ok()) {
                progress((p / 100.0).clamp(0.0, 1.0));
            } else {
                tail.push_back(line);
                if tail.len() > 3 {
                    tail.pop_front();
                }
            }
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(RunError::Failed(format!("whisper.cpp failed: {e}"))),
        }
    };
    let _ = reader.join();
    if status.success() {
        return Ok(());
    }
    let tail: Vec<String> = tail.into_iter().chain(rx.try_iter()).collect();
    Err(RunError::Failed(format!("whisper.cpp exited with {status}: {}", tail.join(" | "))))
}

/// Transcribes one library file. Returns (language, segments).
#[allow(clippy::too_many_arguments)]
fn transcribe_one(
    m: &media::Media,
    env: &MediaEnv,
    cli: &Path,
    model: &Path,
    vad: Option<&Path>,
    language: &str,
    progress: &dyn Fn(f64),
    cancelled: &dyn Fn() -> bool,
) -> Result<(Option<String>, Vec<Segment>), RunError> {
    let dir = env.cache.join("audio");
    std::fs::create_dir_all(&dir).map_err(|e| RunError::Failed(format!("Cannot create {}: {e}", dir.display())))?;
    let wav = dir.join(format!("{}.wav", m.id));
    let out_base = dir.join(m.id.to_string());
    let json_path = out_base.with_extension("json");
    let result = (|| {
        const AUDIO_PART: f64 = 0.15;
        let mut args = os(&["-i"]);
        args.push(m.path.clone().into());
        args.extend(os(&["-map", "0:a:0", "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]));
        args.push(wav.clone().into());
        proxy::run_ffmpeg(env, &args, m.probe.duration_ms, &|f| progress(f * AUDIO_PART), cancelled)?;
        run_whisper(cli, model, vad, &wav, &out_base, language, env.job_object, &|f| progress(AUDIO_PART + f * (1.0 - AUDIO_PART)), cancelled)?;
        let raw = std::fs::read(&json_path).map_err(|e| RunError::Failed(format!("whisper.cpp wrote no output: {e}")))?;
        // whisper.cpp may cut a multi-byte character at a token boundary: decode lossily.
        let v: Value = serde_json::from_str(&String::from_utf8_lossy(&raw))
            .map_err(|e| RunError::Failed(format!("Invalid whisper.cpp output: {e}")))?;
        parse_output(&v).map_err(RunError::Failed)
    })();
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&json_path);
    result
}

/// Job `media.transcribe`, params `{"ids": [..], "model": "whisper-base"|"whisper-small", "language": "auto"|"vi"|"en"}`.
pub fn transcribe_job(job: &Job, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool, env: &MediaEnv, data_root: &Path) -> Outcome {
    let Some(ids) = job.params["ids"].as_array() else {
        return Outcome::Failed("media.transcribe needs params.ids".into());
    };
    let model_id = job.params["model"].as_str().unwrap_or("whisper-base");
    let language = job.params["language"].as_str().unwrap_or("auto");
    if !["auto", "vi", "en"].contains(&language) {
        return Outcome::Failed(format!("Unsupported language: {language}"));
    }
    let cli = whisper_cli(data_root);
    if !cli.exists() {
        return Outcome::Failed(format!("whisper.cpp runtime not found at {}", cli.display()));
    }
    let model = match env.with_db(|c| model_file(c, model_id)) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(e),
    };
    // Optional: used whenever installed.
    let vad = env.with_db(|c| model_file(c, VAD_ID)).ok();
    const STEP: u64 = 1000;
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
        let r = if m.probe.audio_streams == 0 {
            Ok((None, vec![]))
        } else {
            transcribe_one(&m, env, &cli, &model, vad.as_deref(), language, &|f| progress(base + (f * STEP as f64) as u64, total), cancelled)
        };
        match r {
            Ok((lang, segs)) => match env.with_db(|c| store(c, id, model_id, lang.as_deref(), &segs)) {
                Ok(()) => {
                    tracing::info!(media_id = id, model = model_id, language = ?lang, segments = segs.len(), "media transcribed");
                    done.push(json!({"id": id, "language": lang, "segments": segs.len()}));
                }
                Err(e) => failed.push(json!({"id": id, "error": e})),
            },
            Err(RunError::Cancelled) => return Outcome::Cancelled,
            Err(RunError::Failed(e)) => {
                tracing::warn!(media_id = id, error = %e, "transcription failed");
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

    #[test]
    fn parse_store_replace_and_cascade() {
        let out = json!({"result": {"language": "vi"}, "transcription": [
            {"offsets": {"from": 0, "to": 2000}, "text": " Xin chào"},
            {"offsets": {"from": 2000, "to": 2500}, "text": "   "},
            {"offsets": {"from": 2500, "to": 4000}, "text": " hello "}
        ]});
        let (lang, segs) = parse_output(&out).unwrap();
        assert_eq!(lang.as_deref(), Some("vi"));
        assert_eq!(segs, vec![
            Segment { start_ms: 0, end_ms: 2000, text: "Xin chào".into() },
            Segment { start_ms: 2500, end_ms: 4000, text: "hello".into() },
        ]);
        assert!(parse_output(&json!({})).is_err());

        let c = open_in_memory();
        let probe = Probe {
            duration_ms: 4000, width: 960, height: 540, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
            audio_codec: Some("aac".into()), audio_streams: 1, bitrate: None, source_created_at: None,
        };
        let m = upsert(&c, r"E:\v.mp4", 1, None, &probe, "{}").unwrap();
        assert!(get(&c, m.id).unwrap().is_none());
        store(&c, m.id, "whisper-base", lang.as_deref(), &segs).unwrap();
        store(&c, m.id, "whisper-small", Some("en"), &segs[..1]).unwrap();
        let t = get(&c, m.id).unwrap().unwrap();
        assert_eq!((t.model_id.as_str(), t.language.as_deref(), t.segments.len()), ("whisper-small", Some("en"), 1));
        assert!(transcribed(&c).unwrap().contains(&m.id));
        media::remove(&c, m.id).unwrap();
        assert!(get(&c, m.id).unwrap().is_none(), "cascades with the media row");
    }

    #[test]
    fn hallucination_loops_are_dropped() {
        let seg = |t: &str, i: i64| Segment { start_ms: i * 1000, end_ms: i * 1000 + 900, text: t.into() };
        let mut input: Vec<Segment> = (0..14).map(|i| seg("Cảm ơn các bạn.", i)).collect();
        input.extend([seg("Chia!", 14), seg("Chia!", 15), seg("Chia!", 16), seg("Rồi!", 17), seg("Cảm ơn các bạn.", 18)]);
        let texts: Vec<String> = drop_loops(input).into_iter().map(|s| s.text).collect();
        assert_eq!(texts, ["Chia!", "Chia!", "Chia!", "Rồi!", "Cảm ơn các bạn."], "runs of 3 stay, runs of 4+ go");
    }

    #[test]
    fn missing_model_is_reported() {
        let c = open_in_memory();
        assert_eq!(model_file(&c, "whisper-base").unwrap_err(), "Install Whisper base (multilingual) in AI models first.");
    }

    /// Full path with the real runtime: synthetic spoken-free tone clip → FFmpeg audio → whisper.cpp → stored.
    /// Skipped unless FFmpeg, `<E:\Memora Data>\runtime\whisper.cpp` and a staged/installed ggml-base.bin exist.
    #[test]
    fn transcribe_job_with_real_whisper() {
        let root = PathBuf::from(r"E:\Memora Data");
        let model = [root.join(r"models\speech\whisper-base\1\ggml-base.bin"), root.join(r"models\.downloads\whisper-base-1\ggml-base.bin")]
            .into_iter()
            .find(|p| p.exists());
        let (ffprobe, ffmpeg) = (media::tool_path(None, "ffprobe.exe"), media::tool_path(None, "ffmpeg.exe"));
        let (Some(model), true, true, true) = (model, whisper_cli(&root).exists(), ffprobe.exists(), ffmpeg.exists()) else {
            eprintln!("skipped: whisper.cpp runtime, model or FFmpeg not found");
            return;
        };
        let dir = std::env::temp_dir().join(format!("memora-whisper-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("tone.mp4");
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=s=320x240:d=3", "-f", "lavfi", "-i", "sine=f=440:d=3"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
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

        // Pretend the Model Manager installed the model.
        env.with_db(|c| {
            c.execute("INSERT INTO models (id, version, state, path) VALUES ('whisper-base', '1', 'ready', ?1)",
                [model.parent().unwrap().to_string_lossy()]).map_err(err)
        }).unwrap();
        // Exercise the VAD path too when the verified Silero file is on this machine.
        let vad_dirs = [root.join(r"models\speech\silero-vad\1"), root.join(r"models\.downloads\silero-vad-1")];
        if let Some(d) = vad_dirs.iter().find(|d| d.join("ggml-silero-v6.2.0.bin").exists()) {
            env.with_db(|c| {
                c.execute("INSERT INTO models (id, version, state, path) VALUES ('silero-vad', '1', 'ready', ?1)", [d.to_string_lossy()])
                    .map_err(err)
            }).unwrap();
        }
        let job = Job {
            id: 3, kind: "media.transcribe".into(), params: json!({"ids": [m.id], "language": "en"}),
            status: crate::jobs::Status::Running, done: 0, total: 0, result: None, error: None,
        };
        let last = std::sync::Mutex::new((0, 0));
        let Outcome::Completed(r) = transcribe_job(&job, &|d, t| *last.lock().unwrap() = (d, t), &|| false, &env, &root) else {
            panic!("transcription job failed")
        };
        assert_eq!(r["failed"].as_array().unwrap().len(), 0, "{r}");
        let t = env.with_db(|c| get(c, m.id)).unwrap().expect("stored");
        assert_eq!(t.language.as_deref(), Some("en"));
        assert!(t.segments.iter().all(|s| s.end_ms <= 3500), "times within the clip: {:?}", t.segments);
        assert_eq!(*last.lock().unwrap(), (1000, 1000));
        assert!(!dir.join(r"cache\audio").join(format!("{}.wav", m.id)).exists(), "temporary audio removed");

        // Cancellation before start leaves nothing stored.
        env.with_db(|c| clear(c, m.id)).unwrap();
        assert!(matches!(transcribe_job(&job, &|_, _| {}, &|| true, &env, &root), Outcome::Cancelled));
        assert!(env.with_db(|c| get(c, m.id)).unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
