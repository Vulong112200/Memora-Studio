mod analysis;
pub mod cli;
mod db;
mod editing;
mod export;
mod hardware;
mod jobs;
mod logging;
mod media;
mod models;
mod photos;
mod process;
mod projects;
mod proxy;
mod request;
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
    &["validate_devices", "media.import", "media.proxy", "media.export", "media.analyze", "media.transcribe", "media.tag", "media.photo_video", "media.photo_analyze", "models.install"];

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
    /// Worker process of the photo editor (previews do not wait behind queued jobs).
    photo_lane: Arc<photos::Lane>,
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

/// After a crash or forced close: interrupted model installs and temporary files. Runs only while no job
/// is queued or running (app start, data-folder switch), so everything it finds is stale.
fn reconcile(c: &Connection, root: &Path) {
    match models::reset_interrupted(c) {
        Ok(0) => {}
        Ok(n) => tracing::warn!(models = n, "interrupted model installs marked as failed"),
        Err(e) => tracing::error!(error = %e, "cannot reset interrupted model installs"),
    }
    let n = proxy::remove_leftovers(root);
    if n > 0 {
        tracing::info!(files = n, "temporary files of unfinished jobs removed");
    }
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
    // A running job keeps the old folder's paths but writes through the shared database handle:
    // switching under it would store rows in the wrong database.
    if state.jobs.busy() {
        return Err("Wait until the running jobs finish (or cancel them) before changing the data folder.".into());
    }
    let app_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
    let root = settings::prepare_data_root(&path, app_dir.as_deref())?;
    // Open first: a folder whose database cannot be opened is not saved, the current one stays in use.
    let db = open_db(Some(&root))?;
    reconcile(&db, &root);
    let new = settings::Settings { data_root: Some(root.clone()) };
    settings::save(&state.config_dir, &new)?;
    *state.settings.lock().unwrap() = new;
    *state.db.lock().unwrap() = Ok(db);
    allow_cache_assets(&app, &root);
    tracing::info!(data_root = %root.display(), "data root set");
    Ok(view(&state))
}

/// Verified copy of the database in `<data root>\database\backups\`; returns its path.
#[tauri::command]
fn backup_database(state: State<'_, AppState>) -> Result<PathBuf, String> {
    let root = state.settings.lock().unwrap().data_root.clone().ok_or(NO_DATA_ROOT)?;
    let path = with_db(&state, |c| db::backup(c, &db::backups_dir(&root.join("database").join("memora.db")), "manual"))?;
    tracing::info!(backup = %path.display(), "database backed up by the user");
    Ok(path)
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
/// `request` = the sentence from the request box; the style and dates it names win over the selection.
#[tauri::command]
fn generate_plan(
    project_id: i64,
    style: String,
    from: Option<String>,
    to: Option<String>,
    request: Option<String>,
    state: State<'_, AppState>,
) -> Result<editing::Plan, String> {
    with_db(&state, |c| {
        let plan = make_plan(c, project_id, &style, from.as_deref(), to.as_deref(), request.as_deref())?;
        tracing::info!(project_id, style = plan.style, clips = plan.clips.len(), total_ms = plan.total_ms, "editing plan generated");
        Ok(plan)
    })
}

/// What the request box understood (read back to the user before a plan is made).
#[tauri::command]
fn interpret_request(text: String, state: State<'_, AppState>) -> Result<request::Request, String> {
    with_db(&state, |c| Ok(request::parse(&text, &vocabulary(c)?)))
}

/// Concept names the library was tagged with (all taxonomy versions), for the request box.
fn vocabulary(c: &Connection) -> Result<Vec<tagging::TagName>, String> {
    let mut names: Vec<tagging::TagName> = vec![];
    for t in tagging::all(c)?.into_values() {
        // Scene cues (forest trail, small bridge, slope) can be asked for like tags; "view ahead" is framing only.
        for n in t.tags.into_iter().chain(t.cues.into_iter().filter(|c| editing::SCENE_CUES.iter().any(|s| s.0 == c.en))) {
            if !names.iter().any(|x| x.en == n.en) {
                names.push(n);
            }
        }
    }
    Ok(names)
}

/// Builds and stores a plan. A non-empty `request` sentence adds its wishes; its style and dates win.
fn make_plan(c: &Connection, project_id: i64, style: &str, from: Option<&str>, to: Option<&str>, request: Option<&str>) -> Result<editing::Plan, String> {
    let req = match request.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => Some(request::parse(r, &vocabulary(c)?)),
        None => None,
    };
    let st = editing::style(req.as_ref().and_then(|r| r.style.as_deref()).unwrap_or(style))?;
    let (from, to) = match &req {
        Some(r) if r.from.is_some() => (r.from.clone(), r.to.clone()),
        _ => (from.map(String::from), to.map(String::from)),
    };
    let wishes = req.map(|r| r.wishes).unwrap_or_default();
    let (from, to) = (from.as_deref(), to.as_deref());
    let plan = editing::plan_with_photos(&st, &editing::gather(c, from, to)?, &editing::gather_photos(c, from, to)?, &wishes);
    editing::save(c, project_id, &plan)?;
    Ok(plan)
}

/// Replaces the timeline with the kept plan clips and saves the style's look with the user's wishes,
/// including wished library music (the best-suited track of that energy for the new timeline).
fn apply_plan_to(c: &Connection, project_id: i64, keep: &[usize]) -> Result<export::Options, String> {
    editing::apply(c, project_id, keep)?;
    let plan = editing::load(c, project_id)?.ok_or("Generate a plan first.")?;
    let mut o = editing::styled_options(export::load_options(c, project_id), &editing::style(&plan.style)?, &plan);
    if let Some(energy) = plan.wishes.music.as_deref().filter(|m| *m != "none") {
        let tracks = sounds::catalog();
        let pick = recommend_for(c, project_id, tracks.len())?
            .into_iter()
            .find(|r| energy == "any" || tracks.iter().any(|t| t.id == r.id && t.energy == energy));
        if let Some(r) = pick {
            let volume = o.music.as_ref().map_or(0.35, |m| m.volume);
            o.music = Some(export::Music { path: String::new(), library: Some(r.id), volume, duck: true });
        }
    }
    export::save_options(c, project_id, &o)?;
    Ok(o)
}

#[tauri::command]
fn get_plan(project_id: i64, state: State<'_, AppState>) -> Result<Option<editing::Plan>, String> {
    with_db(&state, |c| editing::load(c, project_id))
}

/// Replaces the timeline with the kept plan clips and sets the style's look in the export options.
#[tauri::command]
fn apply_plan(project_id: i64, keep: Vec<usize>, state: State<'_, AppState>) -> Result<export::Options, String> {
    let options = with_db(&state, |c| apply_plan_to(c, project_id, &keep))?;
    tracing::info!(project_id, clips = keep.len(), "editing plan applied");
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

/// Appends library photos to the timeline (each shown `ms`, default 4 s), in the given order.
#[tauri::command]
fn add_photo_clips(project_id: i64, photo_ids: Vec<i64>, ms: Option<i64>, state: State<'_, AppState>) -> Result<Vec<timeline::Clip>, String> {
    with_db(&state, |c| photo_ids.iter().map(|&id| timeline::add_photo(c, project_id, id, ms.unwrap_or(timeline::PHOTO_DEFAULT_MS))).collect())
}

/// New In / Out of a clip (for a photo: 0 and its length).
#[tauri::command]
fn set_clip_range(id: i64, in_ms: i64, out_ms: i64, state: State<'_, AppState>) -> Result<timeline::Clip, String> {
    with_db(&state, |c| timeline::set_range(c, id, in_ms, out_ms))
}

/// Transition into one clip (`transition` null = back to the project's transition).
#[tauri::command]
fn set_clip_transition(id: i64, transition: Option<String>, transition_ms: i64, state: State<'_, AppState>) -> Result<timeline::Clip, String> {
    with_db(&state, |c| timeline::set_transition(c, id, transition.as_deref(), transition_ms))
}

/// Every join of the timeline as it will be exported: `[xfade name, ms]`.
#[tauri::command]
fn list_joins(project_id: i64, state: State<'_, AppState>) -> Result<Vec<export::Join>, String> {
    with_db(&state, |c| export::timeline_joins(c, project_id))
}

/// Short low-resolution render around one join, for checking a transition (file in the cache, shown by the webview).
#[tauri::command]
async fn preview_join(project_id: i64, index: usize, preset: String, state: State<'_, AppState>) -> Result<PathBuf, String> {
    let (db, lane, root) = photo_ctx(&state)?;
    let preset = export::Preset::parse(&preset)?;
    blocking(move || {
        let job_object = process::JobObject::new()?;
        let env = media::MediaEnv {
            ffprobe: media::tool_path(Some(&root), "ffprobe.exe"),
            ffmpeg: media::tool_path(Some(&root), "ffmpeg.exe"),
            cache: root.join("cache"),
            job_object: &job_object,
            db: &db,
        };
        let still = |p: &photos::Photo, out: &std::path::Path| {
            let r = lane.call("photo_render", serde_json::json!({"path": p.path, "edits": p.edits, "maxSide": 1280, "out": out, "quality": 90}))?;
            Ok((r["outWidth"].as_i64().unwrap_or(0), r["outHeight"].as_i64().unwrap_or(0)))
        };
        export::preview_join(&env, project_id, index, preset, &still)
    })
    .await
}

#[tauri::command]
fn set_clip_caption(id: i64, caption: String, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| timeline::set_caption(c, id, &caption))
}

/// Saved export options of a project (look, music, titles); defaults when never saved.
#[tauri::command]
fn get_export_options(project_id: i64, state: State<'_, AppState>) -> Result<export::Options, String> {
    with_db(&state, |c| Ok(export::load_options(c, project_id)))
}

#[tauri::command]
fn set_export_options(project_id: i64, options: export::Options, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| export::save_options(c, project_id, &options))
}

#[tauri::command]
fn list_sounds() -> Vec<sounds::Track> {
    sounds::catalog()
}

/// Library tracks that suit the project's video: its plan style, what is on screen in the timeline clips,
/// the timeline length and how much of it is speech. Without a project (photo video): gentle moods and `video_ms`.
#[tauri::command]
fn recommend_sounds(project_id: Option<i64>, video_ms: Option<i64>, state: State<'_, AppState>) -> Result<Vec<sounds::Recommendation>, String> {
    match project_id {
        Some(id) => with_db(&state, |c| recommend_for(c, id, 3)),
        // Photo video: no tags or speech; gentle "memories" moods and the video length.
        None => Ok(sounds::recommend("memory_nostalgia", &[], video_ms.unwrap_or(0), 0.0, 3)),
    }
}

fn recommend_for(c: &Connection, project_id: i64, n: usize) -> Result<Vec<sounds::Recommendation>, String> {
    let style = editing::load(c, project_id)?.map(|p| p.style).unwrap_or_else(|| "clean_minimal".into());
    let clips = timeline::list(c, project_id)?;
    let mut tags: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let (mut total, mut speech) = (0i64, 0i64);
    for clip in &clips {
        let len = clip.out_ms - clip.in_ms;
        total += len;
        let Some(media_id) = clip.media_id else { continue }; // photos: no tags or speech yet
        if let Some(t) = tagging::get(c, media_id)? {
            for s in tagging::summarize(&t, clip.in_ms, clip.out_ms, 0.1).into_iter().filter(|s| !s.negative) {
                *tags.entry(s.en).or_default() += s.score * len as f64;
            }
        }
        if let Some(t) = transcribe::get(c, media_id)? {
            speech += t.segments.iter().map(|s| (s.end_ms.min(clip.out_ms) - s.start_ms.max(clip.in_ms)).max(0)).sum::<i64>();
        }
    }
    let mut tags: Vec<(String, f64)> = tags.into_iter().map(|(k, v)| (k, v / total.max(1) as f64)).collect();
    tags.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(sounds::recommend(&style, &tags, total, speech as f64 / total.max(1) as f64, n))
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

/// Library photos with thumbnails (missing ones re-created) and a `missing` flag for moved files.
#[tauri::command]
async fn list_photos(state: State<'_, AppState>) -> Result<Vec<photos::Photo>, String> {
    let (db, lane, root) = photo_ctx(&state)?;
    blocking(move || {
        let mut items = { let g = db.lock().unwrap(); photos::list(g.as_ref().map_err(Clone::clone)?)? };
        let cache = root.join("cache");
        photos::attach_thumbs(&cache, &mut items);
        photos::repair_thumbs(&lane, &cache, &mut items);
        Ok(items)
    })
    .await
}

/// Photos moved to `folder` (same name and size) are found again; returns how many.
#[tauri::command]
fn relink_photos(folder: String, state: State<'_, AppState>) -> Result<usize, String> {
    with_db(&state, |c| photos::relink(c, Path::new(&folder)))
}

/// What a photo command running off the UI thread needs: database, photo worker, data root.
fn photo_ctx(state: &AppState) -> Result<(Db, Arc<photos::Lane>, PathBuf), String> {
    let root = state.settings.lock().unwrap().data_root.clone().ok_or(NO_DATA_ROOT)?;
    Ok((state.db.clone(), state.photo_lane.clone(), root))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn photo_of(db: &Db, id: i64) -> Result<photos::Photo, String> {
    let guard = db.lock().unwrap();
    photos::get(guard.as_ref().map_err(Clone::clone)?, id)
}

/// Adds photos (read in place, never modified): row, size, date, thumbnail.
#[tauri::command]
async fn import_photos(paths: Vec<String>, auto: Option<bool>, state: State<'_, AppState>) -> Result<photos::ImportResult, String> {
    let (db, lane, root) = photo_ctx(&state)?;
    let r = blocking(move || {
        let r = photos::import(&db, &lane, &root.join("cache"), &paths)?;
        if auto.unwrap_or(false) {
            photos::auto_unedited(&db, &lane, &r.ids)?;
        }
        Ok(r)
    })
    .await?;
    tracing::info!(imported = r.ids.len(), failed = r.failed.len(), "photos imported");
    Ok(r)
}

/// Suggested edits for the given photos that have none yet (batch "Auto"); returns how many were edited.
#[tauri::command]
async fn auto_photos(ids: Vec<i64>, state: State<'_, AppState>) -> Result<usize, String> {
    let (db, lane, _) = photo_ctx(&state)?;
    blocking(move || photos::auto_unedited(&db, &lane, &ids)).await
}

/// Which photo AI tools are installed: `face` (retouch), `mesh` (precise skin mask), `select` + `inpaint` (click to
/// remove), `inpaint` (AI fill of painted areas), `bokeh` (background blur; `matte` sharpens people).
#[tauri::command]
fn photo_tools(state: State<'_, AppState>) -> Value {
    let root = state.settings.lock().unwrap().data_root.clone();
    let m = root.as_deref().map(photos::photo_models).unwrap_or_default();
    let has = |k: &str| m.get(k).is_some();
    serde_json::json!({"face": has("face"), "mesh": has("landmarks"), "select": has("sam") && has("inpaint"), "inpaint": has("inpaint"),
        "bokeh": has("depth"), "matte": has("matte")})
}

#[tauri::command]
fn set_photo_edits(id: i64, edits: Value, state: State<'_, AppState>) -> Result<(), String> {
    with_db(&state, |c| photos::set_edits(c, id, &edits))
}

/// Live preview (≤ 1600 px) of `edits`, or the untouched photo with `before`; returns the cache file to show.
#[tauri::command]
async fn preview_photo(id: i64, edits: Value, show_mask: bool, before: bool, state: State<'_, AppState>) -> Result<PathBuf, String> {
    let (db, lane, root) = photo_ctx(&state)?;
    blocking(move || photos::preview(&lane, &root.join("cache"), &photo_of(&db, id)?, &edits, show_mask, before)).await
}

/// Suggested starting values for a photo (the UI puts them on the sliders; nothing is saved here).
#[tauri::command]
async fn auto_photo(id: i64, state: State<'_, AppState>) -> Result<Value, String> {
    let (db, lane, _) = photo_ctx(&state)?;
    blocking(move || {
        let p = photo_of(&db, id)?;
        lane.call("photo_auto", serde_json::json!({"path": p.path})).map(|r| r["edits"].clone())
    })
    .await
}

/// Full-size edited copy in `<data root>\photos\exports\`; the original is never written.
#[tauri::command]
async fn export_photo(id: i64, state: State<'_, AppState>) -> Result<PathBuf, String> {
    let (db, lane, root) = photo_ctx(&state)?;
    let out = blocking(move || photos::export(&lane, &root.join("photos").join("exports"), &photo_of(&db, id)?)).await?;
    tracing::info!(photo_id = id, path = %out.display(), "photo exported");
    Ok(out)
}

#[tauri::command]
fn remove_photo(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let cache = cache_dir(&state).ok_or(NO_DATA_ROOT)?;
    with_db(&state, |c| photos::remove(c, &cache, id))
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

/// Executes every job kind: media and model jobs in Rust (child processes), everything else in the AI worker.
/// Shared by the app's job queue and the command-line tool (cli.rs).
fn job_runner(settings: Arc<Mutex<settings::Settings>>, db: Db, worker_info: Arc<Mutex<Option<Value>>>) -> Result<jobs::Runner, String> {
    let job_object = Arc::new(process::JobObject::new()?);
    let s = settings.clone();
    let mut ai = worker::runner(move || s.lock().unwrap().data_root.clone(), job_object.clone(), worker_info);
    Ok(Box::new(move |job, progress, cancelled| {
        if !job.kind.starts_with("media.") && !job.kind.starts_with("models.") {
            return ai(job, progress, cancelled);
        }
        let Some(root) = settings.lock().unwrap().data_root.clone() else {
            return jobs::Outcome::Failed(NO_DATA_ROOT.into());
        };
        if job.kind == "models.install" {
            return models::install_job(job, progress, cancelled, &root.join("models"), &job_object, &db, &mut ai);
        }
        let env = media::MediaEnv {
            ffprobe: media::tool_path(Some(&root), "ffprobe.exe"),
            ffmpeg: media::tool_path(Some(&root), "ffmpeg.exe"),
            cache: root.join("cache"),
            job_object: &job_object,
            db: &db,
        };
        match job.kind.as_str() {
            "media.import" => media::import_job(job, progress, cancelled, &env),
            "media.proxy" => proxy::proxy_job(job, progress, cancelled, &env),
            "media.export" => export::export_job(job, progress, cancelled, &env, &root.join("projects"), &mut ai),
            "media.analyze" => analysis::analyze_job(job, progress, cancelled, &env, &mut ai),
            "media.tag" => tagging::tag_job(job, progress, cancelled, &env, &mut ai),
            "media.transcribe" => transcribe::transcribe_job(job, progress, cancelled, &env, &root),
            "media.photo_video" => photos::video_job(job, progress, cancelled, &env, &root, &mut ai),
            "media.photo_analyze" => photos::analyze_job(job, progress, cancelled, &env, &mut ai),
            k => jobs::Outcome::Failed(format!("Unknown job kind: {k}")),
        }
    }))
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
            let db = open_db(settings.data_root.as_deref());
            if let Some(root) = &settings.data_root {
                allow_cache_assets(app.handle(), root);
                if let Ok(c) = &db {
                    reconcile(c, root);
                }
            }
            let db: Db = Arc::new(Mutex::new(db));

            let settings = Arc::new(Mutex::new(settings));
            let worker_info = Arc::new(Mutex::new(None));
            let runner = job_runner(settings.clone(), db.clone(), worker_info.clone())?;
            let s = settings.clone();
            let photo_lane = Arc::new(photos::Lane::new(move || s.lock().unwrap().data_root.clone())?);
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
                photo_lane,
                log_dir,
                _log_guard: guard,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_data_root,
            backup_database,
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
            interpret_request,
            get_plan,
            apply_plan,
            list_models,
            remove_model,
            list_clips,
            add_clip,
            add_photo_clips,
            auto_photos,
            relink_photos,
            photo_tools,
            set_clip_range,
            set_clip_transition,
            list_joins,
            preview_join,
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
            get_worker_info,
            list_photos,
            import_photos,
            set_photo_edits,
            preview_photo,
            auto_photo,
            export_photo,
            remove_photo
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
