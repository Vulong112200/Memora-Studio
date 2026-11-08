//! Export: renders a project timeline from the ORIGINAL files (never proxies) with FFmpeg,
//! then validates the output (docs/Production.md §8.2, §8.3).

use crate::jobs::{Job, Outcome};
use crate::media::{self, Media, MediaEnv};
use crate::proxy::{Decode, RunError, fit, os, run_ffmpeg, write_atomically};
use crate::timeline;
use serde::Serialize;
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Preset {
    Story,
    Reels,
    Landscape,
    Archive,
}

impl Preset {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "story" => Ok(Self::Story),
            "reels" => Ok(Self::Reels),
            "landscape" => Ok(Self::Landscape),
            "archive" => Ok(Self::Archive),
            _ => Err(format!("Unknown export preset: {s}")),
        }
    }

    /// Output (width, height, fps). Archive keeps the first clip's upright size and frame rate.
    // ponytail: Archive follows the first clip only; mixed-resolution timelines get letterboxed to it.
    pub fn target(self, first: &Media) -> (i64, i64, f64) {
        match self {
            Self::Story | Self::Reels => (1080, 1920, 30.0),
            Self::Landscape => (1920, 1080, 30.0),
            Self::Archive => {
                let (w, h) = fit(first.probe.width, first.probe.height, first.probe.rotation, i64::MAX);
                (w, h, first.probe.fps.unwrap_or(30.0).min(60.0))
            }
        }
    }
}

pub struct Segment {
    pub media: Media,
    pub in_ms: i64,
    pub out_ms: i64,
}

/// How clips are framed and joined (editing styles, docs/Production.md §6.3).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Look {
    /// true: scale to cover the frame and centre-crop (vertical presets); false: letterbox, nothing cropped.
    pub fill: bool,
    /// Crossfade length between clips; 0 = hard cuts.
    pub transition_ms: i64,
}

impl Look {
    /// Crossfade actually used: never longer than half of the shortest clip.
    // ponytail: centre crop only; subject-aware (smart) crop needs detections we do not have yet.
    pub fn transition(&self, segs: &[Segment]) -> i64 {
        let shortest = segs.iter().map(|s| s.out_ms - s.in_ms).min().unwrap_or(0);
        if segs.len() < 2 { 0 } else { self.transition_ms.clamp(0, shortest / 2) }
    }

    /// Output duration: clip lengths minus the crossfade overlaps.
    pub fn duration(&self, segs: &[Segment]) -> i64 {
        segs.iter().map(|s| s.out_ms - s.in_ms).sum::<i64>() - (segs.len() as i64 - 1).max(0) * self.transition(segs)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Encoder {
    Qsv,
    X264,
}

fn secs(ms: i64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// FFmpeg arguments (without the output path) that trim, normalize and join `segs`.
/// Every segment is letterboxed (or centre-cropped with `look.fill`) and gets a stereo 48 kHz audio
/// track (silence when the source has none) so joins always line up. Joins are hard cuts (concat) or
/// crossfades (xfade + acrossfade) per `look`.
pub fn build_args(segs: &[Segment], decode: Decode, encoder: Encoder, (w, h, fps): (i64, i64, f64), look: Look) -> Vec<OsString> {
    let mut args = vec![];
    let mut graph = String::new();
    let mut input = 0;
    let pix = if encoder == Encoder::Qsv { "nv12" } else { "yuv420p" };
    let frame = if look.fill {
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
    } else {
        format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2")
    };
    for (i, s) in segs.iter().enumerate() {
        let dur = secs(s.out_ms - s.in_ms);
        args.extend(decode.input_args());
        args.extend(os(&["-ss", &secs(s.in_ms), "-t", &dur, "-i"]));
        args.push(s.media.path.clone().into());
        let v = input;
        input += 1;
        let a = if s.media.probe.audio_streams > 0 {
            format!("{v}:a:0")
        } else {
            args.extend(os(&["-f", "lavfi", "-t", &dur, "-i", "anullsrc=r=48000:cl=stereo"]));
            input += 1;
            format!("{}:a:0", input - 1)
        };
        graph.push_str(&format!(
            "[{v}:v:0]{up}{frame},setsar=1,fps={fps},format={pix}[v{i}];\
             [{a}]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}];",
            up = decode.upright(s.media.probe.rotation),
        ));
    }
    let d = look.transition(segs);
    if d == 0 {
        for i in 0..segs.len() {
            graph.push_str(&format!("[v{i}][a{i}]"));
        }
        graph.push_str(&format!("concat=n={}:v=1:a=1[v][a]", segs.len()));
    } else {
        // Chain: each xfade starts d before the end of everything joined so far.
        let (mut v, mut a, mut offset) = ("v0".to_string(), "a0".to_string(), 0);
        for i in 1..segs.len() {
            offset += segs[i - 1].out_ms - segs[i - 1].in_ms - d;
            let last = i == segs.len() - 1;
            let (nv, na) = if last { ("v".to_string(), "a".to_string()) } else { (format!("x{i}"), format!("y{i}")) };
            graph.push_str(&format!(
                "[{v}][v{i}]xfade=transition=fade:duration={dur}:offset={off}[{nv}];[{a}][a{i}]acrossfade=d={dur}[{na}];",
                dur = secs(d),
                off = secs(offset),
            ));
            (v, a) = (nv, na);
        }
        graph.pop(); // trailing ';'
    }
    args.extend(os(&["-filter_complex", &graph, "-map", "[v]", "-map", "[a]"]));
    args.extend(os(match encoder {
        Encoder::Qsv => &["-c:v", "h264_qsv", "-preset", "medium", "-global_quality", "22"][..],
        Encoder::X264 => &["-c:v", "libx264", "-preset", "medium", "-crf", "20"][..],
    }));
    args.extend(os(&["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-f", "mp4"]));
    args
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Validation {
    pub ok: bool,
    pub checks: Vec<Check>,
    /// Not failures: dark scenes or loud sources can be intentional, but the user should know.
    pub warnings: Vec<String>,
}

/// §8.3: exists, decodes, duration, resolution, audio present, black frames, audio clipping.
pub fn validate(env: &MediaEnv, out: &Path, expected_ms: i64, segments: usize, (w, h, _): (i64, i64, f64)) -> Validation {
    let mut checks = vec![];
    let mut warnings = vec![];
    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    checks.push(Check { name: "file exists", ok: size > 0, detail: format!("{size} bytes") });

    let probe = media::run_ffprobe(&env.ffprobe, out, env.job_object)
        .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
        .and_then(|v| media::parse_probe(&v));
    match &probe {
        Ok(p) => {
            // Each cut can shift by up to one frame; allow 100 ms per segment plus 200 ms.
            let tolerance = 200 + 100 * segments as i64;
            checks.push(Check {
                name: "duration",
                ok: (p.duration_ms - expected_ms).abs() <= tolerance,
                detail: format!("{} ms, expected {expected_ms} ± {tolerance} ms", p.duration_ms),
            });
            checks.push(Check {
                name: "resolution",
                ok: (p.width, p.height) == (w, h) && p.rotation == 0,
                detail: format!("{}×{} (rotation {}), expected {w}×{h}", p.width, p.height, p.rotation),
            });
            checks.push(Check { name: "audio present", ok: p.audio_streams > 0, detail: format!("{} audio stream(s)", p.audio_streams) });
        }
        Err(e) => checks.push(Check { name: "readable", ok: false, detail: e.clone() }),
    }

    // Full decode with black-frame and loudness analysis; -xerror makes any decode error fatal.
    let mut a = os(&["-v", "info", "-xerror", "-i"]);
    a.push(out.into());
    a.extend(os(&["-vf", "blackdetect=d=1:pix_th=0.10", "-af", "volumedetect", "-f", "null", "-"]));
    match run_ffmpeg(env, &a, 0, &|_| {}, &|| false) {
        Ok(log) => {
            checks.push(Check { name: "decodes fully", ok: true, detail: "no decode errors".into() });
            for line in log.lines().filter(|l| l.contains("black_start:")) {
                let field = |k: &str| line.split_whitespace().find_map(|t| t.strip_prefix(k)).unwrap_or("?").to_string();
                warnings.push(format!("Black video from {} s to {} s", field("black_start:"), field("black_end:")));
            }
            let db = |k: &str| {
                log.lines().find_map(|l| l.split(k).nth(1)?.trim().trim_end_matches("dB").trim().parse::<f64>().ok())
            };
            if db("max_volume:").is_some_and(|v| v >= -0.1) {
                warnings.push("Audio peaks at 0 dB: possible clipping.".into());
            }
            if db("mean_volume:").is_some_and(|v| v <= -80.0) {
                warnings.push("Audio is silent.".into());
            }
        }
        Err(RunError::Failed(e)) => checks.push(Check { name: "decodes fully", ok: false, detail: e }),
        Err(RunError::Cancelled) => {}
    }
    Validation { ok: checks.iter().all(|c| c.ok), checks, warnings }
}

/// `<data root>\projects\<id>\exports\<name>-<preset>-<unix time>.mp4`
fn output_path(projects: &Path, project_id: i64, name: &str, preset: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    projects.join(project_id.to_string()).join("exports").join(format!("{}-{preset}-{now}.mp4", safe.trim()))
}

/// Renders with the hybrid QSV path, falling back to full software (logged).
pub fn render(
    env: &MediaEnv,
    segs: &[Segment],
    target: (i64, i64, f64),
    look: Look,
    out: &Path,
    progress: &dyn Fn(f64),
    cancelled: &dyn Fn() -> bool,
) -> Result<&'static str, RunError> {
    let total_ms = look.duration(segs);
    let mut last = String::new();
    for (name, decode, encoder) in [("hybrid-qsv", Decode::Hybrid, Encoder::Qsv), ("libx264", Decode::Software, Encoder::X264)] {
        let result = write_atomically(out, |part| {
            let mut args = build_args(segs, decode, encoder, target, look);
            args.push(part.into());
            run_ffmpeg(env, &args, total_ms, progress, cancelled).map(drop)
        });
        match result {
            Ok(()) => return Ok(name),
            Err(RunError::Cancelled) => return Err(RunError::Cancelled),
            Err(RunError::Failed(e)) => {
                tracing::warn!(path = name, error = %e, "export attempt failed, trying next");
                last = e;
            }
        }
    }
    Err(RunError::Failed(last))
}

/// Job `media.export`, params `{"projectId": n, "preset": "landscape", "fit": "fit"|"fill", "transitionMs": 0}`.
pub fn export_job(job: &Job, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool, env: &MediaEnv, projects: &Path) -> Outcome {
    let (Some(project_id), Some(preset_name)) = (job.params["projectId"].as_i64(), job.params["preset"].as_str()) else {
        return Outcome::Failed("media.export needs params.projectId and params.preset".into());
    };
    let preset = match Preset::parse(preset_name) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(e),
    };
    let loaded = env.with_db(|c| {
        let name: String = c
            .query_row("SELECT name FROM projects WHERE id = ?1", [project_id], |r| r.get(0))
            .map_err(|_| format!("Project {project_id} not found."))?;
        let segs = timeline::list(c, project_id)?
            .into_iter()
            .map(|clip| {
                let media = media::get(c, clip.media_id)?.ok_or("Clip refers to missing media.")?;
                Ok(Segment { media, in_ms: clip.in_ms, out_ms: clip.out_ms })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((name, segs))
    });
    let (name, segs) = match loaded {
        Ok(x) => x,
        Err(e) => return Outcome::Failed(e),
    };
    let Some(first) = segs.first() else {
        return Outcome::Failed("The project timeline is empty. Add clips first.".into());
    };
    if let Some(s) = segs.iter().find(|s| !Path::new(&s.media.path).exists()) {
        return Outcome::Failed(format!("Original file is missing: {}", s.media.path));
    }
    let target = preset.target(&first.media);
    let look = Look {
        fill: job.params["fit"].as_str() == Some("fill"),
        transition_ms: job.params["transitionMs"].as_i64().unwrap_or(0),
    };
    let out = output_path(projects, project_id, &name, preset_name);
    let expected_ms = look.duration(&segs);

    // Rendering is ~90 % of the job, validation (a full decode) the rest.
    const TOTAL: u64 = 1000;
    progress(0, TOTAL);
    let started = Instant::now();
    let encoder = match render(env, &segs, target, look, &out, &|f| progress((f * 900.0) as u64, TOTAL), cancelled) {
        Ok(e) => e,
        Err(RunError::Cancelled) => return Outcome::Cancelled,
        Err(RunError::Failed(e)) => return Outcome::Failed(format!("Render failed: {e}")),
    };
    let render_seconds = started.elapsed().as_secs_f64();
    progress(900, TOTAL);
    let validation = validate(env, &out, expected_ms, segs.len(), target);
    progress(TOTAL, TOTAL);
    tracing::info!(project_id, preset = preset_name, encoder, render_seconds, ok = validation.ok, path = %out.display(), "export finished");
    Outcome::Completed(json!({
        "path": out,
        "preset": preset_name,
        "encoder": encoder,
        "renderSeconds": (render_seconds * 10.0).round() / 10.0,
        "durationMs": expected_ms,
        "transitionMs": look.transition(&segs),
        "fill": look.fill,
        "width": target.0,
        "height": target.1,
        "validation": validation,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::process::JobObject;
    use std::process::Command;
    use std::sync::Mutex;

    fn tools() -> Option<(PathBuf, PathBuf)> {
        let ffprobe = media::tool_path(None, "ffprobe.exe");
        let ffmpeg = media::tool_path(None, "ffmpeg.exe");
        (ffprobe.exists() && ffmpeg.exists()).then_some((ffprobe, ffmpeg))
    }

    fn ffmpeg(bin: &Path, args: &[&str], out: &Path) {
        let ok = Command::new(bin).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(ok, "ffmpeg failed for {}", out.display());
    }

    fn import(env: &MediaEnv, path: &Path) -> Media {
        let raw = media::run_ffprobe(&env.ffprobe, path, env.job_object).unwrap();
        let p = media::parse_probe(&serde_json::from_str(&raw).unwrap()).unwrap();
        env.with_db(|c| media::upsert(c, &path.to_string_lossy(), 1, None, &p, &raw)).unwrap()
    }

    /// One small grey frame as raw bytes, for comparing renders.
    fn frame(bin: &Path, file: &Path) -> Vec<u8> {
        let out = Command::new(bin)
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(["-frames:v", "1", "-vf", "scale=9:16,format=gray", "-f", "rawvideo", "-"])
            .output()
            .unwrap();
        out.stdout
    }

    #[test]
    fn output_name_is_safe() {
        let p = output_path(Path::new(r"E:\d\projects"), 3, "Đà Lạt: 2026/07?", "story");
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("Đà Lạt_ 2026_07_-story-") && name.ends_with(".mp4"), "{name}");
    }

    /// Real FFmpeg: two sources (one rotated, one without audio), trimmed + concatenated, validated.
    /// Also checks that the hybrid (QSV) path rotates exactly like software autorotate.
    #[test]
    fn export_and_validate_with_real_ffmpeg() {
        let Some((ffprobe, bin)) = tools() else {
            eprintln!("skipped: FFmpeg not found");
            return;
        };
        let dir = std::env::temp_dir().join(format!("memora-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.mp4");
        ffmpeg(&bin, &["-f", "lavfi", "-i", "testsrc2=s=640x360:d=3:r=30", "-f", "lavfi", "-i", "sine=d=3", "-c:v", "libx264", "-c:a", "aac", "-shortest"], &a);
        let land = dir.join("land.mp4");
        ffmpeg(&bin, &["-f", "lavfi", "-i", "testsrc2=s=640x360:d=3:r=30", "-c:v", "libx264"], &land);
        let rot = dir.join("rot.mp4");
        ffmpeg(&bin, &["-display_rotation", "90", "-i", land.to_str().unwrap(), "-c", "copy"], &rot);

        let db = Mutex::new(Ok(open_in_memory()));
        let job_object = JobObject::new().unwrap();
        let env = MediaEnv { ffprobe, ffmpeg: bin.clone(), cache: dir.join("cache"), job_object: &job_object, db: &db };
        let (ma, mr) = (import(&env, &a), import(&env, &rot));
        assert_eq!(mr.probe.rotation, 90);

        // Rotation: hybrid vs software must give the same upright picture.
        let target = (180, 320, 30.0);
        let one = |m: &Media| vec![Segment { media: m.clone(), in_ms: 0, out_ms: 1000 }];
        let (sw, hy) = (dir.join("sw.mp4"), dir.join("hy.mp4"));
        let mut args = build_args(&one(&mr), Decode::Software, Encoder::X264, target, Look::default());
        args.push(sw.clone().into());
        assert!(run_ffmpeg(&env, &args, 1000, &|_| {}, &|| false).is_ok());
        let mut args = build_args(&one(&mr), Decode::Hybrid, Encoder::Qsv, target, Look::default());
        args.push(hy.clone().into());
        if run_ffmpeg(&env, &args, 1000, &|_| {}, &|| false).is_ok() {
            let (fs, fh) = (frame(&bin, &sw), frame(&bin, &hy));
            assert_eq!(fs.len(), 9 * 16);
            let max_diff = fs.iter().zip(&fh).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            assert!(max_diff < 40, "hybrid rotation differs from software autorotate (max diff {max_diff})");
        } else {
            eprintln!("hybrid QSV path unavailable on this machine; rotation comparison skipped");
        }

        // Trim + concat + validate.
        let segs = vec![
            Segment { media: ma.clone(), in_ms: 500, out_ms: 1500 },
            Segment { media: mr.clone(), in_ms: 1000, out_ms: 2500 },
        ];
        let target = Preset::Landscape.target(&ma);
        let out = dir.join("out.mp4");
        let last = Mutex::new(0.0);
        let Ok(encoder) = render(&env, &segs, target, Look::default(), &out, &|f| *last.lock().unwrap() = f, &|| false) else {
            panic!("render failed");
        };
        assert!(*last.lock().unwrap() > 0.9, "progress reported");
        let v = validate(&env, &out, 2500, segs.len(), target);
        assert!(v.ok, "validation failed ({encoder}): {v:?}");
        // Second segment has no audio → silence is inserted, which is not a failure.
        assert!(a.exists() && rot.exists(), "originals untouched");

        // Crossfades + centre crop (vertical output from landscape sources): 1 s + 1.5 s − 0.4 s overlap.
        let look = Look { fill: true, transition_ms: 400 };
        assert_eq!((look.transition(&segs), look.duration(&segs)), (400, 2100));
        assert_eq!(Look { fill: false, transition_ms: 5000 }.transition(&segs), 500, "clamped to half the shortest clip");
        let story = Preset::Story.target(&ma);
        let (xf, fit) = (dir.join("xfade.mp4"), dir.join("fit.mp4"));
        assert!(render(&env, &segs, story, look, &xf, &|_| {}, &|| false).is_ok());
        let v = validate(&env, &xf, 2100, segs.len(), story);
        assert!(v.ok, "crossfade export failed validation: {v:?}");
        assert!(render(&env, &segs[..1], story, Look::default(), &fit, &|_| {}, &|| false).is_ok());
        // Top row of a 9×16 grey thumbnail: letterbox = black bar, fill = picture.
        let top = |f: &Path| frame(&bin, f)[..9].iter().map(|&x| x as u32).sum::<u32>() / 9;
        assert!(top(&fit) < 20 && top(&xf) > 40, "fill crops instead of letterboxing (fit {}, fill {})", top(&fit), top(&xf));

        // Validation catches a wrong expectation.
        assert!(!validate(&env, &out, 9000, 2, target).ok);
        assert!(!validate(&env, &dir.join("missing.mp4"), 2500, 2, target).ok);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
