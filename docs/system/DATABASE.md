# Database

SQLite schema, migrations, backups, and which Rust code reads and writes each table and JSON column.

Last verified: 2026-11-08 against commit `b751673`. Index: [README.md](README.md). Kept current by the `sync-docs` skill ([.claude/skills/sync-docs](../../.claude/skills/sync-docs/SKILL.md)).

Sources: [db.rs](../../apps/desktop/src-tauri/src/db.rs), plus every module that runs SQL:
[projects.rs](../../apps/desktop/src-tauri/src/projects.rs), [media.rs](../../apps/desktop/src-tauri/src/media.rs),
[timeline.rs](../../apps/desktop/src-tauri/src/timeline.rs), [analysis.rs](../../apps/desktop/src-tauri/src/analysis.rs),
[models.rs](../../apps/desktop/src-tauri/src/models.rs), [transcribe.rs](../../apps/desktop/src-tauri/src/transcribe.rs),
[tagging.rs](../../apps/desktop/src-tauri/src/tagging.rs), [editing.rs](../../apps/desktop/src-tauri/src/editing.rs),
[export.rs](../../apps/desktop/src-tauri/src/export.rs). Callers: [lib.rs](../../apps/desktop/src-tauri/src/lib.rs) (Tauri commands, job runner),
[cli.rs](../../apps/desktop/src-tauri/src/cli.rs) (command-line tool). No SQL in `request.rs`, `sounds.rs`, `scoring.rs`, `proxy.rs`, `worker.rs`.

## 1. Location, connection, migrations, backups

### Location and connection

| Item | Value | Where |
|---|---|---|
| File | `<data root>\database\memora.db` (folder created on open) | `lib::open_db`, `cli::open`, `db::open` |
| Data root | App setting (`%APPDATA%\dev.memora.studio\settings.json`); the CLI also honours `MEMORA_DATA_ROOT` | `settings.rs`, `cli::open` |
| Engine | rusqlite, bundled SQLite | `Cargo.toml` |
| Journal | `PRAGMA journal_mode = WAL` (files `memora.db-wal`, `memora.db-shm`) | `db::open` |
| Foreign keys | `PRAGMA foreign_keys = ON` per connection (needed for every `ON DELETE CASCADE` below) | `db::init` |
| Busy timeout | 5 s | `db::init` |
| Sharing | One `Connection` behind `Mutex<Result<Connection, String>>` (`AppState.db`, `MediaEnv.db`); jobs and commands lock it per call | `lib::with_db`, `MediaEnv::with_db` |
| Switching data folder | Refused while a job runs; new DB is opened (and migrated/backed up) first, then `lib::reconcile` runs | `lib::set_data_root` |
| Error text | `db::err` → `"Database error: …"` | `db::err` |
| Tests | `db::open_in_memory()` (test-only) = in-memory + `init` (foreign keys, migrations) | `db.rs` |

### Migration mechanism

- `MIGRATIONS: &[&str]` in `db.rs` — **append-only** list; comment: "Never edit a migration that has shipped; add a new one instead."
- `PRAGMA user_version` = number of migrations applied.
- `db::migrate`: if `user_version > MIGRATIONS.len()` → error *"This database was created by a newer version of Memora Studio (schema N, supported M). Please update the app."* (the database is not touched).
- Each pending migration runs in its own transaction: `execute_batch(sql)` + `user_version = i + 1` + commit.
- Current count: **9 migrations** (schema version 9).
- Tests in `db.rs`: `fresh_database_gets_all_migrations`, `migrate_is_idempotent`, `refuses_newer_schema`, `opens_file_with_wal`, `existing_database_is_backed_up_on_open_and_old_backups_pruned`.

### Backups (`db::backup`, `db::open`)

| Rule | Value |
|---|---|
| Folder | `<data root>\database\backups\` (`db::backups_dir` = DB folder + `backups`) |
| Method | `VACUUM INTO <path>` on the open connection (includes committed WAL content) |
| Verification | Copy opened read-only, `PRAGMA integrity_check` must return `ok`; otherwise the copy is deleted and an error returned |
| Name | `memora-<UTC yyyymmdd-HHMMSSfff>-v<schema>-<label>.db` (dot removed from the seconds); `-2`, `-3` … suffix if the name exists |
| Labels | `startup`, `before-upgrade`, `manual` |
| Retention | `KEEP_BACKUPS = 10` newest `memora-*.db` files (name order = time order); older ones deleted after each backup |
| Start-up copy | When the DB already has a schema (`user_version > 0`) and no file in `backups\` is younger than `STARTUP_BACKUP_INTERVAL` = 10 min. Failure only logs a warning. The interval exists because the CLI opens the DB per command. |
| Before upgrade | When migrations are pending on an existing DB, a `before-upgrade` copy is always made; **if it fails, `open` returns an error and no migration runs** |
| New database | No backup (`user_version = 0`) |
| Manual | Tauri command `backup_database` → `db::backup(..., "manual")`, returns the path; logged |

### Restore (from the `db::open` doc comment)

1. Close the app (and any `memora-cli`).
2. Copy the chosen backup from `<data root>\database\backups\` over `memora.db`.
3. Delete `memora.db-wal` and `memora.db-shm`.
4. Start the app. A backup with an older schema is migrated forward on open (with its own `before-upgrade` copy); a backup with a newer schema than the app is refused.

## 2. Migrations

| # | Adds | Notes |
|---|---|---|
| 1 | Table `projects` (`id`, `name` CHECK 1–200 chars after trim, `created_at`, `updated_at`) | Timestamps default to `strftime('%Y-%m-%dT%H:%M:%fZ','now')` (UTC, ms) |
| 2 | Table `media` (path `COLLATE NOCASE UNIQUE`, size, mtime, probe fields, `probe_json`, `imported_at`) | Windows paths compare case-insensitively |
| 3 | Table `project_clips` (FK project, FK media, `position`, `in_ms`, `out_ms`); index `project_clips_project (project_id, position)` | Both FKs `ON DELETE CASCADE` |
| 4 | Tables `media_analysis` (PK media_id) and `scenes` (UNIQUE `(media_id, idx)`, `samples_json`) | Scores are computed on read, not stored |
| 5 | Table `models` (PK text `id`) | No row = not installed; manifests live in `models.json` |
| 6 | Table `transcripts` (PK media_id, `segments_json`) | whisper.cpp output |
| 7 | Table `media_tags` (PK media_id, `tags_json`) | OpenCLIP observations |
| 8 | Tables `media_marks` (PK media_id, `favorite`, `excluded`) and `editing_plans` (PK project_id, `style`, `plan_json`) | Editing Core |
| 9 | Column `project_clips.caption TEXT`; table `project_settings` (PK project_id, `export_json`) | Per-clip captions, per-project export options |

## 3. Tables

Notation: **W** = writes, **R** = reads. `lib::x` = Tauri command or helper in `lib.rs`; `cli "x"` = command-line command.
All timestamps are TEXT, RFC 3339 UTC with milliseconds, default `now`.

### projects (migration 1)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| id | INTEGER | PK | |
| name | TEXT | NOT NULL, CHECK `length(trim(name)) BETWEEN 1 AND 200` | Also checked in Rust (`projects::clean_name`: trimmed, 1–200 chars) |
| created_at | TEXT | NOT NULL, default now | |
| updated_at | TEXT | NOT NULL, default now | Sort key of the project list |

- Referenced by `project_clips`, `editing_plans`, `project_settings` (all `ON DELETE CASCADE`).
- **W**: `projects::create` (`lib::create_project`, `cli "new-project"`), `projects::rename` (sets `updated_at`; `lib::rename_project`), `projects::delete` (`lib::delete_project`), `timeline::touch` (sets `updated_at`; called by `timeline::add`, `set_range`, `remove`, `move_clip` — not by `set_caption`).
- **R**: `projects::list` (`lib::list_projects`, `cli "projects"`, `cli::status`), `timeline::add` (existence check), `export::export_job` (name for the output file).

### media (migration 2)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| id | INTEGER | PK | Also the cache file name (`cache\thumbs\<id>.jpg`, `cache\proxy\<id>.mp4`, `cache\frames\<id>\`) |
| path | TEXT | NOT NULL, COLLATE NOCASE, UNIQUE | Absolute path of the original (never modified) |
| size | INTEGER | NOT NULL | File size in bytes at import |
| modified_unix | INTEGER | | File mtime (Unix seconds) at import |
| duration_ms | INTEGER | NOT NULL | |
| width, height | INTEGER | NOT NULL | Coded size |
| rotation | INTEGER | NOT NULL, default 0 | Display-matrix rotation |
| fps | REAL | | |
| video_codec | TEXT | NOT NULL | |
| audio_codec | TEXT | | NULL when no audio |
| audio_streams | INTEGER | NOT NULL, default 0 | |
| bitrate | INTEGER | | |
| source_created_at | TEXT | | Recording time from metadata; sort key, date-range filter (`editing::day`) |
| probe_json | TEXT | NOT NULL | Raw FFprobe JSON (`-show_format -show_streams`) |
| imported_at | TEXT | NOT NULL, default now | Not updated on re-import |

- Referenced by `project_clips`, `media_analysis`, `scenes`, `transcripts`, `media_tags`, `media_marks` (all `ON DELETE CASCADE`).
- **W**: `media::upsert` — `INSERT … ON CONFLICT (path) DO UPDATE` of every column except `id`, `path`, `imported_at` (called by `media::import_one` ← job `media.import`); `media::remove` (`lib::remove_media`).
- **R**: `media::list` (`lib::list_media`, `editing::gather`, `cli::status`, `cli::list_media`, `cli::names`), `media::get` (proxy/analyze/tag/transcribe/export jobs, `cli::scenes`), `media::import_one` (previous `size`, `modified_unix`), `timeline::add` / `timeline::set_range` (`duration_ms` to clamp Out).
- `probe_json` and `modified_unix` are not part of `media::COLUMNS`; `probe_json` is written but no code reads it back.

### project_clips (migrations 3, 9)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| id | INTEGER | PK | |
| project_id | INTEGER | NOT NULL, FK projects ON DELETE CASCADE | |
| media_id | INTEGER | NOT NULL, FK media ON DELETE CASCADE | |
| position | INTEGER | NOT NULL | Order; new clip = `MAX(position)+1` (0 first) |
| in_ms | INTEGER | NOT NULL, CHECK `in_ms >= 0` | Times in the ORIGINAL file |
| out_ms | INTEGER | NOT NULL, CHECK `out_ms > in_ms` | Rust also clamps to `media.duration_ms` and requires ≥ 100 ms |
| caption | TEXT | (migration 9) | Lower-third text, ≤ 200 chars, NULL when empty |

- Index `project_clips_project (project_id, position)`.
- **W**: `timeline::add` (`lib::add_clip`, `editing::apply`, `cli "clip-add"`), `timeline::set_range` (`cli "clip-trim"`), `timeline::set_caption` (`lib::set_clip_caption`, `cli "clip-caption"`), `timeline::remove` (`lib::remove_clip`, `cli "clip-remove"`), `timeline::move_clip` (swaps `position` with the neighbour in a transaction; `lib::move_clip`, `cli "clip-move"`), `editing::apply` (deletes the project's clips, then re-adds the kept plan clips in one transaction; `lib::apply_plan_to` ← `lib::apply_plan`, `cli "apply"`).
- **R**: `timeline::list` (`lib::list_clips`, `lib::recommend_for`, `export::export_job`, `cli` projects/timeline/export).

### media_analysis (migration 4)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| media_id | INTEGER | PK, FK media ON DELETE CASCADE | Row exists = file analysed |
| version | INTEGER | NOT NULL | `analysis.py` `VERSION` (currently 1) |
| quality | TEXT | NOT NULL | `fast` / `balanced` / `detailed` |
| analyzed_at | TEXT | NOT NULL, default now | |

- **W**: `analysis::store` (deletes old rows via `analysis::clear`, inserts, in one transaction; ← `analysis::analyze_job`, job `media.analyze`), `analysis::clear` (also ← `media::import_one` when the file changed).
- **R**: `analysis::scene_counts` (`lib::list_media`, `cli::status`, `cli` media list). `version` and `quality` are not read by any code.

### scenes (migration 4)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| id | INTEGER | PK | |
| media_id | INTEGER | NOT NULL, FK media ON DELETE CASCADE | |
| idx | INTEGER | NOT NULL, UNIQUE with media_id | Scene index from the worker |
| start_ms, end_ms | INTEGER | NOT NULL, CHECK `end_ms > start_ms` | Source times (proxy times = original times); zero-length scenes are skipped by `store` |
| shake | REAL | NOT NULL | Jitter score from `analysis.py` |
| frame_path | TEXT | | Representative JPEG `cache\frames\<media id>\<idx>.jpg`; NULL when the scene had no sample |
| samples_json | TEXT | NOT NULL | JSON array, see below |

`samples_json` = `Vec<analysis::Sample>` (serde camelCase):

```json
[{"atMs": 1000, "sharpness": 123.4, "brightness": 0.4521, "contrast": 0.2013,
  "clipLow": 0.0012, "clipHigh": 0.0, "black": false, "dhash": "0f1e2d3c4b5a6978"}]
```

- **W**: `analysis::store`, `analysis::clear` (as above).
- **R**: `analysis::scenes` (`lib::list_scenes`, `editing::gather`, `cli` scenes), `analysis::library_hashes` (representative dHash per scene for duplicate detection; `lib::list_scenes`, `editing::gather`, `cli` scenes), `analysis::scene_counts`. Unparseable `samples_json` reads as an empty list.

### models (migration 5)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| id | TEXT | PK | Manifest id from `models.json` (e.g. `openclip-vit-b32`, `whisper-base`, `silero-vad`) |
| version | TEXT | NOT NULL | Installed manifest version; `ready` + different manifest version is shown as `update_available` |
| state | TEXT | NOT NULL | `downloading`, `verifying`, `installing`, `validating`, `ready`, `download_failed`, `checksum_failed`, `install_failed`, `model_load_failed`, `incompatible` (`not_installed` = no row) |
| error | TEXT | | User-facing message |
| path | TEXT | | Install folder `<data root>\models\<category>\<id>\<version>` |
| details | TEXT | | JSON: validation result — `{"validation": "checksum only"}` or the worker `validate_openvino_ir` result (see [WORKER.md](WORKER.md)) |
| updated_at | TEXT | NOT NULL, default now | |

- No foreign keys.
- **W**: `models::set_state` (upsert; every step of `models::install_job`, job `models.install`), `models::clear_state` (delete; `models::remove` ← `lib::remove_model`, and on cancel during download / hashing), `models::reset_interrupted` (working states → `install_failed`; ← `lib::reconcile` at start-up and data-folder switch).
- **R**: `models::list` (`lib::list_models`), `models::ready_dir` (`tagging::tag_job`, `transcribe::model_file` ← `transcribe::transcribe_job`).

### transcripts (migration 6)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| media_id | INTEGER | PK, FK media ON DELETE CASCADE | |
| model_id | TEXT | NOT NULL | e.g. `whisper-base`, `whisper-small` |
| language | TEXT | | Detected/requested code (`vi`, `en`…); NULL when no audio |
| segments_json | TEXT | NOT NULL | JSON array, see below |
| created_at | TEXT | NOT NULL, default now (reset on update) | |

`segments_json` = `Vec<transcribe::Segment>`: `[{"startMs": 0, "endMs": 2300, "text": "…"}]` (empty texts and loop runs of ≥ `LOOP_RUN` = 4 identical segments are dropped before storing).

- **W**: `transcribe::store` (upsert; ← `transcribe::transcribe_job`, job `media.transcribe`), `transcribe::clear` (← `media::import_one` when the file changed).
- **R**: `transcribe::get` (`lib::get_transcript`, `lib::recommend_for`, `editing::gather`, `export::export_job` for subtitles, `cli` transcript), `transcribe::transcribed` (`lib::list_media`, `cli::status`, `cli` media list).

### media_tags (migration 7)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| media_id | INTEGER | PK, FK media ON DELETE CASCADE | |
| model_id | TEXT | NOT NULL | Always `tagging::MODEL_ID` = `openclip-vit-b32` |
| device | TEXT | NOT NULL | OpenVINO device that ran the model (`GPU`/`CPU`) |
| tags_json | TEXT | NOT NULL | JSON object, see below |
| created_at | TEXT | NOT NULL, default now (reset on update) | |

`tags_json` = `tagging::MediaTags` (only these fields of the worker result are kept; `model`, `fallbacks`, `everyMs`, `seconds` are dropped):

```json
{"device": "GPU",
 "tags": [{"en": "beach", "vi": "bãi biển", "negative": false}, …],
 "samples": [{"atMs": 0, "tags": [[0, 0.4123], [1, 0.2011]]}]}
```

`samples[].tags` = `[index into tags, probability]`, best first (top 5 from the worker, sampled every `EVERY_MS` = 2000 ms). The taxonomy is stored per row, so rows tagged with different taxonomy versions stay readable.

- **W**: `tagging::store` (upsert; ← `tagging::tag_job`, job `media.tag`), `tagging::clear` (← `media::import_one` when the file changed).
- **R**: `tagging::get` (`lib::list_scenes`, `lib::recommend_for`, `editing::gather`, `cli`), `tagging::all` (`lib::list_media`, `lib::vocabulary` for the request box, `cli::status`, `cli` concepts/media). Summaries (`tagging::summarize`) are computed on read.

### media_marks (migration 8)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| media_id | INTEGER | PK, FK media ON DELETE CASCADE | |
| favorite | INTEGER | NOT NULL, default 0 | Bool; boosts ranking |
| excluded | INTEGER | NOT NULL, default 0 | Bool; Editing Core never uses the file |

- **W**: `editing::set_mark` (upsert; `lib::set_media_mark`).
- **R**: `editing::marks` (`lib::list_media`, `editing::gather`, `cli` media list).

### editing_plans (migration 8)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| project_id | INTEGER | PK, FK projects ON DELETE CASCADE | Latest plan only, one per project |
| style | TEXT | NOT NULL | Style id (`travel_cinematic`, `social_story`, `memory_nostalgia`, `travel_documentary`, `clean_minimal`) |
| plan_json | TEXT | NOT NULL | `editing::Plan`, see below |
| created_at | TEXT | NOT NULL, default now (reset on update) | |

`plan_json` = `editing::Plan` (serde camelCase):

```json
{"style": "travel_cinematic",
 "wishes": {"prefer": [], "less": [], "avoid": [], "targetMs": null, "preset": null, "color": null,
            "title": null, "ending": null, "captions": null, "music": null, "request": ""},
 "targetMs": 90000, "totalMs": 88500, "transitionMs": 600, "exportPreset": "landscape",
 "clips": [{"mediaId": 3, "sceneIndex": 2, "inMs": 4000, "outMs": 8500,
            "scores": {"technical": 0.8, "semantic": 0.6, "speech": 0.0, "favorite": 0.0},
            "rank": 0.71, "tags": ["biển"], "reasons": ["…"]}],
 "skipped": ["…"], "candidates": 42, "suggestedTitle": "Biển · 2026-10-01"}
```

`wishes` (`editing::Wishes`, every field defaults when missing; `wishes` and `suggestedTitle` default too, so older plans still load):

| Field | Type | Meaning |
|---|---|---|
| prefer | string[] | Concepts to show more of (rank × up to 1.5); match tag `en`/`vi`, case-insensitive |
| less | string[] | Concepts to show less of (rank × 0.5 where on screen) |
| avoid | string[] | Windows where one scores ≥ `AVOID_MIN` are not used |
| targetMs | int? | Length instead of the style target |
| preset | string? | Export preset instead of the style's (`reels` = 9:16 centre crop) |
| color | string? | Colour look (`export::COLORS`) |
| title, ending | string? | Title / ending text |
| captions | bool? | Speech subtitles on/off |
| music | string? | `none`, or `low`/`medium`/`high`/`any` = best library track of that energy |
| request | string | The request-box sentence the wishes were read from |

- Media ids inside `plan_json` are not foreign keys: a plan can still name a removed file; `editing::apply` then fails in `timeline::add` ("Media N not found.") and its transaction rolls back.
- **W**: `editing::save` (upsert; `lib::make_plan` ← `lib::generate_plan` and `cli "plan" --request`, `cli "plan"`).
- **R**: `editing::load` (`lib::get_plan`, `editing::apply`, `lib::apply_plan_to`, `lib::recommend_for` (style), `cli` show-plan/apply/export). Unparseable JSON reads as "no plan".

### project_settings (migration 9)

| Column | Type | Constraints | Meaning |
|---|---|---|---|
| project_id | INTEGER | PK, FK projects ON DELETE CASCADE | |
| export_json | TEXT | NOT NULL | `export::Options`, see below |

`export_json` = `export::Options` (serde camelCase, every field defaults; `credit` is `#[serde(skip)]` and never stored):

```json
{"fill": false, "transitionMs": 0, "transition": "fade", "color": "none", "fade": false,
 "originalVolume": 1.0,
 "music": {"path": "", "library": "track-id", "volume": 0.35, "duck": true},
 "titles": {"title": "", "subtitle": "", "ending": "", "captions": false},
 "normalize": false}
```

| Field | Meaning |
|---|---|
| fill | true = cover + centre-crop; false = letterbox |
| transitionMs / transition | xfade length (0 = hard cuts) / one of `export::TRANSITIONS` |
| color | one of `export::COLORS` |
| fade | Fade in/out from black (picture and sound) |
| originalVolume | Clips' own sound, 0..2 |
| music | `null` or `export::Music`: `path` (user's own file, empty when `library` set), `library` (track id from `sounds.rs`), `volume` 0..2, `duck` (sidechain ducking under speech) |
| titles | `export::Titles`: `title`, `subtitle`, `ending`, `captions` (burn transcript lines) |
| normalize | EBU R128 to −16 LUFS |

- Defaults when no row: `export::Options::default()`.
- **W**: `export::save_options` (runs `Options::check` first; `lib::set_export_options`, `lib::apply_plan_to`, `cli "options"` when flags are given).
- **R**: `export::load_options` (`lib::get_export_options`, `lib::apply_plan_to`, `cli "options"`, `cli "export"`, `cli::show_timeline`). The export job itself takes `options` from its job params, not from this table.

## 4. Invalidation and cascades

| Event | Database effect | Files |
|---|---|---|
| Media removed (`lib::remove_media` → `media::remove`) | Row deleted; `ON DELETE CASCADE` deletes its `project_clips`, `media_analysis`, `scenes`, `transcripts`, `media_tags`, `media_marks` rows. `editing_plans` rows are not touched (media ids live in JSON). | `proxy::remove_cache`: `cache\thumbs\<id>.jpg`, `cache\proxy\<id>.mp4`, `cache\frames\<id>\`. The original file is never touched. |
| Re-import, same path, **size or mtime changed** (`media::import_one`) | `media` row updated in place (same id); then `analysis::clear` (scenes + media_analysis), `transcribe::clear`, `tagging::clear`. **Kept**: `media_marks`, `project_clips` (their In/Out are not re-validated against the new duration), plans. | `proxy::remove_cache` (thumb, proxy, frames), then a new thumbnail. |
| Re-import, unchanged size and mtime | `media` row refreshed (probe fields); observations kept | Thumbnail regenerated |
| Re-analysis / re-tag / re-transcribe | `analysis::store` replaces all scenes of the file; tags/transcripts are upserts | `analysis.py` writes `<idx>.jpg` into the same frames folder; JPEGs of a previously larger scene count are not deleted |
| Project deleted (`projects::delete`) | Cascade deletes `project_clips`, `editing_plans`, `project_settings`. Media and observations are kept. | Exports in `<data root>\projects\` are not deleted |
| Plan applied (`editing::apply`) | All `project_clips` of the project replaced (transaction); `project_settings` rewritten by `lib::apply_plan_to` | — |
| Model removed (`models::remove`) | `models` row deleted; stored tags/transcripts made with it are kept | Install folder and partial downloads deleted |
| Start-up / data-folder switch (`lib::reconcile`) | `models` rows in a working state → `install_failed` | `proxy::remove_leftovers` deletes temporaries of unfinished jobs |

## 5. How to add a migration

1. **Append** a new string to the end of `MIGRATIONS` in [db.rs](../../apps/desktop/src-tauri/src/db.rs) with a `// N: purpose` comment. Never edit, reorder or delete a shipped migration — databases in the field have already run it.
2. Prefer additive SQL (`CREATE TABLE`, `ALTER TABLE … ADD COLUMN` with a default or nullable). Give child tables `REFERENCES … ON DELETE CASCADE` where rows must die with their media/project; remember foreign keys only work because `db::init` enables them.
3. The migration runs in a transaction together with the `user_version` bump; one statement batch per migration is fine (`execute_batch`).
4. Backups are automatic: an existing database gets a verified `before-upgrade` copy before the migration runs, and the migration is skipped if that copy fails. Nothing to add.
5. Keep JSON columns tolerant: new fields need `#[serde(default)]` so older rows still parse (readers treat unparseable JSON as missing).
6. Add the module's store/read functions and a unit test using `db::open_in_memory()`; existing `db.rs` tests check the count and idempotency automatically. Run `cargo test` in `apps/desktop/src-tauri`.
7. If a file change must invalidate the new data, add its `clear` call to `media::import_one`.
8. Update docs: this file (migrations table, table section, readers/writers, invalidation), [BACKEND.md](BACKEND.md) for the new functions, `CLAUDE.md` status line if it lists migration numbers, and `docs/PROGRESS.md`.
