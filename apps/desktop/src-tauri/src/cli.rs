//! `memora-cli`: the Memora Studio engine without the window — same data folder, database, Editing Core
//! and export as the app, so everything done here shows up in the app (projects, timeline, exports).
//! Output is short plain text, readable by people and cheap for an assistant that drives the edit.
//! Close the app before running jobs here (export, analyse…): both would work on the same data folder.

use crate::editing::{self, Wishes};
use crate::jobs::{Job, Outcome, Status};
use crate::{Db, analysis, db, export, job_runner, media, projects, scoring, settings, sounds, tagging, timeline, transcribe};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const HELP: &str = "memora-cli — Memora Studio without the window (close the app before export/job)

Library
  status                          data folder and library counts
  media                           library files: id, length, recorded, analysis, top concepts
  concepts                        concepts found in the library (seconds on screen)
  scenes MEDIA                    scenes with scores and concepts every 5 s
  transcript MEDIA                speech segments
  job KIND [JSON]                 media.import|proxy|analyze|tag|transcribe, e.g. job media.analyze '{\"ids\":[3]}'
Projects
  projects | new-project NAME
  styles
  plan PROJECT --style ID [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--target SECONDS]
                [--prefer a,b] [--avoid c,d]       concepts in English or Vietnamese
  show-plan PROJECT
  apply PROJECT [--keep 0,2,1]    plan clips (indices, in this order) → timeline + style look
Timeline (times in seconds)
  timeline PROJECT
  clip-add PROJECT MEDIA IN OUT | clip-trim CLIP IN OUT | clip-remove CLIP | clip-move CLIP -1|1
  clip-caption CLIP TEXT          empty text clears it
Look, sound, export
  options PROJECT [--color C] [--transition T] [--transition-ms N] [--fill on|off] [--fade on|off]
                  [--normalize on|off] [--volume 0..2] [--title T] [--subtitle T] [--ending T]
                  [--captions on|off] [--music FILE|none] [--music-library ID] [--music-volume 0..2] [--duck on|off]
  sounds [PROJECT]                free music library (+ suggestions for the project)
  export PROJECT [--preset landscape|story|reels|archive]
";

pub fn run(args: Vec<String>) -> i32 {
    let result = parse(&args).and_then(|a| {
        if a.pos.is_empty() || a.pos[0] == "help" || a.flags.contains_key("help") {
            return Ok(HELP.to_string());
        }
        dispatch(&open()?, &a)
    });
    match result {
        Ok(out) => {
            print!("{out}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

struct Args {
    pos: Vec<String>,
    flags: HashMap<String, String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let (mut pos, mut flags) = (vec![], HashMap::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.strip_prefix("--") {
            Some(k) => {
                let v = it.next().ok_or(format!("--{k} needs a value"))?;
                flags.insert(k.to_string(), v.clone());
            }
            None => pos.push(a.clone()),
        }
    }
    Ok(Args { pos, flags })
}

impl Args {
    fn arg(&self, i: usize, name: &str) -> Result<&str, String> {
        self.pos.get(i).map(String::as_str).ok_or(format!("missing {name} (see `memora-cli help`)"))
    }
    fn id(&self, i: usize, name: &str) -> Result<i64, String> {
        self.arg(i, name)?.parse().map_err(|_| format!("{name} must be a number"))
    }
    fn secs(&self, i: usize, name: &str) -> Result<i64, String> {
        ms(self.arg(i, name)?)
    }
    fn list(&self, k: &str) -> Vec<String> {
        self.flags.get(k).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    }
}

fn ms(s: &str) -> Result<i64, String> {
    s.parse::<f64>().map(|v| (v * 1000.0).round() as i64).map_err(|_| format!("not a time in seconds: {s}"))
}

fn on(v: &str) -> Result<bool, String> {
    match v.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("expected on/off, got {v}")),
    }
}

fn num(v: &str) -> Result<f64, String> {
    v.parse().map_err(|_| format!("not a number: {v}"))
}

/// m:ss.s
fn t(ms: i64) -> String {
    format!("{}:{:04.1}", ms / 60_000, (ms % 60_000) as f64 / 1000.0)
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

struct Ctx {
    root: PathBuf,
    db: Db,
}

/// The app's data folder (`MEMORA_DATA_ROOT` overrides the app setting).
fn open() -> Result<Ctx, String> {
    let root = match std::env::var_os("MEMORA_DATA_ROOT") {
        Some(r) => PathBuf::from(r),
        None => {
            let config = std::env::var_os("APPDATA").map(PathBuf::from).ok_or("APPDATA is not set")?.join("dev.memora.studio");
            settings::load(&config).data_root.ok_or("No data folder yet: open Memora Studio once and choose one.")?
        }
    };
    let c = db::open(&root.join("database").join("memora.db"))?;
    Ok(Ctx { root, db: Arc::new(Mutex::new(Ok(c))) })
}

impl Ctx {
    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let g = self.db.lock().unwrap();
        f(g.as_ref().map_err(Clone::clone)?)
    }

    /// Runs one job to the end (progress on stderr) with the same runner as the app's job queue.
    fn job(&self, kind: &str, params: Value) -> Result<Value, String> {
        let s = Arc::new(Mutex::new(settings::Settings { data_root: Some(self.root.clone()) }));
        let mut runner = job_runner(s, self.db.clone(), Arc::new(Mutex::new(None)))?;
        let job = Job { id: 1, kind: kind.into(), params, status: Status::Running, done: 0, total: 0, result: None, error: None };
        let shown = Cell::new(-1i64);
        let progress = |done: u64, total: u64| {
            let pct = (done * 100 / total.max(1)) as i64 / 10 * 10;
            if pct != shown.get() {
                shown.set(pct);
                eprintln!("  {kind} {pct}%");
            }
        };
        match runner(&job, &progress, &|| false) {
            Outcome::Completed(v) => Ok(v),
            Outcome::Failed(e) => Err(e),
            Outcome::Cancelled => Err("cancelled".into()),
        }
    }
}

fn dispatch(ctx: &Ctx, a: &Args) -> Result<String, String> {
    let cmd = a.pos[0].as_str();
    match cmd {
        "status" => ctx.with(|c| status(c, ctx)),
        "media" => ctx.with(list_media),
        "concepts" => ctx.with(concepts),
        "scenes" => ctx.with(|c| scenes(c, a.id(1, "MEDIA")?)),
        "transcript" => ctx.with(|c| transcript(c, a.id(1, "MEDIA")?)),
        "projects" => ctx.with(|c| {
            Ok(projects::list(c)?
                .iter()
                .map(|p| format!("#{} {} ({} clips)\n", p.id, p.name, timeline::list(c, p.id).map(|l| l.len()).unwrap_or(0)))
                .collect())
        }),
        "new-project" => ctx.with(|c| projects::create(c, &a.pos[1..].join(" ")).map(|p| format!("#{} {}\n", p.id, p.name))),
        "styles" => Ok(editing::styles()
            .iter()
            .map(|s| {
                format!(
                    "{:<18} {} s {:<9} clips {}–{} s, transition {} ms, look {}{}\n",
                    s.id, s.target_ms / 1000, s.export_preset, s.clip_min_ms as f64 / 1000.0, s.clip_max_ms as f64 / 1000.0,
                    s.transition_ms, s.color, if s.captions { ", subtitles" } else { "" }
                )
            })
            .collect()),
        "plan" => ctx.with(|c| {
            let project = a.id(1, "PROJECT")?;
            let st = editing::style(a.flags.get("style").map(String::as_str).ok_or("--style is required (see `styles`)")?)?;
            let wishes = Wishes {
                prefer: a.list("prefer"),
                avoid: a.list("avoid"),
                target_ms: a.flags.get("target").map(|v| ms(v)).transpose()?,
            };
            let from = a.flags.get("from").map(String::as_str);
            let to = a.flags.get("to").map(String::as_str);
            let p = editing::plan_with(&st, &editing::gather(c, from, to)?, &wishes);
            editing::save(c, project, &p)?;
            show_plan(c, project)
        }),
        "show-plan" => ctx.with(|c| show_plan(c, a.id(1, "PROJECT")?)),
        "apply" => ctx.with(|c| {
            let project = a.id(1, "PROJECT")?;
            let plan = editing::load(c, project)?.ok_or("No plan yet: run `plan` first.")?;
            let keep: Vec<usize> = match a.flags.get("keep") {
                Some(k) => k.split(',').map(|i| i.trim().parse().map_err(|_| format!("bad index in --keep: {i}"))).collect::<Result<_, _>>()?,
                None => (0..plan.clips.len()).collect(),
            };
            editing::apply(c, project, &keep)?;
            let o = editing::styled_options(export::load_options(c, project), &editing::style(&plan.style)?, &plan);
            export::save_options(c, project, &o)?;
            Ok(format!("{}{}", show_timeline(c, project)?, show_options(&o)))
        }),
        "timeline" => ctx.with(|c| show_timeline(c, a.id(1, "PROJECT")?)),
        "clip-add" => ctx.with(|c| {
            timeline::add(c, a.id(1, "PROJECT")?, a.id(2, "MEDIA")?, a.secs(3, "IN")?, a.secs(4, "OUT")?)?;
            show_timeline(c, a.id(1, "PROJECT")?)
        }),
        "clip-trim" => ctx.with(|c| {
            let clip = timeline::set_range(c, a.id(1, "CLIP")?, a.secs(2, "IN")?, a.secs(3, "OUT")?)?;
            show_timeline(c, clip.project_id)
        }),
        "clip-remove" => ctx.with(|c| timeline::remove(c, a.id(1, "CLIP")?).map(|()| "removed\n".into())),
        "clip-move" => {
            let mut g = ctx.db.lock().unwrap();
            let c = g.as_mut().map_err(|e| e.clone())?;
            timeline::move_clip(c, a.id(1, "CLIP")?, a.id(2, "DELTA")?).map(|()| "moved\n".into())
        }
        "clip-caption" => ctx.with(|c| timeline::set_caption(c, a.id(1, "CLIP")?, &a.pos[2..].join(" ")).map(|()| "caption set\n".into())),
        "options" => ctx.with(|c| {
            let project = a.id(1, "PROJECT")?;
            let mut o = export::load_options(c, project);
            set_options(&mut o, &a.flags)?;
            if !a.flags.is_empty() {
                export::save_options(c, project, &o)?;
            }
            Ok(show_options(&o))
        }),
        "sounds" => ctx.with(|c| {
            let mut out = String::new();
            if let Some(project) = a.pos.get(1) {
                let project = project.parse().map_err(|_| "PROJECT must be a number")?;
                for r in crate::recommend_for(c, project, 3)? {
                    out += &format!("suggested {} ({:.2}): {}\n", r.id, r.score, r.reasons.join(", "));
                }
            }
            for s in sounds::catalog() {
                out += &format!("{:<28} {} — {} s, {} energy, {}\n", s.id, s.title, s.duration_ms / 1000, s.energy, s.moods.join("/"));
            }
            Ok(out)
        }),
        "export" => {
            let project = a.id(1, "PROJECT")?;
            let (preset, options) = ctx.with(|c| {
                let preset = match a.flags.get("preset") {
                    Some(p) => p.clone(),
                    None => editing::load(c, project)?.map(|p| p.export_preset).unwrap_or_else(|| "landscape".into()),
                };
                Ok((preset, export::load_options(c, project)))
            })?;
            let r = ctx.job("media.export", json!({"projectId": project, "preset": preset, "options": options}))?;
            Ok(export_report(&r))
        }
        "job" => {
            let kind = a.arg(1, "KIND")?;
            if !["media.import", "media.proxy", "media.analyze", "media.tag", "media.transcribe"].contains(&kind) {
                return Err(format!("unknown job kind: {kind}"));
            }
            let params = a.pos.get(2).map(|p| serde_json::from_str(p).map_err(|e| format!("JSON: {e}"))).transpose()?.unwrap_or(Value::Null);
            ctx.job(kind, params).map(|v| format!("{v}\n"))
        }
        _ => Err(format!("unknown command: {cmd} (see `memora-cli help`)")),
    }
}

fn status(c: &Connection, ctx: &Ctx) -> Result<String, String> {
    let m = media::list(c)?;
    Ok(format!(
        "data folder {}\nmedia {} (analysed {}, tagged {}, transcribed {}), projects {}\n",
        ctx.root.display(),
        m.len(),
        analysis::scene_counts(c)?.len(),
        tagging::all(c)?.len(),
        transcribe::transcribed(c)?.len(),
        projects::list(c)?.len()
    ))
}

fn top(tags: &[tagging::TagScore], n: usize) -> String {
    tags.iter().filter(|t| !t.negative).take(n).map(|t| format!("{} {:.0}%", t.vi, t.score * 100.0)).collect::<Vec<_>>().join(", ")
}

fn list_media(c: &Connection) -> Result<String, String> {
    let (counts, tags, said, marks) = (analysis::scene_counts(c)?, tagging::all(c)?, transcribe::transcribed(c)?, editing::marks(c)?);
    let mut out = String::new();
    for m in media::list(c)? {
        let mark = marks.get(&m.id).copied().unwrap_or_default();
        let state = format!(
            "{}{}{}{}{}",
            if counts.contains_key(&m.id) { "A" } else { "-" },
            if tags.contains_key(&m.id) { "T" } else { "-" },
            if said.contains(&m.id) { "S" } else { "-" },
            if mark.favorite { "★" } else { "" },
            if mark.excluded { "⊘" } else { "" },
        );
        let concepts = tags.get(&m.id).map(|t| top(&tagging::summarize(t, 0, i64::MAX, 0.05), 3)).unwrap_or_default();
        let rec = m.probe.source_created_at.as_deref().unwrap_or("?").chars().take(16).collect::<String>().replace('T', " ");
        out += &format!("#{:<3} {:<36} {:>7} {} {:<5} {}\n", m.id, file_name(&m.path), t(m.probe.duration_ms), rec, state, concepts);
    }
    Ok(out + "A analysed, T tagged, S transcribed, ★ favorite, ⊘ excluded\n")
}

fn concepts(c: &Connection) -> Result<String, String> {
    // Seconds where the concept is the strongest one on screen, and in how many files.
    let mut seen: HashMap<(String, String, bool), (i64, usize)> = HashMap::new();
    for t in tagging::all(c)?.values() {
        let step = t.samples.windows(2).next().map_or(2000, |w| w[1].at_ms - w[0].at_ms).max(1);
        let mut here: HashMap<usize, i64> = HashMap::new();
        for s in &t.samples {
            if let Some(&(i, _)) = s.tags.first().filter(|&&(_, p)| p >= 0.2) {
                *here.entry(i).or_default() += step;
            }
        }
        for (i, ms) in here {
            let n = &t.tags[i];
            let e = seen.entry((n.en.clone(), n.vi.clone(), n.negative)).or_default();
            e.0 += ms;
            e.1 += 1;
        }
    }
    let mut all: Vec<_> = seen.into_iter().collect();
    all.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
    Ok(all
        .iter()
        .map(|((en, vi, neg), (ms, files))| format!("{}{en} / {vi}: {} s in {files} file(s)\n", if *neg { "⚠ " } else { "" }, ms / 1000))
        .collect())
}

fn scenes(c: &Connection, media_id: i64) -> Result<String, String> {
    let m = media::get(c, media_id)?.ok_or(format!("Media {media_id} not found."))?;
    let tags = tagging::get(c, media_id)?;
    let mut out = format!("#{} {} ({})\n", m.id, file_name(&m.path), t(m.probe.duration_ms));
    let list = scoring::score(analysis::scenes(c, media_id)?, &analysis::library_hashes(c)?);
    if list.is_empty() {
        return Ok(out + "not analysed yet: job media.analyze '{\"ids\":[ID]}'\n");
    }
    for s in list {
        out += &format!(
            "scene {} {}–{} technical {:.0} {}\n",
            s.scene.index, t(s.scene.start_ms), t(s.scene.end_ms), s.technical.total * 100.0, s.flags.join(" ")
        );
        if let Some(tg) = &tags {
            let mut a = s.scene.start_ms;
            while a < s.scene.end_ms {
                let b = (a + 5000).min(s.scene.end_ms);
                let w = tagging::summarize(tg, a, b, 0.1);
                let bad: Vec<_> = w.iter().filter(|x| x.negative && x.score >= 0.15).map(|x| format!("⚠ {}", x.vi)).collect();
                out += &format!("  {}–{} {} {}\n", t(a), t(b), top(&w, 2), bad.join(" "));
                a = b;
            }
        }
    }
    Ok(out)
}

fn transcript(c: &Connection, media_id: i64) -> Result<String, String> {
    let tr = transcribe::get(c, media_id)?.ok_or("not transcribed yet: job media.transcribe '{\"ids\":[ID],\"model\":\"whisper-small\",\"language\":\"auto\"}'")?;
    let mut out = format!("{} ({})\n", tr.model_id, tr.language.as_deref().unwrap_or("?"));
    for s in &tr.segments {
        out += &format!("[{}–{}] {}\n", t(s.start_ms), t(s.end_ms), s.text.trim());
    }
    Ok(out)
}

fn names(c: &Connection) -> Result<HashMap<i64, String>, String> {
    Ok(media::list(c)?.into_iter().map(|m| (m.id, file_name(&m.path).to_string())).collect())
}

fn show_plan(c: &Connection, project: i64) -> Result<String, String> {
    let p = editing::load(c, project)?.ok_or("No plan yet: run `plan` first.")?;
    let files = names(c)?;
    let mut out = format!(
        "plan {} — {} clips, {:.1} s of {} s, {} candidates, title \"{}\"\n",
        p.style, p.clips.len(), p.total_ms as f64 / 1000.0, p.target_ms / 1000, p.candidates, p.suggested_title
    );
    if p.wishes != Wishes::default() {
        out += &format!("wishes: prefer [{}] avoid [{}]\n", p.wishes.prefer.join(", "), p.wishes.avoid.join(", "));
    }
    for (i, cl) in p.clips.iter().enumerate() {
        out += &format!(
            "{i:>2}. #{} {} {}–{} ({:.1} s) rank {:.2} | {}\n",
            cl.media_id, files.get(&cl.media_id).map_or("?", |s| s.as_str()), t(cl.in_ms), t(cl.out_ms),
            (cl.out_ms - cl.in_ms) as f64 / 1000.0, cl.rank, cl.reasons.join("; ")
        );
    }
    for s in &p.skipped {
        out += &format!("skipped: {s}\n");
    }
    Ok(out)
}

fn show_timeline(c: &Connection, project: i64) -> Result<String, String> {
    let clips = timeline::list(c, project)?;
    let files = names(c)?;
    let o = export::load_options(c, project);
    let shortest = clips.iter().map(|x| x.out_ms - x.in_ms).min().unwrap_or(0);
    let d = if clips.len() < 2 { 0 } else { o.transition_ms.clamp(0, shortest / 2) };
    let total = clips.iter().map(|x| x.out_ms - x.in_ms).sum::<i64>() - (clips.len() as i64 - 1).max(0) * d;
    let mut out = format!("timeline #{project} — {} clips, {:.1} s\n", clips.len(), total as f64 / 1000.0);
    for x in &clips {
        let concept = tagging::get(c, x.media_id)?.map(|tg| top(&tagging::summarize(&tg, x.in_ms, x.out_ms, 0.1), 1)).unwrap_or_default();
        out += &format!(
            "clip {:<4} #{} {} {}–{} ({:.1} s) {} {}\n",
            x.id, x.media_id, files.get(&x.media_id).map_or("?", |s| s.as_str()), t(x.in_ms), t(x.out_ms),
            (x.out_ms - x.in_ms) as f64 / 1000.0, concept, x.caption.as_deref().map(|s| format!("\"{s}\"")).unwrap_or_default()
        );
    }
    Ok(out)
}

fn show_options(o: &export::Options) -> String {
    let music = match &o.music {
        Some(m) => format!(
            "{} vol {:.2}{}",
            m.library.clone().unwrap_or_else(|| m.path.clone()),
            m.volume,
            if m.duck { " ducked" } else { "" }
        ),
        None => "none".into(),
    };
    format!(
        "look: {} {} ms {}, colour {}, fill {}, fade {}, normalize {}, own sound {:.2}\nmusic: {music}\ntext: title \"{}\" subtitle \"{}\" ending \"{}\" subtitles {}\n",
        o.transition, o.transition_ms, if o.transition_ms == 0 { "(cuts)" } else { "" }, o.color, o.fill, o.fade, o.normalize,
        o.original_volume, o.titles.title, o.titles.subtitle, o.titles.ending, o.titles.captions
    )
}

fn set_options(o: &mut export::Options, flags: &HashMap<String, String>) -> Result<(), String> {
    for (k, v) in flags {
        match k.as_str() {
            "color" => o.color = v.clone(),
            "transition" => o.transition = v.clone(),
            "transition-ms" => o.transition_ms = v.parse().map_err(|_| format!("not a number: {v}"))?,
            "fill" => o.fill = on(v)?,
            "fade" => o.fade = on(v)?,
            "normalize" => o.normalize = on(v)?,
            "volume" => o.original_volume = num(v)?,
            "title" => o.titles.title = v.clone(),
            "subtitle" => o.titles.subtitle = v.clone(),
            "ending" => o.titles.ending = v.clone(),
            "captions" => o.titles.captions = on(v)?,
            "music" if v == "none" => o.music = None,
            "music" => o.music = Some(export::Music { path: v.clone(), library: None, ..music_or_default(o) }),
            "music-library" => o.music = Some(export::Music { path: String::new(), library: Some(v.clone()), ..music_or_default(o) }),
            "music-volume" => o.music.get_or_insert_with(default_music).volume = num(v)?,
            "duck" => o.music.get_or_insert_with(default_music).duck = on(v)?,
            _ => return Err(format!("unknown option --{k} (see `memora-cli help`)")),
        }
    }
    o.check()
}

/// Same defaults as the app's music picker.
fn default_music() -> export::Music {
    export::Music { path: String::new(), library: None, volume: 0.35, duck: true }
}

fn music_or_default(o: &export::Options) -> export::Music {
    o.music.clone().unwrap_or_else(default_music)
}

fn export_report(r: &Value) -> String {
    let v = &r["validation"];
    let mut out = format!(
        "{} {}\n{}×{}, {:.1} s, encoder {}, render {} s\n",
        if v["ok"] == true { "OK" } else { "FAILED validation" },
        r["path"].as_str().unwrap_or("?"),
        r["width"], r["height"], r["durationMs"].as_f64().unwrap_or(0.0) / 1000.0, r["encoder"].as_str().unwrap_or("?"), r["renderSeconds"]
    );
    for ch in v["checks"].as_array().into_iter().flatten() {
        out += &format!("  {} {}: {}\n", if ch["ok"] == true { "✓" } else { "✗" }, ch["name"].as_str().unwrap_or(""), ch["detail"].as_str().unwrap_or(""));
    }
    for w in v["warnings"].as_array().into_iter().flatten() {
        out += &format!("  ⚠ {}\n", w.as_str().unwrap_or(""));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx { root: std::env::temp_dir(), db: Arc::new(Mutex::new(Ok(db::open_in_memory()))) }
    }

    fn cli(ctx: &Ctx, line: &[&str]) -> Result<String, String> {
        dispatch(ctx, &parse(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>())?)
    }

    #[test]
    fn parse_times_and_switches() {
        let a = parse(&["clip-trim".into(), "7".into(), "1.5".into(), "--fill".into(), "on".into()]).unwrap();
        assert_eq!((a.id(1, "CLIP").unwrap(), a.secs(2, "IN").unwrap()), (7, 1500));
        assert!(on(&a.flags["fill"]).unwrap() && !on("off").unwrap() && on("maybe").is_err());
        assert!(parse(&["x".into(), "--title".into()]).is_err(), "flag without value");
        assert_eq!(t(75_500), "1:15.5");
    }

    #[test]
    fn project_timeline_options_and_plan_round_trip() {
        let ctx = ctx();
        assert_eq!(cli(&ctx, &["new-project", "Đà", "Lạt"]).unwrap(), "#1 Đà Lạt\n");
        let probe = media::Probe {
            duration_ms: 10_000, width: 1920, height: 1080, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
            audio_codec: None, audio_streams: 0, bitrate: None, source_created_at: Some("2026-02-20T03:00:00Z".into()),
        };
        ctx.with(|c| media::upsert(c, r"E:\clips\walk.mp4", 1, None, &probe, "{}").map(drop)).unwrap();

        let tl = cli(&ctx, &["clip-add", "1", "1", "2", "6.5"]).unwrap();
        assert!(tl.contains("walk.mp4 0:02.0–0:06.5 (4.5 s)"), "{tl}");
        let tl = cli(&ctx, &["clip-trim", "1", "1", "99"]).unwrap();
        assert!(tl.contains("0:01.0–0:10.0"), "out clamped to the file: {tl}");
        cli(&ctx, &["clip-caption", "1", "Lên", "đồi"]).unwrap();
        assert!(cli(&ctx, &["timeline", "1"]).unwrap().contains("\"Lên đồi\""));

        let o = cli(&ctx, &["options", "1", "--color", "warm", "--transition-ms", "500", "--title", "Đà Lạt"]).unwrap();
        assert!(o.contains("colour warm") && o.contains("500 ms") && o.contains("title \"Đà Lạt\""), "{o}");
        assert!(cli(&ctx, &["options", "1", "--color", "neon"]).is_err(), "checked before saving");
        assert!(ctx.with(|c| Ok(export::load_options(c, 1))).unwrap().color == "warm", "bad option not saved");
        assert!(cli(&ctx, &["options", "1", "--music", r"E:\missing.mp3"]).is_err());

        let p = cli(&ctx, &["plan", "1", "--style", "clean_minimal", "--avoid", "selfie", "--target", "30"]).unwrap();
        assert!(p.contains("of 30 s") && p.contains("skipped: walk.mp4: not analysed yet") && p.contains("avoid [selfie]"), "{p}");
        assert!(cli(&ctx, &["plan", "1"]).unwrap_err().contains("--style"));
        assert!(cli(&ctx, &["nope"]).is_err());
        assert!(cli(&ctx, &["styles"]).unwrap().contains("travel_cinematic"));
        assert!(cli(&ctx, &["media"]).unwrap().contains("#1   walk.mp4"));
    }
}
