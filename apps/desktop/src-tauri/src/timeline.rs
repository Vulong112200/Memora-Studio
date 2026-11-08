//! Project timeline: an ordered list of trimmed segments of library media.
//! Phase 3 keeps it manual; the Editing Core (Phase 6) will produce the same structure from an Editing Plan.

use crate::db::err;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: i64,
    pub project_id: i64,
    pub media_id: i64,
    pub position: i64,
    pub in_ms: i64,
    pub out_ms: i64,
    /// Text shown at the start of this clip in exports (lower third).
    pub caption: Option<String>,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Clip> {
    Ok(Clip { id: r.get(0)?, project_id: r.get(1)?, media_id: r.get(2)?, position: r.get(3)?, in_ms: r.get(4)?, out_ms: r.get(5)?, caption: r.get(6)? })
}

const COLUMNS: &str = "id, project_id, media_id, position, in_ms, out_ms, caption";

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
    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)", [project_id], |r| r.get(0)).map_err(err)?;
    if !exists {
        return Err(format!("Project {project_id} not found."));
    }
    let clip = c
        .query_row(
            &format!(
                "INSERT INTO project_clips (project_id, media_id, position, in_ms, out_ms)
                 VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM project_clips WHERE project_id = ?1), ?3, ?4)
                 RETURNING {COLUMNS}"
            ),
            params![project_id, media_id, in_ms, out_ms],
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
