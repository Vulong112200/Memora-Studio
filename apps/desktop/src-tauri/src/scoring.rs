//! Deterministic technical scoring and duplicate detection (docs/Production.md §5.4, §6.1).
//! Pure functions of stored observations: same observations → same scores. Components stay
//! separate; nothing is discarded automatically (a blurry clip can still be an important memory).
//!
//! Calibration: thresholds below are INITIAL ASSUMPTIONS set from synthetic tests and the user's
//! DJI samples (Laplacian variance 1 500–3 800 at 540p for sharp daylight footage). They are not
//! validated on a broad footage set; tune them when real libraries are analysed.

use crate::analysis::{Sample, Scene};
use serde::Serialize;

/// Hamming distance (of 64-bit dHashes) at or below which two scenes count as duplicates.
pub const DUPLICATE_MAX_DISTANCE: u32 = 6;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Technical {
    /// Each component is 0..1, higher is better.
    pub sharpness: f64,
    pub exposure: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub stability: f64,
    /// Weighted combination of the components above; 0 for black scenes.
    pub total: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Duplicate {
    pub media_id: i64,
    pub scene_index: i64,
    pub distance: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredScene {
    #[serde(flatten)]
    pub scene: Scene,
    pub technical: Technical,
    /// Observations for the UI and the Editing Core, never automatic exclusions.
    pub flags: Vec<&'static str>,
    pub duplicates: Vec<Duplicate>,
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Technical score of one scene from its samples and shake.
pub fn technical(samples: &[Sample], shake: f64) -> (Technical, Vec<&'static str>) {
    let m = |f: fn(&Sample) -> f64| median(samples.iter().map(f).collect());
    let sharp = m(|s| s.sharpness);
    let bright = m(|s| s.brightness);
    let clipped = m(|s| s.clip_low + s.clip_high);
    let contrast = m(|s| s.contrast);
    let black_share = samples.iter().filter(|s| s.black).count() as f64 / samples.len().max(1) as f64;

    // log10(Laplacian variance): ≤1.5 (≈30) is very soft, ≥3.5 (≈3 200) is crisp at 540p.
    let sharpness = clamp01((sharp.max(1.0).log10() - 1.5) / 2.0);
    // 25 % of pixels crushed/blown → 0.
    let exposure = clamp01(1.0 - clipped * 4.0);
    // Mean luma 0.25..0.65 is fine; falls to 0 at 0.0 / 0.9.
    let brightness = clamp01(1.0 - ((bright - 0.45).abs() - 0.2).max(0.0) / 0.25);
    let contrast_s = clamp01(contrast / 0.18);
    // Shake = jitter as a fraction of frame width; 0.02 (2 %) is very shaky.
    let stability = clamp01(1.0 - shake / 0.02);
    let black = black_share > 0.5;
    let total = if black || samples.is_empty() {
        0.0
    } else {
        0.35 * sharpness + 0.2 * exposure + 0.15 * brightness + 0.1 * contrast_s + 0.2 * stability
    };

    let mut flags = vec![];
    if samples.is_empty() {
        flags.push("not sampled");
    }
    if black {
        flags.push("black");
    } else if !samples.is_empty() {
        if sharpness < 0.3 {
            flags.push("blurry");
        }
        if stability < 0.4 {
            flags.push("shaky");
        }
        if bright < 0.2 {
            flags.push("dark");
        }
        if clipped > 0.15 && m(|s| s.clip_high) > m(|s| s.clip_low) {
            flags.push("overexposed");
        }
    }
    let t = Technical {
        sharpness: round3(sharpness),
        exposure: round3(exposure),
        brightness: round3(brightness),
        contrast: round3(contrast_s),
        stability: round3(stability),
        total: round3(total),
    };
    (t, flags)
}

pub fn hamming(a: &str, b: &str) -> Option<u32> {
    Some((u64::from_str_radix(a, 16).ok()? ^ u64::from_str_radix(b, 16).ok()?).count_ones())
}

/// dHash of the sample nearest the scene middle (same rule the worker uses for the saved frame).
pub fn representative_hash(scene: &Scene) -> Option<&str> {
    let mid = (scene.start_ms + scene.end_ms) / 2;
    scene.samples.iter().min_by_key(|s| (s.at_ms - mid).abs()).map(|s| s.dhash.as_str())
}

/// Scores `scenes` and finds near-identical scenes anywhere in `library` (excluding themselves).
// ponytail: O(scenes × library) scan; index hashes (BK-tree) if libraries reach ~100k scenes.
pub fn score(scenes: Vec<Scene>, library: &[(i64, i64, String)]) -> Vec<ScoredScene> {
    scenes
        .into_iter()
        .map(|scene| {
            let (technical, mut flags) = technical(&scene.samples, scene.shake);
            let duplicates: Vec<Duplicate> = representative_hash(&scene)
                .map(|h| {
                    library
                        .iter()
                        .filter(|(mid, idx, _)| (*mid, *idx) != (scene.media_id, scene.index))
                        .filter_map(|(mid, idx, other)| {
                            let d = hamming(h, other)?;
                            (d <= DUPLICATE_MAX_DISTANCE).then_some(Duplicate { media_id: *mid, scene_index: *idx, distance: d })
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !duplicates.is_empty() {
                flags.push("duplicate");
            }
            ScoredScene { scene, technical, flags, duplicates }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at_ms: i64, sharpness: f64, brightness: f64, dhash: &str) -> Sample {
        Sample {
            at_ms, sharpness, brightness, contrast: 0.2, clip_low: 0.0, clip_high: 0.0, black: false, dhash: dhash.into(),
        }
    }

    fn scene(media_id: i64, index: i64, samples: Vec<Sample>, shake: f64) -> Scene {
        Scene { id: index, media_id, index, start_ms: 0, end_ms: 2000, shake, frame_path: None, samples }
    }

    #[test]
    fn sharp_steady_beats_blurry_shaky_and_flags_them() {
        let (good, gf) = technical(&[s(0, 3000.0, 0.5, "0"), s(1000, 2500.0, 0.5, "0")], 0.001);
        let (bad, bf) = technical(&[s(0, 40.0, 0.5, "0")], 0.03);
        assert!(good.total > 0.85 && bad.total < 0.5, "{good:?} {bad:?}");
        assert!(gf.is_empty(), "{gf:?}");
        assert_eq!(bf, vec!["blurry", "shaky"]);
        assert!(bad.total > 0.0, "imperfect footage is scored low, never zeroed or dropped");
    }

    #[test]
    fn black_dark_overexposed() {
        let mut b = s(0, 5.0, 0.02, "0");
        b.black = true;
        let (t, f) = technical(&[b.clone(), b], 0.0);
        assert_eq!((t.total, f), (0.0, vec!["black"]));
        let (_, f) = technical(&[s(0, 2000.0, 0.1, "0")], 0.0);
        assert_eq!(f, vec!["dark"]);
        let mut o = s(0, 2000.0, 0.85, "0");
        o.clip_high = 0.3;
        let (t, f) = technical(&[o], 0.0);
        assert!(f.contains(&"overexposed") && t.exposure == 0.0);
        let (t, f) = technical(&[], 0.0);
        assert_eq!((t.total, f), (0.0, vec!["not sampled"]));
    }

    #[test]
    fn deterministic() {
        let samples = vec![s(0, 1234.5, 0.37, "0"), s(500, 987.0, 0.41, "0"), s(1000, 1500.0, 0.39, "0")];
        assert_eq!(technical(&samples, 0.004), technical(&samples, 0.004));
    }

    #[test]
    fn duplicates_by_dhash() {
        assert_eq!(hamming("ffffffffffffffff", "fffffffffffffff0"), Some(4));
        assert_eq!(hamming("zz", "00"), None);
        let a = scene(1, 0, vec![s(1000, 2000.0, 0.5, "f0f0f0f0f0f0f0f0")], 0.0);
        let library = vec![
            (1, 0, "f0f0f0f0f0f0f0f0".to_string()), // itself
            (2, 3, "f0f0f0f0f0f0f0f1".to_string()), // 1 bit off → duplicate
            (2, 4, "0f0f0f0f0f0f0f0f".to_string()), // different
        ];
        let r = score(vec![a], &library);
        assert_eq!(r[0].duplicates.len(), 1);
        assert_eq!((r[0].duplicates[0].media_id, r[0].duplicates[0].scene_index, r[0].duplicates[0].distance), (2, 3, 1));
        assert!(r[0].flags.contains(&"duplicate"));
    }
}
