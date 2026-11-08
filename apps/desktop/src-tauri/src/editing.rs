//! Editing Core (Phase 6, docs/Production.md §6): turns observations (technical scores, OpenCLIP tags,
//! transcripts) and user marks into an explicit, reviewable Editing Plan. Deterministic: the same inputs
//! give the same plan. AI produces observations; this module decides; FFmpeg renders (export.rs).
//! Scores stay separate (§6.1): technical, semantic, speech, favorite — combined only for ranking.

use crate::analysis;
use crate::db::err;
use crate::media::{self, Media};
use crate::scoring::{self, ScoredScene};
use crate::tagging::{self, MediaTags};
use crate::transcribe::{self, Transcript};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Editing style (§6.3). Every field is part of the reviewable contract shown to the user.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    pub id: &'static str,
    pub name: &'static str,
    pub target_ms: i64,
    /// Export preset (export.rs): "landscape" 16:9 or "reels" 9:16.
    pub export_preset: &'static str,
    pub clip_min_ms: i64,
    pub clip_max_ms: i64,
    pub clip_ideal_ms: i64,
    /// 0 = hard cuts; otherwise crossfade length.
    pub transition_ms: i64,
    pub music: &'static str,
    pub titles: &'static str,
    pub pacing: &'static str,
    pub opening: &'static str,
    pub ending: &'static str,
    /// Ranking weights: technical, semantic, speech, favorite.
    pub weights: [f64; 4],
    /// Concepts (tag `en` names) this style favours; empty = any content.
    pub favor: &'static [&'static str],
    /// Concepts preferred for the first / last clip.
    pub open_with: &'static [&'static str],
    pub end_with: &'static [&'static str],
}

const LANDSCAPES: &[&str] = &["aerial view", "mountain", "sea", "beach", "sky and clouds", "sunrise", "lake", "waterfall", "rice field", "forest"];
const PEOPLE: &[&str] = &["people", "group of friends", "family", "child", "selfie"];
const NO_MUSIC: &str = "Original audio only. Background music is not available yet: no licensed offline music source has been chosen (open requirement).";
const NO_TITLES: &str = "No burned-in titles yet: Vietnamese title rendering (libass/freetype) is still to be validated.";

pub fn styles() -> Vec<Style> {
    vec![
        Style {
            id: "travel_cinematic", name: "Travel Cinematic", target_ms: 90_000, export_preset: "landscape",
            clip_min_ms: 3_000, clip_max_ms: 6_000, clip_ideal_ms: 4_500, transition_ms: 600,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Calm; wide, stable shots favoured",
            opening: "Establishing landscape shot", ending: "Calm scenery (sunset, sea, sky)",
            weights: [0.45, 0.4, 0.05, 0.1], favor: LANDSCAPES,
            open_with: &["aerial view", "mountain", "sea", "beach", "sky and clouds", "sunrise"], end_with: &["sunset", "sea", "sky and clouds", "lake"],
        },
        Style {
            id: "social_story", name: "Social Story / Reels", target_ms: 30_000, export_preset: "reels",
            clip_min_ms: 1_500, clip_max_ms: 3_000, clip_ideal_ms: 2_000, transition_ms: 0,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Fast cuts, vertical 9:16",
            opening: "Strongest clip first (hook)", ending: "People moment",
            weights: [0.4, 0.4, 0.0, 0.2], favor: &["people", "group of friends", "selfie", "food", "drinks", "fireworks", "concert", "sunset", "beach"],
            open_with: &[], end_with: PEOPLE,
        },
        Style {
            id: "memory_nostalgia", name: "Memory / Nostalgia", target_ms: 180_000, export_preset: "landscape",
            clip_min_ms: 3_000, clip_max_ms: 7_000, clip_ideal_ms: 5_000, transition_ms: 800,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Slow, chronological; memory value over image quality",
            opening: "Chronological start", ending: "Family / friends moment",
            weights: [0.2, 0.4, 0.2, 0.2], favor: &["family", "child", "group of friends", "people", "birthday party", "wedding", "food", "dog", "cat"],
            open_with: &[], end_with: &["family", "group of friends", "people", "child"],
        },
        Style {
            id: "travel_documentary", name: "Travel Documentary", target_ms: 300_000, export_preset: "landscape",
            clip_min_ms: 5_000, clip_max_ms: 15_000, clip_ideal_ms: 8_000, transition_ms: 300,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Longer clips that keep conversations whole",
            opening: "Establishing landscape shot", ending: "Chronological end",
            weights: [0.3, 0.25, 0.35, 0.1], favor: &["city street", "market", "temple", "church", "building", "restaurant", "mountain", "sea", "hiking trail", "road trip on a highway"],
            open_with: &["aerial view", "mountain", "city street", "sea"], end_with: &[],
        },
        Style {
            id: "clean_minimal", name: "Clean Minimal", target_ms: 60_000, export_preset: "landscape",
            clip_min_ms: 3_000, clip_max_ms: 5_000, clip_ideal_ms: 4_000, transition_ms: 0,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Even rhythm; the technically best footage",
            opening: "Chronological start", ending: "Chronological end",
            weights: [0.6, 0.25, 0.0, 0.15], favor: &[], open_with: &[], end_with: &[],
        },
    ]
}

pub fn style(id: &str) -> Result<Style, String> {
    styles().into_iter().find(|s| s.id == id).ok_or(format!("Unknown editing style: {id}"))
}

// ------------------------------------------------------------------ inputs

/// Everything the Editing Core knows about one library file.
pub struct Source {
    pub media: Media,
    pub scenes: Vec<ScoredScene>,
    pub tags: Option<MediaTags>,
    pub transcript: Option<Transcript>,
    pub favorite: bool,
    pub excluded: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub favorite: bool,
    pub excluded: bool,
}

pub fn marks(c: &Connection) -> Result<HashMap<i64, Mark>, String> {
    let mut s = c.prepare("SELECT media_id, favorite, excluded FROM media_marks").map_err(err)?;
    let rows = s.query_map([], |r| Ok((r.get(0)?, Mark { favorite: r.get(1)?, excluded: r.get(2)? }))).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

pub fn set_mark(c: &Connection, media_id: i64, mark: Mark) -> Result<(), String> {
    c.execute(
        "INSERT INTO media_marks (media_id, favorite, excluded) VALUES (?1, ?2, ?3)
         ON CONFLICT (media_id) DO UPDATE SET favorite = excluded.favorite, excluded = excluded.excluded",
        params![media_id, mark.favorite, mark.excluded],
    )
    .map_err(err)
    .map(drop)
}

/// Recording date (YYYY-MM-DD) used for ranges and chronological order; import date when unknown.
pub fn day(m: &Media) -> String {
    m.probe.source_created_at.as_deref().unwrap_or(&m.imported_at).chars().take(10).collect()
}

/// Library files recorded within [from, to] (inclusive YYYY-MM-DD, either optional), with their observations.
pub fn gather(c: &Connection, from: Option<&str>, to: Option<&str>) -> Result<Vec<Source>, String> {
    let marks = marks(c)?;
    let hashes = analysis::library_hashes(c)?;
    let mut out = vec![];
    for m in media::list(c)? {
        let d = day(&m);
        if from.is_some_and(|f| d.as_str() < f) || to.is_some_and(|t| d.as_str() > t) {
            continue;
        }
        let mark = marks.get(&m.id).copied().unwrap_or_default();
        out.push(Source {
            scenes: scoring::score(analysis::scenes(c, m.id)?, &hashes),
            tags: tagging::get(c, m.id)?,
            transcript: transcribe::get(c, m.id)?,
            favorite: mark.favorite,
            excluded: mark.excluded,
            media: m,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------ plan

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scores {
    pub technical: f64,
    pub semantic: f64,
    pub speech: f64,
    pub favorite: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanClip {
    pub media_id: i64,
    pub scene_index: i64,
    pub in_ms: i64,
    pub out_ms: i64,
    pub scores: Scores,
    /// Weighted rank used for selection (style weights × scores, diversity applied).
    pub rank: f64,
    /// Main visual concepts (Vietnamese labels).
    pub tags: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub style: String,
    pub target_ms: i64,
    pub total_ms: i64,
    pub transition_ms: i64,
    pub export_preset: String,
    pub clips: Vec<PlanClip>,
    /// Why library files were not used.
    pub skipped: Vec<String>,
    pub candidates: usize,
}

const SCENE_TAG_MIN: f64 = 0.1;

fn overlap(a0: i64, a1: i64, b0: i64, b1: i64) -> i64 {
    (a1.min(b1) - a0.max(b0)).max(0)
}

/// Candidate windows inside one scene: the whole scene when it fits the style, else consecutive
/// `ideal`-length windows (the remainder is dropped when shorter than the minimum).
fn windows(start: i64, end: i64, st: &Style) -> Vec<(i64, i64)> {
    let len = end - start;
    if len < st.clip_min_ms {
        return vec![];
    }
    if len <= st.clip_max_ms {
        return vec![(start, end)];
    }
    let mut out = vec![];
    let mut a = start;
    while end - a >= st.clip_min_ms {
        let b = (a + st.clip_ideal_ms).min(end);
        out.push((a, b));
        a = b;
    }
    out
}

struct Candidate {
    clip: PlanClip,
    order: (String, i64, i64),
    dominant: Option<String>,
    duplicates: Vec<(i64, i64)>,
    opens: f64,
    ends: f64,
}

fn tag_score(tags: &[tagging::TagScore], names: &[&str]) -> f64 {
    tags.iter().filter(|t| names.contains(&t.en.as_str())).map(|t| t.score).sum::<f64>().min(1.0)
}

/// Technical score from the scene's sample frames inside [a, b) only (long single-shot clips vary a lot
/// within one scene); the scene score when no sample falls inside.
fn window_technical(sc: &ScoredScene, a: i64, b: i64) -> f64 {
    let samples: Vec<_> = sc.scene.samples.iter().filter(|s| s.at_ms >= a && s.at_ms < b).cloned().collect();
    if samples.is_empty() {
        return sc.technical.total;
    }
    let scene = analysis::Scene { samples, start_ms: a, end_ms: b, ..sc.scene.clone() };
    scoring::score(vec![scene], &[])[0].technical.total
}

fn candidates(src: &Source, st: &Style) -> Vec<Candidate> {
    let mut out = vec![];
    for sc in &src.scenes {
        for (a, b) in windows(sc.scene.start_ms, sc.scene.end_ms, st) {
            let tags = src.tags.as_ref().map(|t| tagging::summarize(t, a, b, SCENE_TAG_MIN)).unwrap_or_default();
            let semantic = match (&src.tags, st.favor.is_empty()) {
                (None, _) => 0.5, // not tagged: neutral
                (Some(_), true) => tags.first().map_or(0.0, |t| t.score),
                (Some(_), false) => 0.5 * tags.first().map_or(0.0, |t| t.score) + 0.5 * (2.0 * tag_score(&tags, st.favor)).min(1.0),
            };
            let speech = src.transcript.as_ref().map_or(0.0, |t| {
                t.segments.iter().map(|s| overlap(a, b, s.start_ms, s.end_ms)).sum::<i64>() as f64 / (b - a) as f64
            });
            let scores = Scores { technical: window_technical(sc, a, b), semantic, speech: speech.min(1.0), favorite: if src.favorite { 1.0 } else { 0.0 } };
            let w = st.weights;
            let rank = w[0] * scores.technical + w[1] * scores.semantic + w[2] * scores.speech + w[3] * scores.favorite;
            let mut reasons = vec![format!("technical {:.0}", scores.technical * 100.0)];
            if let Some(t) = tags.first() {
                reasons.push(format!("{} ({:.0}%)", t.vi, t.score * 100.0));
            }
            if scores.speech >= 0.3 {
                reasons.push(format!("speech {:.0}%", scores.speech * 100.0));
            }
            if src.favorite {
                reasons.push("favorite".into());
            }
            reasons.extend(sc.flags.iter().map(|f| f.to_string()));
            out.push(Candidate {
                order: (day(&src.media), src.media.id, a),
                dominant: tags.first().map(|t| t.en.clone()),
                duplicates: sc.duplicates.iter().map(|d| (d.media_id, d.scene_index)).collect(),
                opens: tag_score(&tags, st.open_with),
                ends: tag_score(&tags, st.end_with),
                clip: PlanClip {
                    media_id: src.media.id,
                    scene_index: sc.scene.index,
                    in_ms: a,
                    out_ms: b,
                    scores,
                    rank,
                    tags: tags.iter().take(3).map(|t| t.vi.clone()).collect(),
                    reasons,
                },
            });
        }
    }
    out
}

/// Diversity: each already chosen clip with the same dominant concept ×0.85, from the same file ×0.9,
/// and ×0.6 for each chosen clip of the same file closer than 3 ideal clip lengths (spreads picks over
/// long continuous shots instead of taking consecutive windows).
// ponytail: greedy O(n²) over candidates; fine for a few thousand windows, revisit for whole-year libraries.
fn diversity(c: &Candidate, chosen: &[&Candidate], ideal_ms: i64) -> f64 {
    let same_tag = chosen.iter().filter(|x| x.dominant.is_some() && x.dominant == c.dominant).count() as i32;
    let same_media: Vec<_> = chosen.iter().filter(|x| x.clip.media_id == c.clip.media_id).collect();
    let near = same_media
        .iter()
        .filter(|x| (x.clip.in_ms - c.clip.out_ms).max(c.clip.in_ms - x.clip.out_ms) < 3 * ideal_ms)
        .count() as i32;
    0.85f64.powi(same_tag) * 0.9f64.powi(same_media.len() as i32) * 0.6f64.powi(near)
}

/// Distance (ms) from `c` to the nearest chosen clip of the same file; MAX when none.
fn distance(c: &Candidate, chosen: &[&Candidate]) -> i64 {
    chosen
        .iter()
        .filter(|x| x.clip.media_id == c.clip.media_id)
        .map(|x| (x.clip.in_ms + x.clip.out_ms - c.clip.in_ms - c.clip.out_ms).abs() / 2)
        .min()
        .unwrap_or(i64::MAX)
}

pub fn plan(st: &Style, sources: &[Source]) -> Plan {
    let mut skipped = vec![];
    let mut pool = vec![];
    for s in sources {
        let name = s.media.path.rsplit(['\\', '/']).next().unwrap_or(&s.media.path).to_string();
        if s.excluded {
            skipped.push(format!("{name}: excluded by you"));
        } else if s.scenes.is_empty() {
            skipped.push(format!("{name}: not analysed yet"));
        } else {
            let c = candidates(s, st);
            if c.is_empty() {
                skipped.push(format!("{name}: no scene long enough for this style"));
            }
            pool.extend(c);
        }
    }
    let candidates = pool.len();

    let mut chosen: Vec<&Candidate> = vec![];
    let mut used: HashSet<(i64, i64)> = HashSet::new(); // (media, scene) already represented
    let mut total = 0;
    while total + st.clip_min_ms <= st.target_ms {
        let best = pool
            .iter()
            .filter(|c| !chosen.iter().any(|x| x.clip.media_id == c.clip.media_id && overlap(x.clip.in_ms, x.clip.out_ms, c.clip.in_ms, c.clip.out_ms) > 0))
            .filter(|c| !c.duplicates.iter().any(|d| used.contains(d)))
            .map(|c| (c, c.clip.rank * diversity(c, &chosen, st.clip_ideal_ms), distance(c, &chosen)))
            // Ties: farthest from the clips already taken from that file (even coverage), then time order.
            .max_by(|(a, ra, da), (b, rb, db)| ra.total_cmp(rb).then(da.cmp(db)).then(b.order.cmp(&a.order)));
        let Some((c, _, _)) = best else { break };
        total += c.clip.out_ms - c.clip.in_ms;
        used.insert((c.clip.media_id, c.clip.scene_index));
        chosen.push(c);
    }

    // Story structure: chronological, then the style's opening and ending clips.
    chosen.sort_by(|a, b| a.order.cmp(&b.order));
    let pick = |chosen: &[&Candidate], f: &dyn Fn(&Candidate) -> f64| -> Option<usize> {
        chosen.iter().enumerate().filter(|(_, c)| f(c) > 0.2).max_by(|(_, a), (_, b)| f(a).total_cmp(&f(b))).map(|(i, _)| i)
    };
    if !st.open_with.is_empty() {
        if let Some(i) = pick(&chosen, &|c| c.opens) {
            let c = chosen.remove(i);
            chosen.insert(0, c);
        }
    } else if st.opening.starts_with("Strongest") && !chosen.is_empty() {
        let i = (0..chosen.len()).max_by(|&a, &b| chosen[a].clip.rank.total_cmp(&chosen[b].clip.rank)).unwrap();
        let c = chosen.remove(i);
        chosen.insert(0, c);
    }
    if !st.end_with.is_empty() && chosen.len() > 2 {
        if let Some(i) = pick(&chosen[1..], &|c| c.ends).map(|i| i + 1) {
            let c = chosen.remove(i);
            chosen.push(c);
        }
    }

    let clips: Vec<PlanClip> = chosen.into_iter().map(|c| c.clip.clone()).collect();
    let n = clips.len() as i64;
    Plan {
        style: st.id.into(),
        target_ms: st.target_ms,
        total_ms: clips.iter().map(|c| c.out_ms - c.in_ms).sum::<i64>() - (n - 1).max(0) * st.transition_ms,
        transition_ms: st.transition_ms,
        export_preset: st.export_preset.into(),
        clips,
        skipped,
        candidates,
    }
}

// ------------------------------------------------------------------ storage

pub fn save(c: &Connection, project_id: i64, p: &Plan) -> Result<(), String> {
    c.execute(
        "INSERT INTO editing_plans (project_id, style, plan_json) VALUES (?1, ?2, ?3)
         ON CONFLICT (project_id) DO UPDATE SET style = excluded.style, plan_json = excluded.plan_json,
            created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![project_id, p.style, serde_json::to_string(p).unwrap()],
    )
    .map_err(err)
    .map(drop)
}

pub fn load(c: &Connection, project_id: i64) -> Result<Option<Plan>, String> {
    let j: Option<String> = c
        .query_row("SELECT plan_json FROM editing_plans WHERE project_id = ?1", [project_id], |r| r.get(0))
        .optional()
        .map_err(err)?;
    Ok(j.and_then(|j| serde_json::from_str(&j).ok()))
}

/// Replaces the project timeline with the plan's clips (in `keep` order: indexes into plan.clips).
/// The user stays in control: the timeline remains editable afterwards.
pub fn apply(c: &Connection, project_id: i64, keep: &[usize]) -> Result<usize, String> {
    let p = load(c, project_id)?.ok_or("Generate a plan first.")?;
    let tx = c.unchecked_transaction().map_err(err)?;
    tx.execute("DELETE FROM project_clips WHERE project_id = ?1", [project_id]).map_err(err)?;
    let mut n = 0;
    for &i in keep {
        let clip = p.clips.get(i).ok_or(format!("No clip #{i} in the plan"))?;
        crate::timeline::add(&tx, project_id, clip.media_id, clip.in_ms, clip.out_ms)?;
        n += 1;
    }
    tx.commit().map_err(err)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Sample, Scene};
    use crate::media::Probe;
    use crate::scoring::Technical;
    use crate::tagging::{TagName, TagSample};
    use crate::transcribe::Segment;

    fn media(id: i64, day: &str, dur: i64) -> Media {
        Media {
            id, path: format!(r"E:\v{id}.mp4"), size: 1,
            probe: Probe { duration_ms: dur, width: 1920, height: 1080, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
                audio_codec: Some("aac".into()), audio_streams: 1, bitrate: None, source_created_at: Some(format!("{day}T10:00:00Z")) },
            imported_at: "2026-11-01T00:00:00Z".into(), thumb: None, proxy: None, scene_count: None, transcribed: false, tags: None, mark: Mark::default(),
        }
    }

    fn scene(media_id: i64, index: i64, start: i64, end: i64, total: f64) -> ScoredScene {
        ScoredScene {
            scene: Scene { id: index, media_id, index, start_ms: start, end_ms: end, shake: 0.0, frame_path: None, samples: Vec::<Sample>::new() },
            technical: Technical { sharpness: total, exposure: total, brightness: total, contrast: total, stability: total, total },
            flags: vec![], duplicates: vec![], tags: vec![],
        }
    }

    fn tags(per_2s: &[usize], probs: f64) -> MediaTags {
        MediaTags {
            device: "CPU".into(),
            tags: [("sea", "biển"), ("people", "con người"), ("bicycle", "xe đạp")].iter().map(|(e, v)| TagName { en: (*e).into(), vi: (*v).into() }).collect(),
            samples: per_2s.iter().enumerate().map(|(i, &k)| TagSample { at_ms: i as i64 * 2000, tags: vec![(k, probs)] }).collect(),
        }
    }

    fn src(m: Media, scenes: Vec<ScoredScene>) -> Source {
        Source { media: m, scenes, tags: None, transcript: None, favorite: false, excluded: false }
    }

    #[test]
    fn windows_follow_style() {
        let st = style("clean_minimal").unwrap(); // 3–5 s, ideal 4 s
        assert!(windows(0, 2_000, &st).is_empty());
        assert_eq!(windows(0, 4_500, &st), [(0, 4_500)]);
        assert_eq!(windows(0, 13_000, &st), [(0, 4_000), (4_000, 8_000), (8_000, 12_000)], "1 s remainder dropped");
        assert_eq!(windows(0, 15_000, &st).last(), Some(&(12_000, 15_000)), "3 s remainder kept");
    }

    #[test]
    fn plan_respects_target_exclusion_and_is_deterministic() {
        let st = style("clean_minimal").unwrap(); // 60 s target
        let sources = vec![
            src(media(1, "2026-02-20", 300_000), vec![scene(1, 0, 0, 300_000, 0.9)]),
            src(media(2, "2026-02-21", 300_000), vec![scene(2, 0, 0, 300_000, 0.5)]),
            Source { excluded: true, ..src(media(3, "2026-02-22", 300_000), vec![scene(3, 0, 0, 300_000, 1.0)]) },
            src(media(4, "2026-02-23", 10_000), vec![]),
        ];
        let p = plan(&st, &sources);
        assert!(p.total_ms <= st.target_ms && p.total_ms > st.target_ms - st.clip_max_ms, "total {}", p.total_ms);
        assert!(p.clips.iter().all(|c| c.media_id != 3), "excluded file never used");
        assert!(p.clips.iter().all(|c| (st.clip_min_ms..=st.clip_max_ms).contains(&(c.out_ms - c.in_ms))));
        assert!(p.skipped.iter().any(|s| s.contains("v3.mp4: excluded")) && p.skipped.iter().any(|s| s.contains("v4.mp4: not analysed")));
        assert!(p.clips.iter().any(|c| c.media_id == 2), "diversity gives the weaker file a share");
        assert!(p.clips.windows(2).all(|w| (w[0].media_id, w[0].in_ms) < (w[1].media_id, w[1].in_ms)), "chronological");
        assert_eq!(serde_json::to_string(&plan(&st, &sources).clips).unwrap(), serde_json::to_string(&p.clips).unwrap(), "deterministic");
    }

    #[test]
    fn semantics_speech_favorites_and_structure() {
        let st = style("travel_cinematic").unwrap();
        // File 1: bicycle, then sea at 8–16 s. File 2: people talking (favorite).
        let mut a = src(media(1, "2026-02-20", 16_000), vec![scene(1, 0, 0, 16_000, 0.7)]);
        a.tags = Some(tags(&[2, 2, 2, 2, 0, 0, 0, 0], 0.9));
        let mut b = src(media(2, "2026-02-19", 9_000), vec![scene(2, 0, 0, 9_000, 0.7)]);
        b.tags = Some(tags(&[1, 1, 1, 1, 1], 0.9));
        b.favorite = true;
        b.transcript = Some(Transcript { media_id: 2, model_id: "w".into(), language: Some("vi".into()),
            segments: vec![Segment { start_ms: 0, end_ms: 4_000, text: "xin chào".into() }], created_at: String::new() });
        let p = plan(&st, &[a, b]);
        let sea = p.clips.iter().find(|c| c.media_id == 1 && c.in_ms >= 8_000).expect("sea window chosen");
        let bike = p.clips.iter().find(|c| c.media_id == 1 && c.in_ms < 4_000);
        assert!(bike.is_none_or(|b| sea.scores.semantic > b.scores.semantic), "landscape style prefers sea");
        assert_eq!((p.clips[0].media_id, p.clips[0].tags[0].as_str()), (1, "biển"), "opens with the establishing sea shot, not chronology");
        let talk = p.clips.iter().find(|c| c.media_id == 2 && c.in_ms == 0).unwrap();
        assert!(talk.scores.speech > 0.8 && talk.scores.favorite == 1.0 && talk.reasons.iter().any(|r| r == "favorite"));
        assert_eq!(p.total_ms, p.clips.iter().map(|c| c.out_ms - c.in_ms).sum::<i64>() - (p.clips.len() as i64 - 1) * 600);
    }

    #[test]
    fn long_single_shot_is_sampled_spread_out() {
        let st = style("clean_minimal").unwrap(); // 60 s from one 300 s continuous shot
        let p = plan(&st, &[src(media(1, "2026-02-20", 300_000), vec![scene(1, 0, 0, 300_000, 0.8)])]);
        assert!(p.clips.len() >= 12, "{}", p.clips.len());
        assert!(p.clips.windows(2).all(|w| w[1].in_ms - w[0].out_ms >= 2 * st.clip_ideal_ms), "no consecutive windows: {:?}",
            p.clips.iter().map(|c| c.in_ms / 1000).collect::<Vec<_>>());
        assert!(p.clips.last().unwrap().out_ms > 240_000, "covers the whole shot");
    }

    #[test]
    fn duplicates_are_not_picked_twice() {
        let st = style("clean_minimal").unwrap();
        let mut s1 = scene(1, 0, 0, 4_000, 0.9);
        s1.duplicates = vec![scoring::Duplicate { media_id: 2, scene_index: 0, distance: 2 }];
        let mut s2 = scene(2, 0, 0, 4_000, 0.8);
        s2.duplicates = vec![scoring::Duplicate { media_id: 1, scene_index: 0, distance: 2 }];
        let p = plan(&st, &[src(media(1, "2026-01-01", 4_000), vec![s1]), src(media(2, "2026-01-02", 4_000), vec![s2])]);
        assert_eq!(p.clips.iter().map(|c| c.media_id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn marks_plans_and_apply_round_trip() {
        let c = crate::db::open_in_memory();
        let probe = media(0, "2026-02-20", 300_000).probe;
        let m = media::upsert(&c, r"E:\a.mp4", 1, None, &probe, "{}").unwrap();
        set_mark(&c, m.id, Mark { favorite: true, excluded: false }).unwrap();
        assert!(marks(&c).unwrap()[&m.id].favorite);
        let p = crate::projects::create(&c, "Trip").unwrap();
        assert!(apply(&c, p.id, &[0]).is_err(), "no plan yet");
        let sources = vec![src(m.clone(), vec![scene(m.id, 0, 0, 300_000, 0.8)])];
        let plan = plan(&style("social_story").unwrap(), &sources);
        save(&c, p.id, &plan).unwrap();
        assert_eq!(load(&c, p.id).unwrap().unwrap().clips, plan.clips);
        assert_eq!(apply(&c, p.id, &[2, 0]).unwrap(), 2);
        let tl = crate::timeline::list(&c, p.id).unwrap();
        assert_eq!((tl[0].in_ms, tl[1].in_ms), (plan.clips[2].in_ms, plan.clips[0].in_ms), "user order kept");
        assert!(apply(&c, p.id, &[99]).is_err());
        assert_eq!(crate::timeline::list(&c, p.id).unwrap().len(), 2, "failed apply rolls back");
        assert_eq!(gather(&c, Some("2026-03-01"), None).unwrap().len(), 0);
        assert_eq!(gather(&c, Some("2026-02-01"), Some("2026-02-28")).unwrap().len(), 1);
    }
}

/// Manual check on a real library: `MEMORA_DB=<copy of memora.db> cargo test real_library_plans -- --ignored --nocapture`.
/// Opens a COPY (migrations may run on it), never the live database.
#[cfg(test)]
#[test]
#[ignore]
fn real_library_plans() {
    let path = std::env::var("MEMORA_DB").expect("set MEMORA_DB to a copy of memora.db");
    let c = crate::db::open(std::path::Path::new(&path)).unwrap();
    let sources = gather(&c, None, None).unwrap();
    for st in styles() {
        let p = plan(&st, &sources);
        println!("== {} target {}s → {} clips, {:.1}s, {} candidates, skipped {:?}", st.name, st.target_ms / 1000, p.clips.len(), p.total_ms as f64 / 1000.0, p.candidates, p.skipped);
        for c in p.clips.iter().take(6) {
            println!("   #{} {:>6.1}-{:<6.1} rank {:.2} {:?}", c.media_id, c.in_ms as f64 / 1000.0, c.out_ms as f64 / 1000.0, c.rank, c.reasons);
        }
    }
}
