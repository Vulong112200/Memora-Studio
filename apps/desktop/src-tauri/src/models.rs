//! Model Manager (docs/Production.md §4): manifests, install pipeline, offline install, removal.
//! Pipeline: download (resumable, Windows' built-in curl.exe) or copy a local file → SHA-256 + size
//! check → install into `<data root>\models\<category>\<id>\<version>\` → load validation.
//! Downloads are possible only for artifacts with a verified URL, SHA-256 and size in models.json,
//! and the UI asks the user to confirm every download (name, size, license, source).

use crate::db::err;
use crate::jobs::{Job, Outcome, Status};
use crate::process::{JobObject, command};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

const MANIFESTS: &str = include_str!("../models.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub file: String,
    pub url: Option<String>,
    pub revision: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSupport {
    pub runtime: String,
    pub devices: Vec<String>,
    pub validated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub category: String,
    pub description: String,
    pub required: bool,
    pub license: String,
    pub minimum_ram_gb: u32,
    /// "openvino-ir" (worker compiles and runs it) or "none" (checksum only).
    pub validation: String,
    pub artifacts: Vec<Artifact>,
    pub runtimes: Vec<RuntimeSupport>,
    pub notes: String,
}

impl Manifest {
    /// Why this model cannot be downloaded, or None when every artifact is verified.
    pub fn download_blocker(&self) -> Option<&'static str> {
        let ok = self.artifacts.iter().all(|a| a.url.is_some() && a.sha256.is_some() && a.size_bytes.is_some());
        (!ok).then_some("No verified download source yet: the model proposal (source, license, checksum) must be approved first.")
    }
}

pub fn manifests() -> Vec<Manifest> {
    serde_json::from_str(MANIFESTS).expect("models.json is valid")
}

fn manifest(id: &str) -> Result<Manifest, String> {
    manifests().into_iter().find(|m| m.id == id).ok_or(format!("Unknown model: {id}"))
}

/// Manifest + install state, for the UI.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// "not_installed", "downloading", "verifying", "installing", "validating", "ready",
    /// "download_failed", "checksum_failed", "install_failed", "model_load_failed", "incompatible".
    pub state: String,
    pub error: Option<String>,
    pub path: Option<String>,
    pub details: Option<Value>,
    pub download_blocker: Option<&'static str>,
}

pub fn list(c: &Connection) -> Result<Vec<ModelView>, String> {
    manifests()
        .into_iter()
        .map(|m| {
            let row: Option<(String, Option<String>, Option<String>, Option<String>, String)> = c
                .query_row("SELECT state, error, path, details, version FROM models WHERE id = ?1", [&m.id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .optional()
                .map_err(err)?;
            let (mut state, mut error, path, details, version) = row.unwrap_or(("not_installed".into(), None, None, None, m.version.clone()));
            if state == "ready" && version != m.version {
                // Installed files still work, but the app expects the newer version's content.
                state = "update_available".into();
                error = Some(format!("Version {version} installed; version {} is available.", m.version));
            }
            Ok(ModelView {
                download_blocker: m.download_blocker(),
                manifest: m,
                state,
                error,
                path,
                details: details.and_then(|d| serde_json::from_str(&d).ok()),
            })
        })
        .collect()
}

/// Install folder of a model in state "ready", or a message telling the user what to install.
pub fn ready_dir(c: &Connection, id: &str) -> Result<(Manifest, PathBuf), String> {
    let m = list(c)?.into_iter().find(|m| m.manifest.id == id).ok_or(format!("Unknown model {id}"))?;
    match (m.state.as_str(), m.path) {
        ("ready", Some(dir)) => Ok((m.manifest, PathBuf::from(dir))),
        ("update_available", _) => Err(format!("Update {} in AI models first (a newer version is available).", m.manifest.name)),
        _ => Err(format!("Install {} in AI models first.", m.manifest.name)),
    }
}

fn set_state(db: &Mutex<Result<Connection, String>>, m: &Manifest, state: &str, error: Option<&str>, path: Option<&Path>, details: Option<&Value>) {
    let guard = db.lock().unwrap();
    if let Ok(c) = guard.as_ref() {
        let r = c.execute(
            "INSERT INTO models (id, version, state, error, path, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET version = excluded.version, state = excluded.state, error = excluded.error,
                path = excluded.path, details = excluded.details, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![m.id, m.version, state, error, path.map(|p| p.to_string_lossy().to_string()), details.map(|d| d.to_string())],
        );
        if let Err(e) = r {
            tracing::error!(model = m.id, error = %e, "cannot save model state");
        }
    }
}

/// Run before any job starts (app start, data-folder switch): rows still in a working state belong to an
/// install the app did not finish (crash or forced close). They become `install_failed`, so the user sees
/// what happened and can Remove or install again. Partial downloads stay for resuming.
pub fn reset_interrupted(c: &Connection) -> Result<usize, String> {
    c.execute(
        "UPDATE models SET state = 'install_failed',
            error = 'Interrupted: Memora Studio closed during installation. Install again.',
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE state IN ('downloading', 'verifying', 'installing', 'validating')",
        [],
    )
    .map_err(err)
}

fn clear_state(c: &Connection, id: &str) -> Result<(), String> {
    c.execute("DELETE FROM models WHERE id = ?1", [id]).map_err(err).map(drop)
}

pub fn install_dir(models_root: &Path, m: &Manifest) -> PathBuf {
    models_root.join(&m.category).join(&m.id).join(&m.version)
}

fn downloads_dir(models_root: &Path, m: &Manifest) -> PathBuf {
    models_root.join(".downloads").join(format!("{}-{}", m.id, m.version))
}

/// Streams the file through SHA-256 (lowercase hex). Checks cancellation between chunks.
pub fn sha256_file(path: &Path, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(u64)) -> Result<Option<String>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if cancelled() {
            return Ok(None);
        }
        let n = f.read(&mut buf).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done);
    }
    Ok(Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

pub(crate) enum Step {
    Cancelled,
    Failed(&'static str, String), // (failure state, message)
}

/// curl.exe download with resume (`-C -`) into `<file>.part`, renamed when complete.
/// Progress is the size of the partial file.
pub(crate) fn download(url: &str, dest: &Path, job_object: &JobObject, progress: &dyn Fn(u64), cancelled: &dyn Fn() -> bool) -> Result<(), Step> {
    let part = dest.with_extension("part");
    let mut cmd = command(Path::new("curl.exe"));
    cmd.args(["--location", "--fail", "--silent", "--show-error", "--retry", "3", "--continue-at", "-", "--output"])
        .arg(&part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = crate::process::spawn(&mut cmd, job_object).map_err(|e| Step::Failed("download_failed", format!("Cannot start curl.exe: {e}")))?;
    loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Step::Cancelled); // the .part file stays for resuming
        }
        progress(std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0));
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                let mut msg = String::new();
                let _ = child.stderr.take().map(|mut e| e.read_to_string(&mut msg));
                return Err(Step::Failed("download_failed", format!("Download failed ({status}): {}", msg.trim())));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(Step::Failed("download_failed", e.to_string())),
        }
    }
    std::fs::rename(&part, dest).map_err(|e| Step::Failed("download_failed", format!("Cannot finish download: {e}")))
}

/// Install pipeline for one model. `local_files` maps artifact file name → user-provided path
/// (offline install); the user's files are copied, never moved.
#[allow(clippy::too_many_arguments)]
pub fn install(
    m: &Manifest,
    local_files: Option<&serde_json::Map<String, Value>>,
    models_root: &Path,
    job_object: &JobObject,
    db: &Mutex<Result<Connection, String>>,
    validate: &mut dyn FnMut(&Manifest, &Path) -> Result<Value, String>,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Outcome {
    let fail = |state: &str, msg: String| {
        tracing::warn!(model = m.id, state, error = %msg, "model install failed");
        set_state(db, m, state, Some(&msg), None, None);
        Outcome::Failed(msg)
    };
    if local_files.is_none()
        && let Some(why) = m.download_blocker()
    {
        return Outcome::Failed(why.into());
    }
    // Sources: local file or downloaded copy.
    let dl_dir = downloads_dir(models_root, m);
    let mut sources = vec![];
    for a in &m.artifacts {
        match local_files {
            Some(files) => match files.get(&a.file).and_then(Value::as_str) {
                Some(p) => sources.push(PathBuf::from(p)),
                None => return fail("install_failed", format!("Missing local file for {}", a.file)),
            },
            None => sources.push(dl_dir.join(&a.file)),
        }
    }
    let sizes: Vec<u64> = match local_files {
        Some(_) => sources.iter().map(|p| std::fs::metadata(p).map(|x| x.len()).unwrap_or(0)).collect(),
        None => m.artifacts.iter().map(|a| a.size_bytes.unwrap_or(0)).collect(),
    };
    // Progress: download (if any) + hashing, in bytes.
    let total: u64 = sizes.iter().sum::<u64>() * if local_files.is_some() { 1 } else { 2 };
    progress(0, total);

    let mut offset = 0u64;
    if local_files.is_none() {
        set_state(db, m, "downloading", None, None, None);
        if let Err(e) = std::fs::create_dir_all(&dl_dir) {
            return fail("download_failed", format!("Cannot create {}: {e}", dl_dir.display()));
        }
        for (a, (src, size)) in m.artifacts.iter().zip(sources.iter().zip(&sizes)) {
            if !src.exists() {
                let base = offset;
                match download(a.url.as_deref().unwrap(), src, job_object, &|b| progress(base + b.min(*size), total), cancelled) {
                    Ok(()) => {}
                    Err(Step::Cancelled) => {
                        let _ = db.lock().unwrap().as_ref().map(|c| clear_state(c, &m.id));
                        return Outcome::Cancelled;
                    }
                    Err(Step::Failed(state, msg)) => return fail(state, msg),
                }
            }
            offset += size;
        }
    }

    set_state(db, m, "verifying", None, None, None);
    for (a, (src, size)) in m.artifacts.iter().zip(sources.iter().zip(&sizes)) {
        let base = offset;
        let actual_size = std::fs::metadata(src).map(|x| x.len()).unwrap_or(0);
        if let Some(expected) = a.size_bytes
            && expected != actual_size
        {
            if local_files.is_none() {
                let _ = std::fs::remove_file(src);
            }
            return fail("checksum_failed", format!("{}: size {actual_size} bytes, expected {expected}", a.file));
        }
        let hash = match sha256_file(src, cancelled, &|b| progress(base + b, total)) {
            Ok(Some(h)) => h,
            Ok(None) => {
                let _ = db.lock().unwrap().as_ref().map(|c| clear_state(c, &m.id));
                return Outcome::Cancelled;
            }
            Err(e) => return fail("install_failed", e),
        };
        if let Some(expected) = &a.sha256
            && !expected.eq_ignore_ascii_case(&hash)
        {
            if local_files.is_none() {
                let _ = std::fs::remove_file(src); // corrupted download: start over next time
            }
            return fail("checksum_failed", format!("{}: SHA-256 {hash} does not match the manifest", a.file));
        }
        offset += size;
    }

    set_state(db, m, "installing", None, None, None);
    let dest = install_dir(models_root, m);
    let _ = std::fs::remove_dir_all(&dest);
    if let Err(e) = std::fs::create_dir_all(&dest) {
        return fail("install_failed", format!("Cannot create {}: {e}", dest.display()));
    }
    for (a, src) in m.artifacts.iter().zip(&sources) {
        let target = dest.join(&a.file);
        let r = if local_files.is_some() { std::fs::copy(src, &target).map(drop) } else { std::fs::rename(src, &target) };
        if let Err(e) = r {
            let _ = std::fs::remove_dir_all(&dest);
            return fail("install_failed", format!("Cannot install {}: {e}", a.file));
        }
    }
    let _ = std::fs::remove_dir_all(&dl_dir);

    set_state(db, m, "validating", None, Some(&dest), None);
    match validate(m, &dest) {
        Ok(details) => {
            set_state(db, m, "ready", None, Some(&dest), Some(&details));
            progress(total, total);
            tracing::info!(model = m.id, path = %dest.display(), "model ready");
            Outcome::Completed(json!({"id": m.id, "path": dest, "details": details}))
        }
        Err(e) => {
            set_state(db, m, "model_load_failed", Some(&e), Some(&dest), None);
            tracing::warn!(model = m.id, error = %e, "model load validation failed");
            Outcome::Failed(format!("Model installed but failed to load: {e}"))
        }
    }
}

/// Deletes the installed files, partial downloads and the state row.
pub fn remove(c: &Connection, models_root: &Path, id: &str) -> Result<(), String> {
    let m = manifest(id)?;
    let _ = std::fs::remove_dir_all(install_dir(models_root, &m));
    let _ = std::fs::remove_dir_all(downloads_dir(models_root, &m));
    clear_state(c, id)?;
    tracing::info!(model = id, "model removed");
    Ok(())
}

/// Job `models.install`, params `{"id": "...", "files": {"<artifact file>": "<local path>"}?}`.
/// Validation of OpenVINO IR runs in the AI worker (`validate_openvino_ir`).
pub fn install_job(
    job: &Job,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
    models_root: &Path,
    job_object: &JobObject,
    db: &Mutex<Result<Connection, String>>,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
) -> Outcome {
    let m = match job.params["id"].as_str().map(manifest) {
        Some(Ok(m)) => m,
        Some(Err(e)) => return Outcome::Failed(e),
        None => return Outcome::Failed("models.install needs params.id".into()),
    };
    install_manifest_job(&m, job, progress, cancelled, models_root, job_object, db, ai)
}

#[allow(clippy::too_many_arguments)]
fn install_manifest_job(
    m: &Manifest,
    job: &Job,
    progress: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
    models_root: &Path,
    job_object: &JobObject,
    db: &Mutex<Result<Connection, String>>,
    ai: &mut dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome,
) -> Outcome {
    let mut validate = |m: &Manifest, dir: &Path| -> Result<Value, String> {
        match m.validation.as_str() {
            "none" => Ok(json!({"validation": "checksum only"})),
            "openvino-ir" => {
                let xml = m.artifacts.iter().find(|a| a.file.ends_with(".xml")).ok_or("manifest has no .xml artifact")?;
                let sub = Job {
                    id: job.id,
                    kind: "validate_openvino_ir".into(),
                    params: json!({"xml": dir.join(&xml.file)}),
                    status: Status::Running,
                    done: 0,
                    total: 0,
                    result: None,
                    error: None,
                };
                match ai(&sub, &|_, _| {}, &|| false) {
                    Outcome::Completed(v) => Ok(v),
                    Outcome::Failed(e) => Err(e),
                    Outcome::Cancelled => Err("validation cancelled".into()),
                }
            }
            other => Err(format!("unknown validation kind: {other}")),
        }
    };
    install(m, job.params["files"].as_object(), models_root, job_object, db, &mut validate, progress, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn shipped_manifests_parse_and_are_not_downloadable_without_verification() {
        let all = manifests();
        assert!(all.iter().any(|m| m.id == "openclip-vit-b32") && all.iter().any(|m| m.id == "whisper-base"));
        for m in &all {
            if m.artifacts.iter().any(|a| a.url.is_none() || a.sha256.is_none()) {
                assert!(m.download_blocker().is_some(), "{} must stay blocked", m.id);
            }
        }
        let c = open_in_memory();
        assert!(list(&c).unwrap().iter().all(|v| v.state == "not_installed"));
    }

    #[test]
    fn interrupted_installs_become_failed_and_ready_ones_stay() {
        let c = open_in_memory();
        for (id, state) in [("whisper-base", "downloading"), ("whisper-small", "validating"), ("silero-vad", "ready")] {
            c.execute("INSERT INTO models (id, version, state) VALUES (?1, '1', ?2)", [id, state]).unwrap();
        }
        assert_eq!(reset_interrupted(&c).unwrap(), 2);
        let state = |id: &str| -> (String, Option<String>) {
            c.query_row("SELECT state, error FROM models WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
        };
        assert_eq!(state("whisper-base").0, "install_failed");
        assert!(state("whisper-small").1.unwrap().contains("Interrupted"));
        assert_eq!(state("silero-vad"), ("ready".into(), None));
        assert_eq!(reset_interrupted(&c).unwrap(), 0, "idempotent");
    }

    #[test]
    fn sha256_known_vector() {
        let p = std::env::temp_dir().join(format!("memora-sha-{}.txt", std::process::id()));
        std::fs::write(&p, "abc").unwrap();
        let h = sha256_file(&p, &|| false, &|_| {}).unwrap().unwrap();
        assert_eq!(h, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        std::fs::remove_file(p).unwrap();
    }

    fn test_manifest(dir: &Path, content: &[u8], sha: &str) -> (Manifest, PathBuf) {
        let src = dir.join("source").join("weights.bin");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, content).unwrap();
        // file:// URL: curl.exe "downloads" from disk, exercising the real download path offline.
        let url = format!("file:///{}", src.to_string_lossy().replace('\\', "/"));
        let m = Manifest {
            id: "test-model".into(), name: "Test".into(), version: "1".into(), category: "test".into(),
            description: String::new(), required: false, license: "test".into(), minimum_ram_gb: 1,
            validation: "none".into(), notes: String::new(), runtimes: vec![],
            artifacts: vec![Artifact {
                file: "weights.bin".into(), url: Some(url), revision: None, sha256: Some(sha.into()),
                size_bytes: Some(content.len() as u64),
            }],
        };
        (m, src)
    }

    #[test]
    fn download_verify_install_remove_and_checksum_failure() {
        let dir = std::env::temp_dir().join(format!("memora-models-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let content = vec![7u8; 300_000];
        let hash: String = Sha256::digest(&content).iter().map(|b| format!("{b:02x}")).collect();
        let (m, src) = test_manifest(&dir, &content, &hash);
        let root = dir.join("models");
        let db = Mutex::new(Ok(open_in_memory()));
        let jo = JobObject::new().unwrap();
        let mut ok = |_: &Manifest, _: &Path| Ok(json!({"validation": "test"}));
        let last = Mutex::new((0, 0));

        let r = install(&m, None, &root, &jo, &db, &mut ok, &|d, t| *last.lock().unwrap() = (d, t), &|| false);
        assert!(matches!(r, Outcome::Completed(_)), "download install failed");
        let installed = install_dir(&root, &m).join("weights.bin");
        assert_eq!(std::fs::read(&installed).unwrap(), content);
        let (d, t) = *last.lock().unwrap();
        assert_eq!(d, t);
        let state: String = db.lock().unwrap().as_ref().unwrap().query_row("SELECT state FROM models", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "ready");
        assert!(!downloads_dir(&root, &m).exists(), "download folder cleaned up");

        // Offline install: user file is copied, not moved.
        let mut files = serde_json::Map::new();
        files.insert("weights.bin".into(), json!(src));
        assert!(matches!(install(&m, Some(&files), &root, &jo, &db, &mut ok, &|_, _| {}, &|| false), Outcome::Completed(_)));
        assert!(src.exists(), "user's file untouched");

        // Wrong checksum → checksum_failed, nothing installed for the bad copy.
        let (bad, _) = test_manifest(&dir, &content, &"0".repeat(64));
        let mut bad = bad;
        bad.id = "bad-model".into();
        let r = install(&bad, None, &root, &jo, &db, &mut ok, &|_, _| {}, &|| false);
        assert!(matches!(r, Outcome::Failed(ref e) if e.contains("does not match")));
        let state: String =
            db.lock().unwrap().as_ref().unwrap().query_row("SELECT state FROM models WHERE id='bad-model'", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "checksum_failed");
        assert!(!install_dir(&root, &bad).exists());

        // Validation failure is reported as model_load_failed.
        let mut broken = |_: &Manifest, _: &Path| Err("cannot load".to_string());
        assert!(matches!(install(&m, Some(&files), &root, &jo, &db, &mut broken, &|_, _| {}, &|| false), Outcome::Failed(_)));
        let state: String = db.lock().unwrap().as_ref().unwrap().query_row("SELECT state FROM models WHERE id='test-model'", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "model_load_failed");

        // Remove needs a shipped manifest id; remove the test dirs directly instead.
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Offline install of the shipped OpenCLIP manifest with a tiny stand-in IR (built with OpenVINO
    /// in-test, dynamic image input), validated by the real AI worker. Skipped without the dev venv.
    #[test]
    fn offline_install_validates_openvino_ir_with_real_worker() {
        let Ok(paths) = crate::worker::resolve(None) else {
            eprintln!("skipped: worker venv not found");
            return;
        };
        let dir = std::env::temp_dir().join(format!("memora-ir-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = "import sys, numpy as np, openvino as ov, openvino.opset13 as ops\n\
                      x = ops.parameter([-1, 3, -1, -1], np.float32)\n\
                      y = ops.reduce_mean(x, ops.constant(np.array([2, 3], dtype=np.int64)), False)\n\
                      ov.save_model(ov.Model([y], [x]), sys.argv[1] + '/openclip_image.xml')\n";
        let ok = std::process::Command::new(&paths.python).arg("-c").arg(script).arg(&dir).status().unwrap().success();
        assert!(ok, "could not build test IR");

        let db = Mutex::new(Ok(open_in_memory()));
        let jo = std::sync::Arc::new(JobObject::new().unwrap());
        let mut ai = crate::worker::runner(|| None, jo.clone(), std::sync::Arc::new(Mutex::new(None)));
        let job = Job {
            id: 1, kind: "models.install".into(), status: Status::Running, done: 0, total: 0, result: None, error: None,
            params: json!({"id": "openclip-vit-b32", "files": {
                "openclip_image.xml": dir.join("openclip_image.xml"), "openclip_image.bin": dir.join("openclip_image.bin")}}),
        };
        let root = dir.join("models");
        // Synthetic IR: same manifest without the real export's checksums and tag file.
        let mut m = manifest("openclip-vit-b32").unwrap();
        m.artifacts.retain(|a| a.file.starts_with("openclip_image"));
        m.artifacts.iter_mut().for_each(|a| (a.sha256, a.size_bytes) = (None, None));
        let r = install_manifest_job(&m, &job, &|_, _| {}, &|| false, &root, &jo, &db, &mut ai);
        let Outcome::Completed(v) = r else { panic!("install failed: {:?}", list(db.lock().unwrap().as_ref().unwrap()).unwrap()[0].error) };
        assert_eq!(v["details"]["inputs"][0]["shape"], json!([1, 3, 224, 224]));
        assert_eq!(v["details"]["devices"][0]["ok"], true);
        let views = list(db.lock().unwrap().as_ref().unwrap()).unwrap();
        assert_eq!(views.iter().find(|m| m.manifest.id == "openclip-vit-b32").unwrap().state, "ready");
        remove(db.lock().unwrap().as_ref().unwrap(), &root, "openclip-vit-b32").unwrap();
        assert!(!install_dir(&root, &manifest("openclip-vit-b32").unwrap()).exists());
        drop(ai);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn blocked_download_is_refused() {
        let m = manifest("openclip-vit-b32").unwrap(); // exported locally: never downloadable
        let db = Mutex::new(Ok(open_in_memory()));
        let mut ok = |_: &Manifest, _: &Path| Ok(Value::Null);
        let r = install(&m, None, Path::new("Z:\\nowhere"), &JobObject::new().unwrap(), &db, &mut ok, &|_, _| {}, &|| false);
        assert!(matches!(r, Outcome::Failed(ref e) if e.contains("model proposal")));
    }
}
