# Backend (Rust, `apps/desktop/src-tauri`)

The Tauri 2 Rust backend: it orchestrates the database, the job queue and every child process (FFmpeg, FFprobe, whisper.cpp, curl, Python AI worker) for the React UI and the `memora-cli` tool.

Last verified: 2026-11-12 against commit `7eeb0bd` + uncommitted changes. Index: [README.md](README.md). Kept current by the `sync-docs` skill (`.claude/skills/sync-docs`).

Sources: [src/](../../apps/desktop/src-tauri/src/), [Cargo.toml](../../apps/desktop/src-tauri/Cargo.toml), [tauri.conf.json](../../apps/desktop/src-tauri/tauri.conf.json), [capabilities/default.json](../../apps/desktop/src-tauri/capabilities/default.json), [models.json](../../apps/desktop/src-tauri/models.json), [sounds.json](../../apps/desktop/src-tauri/sounds.json).

---

## 1. Crate layout

| Item | Value |
|---|---|
| Package | `memora-desktop` 0.1.0, edition 2024, `publish = false` |
| Library | `memora_desktop_lib` (`_lib` suffix avoids the Windows lib/bin name clash, cargo#8519), crate types `staticlib`, `cdylib`, `rlib` |
| Bin `memora-desktop` | [main.rs](../../apps/desktop/src-tauri/src/main.rs) → `memora_desktop_lib::run()`; `windows_subsystem = "windows"` in release (no console) |
| Bin `memora-cli` | [bin/memora-cli.rs](../../apps/desktop/src-tauri/src/bin/memora-cli.rs) → `memora_desktop_lib::cli::run(args)`; console program |
| `default-run` | `memora-desktop` (so `cargo run` / `tauri dev` start the app) |
| Build script | [build.rs](../../apps/desktop/src-tauri/build.rs) → `tauri_build::build()` |
| Release profile | `codegen-units = 1`, `lto = true`, `opt-level = 3`, `panic = "abort"`, `strip = true` |
| Public API of the lib | `pub mod cli`, `pub fn run()`; every other module is private (`mod x;` in lib.rs) |
| Test | `cargo test` in `apps/desktop/src-tauri` (85 `#[test]` functions, 3 `#[ignore]`) |

### Dependencies

| Crate | Version / features | Why |
|---|---|---|
| `tauri` | 2, `protocol-asset` | App shell, commands, `job-updated` events, asset protocol for cache previews |
| `tauri-build` | 2 (build-dep) | Code generation in build.rs (capabilities schema, context) |
| `tauri-plugin-dialog` | 2 | Native open dialog (`dialog:allow-open`) for media/model files and the data folder |
| `serde`, `serde_json` | 1, `derive` | IPC types, JSON columns, FFprobe / worker / whisper.cpp JSON, manifests |
| `tracing` | 0.1 | Structured log events |
| `tracing-subscriber` | 0.3, `json`, `env-filter` | stderr + JSON file layers, `MEMORA_LOG` filter |
| `tracing-appender` | 0.2 | Daily rolling log file, non-blocking writer |
| `rusqlite` | 0.37, `bundled` | SQLite (bundled, no system DLL) |
| `sha2` | 0.10 | SHA-256 of model artifacts and library music |
| `windows` | 0.62 (Windows only): `Win32_Foundation`, `Win32_Graphics_Dxgi`, `Win32_Security`, `Win32_Storage_FileSystem`, `Win32_System_JobObjects`, `Win32_System_Threading`, `Win32_System_Registry`, `Win32_System_SystemInformation` | Job Object (process.rs), DXGI / registry / memory / disk (hardware.rs) |

### Configuration files (summary)

| File | Content |
|---|---|
| [tauri.conf.json](../../apps/desktop/src-tauri/tauri.conf.json) | `productName` "Memora Studio", `identifier` `dev.memora.studio` (→ config dir `%APPDATA%\dev.memora.studio`), dev URL `http://localhost:1420`, `frontendDist ../dist`; one window 1200×800 (min 900×600); CSP `default-src 'self'`, `img-src` + `media-src` allow `asset:` / `http://asset.localhost`, `media-src` also `https://incompetech.com` and `https://www.scottbuckley.com.au` (library music previews); `assetProtocol.enable = true` with an **empty static scope** (the cache folder is allowed at runtime by `allow_cache_assets`); bundle `targets: all` |
| [capabilities/default.json](../../apps/desktop/src-tauri/capabilities/default.json) | Window `main`: `core:default`, `dialog:allow-open` only |
| [models.json](../../apps/desktop/src-tauri/models.json) | `include_str!` into models.rs. 4 manifests: `openclip-vit-b32` v4 (vision, v3 added the camera cue "view ahead", v4 the scene cues "forest trail", "small bridge", "slope"; same IR as v3; `openvino-ir`, 3 artifacts, all `url: null` → never downloadable, install from files), `whisper-base` v1, `whisper-small` v1, `silero-vad` v1 (speech, validation `none`, 1 artifact each with URL + SHA-256 + size) |
| [sounds.json](../../apps/desktop/src-tauri/sounds.json) | `include_str!` into sounds.rs. 32 tracks, CC BY 4.0: 22 Kevin MacLeod (incompetech.com) + 10 Scott Buckley (scottbuckley.com.au, added 2026-11-09 after the owner rejected the first choices; file link and licence checked on each track page, SHA-256 / size / duration measured), each with URL, SHA-256, size, duration, moods (from the artists' descriptions, unheard), energy and credit line |

---

## 2. Module map

| Module | Responsibility | Depends on (`crate::`) |
|---|---|---|
| [lib.rs](../../apps/desktop/src-tauri/src/lib.rs) | App setup, `AppState` (incl. `photo_lane`), 40 Tauri commands, `job_runner` dispatch, plan/music glue | all modules |
| [main.rs](../../apps/desktop/src-tauri/src/main.rs) | App entry point | lib (`run`) |
| [cli.rs](../../apps/desktop/src-tauri/src/cli.rs) | `memora-cli` commands over the same data folder | lib (`Db`, `job_runner`, `make_plan`, `apply_plan_to`, `recommend_for`, `vocabulary`), analysis, db, editing, export, jobs, media, projects, request, scoring, settings, sounds, tagging, timeline, transcribe |
| [bin/memora-cli.rs](../../apps/desktop/src-tauri/src/bin/memora-cli.rs) | CLI entry point | cli |
| [analysis.rs](../../apps/desktop/src-tauri/src/analysis.rs) | Store worker scene analysis; job `media.analyze` | db, jobs, media, proxy, scoring |
| [db.rs](../../apps/desktop/src-tauri/src/db.rs) | Open/migrate SQLite, backups | — |
| [editing.rs](../../apps/desktop/src-tauri/src/editing.rs) | Editing Core: styles, marks, Editing Plan, apply plan | analysis, db, export, media, scoring, tagging, timeline, transcribe |
| [export.rs](../../apps/desktop/src-tauri/src/export.rs) | Export options, FFmpeg graph, render, loudness, validation; job `media.export` | db, jobs, media, process, proxy, sounds, timeline, transcribe |
| [hardware.rs](../../apps/desktop/src-tauri/src/hardware.rs) | OS/CPU/RAM/GPU/disk detection via Windows APIs | — |
| [jobs.rs](../../apps/desktop/src-tauri/src/jobs.rs) | In-memory FIFO job queue, one job at a time | — |
| [logging.rs](../../apps/desktop/src-tauri/src/logging.rs) | tracing setup | — |
| [media.rs](../../apps/desktop/src-tauri/src/media.rs) | Media library, FFprobe, tool lookup, `MediaEnv`; job `media.import` | analysis, db, editing, jobs, process, proxy, tagging, transcribe |
| [models.rs](../../apps/desktop/src-tauri/src/models.rs) | Model Manager: manifests, curl download, verify, install, remove; job `models.install` | db, jobs, process |
| [photos.rs](../../apps/desktop/src-tauri/src/photos.rs) | Photo editor (ADR 0006 B1): photo rows, edit stacks, import / preview / export through a dedicated worker process (`Lane`) | db, export (`free_name`), jobs, process, worker |
| [process.rs](../../apps/desktop/src-tauri/src/process.rs) | Job Object, no-window `Command`, `wait_output` with timeout/cancel | — |
| [projects.rs](../../apps/desktop/src-tauri/src/projects.rs) | Project CRUD | db |
| [proxy.rs](../../apps/desktop/src-tauri/src/proxy.rs) | Thumbnails, proxies, `run_ffmpeg`, `write_atomically`, leftovers; job `media.proxy` | jobs, media, process |
| [request.rs](../../apps/desktop/src-tauri/src/request.rs) | Offline keyword parser for the request box | editing, tagging |
| [scoring.rs](../../apps/desktop/src-tauri/src/scoring.rs) | Deterministic technical score, flags, dHash duplicates | analysis, tagging |
| [settings.rs](../../apps/desktop/src-tauri/src/settings.rs) | `settings.json` (data root only), data-root layout | — |
| [sounds.rs](../../apps/desktop/src-tauri/src/sounds.rs) | Free music catalog, recommendations, verified temporary download | models, process |
| [tagging.rs](../../apps/desktop/src-tauri/src/tagging.rs) | OpenCLIP tag observations, summaries; job `media.tag` | db, jobs, media, models, proxy |
| [timeline.rs](../../apps/desktop/src-tauri/src/timeline.rs) | Project timeline (ordered trimmed clips, captions) | db |
| [transcribe.rs](../../apps/desktop/src-tauri/src/transcribe.rs) | whisper.cpp transcription; job `media.transcribe` | db, jobs, media, models, process, proxy |
| [worker.rs](../../apps/desktop/src-tauri/src/worker.rs) | Python AI worker process, protocol v1, runner | jobs, process |

---

## 3. Modules

### lib.rs

- **Purpose:** declares all modules, builds `AppState` in `run()` (settings → logging → database → reconcile → job runner → `JobManager`), registers the plugin and the 33 commands, and holds glue shared with the CLI (`make_plan`, `apply_plan_to`, `recommend_for`, `vocabulary`, `job_runner`).
- **Key types:**
  - `Db = Arc<Mutex<Result<Connection, String>>>` → the open database or the reason it is unavailable.
  - `AppState` (private) → `config_dir`, `settings: Arc<Mutex<Settings>>`, `jobs: JobManager`, `worker_info: Arc<Mutex<Option<Value>>>` (worker `hello`), `db: Db`, `log_dir`, `_log_guard`.
  - `SettingsView` (serde camelCase) → `dataRoot`, `defaultDataRoot`, `logDir`, `restartRequired` (saved root ≠ root this session logs to), `databaseError` (hidden when it is only `NO_DATA_ROOT`).
- **Functions (crate-visible, used by cli.rs):**
  - `job_runner(settings, db, worker_info) -> Result<jobs::Runner, String>` → creates one `JobObject` and the worker runner; kinds not starting with `media.`/`models.` go to the AI worker; `models.install` → `models::install_job`; `media.*` → module runners with a `MediaEnv` (`ffprobe`/`ffmpeg` from `media::tool_path`, `cache = <root>\cache`). Fails with `NO_DATA_ROOT` when no data root.
  - `make_plan(c, project_id, style, from, to, request) -> Result<Plan, String>` → parses a non-empty `request` with `request::parse`; its style and dates win over the arguments; `editing::plan_with` + `editing::save`.
  - `apply_plan_to(c, project_id, keep) -> Result<export::Options, String>` → `editing::apply`, then `editing::styled_options`; if `wishes.music` is set and ≠ `"none"`, picks the first `recommend_for` track whose energy matches (or any for `"any"`), keeps the existing music volume (else 0.35), `duck = true`; saves options.
  - `recommend_for(c, project_id, n) -> Result<Vec<Recommendation>, String>` → plan style (default `clean_minimal`), timeline clip tags (non-negative, `summarize` min 0.1, weighted by clip length), total length and speech share → `sounds::recommend`.
  - `vocabulary(c) -> Result<Vec<TagName>, String>` → distinct tag names (by `en`) across all tagged media.
  - Private: `cache_dir`, `allow_cache_assets` (asset scope = `<root>\cache` recursive), `open_db` (`<root>\database\memora.db`), `reconcile` (`models::reset_interrupted` + `proxy::remove_leftovers`, run only when no job runs), `with_db`, `view`.
- **Constants:** `NO_DATA_ROOT = "Choose a data folder first."`; `UI_JOB_KINDS = [validate_devices, media.import, media.proxy, media.export, media.analyze, media.transcribe, media.tag, models.install]` (`diagnostic.*` are test-only and refused).
- **Tauri commands:** all 33, see §4. Event emitted: `job-updated` (payload `Job`) on every job change.
- **DB:** through the modules it calls.
- **Child processes:** none directly.
- **Files:** `set_data_root` creates the data-root layout; startup/switch runs `remove_leftovers`; `remove_media` deletes the media's cache files.
- **Tests:** none (covered through modules and cli.rs).
- **Notes:** `set_data_root` refuses while `jobs.busy()` (a running job would write to the wrong database); it opens the new database before saving settings, so a broken folder is never saved. File logging keeps the folder chosen at launch (`restartRequired`).

### main.rs

- **Purpose:** app binary; calls `memora_desktop_lib::run()`. `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` hides the console in release.
- **Tests:** none.

### cli.rs

- **Purpose:** `memora-cli`, the engine without the window: same data folder, database, Editing Core and export. Plain-text output for people and assistants. Jobs run synchronously with the same `job_runner` as the app.
- **Key types:** `Args { pos, flags }` (`--key value` pairs; every flag needs a value), `Ctx { root, db: Db }`.
- **Public functions:** `run(args: Vec<String>) -> i32` → prints result to stdout (exit 0) or `error: …` to stderr (exit 1).
- **Data root:** `MEMORA_DATA_ROOT` env var, else `settings.json` in `%APPDATA%\dev.memora.studio`; error when none.
- **Commands** (from `HELP`): `status`, `media`, `concepts`, `scenes MEDIA`, `transcript MEDIA`, `job KIND [JSON]` (only `media.import|proxy|analyze|tag|transcribe` and `models.install`, e.g. offline `{"id", "files": {artifact: path}}`), `projects`, `new-project NAME`, `styles`, `understand TEXT`, `plan PROJECT --request "…"` (default style `travel_cinematic`) or `plan PROJECT --style ID [--from] [--to] [--target SECONDS] [--prefer] [--less] [--avoid]`, `mark MEDIA [--favorite on|off] [--exclude on|off]` (`editing::set_mark`), `show-plan PROJECT`, `apply PROJECT [--keep 0,2,1]`, `timeline PROJECT`, `clip-add PROJECT MEDIA IN OUT`, `clip-trim CLIP IN OUT`, `clip-remove CLIP`, `clip-move CLIP -1|1`, `clip-caption CLIP TEXT`, `options PROJECT [--color --transition --transition-ms --fill --fade --normalize --volume --title --subtitle --ending --captions --music FILE|none --music-library ID --music-volume --duck]`, `sounds [PROJECT]`, `export PROJECT [--preset]` (default: plan's preset, else `landscape`), `help`.
- **Behaviour details:** times in seconds (`ms()` rounds to ms); booleans accept on/true/yes/1 and off/false/no/0; `options` validates with `Options::check` before saving and saves only when flags were given; default music for the CLI = `volume 0.35, duck true` (same as the app picker); `job` progress printed to stderr in 10 % steps; `concepts` counts seconds where a concept is the top tag with probability ≥ 0.2.
- **Job kinds:** runs `media.export` (from `export`) and the five `job` kinds above in-process; cancel callback is always `false`.
- **DB:** reads everything; writes projects, timeline, plans, export options via the modules.
- **Tests:** `parse_times_and_switches`, `project_timeline_options_and_plan_round_trip`.
- **Notes:** "Close the app before running jobs here": nothing enforces it (no lock between processes). The CLI never calls `logging::init`, so `tracing` events from modules are not recorded when running the CLI. `clip-trim` (`timeline::set_range`) has no Tauri command equivalent.

### bin/memora-cli.rs

- **Purpose:** CLI binary; `std::process::exit(cli::run(args.skip(1)))`.
- **Tests:** none.

### analysis.rs

- **Purpose:** stores the classical analysis observations (scenes + per-sample metrics) returned by the worker's `analyze_video` (run on the proxy; times map 1:1 to the original). Scores are computed on read in scoring.rs.
- **Key types:**
  - `Sample` (camelCase, serde both ways) → `atMs`, `sharpness` (Laplacian variance at 540p), `brightness`, `contrast`, `clipLow`, `clipHigh`, `black`, `dhash` (hex string).
  - `Scene` → `id`, `mediaId`, `index`, `startMs`, `endMs`, `shake`, `framePath`, `samples`.
  - `WorkerScene` (private, deserialize) → worker result shape.
- **Public functions:**
  - `store(c, media_id, result: &Value) -> Result<usize, String>` → replaces analysis in a transaction; skips scenes with `endMs <= startMs`; returns scene count.
  - `clear(c, media_id) -> Result<(), String>` → deletes scenes + `media_analysis` row.
  - `scenes(c, media_id) -> Result<Vec<Scene>, String>`.
  - `library_hashes(c) -> Result<Vec<(i64, i64, String)>, String>` → (media, scene index, representative dHash) for duplicate detection.
  - `scene_counts(c) -> Result<HashMap<i64, i64>, String>` → analysed media only.
  - `analyze_job(job, progress, cancelled, env, ai) -> Outcome`.
- **Constants:** `STEP = 1000` per file; `PROXY_PART = 300.0` (share of a file's progress spent creating a missing proxy).
- **Job:** `media.analyze`, params `{"ids": [i64], "quality": "fast"|"balanced"|"detailed"}` (default `balanced`) → result `{"done": [{"id", "scenes"}], "failed": [{"id", "error"}]}`. Per file: `proxy::ensure_proxy`, then worker sub-job `analyze_video` with `{"path": <proxy>, "framesDir": <cache>\frames\<id>, "quality"}`.
- **DB:** writes `media_analysis`, `scenes`; reads `media`, `scenes`.
- **Child processes:** FFmpeg (proxy, via proxy.rs), Python worker (via `ai`).
- **Files:** reads `cache\proxy\<id>.mp4`; worker writes scene JPEGs to `cache\frames\<id>\`.
- **Tests:** `store_replaces_and_cascades`, `analyze_job_with_real_worker` (skips without FFmpeg or worker venv).
- **Notes:** invalid `samples_json` reads as empty (`unwrap_or_default`).

### db.rs

- **Purpose:** opens `<data root>\database\memora.db` in WAL mode, backs it up, and applies append-only migrations tracked by `PRAGMA user_version`. Refuses databases with a newer schema.
- **Key items:** `MIGRATIONS` (9 entries, never edit shipped ones):
  1. `projects`; 2. `media` (`path` UNIQUE COLLATE NOCASE, raw `probe_json`); 3. `project_clips` (+ index, cascades on project/media delete); 4. `media_analysis`, `scenes`; 5. `models`; 6. `transcripts`; 7. `media_tags`; 8. `media_marks`, `editing_plans`; 9. `project_clips.caption` column + `project_settings` (`export_json`). Details in [DATABASE.md](DATABASE.md).
- **Public functions:**
  - `err(e: rusqlite::Error) -> String` → `"Database error: {e}"`, the standard mapper.
  - `open(path) -> Result<Connection, String>` → creates the folder, sets WAL, backs up an existing database (label `before-upgrade` when migrations are pending — failure then aborts the open; `startup` otherwise, skipped when a backup is younger than 10 min, failure only logged), then `init` (foreign keys on, busy timeout, migrate).
  - `backups_dir(db_path) -> PathBuf` → `<database folder>\backups`.
  - `backup(c, dir, label) -> Result<PathBuf, String>` → `VACUUM INTO memora-<UTC yyyymmdd-HHMMSSfff>-v<schema>-<label>[-n].db`, verified read-only with `PRAGMA integrity_check` (deleted if not `ok`), prunes to the newest `KEEP_BACKUPS`.
  - `open_in_memory()` (`#[cfg(test)]`, `pub`) → migrated in-memory DB for tests.
- **Constants:** `KEEP_BACKUPS = 10`; `STARTUP_BACKUP_INTERVAL = 10 min`; busy timeout `5 s`.
- **DB:** all tables (schema owner).
- **Files:** `database\memora.db` (+ `-wal`, `-shm`), `database\backups\memora-*.db`.
- **Tests:** `fresh_database_gets_all_migrations`, `migrate_is_idempotent`, `refuses_newer_schema`, `opens_file_with_wal`, `existing_database_is_backed_up_on_open_and_old_backups_pruned`.
- **Notes:** restore procedure (doc comment): close the app, copy a backup over `memora.db`, delete `memora.db-wal` and `memora.db-shm`.

### editing.rs

- **Purpose:** the Editing Core. Turns technical scores, OpenCLIP tags, transcripts and user marks into an explicit, deterministic, reviewable Editing Plan; applies a plan to the timeline and the style's look to export options. AI observes → this module decides → export.rs renders.
- **Key types:**
  - `Style` (serialize) → `id`, `name`, `targetMs`, `exportPreset`, `clipMinMs`, `clipMaxMs`, `clipIdealMs`, `transitionMs`, `music`, `titles`, `pacing`, `opening`, `ending`, `weights` [technical, semantic, speech, favorite], `favor`, `openWith`, `endWith`, `transition`, `color`, `captions`.
  - `Source` → one library file with `media`, scored `scenes`, `tags`, `transcript`, `favorite`, `excluded`.
  - `Mark` → `favorite`, `excluded` (user override).
  - `Scores` → `technical`, `semantic`, `speech`, `favorite` (kept separate, combined only for ranking).
  - `PlanClip` → `mediaId`, `sceneIndex`, `inMs`, `outMs`, `scores`, `rank`, `tags` (top 3 Vietnamese labels), `reasons`, `photoId` (serde default; a photo clip has `mediaId` = −photo id, `sceneIndex` −1).
  - `PhotoSource { photo, analysis: Option<photos::Analysis> }`; `gather_photos(c, from, to)` (photo local date in range).
  - `plan_with_photos(st, sources, photos, wishes)` (`plan_with` = no photos, tests only): target as before, then `pick_photos` (taken within the analysed videos' recording span ± `PHOTO_TRIP_MARGIN_MS` 3 h, all photos when there is no video; drops blurry / dark / overexposed / black (`scoring::technical` on the one sample), bad-shot tags ≥ `PHOTO_BAD_SHOT` 0.3, avoided concepts; one per near-identical dHash (`SAME_PICTURE`); rank = technical × prefer / less; at most `PHOTO_SHARE` 25 % of the length (`fewer` 12 %, `more` 40 %, all when no video), best of each equal run in time order; `PHOTO_MS` 4 s or `wishes.photoMs`) → videos fill `target − photos` (≥ half) → `place_photos` after the last clip recorded before the photo (opening clip kept first, ending last) → `output_ms` with photo joins. Notes: photos used, outside the trip, not analysed.
  - `Wishes` (serde default) → `prefer`, `less`, `avoid` (concept names, `en` or `vi`, case-insensitive), `targetMs`, `preset`, `color`, `title`, `ending`, `captions`, `music` (`none`|`low`|`medium`|`high`|`any`), `photos` (`none`|`fewer`|`yes`|`more`), `photoMs`, `request` (original sentence).
  - `Plan` → `style`, `wishes`, `targetMs`, `totalMs`, `transitionMs`, `exportPreset`, `clips`, `skipped` (why files were not used), `candidates`, `suggestedTitle`, `notes` (how the plan was built: auto length, coverage clips; serde default).
- **Styles** (`styles()`):

| id | target | preset | clip min/ideal/max | transition | weights T/S/Sp/F | look | captions |
|---|---|---|---|---|---|---|---|
| `travel_cinematic` | 90 s | landscape | 3 / 4.5 / 6 s | 600 ms fade | .45/.4/.05/.1 | cinematic | no |
| `social_story` | 30 s | reels | 1.5 / 2 / 3 s | 0 (cuts) | .4/.4/0/.2 | vivid | no |
| `memory_nostalgia` | 180 s | landscape | 3 / 5 / 7 s | 800 ms fade | .2/.4/.2/.2 | warm | no |
| `travel_documentary` | 300 s | landscape | 5 / 8 / 15 s | 300 ms fade | .3/.25/.35/.1 | none | yes |
| `journey_recap` | 300 s (auto from the footage when no length is asked) | landscape | 4 / 6 / 9 s | 700 ms `auto` | .4/.35/.1/.15 | warm | no |
| `clean_minimal` | 60 s | landscape | 3 / 4 / 5 s | 0 (cuts) | .6/.25/0/.15 | none | no |

- **Public functions:**
  - `styles() -> Vec<Style>`; `style(id) -> Result<Style, String>`.
  - `marks(c) -> Result<HashMap<i64, Mark>, String>`; `set_mark(c, media_id, mark) -> Result<(), String>` (upsert).
  - `day(m: &Media) -> String` → recording date `YYYY-MM-DD` (import date when unknown).
  - `gather(c, from, to) -> Result<Vec<Source>, String>` → library files recorded within the inclusive date range, with scores, tags, transcript, marks.
  - `plan_with(st, sources, wishes) -> Plan` (and `plan(st, sources)` in tests).
  - `styled_options(o, st, plan) -> export::Options` → `fill = preset == "reels"`, style transition + length, colour (wish wins), `fade = true`, `normalize = true`, captions (wish wins), title (wish, else keep user's, else `suggestedTitle`), ending wish, `music = None` when wish is `none`; with music (present or wished) and no subtitles, `originalVolume = min(originalVolume, AMBIENCE_VOLUME)`; without music a level equal to `AMBIENCE_VOLUME` goes back to 1.0.
  - `save(c, project_id, plan)`; `load(c, project_id) -> Result<Option<Plan>, String>`; `apply(c, project_id, keep: &[usize]) -> Result<usize, String>` (replaces the timeline in a transaction, in `keep` order; bad index rolls back).
  - `LANDSCAPES` (`pub(crate)`) → used by request.rs for "nature". `JOURNEY` → concepts `journey_recap` favours (landscapes, river, waterfall, hiking trail, big tree, bicycle, group of friends, boat).
- **Planning algorithm:** candidate windows per scene (whole scene if it fits, else consecutive `ideal` windows; remainder kept only if ≥ min) → windows whose "view ahead" cue (`tagging::cue`, model v3) is below `VIEW_DOWN` are left out and counted ("camera pointing down at the handlebars / ground") unless the "forest trail" cue (model v4) is ≥ 0.9; on a trail, when the main tag is a ride or walk (`ON_THE_MOVE`), the first `SCENE_CUES` entry over its threshold (small bridge, slope, forest trail) becomes the window's first tag and dominant concept → transcript lines with implausible timing (`transcribe::Segment::timing_plausible`) are ignored → `fit_speech` moves edges out of speech (extend to the whole utterance up to `3 × clip_max`, else shrink, else drop; 4 iterations) → per window: avoid filter, semantic score (untagged 0.5; no `favor` → top tag score; else `0.5·top + 0.5·min(1, 2·favor score)`), speech share, window-local technical score, favorite → rank = weights × scores × bad-shot penalty × prefer/less factor × `view_factor` (0.6 + 0.7·max(view, 0.57·forest trail on a trail); no cue → 1) → `journey_recap` only: length from the footage when the user names none (`RECAP_S_PER_FOOTAGE_MIN`, clamped `RECAP_MIN_MS`..`RECAP_MAX_MS`, note added) and a coverage pass (best clean window of every concept with ≥ `COVER_CONCEPT_MS` of candidates, then of every `COVER_SLICE_MS` slice of recording time without one; within the target; note added) → greedy selection until `target − clip_min` (no overlap, no duplicate scene, `new_content`, concept share first, then rank × diversity, then distance from same-file picks, then time order) → chronological sort → opening (`open_with` pick, or best clean clip for "Strongest…" hooks) and ending (`end_with`) → `merge_near` → `suggest_title` ("Concept · dd/mm/yyyy" or month range).
- **Constants:** `SCENE_TAG_MIN = 0.1`; `BAD_SHOT_WEIGHT = 2.0` (penalty capped at 80 %); `AVOID_MIN = 0.25`; `PREFER_BOOST = 0.5` (rank × up to 1.5); `LESS_CUT = 0.5` (rank × down to 0.5); `SAME_PICTURE = 12` (dHash bits); `SCENE_MS_PER_CLIP = 60_000` (per concept, ≤ 1 + scene/60 s clips per scene); `MAX_CONCEPT_SHARE = 0.35` (**initial value**, "tune on real libraries"); `MERGE_GAP_MS = 1_500`; diversity factors ×0.85 per same dominant concept (at most 3 times), ×0.9 per same file, ×0.6 per same-file pick closer than 3 ideal lengths; opening/ending pick threshold 0.2; bad-shot reason shown at ≥ 0.15; speech reason at ≥ 0.3; `VIEW_CUE = "view ahead"`, `VIEW_DOWN = 0.1` (**initial value**, checked on 850 frames of the owner's footage); `CONCEPT_REPEAT_MAX = 3` (the ×0.85 same-concept factor stops after 3 repeats; the share cap limits a concept); `SCENE_CUES = [(small bridge, 0.6), (slope, 0.95), (forest trail, 0.9)]` (**initial values**, 1 010 frames of the owner's footage, 2026-11-11); `ON_THE_MOVE = [bicycle, motorbike, hiking trail, forest, park, road trip on a highway]`; `RECAP_S_PER_FOOTAGE_MIN = 4.2`, `RECAP_MIN_MS = 90 s`, `RECAP_MAX_MS = 360 s`, `COVER_CONCEPT_MS = 20 s`, `COVER_SLICE_MS = 8 min` (**initial values**, one trip); `AMBIENCE_VOLUME = 0.35` (clips' own sound under music, −9 dB; **initial value** from one library: DJI own sound ≈ −25 dBFS is above the ≈ −34 dBFS ducking threshold).
- **DB:** reads `media`, `scenes`, `media_tags`, `transcripts`, `media_marks`; writes `media_marks`, `editing_plans`, `project_clips` (apply).
- **Child processes / files:** none.
- **Tests:** `windows_follow_style`, `plan_respects_target_exclusion_and_is_deterministic`, `semantics_speech_favorites_and_structure`, `long_single_shot_is_sampled_spread_out`, `speech_is_never_cut_mid_sentence`, `one_unchanging_shot_is_not_chopped`, `near_clips_merge`, `title_suggestion_and_style_look`, `bad_shots_are_penalised_and_never_the_hook`, `variety_beats_one_repeated_activity`, `wishes_prefer_avoid_and_length`, `applying_honours_look_and_text_wishes`, `same_day_clips_follow_recording_time`, `journey_recap_keeps_trip_order_and_longer_clips`, `raised_camera_beats_camera_pointing_down`, `forest_trails_and_bridges_kept_though_handlebars_fill_the_frame`, `stretched_whisper_lines_do_not_stretch_clips`, `duplicates_are_not_picked_twice`, `marks_plans_and_apply_round_trip`, `real_library_plans` (`#[ignore]`, needs `MEMORA_DB` = a copy of the database).
- **Notes:** `ponytail:` greedy O(n²) over candidates; fine for a few thousand windows, revisit for whole-year libraries. The `music` / `titles` text of every style (`NO_MUSIC`, `NO_TITLES`) still says music and burned-in titles are not available, although export.rs now renders both.

### export.rs

- **Purpose:** renders a project timeline from the ORIGINAL files (never proxies) with FFmpeg: trim, fit/crop, transitions, colour, fades, ASS text, own sound, optional music with ducking, loudness; then a second loudness pass and §8.3 validation.
- **Key types:**
  - `Preset` → `Story`, `Reels` (1080×1920 @ 30), `Landscape` (1920×1080 @ 30), `Archive` (first clip's upright size, its fps capped at 60, default 30). `parse(&str)`, `target(&Media)`.
  - `Segment` → `media`, `in_ms`, `out_ms`, `caption`, `speech` (transcript segments), `transition` (the user's transition into it, from the timeline clip). A still photo (photo video) is a `Segment` whose `media.probe.video_codec == STILL` (`"still"`), path = rendered JPEG, no audio.
  - `Music` (camelCase, default) → `path` (user file, never modified), `library` (track id), `volume` 0..2, `duck`.
  - `Titles` → `title`, `subtitle`, `ending`, `captions` (burn speech subtitles).
  - `Options` (camelCase, every field defaults) → `fill`, `transitionMs`, `transition`, `color`, `fade`, `originalVolume`, `music`, `titles`, `normalize`, `credit` (`#[serde(skip)]`, set by the job for library music), `intermediate` (`#[serde(skip)]`, parts of a long render: QSV `global_quality 16` / x264 `crf 14`, AAC 320k), `fixed_joins` (`#[serde(skip)]`, joins decided from the whole timeline, used by `render_parts`). Defaults: no fill, 0 ms, `fade`, `none`, no fade, 1.0, no music, empty titles, no normalize.
  - `Encoder` → `Qsv`, `X264`. `Check { name, ok, detail }`, `Validation { ok, checks, warnings }`.
- **Public functions:**
  - `load_options(c, project_id) -> Options` (defaults when absent/invalid); `save_options(c, project_id, &o) -> Result<(), String>` (checks first).
  - `Options::check()` → transition ∈ `TRANSITIONS`, colour ∈ `COLORS`, volumes 0..2, library track exists or music file exists.
  - `Join = (String, i64)` (xfade name, ms); `JoinClip = (media id, recording start ISO 8601?, in, out)`.
  - `joins(transition, transition_ms, clips) -> Vec<Join>` → one per join; named transitions keep their length; `auto` (2026-11-11): same file ≤ `AUTO_SAME_SHOT_MS` apart → `fade` `AUTO_SHORT_MS` (capped by the length), recorded ≥ `AUTO_CHAPTER_MS` apart → `fadeblack` max(`AUTO_CHAPTER_FADE_MS`, length), else `fade` with the length; 0 ms → cuts. Every join clamped to half of each neighbouring clip (audit E4). Used by export, `editing::output_ms` (plan total) and `cli` `timeline`.
  - `joins_with(transition, ms, clips, into)` → `joins`, then each `into[i]` (user transition before clip i) replaces join i−1 (`auto` allowed; clamped the same way). `timeline_joins(c, project_id) -> Vec<Join>` → the joins of a project timeline as exported (videos keyed by media id, photos by −photo id and dated by `taken_at`); used by the UI (`list_joins`), `preview_join` and CLI `timeline`.
  - `Options::joins(segs)` (`fixed_joins` when set, clamped again; else `joins_with` with each segment's `transition`); `Options::duration(segs) -> i64` (sum − join lengths); `unix_ms(iso) -> Option<i64>` (crate-public, days-from-civil, UTC).
  - `subtitles(segs, o, (w, h)) -> Option<String>` → ASS script: `Title` (0–4 s, title + smaller subtitle), `Label` (clip caption, ≤ 4 s at clip start, boxed), `Caption` (speech clipped to the clip, shifted by overlaps), ending `Title` (last 3.5 s), `Credit` (last 4 s); font Segoe UI Semibold; side margin 8 % of width; bottom margin 18 % of height for vertical, 8 % otherwise; user braces/backslashes neutralised. `None` when no text.
  - `measure_lufs(env, file) -> Result<f64, String>` → `ebur128` integrated loudness.
  - `level_loudness(env, out, cancelled) -> Result<(f64, f64), RunError>` → second pass to `TARGET_LUFS` (gain clamped ±12 dB, skipped below 0.5 dB), `alimiter limit=0.891`, video copied, audio AAC 192k, via `<out>.level.mp4`.
  - `build_args(segs, decode, encoder, (w, h, fps), o, ass) -> Vec<OsString>` → per segment `-ss/-t -i` (stills: `-loop 1 -framerate <fps> -t -i`, never hardware-decoded, no rotation filter) (+ `anullsrc` stereo 48 kHz when no audio); letterbox on a blurred, darkened copy when aspect differs > 5 % and not `fill`, else `scale+pad` or `scale+crop`; `concat` (all joins 0) or chained `xfade`/`acrossfade` with each join's own name and length; colour filter, fade (`FADE_MS` capped at total/4), `ass` with `%WINDIR%\Fonts`; `volume`; music `-stream_loop -1`, trimmed, 1.5 s fade-out, optional `sidechaincompress` duck (threshold 0.02, ratio 8, attack 20, release 400), `amix normalize=0`; `loudnorm I=-16:TP=-1.5:LRA=11`. Encoders: QSV `h264_qsv preset medium global_quality 22`, x264 `preset medium crf 20`; AAC 192k, `+faststart`.
  - `validate(env, out, expected_ms, segments, target) -> Validation` → checks `file exists`, `duration` (± 200 ms + 100 ms per segment), `resolution` (exact, rotation 0), `audio present`, `decodes fully` (`-xerror`, `blackdetect d=1 pix_th=0.10`, `volumedetect`); warnings for black ranges, peaks ≥ −0.1 dB, mean ≤ −80 dB.
  - `render(env, segs, target, o, out, progress, cancelled) -> Result<&'static str, RunError>` → writes a temporary `cache\render\titles-<pid>-<nanos>.ass`, tries `hybrid-qsv` (Hybrid decode + QSV) then `libx264` (software), each via `write_atomically`; timelines over `MAX_INPUTS` clips go through `render_parts` (parts of ≤ 10 clips rendered from the originals with picture, own sound and the transition into `cache\render\part-<pid>-<nanos>-<i>.mp4` as `intermediate`, deleted afterwards via `sounds::TempFile`; then one joining pass with the boundary joins taken from the whole timeline adds colour, fades, ASS, volume, music and loudness — same duration and text timing; progress 0–60 % parts, 60–100 % join); storage errors are not retried; ASS file deleted afterwards.
  - `export_job(job, progress, cancelled, env, projects, ai) -> Outcome` → loads the timeline (video clips → segments; photo clips → `photos::still` with the job worker `ai`, stills deleted afterwards; progress 0–100 for stills when there are photos), then `finish`.
  - `preview_join(env, project_id, index, preset, still) -> Result<PathBuf, String>` → end of clip `index` + the join + start of the next (each side `max(1.5 s, join)` plus the join), from the proxies when present (upright, same timing) else originals, photos via `still(photo, out) -> (w, h)`; half the preset size, ≤ 30 fps, project fill + colour only, x264 `veryfast`, `cache\render\join-preview-<nanos>.mp4` (older ones removed; temporary `join-still-*.jpg` deleted).
  - `finish(env, segs, target, preset_name, o, planned, progress, cancelled) -> Outcome` → shared end of every video export (projects and `photos::video_job`): options check, export folder, library music, `render`, loudness pass, validation, promotion to `planned` (or `-failed-validation`), result JSON.
- **Constants:** `TRANSITIONS = [auto, fade, dissolve, fadeblack, smoothleft, slideleft, wipeleft, circleopen]` (xfade `fade` = clean cross dissolve, `dissolve` = grainy); `AUTO_SAME_SHOT_MS = 60 s`, `AUTO_SHORT_MS = 400`, `AUTO_CHAPTER_MS = 20 min`, `AUTO_CHAPTER_FADE_MS = 1000` (**initial values**); `COLORS = [none, warm, cool, vivid, cinematic, bw, vintage]` (gentle `colorbalance`/`eq`/`curves`/`vignette` grades); `FADE_MS = 600`; `TARGET_LUFS = -16.0`; `LUFS_TIMEOUT = 600 s` (**assumed** generous limit); `MAX_INPUTS = 10` (originals per FFmpeg run; **initial value** — measured 2026-11-08: 26 × 2.7K HEVC inputs in one run peaked at 11 GB working set / 12 GB commit, parts of 10 at 5.2 / 5.6 GB, render 149 s vs 121 s).
- **Job:** `media.export`, params `{"projectId": i64, "preset": "story"|"reels"|"landscape"|"archive", "options": Options}` (legacy `fit: "fill"` / `transitionMs` read when `options` absent) → result `{"path", "preset", "encoder", "renderSeconds", "durationMs", "joins": [[name, ms]…], "options", "music": {title, credit, file}|null, "loudness": {measuredLufs, gainDb, targetLufs}|null, "width", "height", "validation"}`. Fails on empty timeline or a missing original. Progress units: total 1000 = 0–50 library music download, 50–900 render, 900 validation, 1000 done.
- **DB:** reads `projects`, `project_clips`, `media`, `transcripts`, `project_settings`; writes `project_settings`.
- **Child processes:** FFmpeg (render, loudness pass, validation decode, LUFS measure), FFprobe (validation), curl.exe (library music via sounds.rs). Render/level/validate: killed on cancel (render/level only) with no timeout; LUFS measure: `wait_output` 600 s, not cancellable.
- **Files:** output `projects\<id>\exports\<safe name>-<preset>-<unix secs>.mp4` (never overwrites: `-2`, `-3`…); staged as `….unvalidated.mp4` (+ `.part`, `.level.mp4`), renamed to the final name, or `…-failed-validation.mp4` when validation fails (kept); library music kept in `music\<id>.mp3` (downloaded once, verified on every export); ASS and long-render parts in `cache\render\`.
- **Tests:** `auto_joins_follow_recording_time_and_clamp_per_join`, `stills_are_looped_and_never_hardware_decoded`, (mixed timeline + join preview: photos.rs `mixed_timeline_export_and_join_preview`), `subtitles_timing_text_and_safe_area`, `options_are_checked`, `output_name_is_safe`, `earlier_exports_are_never_replaced`, `storage_errors_are_not_retried`, `export_and_validate_with_real_ffmpeg` (skips without FFmpeg; QSV comparison skipped when unavailable; also forces `render_parts` with one clip per part and checks duration, validation and part removal), `real_library_export` (`#[ignore]`, `MEMORA_DB`, `MEMORA_OUT`, optional `MEMORA_STYLE`, `MEMORA_MUSIC`).
- **Notes:** `ponytail:` Archive follows the first clip only; mixed-resolution timelines get letterboxed to it. `ponytail:` centre crop only; subject-aware crop needs detections (comment sits above `Options::joins`, not the crop code).

### hardware.rs

- **Purpose:** hardware report with Windows APIs only (no child processes). GPU memory from DXGI, so dedicated VRAM and shared system RAM stay separate. OpenVINO devices are reported by the worker (`validate_devices`), not here.
- **Key types** (camelCase): `HardwareInfo { os, cpu, memory, gpus, dataDisk }`; `OsInfo { name, edition, displayVersion, build }`; `CpuInfo { name, logicalProcessors }`; `MemoryInfo { totalPhysical, availablePhysical, commitLimit, commitAvailable, loadPercent }` (bytes; commit = RAM + pagefile); `GpuInfo { name, vendorId, dedicatedVideoMemory, sharedSystemMemory, driverVersion }`; `DiskInfo { path, total, free }`.
- **Public functions:** `detect(data_root: Option<&Path>) -> HardwareInfo`.
- **Sources:** registry `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` (`CurrentBuild`, `UBR`, `EditionID`, `DisplayVersion`, `ProductName`) — name forced to "Windows 11" when build ≥ 22000; `HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0\ProcessorNameString`; `GlobalMemoryStatusEx`; DXGI `EnumAdapters1` (software adapters skipped; UMD driver version from `CheckInterfaceSupport`); `GetDiskFreeSpaceExW` on the nearest existing ancestor of the data root.
- **Tests:** `detects_basic_hardware`.

### jobs.rs

- **Purpose:** in-memory background job manager: FIFO, exactly one job at a time (resource policy), progress, cancellation, change notifications. Jobs do not survive a restart.
- **Key types:** `Status` (lowercase: `queued`, `running`, `completed`, `failed`, `cancelled`); `Job { id: u64, kind, params, status, done, total, result, error }` (camelCase); `Outcome { Completed(Value), Cancelled, Failed(String) }`; `Runner = Box<dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome + Send>`; `Notify = Box<dyn Fn(&Job) + Send + Sync>`; `JobManager` (clonable `Arc`).
- **Public functions:** `JobManager::start(runner, notify) -> Self` (spawns thread `job-executor`); `submit(kind, params) -> Job`; `cancel(id) -> Result<(), String>` (queued → cancelled at once; running → flag checked by the runner; finished → error); `list() -> Vec<Job>`; `busy() -> bool`.
- **Constants:** `KEEP_FINISHED = 50` (older finished jobs pruned, active ones always kept).
- **Tests:** `completes_with_progress`, `failure_is_reported`, `runs_in_fifo_order_one_at_a_time`, `cancel_queued_and_running`, `prune_keeps_recent_finished_and_all_active`.
- **Notes:** every progress update calls `notify` (→ `job-updated` event); outcome logged at info/warn.

### logging.rs

- **Purpose:** tracing setup: human-readable on stderr, JSON lines in `<data root>\logs\memora.log.YYYY-MM-DD` (daily rolling, non-blocking).
- **Public functions:** `init(log_dir: Option<PathBuf>) -> (Option<PathBuf>, Option<WorkerGuard>)` → without a data root (first run) only stderr, nothing on disk.
- **Constants/env:** `MEMORA_LOG` (EnvFilter syntax), default `info`.
- **Tests:** none.

### media.rs

- **Purpose:** media library. Originals are referenced by path and only read; metadata from FFprobe; the raw FFprobe JSON is stored.
- **Key types:**
  - `Probe` (camelCase) → `durationMs`, `width`/`height` (coded size), `rotation` (CCW degrees 0..359, display-matrix convention), `fps`, `videoCodec`, `audioCodec`, `audioStreams`, `bitrate`, `sourceCreatedAt` (camera value, unmodified).
  - `Media` (camelCase, `probe` flattened) → `id`, `path`, `size`, probe fields, `importedAt`, plus fields filled by `list_media`: `thumb`, `proxy`, `sceneCount`, `transcribed`, `tags`, `mark`.
  - `MediaEnv<'a>` → `ffprobe`, `ffmpeg`, `cache`, `job_object`, `db`; `with_db(f)`.
- **Public functions:**
  - `parse_probe(&Value) -> Result<Probe, String>` → first video stream that is not an attached picture; rejects no video / no duration; rotation from side data, else legacy `rotate` tag (clockwise, negated); date from `com.apple.quicktime.creationdate` else `creation_time`.
  - `tool_path(data_root, exe) -> PathBuf` → `<root>\runtime\ffmpeg\<exe>` → `PATH` → `%LOCALAPPDATA%\Microsoft\WinGet\Links\<exe>` → bare name.
  - `run_ffprobe(ffprobe, file, job_object) -> Result<String, String>`.
  - `upsert(c, path, size, modified_unix, probe, raw) -> Result<Media, String>` (same path case-insensitive → refresh).
  - `list(c)` (newest recording first, undated last); `get(c, id)`; `remove(c, id)` (row only; original untouched; cascades).
  - `import_job(job, progress, cancelled, env) -> Outcome`.
- **Constants:** `FFPROBE_TIMEOUT = 60 s` (**assumed** limit, "not measured on slow network drives").
- **Job:** `media.import`, params `{"paths": [string]}` → result `{"imported": n, "ids": [i64], "failed": [{"path", "error"}]}`. Progress: 1 unit per file. When size or mtime of an already imported path changed, analysis, transcript and tags are cleared and the cache deleted. A thumbnail is (re)made after each import (failure only logged).
- **DB:** writes `media`; clears `scenes`/`media_analysis`, `transcripts`, `media_tags` on change.
- **Child processes:** FFprobe (`-v error -print_format json -show_format -show_streams`, `wait_output` 60 s, not cancellable), FFmpeg (thumbnail).
- **Files:** `cache\thumbs\<id>.jpg`; `remove_cache` on change.
- **Tests:** `parses_ffprobe_output`, `phone_metadata_rotation_and_dates`, `rejects_non_videos`, `upsert_list_remove`, `import_job_with_real_ffprobe` (also runs `media.proxy`; skips without FFmpeg).
- **Notes:** `ponytail:` winget fallback is a dev convenience; drop once the installer ships `runtime\ffmpeg`.

### models.rs

- **Purpose:** Model Manager: manifests from models.json, install pipeline (resumable curl.exe download or copy of local files → size + SHA-256 → versioned install → load validation), removal, crash recovery. Downloads only for artifacts with URL + SHA-256 + size.
- **Key types:** `Artifact { file, url, revision, sha256, sizeBytes }`; `RuntimeSupport { runtime, devices, validated }`; `Manifest { id, name, version, category, description, required, license, minimumRamGb, validation ("openvino-ir"|"none"), artifacts, runtimes, notes }` with `download_blocker()`; `ModelView` (manifest flattened + `state`, `error`, `path`, `details`, `downloadBlocker`); `Step { Cancelled, Failed(state, msg) }` (`pub(crate)`).
- **States:** `not_installed`, `downloading`, `verifying`, `installing`, `validating`, `ready`, `download_failed`, `checksum_failed`, `install_failed`, `model_load_failed`, `incompatible`, plus `update_available` computed by `list` when a `ready` row's version ≠ manifest version.
- **Public functions:**
  - `manifests() -> Vec<Manifest>`; `list(c) -> Result<Vec<ModelView>, String>`.
  - `ready_dir(c, id) -> Result<(Manifest, PathBuf), String>` → install folder when `ready`, else "Install … in AI models first." / "Update … first".
  - `reset_interrupted(c) -> Result<usize, String>` → rows in a working state → `install_failed` ("Interrupted…"); partial downloads kept.
  - `install_dir(models_root, m) -> PathBuf` → `models\<category>\<id>\<version>\`.
  - `sha256_file(path, cancelled, progress) -> Result<Option<String>, String>` (1 MiB chunks; `None` = cancelled).
  - `download(url, dest, job_object, progress, cancelled) -> Result<(), Step>` (`pub(crate)`, also used by sounds.rs) → `curl.exe --location --fail --silent --show-error --retry 3 --continue-at - --output <dest>.part`, polled every 250 ms, progress = partial size, renamed when done; `.part` kept on cancel for resume.
  - `install(m, local_files, models_root, job_object, db, validate, progress, cancelled) -> Outcome`.
  - `remove(c, models_root, id) -> Result<(), String>` → deletes install folder, partial downloads and row.
  - `install_job(job, progress, cancelled, models_root, job_object, db, ai) -> Outcome`.
- **Job:** `models.install`, params `{"id": string, "files": {"<artifact file>": "<local path>"}?}` → result `{"id", "path", "details"}`. Validation: `none` → `{"validation": "checksum only"}`; `openvino-ir` → worker sub-job `validate_openvino_ir` `{"xml": <installed .xml>}`. Progress in bytes: download + hash (local files: hash only). Cancel during download/hash deletes the state row.
- **DB:** reads/writes `models`.
- **Child processes:** curl.exe (cancel only, no timeout), Python worker (validation).
- **Files:** `models\.downloads\<id>-<version>\` (partial downloads), `models\<category>\<id>\<version>\` (install; previous content removed first; downloads renamed, local files copied). A checksum/size mismatch deletes the downloaded file.
- **Tests:** `shipped_manifests_parse_and_are_not_downloadable_without_verification`, `interrupted_installs_become_failed_and_ready_ones_stay`, `sha256_known_vector`, `download_verify_install_remove_and_checksum_failure` (uses a `file:///` URL through real curl), `offline_install_validates_openvino_ir_with_real_worker` (skips without venv), `blocked_download_is_refused`.

### photos.rs

- **Purpose:** photo editor slice B1 (ADR 0006). Photos are read in place and never written; each has a non-destructive edit stack (JSON, photo.py keys); rendering is done by `workers/ai-worker/photo.py` in a **second worker process** (`Lane`) so previews never wait behind queued video jobs.
- **Types:** `Photo` (camelCase) → `id, path, size, width, height, takenAt` (EXIF DateTimeOriginal, else file modified time UTC), `edits` (object or null), `importedAt`, `thumb` (cache path when present). `ImportResult { ids, failed: [{path, error}] }`. `Lane` (data-root closure, own `JobObject`, `Mutex<Option<Worker>>`, call counter).
- **Public functions:** `list` (newest taken/imported first), `get`, `upsert(path, size, modified)` (`ON CONFLICT(path)`, edits kept), `set_info(width, height, taken)`, `set_edits` (object or null, else error; unknown id → error), `remove` (row + `cache\photo\<id>\`), `cache_dir`, `attach_thumbs`, `import(db, lane, cache, paths)` (extension check → row → worker `photo_render` 480 px thumbnail with `info` → size and date; a file the worker cannot read is removed again and reported), `preview(lane, cache, photo, edits, show_mask, before)` (`before.jpg` rendered once, else `preview-<n>.jpg` with a new name each time, older previews deleted; ≤ 1600 px), `export(lane, exports, photo)` (full size, JPEG q95, `export::free_name` so nothing is replaced, `.jpg.part` then rename). `Analysis { metrics: analysis::Sample, tags: Option<MediaTags> }`; `store_analysis(c, id, result)`; `analyses(c) -> HashMap<id, Analysis>`; `analyze_job(job, progress, cancelled, env, ai)` → job `media.photo_analyze` (worker `photo_analyze` on each photo as edited, with the OpenCLIP folder when `models::ready_dir` finds it; per-photo failures reported). `Photo.analyzed` (EXISTS in `photo_analysis`). `still(job, ai, cancelled, dir, photo, ms, i) -> Result<(Segment, TempFile), Outcome>` → photo rendered with its edit stack by the job-queue worker (`photo_render`, long side `VIDEO_SIDE`) to `dir\still-<job>-<pid>-<i>.jpg` (deleted when the guard drops) as a `STILL` segment; `still_media(photo, file, w, h, ms)` → the stand-in `Media` (id −photo id, `source_created_at` = taken date). `video_job(job, progress, cancelled, env, root, ai)` → job `media.photo_video`: `still` per photo, one segment of `photoMs` each (`source_created_at` = taken date, so `auto` transitions follow it), then `export::finish` to `photos\exports\<name>-<preset>[-N].mp4`. `Lane::call(kind, params)` spawns the worker on first use, runs one job through `worker::run_on`, restarts after a crash.
- **Constants:** `EXTENSIONS` = jpg, jpeg, png, tif, tiff, webp, bmp; `THUMB_SIDE = 480`; `PREVIEW_SIDE = 1600`; `VIDEO_SIDE = 2880` (covers 1920 px and a 9:16 centre crop of a 3:2 photo); `photoMs` clamped to `timeline::PHOTO_MS`.
- **Job:** `media.photo_video`, params `{"photoIds": [i64…] (order), "photoMs": i64 (default 3000), "preset", "options": export::Options, "name"}` → the `media.export` result shape. Progress: 0–200 stills, 200–1000 `export::finish`.
- **DB:** `photos` (migration 10).
- **Child processes:** the photo worker (Python, same script and protocol as the AI worker, own Job Object).
- **Files:** `cache\photo\<id>\thumb.jpg | before.jpg | preview-<n>.jpg`; exports `photos\exports\<name>-edit[-N].jpg` and photo videos `photos\exports\<name>-<preset>[-N].mp4`; stills in `cache\render\` while a photo video renders.
- **Tests:** `rows_edits_and_dates`, `import_preview_export_with_real_worker` (real worker + FFmpeg test image; checks thumbnail, preview names, auto, two exports `-edit` / `-edit-2`, original unchanged; skips without the dev venv or FFmpeg), `photo_video_with_real_worker` (two photos, one edited and one portrait → 2.5 s 1920×1080 video validated, stills removed, photo unchanged), `mixed_timeline_export_and_join_preview` (video + photo with its own `fadeblack` 0.5 s → `timeline_joins`, project export 4.5 s validated, stills removed, join preview file; a second join refused).
- **Notes:** `ponytail:` the photo worker stays up for the session once used (no model in it). SQLite `NOCASE` folds ASCII only, so `Ảnh` / `ảnh` paths are different rows (same as `media`).

### process.rs

- **Purpose:** child-process helpers: every child joins one Job Object with `KILL_ON_JOB_CLOSE`, so it dies with the app (crash or forced kill), and starts without a console window.
- **Key types:** `JobObject(HANDLE)` (`Send + Sync`; handle never closed on purpose — Windows closes it at process exit, killing children).
- **Public functions:** `JobObject::new() -> Result<Self, String>`; `JobObject::assign(&Child)`; `command(program) -> Command` (adds `CREATE_NO_WINDOW`); `spawn(cmd, job) -> Result<Child, String>` (a child that cannot join is killed at once); `wait_output(child, timeout, cancelled) -> Result<Output, String>` (drains stdout/stderr on threads, polls every 50 ms, kills on cancel → `"cancelled"` or on timeout → `"timed out after N s"`).
- **Constants:** `CREATE_NO_WINDOW = 0x0800_0000`.
- **Tests:** `wait_output_returns_output_times_out_and_cancels` (uses `ping.exe`).

### projects.rs

- **Purpose:** project CRUD. A project groups editing decisions; deleting one never touches originals (cascades to timeline, plan, settings).
- **Key types:** `Project { id, name, createdAt, updatedAt }` (RFC 3339 UTC, ms).
- **Public functions:** `list(c)` (most recently updated first); `create(c, name)`; `rename(c, id, name)` (bumps `updated_at`); `delete(c, id)`. Names trimmed, 1–200 characters (counted in chars).
- **DB:** `projects`.
- **Tests:** `create_list_rename_delete`, `rejects_invalid_names`, `missing_project_errors`.

### proxy.rs

- **Purpose:** thumbnails and preview proxies under `<data root>\cache` (preview/analysis only, never rendered from), plus shared FFmpeg plumbing used by export/transcribe.
- **Key types:** `RunError { Cancelled, Failed(String) }` (shared error of FFmpeg-based steps); `Decode { Hybrid, Software }` → `input_args()` (Hybrid: `-hwaccel qsv -hwaccel_output_format qsv -noautorotate`), `upright(rotation)` (Hybrid: `vpp_qsv=format=nv12,hwdownload,format=nv12,` + transpose/flip + `sidedata=mode=delete:type=DISPLAYMATRIX,`).
- **Public functions:**
  - `thumb_path(cache, id)` → `thumbs\<id>.jpg`; `proxy_path(cache, id)` → `proxy\<id>.mp4`; `remove_cache(cache, id)` (thumb, proxy, `frames\<id>\`).
  - `remove_leftovers(root) -> usize` → deletes everything in `cache\render`, `cache\audio`, `cache\downloads\sounds`, and `*.part` / `*.level.mp4` / `*.unvalidated.mp4` in `cache\thumbs`, `cache\proxy`, `projects\*\exports`; never finished files, never model partial downloads.
  - `ensure_proxy(m, env, progress, cancelled) -> Result<(), RunError>`; `attach_cache(cache, items)`.
  - `fit(width, height, rotation, short) -> (i64, i64)` → upright size with short side ≤ `short`, never upscaled, even numbers.
  - `os(&[&str]) -> Vec<OsString>`.
  - `run_ffmpeg(env, args, duration_ms, progress, cancelled) -> Result<String, RunError>` → adds `-hide_banner -nostdin -v error -progress pipe:1 -nostats -y`; progress from `out_time_us`; returns stderr; error shows the last 3 stderr lines.
  - `write_atomically(out, run) -> Result<(), RunError>` → runs into `<out>.part`, renames on success, deletes the part on failure.
  - `make_thumbnail(m, env) -> Result<(), String>` → one MJPEG frame at 10 % of the duration, `-q:v 4`.
  - `proxy_job(job, progress, cancelled, env) -> Outcome`.
- **Proxy attempts (in order):** `qsv` (only rotation 0: full QSV decode + `vpp_qsv` scale + `h264_qsv veryfast global_quality 28`), `hybrid` (QSV decode, CPU filters, `h264_qsv`), `libx264` (`veryfast crf 26`). Fps capped to 30 when source > 30.5; audio AAC 96k stereo or `-an`; `+faststart`.
- **Constants:** `PROXY_SHORT_SIDE = 540`; `THUMB_SHORT_SIDE = 180`; `STEP = 1000` per file.
- **Job:** `media.proxy`, params `{"ids": [i64]}` → result `{"done": [{"id", "encoder", "seconds"}], "failed": [{"id", "error"}]}`; existing proxies kept; missing thumbnails made too.
- **DB:** reads `media`.
- **Child processes:** FFmpeg; cancel checked on each `-progress` line (child killed); no timeout. Thumbnails are not cancellable.
- **Files:** `cache\thumbs\`, `cache\proxy\` (+ `.part`), sweep folders above.
- **Tests:** `fit_keeps_aspect_rotation_and_never_upscales`, `leftovers_are_removed_but_finished_files_kept`.
- **Notes:** a comment records "measured ~5x faster than software on 10-bit HEVC 2.7K" for the full-QSV path (numbers in [HARDWARE_VALIDATION.md](../HARDWARE_VALIDATION.md) §6a).

### request.rs

- **Purpose:** request box parser (owner decision 2026-11-08: offline, keyword based, no language model). Reads a Vietnamese or English sentence into a style, dates and `Wishes`, and reads back what it understood. Unknown words are ignored.
- **Key types:** `Request { style, from, to, wishes, understood }` (camelCase); private `Text` (lower-case text + diacritic-folded copy aligned char by char; matched spans blanked so a word is used once).
- **Public functions:** `parse(text, vocab: &[TagName]) -> Request`.
- **Parse order:** title (`tiêu đề`, `title`, `tên video`…) and ending (`lời kết`, `kết thúc bằng`, `ending`…) free text (quoted or rest of clause) → dates (`dd/mm/yyyy` pair or single, `tháng m/yyyy`, `tháng m năm yyyy`, `mm/yyyy`, `năm yyyy`; months end at `-31`, compared as text) → duration (`30 giây`, `1 phút 30 giây`, `45s`, `2 min`; ≥ 1 s) → music → colour → style → vertical/horizontal (vertical also sets style `social_story` if none) → subtitles on/off → (before the length) `PHOTO_EACH` "mỗi ảnh N giây" → `photoMs` (span blanked so it is not the video length) and `PHOTOS` words → `photos` (`none` / `more` / `fewer` / `yes`; a match followed by "tự sướng" is the selfie concept, not photos) → concepts from the library vocabulary (non-negative tags, `en`/`vi`, folded `vi` only when multi-word or ≥ 5 chars; longest first) → "nature" words expand to the `LANDSCAPES` the library knows. Clause words decide the class: `LESS_WORDS` → less, `AVOID_WORDS` → avoid, else prefer.
- **Constants:** `AVOID_WORDS`, `LESS_WORDS`, `ALIASES` (activity words for tags: đạp xe / cycling / biking → bicycle, đi bộ trong rừng / trekking / hiking → hiking trail), `STYLES` (keywords per style id; `journey_recap` first: tổng hợp, hành trình, cả chuyến đi, recap, journey), `VERTICAL`, `HORIZONTAL`, `COLORS` (bw, warm, cool, vivid, vintage), `MUSIC` (none, low, high, medium, any), `PHOTOS`, `PHOTO_EACH`, `NATURE`, `CLAUSE_END = , . ; ! ? \n`.
- **DB / processes / files:** none (vocabulary passed in by lib.rs).
- **Tests:** `owner_style_request_in_vietnamese`, `without_diacritics_and_dates`, `free_text_is_not_read_as_wishes`, `longer_names_win_and_clauses_scope_negation`, `nothing_known_means_nothing_changed`.

### scoring.rs

- **Purpose:** deterministic technical scoring and duplicate detection, computed on read from stored observations. Components stay separate; nothing is excluded automatically.
- **Key types:** `Technical { sharpness, exposure, brightness, contrast, stability, total }` (0..1, rounded to 3 decimals); `Duplicate { mediaId, sceneIndex, distance }`; `ScoredScene` (scene flattened + `technical`, `flags`, `duplicates`, `tags` filled by callers).
- **Public functions:** `technical(samples, shake) -> (Technical, Vec<&str>)`; `hamming(a, b) -> Option<u32>` (64-bit hex dHash); `representative_hash(scene) -> Option<&str>` (sample nearest the middle); `score(scenes, library) -> Vec<ScoredScene>`.
- **Formulas (all INITIAL ASSUMPTIONS per the module doc, from synthetic tests and the owner's DJI samples):** medians over samples; sharpness `clamp((log10(var) − 1.5) / 2)`; exposure `1 − 4 × (clipLow + clipHigh)`; brightness 1 inside 0.25..0.65, falling to 0 at 0.0 / 0.9; contrast `/ 0.18`; stability `1 − shake / 0.02`; total `0.35 S + 0.2 E + 0.15 B + 0.1 C + 0.2 St`, 0 when > 50 % samples black or no samples.
- **Flags:** `not sampled`, `black`, `blurry` (sharpness < 0.3), `shaky` (stability < 0.4), `dark` (median brightness < 0.2), `overexposed` (clipped > 0.15 and high > low), `duplicate`.
- **Constants:** `DUPLICATE_MAX_DISTANCE = 6`.
- **Tests:** `sharp_steady_beats_blurry_shaky_and_flags_them`, `black_dark_overexposed`, `deterministic`, `duplicates_by_dhash`.
- **Notes:** `ponytail:` O(scenes × library) scan; index hashes (BK-tree) if libraries reach ~100k scenes.

### settings.rs

- **Purpose:** app settings in `%APPDATA%\dev.memora.studio\settings.json` — only the data root; everything else lives under the data root.
- **Key types:** `Settings { dataRoot: Option<PathBuf> }` (None until first-run choice; nothing written before).
- **Public functions:** `load(config_dir) -> Settings` (missing/unreadable → default); `save(config_dir, &Settings) -> Result<(), String>`; `default_data_root() -> PathBuf` (`%LOCALAPPDATA%\MemoraStudio`, temp dir fallback; suggestion only); `prepare_data_root(input, app_dir) -> Result<PathBuf, String>` (absolute path required, not inside the app folder, creates `SUBDIRS`).
- **Constants:** `SUBDIRS = [runtime, models, projects, cache, logs, database]`.
- **Tests:** `missing_settings_are_default`, `settings_round_trip`, `rejects_relative_path`, `rejects_folder_inside_app_dir`, `creates_layout`.
- **Notes:** `save` writes `settings.json` directly (no `.part` + rename).

### sounds.rs

- **Purpose:** free music library: curated catalog (sounds.json) with source, licence, credit, size and SHA-256 recorded at development time. A track is downloaded only for an export that uses it, verified, rendered, then deleted.
- **Key types:** `Track { id, title, artist, moods, energy (low|medium|high), durationMs, url, sha256, sizeBytes, license, licenseUrl, credit }` (moods are developer-assigned assumptions); `Recommendation { id, score, reasons }`; `TempFile(PathBuf)` (deletes the file and its `.part` on drop).
- **Public functions:** `catalog() -> Vec<Track>`; `track(id) -> Result<Track, String>`; `recommend(style, tags, video_ms, speech_share, n) -> Vec<Recommendation>`; `fetch(t, dir, job_object, progress, cancelled) -> Result<TempFile, String>` (unique name `<id>-<pid>-<nanos>.mp3`, `models::download`, size + SHA-256 check).
- **Recommendation formula (deterministic):** `0.5 × style fit (share of track moods in the style's moods) + 0.3 × content (tag weight whose moods match) + 0.1 × length (1.0 if track ≥ video else 0.3) + 0.1 × talk (speech share > 0.3: low energy 1.0, high 0.0; else 0.6)`; ties by id. Style→moods and tag→moods tables in `style_moods` / `content_moods`.
- **Child processes:** curl.exe (via models.rs; cancel only).
- **Files:** `<data root>\music\<id>.mp3`, kept (owner decision 2026-11-09: up to 2 GB); size + SHA-256 checked on every use, a failing file is deleted and downloaded again.
- **Tests:** `catalog_is_complete_and_verified` (32 tracks, official artist URLs only), `recommendations_follow_style_content_length_and_speech`, `temp_file_is_deleted_on_drop`, `kept_track_is_reused_and_a_damaged_one_is_not`, `fetch_real_track` (`#[ignore]`, needs internet).

### tagging.rs

- **Purpose:** stores OpenCLIP tag observations from the worker's `tag_video` (image encoder as OpenVINO IR, GPU → CPU fallback, on the proxy every `EVERY_MS`) and summarises them per time range.
- **Key types:** `TagName { en, vi, negative }` (negative = bad-shot detector); `TagSample { atMs, tags: [(index, probability)], cues: [probability] }`; `MediaTags { device, tags, cues: [TagName], samples }` (`cues` empty for tags made before model v3); `TagScore { en, vi, negative, score }`.
- **Public functions:** `cue(t, en, start_ms, end_ms) -> Option<f64>` (mean cue probability in the range; None without that cue); `summarize(t, start_ms, end_ms, min) -> Vec<TagScore>` (mean probability over samples in `[start, end)`, missing = 0, best first); `store(c, media_id, result) -> Result<usize, String>` (upsert, returns sample count); `get(c, media_id)`; `all(c) -> HashMap<i64, MediaTags>`; `clear(c, media_id)`; `tag_job(job, progress, cancelled, env, ai) -> Outcome`.
- **Constants:** `MODEL_ID = "openclip-vit-b32"`; `EVERY_MS = 2000`; `STEP = 1000`, `PROXY_PART = 300.0`.
- **Job:** `media.tag`, params `{"ids": [i64]}` → result `{"done": [{"id", "samples", "device"}], "failed": [{"id", "error"}]}`. Requires the model `ready` (`models::ready_dir`); per file `ensure_proxy`, then worker sub-job `tag_video` `{"path": <proxy>, "modelDir", "everyMs"}`.
- **DB:** writes `media_tags`; reads `models`, `media`.
- **Child processes:** FFmpeg (proxy), Python worker.
- **Tests:** `summarize_ranges`, `store_get_cascade`, `job_needs_installed_model`, `tag_job_with_real_worker_and_ir` (skips unless the export exists at the hard-coded `E:\Memora Data\downloads\openclip-export-v3`; also checks the \"view ahead\" cue).

### timeline.rs

- **Purpose:** project timeline: ordered trimmed segments of library videos and still photos (migration 11) with optional captions and an optional transition into each clip. Every change bumps the project's `updated_at`.
- **Key types:** `Clip { id, projectId, mediaId?, photoId?, position, inMs, outMs, caption, transition?, transitionMs? }`; `Clip::join() -> Option<export::Join>`.
- **Constants:** `PHOTO_MS = (1 s, 20 s)` (photo length range), `PHOTO_DEFAULT_MS = 4 s`.
- **Public functions:** `list(c, project_id)` (by position, id); `add(c, project_id, media_id, in_ms, out_ms) -> Result<Clip, String>` (appends; `out` clamped to media length; ≥ 100 ms; checks media and project exist); `add_photo(c, project_id, photo_id, ms)` (appends a photo, length clamped to `PHOTO_MS`); `set_transition(c, clip_id, Option<&str>, ms)` (name in `export::TRANSITIONS`, 0–3000 ms; None clears); `set_range(c, clip_id, in_ms, out_ms)` (same rules; photos clamp to `PHOTO_MS.1`); `set_caption(c, clip_id, caption)` (trimmed, ≤ 200 chars, empty clears); `remove(c, clip_id)`; `move_clip(c: &mut Connection, clip_id, delta)` (swap with neighbour in a transaction; no-op at the ends).
- **DB:** `project_clips`, `projects.updated_at`, reads `media.duration_ms`, `photos`.
- **Tests:** `add_validates_and_clamps`, `set_range_validates_and_clamps`, `photos_and_per_join_transitions`, `move_remove_and_cascade`.
- **Notes:** `editing::apply` replaces the whole timeline (photo clips included) with the plan's video clips.

### transcribe.rs

- **Purpose:** speech transcription with whisper.cpp `whisper-cli.exe` as a separate native process. Audio extracted from the ORIGINAL to a temporary 16 kHz mono WAV, transcribed with an installed Whisper model (+ Silero VAD when installed), stored as timed segments (original-file times).
- **Key types:** `Segment { startMs, endMs, text }`; `Transcript { mediaId, modelId, language, segments, createdAt }`.
- **Public functions:** `Segment::timing_plausible()` → line lasts ≤ 3 s + 1 s per word (**assumption**; Whisper stretches short lines over noise; editing.rs ignores implausible lines for cuts and the speech score); `whisper_cli(data_root) -> PathBuf` (`runtime\whisper.cpp\b5454\whisper-cli.exe`); `parse_output(&Value) -> Result<(Option<String>, Vec<Segment>), String>` (drops empty text, applies `drop_loops`); `store(c, media_id, model_id, language, segments)`; `get(c, media_id)`; `clear(c, media_id)`; `transcribed(c) -> HashSet<i64>`; `transcribe_job(job, progress, cancelled, env, data_root) -> Outcome`.
- **whisper-cli arguments:** `[--vad -vm <silero>] -m <model> -f <wav> -of <base> -oj -np -pp -mc 0 -sns -l <lang> -t <threads>`; threads = half the logical CPUs clamped 1..8; progress parsed from `progress =` lines on stderr; output JSON decoded lossily (UTF-8 cut at token boundaries).
- **Constants:** `WHISPER_DIR = "b5454"`; `VAD_ID = "silero-vad"`; `LOOP_RUN = 4` (runs of ≥ 4 identical consecutive segments removed as hallucination loops); `AUDIO_PART = 0.15` (share of a file's progress for audio extraction); `STEP = 1000`.
- **Job:** `media.transcribe`, params `{"ids": [i64], "model": "whisper-base"|"whisper-small" (default base), "language": "auto"|"vi"|"en" (default auto)}` → result `{"done": [{"id", "language", "segments"}], "failed": [{"id", "error"}]}`. Files without audio store an empty transcript with `language = null`. Fails up front when the runtime or model is missing.
- **DB:** writes `transcripts`; reads `models`, `media`.
- **Child processes:** FFmpeg (audio extract, cancel via `run_ffmpeg`), whisper-cli (cancel polled every 200 ms, killed; no timeout).
- **Files:** `cache\audio\<id>.wav` and `<id>.json`, deleted after each file (swept by `remove_leftovers` after a crash); reads `runtime\whisper.cpp\b5454\`, `models\speech\…`.
- **Tests:** `parse_store_replace_and_cascade`, `hallucination_loops_are_dropped`, `missing_model_is_reported`, `transcribe_job_with_real_whisper` (skips unless runtime/model exist under the hard-coded `E:\Memora Data`).
- **Notes:** `ponytail:` one fixed version dir; resolve by manifest once a second version ships. `ponytail:` text heuristic; "VAD (silero) before decoding is the real fix, pending model approval" (VAD is already used when installed). `ponytail:` half the logical CPUs, max 8; measure before tuning.

### worker.rs

- **Purpose:** the Python AI worker process (`workers/ai-worker/memora_worker.py`, protocol v1, ADR 0002): started on demand by the job runner, restarted on the next job after a crash. Details of the Python side in [WORKER.md](WORKER.md).
- **Key types:** `WorkerPaths { python, script }`; `Recv { Msg(Value), Timeout, Exited }`; `Worker { child, stdin, rx, hello }`.
- **Public functions:** `resolve(data_root) -> Result<WorkerPaths, String>` (`<root>\runtime\ai-worker\python\python.exe` + `app\memora_worker.py`, else repo `workers/ai-worker/.venv\Scripts\python.exe` + `memora_worker.py`); `Worker::spawn(paths, job_object)` (`python -u <script>`, waits for `{"type":"hello","protocol":1}`); `Worker::send(&Value)`; `Worker::recv(timeout) -> Recv`; `runner(data_root, job_object, info) -> Runner`.
- **Protocol (JSON lines):** app → worker `{"type":"job.start","id","kind","params"}`, `{"type":"job.cancel","id"}`; worker → app `hello`, `job.progress {done,total}`, `job.result {result}`, `job.cancelled`, `job.error {error}`; messages for other ids ignored; non-JSON stdout logged and ignored; stderr lines logged at warn with target `ai_worker`.
- **Constants:** `PROTOCOL = 1`; `HELLO_TIMEOUT = 60 s`; `CANCEL_GRACE = 10 s` (**assumed**, "not measured on the slowest model compiles": after a cancel the worker is killed if it has not stopped).
- **Job kinds handled here:** anything not `media.*` / `models.*` from `job_runner`: `validate_devices` (UI) and `diagnostic.*` (tests); plus sub-jobs sent by Rust modules: `analyze_video`, `tag_video`, `validate_openvino_ir`.
- **Child processes:** Python worker in the Job Object; stdin/stdout/stderr piped; stdout/stderr read on threads (the Rust side never blocks on the pipe); polled every 100 ms during a job; no overall job timeout.
- **Tests:** `worker_lifecycle_end_to_end` (progress, cancel, stuck-job kill after grace, unknown kind, crash + restart, `validate_devices` CPU ok; skips without the venv).
- **Notes:** `ponytail:` dev fallback to the repo venv; remove once the installer provisions `runtime\ai-worker`. `worker_info` is cleared when the worker exits.

---

## 4. Tauri commands

45 commands registered in `generate_handler!` ([lib.rs](../../apps/desktop/src-tauri/src/lib.rs)). Rust snake_case arguments are camelCase on the JS side (`media_id` → `mediaId`). All return `Result<_, String>` unless noted.

| # | Command | Args | Returns | Work done by |
|---|---|---|---|---|
| 1 | `get_settings` | — | `SettingsView` (infallible) | lib.rs `view` |
| 2 | `set_data_root` | `path: String` | `SettingsView` | settings `prepare_data_root`/`save`, db `open`, lib `reconcile`, asset scope; refused while jobs run |
| 3 | `backup_database` | — | `PathBuf` | db `backup` (label `manual`) |
| 4 | `get_hardware_info` | — (async) | `HardwareInfo` | hardware `detect` (spawn_blocking) |
| 5 | `list_projects` | — | `Vec<Project>` | projects `list` |
| 6 | `create_project` | `name` | `Project` | projects `create` |
| 7 | `rename_project` | `id: i64, name` | `Project` | projects `rename` |
| 8 | `delete_project` | `id: i64` | `()` | projects `delete` |
| 9 | `list_media` | — | `Vec<Media>` | media `list` + analysis `scene_counts` + transcribe `transcribed` + tagging `all`/`summarize` (min 0.05) + editing `marks` + proxy `attach_cache` |
| 10 | `remove_media` | `id: i64` | `()` | media `remove` + proxy `remove_cache` |
| 11 | `list_scenes` | `media_id: i64` | `Vec<ScoredScene>` | analysis `scenes`/`library_hashes` + scoring `score` + tagging `summarize` (min 0.1) |
| 12 | `get_transcript` | `media_id: i64` | `Option<Transcript>` | transcribe `get` |
| 13 | `set_media_mark` | `media_id: i64, mark: Mark` | `()` | editing `set_mark` |
| 14 | `list_styles` | — | `Vec<Style>` (infallible) | editing `styles` |
| 15 | `generate_plan` | `project_id, style, from?, to?, request?` | `Plan` | lib `make_plan` → request `parse`, editing `gather`/`plan_with`/`save` |
| 16 | `interpret_request` | `text` | `Request` | request `parse` + lib `vocabulary` |
| 17 | `get_plan` | `project_id` | `Option<Plan>` | editing `load` |
| 18 | `apply_plan` | `project_id, keep: Vec<usize>` | `export::Options` | lib `apply_plan_to` → editing `apply`/`styled_options`, `recommend_for`, export `save_options` |
| 19 | `list_models` | — | `Vec<ModelView>` | models `list` |
| 20 | `remove_model` | `id: String` | `()` | models `remove` |
| 21 | `list_clips` | `project_id` | `Vec<Clip>` | timeline `list` |
| 22 | `add_clip` | `project_id, media_id, in_ms, out_ms` | `Clip` | timeline `add` |
| 23 | `remove_clip` | `id: i64` | `()` | timeline `remove` |
| 24 | `list_sounds` | — | `Vec<Track>` (infallible) | sounds `catalog` |
| 25 | `recommend_sounds` | `project_id?`, `video_ms?` | `Vec<Recommendation>` (top 3) | lib `recommend_for` → sounds `recommend`; no project (photo video) → `recommend("memory_nostalgia", [], video_ms, 0)` |
| 26 | `set_clip_caption` | `id: i64, caption` | `()` | timeline `set_caption` |
| 27 | `get_export_options` | `project_id` | `export::Options` | export `load_options` |
| 28 | `set_export_options` | `project_id, options: Options` | `()` | export `save_options` (checked) |
| 29 | `move_clip` | `id: i64, delta: i64` | `()` | timeline `move_clip` |
| 30 | `submit_job` | `kind, params?: Value` | `Job` | jobs `submit`; kind must be in `UI_JOB_KINDS` |
| 31 | `list_jobs` | — | `Vec<Job>` (infallible) | jobs `list` |
| 32 | `cancel_job` | `id: u64` | `()` | jobs `cancel` |
| 33 | `get_worker_info` | — | `Option<Value>` (infallible) | worker `hello` from `AppState.worker_info` |
| 34 | `list_photos` | — | `Vec<Photo>` | photos `list` + `attach_thumbs` |
| 35 | `import_photos` | `paths: Vec<String>` (async) | `ImportResult` | photos `import` (spawn_blocking, photo worker) |
| 36 | `set_photo_edits` | `id, edits: Value` | `()` | photos `set_edits` |
| 37 | `preview_photo` | `id, edits, show_mask, before` (async) | `PathBuf` (cache file) | photos `preview` |
| 38 | `auto_photo` | `id` (async) | `Value` (suggested edits) | photo worker `photo_auto` |
| 39 | `export_photo` | `id` (async) | `PathBuf` | photos `export` |
| 40 | `remove_photo` | `id` | `()` | photos `remove` |
| 41 | `add_photo_clips` | `project_id, photo_ids: Vec<i64>, ms?` | `Vec<Clip>` | timeline `add_photo` per photo (default `PHOTO_DEFAULT_MS`) |
| 42 | `set_clip_range` | `id, in_ms, out_ms` | `Clip` | timeline `set_range` (photo length: 0, ms) |
| 43 | `set_clip_transition` | `id, transition: Option<String>, transition_ms` | `Clip` | timeline `set_transition` |
| 44 | `list_joins` | `project_id` | `Vec<Join>` | export `timeline_joins` |
| 45 | `preview_join` | `project_id, index, preset` (async) | `PathBuf` (cache file) | export `preview_join` (spawn_blocking; own JobObject; photos via the photo lane at 1280 px) |

---

## 5. Job kinds

All run on the single `job-executor` thread through `job_runner` (lib.rs). UI-submittable kinds = `UI_JOB_KINDS` (10); the CLI runs `media.export` and `media.import|proxy|analyze|tag|transcribe`.

| Kind | Runner | Params | Result | Child processes |
|---|---|---|---|---|
| `validate_devices` | worker.rs `runner` (Python) | none | worker result (`devices`: per OpenVINO device `{device, ok, …}`) | Python worker |
| `media.import` | media.rs `import_job` | `{"paths": [string]}` | `{"imported", "ids", "failed": [{path, error}]}` | FFprobe, FFmpeg (thumbnail) |
| `media.proxy` | proxy.rs `proxy_job` | `{"ids": [i64]}` | `{"done": [{id, encoder, seconds}], "failed": [{id, error}]}` | FFmpeg |
| `media.analyze` | analysis.rs `analyze_job` | `{"ids": [i64], "quality"?: "fast"|"balanced"|"detailed"}` | `{"done": [{id, scenes}], "failed"}` | FFmpeg (missing proxy), Python worker (`analyze_video`) |
| `media.tag` | tagging.rs `tag_job` | `{"ids": [i64]}` | `{"done": [{id, samples, device}], "failed"}` | FFmpeg (missing proxy), Python worker (`tag_video`) |
| `media.transcribe` | transcribe.rs `transcribe_job` | `{"ids": [i64], "model"?: "whisper-base"|"whisper-small", "language"?: "auto"|"vi"|"en"}` | `{"done": [{id, language, segments}], "failed"}` | FFmpeg (audio), whisper-cli |
| `media.export` | export.rs `export_job` | `{"projectId", "preset", "options"?: Options}` (legacy `fit`, `transitionMs`) | `{"path", "preset", "encoder", "renderSeconds", "durationMs", "joins", "options", "music", "loudness", "width", "height", "validation"}` | curl.exe (library music), FFmpeg (render, loudness, validation), FFprobe |
| `media.photo_video` | photos.rs `video_job` | `{"photoIds", "photoMs", "preset", "options", "name"}` | same as `media.export` | AI worker (`photo_render`), curl.exe (library music), FFmpeg, FFprobe |
| `media.photo_analyze` | photos.rs `analyze_job` | `{"ids": [i64…]}` | `{"done": [ids], "failed": [{id, error}], "tagged": bool}` | AI worker (`photo_analyze`) |
| `models.install` | models.rs `install_job` | `{"id", "files"?: {artifact: path}}` | `{"id", "path", "details"}` | curl.exe (download), Python worker (`validate_openvino_ir`) |
| `diagnostic.*` (test-only) | worker.rs `runner` | e.g. `{"steps"}`, `{"seconds"}` | worker result | Python worker |

Worker sub-jobs (not queue jobs; sent by Rust modules through the same worker): `analyze_video`, `tag_video`, `validate_openvino_ir`.

---

## 6. Error handling and conventions

- **`Result<_, String>` everywhere.** Errors are user-readable English sentences returned straight to the UI (Tauri serialises `Err(String)`). No custom error enums at command level.
- **`db::err`** maps every `rusqlite::Error` to `"Database error: …"`; used with `.map_err(err)` in all DB modules.
- **Database handle** = `Arc<Mutex<Result<Connection, String>>>`: `lib::with_db` / `MediaEnv::with_db` lock it and turn an unavailable database into its reason (e.g. `NO_DATA_ROOT`). Jobs lock it only for short DB steps, never during child processes.
- **Step errors:** `proxy::RunError { Cancelled, Failed(String) }` for FFmpeg / whisper steps (proxy, export, transcribe, analysis, tagging); `models::Step { Cancelled, Failed(state, msg) }` for download/verify; `jobs::Outcome { Completed, Cancelled, Failed }` for job results.
- **Batch jobs never fail on one file:** per-file errors go into `failed: [{id|path, error}]`; only missing params, DB unavailability or cancellation end the job.
- **Progress units:** batch jobs use **1000 units per file** (`STEP`), sub-steps scaled inside (`PROXY_PART = 300` for a missing proxy in analyze/tag; `AUDIO_PART = 0.15` in transcribe); `media.import` uses 1 unit per file; `media.export` uses 1000 total; `models.install` uses bytes.
- **Atomic writes:** `proxy::write_atomically` (`<out>.part` → rename, part deleted on failure) for thumbnails, proxies and export staging; curl downloads go to `.part` (kept for resume); exports are staged as `.unvalidated.mp4`, levelled via `.level.mp4`, then renamed — an earlier export is never overwritten (`free_name`); a failed validation keeps the file as `-failed-validation.mp4`. `proxy::remove_leftovers` cleans temporaries at startup / data-folder switch (only when no job runs). Exceptions: `settings.json` is written directly; model install removes the old folder before copying (state row records failures).
- **Job Object rule:** every child process is started with `process::command` (no console window) and `process::spawn` (joins the Job Object with `KILL_ON_JOB_CLOSE`; a child that cannot join is killed). Never use `Command::spawn` directly in app code (tests do, for fixtures).
- **Cancellation / timeouts:** FFmpeg via `run_ffmpeg` (killed on cancel, no timeout); FFprobe `wait_output` 60 s; LUFS `wait_output` 600 s; whisper-cli polled 200 ms; curl polled 250 ms; worker cancel message then kill after `CANCEL_GRACE` 10 s.
- **Originals:** never written; renders and audio extraction read originals; analysis/tagging read proxies.
- **Logging:** `tracing` macros with structured fields (`media_id`, `job_id`, `kind`, `project_id`, `model`…); default target = module path; worker stderr uses target `ai_worker`; level via `MEMORA_LOG`; files `<root>\logs\memora.log.YYYY-MM-DD` (JSON), app only.
- **Serde:** IPC structs use `#[serde(rename_all = "camelCase")]`; stored JSON (`plan_json`, `export_json`, `tags_json`, `samples_json`, `segments_json`) is the same serde shape; options/wishes use `#[serde(default)]` so older rows still load.
- **Panics:** mutex `lock().unwrap()` and manifest/catalog `expect` (compile-time JSON); release builds abort on panic.

---

## 7. How to add a module / command / job

**New module**
1. Create `src/<name>.rs` with a `//!` doc comment (purpose, spec section) and `#[cfg(test)] mod tests` (one test per processing path; real-tool tests must skip when the tool is missing).
2. Add `mod <name>;` to [lib.rs](../../apps/desktop/src-tauri/src/lib.rs) (alphabetical); `pub` only if the CLI binary needs it outside the crate.
3. New table/column → append a migration to `MIGRATIONS` in [db.rs](../../apps/desktop/src-tauri/src/db.rs) (never edit a shipped one).
4. Child processes → `process::command` + `process::spawn` with the shared `JobObject`; long output via `run_ffmpeg` / `wait_output` with cancel (and a timeout when the tool can hang).

**New Tauri command**
1. `#[tauri::command] fn <name>(…, state: State<'_, AppState>) -> Result<T, String>` in lib.rs; DB access via `with_db`; keep logic in the module.
2. Add it to `tauri::generate_handler![…]` in `run()`.
3. Add a typed wrapper in [api.ts](../../apps/desktop/src/api.ts) (`invoke<T>("<name>", { camelCaseArgs })`).
4. New plugin permission → [capabilities/default.json](../../apps/desktop/src-tauri/capabilities/default.json).

**New job kind**
1. Runner `pub fn <x>_job(job, progress, cancelled, env, …) -> Outcome` in the module; validate params first; 1000 progress units per file; per-file errors into `failed`; return `Outcome::Cancelled` promptly.
2. Add the kind to the `match job.kind` in `job_runner` (lib.rs) — or nothing, if the AI worker handles it (non-`media.`/`models.` kinds go to Python).
3. Add it to `UI_JOB_KINDS` if the UI may start it; to the `job` allow-list in [cli.rs](../../apps/desktop/src-tauri/src/cli.rs) if the CLI should.
4. Worker-side job → implement in `workers/ai-worker/` and document in [WORKER.md](WORKER.md).
5. UI: call `submitJob("<kind>", params)` and handle the `job-updated` event.

**Docs to update** (then run the `sync-docs` checker): this file (module section, §4 commands table + count, §5 jobs table), [DATABASE.md](DATABASE.md) (migrations/tables), [FRONTEND.md](FRONTEND.md) (api.ts wrapper), [FLOWS.md](FLOWS.md), [FOLDERS.md](FOLDERS.md) (new data-root paths), [ARCHITECTURE.md](ARCHITECTURE.md) (new process/dependency), `CLAUDE.md` status, [PROGRESS.md](../PROGRESS.md).
