//! Free music library (owner request 2026-11-08): a curated catalog (sounds.json) of verified tracks —
//! source, licence, credit, size and SHA-256 recorded at development time. A track is downloaded only for
//! an export that uses it, verified, rendered, then deleted. The user's own audio files are never touched.

use crate::models::{Step, download, sha256_file};
use crate::process::JobObject;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CATALOG: &str = include_str!("../sounds.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    /// Developer-assigned character labels (assumptions; the user listens before choosing).
    pub moods: Vec<String>,
    /// "low" | "medium" | "high".
    pub energy: String,
    pub duration_ms: i64,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub license: String,
    pub license_url: String,
    /// Credit line required by the licence; burned into the end of the video.
    pub credit: String,
}

pub fn catalog() -> Vec<Track> {
    serde_json::from_str(CATALOG).expect("sounds.json is valid")
}

pub fn track(id: &str) -> Result<Track, String> {
    catalog().into_iter().find(|t| t.id == id).ok_or(format!("Unknown library track: {id}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub id: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

/// Moods that fit each editing style (editing.rs ids).
fn style_moods(style: &str) -> &'static [&'static str] {
    match style {
        "travel_cinematic" => &["cinematic", "uplifting", "adventure", "epic", "calm"],
        "social_story" => &["upbeat", "happy", "playful"],
        "memory_nostalgia" => &["nostalgic", "emotional", "warm", "gentle", "calm"],
        "travel_documentary" => &["calm", "chill", "adventure", "light", "folk"],
        _ => &["calm", "chill", "light", "ambient"],
    }
}

/// Moods suggested by what is on screen (OpenCLIP tag `en` names).
fn content_moods(tag: &str) -> &'static [&'static str] {
    match tag {
        "people" | "group of friends" | "selfie" | "birthday party" | "food" | "drinks" | "fireworks" | "concert" => &["happy", "upbeat", "playful"],
        "family" | "child" | "wedding" | "dog" | "cat" => &["warm", "nostalgic", "happy"],
        "bicycle" | "motorbike" | "sports" | "road trip on a highway" | "hiking trail" | "camping" => &["adventure", "upbeat", "folk"],
        "sunset" | "sunrise" | "sea" | "beach" | "lake" | "sky and clouds" | "aerial view" | "mountain" | "waterfall" => &["cinematic", "calm", "dreamy"],
        "temple" | "church" | "forest" | "big tree" | "flowers" | "rice field" => &["calm", "ambient", "gentle"],
        "city street" | "market" | "night city lights" | "restaurant" => &["chill", "upbeat", "light"],
        _ => &[],
    }
}

/// Ranks the library for one video: style fit (50 %), on-screen content (30 %), length (10 %) and
/// talk-friendliness (10 %: calm tracks when there is much speech). Deterministic; top `n` with reasons.
pub fn recommend(style: &str, tags: &[(String, f64)], video_ms: i64, speech_share: f64, n: usize) -> Vec<Recommendation> {
    let wanted = style_moods(style);
    let total_tag: f64 = tags.iter().map(|(_, s)| s).sum::<f64>().max(1e-9);
    let mut out: Vec<Recommendation> = catalog()
        .into_iter()
        .map(|t| {
            let mut reasons = vec![];
            let style_fit = t.moods.iter().filter(|m| wanted.contains(&m.as_str())).count() as f64 / t.moods.len().max(1) as f64;
            if style_fit > 0.0 {
                reasons.push(format!("{} fits this style", t.moods.join(" / ")));
            }
            let mut content = 0.0;
            let mut matched = vec![];
            for (tag, s) in tags {
                if t.moods.iter().any(|m| content_moods(tag).contains(&m.as_str())) {
                    content += s / total_tag;
                    matched.push(tag.as_str());
                }
            }
            if !matched.is_empty() {
                reasons.push(format!("matches {}", matched.iter().take(3).copied().collect::<Vec<_>>().join(", ")));
            }
            let length = if t.duration_ms >= video_ms {
                reasons.push("long enough, no loop".into());
                1.0
            } else {
                0.3
            };
            let talk = match (speech_share > 0.3, t.energy.as_str()) {
                (true, "low") => {
                    reasons.push("quiet under speech".into());
                    1.0
                }
                (true, "high") => 0.0,
                _ => 0.6,
            };
            let score = 0.5 * style_fit + 0.3 * content.min(1.0) + 0.1 * length + 0.1 * talk;
            Recommendation { id: t.id, score: (score * 1000.0).round() / 1000.0, reasons }
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
    out.truncate(n);
    out
}

/// Downloads `t` to a unique file in `dir`, verifies size + SHA-256, and returns its path. The caller
/// deletes it after rendering (see `TempFile`). Never writes outside `dir`.
pub fn fetch(t: &Track, dir: &Path, job_object: &JobObject, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<TempFile, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let file = TempFile(dir.join(format!("{}-{}-{nanos}.mp3", t.id, std::process::id())));
    match download(&t.url, &file.0, job_object, &|b| progress(b as f64 / t.size_bytes.max(1) as f64), cancelled) {
        Ok(()) => {}
        Err(Step::Cancelled) => return Err("cancelled".into()),
        Err(Step::Failed(_, e)) => return Err(format!("Cannot download \"{}\" (internet needed for library music): {e}", t.title)),
    }
    let size = std::fs::metadata(&file.0).map(|m| m.len()).unwrap_or(0);
    let hash = sha256_file(&file.0, cancelled, &|_| {})?.ok_or("cancelled")?;
    if size != t.size_bytes || !hash.eq_ignore_ascii_case(&t.sha256) {
        return Err(format!("\"{}\" failed verification (size {size}, SHA-256 {hash}); not used.", t.title));
    }
    Ok(file)
}

/// A downloaded library file, deleted when dropped (after the render, whatever its outcome).
pub struct TempFile(pub PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("part"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_and_verified() {
        let c = catalog();
        assert_eq!(c.len(), 22);
        for t in &c {
            assert!(t.url.starts_with("https://incompetech.com/music/royalty-free/mp3-royaltyfree/"), "{}", t.url);
            assert!(t.sha256.len() == 64 && t.size_bytes > 0 && t.duration_ms > 30_000, "{}", t.id);
            assert!(t.credit.contains(&t.title) && t.credit.contains("By Attribution 4.0"));
            assert!(["low", "medium", "high"].contains(&t.energy.as_str()) && !t.moods.is_empty());
        }
        let mut ids: Vec<_> = c.iter().map(|t| &t.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 22, "unique ids");
    }

    #[test]
    fn recommendations_follow_style_content_length_and_speech() {
        let party = vec![("group of friends".to_string(), 0.6), ("food".to_string(), 0.3)];
        let r = recommend("social_story", &party, 30_000, 0.0, 3);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|x| track(&x.id).unwrap().moods.iter().any(|m| ["upbeat", "happy", "playful"].contains(&m.as_str()))), "{r:?}");
        let calm = recommend("memory_nostalgia", &[("family".to_string(), 0.8)], 180_000, 0.6, 3);
        assert!(calm.iter().all(|x| track(&x.id).unwrap().energy != "high"), "quiet music under speech: {calm:?}");
        assert!(calm[0].reasons.iter().any(|s| s.contains("fits this style")));
        assert_eq!(recommend("travel_cinematic", &[], 90_000, 0.0, 5).len(), 5);
        assert_eq!(serde_json::to_string(&recommend("travel_cinematic", &party, 90_000, 0.1, 5)).unwrap(),
            serde_json::to_string(&recommend("travel_cinematic", &party, 90_000, 0.1, 5)).unwrap(), "deterministic");
    }

    #[test]
    fn temp_file_is_deleted_on_drop() {
        let p = std::env::temp_dir().join(format!("memora-sound-{}.mp3", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        drop(TempFile(p.clone()));
        assert!(!p.exists());
    }

    /// Real download of the smallest track (internet), verify, delete. Skipped offline.
    #[test]
    #[ignore]
    fn fetch_real_track() {
        let t = catalog().into_iter().min_by_key(|t| t.size_bytes).unwrap();
        let dir = std::env::temp_dir().join(format!("memora-sound-fetch-{}", std::process::id()));
        let f = fetch(&t, &dir, &JobObject::new().unwrap(), &|_| {}, &|| false).unwrap();
        assert!(f.0.exists());
        let path = f.0.clone();
        drop(f);
        assert!(!path.exists(), "deleted after use");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
