//! Application settings stored in the per-user config dir (`%APPDATA%\dev.memora.studio\settings.json`).
//! Only the data root lives here; everything else lives under the data root.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Folders created under the data root (docs/Production.md §4.4).
pub const SUBDIRS: [&str; 6] = ["runtime", "models", "projects", "cache", "logs", "database"];

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// `None` until the user picks a data folder on first run. Nothing is written to disk before that.
    pub data_root: Option<PathBuf>,
}

fn file(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

/// Missing or unreadable settings fall back to defaults (first run).
pub fn load(config_dir: &Path) -> Settings {
    fs::read_to_string(file(config_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| format!("Cannot create config folder: {e}"))?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(file(config_dir), json).map_err(|e| format!("Cannot write settings: {e}"))
}

/// Suggested default shown on first run. Not created until the user confirms.
pub fn default_data_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("MemoraStudio")
}

/// Validate a user-supplied folder and create the data root layout in it.
pub fn prepare_data_root(input: &str, app_dir: Option<&Path>) -> Result<PathBuf, String> {
    let root = PathBuf::from(input.trim());
    if !root.is_absolute() {
        return Err("The data folder must be an absolute path, e.g. E:\\Memora Data".into());
    }
    if app_dir.is_some_and(|dir| root.starts_with(dir)) {
        return Err("The data folder must not be inside the application folder (it is replaced on updates).".into());
    }
    for sub in SUBDIRS {
        let dir = root.join(sub);
        fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("memora-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_settings_are_default() {
        assert_eq!(load(&temp("missing")), Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let dir = temp("roundtrip");
        let s = Settings { data_root: Some(PathBuf::from(r"D:\MemoraStudio")) };
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir), s);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_relative_path() {
        assert!(prepare_data_root("MemoraStudio", None).is_err());
    }

    #[test]
    fn rejects_folder_inside_app_dir() {
        let app = temp("app");
        assert!(prepare_data_root(app.join("data").to_str().unwrap(), Some(&app)).is_err());
    }

    #[test]
    fn creates_layout() {
        let root = temp("layout");
        let got = prepare_data_root(root.to_str().unwrap(), None).unwrap();
        assert_eq!(got, root);
        assert!(SUBDIRS.iter().all(|d| root.join(d).is_dir()));
        fs::remove_dir_all(root).unwrap();
    }
}
