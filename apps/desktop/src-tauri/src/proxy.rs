//! Thumbnails and preview proxies under `<data root>\cache` (docs/Production.md §8.1).
//! Proxies are for preview only; final renders always read the original files.

use crate::jobs::{Job, Outcome};
use crate::media::{self, Media, MediaEnv};
use crate::process::command;
use serde_json::json;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

/// Proxy short side in pixels (960×540 for 16:9 landscape).
const PROXY_SHORT_SIDE: i64 = 540;
const THUMB_SHORT_SIDE: i64 = 180;

pub fn thumb_path(cache: &Path, id: i64) -> PathBuf {
    cache.join("thumbs").join(format!("{id}.jpg"))
}

pub fn proxy_path(cache: &Path, id: i64) -> PathBuf {
    cache.join("proxy").join(format!("{id}.mp4"))
}

pub fn remove_cache(cache: &Path, id: i64) {
    let _ = std::fs::remove_file(thumb_path(cache, id));
    let _ = std::fs::remove_file(proxy_path(cache, id));
    let _ = std::fs::remove_dir_all(cache.join("frames").join(id.to_string()));
}

/// Temporary files of jobs the app did not finish (crash or forced close); call only while no job runs.
/// Touches app-made temporaries only: titles, whisper audio and library music in the cache, and
/// `.part` / `.level.mp4` files next to proxies, thumbnails and exports. Finished exports, proxies and
/// the user's own files are never removed. Partial model downloads (resumable) are not here.
pub fn remove_leftovers(root: &Path) -> usize {
    let cache = root.join("cache");
    let mut removed = 0;
    let mut sweep = |dir: PathBuf, everything: bool| {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let temp = everything || name.ends_with(".part") || name.ends_with(".level.mp4") || name.ends_with(".unvalidated.mp4");
            if temp && e.path().is_file() && std::fs::remove_file(e.path()).is_ok() {
                removed += 1;
            }
        }
    };
    sweep(cache.join("render"), true);
    sweep(cache.join("audio"), true);
    sweep(cache.join("downloads").join("sounds"), true);
    sweep(cache.join("thumbs"), false);
    sweep(cache.join("proxy"), false);
    for project in std::fs::read_dir(root.join("projects")).into_iter().flatten().flatten() {
        sweep(project.path().join("exports"), false);
    }
    removed
}

/// Creates the proxy unless it already exists.
pub fn ensure_proxy(m: &Media, env: &MediaEnv, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<(), RunError> {
    if proxy_path(&env.cache, m.id).exists() {
        return Ok(());
    }
    let encoder = make_proxy(m, env, progress, cancelled)?;
    tracing::info!(media_id = m.id, encoder, "proxy created");
    Ok(())
}

/// Fills `thumb`/`proxy` with the cached files that exist.
pub fn attach_cache(cache: &Path, items: &mut [Media]) {
    for m in items {
        m.thumb = Some(thumb_path(cache, m.id)).filter(|p| p.exists());
        m.proxy = Some(proxy_path(cache, m.id)).filter(|p| p.exists());
    }
}

/// Display size (rotation applied) scaled so the short side is at most `short`; never upscales; even numbers.
pub fn fit(width: i64, height: i64, rotation: i64, short: i64) -> (i64, i64) {
    let (w, h) = if rotation % 180 == 0 { (width, height) } else { (height, width) };
    let scale = (short as f64 / w.min(h) as f64).min(1.0);
    let even = |x: i64| ((x as f64 * scale / 2.0).round() as i64 * 2).max(2);
    (even(w), even(h))
}

pub enum RunError {
    Cancelled,
    Failed(String),
}

pub fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// How a source is decoded. `Hybrid` = QSV decode, frames downloaded for CPU filters; FFmpeg's
/// autorotate cannot run on GPU surfaces, so rotation is applied here (verified against software
/// autorotate on a synthetic 90° clip) and the display matrix is dropped so players do not rotate twice.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Decode {
    Hybrid,
    Software,
}

impl Decode {
    /// Input options placed before `-i`.
    pub fn input_args(self) -> Vec<OsString> {
        match self {
            Decode::Hybrid => os(&["-hwaccel", "qsv", "-hwaccel_output_format", "qsv", "-noautorotate"]),
            Decode::Software => vec![],
        }
    }

    /// Filters that turn decoded frames into upright CPU frames (ends with a trailing comma, or empty).
    pub fn upright(self, rotation: i64) -> String {
        match self {
            Decode::Software => String::new(), // autorotate already applied
            Decode::Hybrid => {
                let rotate = match rotation {
                    90 => "transpose=2,",
                    180 => "hflip,vflip,",
                    270 => "transpose=1,",
                    _ => "",
                };
                format!("vpp_qsv=format=nv12,hwdownload,format=nv12,{rotate}sidedata=mode=delete:type=DISPLAYMATRIX,")
            }
        }
    }
}

/// Runs FFmpeg with `-progress`, reporting the fraction done of `duration_ms`. Killed on cancel.
/// Returns FFmpeg's stderr (log output) on success.
pub fn run_ffmpeg(
    env: &MediaEnv,
    args: &[OsString],
    duration_ms: i64,
    progress: &dyn Fn(f64),
    cancelled: &dyn Fn() -> bool,
) -> Result<String, RunError> {
    let mut cmd = command(&env.ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-progress", "pipe:1", "-nostats", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::spawn(&mut cmd, env.job_object)
        .map_err(|e| RunError::Failed(format!("Cannot start FFmpeg ({}): {e}", env.ffmpeg.display())))?;
    let mut stderr = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError::Cancelled);
        }
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<i64>().ok())
            && duration_ms > 0
        {
            progress((us as f64 / 1000.0 / duration_ms as f64).clamp(0.0, 1.0));
        }
    }
    let status = child.wait().map_err(|e| RunError::Failed(format!("FFmpeg failed: {e}")))?;
    let errors = errors.join().unwrap_or_default();
    if status.success() {
        return Ok(errors);
    }
    let tail: String = errors.trim().lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
    Err(RunError::Failed(format!("FFmpeg exited with {status}: {tail}")))
}

/// Writes `out` via a `.part` file so a cancelled or failed run never leaves a broken file.
pub fn write_atomically(out: &Path, run: impl FnOnce(&Path) -> Result<(), RunError>) -> Result<(), RunError> {
    let dir = out.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| RunError::Failed(format!("Cannot create {}: {e}", dir.display())))?;
    let part = out.with_extension("part");
    let result = run(&part).and_then(|()| {
        std::fs::rename(&part, out).map_err(|e| RunError::Failed(format!("Cannot write {}: {e}", out.display())))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// One JPEG frame at 10 % of the clip (skips black first frames), rotation applied.
pub fn make_thumbnail(m: &Media, env: &MediaEnv) -> Result<(), String> {
    let (w, h) = fit(m.probe.width, m.probe.height, m.probe.rotation, THUMB_SHORT_SIDE);
    let at = format!("{:.3}", m.probe.duration_ms as f64 / 1000.0 * 0.1);
    write_atomically(&thumb_path(&env.cache, m.id), |part| {
        let mut a = os(&["-ss", &at, "-i"]);
        a.push(m.path.clone().into());
        a.extend(os(&["-frames:v", "1", "-vf", &format!("scale={w}:{h}"), "-q:v", "4", "-f", "mjpeg"]));
        a.push(part.into());
        run_ffmpeg(env, &a, 0, &|_| {}, &|| false).map(drop)
    })
    .map_err(|e| match e {
        RunError::Cancelled => "cancelled".into(),
        RunError::Failed(e) => e,
    })
}

/// Creates the preview proxy. Returns the encoder path used.
fn make_proxy(m: &Media, env: &MediaEnv, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<&'static str, RunError> {
    let (w, h) = fit(m.probe.width, m.probe.height, m.probe.rotation, PROXY_SHORT_SIDE);
    let cap_fps = m.probe.fps.is_some_and(|f| f > 30.5);
    let mut tail = os(&["-map", "0:v:0"]);
    tail.extend(os(if m.probe.audio_streams > 0 {
        &["-map", "0:a:0", "-c:a", "aac", "-b:a", "96k", "-ac", "2"][..]
    } else {
        &["-an"][..]
    }));
    tail.extend(os(&["-movflags", "+faststart", "-f", "mp4"]));

    let mut attempts: Vec<(&'static str, Vec<OsString>)> = vec![];
    // Full GPU pipeline (QSV decode + scale + encode): measured ~5x faster than software on 10-bit HEVC 2.7K.
    // vpp_qsv ignores the display matrix, so rotated sources use the hybrid path instead.
    if m.probe.rotation == 0 {
        let mut a = os(&["-hwaccel", "qsv", "-hwaccel_output_format", "qsv", "-i"]);
        a.push(m.path.clone().into());
        let fr = if cap_fps { ":framerate=30" } else { "" };
        a.extend(os(&["-vf", &format!("vpp_qsv=w={w}:h={h}{fr}:format=nv12")]));
        a.extend(os(&["-c:v", "h264_qsv", "-preset", "veryfast", "-global_quality", "28"]));
        attempts.push(("qsv", a));
    }
    for (name, decode, encoder) in [
        ("hybrid", Decode::Hybrid, &["-c:v", "h264_qsv", "-preset", "veryfast", "-global_quality", "28"][..]),
        ("libx264", Decode::Software, &["-c:v", "libx264", "-preset", "veryfast", "-crf", "26"][..]),
    ] {
        let mut a = decode.input_args();
        a.extend(os(&["-i"]));
        a.push(m.path.clone().into());
        let fps = if cap_fps { "fps=30," } else { "" };
        let format = if decode == Decode::Hybrid { "nv12" } else { "yuv420p" };
        a.extend(os(&["-vf", &format!("{}{fps}scale={w}:{h},format={format}", decode.upright(m.probe.rotation))]));
        a.extend(os(encoder));
        attempts.push((name, a));
    }

    let out = proxy_path(&env.cache, m.id);
    let mut last = String::new();
    for (name, mut args) in attempts {
        let result = write_atomically(&out, |part| {
            args.extend(tail.iter().cloned());
            args.push(part.into());
            run_ffmpeg(env, &args, m.probe.duration_ms, progress, cancelled).map(drop)
        });
        match result {
            Ok(()) => return Ok(name),
            Err(RunError::Cancelled) => return Err(RunError::Cancelled),
            Err(RunError::Failed(e)) => {
                tracing::warn!(media_id = m.id, path = name, error = %e, "proxy attempt failed, trying next");
                last = e;
            }
        }
    }
    Err(RunError::Failed(last))
}

/// Job `media.proxy`, params `{"ids": [..]}`. Existing proxies are kept; per-file failures are reported.
pub fn proxy_job(job: &Job, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool, env: &MediaEnv) -> Outcome {
    let Some(ids) = job.params["ids"].as_array() else {
        return Outcome::Failed("media.proxy needs params.ids".into());
    };
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
        if !thumb_path(&env.cache, id).exists()
            && let Err(e) = make_thumbnail(&m, env)
        {
            tracing::warn!(media_id = id, error = %e, "thumbnail failed");
        }
        if !proxy_path(&env.cache, id).exists() {
            let started = Instant::now();
            let step = |f: f64| progress(base + (f * STEP as f64) as u64, total);
            match make_proxy(&m, env, &step, cancelled) {
                Ok(encoder) => {
                    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
                    tracing::info!(media_id = id, encoder, seconds, "proxy created");
                    done.push(json!({"id": id, "encoder": encoder, "seconds": seconds}));
                }
                Err(RunError::Cancelled) => return Outcome::Cancelled,
                Err(RunError::Failed(e)) => {
                    tracing::warn!(media_id = id, error = %e, "proxy failed");
                    failed.push(json!({"id": id, "error": e}));
                }
            }
        }
        progress(base + STEP, total);
    }
    Outcome::Completed(json!({"done": done, "failed": failed}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect_rotation_and_never_upscales() {
        assert_eq!(fit(2688, 1512, 0, 540), (960, 540));
        assert_eq!(fit(1920, 1080, 270, 540), (540, 960), "portrait phone clip");
        assert_eq!(fit(320, 240, 0, 540), (320, 240), "no upscaling");
        assert_eq!(fit(3840, 2160, 0, 180), (320, 180));
        assert_eq!(fit(1440, 1080, 0, 540), (720, 540));
    }

    #[test]
    fn leftovers_are_removed_but_finished_files_kept() {
        let root = std::env::temp_dir().join(format!("memora-leftovers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let files = [
            ("cache/render/titles-1-2.ass", false),
            ("cache/audio/7.wav", false),
            ("cache/downloads/sounds/track-1-2.mp3", false),
            ("cache/proxy/7.part", false),
            ("cache/proxy/8.mp4", true),
            ("cache/thumbs/8.jpg", true),
            ("cache/frames/8/0.jpg", true),
            ("projects/1/exports/Trip-reels-1.part", false),
            ("projects/1/exports/Trip-reels-1.level.mp4", false),
            ("projects/1/exports/Trip-reels-1.unvalidated.mp4", false),
            ("projects/1/exports/Trip-reels-2.mp4", true),
            ("models/.downloads/whisper-base-1/ggml-base.part", true),
        ];
        for (f, _) in files {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
        }
        assert_eq!(remove_leftovers(&root), files.iter().filter(|(_, keep)| !keep).count());
        for (f, keep) in files {
            assert_eq!(root.join(f).exists(), keep, "{f}");
        }
        assert_eq!(remove_leftovers(&root), 0);
        assert_eq!(remove_leftovers(&root.join("missing")), 0, "absent folders are fine");
        std::fs::remove_dir_all(root).unwrap();
    }
}
