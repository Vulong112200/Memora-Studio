//! Free music library (owner request 2026-11-08): a curated catalog (sounds.json) of verified tracks —
//! source, licence, credit, size and SHA-256 recorded at development time. A track is downloaded the first
//! time an export uses it and kept in `<data root>\music\` (verified on every use). The user's own audio
//! files are never touched.

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
        "journey_recap" => &["gentle", "calm", "uplifting", "warm", "folk"],
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

/// Returns `<dir>\<id>.mp3`, downloading it once (owner decision 2026-11-08: keep library music, up to 2 GB,
/// so exports work offline and the artists' servers are not hit on every export). Size + SHA-256 are checked
/// on every use; a file that fails is deleted and downloaded again. Never writes outside `dir`.
pub fn fetch(t: &Track, dir: &Path, job_object: &JobObject, progress: &dyn Fn(f64), cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.mp3", t.id));
    let verified = |cancelled: &dyn Fn() -> bool| -> Result<bool, String> {
        let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        if size != t.size_bytes {
            return Ok(false);
        }
        Ok(sha256_file(&file, cancelled, &|_| {})?.ok_or("cancelled")?.eq_ignore_ascii_case(&t.sha256))
    };
    if verified(cancelled)? {
        progress(1.0);
        return Ok(file);
    }
    let _ = std::fs::remove_file(&file);
    match download(&t.url, &file, job_object, &|b| progress(b as f64 / t.size_bytes.max(1) as f64), cancelled) {
        Ok(()) => {}
        Err(Step::Cancelled) => return Err("cancelled".into()),
        Err(Step::Failed(_, e)) => return Err(format!("Cannot download \"{}\" (internet needed the first time): {e}", t.title)),
    }
    if !verified(cancelled)? {
        let _ = std::fs::remove_file(&file);
        return Err(format!("\"{}\" failed verification (size or SHA-256 differs from the catalog); not used.", t.title));
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
        assert_eq!(c.len(), 32);
        // Official artist sites only (verified 2026-11-08: file link and CC BY 4.0 on each track page).
        const SOURCES: [&str; 2] =
            ["https://incompetech.com/music/royalty-free/mp3-royaltyfree/", "https://www.scottbuckley.com.au/library/wp-content/uploads/"];
        for t in &c {
            assert!(SOURCES.iter().any(|s| t.url.starts_with(s)), "{}", t.url);
            assert!(t.sha256.len() == 64 && t.size_bytes > 0 && t.duration_ms > 30_000, "{}", t.id);
            assert!(t.license == "CC BY 4.0" && t.credit.contains(&t.title) && (t.credit.contains("By Attribution 4.0") || t.credit.contains("CC-BY 4.0")));
            assert!(["low", "medium", "high"].contains(&t.energy.as_str()) && !t.moods.is_empty());
        }
        let mut ids: Vec<_> = c.iter().map(|t| &t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 32, "unique ids");
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

    /// Kept file: verified and reused without a download; a damaged one is replaced (needs no network to fail cleanly).
    #[test]
    fn kept_track_is_reused_and_a_damaged_one_is_not() {
        let dir = std::env::temp_dir().join(format!("memora-sound-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = b"not really an mp3";
        let mut t = catalog().remove(0);
        t.size_bytes = body.len() as u64;
        t.sha256 = "8a1d3dbf6c64d5d74fb4d7e5c0d5bbb3a8c8ff3cba1ef0f55e5e7b3c3f9a4d21".into(); // wrong on purpose first
        t.url = "file:///nonexistent/never.mp3".into();
        std::fs::write(dir.join(format!("{}.mp3", t.id)), body).unwrap();
        let jo = JobObject::new().unwrap();
        assert!(fetch(&t, &dir, &jo, &|_| {}, &|| false).is_err(), "checksum mismatch → re-download → fails offline");
        assert!(!dir.join(format!("{}.mp3", t.id)).exists(), "damaged file removed");
        std::fs::write(dir.join(format!("{}.mp3", t.id)), body).unwrap();
        t.sha256 = sha256_file(&dir.join(format!("{}.mp3", t.id)), &|| false, &|_| {}).unwrap().unwrap();
        assert_eq!(fetch(&t, &dir, &jo, &|_| {}, &|| false).unwrap(), dir.join(format!("{}.mp3", t.id)), "verified copy reused, no download");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Real download of the smallest track (internet), verified and kept. Skipped offline.
    #[test]
    #[ignore]
    fn fetch_real_track() {
        let t = catalog().into_iter().min_by_key(|t| t.size_bytes).unwrap();
        let dir = std::env::temp_dir().join(format!("memora-sound-fetch-{}", std::process::id()));
        let f = fetch(&t, &dir, &JobObject::new().unwrap(), &|_| {}, &|| false).unwrap();
        assert!(f.exists(), "kept after use");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
