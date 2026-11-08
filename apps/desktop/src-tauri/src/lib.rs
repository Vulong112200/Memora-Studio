mod analysis;
mod db;
mod editing;
mod export;
mod hardware;
mod jobs;
mod logging;
mod media;
mod models;
mod process;
mod projects;
mod proxy;
mod scoring;
mod settings;
mod sounds;
mod tagging;
mod timeline;
mod transcribe;
mod worker;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};

const NO_DATA_ROOT: &str = "Choose a data folder first.";
/// Job kinds the UI may start. `diagnostic.*` kinds exist for tests only.
const UI_JOB_KINDS: &[&str] =
    &["validate_devices", "media.import", "media.proxy", "media.export", "media.analyze", "media.transcribe", "media.tag", "models.install"];

/// `<data root>\cache`: thumbnails and proxies. The only folder the webview may load files from.
fn cache_dir(state: &AppState) -> Option<PathBuf> {
    state.settings.lock().unwrap().data_root.as_ref().map(|r| r.join("cache"))
}

fn allow_cache_assets(app: &tauri::AppHandle, root: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(root.join("cache"), true) {
        tracing::error!(error = %e, "cannot allow cache folder for previews");
    }
}

type Db = Arc<Mutex<Result<Connection, String>>>;

struct AppState {
    config_dir: PathBuf,
    settings: Arc<Mutex<settings::Settings>>,
    jobs: jobs::JobManager,
    /// `hello` of the running AI worker, None when it is not running.
    worker_info: Arc<Mutex<Option<Value>>>,
    /// Open database, or the reason it is unavailable (no data root yet, open/migration failure).
    db: Db,
    /// Log folder in use for this session (file logging starts at launch only).
    log_dir: Option<PathBuf>,
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    data_root: Option<PathBuf>,
    default_data_root: PathBuf,
    log_dir: Option<PathBuf>,
    /// True when the saved data root differs from the one this session logs to.
    restart_required: bool,
    database_error: Option<String>,
}

fn view(state: &AppState) -> SettingsView {
    let data_root = state.settings.lock().unwrap().data_root.clone();
    let restart_required = data_root.as_ref().map(|r| r.join("logs")) != state.log_dir;
    let database_error = state.db.lock().unwrap().as_ref().err().filter(|e| *e != NO_DATA_ROOT).cloned();
    SettingsView {
        data_root,
        default_data_root: settings::default_data_root(),
        log_dir: state.log_dir.clone(),
        restart_required,
        database_error,
    }
}

fn open_db(data_root: Option<&Path>) -> Result<Connection, String> {
    let root = data_root.ok_or(NO_DATA_ROOT)?;
    let result = db::open(&root.join("database").join("memora.db"));
    match &result {
        Ok(_) => tracing::info!(data_root = %root.display(), "database opened"),
        Err(e) => tracing::error!(error = %e, "database unavailable"),
    }
    result
}

fn with_db<T>(state: &AppState, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let guard = state.db.lock().unwrap();
    f(guard.as_ref().map_err(Clone::clone)?)
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> SettingsView {
    view(&state)
}

#[tauri::command]
fn set_data_root(path: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<SettingsView, String> {
    let app_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
    let root = settings::prepare_data_root(&path, app_dir.as_deref())?;
    let new = settings::Settings { data_root: Some(root.clone()) };
    settings::save(&state.config_dir, &new)?;
    *state.settings.lock().unwrap() = new;
    *state.db.lock().unwrap() = open_db(Some(&root));
    allow_cache_assets(&app, &root);
    tracing::info!(data_root = %root.display(), "data root set");
    Ok(view(&state))
}

#[tauri::command]
async fn get_hardware_info(state: State<'_, AppState>) -> Result<hardware::HardwareInfo, String> {
    let root = state.settings.lock().unwrap().data_root.clone();
    tauri::async_runtime::spawn_blocking(move || hardware::detect(root.as_deref()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_projects(state: State<'_, AppState>) -> Result<Vec<projects::Project>, String> {
    with_db(&state, projects::list)
}

#[tauri::command]
fn create_project(name: String, state: State<'_, AppState>) -> Result<projects::Project, String> {
    let p = with_db(&state, |c| projects::create(c, &name))?;
    tracing::info!(project_id = p.id, "project created");
    Ok(p)
}

#[tauri::command]
fn rename_project(id: i64, name: String, state: State<'_, AppState>) -> Result<projects::Project, String> {
    with_db(&state, |c| projects::rename(c, id, &name))
}

#[tauri::command]
fn delete_project(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| projects::delete(c, id))?;
    tracing::info!(project_id = id, "project deleted");
    Ok(())
}

#[tauri::command]
fn list_media(state: State<'_, AppState>) -> Result<Vec<media::Media>, String> {
    let (mut items, counts, transcribed, tags, marks) = with_db(&state, |c| {
        Ok((media::list(c)?, analysis::scene_counts(c)?, transcribe::transcribed(c)?, tagging::all(c)?, editing::marks(c)?))
    })?;
    if let Some(cache) = cache_dir(&state) {
        proxy::attach_cache(&cache, &mut items);
    }
    for m in &mut items {
        m.scene_count = counts.get(&m.id).copied();
        m.transcribed = transcribed.contains(&m.id);
        m.tags = tags.get(&m.id).map(|t| tagging::summarize(t, 0, i64::MAX, 0.05));
        m.mark = marks.get(&m.id).copied().unwrap_or_default();
    }
    Ok(items)
}

#[tauri::command]
fn list_scenes(media_id: i64, state: State<'_, AppState>) -> Result<Vec<scoring::ScoredScene>, String> {
    with_db(&state, |c| {
        let mut scenes = scoring::score(analysis::scenes(c, media_id)?, &analysis::library_hashes(c)?);
        if let Some(t) = tagging::get(c, media_id)? {
            for s in &mut scenes {
                s.tags = tagging::summarize(&t, s.scene.start_ms, s.scene.end_ms, 0.1);
            }
        }
        Ok(scenes)
    })
}

#[tauri::command]
fn get_transcript(media_id: i64, state: State<'_, AppState>) -> Result<Option<transcribe::Transcript>, String> {
    with_db(&state, |c| transcribe::get(c, media_id))
}

#[tauri::command]
fn set_media_mark(media_id: i64, mark: editing::Mark, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| editing::set_mark(c, media_id, mark))
}

#[tauri::command]
fn list_styles() -> Vec<editing::Style> {
    editing::styles()
}

/// Smart mode: builds and stores an Editing Plan for review. `from`/`to` = recording dates YYYY-MM-DD.
#[tauri::command]
fn generate_plan(project_id: i64, style: String, from: Option<String>, to: Option<String>, state: State<'_, AppState>) -> Result<editing::Plan, String> {
    let st = editing::style(&style)?;
    with_db(&state, |c| {
        let plan = editing::plan(&st, &editing::gather(c, from.as_deref(), to.as_deref())?);
        editing::save(c, project_id, &plan)?;
        tracing::info!(project_id, style, clips = plan.clips.len(), total_ms = plan.total_ms, "editing plan generated");
        Ok(plan)
    })
}

#[tauri::command]
fn get_plan(project_id: i64, state: State<'_, AppState>) -> Result<Option<editing::Plan>, String> {
    with_db(&state, |c| editing::load(c, project_id))
}

/// Replaces the timeline with the kept plan clips and sets the style's look in the export options.
#[tauri::command]
fn apply_plan(project_id: i64, keep: Vec<usize>, state: State<'_, AppState>) -> Result<export::Options, String> {
    let n = with_db(&state, |c| editing::apply(c, project_id, &keep))?;
    let plan = with_db(&state, |c| editing::load(c, project_id))?.ok_or("Generate a plan first.")?;
    let options = editing::styled_options(get_export_options(project_id, state.clone())?, &editing::style(&plan.style)?, &plan);
    set_export_options(project_id, options.clone(), state)?;
    tracing::info!(project_id, clips = n, "editing plan applied");
    Ok(options)
}

#[tauri::command]
fn remove_media(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| media::remove(c, id))?;
    if let Some(cache) = cache_dir(&state) {
        proxy::remove_cache(&cache, id);
    }
    tracing::info!(media_id = id, "media removed from library");
    Ok(())
}

#[tauri::command]
fn list_models(state: State<'_, AppState>) -> Result<Vec<models::ModelView>, String> {
    with_db(&state, models::list)
}

#[tauri::command]
fn remove_model(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = state.settings.lock().unwrap().data_root.clone().ok_or(NO_DATA_ROOT)?;
    with_db(&state, |c| models::remove(c, &root.join("models"), &id))
}

#[tauri::command]
fn list_clips(project_id: i64, state: State<'_, AppState>) -> Result<Vec<timeline::Clip>, String> {
    with_db(&state, |c| timeline::list(c, project_id))
}

#[tauri::command]
fn add_clip(project_id: i64, media_id: i64, in_ms: i64, out_ms: i64, state: State<'_, AppState>) -> Result<timeline::Clip, String> {
    with_db(&state, |c| timeline::add(c, project_id, media_id, in_ms, out_ms))
}

#[tauri::command]
fn set_clip_caption(id: i64, caption: String, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| timeline::set_caption(c, id, &caption))
}

/// Saved export options of a project (look, music, titles); defaults when never saved.
#[tauri::command]
fn get_export_options(project_id: i64, state: State<'_, AppState>) -> Result<export::Options, String> {
    with_db(&state, |c| {
        let j: Option<String> = c
            .query_row("SELECT export_json FROM project_settings WHERE project_id = ?1", [project_id], |r| r.get(0))
            .ok();
        Ok(j.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    })
}

#[tauri::command]
fn set_export_options(project_id: i64, options: export::Options, state: State<'_, AppState>) -> Result<(), String> {
    options.check()?;
    with_db(&state, |c| {
        c.execute(
            "INSERT INTO project_settings (project_id, export_json) VALUES (?1, ?2)
             ON CONFLICT (project_id) DO UPDATE SET export_json = excluded.export_json",
            rusqlite::params![project_id, serde_json::to_string(&options).unwrap()],
        )
        .map_err(db::err)
        .map(drop)
    })
}

#[tauri::command]
fn list_sounds() -> Vec<sounds::Track> {
    sounds::catalog()
}

/// Library tracks that suit the project's video: its plan style, what is on screen in the timeline clips,
/// the timeline length and how much of it is speech.
#[tauri::command]
fn recommend_sounds(project_id: i64, state: State<'_, AppState>) -> Result<Vec<sounds::Recommendation>, String> {
    with_db(&state, |c| {
        let style = editing::load(c, project_id)?.map(|p| p.style).unwrap_or_else(|| "clean_minimal".into());
        let clips = timeline::list(c, project_id)?;
        let mut tags: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let (mut total, mut speech) = (0i64, 0i64);
        for clip in &clips {
            let len = clip.out_ms - clip.in_ms;
            total += len;
            if let Some(t) = tagging::get(c, clip.media_id)? {
                for s in tagging::summarize(&t, clip.in_ms, clip.out_ms, 0.1).into_iter().filter(|s| !s.negative) {
                    *tags.entry(s.en).or_default() += s.score * len as f64;
                }
            }
            if let Some(t) = transcribe::get(c, clip.media_id)? {
                speech += t.segments.iter().map(|s| (s.end_ms.min(clip.out_ms) - s.start_ms.max(clip.in_ms)).max(0)).sum::<i64>();
            }
        }
        let mut tags: Vec<(String, f64)> = tags.into_iter().map(|(k, v)| (k, v / total.max(1) as f64)).collect();
        tags.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(sounds::recommend(&style, &tags, total, speech as f64 / total.max(1) as f64, 3))
    })
}

#[tauri::command]
fn remove_clip(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| timeline::remove(c, id))
}

#[tauri::command]
fn move_clip(id: i64, delta: i64, state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.db.lock().unwrap();
    timeline::move_clip(guard.as_mut().map_err(|e| e.clone())?, id, delta)
}

#[tauri::command]
fn submit_job(kind: String, params: Option<Value>, state: State<'_, AppState>) -> Result<jobs::Job, String> {
    if !UI_JOB_KINDS.contains(&kind.as_str()) {
        return Err(format!("Unknown job kind: {kind}"));
    }
    let job = state.jobs.submit(&kind, params.unwrap_or(Value::Null));
    tracing::info!(job_id = job.id, kind, "job submitted");
    Ok(job)
}

#[tauri::command]
fn list_jobs(state: State<'_, AppState>) -> Vec<jobs::Job> {
    state.jobs.list()
}

#[tauri::command]
fn cancel_job(id: u64, state: State<'_, AppState>) -> Result<(), String> {
    state.jobs.cancel(id)
}

#[tauri::command]
fn get_worker_info(state: State<'_, AppState>) -> Option<Value> {
    state.worker_info.lock().unwrap().clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let settings = settings::load(&config_dir);
            let (log_dir, guard) = logging::init(settings.data_root.as_ref().map(|r| r.join("logs")));
            tracing::info!(version = env!("CARGO_PKG_VERSION"), data_root = ?settings.data_root, "Memora Studio started");
            let db: Db = Arc::new(Mutex::new(open_db(settings.data_root.as_deref())));
            if let Some(root) = &settings.data_root {
                allow_cache_assets(app.handle(), root);
            }

            let settings = Arc::new(Mutex::new(settings));
            let worker_info = Arc::new(Mutex::new(None));
            let job_object = Arc::new(process::JobObject::new()?);
            let s = settings.clone();
            let mut ai = worker::runner(
                move || s.lock().unwrap().data_root.clone(),
                job_object.clone(),
                worker_info.clone(),
            );
            let (s, media_db) = (settings.clone(), db.clone());
            // Media and model jobs run in Rust (child processes); everything else goes to the AI worker.
            let runner: jobs::Runner = Box::new(move |job, progress, cancelled| {
                if !job.kind.starts_with("media.") && !job.kind.starts_with("models.") {
                    return ai(job, progress, cancelled);
                }
                let Some(root) = s.lock().unwrap().data_root.clone() else {
                    return jobs::Outcome::Failed(NO_DATA_ROOT.into());
                };
                if job.kind == "models.install" {
                    return models::install_job(job, progress, cancelled, &root.join("models"), &job_object, &media_db, &mut ai);
                }
                let env = media::MediaEnv {
                    ffprobe: media::tool_path(Some(&root), "ffprobe.exe"),
                    ffmpeg: media::tool_path(Some(&root), "ffmpeg.exe"),
                    cache: root.join("cache"),
                    job_object: &job_object,
                    db: &media_db,
                };
                match job.kind.as_str() {
                    "media.import" => media::import_job(job, progress, cancelled, &env),
                    "media.proxy" => proxy::proxy_job(job, progress, cancelled, &env),
                    "media.export" => export::export_job(job, progress, cancelled, &env, &root.join("projects")),
                    "media.analyze" => analysis::analyze_job(job, progress, cancelled, &env, &mut ai),
                    "media.tag" => tagging::tag_job(job, progress, cancelled, &env, &mut ai),
                    "media.transcribe" => transcribe::transcribe_job(job, progress, cancelled, &env, &root),
                    k => jobs::Outcome::Failed(format!("Unknown job kind: {k}")),
                }
            });
            let handle = app.handle().clone();
            let jobs = jobs::JobManager::start(
                runner,
                Box::new(move |job| {
                    let _ = handle.emit("job-updated", job);
                }),
            );

            app.manage(AppState {
                config_dir,
                settings,
                jobs,
                worker_info,
                db,
                log_dir,
                _log_guard: guard,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_data_root,
            get_hardware_info,
            list_projects,
            create_project,
            rename_project,
            delete_project,
            list_media,
            remove_media,
            list_scenes,
            get_transcript,
            set_media_mark,
            list_styles,
            generate_plan,
            get_plan,
            apply_plan,
            list_models,
            remove_model,
            list_clips,
            add_clip,
            remove_clip,
            list_sounds,
            recommend_sounds,
            set_clip_caption,
            get_export_options,
            set_export_options,
            move_clip,
            submit_job,
            list_jobs,
            cancel_job,
            get_worker_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
