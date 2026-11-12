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
    /// Export look applied with the plan (export.rs TRANSITIONS / COLORS); editable before export.
    pub transition: &'static str,
    pub color: &'static str,
    /// Burn speech subtitles by default (talk-driven styles).
    pub captions: bool,
}

pub(crate) const LANDSCAPES: &[&str] = &["aerial view", "mountain", "sea", "beach", "sky and clouds", "sunrise", "lake", "waterfall", "rice field", "forest"];
const JOURNEY: &[&str] = &[
    "aerial view", "mountain", "sky and clouds", "sunrise", "sunset", "lake", "river", "waterfall", "rice field", "forest",
    "hiking trail", "big tree", "bicycle", "group of friends", "boat", "forest trail", "small bridge", "slope",
];
const PEOPLE: &[&str] =&["people", "group of friends", "family", "child", "selfie"];
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
            transition: "fade", color: "cinematic", captions: false,
        },
        Style {
            id: "social_story", name: "Social Story / Reels", target_ms: 30_000, export_preset: "reels",
            clip_min_ms: 1_500, clip_max_ms: 3_000, clip_ideal_ms: 2_000, transition_ms: 0,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Fast cuts, vertical 9:16",
            opening: "Strongest clip first (hook)", ending: "People moment",
            weights: [0.4, 0.4, 0.0, 0.2], favor: &["people", "group of friends", "selfie", "food", "drinks", "fireworks", "concert", "sunset", "beach"],
            open_with: &[], end_with: PEOPLE,
            transition: "fade", color: "vivid", captions: false,
        },
        Style {
            id: "memory_nostalgia", name: "Memory / Nostalgia", target_ms: 180_000, export_preset: "landscape",
            clip_min_ms: 3_000, clip_max_ms: 7_000, clip_ideal_ms: 5_000, transition_ms: 800,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Slow, chronological; memory value over image quality",
            opening: "Chronological start", ending: "Family / friends moment",
            weights: [0.2, 0.4, 0.2, 0.2], favor: &["family", "child", "group of friends", "people", "birthday party", "wedding", "food", "dog", "cat"],
            open_with: &[], end_with: &["family", "group of friends", "people", "child"],
            transition: "fade", color: "warm", captions: false,
        },
        Style {
            id: "travel_documentary", name: "Travel Documentary", target_ms: 300_000, export_preset: "landscape",
            clip_min_ms: 5_000, clip_max_ms: 15_000, clip_ideal_ms: 8_000, transition_ms: 300,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Longer clips that keep conversations whole",
            opening: "Establishing landscape shot", ending: "Chronological end",
            weights: [0.3, 0.25, 0.35, 0.1], favor: &["city street", "market", "temple", "church", "building", "restaurant", "mountain", "sea", "hiking trail", "road trip on a highway"],
            open_with: &["aerial view", "mountain", "city street", "sea"], end_with: &[],
            transition: "fade", color: "none", captions: true,
        },
        Style {
            // Owner request 2026-11-08 (Nam Cát Tiên trip): a long recap of a whole trip that flows, no choppy cuts.
            id: "journey_recap", name: "Journey Recap", target_ms: 300_000, export_preset: "landscape",
            clip_min_ms: 4_000, clip_max_ms: 9_000, clip_ideal_ms: 6_000, transition_ms: 700,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Chronological, unhurried; the whole trip in order",
            opening: "Chronological start", ending: "Chronological end",
            weights: [0.4, 0.35, 0.1, 0.15], favor: JOURNEY,
            open_with: &[], end_with: &[],
            // "auto": short cross dissolves inside a shot, longer ones between places, dip to black between parts of the day.
            transition: "auto", color: "warm", captions: false,
        },
        Style {
            id: "clean_minimal", name: "Clean Minimal", target_ms: 60_000, export_preset: "landscape",
            clip_min_ms: 3_000, clip_max_ms: 5_000, clip_ideal_ms: 4_000, transition_ms: 0,
            music: NO_MUSIC, titles: NO_TITLES, pacing: "Even rhythm; the technically best footage",
            opening: "Chronological start", ending: "Chronological end",
            weights: [0.6, 0.25, 0.0, 0.15], favor: &[], open_with: &[], end_with: &[],
            transition: "fade", color: "none", captions: false,
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

/// Recording time (ISO 8601, UTC) for chronological order; import time when unknown.
fn recorded(m: &Media) -> String {
    m.probe.source_created_at.clone().unwrap_or_else(|| m.imported_at.clone())
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
    /// Weighted rank (style weights × scores, bad-shot penalty). Diversity is applied during selection, not stored.
    pub rank: f64,
    /// Main visual concepts (Vietnamese labels).
    pub tags: Vec<String>,
    pub reasons: Vec<String>,
    /// A still photo (shown `out_ms - in_ms`); `media_id` is then −photo id (see `export::timeline_joins`).
    #[serde(default)]
    pub photo_id: Option<i64>,
}

/// The user's own requests on top of a style (concept names match tag `en` or `vi`, case-insensitive).
/// Stored with the plan; applying the plan honours the look / music / text wishes too.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Wishes {
    /// Concepts to show more of: rank × up to 1.5.
    pub prefer: Vec<String>,
    /// Concepts to show less of: rank × 0.5 where they are on screen.
    pub less: Vec<String>,
    /// Concepts to leave out: windows where one scores ≥ `AVOID_MIN` are not used.
    pub avoid: Vec<String>,
    /// Length instead of the style's target.
    pub target_ms: Option<i64>,
    /// Export preset instead of the style's ("reels" = vertical 9:16, centre crop).
    pub preset: Option<String>,
    /// Colour look (export::COLORS) instead of the style's.
    pub color: Option<String>,
    pub title: Option<String>,
    pub ending: Option<String>,
    /// Speech subtitles on / off instead of the style's default.
    pub captions: Option<bool>,
    /// "none" = no music; "low" | "medium" | "high" | "any" = the best-suited library track of that energy.
    pub music: Option<String>,
    /// Photos between the video clips: "none" | "fewer" | "yes" | "more" (None = like "yes" when there are photos).
    pub photos: Option<String>,
    /// Seconds each photo stays on screen (ms).
    pub photo_ms: Option<i64>,
    /// The sentence these wishes were read from (request box), kept for the record.
    pub request: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub style: String,
    #[serde(default)]
    pub wishes: Wishes,
    pub target_ms: i64,
    pub total_ms: i64,
    pub transition_ms: i64,
    pub export_preset: String,
    pub clips: Vec<PlanClip>,
    /// Why library files were not used.
    pub skipped: Vec<String>,
    pub candidates: usize,
    /// Title proposal from the main concept and the recording dates (the user edits it before export).
    #[serde(default)]
    pub suggested_title: String,
    /// How the plan was built where it is not obvious (length chosen from the footage, scenes kept for coverage).
    #[serde(default)]
    pub notes: Vec<String>,
}

const SCENE_TAG_MIN: f64 = 0.1;
/// Rank penalty per unit of bad-shot probability (summed over detectors), capped at 80 %.
const BAD_SHOT_WEIGHT: f64 = 2.0;

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
    hash: Option<String>,
    scene_ms: i64,
    duplicates: Vec<(i64, i64)>,
    opens: f64,
    ends: f64,
}

fn tag_score(tags: &[tagging::TagScore], names: &[&str]) -> f64 {
    tags.iter().filter(|t| names.contains(&t.en.as_str())).map(|t| t.score).sum::<f64>().min(1.0)
}

/// A window where an avoided concept reaches this score is left out (tag summaries start at 0.1).
const AVOID_MIN: f64 = 0.25;
/// Most a preferred concept can raise a window's rank (×1.5 when the concept fills the window).
const PREFER_BOOST: f64 = 0.5;
/// Most a "less of" concept can lower a window's rank (×0.5 when the concept fills the window).
const LESS_CUT: f64 = 0.5;

/// Camera cue (OpenCLIP model v3): the road and landscape ahead are visible, as opposed to the camera tilted down
/// at the handlebars and the ground (owner feedback 2026-11-08: "the shots where I sat up and filmed the whole
/// road, not the same shots of the camera pointing down").
const VIEW_CUE: &str = "view ahead";
/// Below this the camera points down at the handlebars / ground and the window is left out (checked on 850
/// frames of the owner's footage; initial value).
const VIEW_DOWN: f64 = 0.1;

/// Rank factor from the "view ahead" cue for the windows kept: ×0.67 just above `VIEW_DOWN`, ×1.3 for a full
/// view (linear). Initial values from one library; walking, ferry and river shots score 0.6–0.9.
fn view_factor(v: f64) -> f64 {
    0.6 + 0.7 * v.clamp(0.0, 1.0)
}

/// Scene cues (OpenCLIP model v4, owner feedback 2026-11-11: "no forest paths, uphill, small bridges, downhill
/// through the leaves"): (cue, threshold), checked in this order. Handlebars are always in frame on the owner's
/// rides, so "view ahead" alone scored these < 0.1 and left them out. Thresholds from 1 010 frames of one library
/// (initial values): forest trail ≥ 0.9 = dirt path / forest road (paved open road 0.01–0.4); small bridge and slope
/// also fire on river views and the big-tree clearing, so they count only on a forest trail.
pub(crate) const SCENE_CUES: &[(&str, f64)] = &[("small bridge", 0.6), ("slope", 0.95), ("forest trail", 0.9)];
const FOREST_TRAIL: &str = "forest trail";
/// Main concepts a scene cue may replace (a ride or walk); a big tree or people on a trail keep their own concept.
const ON_THE_MOVE: &[&str] = &["bicycle", "motorbike", "hiking trail", "forest", "park", "road trip on a highway"];

fn named(t: &tagging::TagScore, names: &[String]) -> bool {
    names.iter().any(|n| n.trim().eq_ignore_ascii_case(&t.en) || n.trim().to_lowercase() == t.vi.to_lowercase())
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

/// Moves window edges out of speech so no sentence is cut: extend to the whole utterance when the clip
/// stays ≤ `speech_cap`, else shrink to exclude it, else drop the window (owner feedback: a sentence cut
/// in half is worse than no sentence). Edges never leave the scene.
fn fit_speech(mut a: i64, mut b: i64, speech: &[(i64, i64)], scene: (i64, i64), st: &Style) -> Option<(i64, i64)> {
    let cap = st.clip_max_ms * 3;
    for _ in 0..4 {
        let cut_a = speech.iter().find(|&&(s, e)| s < a && a < e).copied();
        let cut_b = speech.iter().find(|&&(s, e)| s < b && b < e).copied();
        match (cut_a, cut_b) {
            (None, None) => return (b - a >= st.clip_min_ms).then_some((a, b)),
            (Some((s, e)), _) => {
                if s >= scene.0 && b - s <= cap { a = s } else if b - e >= st.clip_min_ms { a = e } else { return None }
            }
            (None, Some((s, e))) => {
                if e <= scene.1 && e - a <= cap { b = e } else if s - a >= st.clip_min_ms { b = s } else { return None }
            }
        }
    }
    None
}

/// dHash of the sample frame nearest the window centre (visual "what is on screen").
fn window_hash(sc: &ScoredScene, a: i64, b: i64) -> Option<String> {
    let mid = (a + b) / 2;
    sc.scene.samples.iter().min_by_key(|s| (s.at_ms - mid).abs()).map(|s| s.dhash.clone())
}

/// Candidate windows of one file, and how many were left out because of `wishes.avoid` and because the
/// camera points down at the handlebars / ground ("view ahead" cue below `VIEW_DOWN`).
fn candidates(src: &Source, st: &Style, wishes: &Wishes) -> (Vec<Candidate>, usize, usize) {
    let mut out = vec![];
    let (mut avoided, mut tilted) = (0, 0);
    let speech: Vec<(i64, i64)> = src
        .transcript
        .as_ref()
        .map_or(vec![], |t| t.segments.iter().filter(|s| s.timing_plausible()).map(|s| (s.start_ms, s.end_ms)).collect());
    for sc in &src.scenes {
        let mut seen = HashSet::new();
        let fitted = windows(sc.scene.start_ms, sc.scene.end_ms, st)
            .into_iter()
            .filter_map(|(a, b)| fit_speech(a, b, &speech, (sc.scene.start_ms, sc.scene.end_ms), st))
            .filter(|w| seen.insert(*w));
        for (a, b) in fitted.collect::<Vec<_>>() {
            let all = src.tags.as_ref().map(|t| tagging::summarize(t, a, b, SCENE_TAG_MIN)).unwrap_or_default();
            if all.iter().any(|t| t.score >= AVOID_MIN && named(t, &wishes.avoid)) {
                avoided += 1;
                continue;
            }
            let cue = |name: &str| src.tags.as_ref().and_then(|t| tagging::cue(t, name, a, b));
            let view = cue(VIEW_CUE);
            let forest = cue(FOREST_TRAIL).unwrap_or(0.0);
            let on_trail = SCENE_CUES.iter().any(|&(n, min)| n == FOREST_TRAIL && forest >= min);
            // Owner feedback 2026-11-08: tilted-down riding shots are left out, not used to fill the target —
            // except on forest trails (2026-11-11), where the trail itself is the subject.
            if view.is_some_and(|v| v < VIEW_DOWN) && !on_trail {
                tilted += 1;
                continue;
            }
            let (bad, mut tags): (Vec<_>, Vec<_>) = all.into_iter().partition(|t| t.negative);
            // A ride or walk on a trail is shown as its scene (bridge / slope / forest trail): its own concept for the
            // share cap, diversity and the request box, instead of one more "bicycle" clip.
            let scene = SCENE_CUES
                .iter()
                .filter(|_| on_trail && tags.first().is_none_or(|t| ON_THE_MOVE.contains(&t.en.as_str())))
                .find_map(|&(n, min)| cue(n).filter(|&s| s >= min).map(|s| (n, s)))
                .and_then(|(n, s)| src.tags.as_ref()?.cues.iter().find(|c| c.en == n).map(|c| tagging::TagScore { en: c.en.clone(), vi: c.vi.clone(), score: s, negative: false }));
            if let Some(s) = scene {
                tags.insert(0, s);
            }
            let wanted: Vec<_> = tags.iter().filter(|t| named(t, &wishes.prefer)).collect();
            let prefer = 1.0 + PREFER_BOOST * (2.0 * wanted.iter().map(|t| t.score).sum::<f64>()).min(1.0);
            let fewer: Vec<_> = tags.iter().filter(|t| named(t, &wishes.less)).collect();
            let prefer = prefer * (1.0 - LESS_CUT * (2.0 * fewer.iter().map(|t| t.score).sum::<f64>()).min(1.0));
            let semantic = match (&src.tags, st.favor.is_empty()) {
                (None, _) => 0.5, // not tagged: neutral
                (Some(_), true) => tags.first().map_or(0.0, |t| t.score),
                (Some(_), false) => 0.5 * tags.first().map_or(0.0, |t| t.score) + 0.5 * (2.0 * tag_score(&tags, st.favor)).min(1.0),
            };
            let speech = speech.iter().map(|&(s, e)| overlap(a, b, s, e)).sum::<i64>() as f64 / (b - a) as f64;
            let scores = Scores { technical: window_technical(sc, a, b), semantic, speech: speech.min(1.0), favorite: if src.favorite { 1.0 } else { 0.0 } };
            let w = st.weights;
            // Bad shot (finger over the lens, accidental ground shot…): up to −80 %, never chosen as a hook.
            let bad_score: f64 = bad.iter().map(|t| t.score).sum();
            let rank = (w[0] * scores.technical + w[1] * scores.semantic + w[2] * scores.speech + w[3] * scores.favorite)
                * (1.0 - (BAD_SHOT_WEIGHT * bad_score).min(0.8))
                * prefer
                // A forest trail counts as a partial view (×1.0 at 100 %): raised-camera shots still rank first.
                * view.map_or(1.0, |v| view_factor(v.max(if on_trail { 0.57 * forest } else { 0.0 })));
            let mut reasons = vec![format!("technical {:.0}", scores.technical * 100.0)];
            if let Some(v) = view.filter(|&v| v >= 0.5) {
                reasons.push(format!("thấy đường phía trước {:.0}%", v * 100.0));
            }
            if let Some(t) = tags.first() {
                reasons.push(format!("{} ({:.0}%)", t.vi, t.score * 100.0));
            }
            if let Some(t) = wanted.first() {
                reasons.push(format!("wanted: {} ({:.0}%)", t.vi, t.score * 100.0));
            }
            if let Some(t) = fewer.first() {
                reasons.push(format!("less wanted: {} ({:.0}%)", t.vi, t.score * 100.0));
            }
            if scores.speech >= 0.3 {
                reasons.push(format!("speech {:.0}%, sentences kept whole", scores.speech * 100.0));
            }
            if src.favorite {
                reasons.push("favorite".into());
            }
            reasons.extend(sc.flags.iter().map(|f| f.to_string()));
            reasons.extend(bad.iter().filter(|t| t.score >= 0.15).map(|t| format!("⚠ {} ({:.0}%)", t.vi, t.score * 100.0)));
            out.push(Candidate {
                // Full recording time, so clips of one day play in the order they were filmed.
                order: (recorded(&src.media), src.media.id, a),
                dominant: tags.first().map(|t| t.en.clone()),
                hash: window_hash(sc, a, b),
                scene_ms: sc.scene.end_ms - sc.scene.start_ms,
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
                    photo_id: None,
                },
            });
        }
    }
    (out, avoided, tilted)
}

/// Diversity: each already chosen clip with the same dominant concept ×0.85 (at most `CONCEPT_REPEAT_MAX` times:
/// the share cap `MAX_CONCEPT_SHARE` already limits one concept, and an unbounded penalty buried the trip's main
/// activity under weaker shots — owner feedback 2026-11-08, Nam Cát Tiên), from the same file ×0.9,
/// and ×0.6 for each chosen clip of the same file closer than 3 ideal clip lengths (spreads picks over
/// long continuous shots instead of taking consecutive windows).
// ponytail: greedy O(n²) over candidates; fine for a few thousand windows, revisit for whole-year libraries.
const CONCEPT_REPEAT_MAX: i32 = 3;

fn diversity(c: &Candidate, chosen: &[&Candidate], ideal_ms: i64) -> f64 {
    let same_tag = (chosen.iter().filter(|x| x.dominant.is_some() && x.dominant == c.dominant).count() as i32).min(CONCEPT_REPEAT_MAX);
    let same_media: Vec<_> = chosen.iter().filter(|x| x.clip.media_id == c.clip.media_id).collect();
    let near = same_media
        .iter()
        .filter(|x| (x.clip.in_ms - c.clip.out_ms).max(c.clip.in_ms - x.clip.out_ms) < 3 * ideal_ms)
        .count() as i32;
    0.85f64.powi(same_tag) * 0.9f64.powi(same_media.len() as i32) * 0.6f64.powi(near)
}

/// Same picture (dHash distance ≤ this, of 64 bits) and same main concept = nothing new to show.
const SAME_PICTURE: u32 = 12;
/// Per main concept, at most one clip per this much scene length (plus one), so one long shot of the same
/// thing is not chopped into many parts. A different concept in the same shot (walking a path, then the
/// big tree at its end: owner feedback 2026-11-08) has its own allowance.
const SCENE_MS_PER_CLIP: i64 = 60_000;
/// While other content is available, one main concept fills at most this share of the target (owner
/// feedback 2026-11-08: a recap that is mostly cycling is dull). Initial value, tune on real libraries.
const MAX_CONCEPT_SHARE: f64 = 0.35;

/// A further clip from an already used scene must show something different, within the per-concept cap.
fn new_content(c: &Candidate, chosen: &[&Candidate]) -> bool {
    let same_scene: Vec<_> = chosen.iter().filter(|x| x.clip.media_id == c.clip.media_id && x.clip.scene_index == c.clip.scene_index).collect();
    if same_scene.iter().filter(|x| x.dominant == c.dominant).count() as i64 >= 1 + c.scene_ms / SCENE_MS_PER_CLIP {
        return false;
    }
    !same_scene.iter().any(|x| {
        let similar = match (&x.hash, &c.hash) {
            (Some(h1), Some(h2)) => scoring::hamming(h1, h2).is_some_and(|d| d <= SAME_PICTURE),
            _ => false, // no frames to compare: only the per-scene cap applies
        };
        similar && x.dominant == c.dominant
    })
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

/// Consecutive clips of one file with ≤ MERGE_GAP_MS between them play as one continuous clip (no jump cut).
const MERGE_GAP_MS: i64 = 1_500;

fn merge_near(clips: Vec<PlanClip>) -> Vec<PlanClip> {
    let mut out: Vec<PlanClip> = vec![];
    for c in clips {
        if let Some(last) = out.last_mut()
            && last.media_id == c.media_id
            && c.in_ms >= last.out_ms
            && c.in_ms - last.out_ms <= MERGE_GAP_MS
        {
            last.out_ms = c.out_ms;
            last.rank = last.rank.max(c.rank);
            if !last.reasons.iter().any(|r| r == "merged") {
                last.reasons.push("merged".into());
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
pub fn plan(st: &Style, sources: &[Source]) -> Plan {
    plan_with(st, sources, &Wishes::default())
}

/// Whole-trip recap length from the footage when the user names none (owner 2026-11-11: "3 minutes feels short,
/// about 5 minutes for this trip, or what the footage needs"): seconds of video per minute of footage, and limits.
/// Initial values from one trip (72 min of footage → ~5 min).
const RECAP_S_PER_FOOTAGE_MIN: f64 = 4.2;
const RECAP_MIN_MS: i64 = 90_000;
const RECAP_MAX_MS: i64 = 360_000;
/// Coverage (journey recap): every concept with at least this much candidate footage gets a clip, and so does
/// every slice of recording time of this length (owner: the trip's forest paths and bridges must be in).
const COVER_CONCEPT_MS: i64 = 20_000;
const COVER_SLICE_MS: i64 = 8 * 60_000;

/// Plan for `st` with the user's wishes (preferred / avoided concepts, length).
#[cfg(test)]
pub fn plan_with(st: &Style, sources: &[Source], wishes: &Wishes) -> Plan {
    plan_with_photos(st, sources, &[], wishes)
}

/// A plan from videos and photos: photos are chosen first (`pick_photos`, part of the length), the videos fill the
/// rest, then each photo is placed among the clips by the time it was taken.
pub fn plan_with_photos(st: &Style, sources: &[Source], photos: &[PhotoSource], wishes: &Wishes) -> Plan {
    let mut notes = vec![];
    let journey = st.id == "journey_recap";
    let auto_ms = (journey && wishes.target_ms.is_none_or(|t| t <= 0)).then(|| {
        let footage: i64 = sources.iter().filter(|s| !s.excluded && !s.scenes.is_empty()).map(|s| s.media.probe.duration_ms).sum();
        let ms = ((footage as f64 / 60_000.0 * RECAP_S_PER_FOOTAGE_MIN * 1000.0) as i64).clamp(RECAP_MIN_MS, RECAP_MAX_MS);
        notes.push(format!("length {} chosen from {} min of footage (say \"N phút\" to change it)", clock(ms), footage / 60_000));
        ms
    });
    let st = &Style { target_ms: wishes.target_ms.filter(|&t| t > 0).or(auto_ms).unwrap_or(st.target_ms), ..st.clone() };
    let full_target = st.target_ms;
    let (picked, photo_notes, photo_skipped) = pick_photos(sources, photos, wishes, full_target);
    notes.extend(photo_notes);
    let photo_total: i64 = picked.iter().map(|p| p.clip.out_ms - p.clip.in_ms).sum();
    let st = &Style { target_ms: (full_target - photo_total).max(full_target / 2), ..st.clone() };
    let mut skipped = photo_skipped;
    let mut pool = vec![];
    let (mut avoided, mut tilted) = (0, 0);
    for s in sources {
        let name = s.media.path.rsplit(['\\', '/']).next().unwrap_or(&s.media.path).to_string();
        if s.excluded {
            skipped.push(format!("{name}: excluded by you"));
        } else if s.scenes.is_empty() {
            skipped.push(format!("{name}: not analysed yet"));
        } else {
            let (c, a, down) = candidates(s, st, wishes);
            avoided += a;
            tilted += down;
            if c.is_empty() && a == 0 && down == 0 {
                skipped.push(format!("{name}: no scene long enough for this style"));
            }
            pool.extend(c);
        }
    }
    if avoided > 0 {
        skipped.push(format!("{avoided} windows left out: they show {}", wishes.avoid.join(", ")));
    }
    if tilted > 0 {
        skipped.push(format!("{tilted} windows left out: camera pointing down at the handlebars / ground"));
    }
    let candidates = pool.len();

    let mut chosen: Vec<&Candidate> = vec![];
    let mut used: HashSet<(i64, i64)> = HashSet::new(); // (media, scene) already represented
    let share_ms = (st.target_ms as f64 * MAX_CONCEPT_SHARE) as i64;
    let mut total = 0;
    let eligible = |c: &Candidate, chosen: &[&Candidate], used: &HashSet<(i64, i64)>| {
        !chosen.iter().any(|x| x.clip.media_id == c.clip.media_id && overlap(x.clip.in_ms, x.clip.out_ms, c.clip.in_ms, c.clip.out_ms) > 0)
            && !c.duplicates.iter().any(|d| used.contains(d))
            && new_content(c, chosen)
    };
    let take = |c: &'_ Candidate, total: &mut i64, concept_ms: &mut HashMap<String, i64>, used: &mut HashSet<(i64, i64)>| {
        *total += c.clip.out_ms - c.clip.in_ms;
        if let Some(d) = &c.dominant {
            *concept_ms.entry(d.clone()).or_default() += c.clip.out_ms - c.clip.in_ms;
        }
        used.insert((c.clip.media_id, c.clip.scene_index));
    };
    let mut concept_ms: HashMap<String, i64> = HashMap::new();

    // Coverage first (journey recap): the best clean window of every concept with enough footage, then of every
    // slice of recording time that has none yet, so no part of the trip is left out by stronger repeated scenes.
    if journey {
        let clean = |c: &&Candidate| !c.clip.reasons.iter().any(|r| r.starts_with('⚠'));
        let mut footage: HashMap<&str, i64> = HashMap::new();
        for c in &pool {
            *footage.entry(c.dominant.as_deref().unwrap_or("")).or_default() += c.clip.out_ms - c.clip.in_ms;
        }
        let mut concepts: Vec<&str> = footage.iter().filter(|(k, ms)| !k.is_empty() && **ms >= COVER_CONCEPT_MS).map(|(k, _)| *k).collect();
        concepts.sort();
        let at = |c: &Candidate| crate::export::unix_ms(&c.order.0).map(|t| t + c.clip.in_ms);
        let t0 = pool.iter().filter_map(at).min().unwrap_or(0);
        let mut slices: Vec<i64> = pool.iter().filter_map(at).map(|t| (t - t0) / COVER_SLICE_MS).collect();
        slices.sort();
        slices.dedup();
        let (mut by_concept, mut by_time) = (0, 0);
        for key in concepts.iter().map(|k| Ok(*k)).chain(slices.iter().map(|s| Err(*s))) {
            let covered = chosen.iter().any(|c| match key {
                Ok(k) => c.dominant.as_deref() == Some(k),
                Err(s) => at(c).is_some_and(|t| (t - t0) / COVER_SLICE_MS == s),
            });
            if covered {
                continue;
            }
            let best = pool
                .iter()
                .filter(clean)
                .filter(|c| match key {
                    Ok(k) => c.dominant.as_deref() == Some(k),
                    Err(s) => at(c).is_some_and(|t| (t - t0) / COVER_SLICE_MS == s),
                })
                .filter(|c| eligible(c, &chosen, &used) && total + c.clip.out_ms - c.clip.in_ms <= st.target_ms)
                .max_by(|a, b| a.clip.rank.total_cmp(&b.clip.rank).then(b.order.cmp(&a.order)));
            if let Some(c) = best {
                take(c, &mut total, &mut concept_ms, &mut used);
                chosen.push(c);
                if key.is_ok() { by_concept += 1 } else { by_time += 1 }
            }
        }
        if by_concept + by_time > 0 {
            notes.push(format!("coverage: {by_concept} clips so every scene type appears, {by_time} so every part of the trip appears"));
        }
    }

    while total + st.clip_min_ms <= st.target_ms {
        let best = pool
            .iter()
            .filter(|c| eligible(c, &chosen, &used))
            .map(|c| {
                let len = c.clip.out_ms - c.clip.in_ms;
                let within = c.dominant.as_deref().is_none_or(|d| concept_ms.get(d).copied().unwrap_or(0) + len <= share_ms);
                (c, within, c.clip.rank * diversity(c, &chosen, st.clip_ideal_ms), distance(c, &chosen))
            })
            // Concepts still within their share first (others only fill up the target when nothing else is left);
            // ties: farthest from the clips already taken from that file (even coverage), then time order.
            .max_by(|(a, wa, ra, da), (b, wb, rb, db)| wa.cmp(wb).then(ra.total_cmp(rb)).then(da.cmp(db)).then(b.order.cmp(&a.order)));
        let Some((c, _, _, _)) = best else { break };
        take(c, &mut total, &mut concept_ms, &mut used);
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
        // Hook = best clip without bad-shot warnings (a hook must be instantly readable).
        let clean = |c: &Candidate| !c.clip.reasons.iter().any(|r| r.starts_with('⚠'));
        let i = (0..chosen.len())
            .max_by(|&a, &b| (clean(chosen[a]), chosen[a].clip.rank).partial_cmp(&(clean(chosen[b]), chosen[b].clip.rank)).unwrap())
            .unwrap();
        let c = chosen.remove(i);
        chosen.insert(0, c);
    }
    if !st.end_with.is_empty() && chosen.len() > 2 {
        if let Some(i) = pick(&chosen[1..], &|c| c.ends).map(|i| i + 1) {
            let c = chosen.remove(i);
            chosen.push(c);
        }
    }

    let mut clips = merge_near(chosen.into_iter().map(|c| c.clip.clone()).collect());
    place_photos(&mut clips, picked, sources, !st.open_with.is_empty() || st.opening.starts_with("Strongest"), !st.end_with.is_empty());
    let suggested_title = suggest_title(&clips, sources);
    Plan {
        style: st.id.into(),
        wishes: wishes.clone(),
        target_ms: full_target,
        total_ms: output_ms(&clips, sources, photos, st),
        transition_ms: st.transition_ms,
        export_preset: wishes.preset.clone().unwrap_or_else(|| st.export_preset.into()),
        clips,
        skipped,
        candidates,
        suggested_title,
        notes,
    }
}

// ------------------------------------------------------------------ photos

/// A library photo with its observations (None until analysed: job `media.photo_analyze`).
pub struct PhotoSource {
    pub photo: crate::photos::Photo,
    pub analysis: Option<crate::photos::Analysis>,
}

/// Library photos taken within [from, to] (their own local date, YYYY-MM-DD), with their observations.
pub fn gather_photos(c: &Connection, from: Option<&str>, to: Option<&str>) -> Result<Vec<PhotoSource>, String> {
    let mut found = crate::photos::analyses(c)?;
    Ok(crate::photos::list(c)?
        .into_iter()
        .filter(|p| {
            let d: String = p.taken_at.as_deref().unwrap_or(&p.imported_at).chars().take(10).collect();
            !(from.is_some_and(|f| d.as_str() < f) || to.is_some_and(|t| d.as_str() > t))
        })
        .map(|p| PhotoSource { analysis: found.remove(&p.id), photo: p })
        .collect())
}

/// Seconds a photo stays on screen unless the request says otherwise.
pub(crate) const PHOTO_MS: i64 = 4_000;
/// Share of the length given to photos: by default, "fewer", "more" (initial values; the owner reviews them).
const PHOTO_SHARE: f64 = 0.25;
const PHOTO_SHARE_FEWER: f64 = 0.12;
const PHOTO_SHARE_MORE: f64 = 0.4;
/// Photos taken this long before the first / after the last video still belong to the trip.
const PHOTO_TRIP_MARGIN_MS: i64 = 3 * 3_600_000;
/// A bad-shot concept (finger, blurry…) at least this likely leaves the photo out.
const PHOTO_BAD_SHOT: f64 = 0.3;

struct PickedPhoto {
    clip: PlanClip,
    at: i64,
}

/// Chooses photos for a plan: taken during the trip (the analysed videos' recording span ± `PHOTO_TRIP_MARGIN_MS`;
/// all photos when there is no video), not blurry / dark / overexposed / a bad shot / an avoided concept, one of each
/// near-identical picture (dHash), at most the photo share of the length, spread evenly over the trip.
fn pick_photos(sources: &[Source], photos: &[PhotoSource], wishes: &Wishes, target_ms: i64) -> (Vec<PickedPhoto>, Vec<String>, Vec<String>) {
    let (mut notes, mut skipped) = (vec![], vec![]);
    if photos.is_empty() {
        return (vec![], notes, skipped);
    }
    if wishes.photos.as_deref() == Some("none") {
        notes.push(format!("{} photos left out: you asked for video only", photos.len()));
        return (vec![], notes, skipped);
    }
    let spans: Vec<(i64, i64)> = sources
        .iter()
        .filter(|s| !s.excluded && !s.scenes.is_empty())
        .filter_map(|s| crate::export::unix_ms(s.media.probe.source_created_at.as_deref()?).map(|t| (t, t + s.media.probe.duration_ms)))
        .collect();
    let trip = spans.iter().map(|s| s.0).min().zip(spans.iter().map(|s| s.1).max());
    let photo_ms = wishes.photo_ms.unwrap_or(PHOTO_MS).clamp(crate::timeline::PHOTO_MS.0, crate::timeline::PHOTO_MS.1);
    let (mut unanalysed, mut other_days, mut avoided) = (0, 0, 0);
    let mut usable: Vec<(f64, i64, &PhotoSource, Vec<tagging::TagScore>)> = vec![];
    for p in photos {
        let name = p.photo.path.rsplit(['\\', '/']).next().unwrap_or(&p.photo.path).to_string();
        let Some(a) = &p.analysis else {
            unanalysed += 1;
            continue;
        };
        let at = p.photo.taken_at.as_deref().and_then(crate::export::unix_ms);
        match (trip, at) {
            (Some((first, last)), Some(t)) if t < first - PHOTO_TRIP_MARGIN_MS || t > last + PHOTO_TRIP_MARGIN_MS => {
                other_days += 1;
                continue;
            }
            (Some(_), None) => {
                other_days += 1;
                continue;
            }
            _ => {}
        }
        let (tech, flags) = scoring::technical(std::slice::from_ref(&a.metrics), 0.0);
        if let Some(f) = flags.iter().find(|f| ["black", "blurry", "dark", "overexposed"].contains(f)) {
            skipped.push(format!("{name}: photo {f}"));
            continue;
        }
        let tags = a.tags.as_ref().map(|t| tagging::summarize(t, 0, 1, 0.0)).unwrap_or_default();
        if let Some(bad) = tags.iter().find(|t| t.negative && t.score >= PHOTO_BAD_SHOT) {
            skipped.push(format!("{name}: photo looks like \"{}\" ({:.0}%)", bad.vi, bad.score * 100.0));
            continue;
        }
        if tags.iter().any(|t| t.score >= AVOID_MIN && named(t, &wishes.avoid)) {
            avoided += 1;
            continue;
        }
        let prefer: f64 = tags.iter().filter(|t| named(t, &wishes.prefer)).map(|t| t.score).sum::<f64>().min(1.0);
        let less: f64 = tags.iter().filter(|t| named(t, &wishes.less)).map(|t| t.score).sum::<f64>().min(1.0);
        let rank = tech.total * (1.0 + PREFER_BOOST * prefer) * (1.0 - LESS_CUT * less);
        usable.push((rank, at.unwrap_or(0), p, tags));
    }
    // One of each near-identical picture: the best ranked.
    usable.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.photo.id.cmp(&b.2.photo.id)));
    let mut kept: Vec<(f64, i64, &PhotoSource, Vec<tagging::TagScore>)> = vec![];
    for u in usable {
        let h = &u.2.analysis.as_ref().unwrap().metrics.dhash;
        if kept.iter().any(|k| scoring::hamming(h, &k.2.analysis.as_ref().unwrap().metrics.dhash).is_some_and(|d| d <= SAME_PICTURE)) {
            skipped.push(format!("{}: same picture as another photo", u.2.photo.path.rsplit(['\\', '/']).next().unwrap_or("")));
            continue;
        }
        kept.push(u);
    }
    let share = match wishes.photos.as_deref() {
        Some("fewer") => PHOTO_SHARE_FEWER,
        Some("more") => PHOTO_SHARE_MORE,
        _ => PHOTO_SHARE,
    };
    let budget = if trip.is_some() { ((target_ms as f64 * share) as i64 / photo_ms).max(1) as usize } else { (target_ms / photo_ms).max(1) as usize };
    kept.sort_by_key(|k| (k.1, k.2.photo.id));
    let usable_n = kept.len();
    if kept.len() > budget {
        // Even coverage: the best photo of each of `budget` equal runs in time order.
        let n = kept.len();
        let mut chosen = vec![];
        for b in 0..budget {
            let run = &kept[b * n / budget..(b + 1) * n / budget];
            if let Some(best) = run.iter().max_by(|x, y| x.0.total_cmp(&y.0)) {
                chosen.push(best.clone());
            }
        }
        kept = chosen;
    }
    if unanalysed > 0 {
        notes.push(format!("{unanalysed} photos not analysed yet (open the Photos page to analyse them)"));
    }
    if other_days > 0 {
        notes.push(format!("{other_days} photos taken outside this trip's recording time left out"));
    }
    if avoided > 0 {
        skipped.push(format!("{avoided} photos left out: they show {}", wishes.avoid.join(", ")));
    }
    if !kept.is_empty() {
        notes.push(format!("{} of {usable_n} usable photos used, {} s each, placed by the time they were taken", kept.len(), photo_ms / 1000));
    }
    let picked = kept
        .into_iter()
        .map(|(rank, at, p, tags)| {
            let a = p.analysis.as_ref().unwrap();
            let (tech, _) = scoring::technical(std::slice::from_ref(&a.metrics), 0.0);
            let content: Vec<&tagging::TagScore> = tags.iter().filter(|t| !t.negative).collect();
            let mut reasons = vec![format!("photo · technical {:.0}", tech.total * 100.0)];
            reasons.extend(content.iter().take(2).map(|t| format!("{} ({:.0}%)", t.vi, t.score * 100.0)));
            PickedPhoto {
                at,
                clip: PlanClip {
                    media_id: -p.photo.id,
                    scene_index: -1,
                    in_ms: 0,
                    out_ms: photo_ms,
                    scores: Scores { technical: tech.total, semantic: content.first().map_or(0.0, |t| t.score), speech: 0.0, favorite: 0.0 },
                    rank,
                    tags: content.iter().take(3).map(|t| t.vi.clone()).collect(),
                    reasons,
                    photo_id: Some(p.photo.id),
                },
            }
        })
        .collect();
    (picked, notes, skipped)
}

/// Puts each photo after the last clip recorded before it was taken (the opening clip stays first, the ending clip last).
fn place_photos(clips: &mut Vec<PlanClip>, mut picked: Vec<PickedPhoto>, sources: &[Source], keep_first: bool, keep_last: bool) {
    let start: HashMap<i64, i64> =
        sources.iter().filter_map(|s| Some((s.media.id, crate::export::unix_ms(s.media.probe.source_created_at.as_deref()?)?))).collect();
    let mut at: HashMap<i64, i64> = HashMap::new(); // photo clips already placed: media id (−photo) → taken
    picked.sort_by_key(|p| p.at);
    for p in picked {
        let video_clips = clips.len();
        let mut i = clips
            .iter()
            .rposition(|c| at.get(&c.media_id).copied().or_else(|| start.get(&c.media_id).map(|t| t + c.in_ms)).is_some_and(|t| t <= p.at))
            .map_or(0, |k| k + 1);
        if keep_first && video_clips > 0 {
            i = i.max(1);
        }
        if keep_last && video_clips > 2 {
            i = i.min(video_clips - 1);
        }
        at.insert(p.clip.media_id, p.at);
        clips.insert(i, p.clip);
    }
}

fn clock(ms: i64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

/// Length of the exported video: clip lengths minus the style's transitions, with the same rules as export.
fn output_ms(clips: &[PlanClip], sources: &[Source], photos: &[PhotoSource], st: &Style) -> i64 {
    let mut recorded: HashMap<i64, Option<&str>> = sources.iter().map(|s| (s.media.id, s.media.probe.source_created_at.as_deref())).collect();
    recorded.extend(photos.iter().map(|p| (-p.photo.id, p.photo.taken_at.as_deref())));
    let joins: Vec<crate::export::JoinClip> = clips.iter().map(|c| (c.media_id, recorded.get(&c.media_id).copied().flatten(), c.in_ms, c.out_ms)).collect();
    clips.iter().map(|c| c.out_ms - c.in_ms).sum::<i64>() - crate::export::joins(st.transition, st.transition_ms, &joins).iter().map(|j| j.1).sum::<i64>()
}

/// "Xe đạp · 20/02/2026" or "Biển · 02/2026 – 05/2026": most frequent main concept + date range.
fn suggest_title(clips: &[PlanClip], sources: &[Source]) -> String {
    let mut counts: Vec<(String, usize)> = vec![];
    for t in clips.iter().filter_map(|c| c.tags.first()) {
        match counts.iter_mut().find(|(n, _)| n == t) {
            Some(e) => e.1 += 1,
            None => counts.push((t.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut days: Vec<String> = clips
        .iter()
        .filter_map(|c| sources.iter().find(|s| s.media.id == c.media_id).map(|s| day(&s.media)))
        .filter(|d| d.len() == 10)
        .collect();
    days.sort();
    let fmt = |d: &str| format!("{}/{}/{}", &d[8..10], &d[5..7], &d[..4]);
    let month = |d: &str| format!("{}/{}", &d[5..7], &d[..4]);
    let dates = match (days.first(), days.last()) {
        (Some(a), Some(b)) if a == b => fmt(a),
        (Some(a), Some(b)) if a[..7] == b[..7] => format!("{} – {}", &fmt(a)[..2], fmt(b)),
        (Some(a), Some(b)) => format!("{} – {}", month(a), month(b)),
        _ => String::new(),
    };
    let concept = counts.first().map(|(t, _)| {
        let mut c = t.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    });
    match (concept, dates.is_empty()) {
        (Some(c), false) => format!("{c} · {dates}"),
        (Some(c), true) => c,
        (None, false) => dates,
        (None, true) => String::new(),
    }
}

/// Export options after applying a plan: the style's look, then the user's wishes (colour, text, subtitles,
/// no music); existing music/titles kept otherwise, the suggested title used only when the user has none.
/// Wished library music is chosen by the caller (it needs the applied timeline).
pub fn styled_options(mut o: crate::export::Options, st: &Style, p: &Plan) -> crate::export::Options {
    let w = &p.wishes;
    o.fill = p.export_preset == "reels";
    o.transition_ms = st.transition_ms;
    o.transition = st.transition.into();
    o.color = w.color.clone().unwrap_or_else(|| st.color.into());
    o.fade = true;
    o.normalize = true;
    o.titles.captions = w.captions.unwrap_or(st.captions);
    if let Some(t) = &w.title {
        o.titles.title = t.clone();
    } else if o.titles.title.trim().is_empty() {
        o.titles.title = p.suggested_title.clone();
    }
    if let Some(e) = &w.ending {
        o.titles.ending = e.clone();
    }
    if w.music.as_deref() == Some("none") {
        o.music = None;
    }
    // Measured on the owner's DJI footage (2026-11-08): the clips' own sound averages about −25 dBFS, above the
    // ducking threshold (≈ −34 dBFS), so at full level the music stays ~8 dB down for the whole video under
    // road and wind noise. With music and no subtitles the clips' sound becomes a quiet natural bed; louder
    // moments (voices) still duck the music.
    let music = o.music.is_some() || w.music.as_deref().is_some_and(|m| m != "none");
    if music && !o.titles.captions {
        o.original_volume = o.original_volume.min(AMBIENCE_VOLUME);
    } else if o.original_volume == AMBIENCE_VOLUME {
        o.original_volume = 1.0; // the level this function set for earlier music: back to full without it
    }
    o
}

/// Level of the clips' own sound under music (−9 dB). Initial value from one library, tune by ear.
const AMBIENCE_VOLUME: f64 = 0.35;

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
        match clip.photo_id {
            Some(photo) => crate::timeline::add_photo(&tx, project_id, photo, clip.out_ms - clip.in_ms).map(drop)?,
            None => crate::timeline::add(&tx, project_id, clip.media_id, clip.in_ms, clip.out_ms).map(drop)?,
        }
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
            tags: [("sea", "biển"), ("people", "con người"), ("bicycle", "xe đạp")].iter().map(|(e, v)| TagName { en: (*e).into(), vi: (*v).into(), negative: false }).collect(),
            cues: vec![],
            samples: per_2s.iter().enumerate().map(|(i, &k)| TagSample { at_ms: i as i64 * 2000, tags: vec![(k, probs)], cues: vec![] }).collect(),
        }
    }

    fn src(m: Media, scenes: Vec<ScoredScene>) -> Source {
        Source { media: m, scenes, tags: None, transcript: None, favorite: false, excluded: false }
    }

    fn photo(id: i64, taken: &str, sharpness: f64, dhash: &str, analysed: bool) -> PhotoSource {
        PhotoSource {
            photo: crate::photos::Photo {
                id, path: format!(r"E:\p{id}.jpg"), size: 1, width: Some(4000), height: Some(3000), taken_at: Some(taken.into()),
                edits: serde_json::Value::Null, imported_at: "2026-11-01T00:00:00Z".into(), thumb: None, analyzed: analysed, missing: false,
            },
            analysis: analysed.then(|| crate::photos::Analysis {
                metrics: Sample { at_ms: 0, sharpness, brightness: 0.45, contrast: 0.2, clip_low: 0.0, clip_high: 0.0, black: false, dhash: dhash.into() },
                tags: Some(MediaTags {
                    device: "GPU".into(),
                    tags: vec![TagName { en: "river".into(), vi: "sông".into(), negative: false }],
                    cues: vec![],
                    samples: vec![TagSample { at_ms: 0, tags: vec![(0, 0.8)], cues: vec![] }],
                }),
            }),
        }
    }

    #[test]
    fn photos_chosen_by_quality_and_placed_by_time_taken() {
        let st = style("journey_recap").unwrap();
        let sources = vec![
            src(media(1, "2026-02-20", 600_000), vec![scene(1, 0, 0, 600_000, 0.9)]),
            Source { media: Media { probe: Probe { source_created_at: Some("2026-02-20T12:00:00Z".into()), ..media(2, "2026-02-20", 600_000).probe }, ..media(2, "2026-02-20", 600_000) },
                ..src(media(2, "2026-02-20", 600_000), vec![scene(2, 0, 0, 600_000, 0.9)]) },
        ];
        let photos = vec![
            photo(1, "2026-02-20T18:30:00+07:00", 5_000.0, "ece8ec6e6ae47351", true), // 11:30 UTC: between the two videos
            photo(2, "2026-02-20T18:31:00+07:00", 10.0, "0000000000000000", true),  // blurry
            photo(3, "2026-02-20T18:32:00+07:00", 3_000.0, "ece8ec6e6ae47350", true), // same picture as 1, softer
            photo(4, "2026-03-01T09:00:00+07:00", 5_000.0, "ffffffffffffffff", true), // another trip
            photo(5, "2026-02-20T18:33:00+07:00", 5_000.0, "1234567812345678", false), // not analysed
        ];
        let p = plan_with_photos(&st, &sources, &photos, &Wishes::default());
        let at: Vec<usize> = p.clips.iter().enumerate().filter(|(_, c)| c.photo_id.is_some()).map(|(i, _)| i).collect();
        assert_eq!(at.len(), 1, "{:?}", p.clips.iter().map(|c| (c.media_id, c.in_ms)).collect::<Vec<_>>());
        let photo_clip = &p.clips[at[0]];
        assert_eq!((photo_clip.photo_id, photo_clip.media_id, photo_clip.out_ms - photo_clip.in_ms), (Some(1), -1, PHOTO_MS));
        assert!(p.clips[..at[0]].iter().all(|c| c.media_id == 1) && p.clips[at[0] + 1..].iter().all(|c| c.media_id == 2), "taken between the two recordings");
        assert!(p.skipped.iter().any(|s| s.contains("p2.jpg: photo blurry")), "{:?}", p.skipped);
        assert!(p.skipped.iter().any(|s| s.contains("p3.jpg: same picture")));
        assert!(p.notes.iter().any(|n| n.contains("1 photos taken outside")) && p.notes.iter().any(|n| n.contains("1 photos not analysed")), "{:?}", p.notes);
        let videos: i64 = p.clips.iter().filter(|c| c.photo_id.is_none()).map(|c| c.out_ms - c.in_ms).sum();
        assert!(videos + PHOTO_MS <= p.target_ms + st.clip_max_ms, "photos are part of the length");

        let none = plan_with_photos(&st, &sources, &photos, &Wishes { photos: Some("none".into()), ..Wishes::default() });
        assert!(none.clips.iter().all(|c| c.photo_id.is_none()) && none.notes.iter().any(|n| n.contains("video only")));
        let longer = plan_with_photos(&st, &sources, &photos, &Wishes { photo_ms: Some(6_000), ..Wishes::default() });
        assert_eq!(longer.clips.iter().find(|c| c.photo_id.is_some()).map(|c| c.out_ms), Some(6_000));
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
        assert!(p.total_ms <= st.target_ms && p.clips.len() == 12, "per-scene cap (6 + 6) before the 60 s target: {} ms, {} clips", p.total_ms, p.clips.len());
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
    fn journey_recap_keeps_trip_order_and_longer_clips() {
        let st = style("journey_recap").unwrap();
        let mut sources: Vec<Source> = (1..=8).map(|i| src(media(i, "2026-02-20", 60_000), vec![scene(i, 0, 0, 60_000, 0.8)])).collect();
        sources[4].tags = Some(tags(&[0; 30], 0.9)); // a sea view in the middle of the trip is not moved to the front
        let p = plan(&st, &sources);
        assert_eq!(p.clips[0].media_id, 1, "starts where the trip starts");
        assert!(p.clips.windows(2).all(|w| (w[0].media_id, w[0].in_ms) < (w[1].media_id, w[1].in_ms)), "trip order");
        assert!(p.clips.iter().all(|c| c.out_ms - c.in_ms >= st.clip_min_ms), "no short, choppy clips");
        assert!(p.clips.iter().map(|c| c.media_id).collect::<HashSet<_>>().len() == 8, "every part of the trip appears");
    }

    #[test]
    fn long_single_shot_is_sampled_spread_out() {
        let st = style("clean_minimal").unwrap(); // 60 s from one 300 s continuous shot
        let p = plan(&st, &[src(media(1, "2026-02-20", 300_000), vec![scene(1, 0, 0, 300_000, 0.8)])]);
        assert_eq!(p.clips.len(), 6, "capped at 1 + 300/60 clips for one shot");
        assert!(p.clips.windows(2).all(|w| w[1].in_ms - w[0].out_ms >= 2 * st.clip_ideal_ms), "no consecutive windows: {:?}",
            p.clips.iter().map(|c| c.in_ms / 1000).collect::<Vec<_>>());
        assert!(p.clips.last().unwrap().out_ms > 240_000, "covers the whole shot");
    }

    #[test]
    fn speech_is_never_cut_mid_sentence() {
        let st = style("clean_minimal").unwrap(); // 3–5 s, cap 15 s
        let speech = [(3_000, 6_000), (20_000, 45_000)];
        assert_eq!(fit_speech(0, 4_000, &speech, (0, 60_000), &st), Some((0, 6_000)), "extended to the end of the sentence");
        assert_eq!(fit_speech(4_000, 8_000, &speech, (0, 60_000), &st), Some((3_000, 8_000)), "extended back to its start");
        assert_eq!(fit_speech(24_000, 28_000, &speech, (0, 60_000), &st), None, "25 s speech cannot fit: dropped, not cut");
        assert_eq!(fit_speech(16_000, 22_000, &speech, (0, 60_000), &st), Some((16_000, 20_000)), "talk too long to include: ends before it");
        assert_eq!(fit_speech(18_000, 22_000, &speech, (0, 60_000), &st), None, "shrinking would go below the minimum");
        assert_eq!(fit_speech(12_000, 25_000, &speech, (0, 60_000), &st), Some((12_000, 20_000)), "shrunk to exclude the long talk");
        assert_eq!(fit_speech(4_000, 8_000, &speech, (4_000, 60_000), &st), None, "sentence starts before the scene and cannot be excluded");
    }

    #[test]
    fn forest_trails_and_bridges_kept_though_handlebars_fill_the_frame() {
        let st = style("journey_recap").unwrap();
        // File 1: 120 s ride, handlebars always in frame (view 0.01). 0–60 s forest trail with a bridge at 40–60 s,
        // 60–120 s paved open road. File 2: 120 s of sea, camera raised.
        let mut ride = tags(&[2; 60], 0.9);
        ride.cues = [("view ahead", "thấy đường phía trước"), ("forest trail", "đường mòn trong rừng"), ("small bridge", "qua cầu nhỏ")]
            .iter().map(|(e, v)| TagName { en: (*e).into(), vi: (*v).into(), negative: false }).collect();
        for s in ride.samples.iter_mut() {
            let t = s.at_ms;
            s.cues = vec![0.01, if t < 60_000 { 0.98 } else { 0.1 }, if (40_000..60_000).contains(&t) { 0.8 } else { 0.05 }];
        }
        let mut a = src(media(1, "2026-02-20", 120_000), vec![scene(1, 0, 0, 120_000, 0.8)]);
        a.tags = Some(ride);
        let mut sea = tags(&[0; 60], 0.9);
        sea.cues = vec![TagName { en: "view ahead".into(), vi: "thấy đường phía trước".into(), negative: false }];
        sea.samples.iter_mut().for_each(|s| s.cues = vec![0.9]);
        let mut b = src(media(2, "2026-02-20", 120_000), vec![scene(2, 0, 0, 120_000, 0.9)]);
        b.tags = Some(sea);
        let p = plan(&st, &[a, b]);
        let ride: Vec<_> = p.clips.iter().filter(|c| c.media_id == 1).collect();
        assert!(!ride.is_empty() && ride.iter().all(|c| c.out_ms <= 60_000), "trail kept, paved handlebar shots left out: {:?}", ride.iter().map(|c| (c.in_ms, c.out_ms)).collect::<Vec<_>>());
        assert!(ride.iter().any(|c| c.tags[0] == "qua cầu nhỏ"), "the bridge is its own scene: {:?}", ride.iter().map(|c| &c.tags).collect::<Vec<_>>());
        assert!(ride.iter().any(|c| c.tags[0] == "đường mòn trong rừng"), "coverage: the forest trail appears too");
        assert_eq!(p.target_ms, RECAP_MIN_MS, "4 min of footage → shortest auto length");
        assert!(p.notes.iter().any(|n| n.starts_with("length 1:30 chosen from 4 min")), "{:?}", p.notes);
        assert!(p.skipped.iter().any(|s| s.contains("camera pointing down")));
        let fixed = plan_with(&st, &[], &Wishes { target_ms: Some(200_000), ..Default::default() });
        assert!(fixed.target_ms == 200_000 && fixed.notes.is_empty(), "a length the user asked for wins");
    }

    #[test]
    fn raised_camera_beats_camera_pointing_down() {
        let st = style("social_story").unwrap(); // hook = strongest clean clip
        // One 24 s cycling shot: first 12 s camera tilted down, last 12 s raised (road and landscape ahead).
        let mut t = tags(&[2; 12], 0.9);
        t.cues = vec![TagName { en: "view ahead".into(), vi: "thấy đường phía trước".into(), negative: false }];
        for s in t.samples.iter_mut() {
            s.cues = vec![if s.at_ms < 12_000 { 0.01 } else { 0.95 }];
        }
        let shot = || Source { tags: Some(t.clone()), ..src(media(1, "2026-02-20", 24_000), vec![scene(1, 0, 0, 24_000, 0.9)]) };
        let p = plan_with(&st, &[shot()], &Wishes { target_ms: Some(4_000), ..Default::default() });
        assert!(p.clips.iter().all(|c| c.in_ms >= 12_000), "raised camera first: {:?}", p.clips.iter().map(|c| c.in_ms).collect::<Vec<_>>());
        let full = plan_with(&st, &[shot()], &Wishes { target_ms: Some(30_000), ..Default::default() });
        assert!(full.clips.iter().all(|c| c.in_ms >= 12_000), "tilted-down windows never fill the target");
        assert!(full.skipped.iter().any(|s| s.contains("camera pointing down")), "{:?}", full.skipped);
        assert!(p.clips[0].reasons.iter().any(|r| r.starts_with("thấy đường phía trước")));
        let mut old = src(media(2, "2026-02-20", 4_000), vec![scene(2, 0, 0, 4_000, 0.9)]);
        old.tags = Some(tags(&[2, 2], 0.9)); // tagged with a model without cues
        let o = plan(&st, &[old]);
        assert!(o.clips.iter().all(|c| !c.reasons.iter().any(|r| r.contains("máy cúi") || r.contains("phía trước"))), "older tags: no effect");
        assert!((view_factor(0.0) - 0.6).abs() < 1e-9 && (view_factor(1.0) - 1.3).abs() < 1e-9);
    }

    #[test]
    fn stretched_whisper_lines_do_not_stretch_clips() {
        let st = style("journey_recap").unwrap();
        let mut a = src(media(1, "2026-02-20", 60_000), vec![scene(1, 0, 0, 60_000, 0.8)]);
        let line = |s: i64, e: i64, text: &str| Segment { start_ms: s, end_ms: e, text: text.into() };
        // Real line from the owner's footage: 4 syllables timed over 22 s of road noise; and one normal line.
        a.transcript = Some(Transcript { media_id: 1, model_id: "w".into(), language: Some("vi".into()),
            segments: vec![line(3_000, 25_000, "Tính ra lẹ chứ"), line(40_000, 42_000, "Đi tiếp không chị?")], created_at: String::new() });
        let p = plan(&st, &[a]);
        assert!(p.clips.iter().all(|c| c.out_ms - c.in_ms <= st.clip_max_ms * 3 && (c.in_ms >= 25_000 || c.scores.speech == 0.0)),
            "stretched line ignored: {:?}", p.clips.iter().map(|c| (c.in_ms, c.out_ms, c.scores.speech)).collect::<Vec<_>>());
        assert!(p.clips.iter().all(|c| c.out_ms <= 40_000 || c.in_ms >= 42_000 || (c.in_ms <= 40_000 && c.out_ms >= 42_000)), "real line kept whole");
    }

    #[test]
    fn one_unchanging_shot_is_not_chopped() {
        let st = style("clean_minimal").unwrap();
        // 120 s shot, same picture and tags throughout → at most 1 + 120/60 = 3 clips, and only if different.
        let mut sc = scene(1, 0, 0, 120_000, 0.8);
        sc.scene.samples = (0..60).map(|i| Sample { at_ms: i * 2000, sharpness: 300.0, brightness: 0.5, contrast: 0.2, clip_low: 0.0,
            clip_high: 0.0, black: false, dhash: "00ff00ff00ff00ff".into() }).collect();
        let p = plan(&st, &[src(media(1, "2026-02-20", 120_000), vec![sc])]);
        assert_eq!(p.clips.len(), 1, "identical content: one clip, not {}", p.clips.len());
    }

    #[test]
    fn near_clips_merge() {
        let c = |a: i64, b: i64, m: i64| PlanClip { media_id: m, scene_index: 0, in_ms: a, out_ms: b, scores: Scores { technical: 1.0, semantic: 1.0, speech: 0.0, favorite: 0.0 },
            rank: 0.5, tags: vec![], reasons: vec![], photo_id: None };
        let merged = merge_near(vec![c(0, 4_000, 1), c(5_000, 9_000, 1), c(20_000, 24_000, 1), c(24_500, 26_000, 2)]);
        assert_eq!(merged.iter().map(|x| (x.media_id, x.in_ms, x.out_ms)).collect::<Vec<_>>(), [(1, 0, 9_000), (1, 20_000, 24_000), (2, 24_500, 26_000)]);
    }

    #[test]
    fn title_suggestion_and_style_look() {
        let st = style("memory_nostalgia").unwrap();
        let mut a = src(media(1, "2026-02-20", 16_000), vec![scene(1, 0, 0, 16_000, 0.7)]);
        a.tags = Some(tags(&[2; 8], 0.9));
        let b = src(media(2, "2026-02-27", 9_000), vec![scene(2, 0, 0, 9_000, 0.7)]);
        let p = plan(&st, &[a, b]);
        assert_eq!(p.suggested_title, "Xe đạp · 20 – 27/02/2026");
        let o = styled_options(crate::export::Options { titles: crate::export::Titles { title: "Của tôi".into(), ..Default::default() }, ..Default::default() }, &st, &p);
        assert_eq!((o.color.as_str(), o.transition_ms, o.fade, o.fill, o.titles.title.as_str()), ("warm", 800, true, false, "Của tôi"), "user title kept");
        assert!(styles().iter().all(|s| crate::export::TRANSITIONS.contains(&s.transition) && crate::export::COLORS.contains(&s.color)));
    }

    #[test]
    fn bad_shots_are_penalised_and_never_the_hook() {
        let st = style("social_story").unwrap();
        let mut t = tags(&[1, 1, 1, 1, 1, 1], 0.6);
        t.tags.push(TagName { en: "finger over the lens".into(), vi: "ngón tay che ống kính".into(), negative: true });
        for s in t.samples.iter_mut().filter(|s| s.at_ms < 4_000) {
            s.tags.push((3, 0.5)); // finger over the lens in the first 4 s
        }
        let mut a = src(media(1, "2026-02-20", 12_000), vec![scene(1, 0, 0, 12_000, 0.9)]);
        a.tags = Some(t);
        let p = plan(&st, &[a]);
        let bad = p.clips.iter().find(|c| c.in_ms < 4_000);
        assert!(bad.is_none_or(|c| c.reasons.iter().any(|r| r.starts_with("⚠ ngón tay"))), "flagged when used");
        assert!(p.clips[0].in_ms >= 4_000, "hook is a clean shot: {:?}", p.clips.iter().map(|c| (c.in_ms, c.reasons.clone())).collect::<Vec<_>>());
        assert!(p.clips.iter().all(|c| !c.tags.iter().any(|t| t.contains("ngón tay"))), "bad shots are not content tags");
    }

    #[test]
    fn variety_beats_one_repeated_activity() {
        // Owner's library pattern: many technically perfect cycling clips, one continuous shot that walks a path
        // (here "sea") and then reaches the big tree ("people"), and other moments of lower technical quality.
        let st = style("clean_minimal").unwrap(); // 60 s, no favoured concepts
        let mut sources = vec![];
        for i in 1..=10 {
            let mut bike = src(media(i, "2026-02-20", 30_000), vec![scene(i, 0, 0, 30_000, 0.95)]);
            bike.tags = Some(tags(&[2; 15], 0.95));
            sources.push(bike);
        }
        let mut walk = src(media(20, "2026-02-21", 56_000), vec![scene(20, 0, 0, 56_000, 0.8)]);
        walk.tags = Some(tags(&[[0; 5].as_slice(), [1; 23].as_slice()].concat(), 0.9));
        sources.push(walk);
        for i in 30..40 {
            let mut other = src(media(i, "2026-02-22", 20_000), vec![scene(i, 0, 0, 20_000, 0.7)]);
            let mut t = tags(&[3; 10], 0.9);
            let (en, vi) = [("market", "chợ"), ("temple", "chùa"), ("food", "đồ ăn"), ("beach", "bãi biển"), ("dog", "chó")][(i % 5) as usize];
            t.tags.push(TagName { en: en.into(), vi: vi.into(), negative: false });
            other.tags = Some(t);
            sources.push(other);
        }
        let p = plan(&st, &sources);
        let walk_clips: Vec<_> = p.clips.iter().filter(|c| c.media_id == 20).collect();
        assert!(walk_clips.iter().any(|c| c.in_ms < 10_000), "the walk up the path is kept: {walk_clips:?}");
        assert!(walk_clips.iter().any(|c| c.in_ms >= 10_000), "and the tree at its end");
        let bike_ms: i64 = p.clips.iter().filter(|c| c.tags.first().is_some_and(|t| t == "xe đạp")).map(|c| c.out_ms - c.in_ms).sum();
        assert!(bike_ms as f64 <= st.target_ms as f64 * MAX_CONCEPT_SHARE, "cycling limited to its share: {bike_ms} ms");
        assert!(bike_ms > 0, "the best footage still appears");

        // Nothing else to show: the share only orders the choice, the target is still filled.
        let only_bikes = &sources[..10];
        assert!(plan(&st, only_bikes).total_ms as f64 > st.target_ms as f64 * MAX_CONCEPT_SHARE);
    }

    #[test]
    fn wishes_prefer_avoid_and_length() {
        let st = style("clean_minimal").unwrap();
        let library = || {
            let mut people = src(media(1, "2026-02-20", 40_000), vec![scene(1, 0, 0, 40_000, 0.9)]);
            people.tags = Some(tags(&[1; 20], 0.9));
            let mut sea = src(media(2, "2026-02-20", 40_000), vec![scene(2, 0, 0, 40_000, 0.6)]);
            sea.tags = Some(tags(&[0; 20], 0.9));
            vec![people, sea]
        };
        let best = |p: &Plan, m: i64| p.clips.iter().filter(|c| c.media_id == m).map(|c| c.rank).fold(0.0, f64::max);

        let plain = plan(&st, &library());
        assert!(best(&plain, 1) > best(&plain, 2), "sharper people footage wins by default");
        let wished = plan_with(&st, &library(), &Wishes { prefer: vec!["Biển".into()], ..Default::default() });
        assert!(best(&wished, 2) > best(&wished, 1), "a preferred concept (Vietnamese name, any case) wins");
        assert!(wished.clips.iter().any(|c| c.reasons.iter().any(|r| r.starts_with("wanted: biển"))));

        let w = Wishes { avoid: vec!["people".into()], target_ms: Some(12_000), ..Default::default() };
        let p = plan_with(&st, &library(), &w);
        assert!(p.clips.iter().all(|c| c.media_id == 2), "avoided concept never used");
        assert!(p.skipped.iter().any(|s| s.contains("windows left out: they show people")), "{:?}", p.skipped);
        assert!(p.total_ms <= 12_000 && p.target_ms == 12_000 && p.wishes == w, "length wish and wishes stored with the plan");
    }

    #[test]
    fn applying_honours_look_and_text_wishes() {
        let st = style("travel_cinematic").unwrap(); // landscape, cinematic, no subtitles
        let w = Wishes {
            preset: Some("reels".into()), color: Some("warm".into()), title: Some("Đà Lạt".into()), ending: Some("Hết".into()),
            captions: Some(true), music: Some("none".into()), ..Default::default()
        };
        let p = plan_with(&st, &[src(media(1, "2026-02-20", 20_000), vec![scene(1, 0, 0, 20_000, 0.8)])], &w);
        assert_eq!(p.export_preset, "reels");
        let before = crate::export::Options {
            music: Some(crate::export::Music { path: r"E:\song.mp3".into(), library: None, volume: 0.4, duck: true }),
            titles: crate::export::Titles { title: "Old".into(), ..Default::default() },
            ..Default::default()
        };
        let o = styled_options(before, &st, &p);
        assert_eq!((o.fill, o.color.as_str(), o.titles.title.as_str(), o.titles.ending.as_str(), o.titles.captions), (true, "warm", "Đà Lạt", "Hết", true));
        assert!(o.music.is_none(), "\"no music\" removes the music");
        let o = styled_options(crate::export::Options::default(), &st, &plan(&st, &[]));
        assert_eq!((o.fill, o.color.as_str(), o.titles.captions), (false, "cinematic", false), "no wishes: the style decides");
        assert_eq!(o.original_volume, 1.0, "no music: the clips' sound stays at full level");
        let music = plan_with(&st, &[], &Wishes { music: Some("low".into()), ..Default::default() });
        let bed = styled_options(crate::export::Options::default(), &st, &music);
        assert_eq!(bed.original_volume, AMBIENCE_VOLUME, "under music: a quiet bed");
        let silent = plan_with(&st, &[], &Wishes { music: Some("none".into()), ..Default::default() });
        assert_eq!(styled_options(bed, &st, &silent).original_volume, 1.0, "music removed: own sound back to full");
        let own = crate::export::Options { original_volume: 0.7, ..Default::default() };
        assert_eq!(styled_options(own, &st, &silent).original_volume, 0.7, "a level the user chose is kept");
        let talk = style("travel_documentary").unwrap(); // subtitles on: speech matters, level kept
        assert_eq!(styled_options(crate::export::Options::default(), &talk, &plan_with(&talk, &[], &music.wishes)).original_volume, 1.0);
    }

    #[test]
    fn same_day_clips_follow_recording_time() {
        let st = style("clean_minimal").unwrap();
        let mut late = media(1, "2026-02-20", 4_000);
        late.probe.source_created_at = Some("2026-02-20T16:00:00Z".into());
        let mut early = media(2, "2026-02-20", 4_000);
        early.probe.source_created_at = Some("2026-02-20T08:00:00Z".into());
        let p = plan(&st, &[src(late, vec![scene(1, 0, 0, 4_000, 0.9)]), src(early, vec![scene(2, 0, 0, 4_000, 0.9)])]);
        assert_eq!(p.clips.iter().map(|c| c.media_id).collect::<Vec<_>>(), [2, 1], "morning before afternoon, whatever the import order");
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
        let mut share: Vec<(String, i64)> = vec![];
        for c in &p.clips {
            println!("   #{} {:>6.1}-{:<6.1} rank {:.2} {:?}", c.media_id, c.in_ms as f64 / 1000.0, c.out_ms as f64 / 1000.0, c.rank, c.reasons);
            let t = c.tags.first().cloned().unwrap_or_default();
            match share.iter_mut().find(|(n, _)| *n == t) {
                Some(e) => e.1 += c.out_ms - c.in_ms,
                None => share.push((t, c.out_ms - c.in_ms)),
            }
        }
        share.sort_by_key(|(_, ms)| -ms);
        println!("   concepts: {}", share.iter().map(|(t, ms)| format!("{t} {:.0}s", *ms as f64 / 1000.0)).collect::<Vec<_>>().join(", "));
    }
}
