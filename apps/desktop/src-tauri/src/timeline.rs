//! Project timeline: an ordered list of trimmed segments of library videos and still photos (migration 11),
//! each with an optional transition into it. Edited by hand or filled from an Editing Plan (`editing::apply`).

use crate::db::err;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: i64,
    pub project_id: i64,
    /// Video clip: the library video; None for a photo.
    pub media_id: Option<i64>,
    /// Photo clip: the library photo (shown `out_ms - in_ms`, rendered with its edits); None for a video.
    pub photo_id: Option<i64>,
    pub position: i64,
    pub in_ms: i64,
    pub out_ms: i64,
    /// Text shown at the start of this clip in exports (lower third).
    pub caption: Option<String>,
    /// Transition into this clip (xfade name, one of `export::TRANSITIONS`); None = the project's transition.
    pub transition: Option<String>,
    pub transition_ms: Option<i64>,
}

impl Clip {
    /// The user's transition into this clip, if set.
    pub fn join(&self) -> Option<crate::export::Join> {
        self.transition.clone().map(|t| (t, self.transition_ms.unwrap_or(0)))
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Clip> {
    Ok(Clip {
        id: r.get(0)?, project_id: r.get(1)?, media_id: r.get(2)?, photo_id: r.get(3)?, position: r.get(4)?, in_ms: r.get(5)?, out_ms: r.get(6)?,
        caption: r.get(7)?, transition: r.get(8)?, transition_ms: r.get(9)?,
    })
}

const COLUMNS: &str = "id, project_id, media_id, photo_id, position, in_ms, out_ms, caption, transition, transition_ms";
/// How long a photo may stay on screen, and the default.
pub const PHOTO_MS: (i64, i64) = (1_000, 20_000);
pub const PHOTO_DEFAULT_MS: i64 = 4_000;

fn touch(c: &Connection, project_id: i64) -> Result<(), String> {
    c.execute("UPDATE projects SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1", [project_id]).map_err(err)?;
    Ok(())
}

pub fn list(c: &Connection, project_id: i64) -> Result<Vec<Clip>, String> {
    let mut s = c.prepare(&format!("SELECT {COLUMNS} FROM project_clips WHERE project_id = ?1 ORDER BY position, id")).map_err(err)?;
    s.query_map([project_id], row).map_err(err)?.collect::<Result<_, _>>().map_err(err)
}

/// Appends a segment. `out_ms` is clamped to the clip length.
pub fn add(c: &Connection, project_id: i64, media_id: i64, in_ms: i64, out_ms: i64) -> Result<Clip, String> {
    let duration: i64 = c
        .query_row("SELECT duration_ms FROM media WHERE id = ?1", [media_id], |r| r.get(0))
        .optional()
        .map_err(err)?
        .ok_or(format!("Media {media_id} not found."))?;
    let out_ms = out_ms.min(duration);
    if in_ms < 0 || out_ms - in_ms < 100 {
        return Err("A clip must be at least 0.1 s long (In must be before Out).".into());
    }
    insert(c, project_id, "media_id", media_id, in_ms, out_ms)
}

/// Appends a still photo shown `ms` (clamped to `PHOTO_MS`).
pub fn add_photo(c: &Connection, project_id: i64, photo_id: i64, ms: i64) -> Result<Clip, String> {
    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM photos WHERE id = ?1)", [photo_id], |r| r.get(0)).map_err(err)?;
    if !exists {
        return Err(format!("Photo {photo_id} not found."));
    }
    insert(c, project_id, "photo_id", photo_id, 0, ms.clamp(PHOTO_MS.0, PHOTO_MS.1))
}

fn insert(c: &Connection, project_id: i64, column: &str, source: i64, in_ms: i64, out_ms: i64) -> Result<Clip, String> {
    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)", [project_id], |r| r.get(0)).map_err(err)?;
    if !exists {
        return Err(format!("Project {project_id} not found."));
    }
    let clip = c
        .query_row(
            &format!(
                "INSERT INTO project_clips (project_id, {column}, position, in_ms, out_ms)
                 VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM project_clips WHERE project_id = ?1), ?3, ?4)
                 RETURNING {COLUMNS}"
            ),
            params![project_id, source, in_ms, out_ms],
            row,
        )
        .map_err(err)?;
    touch(c, project_id)?;
    Ok(clip)
}

/// Sets (name + length) or clears (None) the transition into one clip. Length 0 = hard cut.
pub fn set_transition(c: &Connection, clip_id: i64, transition: Option<&str>, ms: i64) -> Result<Clip, String> {
    if let Some(t) = transition {
        if !crate::export::TRANSITIONS.contains(&t) {
            return Err(format!("Unknown transition: {t}"));
        }
        if !(0..=3_000).contains(&ms) {
            return Err("A transition lasts 0 to 3 s.".into());
        }
    }
    let clip = c
        .query_row(
            &format!("UPDATE project_clips SET transition = ?2, transition_ms = ?3 WHERE id = ?1 RETURNING {COLUMNS}"),
            params![clip_id, transition, transition.map(|_| ms)],
            row,
        )
        .optional()
        .map_err(err)?
        .ok_or(format!("Clip {clip_id} not found."))?;
    touch(c, clip.project_id)?;
    Ok(clip)
}

/// New In / Out of one clip (same rules as `add`: Out clamped to the media length, at least 0.1 s; a photo's length to `PHOTO_MS`).
pub fn set_range(c: &Connection, clip_id: i64, in_ms: i64, out_ms: i64) -> Result<Clip, String> {
    let (project_id, duration): (i64, i64) = c
        .query_row(
            &format!("SELECT p.project_id, COALESCE(m.duration_ms, {}) FROM project_clips p LEFT JOIN media m ON m.id = p.media_id WHERE p.id = ?1", PHOTO_MS.1),
            [clip_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?
        .ok_or(format!("Clip {clip_id} not found."))?;
    let out_ms = out_ms.min(duration);
    if in_ms < 0 || out_ms - in_ms < 100 {
        return Err("A clip must be at least 0.1 s long (In must be before Out).".into());
    }
    let clip = c
        .query_row(
            &format!("UPDATE project_clips SET in_ms = ?2, out_ms = ?3 WHERE id = ?1 RETURNING {COLUMNS}"),
            params![clip_id, in_ms, out_ms],
            row,
        )
        .map_err(err)?;
    touch(c, project_id)?;
    Ok(clip)
}

/// Sets (or clears, with empty text) the caption of one clip.
pub fn set_caption(c: &Connection, clip_id: i64, caption: &str) -> Result<(), String> {
    let text = caption.trim();
    if text.chars().count() > 200 {
        return Err("Caption is limited to 200 characters.".into());
    }
    let n = c
        .execute("UPDATE project_clips SET caption = ?2 WHERE id = ?1", rusqlite::params![clip_id, (!text.is_empty()).then_some(text)])
        .map_err(err)?;
    if n == 0 { Err("Clip not found.".into()) } else { Ok(()) }
}

pub fn remove(c: &Connection, clip_id: i64) -> Result<(), String> {
    let project_id: Option<i64> =
        c.query_row("DELETE FROM project_clips WHERE id = ?1 RETURNING project_id", [clip_id], |r| r.get(0)).optional().map_err(err)?;
    touch(c, project_id.ok_or(format!("Clip {clip_id} not found."))?)
}

/// Swaps the clip with its neighbour (`delta` -1 = earlier, +1 = later). No-op at either end.
pub fn move_clip(c: &mut Connection, clip_id: i64, delta: i64) -> Result<(), String> {
    let tx = c.transaction().map_err(err)?;
    let (project_id, position): (i64, i64) = tx
        .query_row("SELECT project_id, position FROM project_clips WHERE id = ?1", [clip_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(err)?
        .ok_or(format!("Clip {clip_id} not found."))?;
    let order = if delta < 0 { "position < ?2 ORDER BY position DESC" } else { "position > ?2 ORDER BY position" };
    let neighbour: Option<(i64, i64)> = tx
        .query_row(
            &format!("SELECT id, position FROM project_clips WHERE project_id = ?1 AND {order} LIMIT 1"),
            [project_id, position],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((other, other_pos)) = neighbour {
        tx.execute("UPDATE project_clips SET position = ?1 WHERE id = ?2", [other_pos, clip_id]).map_err(err)?;
        tx.execute("UPDATE project_clips SET position = ?1 WHERE id = ?2", [position, other]).map_err(err)?;
        touch(&tx, project_id)?;
    }
    tx.commit().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::media::{Probe, upsert};
    use crate::projects;

    fn setup() -> (Connection, i64, i64) {
        let c = open_in_memory();
        let p = projects::create(&c, "Trip").unwrap();
        let probe = Probe {
            duration_ms: 10_000, width: 1920, height: 1080, rotation: 0, fps: Some(30.0), video_codec: "h264".into(),
            audio_codec: None, audio_streams: 0, bitrate: None, source_created_at: None,
        };
        let m = upsert(&c, r"E:\v.mp4", 1, None, &probe, "{}").unwrap();
        (c, p.id, m.id)
    }

    #[test]
    fn add_validates_and_clamps() {
        let (c, p, m) = setup();
        let clip = add(&c, p, m, 1000, 99_000).unwrap();
        assert_eq!((clip.position, clip.in_ms, clip.out_ms), (0, 1000, 10_000), "out clamped to duration");
        assert!(add(&c, p, m, 5000, 5000).is_err());
        assert!(add(&c, p, 999, 0, 1000).is_err());
        assert!(add(&c, 999, m, 0, 1000).is_err());
        assert_eq!(add(&c, p, m, 0, 1000).unwrap().position, 1);
    }

    #[test]
    fn set_range_validates_and_clamps() {
        let (c, p, m) = setup();
        let id = add(&c, p, m, 0, 1000).unwrap().id;
        let clip = set_range(&c, id, 2000, 99_000).unwrap();
        assert_eq!((clip.in_ms, clip.out_ms, clip.position), (2000, 10_000, 0), "out clamped, position kept");
        assert!(set_range(&c, id, 5000, 5050).is_err());
        assert!(set_range(&c, id, -1, 1000).is_err());
        assert!(set_range(&c, 999, 0, 1000).is_err());
        assert_eq!(list(&c, p).unwrap()[0].in_ms, 2000, "failed updates change nothing");
    }

    #[test]
    fn photos_and_per_join_transitions() {
        let (c, p, m) = setup();
        let photo = crate::photos::upsert(&c, r"E:\a.jpg", 1, None).unwrap();
        add(&c, p, m, 0, 1000).unwrap();
        let clip = add_photo(&c, p, photo, 60_000).unwrap();
        assert_eq!((clip.media_id, clip.photo_id, clip.in_ms, clip.out_ms, clip.position), (None, Some(photo), 0, PHOTO_MS.1, 1), "length clamped");
        assert!(add_photo(&c, p, 999, 4000).is_err());
        assert_eq!(set_range(&c, clip.id, 0, 3_000).unwrap().out_ms, 3_000);
        let t = set_transition(&c, clip.id, Some("fadeblack"), 1_200).unwrap();
        assert_eq!(t.join(), Some(("fadeblack".to_string(), 1_200)));
        assert!(set_transition(&c, clip.id, Some("spin"), 500).is_err());
        assert!(set_transition(&c, clip.id, Some("fade"), 9_000).is_err());
        assert_eq!(set_transition(&c, clip.id, None, 0).unwrap().join(), None, "back to the project's transition");
        assert!(set_transition(&c, 999, None, 0).is_err());
    }

    #[test]
    fn move_remove_and_cascade() {
        let (mut c, p, m) = setup();
        let a = add(&c, p, m, 0, 1000).unwrap().id;
        let b = add(&c, p, m, 1000, 2000).unwrap().id;
        let d = add(&c, p, m, 2000, 3000).unwrap().id;
        move_clip(&mut c, d, -1).unwrap();
        move_clip(&mut c, a, -1).unwrap(); // already first: no-op
        let ids = |c: &Connection| list(c, p).unwrap().iter().map(|x| x.id).collect::<Vec<_>>();
        assert_eq!(ids(&c), vec![a, d, b]);
        remove(&c, d).unwrap();
        assert_eq!(ids(&c), vec![a, b]);
        assert!(remove(&c, d).is_err());
        crate::media::remove(&c, m).unwrap();
        assert!(ids(&c).is_empty(), "removing media from the library removes its clips");
    }
}
