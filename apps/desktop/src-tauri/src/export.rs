//! Export: renders a project timeline from the ORIGINAL files (never proxies) with FFmpeg,
//! then validates the output (docs/Production.md §8.2, §8.3).

use crate::jobs::{Job, Outcome};
use crate::media::{self, Media, MediaEnv};
use crate::proxy::{Decode, RunError, fit, os, run_ffmpeg, write_atomically};
use crate::timeline;
use serde::{Deserialize, Serialize};
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
    /// User text shown at the start of this clip (lower third).
    pub caption: Option<String>,
    /// Transcript lines of the source (original-file times), for automatic subtitles.
    pub speech: Vec<crate::transcribe::Segment>,
}

/// Background music chosen by the user (their own file; never modified).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Music {
    /// The user's own file (never modified or deleted). Empty when `library` is set.
    pub path: String,
    /// Track id from the free music library (sounds.rs): downloaded for the export, then deleted.
    pub library: Option<String>,
    /// Music level, 0..2 (1 = file level).
    pub volume: f64,
    /// Lower the music automatically while the clips have sound (speech), via sidechain compression.
    pub duck: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Titles {
    pub title: String,
    pub subtitle: String,
    pub ending: String,
    /// Burn transcript lines in as subtitles.
    pub captions: bool,
}

/// Look and sound of an export (editing styles, docs/Production.md §6.3). Every field has a safe default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// true: cover the frame and centre-crop (vertical presets); false: letterbox, nothing cropped.
    pub fill: bool,
    /// Transition length between clips; 0 = hard cuts.
    pub transition_ms: i64,
    /// FFmpeg xfade transition (one of TRANSITIONS).
    pub transition: String,
    /// Colour look (one of COLORS).
    pub color: String,
    /// Fade in from / out to black, picture and sound.
    pub fade: bool,
    /// Level of the clips' own sound, 0..2.
    pub original_volume: f64,
    pub music: Option<Music>,
    pub titles: Titles,
    /// EBU R128 loudness normalisation to −16 LUFS (typical for phones / social media).
    pub normalize: bool,
    /// Licence credit burned in at the end (set by the export job for library music).
    #[serde(skip)]
    pub credit: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            fill: false, transition_ms: 0, transition: "fade".into(), color: "none".into(), fade: false,
            original_volume: 1.0, music: None, titles: Titles::default(), normalize: false, credit: None,
        }
    }
}

/// Saved export options of a project (look, music, titles); defaults when never saved.
pub fn load_options(c: &rusqlite::Connection, project_id: i64) -> Options {
    c.query_row("SELECT export_json FROM project_settings WHERE project_id = ?1", [project_id], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

/// Checks and saves the export options of a project.
pub fn save_options(c: &rusqlite::Connection, project_id: i64, o: &Options) -> Result<(), String> {
    o.check()?;
    c.execute(
        "INSERT INTO project_settings (project_id, export_json) VALUES (?1, ?2)
         ON CONFLICT (project_id) DO UPDATE SET export_json = excluded.export_json",
        rusqlite::params![project_id, serde_json::to_string(o).unwrap()],
    )
    .map_err(crate::db::err)
    .map(drop)
}

pub const TRANSITIONS: &[&str] = &["fade", "dissolve", "fadeblack", "smoothleft", "slideleft", "wipeleft", "circleopen"];
pub const COLORS: &[&str] = &["none", "warm", "cool", "vivid", "cinematic", "bw", "vintage"];
const FADE_MS: i64 = 600;

/// FFmpeg filters for a colour look (gentle grades; nothing clips highlights hard).
fn color_filter(name: &str) -> Option<&'static str> {
    match name {
        "warm" => Some("colorbalance=rs=0.05:bs=-0.05:rm=0.04:bm=-0.04,eq=saturation=1.08"),
        "cool" => Some("colorbalance=rs=-0.04:bs=0.05:bm=0.03,eq=saturation=1.02"),
        "vivid" => Some("eq=saturation=1.28:contrast=1.05"),
        "cinematic" => Some("colorbalance=rs=-0.03:bs=0.04:rh=0.05:bh=-0.04,eq=contrast=1.06:saturation=0.92,vignette=angle=PI/5"),
        "bw" => Some("hue=s=0,eq=contrast=1.1"),
        "vintage" => Some("curves=preset=vintage,vignette=angle=PI/4"),
        _ => None,
    }
}

impl Options {
    pub fn check(&self) -> Result<(), String> {
        if !TRANSITIONS.contains(&self.transition.as_str()) {
            return Err(format!("Unknown transition: {}", self.transition));
        }
        if !COLORS.contains(&self.color.as_str()) {
            return Err(format!("Unknown colour look: {}", self.color));
        }
        if !(0.0..=2.0).contains(&self.original_volume) {
            return Err("Original sound level must be between 0 and 2.".into());
        }
        if let Some(m) = &self.music {
            match &m.library {
                Some(id) => drop(crate::sounds::track(id)?),
                None if !Path::new(&m.path).is_file() => return Err(format!("Music file not found: {}", m.path)),
                None => {}
            }
            if !(0.0..=2.0).contains(&m.volume) {
                return Err("Music level must be between 0 and 2.".into());
            }
        }
        Ok(())
    }

    /// Transition actually used: never longer than half of the shortest clip.
    // ponytail: centre crop only; subject-aware (smart) crop needs detections we do not have yet.
    pub fn transition(&self, segs: &[Segment]) -> i64 {
        let shortest = segs.iter().map(|s| s.out_ms - s.in_ms).min().unwrap_or(0);
        if segs.len() < 2 { 0 } else { self.transition_ms.clamp(0, shortest / 2) }
    }

    /// Output duration: clip lengths minus the transition overlaps.
    pub fn duration(&self, segs: &[Segment]) -> i64 {
        segs.iter().map(|s| s.out_ms - s.in_ms).sum::<i64>() - (segs.len() as i64 - 1).max(0) * self.transition(segs)
    }

    /// Output time at which each segment starts.
    fn starts(&self, segs: &[Segment]) -> Vec<i64> {
        let d = self.transition(segs);
        let mut t = 0;
        segs.iter().map(|s| { let start = t; t += s.out_ms - s.in_ms - d; start }).collect()
    }
}

fn ass_time(ms: i64) -> String {
    let cs = ms.max(0) / 10;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

/// Plain text → ASS text (no override tags from user input; line breaks kept).
fn ass_text(s: &str) -> String {
    s.trim().replace('\\', "/").replace('{', "(").replace('}', ")").replace("\r\n", "\\N").replace('\n', "\\N")
}

/// ASS script for titles, clip captions and speech subtitles, sized for (w, h) with title-safe margins
/// (vertical video keeps text above the bottom ~18 % where phone apps draw their buttons). None = no text.
pub fn subtitles(segs: &[Segment], o: &Options, (w, h): (i64, i64)) -> Option<String> {
    let total = o.duration(segs);
    let starts = o.starts(segs);
    let unit = w.min(h) as f64;
    let (side, bottom) = ((w as f64 * 0.08) as i64, if h > w { (h as f64 * 0.18) as i64 } else { (h as f64 * 0.08) as i64 });
    let mut events = vec![];
    let mut add = |style: &str, a: i64, b: i64, text: String| {
        if b > a && !text.is_empty() {
            events.push(format!("Dialogue: 0,{},{},{style},,0,0,0,,{text}", ass_time(a), ass_time(b)));
        }
    };
    let t = &o.titles;
    if !t.title.trim().is_empty() || !t.subtitle.trim().is_empty() {
        let mut text = format!("{{\\fad(500,500)}}{}", ass_text(&t.title));
        if !t.subtitle.trim().is_empty() {
            text.push_str(&format!("\\N{{\\fs{:.0}}}{}", unit * 0.045, ass_text(&t.subtitle)));
        }
        add("Title", 0, total.min(4_000), text);
    }
    for (s, &start) in segs.iter().zip(&starts) {
        if let Some(c) = s.caption.as_deref().filter(|c| !c.trim().is_empty()) {
            add("Label", start, start + (s.out_ms - s.in_ms).min(4_000), format!("{{\\fad(300,300)}}{}", ass_text(c)));
        }
        if t.captions {
            for sp in &s.speech {
                let (a, b) = (sp.start_ms.max(s.in_ms), sp.end_ms.min(s.out_ms));
                add("Caption", start + a - s.in_ms, start + b - s.in_ms, ass_text(&sp.text));
            }
        }
    }
    if !t.ending.trim().is_empty() {
        add("Title", total - total.min(3_500), total, format!("{{\\fad(500,700)}}{}", ass_text(&t.ending)));
    }
    if let Some(c) = &o.credit {
        add("Credit", total - total.min(4_000), total, format!("{{\\fad(400,400)}}{}", ass_text(c)));
    }
    if events.is_empty() {
        return None;
    }
    let style = |name: &str, size: f64, align: i32, margin_v: i64, box_: bool| {
        format!(
            "Style: {name},Segoe UI Semibold,{size:.0},&H00FFFFFF,&H00FFFFFF,&H64000000,&H8C000000,0,0,0,0,100,100,0,0,{},{},{},{align},{side},{side},{margin_v},1",
            if box_ { 3 } else { 1 },
            if box_ { (unit * 0.012) as i64 } else { (unit * 0.004).max(2.0) as i64 },
            if box_ { 0 } else { 2 },
        )
    };
    Some(format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         {}\n{}\n{}\n{}\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n{}\n",
        style("Title", unit * 0.085, 5, 0, false),
        style("Caption", unit * 0.05, 2, bottom, false),
        style("Label", unit * 0.042, 1, bottom + (unit * 0.12) as i64, true),
        style("Credit", unit * 0.024, 2, (unit * 0.03) as i64, false),
        events.join("\n"),
    ))
}

/// Target integrated loudness for `Options::normalize` (phones / social media).
pub const TARGET_LUFS: f64 = -16.0;

/// Audio-only decode of an export; assumed generous limit (a 5 min export measures in seconds).
const LUFS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Integrated loudness (LUFS) of a file's first audio stream (EBU R128).
pub fn measure_lufs(env: &MediaEnv, file: &Path) -> Result<f64, String> {
    let mut cmd = crate::process::command(&env.ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-af", "ebur128=framelog=quiet", "-f", "null", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let child = crate::process::spawn(&mut cmd, env.job_object).map_err(|e| format!("Cannot start FFmpeg: {e}"))?;
    let out = crate::process::wait_output(child, LUFS_TIMEOUT, &|| false)?;
    let log = String::from_utf8_lossy(&out.stderr);
    log.lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("I:").and_then(|v| v.trim().trim_end_matches("LUFS").trim().parse().ok()))
        .ok_or("No loudness measurement in FFmpeg output".into())
}

/// Second loudness pass: measures the render and corrects its level to `TARGET_LUFS` (±12 dB at most),
/// with a limiter at −1 dBTP. Picture is copied, only sound is re-encoded. Returns (before, gain dB).
pub fn level_loudness(env: &MediaEnv, out: &Path, cancelled: &dyn Fn() -> bool) -> Result<(f64, f64), RunError> {
    let before = measure_lufs(env, out).map_err(RunError::Failed)?;
    let gain = (TARGET_LUFS - before).clamp(-12.0, 12.0);
    if gain.abs() < 0.5 || !before.is_finite() {
        return Ok((before, 0.0));
    }
    let leveled = out.with_extension("level.mp4");
    let r = write_atomically(&leveled, |part| {
        let mut args = vec![OsString::from("-i"), out.into()];
        args.extend(os(&["-map", "0:v:0", "-map", "0:a:0", "-c:v", "copy", "-af", &format!("volume={gain:.2}dB,alimiter=limit=0.891:level=false"),
            "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-f", "mp4"]));
        args.push(part.into());
        run_ffmpeg(env, &args, 0, &|_| {}, cancelled).map(drop)
    });
    if let Err(e) = r {
        let _ = std::fs::remove_file(&leveled);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&leveled, out) {
        let _ = std::fs::remove_file(&leveled);
        return Err(RunError::Failed(format!("Cannot replace {}: {e}", out.display())));
    }
    Ok((before, gain))
}

/// Path inside a single-quoted filtergraph argument (Windows drive colon escaped).
fn filter_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").replace(':', "\\:").replace('\'', "'\\''")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Encoder {
    Qsv,
    X264,
}

fn secs(ms: i64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// FFmpeg arguments (without the output path) that trim, normalize, join and finish `segs`:
/// letterbox on a blurred copy of the clip (no black bars) or centre-crop, cuts or xfade transitions, colour look, fades, burned-in ASS text (`ass`),
/// own sound level, optional music (looped, ducked under the clips' sound) and loudness normalisation.
/// Every segment gets a stereo 48 kHz track (silence when the source has none) so joins line up.
pub fn build_args(segs: &[Segment], decode: Decode, encoder: Encoder, (w, h, fps): (i64, i64, f64), o: &Options, ass: Option<&Path>) -> Vec<OsString> {
    let mut args = vec![];
    let mut graph = String::new();
    let mut input = 0;
    let pix = if encoder == Encoder::Qsv { "nv12" } else { "yuv420p" };
    let frame = if o.fill {
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
        let up = decode.upright(s.media.probe.rotation);
        let (sw, sh) = fit(s.media.probe.width, s.media.probe.height, s.media.probe.rotation, i64::MAX);
        let mismatch = ((sw * h) as f64 / (sh * w).max(1) as f64 - 1.0).abs() > 0.05;
        if !o.fill && mismatch {
            // Different shape (e.g. phone portrait in 16:9): blurred, darkened copy behind instead of black bars.
            graph.push_str(&format!(
                "[{v}:v:0]{up}split=2[s{i}][t{i}];[s{i}]scale={bw}:{bh}:force_original_aspect_ratio=increase,crop={bw}:{bh},gblur=sigma=6,eq=brightness=-0.12:saturation=0.8,scale={w}:{h}[b{i}];[t{i}]scale={w}:{h}:force_original_aspect_ratio=decrease[f{i}];[b{i}][f{i}]overlay=(W-w)/2:(H-h)/2,setsar=1,fps={fps},format=yuv420p[v{i}];",
                bw = (w / 4) & !1,
                bh = (h / 4) & !1,
            ));
        } else {
            graph.push_str(&format!("[{v}:v:0]{up}{frame},setsar=1,fps={fps},format=yuv420p[v{i}];"));
        }
        graph.push_str(&format!("[{a}]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}];"));
    }
    let d = o.transition(segs);
    if d == 0 {
        for i in 0..segs.len() {
            graph.push_str(&format!("[v{i}][a{i}]"));
        }
        graph.push_str(&format!("concat=n={}:v=1:a=1[vj][aj];", segs.len()));
    } else {
        // Chain: each transition starts d before the end of everything joined so far.
        let (mut v, mut a, mut offset) = ("v0".to_string(), "a0".to_string(), 0);
        for i in 1..segs.len() {
            offset += segs[i - 1].out_ms - segs[i - 1].in_ms - d;
            let last = i == segs.len() - 1;
            let (nv, na) = if last { ("vj".to_string(), "aj".to_string()) } else { (format!("x{i}"), format!("y{i}")) };
            graph.push_str(&format!(
                "[{v}][v{i}]xfade=transition={t}:duration={dur}:offset={off}[{nv}];[{a}][a{i}]acrossfade=d={dur}[{na}];",
                t = o.transition,
                dur = secs(d),
                off = secs(offset),
            ));
            (v, a) = (nv, na);
        }
    }
    let total = o.duration(segs);
    let fade = FADE_MS.min(total / 4);

    // Picture: colour → fades → text → encoder format.
    let mut vf: Vec<String> = color_filter(&o.color).map(String::from).into_iter().collect();
    if o.fade && fade > 0 {
        vf.push(format!("fade=t=in:st=0:d={0},fade=t=out:st={1}:d={0}", secs(fade), secs(total - fade)));
    }
    if let Some(p) = ass {
        let fonts = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| r"C:\Windows\Fonts".into());
        vf.push(format!("ass=filename='{}':fontsdir='{}'", filter_path(p), filter_path(Path::new(&fonts))));
    }
    vf.push(format!("format={pix}"));
    graph.push_str(&format!("[vj]{}[v];", vf.join(",")));

    // Sound: own level → music (looped, trimmed, optionally ducked) → fades → loudness.
    graph.push_str(&format!("[aj]volume={:.3}[ao];", o.original_volume));
    let mut sound = "ao".to_string();
    if let Some(m) = &o.music {
        args.extend(os(&["-stream_loop", "-1", "-i"]));
        args.push(m.path.clone().into());
        graph.push_str(&format!(
            "[{input}:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={:.3},atrim=0:{},afade=t=out:st={}:d=1.5[m0];",
            m.volume,
            secs(total),
            secs((total - 1500).max(0)),
        ));
        if m.duck {
            graph.push_str("[ao]asplit=2[ao1][sc];[m0][sc]sidechaincompress=threshold=0.02:ratio=8:attack=20:release=400[m1];[ao1][m1]");
        } else {
            graph.push_str("[ao][m0]");
        }
        graph.push_str("amix=inputs=2:duration=first:dropout_transition=0:normalize=0[mix];");
        sound = "mix".into();
    }
    let mut af = vec![];
    if o.fade && fade > 0 {
        af.push(format!("afade=t=in:st=0:d={0},afade=t=out:st={1}:d={0}", secs(fade), secs(total - fade)));
    }
    if o.normalize {
        af.push("loudnorm=I=-16:TP=-1.5:LRA=11,aresample=48000".into());
    }
    if af.is_empty() {
        af.push("anull".into());
    }
    graph.push_str(&format!("[{sound}]{}[a]", af.join(",")));

    args.extend(os(&["-filter_complex", &graph, "-map", "[v]", "-map", "[a]", "-t", &secs(total)]));
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

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// `p` if no file has that name yet, else `name-2.mp4`, `name-3.mp4`, … — an earlier export is never replaced.
fn free_name(p: &Path) -> PathBuf {
    let s = stem(p);
    (1..).map(|i| if i == 1 { p.to_path_buf() } else { p.with_file_name(format!("{s}-{i}.mp4")) }).find(|c| !c.exists()).unwrap()
}

/// Failures another encoder cannot fix (full disk, no write permission): retrying would render twice for nothing.
fn storage_error(e: &str) -> bool {
    let e = e.to_lowercase();
    ["no space left", "not enough space", "disk full", "permission denied", "access is denied", "(os error 5)", "(os error 112)", "cannot create"]
        .iter()
        .any(|k| e.contains(k))
}

/// Renders with the hybrid QSV path, falling back to full software (logged).
pub fn render(
    env: &MediaEnv,
    segs: &[Segment],
    target: (i64, i64, f64),
    o: &Options,
    out: &Path,
    progress: &dyn Fn(f64),
    cancelled: &dyn Fn() -> bool,
) -> Result<&'static str, RunError> {
    let total_ms = o.duration(segs);
    // Text goes through a temporary ASS file in the cache (never next to the user's output).
    let ass = subtitles(segs, o, (target.0, target.1)).map(|text| {
        let dir = env.cache.join("render");
        let _ = std::fs::create_dir_all(&dir);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let p = dir.join(format!("titles-{}-{nanos}.ass", std::process::id()));
        std::fs::write(&p, text).map(|()| p)
    });
    let ass = match ass {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => return Err(RunError::Failed(format!("Cannot write the titles file: {e}"))),
        None => None,
    };
    let result = render_with(env, segs, target, o, ass.as_deref(), out, total_ms, progress, cancelled);
    if let Some(p) = ass {
        let _ = std::fs::remove_file(p);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn render_with(
    env: &MediaEnv,
    segs: &[Segment],
    target: (i64, i64, f64),
    o: &Options,
    ass: Option<&Path>,
    out: &Path,
    total_ms: i64,
    progress: &dyn Fn(f64),
    cancelled: &dyn Fn() -> bool,
) -> Result<&'static str, RunError> {
    let mut last = String::new();
    for (name, decode, encoder) in [("hybrid-qsv", Decode::Hybrid, Encoder::Qsv), ("libx264", Decode::Software, Encoder::X264)] {
        let result = write_atomically(out, |part| {
            let mut args = build_args(segs, decode, encoder, target, o, ass);
            args.push(part.into());
            run_ffmpeg(env, &args, total_ms, progress, cancelled).map(drop)
        });
        match result {
            Ok(()) => return Ok(name),
            Err(RunError::Cancelled) => return Err(RunError::Cancelled),
            Err(RunError::Failed(e)) if storage_error(&e) => return Err(RunError::Failed(e)),
            Err(RunError::Failed(e)) => {
                tracing::warn!(path = name, error = %e, "export attempt failed, trying next");
                last = e;
            }
        }
    }
    Err(RunError::Failed(last))
}

/// Job `media.export`, params `{"projectId": n, "preset": "landscape", "options": Options}`
/// (legacy `fit` / `transitionMs` keys are still read when `options` is absent).
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
                let speech = crate::transcribe::get(c, clip.media_id)?.map(|t| t.segments).unwrap_or_default();
                Ok(Segment { media, in_ms: clip.in_ms, out_ms: clip.out_ms, caption: clip.caption, speech })
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
    let o: Options = match job.params.get("options") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(o) => o,
            Err(e) => return Outcome::Failed(format!("Invalid export options: {e}")),
        },
        None => Options {
            fill: job.params["fit"].as_str() == Some("fill"),
            transition_ms: job.params["transitionMs"].as_i64().unwrap_or(0),
            ..Options::default()
        },
    };
    let mut o = o;
    if let Err(e) = o.check() {
        return Outcome::Failed(e);
    }
    let planned = output_path(projects, project_id, &name, preset_name);
    // Rendered, levelled and validated under a temporary name; only then promoted to the final name.
    let staging = planned.with_extension("unvalidated.mp4");
    let expected_ms = o.duration(&segs);

    // Library music: downloaded now, deleted when `_music` drops (after render + validation, any outcome).
    const TOTAL: u64 = 1000;
    progress(0, TOTAL);
    let mut music_info = serde_json::Value::Null;
    let _music = match o.music.as_ref().and_then(|m| m.library.clone()) {
        Some(id) => {
            let t = match crate::sounds::track(&id) {
                Ok(t) => t,
                Err(e) => return Outcome::Failed(e),
            };
            match crate::sounds::fetch(&t, &env.cache.join("downloads").join("sounds"), env.job_object, &|f| progress((f * 50.0) as u64, TOTAL), cancelled) {
                Ok(file) => {
                    o.music.as_mut().unwrap().path = file.0.to_string_lossy().into();
                    o.credit = Some(t.credit.clone());
                    music_info = json!({"title": t.title, "credit": t.credit, "downloadedAndRemoved": true});
                    Some(file)
                }
                Err(e) if cancelled() => {
                    tracing::info!(error = %e, "library music download cancelled");
                    return Outcome::Cancelled;
                }
                Err(e) => return Outcome::Failed(e),
            }
        }
        None => None,
    };
    // Rendering is ~85 % of the job, validation (a full decode) the rest.
    let started = Instant::now();
    let encoder = match render(env, &segs, target, &o, &staging, &|f| progress(50 + (f * 850.0) as u64, TOTAL), cancelled) {
        Ok(e) => e,
        Err(RunError::Cancelled) => return Outcome::Cancelled,
        Err(RunError::Failed(e)) => return Outcome::Failed(format!("Render failed: {e}")),
    };
    let mut loudness = serde_json::Value::Null;
    if o.normalize {
        match level_loudness(env, &staging, cancelled) {
            Ok((before, gain)) => loudness = json!({"measuredLufs": before, "gainDb": gain, "targetLufs": TARGET_LUFS}),
            Err(RunError::Cancelled) => {
                let _ = std::fs::remove_file(&staging);
                return Outcome::Cancelled;
            }
            Err(RunError::Failed(e)) => tracing::warn!(error = %e, "loudness pass failed; first-pass level kept"),
        }
    }
    let render_seconds = started.elapsed().as_secs_f64();
    progress(900, TOTAL);
    let validation = validate(env, &staging, expected_ms, segs.len(), target);
    // A file that failed validation is kept (the user may still want it) but never under the normal name.
    let out = free_name(&if validation.ok { planned } else { planned.with_file_name(format!("{}-failed-validation.mp4", stem(&planned))) });
    if let Err(e) = std::fs::rename(&staging, &out) {
        return Outcome::Failed(format!("Cannot move the finished video to {}: {e} (it is still at {})", out.display(), staging.display()));
    }
    progress(TOTAL, TOTAL);
    tracing::info!(project_id, preset = preset_name, encoder, render_seconds, ok = validation.ok, path = %out.display(), "export finished");
    Outcome::Completed(json!({
        "path": out,
        "preset": preset_name,
        "encoder": encoder,
        "renderSeconds": (render_seconds * 10.0).round() / 10.0,
        "durationMs": expected_ms,
        "transitionMs": o.transition(&segs),
        "options": o,
        "music": music_info,
        "loudness": loudness,
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

    fn seg(in_ms: i64, out_ms: i64, caption: Option<&str>, speech: &[(i64, i64, &str)]) -> Segment {
        let probe = media::Probe { duration_ms: 60_000, width: 1920, height: 1080, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
            audio_codec: None, audio_streams: 0, bitrate: None, source_created_at: None };
        Segment {
            media: Media { id: 1, path: "x".into(), size: 1, probe, imported_at: String::new(), thumb: None, proxy: None, scene_count: None,
                transcribed: false, tags: None, mark: Default::default() },
            in_ms, out_ms, caption: caption.map(String::from),
            speech: speech.iter().map(|&(a, b, t)| crate::transcribe::Segment { start_ms: a, end_ms: b, text: t.into() }).collect(),
        }
    }

    #[test]
    fn subtitles_timing_text_and_safe_area() {
        let segs = [seg(10_000, 15_000, Some("Ngày 1 {bold}"), &[(9_000, 11_000, "Xin chào"), (20_000, 21_000, "ngoài clip")]),
                    seg(30_000, 36_000, None, &[(31_000, 33_500, "Đi thôi")])];
        let o = Options { transition_ms: 1_000, titles: Titles { title: "Đà Lạt".into(), subtitle: String::new(), ending: "Hết".into(), captions: true }, ..Options::default() };
        assert_eq!(o.starts(&segs), [0, 4_000]);
        let ass = subtitles(&segs, &o, (1080, 1920)).unwrap();
        assert!(ass.contains("Dialogue: 0,0:00:00.00,0:00:04.00,Title,,0,0,0,,{\\fad(500,500)}Đà Lạt"), "{ass}");
        assert!(ass.contains("0:00:00.00,0:00:04.00,Label,,0,0,0,,{\\fad(300,300)}Ngày 1 (bold)"), "user braces neutralised");
        assert!(ass.contains("0:00:00.00,0:00:01.00,Caption,,0,0,0,,Xin chào"), "clipped to the clip start");
        assert!(ass.contains("0:00:05.00,0:00:07.50,Caption,,0,0,0,,Đi thôi"), "shifted by the transition overlap");
        assert!(!ass.contains("ngoài clip"));
        assert!(ass.contains("0:00:06.50,0:00:10.00,Title,,0,0,0,,{\\fad(500,700)}Hết"));
        assert!(ass.contains(",2,86,86,345,1"), "vertical captions sit above the bottom 18 %: {ass}");
        assert!(subtitles(&[seg(0, 1_000, None, &[(0, 500, "x")])], &Options::default(), (1920, 1080)).is_none(), "no text → no ASS pass");
        let credited = Options { credit: Some("\"Carefree\" Kevin MacLeod".into()), ..Options::default() };
        let ass = subtitles(&[seg(0, 10_000, None, &[])], &credited, (1920, 1080)).unwrap();
        assert!(ass.contains("0:00:06.00,0:00:10.00,Credit,,0,0,0,,{\\fad(400,400)}\"Carefree\" Kevin MacLeod"), "{ass}");
    }

    #[test]
    fn options_are_checked() {
        assert!(Options::default().check().is_ok());
        assert!(Options { transition: "explode".into(), ..Options::default() }.check().is_err());
        assert!(Options { color: "neon".into(), ..Options::default() }.check().is_err());
        let lib = |id: &str| Options { music: Some(Music { library: Some(id.into()), volume: 0.4, duck: true, ..Default::default() }), ..Options::default() };
        assert!(lib("carefree").check().is_ok(), "library track needs no local file");
        assert!(lib("not-a-track").check().is_err());
        assert!(Options { music: Some(Music { path: r"Z:\nope.mp3".into(), library: None, volume: 1.0, duck: true }), ..Options::default() }.check().is_err());
        let o: Options = serde_json::from_value(json!({"fill": true, "color": "warm"})).unwrap();
        assert_eq!((o.fill, o.color.as_str(), o.original_volume, o.transition.as_str()), (true, "warm", 1.0, "fade"), "defaults fill the rest");
    }

    #[test]
    fn output_name_is_safe() {
        let p = output_path(Path::new(r"E:\d\projects"), 3, "Đà Lạt: 2026/07?", "story");
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("Đà Lạt_ 2026_07_-story-") && name.ends_with(".mp4"), "{name}");
    }

    #[test]
    fn earlier_exports_are_never_replaced() {
        let dir = std::env::temp_dir().join(format!("memora-free-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Trip-reels-100.mp4");
        assert_eq!(free_name(&p), p);
        std::fs::write(&p, "x").unwrap();
        assert_eq!(free_name(&p), dir.join("Trip-reels-100-2.mp4"));
        std::fs::write(dir.join("Trip-reels-100-2.mp4"), "x").unwrap();
        assert_eq!(free_name(&p), dir.join("Trip-reels-100-3.mp4"));
        assert_eq!(p.with_extension("unvalidated.mp4").file_name().unwrap(), "Trip-reels-100.unvalidated.mp4");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn storage_errors_are_not_retried() {
        assert!(storage_error("FFmpeg exited with exit code: 1: av_interleaved_write_frame(): No space left on device"));
        assert!(storage_error("Cannot write E:\\x.mp4: Access is denied. (os error 5)"));
        assert!(storage_error("Error opening output E:\\x.part: Permission denied"));
        assert!(!storage_error("FFmpeg exited with exit code: 1: Error initializing filter 'vpp_qsv'"));
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
        let one = |m: &Media| vec![Segment { media: m.clone(), in_ms: 0, out_ms: 1000, caption: None, speech: vec![] }];
        let (sw, hy) = (dir.join("sw.mp4"), dir.join("hy.mp4"));
        let mut args = build_args(&one(&mr), Decode::Software, Encoder::X264, target, &Options::default(), None);
        args.push(sw.clone().into());
        assert!(run_ffmpeg(&env, &args, 1000, &|_| {}, &|| false).is_ok());
        let mut args = build_args(&one(&mr), Decode::Hybrid, Encoder::Qsv, target, &Options::default(), None);
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
            Segment { media: ma.clone(), in_ms: 500, out_ms: 1500, caption: Some("Đà Lạt".into()), speech: vec![] },
            Segment { media: mr.clone(), in_ms: 1000, out_ms: 2500, caption: None, speech: vec![crate::transcribe::Segment { start_ms: 1200, end_ms: 2000, text: "Xin chào".into() }] },
        ];
        let target = Preset::Landscape.target(&ma);
        let out = dir.join("out.mp4");
        let last = Mutex::new(0.0);
        let Ok(encoder) = render(&env, &segs, target, &Options::default(), &out, &|f| *last.lock().unwrap() = f, &|| false) else {
            panic!("render failed");
        };
        assert!(*last.lock().unwrap() > 0.9, "progress reported");
        let v = validate(&env, &out, 2500, segs.len(), target);
        assert!(v.ok, "validation failed ({encoder}): {v:?}");
        // Second segment has no audio → silence is inserted, which is not a failure.
        assert!(a.exists() && rot.exists(), "originals untouched");

        // Crossfades + centre crop (vertical output from landscape sources): 1 s + 1.5 s − 0.4 s overlap.
        // Full look: crop, dissolve, colour, fades, titles + captions, music ducked under the clips, loudness.
        let music = dir.join("music.m4a");
        ffmpeg(&bin, &["-f", "lavfi", "-i", "sine=f=330:d=1.2", "-c:a", "aac"], &music);
        let look = Options {
            fill: true, transition_ms: 400, transition: "dissolve".into(), color: "cinematic".into(), fade: true,
            original_volume: 0.8, music: Some(Music { path: music.to_string_lossy().into(), library: None, volume: 0.5, duck: true }),
            titles: Titles { title: "Đà Lạt – Tháng Hai".into(), subtitle: "Đạp xe".into(), ending: "Hẹn gặp lại".into(), captions: true },
            normalize: true, credit: None,
        };
        assert!(look.check().is_ok());
        assert_eq!((look.transition(&segs), look.duration(&segs)), (400, 2100));
        assert_eq!(Options { transition_ms: 5000, ..Options::default() }.transition(&segs), 500, "clamped to half the shortest clip");
        let story = Preset::Story.target(&ma);
        let (xf, fit) = (dir.join("xfade.mp4"), dir.join("fit.mp4"));
        if let Err(RunError::Failed(e)) = render(&env, &segs, story, &look, &xf, &|_| {}, &|| false) {
            panic!("full-look render failed: {e}");
        }
        assert!(std::fs::read_dir(dir.join("cache").join("render")).unwrap().next().is_none(), "temporary ASS removed");
        let (_, gain) = level_loudness(&env, &xf, &|| false).unwrap_or_else(|_| panic!("loudness pass failed"));
        let after = measure_lufs(&env, &xf).unwrap();
        assert!(gain == 0.0 || (after - TARGET_LUFS).abs() < 2.0, "levelled to {after} LUFS (gain {gain})");
        let v = validate(&env, &xf, 2100, segs.len(), story);
        assert!(v.ok, "crossfade export failed validation: {v:?}");
        assert!(render(&env, &segs[..1], story, &Options::default(), &fit, &|_| {}, &|| false).is_ok());
        let fill = dir.join("fill.mp4"); // no fade, so the first frame shows the picture
        assert!(render(&env, &segs[..1], story, &Options { fill: true, ..Options::default() }, &fill, &|_| {}, &|| false).is_ok());
        // Top row of a 9×16 grey thumbnail: letterbox = black bar, fill = picture.
        let top = |f: &Path| frame(&bin, f)[..9].iter().map(|&x| x as u32).sum::<u32>() / 9;
        assert!(top(&fit) > 20 && top(&fill) > 40, "no black bars: blurred background (fit {}) or crop (fill {})", top(&fit), top(&fill));
        assert_ne!(frame(&bin, &fit), frame(&bin, &fill), "background fill differs from a crop");
        assert!(top(&xf) < 20, "full look starts from black (fade in)");

        // Validation catches a wrong expectation.
        assert!(!validate(&env, &out, 9000, 2, target).ok);
        assert!(!validate(&env, &dir.join("missing.mp4"), 2500, 2, target).ok);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Manual end-to-end check on real footage:
/// `MEMORA_DB=<copy of memora.db> MEMORA_OUT=<file.mp4> MEMORA_STYLE=travel_cinematic cargo test real_library_export -- --ignored --nocapture`
/// Plans from a COPY of the library database, renders from the original files, validates.
#[cfg(test)]
#[test]
#[ignore]
fn real_library_export() {
    use crate::editing;
    let db = crate::db::open(Path::new(&std::env::var("MEMORA_DB").expect("MEMORA_DB"))).unwrap();
    let out = PathBuf::from(std::env::var("MEMORA_OUT").expect("MEMORA_OUT"));
    let st = editing::style(&std::env::var("MEMORA_STYLE").unwrap_or("travel_cinematic".into())).unwrap();
    let sources = editing::gather(&db, None, None).unwrap();
    let plan = editing::plan(&st, &sources);
    let mut o = editing::styled_options(Options::default(), &st, &plan);
    o.titles.subtitle = "Memora Studio · test".into();
    o.titles.ending = "Hẹn gặp lại".into();
    o.titles.captions = true;
    let segs: Vec<Segment> = plan
        .clips
        .iter()
        .map(|c| {
            let m = sources.iter().find(|s| s.media.id == c.media_id).unwrap();
            let speech = m.transcript.as_ref().map(|t| t.segments.clone()).unwrap_or_default();
            Segment { media: m.media.clone(), in_ms: c.in_ms, out_ms: c.out_ms, caption: None, speech }
        })
        .collect();
    let target = Preset::parse(st.export_preset).unwrap().target(&segs[0].media);
    let jo = crate::process::JobObject::new().unwrap();
    let mutex = std::sync::Mutex::new(Ok(crate::db::open_in_memory()));
    let env = MediaEnv {
        ffprobe: media::tool_path(None, "ffprobe.exe"), ffmpeg: media::tool_path(None, "ffmpeg.exe"),
        cache: out.parent().unwrap().join("cache"), job_object: &jo, db: &mutex,
    };
    // Optional library music (MEMORA_MUSIC=<track id>): fetched, used, deleted like in the export job.
    let _music = std::env::var("MEMORA_MUSIC").ok().map(|id| {
        let tr = crate::sounds::track(&id).unwrap();
        let f = crate::sounds::fetch(&tr, &env.cache.join("downloads").join("sounds"), &jo, &|_| {}, &|| false).unwrap();
        o.music = Some(Music { path: f.0.to_string_lossy().into(), library: Some(id), volume: 0.35, duck: true });
        o.credit = Some(tr.credit);
        f
    });
    let t = Instant::now();
    let enc = render(&env, &segs, target, &o, &out, &|_| {}, &|| false).unwrap_or_else(|_| panic!("render failed"));
    if o.normalize {
        let (before, gain) = level_loudness(&env, &out, &|| false).unwrap_or_else(|_| panic!("loudness pass failed"));
        println!("loudness {before:.1} LUFS, gain {gain:+.1} dB → {:.1} LUFS", measure_lufs(&env, &out).unwrap());
    }
    let v = validate(&env, &out, o.duration(&segs), segs.len(), target);
    println!("{} clips, {:.1} s, title {:?}, {} in {:.0} s, valid {}: {:?}", segs.len(), o.duration(&segs) as f64 / 1000.0,
        o.titles.title, enc, t.elapsed().as_secs_f64(), v.ok, v.warnings);
    assert!(v.ok, "{v:?}");
}
