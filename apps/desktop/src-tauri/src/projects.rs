//! Project manager. A project groups editing decisions; deleting one never touches original media.

use crate::db::err;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    /// RFC 3339 UTC.
    pub created_at: String,
    pub updated_at: String,
}

const COLUMNS: &str = "id, name, created_at, updated_at";
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

fn from_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, updated_at: r.get(3)? })
}

fn clean_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    match name.chars().count() {
        0 => Err("Project name is required.".into()),
        n if n > 200 => Err("Project name must be at most 200 characters.".into()),
        _ => Ok(name),
    }
}

pub fn list(c: &Connection) -> Result<Vec<Project>, String> {
    let mut stmt = c
        .prepare(&format!("SELECT {COLUMNS} FROM projects ORDER BY updated_at DESC, id DESC"))
        .map_err(err)?;
    stmt.query_map([], from_row).map_err(err)?.collect::<Result<_, _>>().map_err(err)
}

pub fn create(c: &Connection, name: &str) -> Result<Project, String> {
    c.query_row(&format!("INSERT INTO projects (name) VALUES (?1) RETURNING {COLUMNS}"), [clean_name(name)?], from_row)
        .map_err(err)
}

pub fn rename(c: &Connection, id: i64, name: &str) -> Result<Project, String> {
    c.query_row(
        &format!("UPDATE projects SET name = ?1, updated_at = {NOW} WHERE id = ?2 RETURNING {COLUMNS}"),
        params![clean_name(name)?, id],
        from_row,
    )
    .optional()
    .map_err(err)?
    .ok_or_else(|| format!("Project {id} not found."))
}

pub fn delete(c: &Connection, id: i64) -> Result<(), String> {
    match c.execute("DELETE FROM projects WHERE id = ?1", [id]).map_err(err)? {
        0 => Err(format!("Project {id} not found.")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn create_list_rename_delete() {
        let c = open_in_memory();
        let a = create(&c, "  Đà Lạt 2026  ").unwrap();
        assert_eq!(a.name, "Đà Lạt 2026");
        assert!(a.created_at.ends_with('Z'));
        let b = create(&c, "Tokyo").unwrap();
        assert_eq!(list(&c).unwrap().iter().map(|p| p.id).collect::<Vec<_>>(), vec![b.id, a.id]);
        // Timestamps have millisecond resolution; age the rows so the rename is strictly newer.
        c.execute("UPDATE projects SET updated_at = '2026-01-01T00:00:00.000Z'", []).unwrap();
        let b = list(&c).unwrap().into_iter().find(|p| p.id == b.id).unwrap();

        let renamed = rename(&c, a.id, "Đà Lạt trip").unwrap();
        assert_eq!(renamed.name, "Đà Lạt trip");
        assert!(renamed.updated_at > b.updated_at);
        assert_eq!(list(&c).unwrap()[0].id, a.id, "recently updated first");

        delete(&c, a.id).unwrap();
        assert_eq!(list(&c).unwrap(), vec![b]);
    }

    #[test]
    fn rejects_invalid_names() {
        let c = open_in_memory();
        assert!(create(&c, "   ").is_err());
        assert!(create(&c, &"x".repeat(201)).is_err());
        assert!(create(&c, &"ệ".repeat(200)).is_ok(), "limit counts characters, not bytes");
    }

    #[test]
    fn missing_project_errors() {
        let c = open_in_memory();
        assert!(rename(&c, 42, "x").unwrap_err().contains("not found"));
        assert!(delete(&c, 42).unwrap_err().contains("not found"));
    }
}
