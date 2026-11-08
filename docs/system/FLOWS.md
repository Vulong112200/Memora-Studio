# Processing flows

Last verified: 2026-11-08 against commit `b751673`. Index: [README.md](README.md). Kept current by the `sync-docs` skill.

What happens, step by step, when the app starts or the user does something. Function names are
`module::function` in `apps/desktop/src-tauri/src/` (details in [BACKEND.md](BACKEND.md)); UI files are in
`apps/desktop/src/` ([FRONTEND.md](FRONTEND.md)); tables in [DATABASE.md](DATABASE.md).

Common rules for every job: one job runs at a time (`jobs.rs` FIFO); progress is reported as `done/total`
(media jobs: 1000 units per file) and pushed to the UI as `job-updated`; per-file failures are collected
in the result (`{done, failed}`), they do not stop the job; every child process goes through
`process::spawn` (Job Object, no console window).

## 1. App start

1. `lib::run` → `settings::load(%APPDATA%\dev.memora.studio)` → `logging::init(<data root>\logs)`.
2. `lib::open_db` → `db::open`:
   - existing database: verified copy to `database\backups\` unless one is < 10 min old; **always** before a pending migration (if that copy fails the migration is not run and the database stays unchanged);
   - migrations (`PRAGMA user_version`), refusing a newer schema.
3. `lib::reconcile` (nothing runs yet, so leftovers are stale): `models::reset_interrupted` (working states → `install_failed` "Interrupted…"), `proxy::remove_leftovers` (temporaries, see [FOLDERS.md](FOLDERS.md)).
4. Asset scope allows `<data root>\cache`; `lib::job_runner` + `jobs::JobManager::start` (executor thread); `AppState` managed.
5. UI `App.tsx` loads settings + hardware; cards render by condition (data root set, no DB error, project open).

## 2. Choose / change the data folder

`App.tsx` DataFolder → `set_data_root(path)`:
1. Refused while a job is queued or running (`JobManager::busy`) — a running job would write to the wrong database.
2. `settings::prepare_data_root` (absolute, not in the exe folder, creates subfolders).
3. Opens the new database first (backup + migrations), runs `reconcile` on it; only then saves `settings.json`, swaps the DB handle and allows the new cache folder. Nothing is copied or deleted in the old folder. Logs move after a restart.

## 3. Import videos

`MediaLibrary.tsx` "Add videos" (file dialog) → `submit_job("media.import", {paths})` → `media::import_job`, per file:
1. `media::run_ffprobe` (60 s timeout) → `parse_probe` (first real video stream, duration, rotation from display matrix, fps, recording time).
2. `media::upsert` by path (case-insensitive). If the path was known and size/mtime changed: analysis, transcript, tags and cache files are cleared.
3. Thumbnail (`proxy` thumbnail at 10 % of the clip).
4. UI reloads the library and automatically submits `media.analyze` for the new ids.

## 4. Proxy (preview + analysis input)

`proxy::ensure_proxy` (called by analyze / tag jobs, or `media.proxy`): skip if `cache\proxy\<id>.mp4` exists; else try in order
`qsv` (full GPU, unrotated sources only) → `hybrid` (QSV decode, CPU rotation) → `libx264`; 540 p short side, ≤ 30 fps, AAC 96k; written via `.part` and renamed.

## 5. Analyse (scenes and technical quality)

`submit_job("media.analyze", {ids, quality})` → `analysis::analyze_job`, per file:
ensure proxy → AI worker job `analyze_video {path: proxy, framesDir, quality}` ([WORKER.md](WORKER.md): PySceneDetect scenes, phase-correlation shake, sample metrics, dHash, scene JPEGs) → `analysis::store` replaces `media_analysis` + `scenes`.
Scores are **not stored**: `scoring::score` computes technical score, flags and duplicates on every read (`list_scenes`, plan generation).

## 6. Tag (what is on screen)

`submit_job("media.tag", {ids})` → `tagging::tag_job`: needs `openclip-vit-b32` in state `ready` (`models::ready_dir`) → ensure proxy → worker `tag_video` (OpenCLIP IR, GPU → CPU, every 2 s, top-5 of the bilingual taxonomy incl. 4 bad-shot detectors) → `tagging::store` (`media_tags`). `tagging::summarize(start, end, min)` gives per-range concept scores on read.

## 7. Transcribe (speech)

`submit_job("media.transcribe", {ids, model, language})` → `transcribe::transcribe_job`, per file with audio:
FFmpeg extracts 16 kHz mono WAV from the **original** → `whisper-cli -oj -np -pp -mc 0 -sns -l <lang> -t <threads>` (+ `--vad -vm <silero>` when installed; cancel checked every 200 ms) → `parse_output` drops empty segments and loops of ≥ 4 identical lines → `transcribe::store` → temp WAV/JSON deleted. No audio → empty transcript.

## 8. Smart edit: request box → plan

1. `EditingPlan.tsx` request box: while typing (250 ms pause) `interpret_request(text)` → `request::parse(text, lib::vocabulary)` → "Understood: …" chips (or a hint listing words that work). Offline keywords only.
2. "Generate plan" → `generate_plan(projectId, style, from, to, request)` → `lib::make_plan`:
   - parse the request; its style and dates (if any) replace the selection; its wishes go into `editing::Wishes` (prefer / less / avoid concepts, length, preset, colour, title, ending, subtitles, music, request text);
   - `editing::gather(from, to)`: library files recorded in the range, with scored scenes, tags, transcript, marks (★ / ⊘);
   - `editing::plan_with(style, sources, wishes)`:
     1. skip excluded / unanalysed files (reason in `skipped`);
     2. candidate windows per scene (whole scene if ≤ max, else ideal-length windows); `fit_speech` moves edges out of sentences (extend ≤ 3× max, else shrink, else drop; ≤ 4 passes);
     3. windows showing an avoided concept (score ≥ 0.25) are left out and counted;
     4. scores: technical (window samples), semantic (top tag + style favourites), speech share, favourite; rank = style weights × scores × bad-shot penalty (≤ −80 %) × prefer boost (≤ ×1.5) × less cut (≥ ×0.5);
     5. greedy pick while the target has room: no overlap in a file, no duplicate scenes, per-scene **per-concept** cap (1 + scene/60 s), dHash novelty, concepts within their 35 % share first, diversity multipliers, tie-breaks;
     6. story order: recording time, then opening clip (style `open_with` or the hook) first, ending clip last; consecutive clips ≤ 1.5 s apart merged; suggested title;
   - `editing::save` → `editing_plans.plan_json` (with the wishes).
3. The card shows clips with reasons and T/S/V/★ scores; the user can reorder / drop (frontend only).

## 9. Apply the plan

"Apply to timeline" / "Apply and export" → `apply_plan(projectId, keep)` → `lib::apply_plan_to`:
`editing::apply` replaces `project_clips` in one transaction (keep order) → `editing::styled_options` (style look, then wishes: preset → fill, colour, title or suggested title, ending, subtitles, "no music") → wished music energy → best library track from `lib::recommend_for` → `export::save_options` (`project_settings`). "Apply and export" then submits `media.export` with the plan's preset.

## 10. Manual timeline editing

Media library preview (proxy) → "Set In" / "Set Out" → `add_clip`; scene / transcript click sets In/Out. Timeline card: `set_clip_caption` (on blur), `move_clip ±1`, `remove_clip`; Look & sound panel → `set_export_options` on every change (checked by `Options::check`). CLI adds `clip-trim` (`timeline::set_range`).

## 11. Export

`submit_job("media.export", {projectId, preset, options})` → `export::export_job`:
1. Load project name + timeline clips + media + transcripts; fail if empty or **any original file is missing** (never a proxy fallback); `Options::check`.
2. Library music: `sounds::fetch` downloads to `cache\downloads\sounds\`, verifies size + SHA-256 (deleted when the job ends, whatever the outcome); credit line added.
3. `export::render` → staging file `<name>-<preset>-<unix>.unvalidated.mp4` via `.part`: `hybrid-qsv` then `libx264` (no retry on disk-full / permission errors); filter graph: trim, fit / fill / blurred background, xfade, colour look, fades, ASS titles/captions/subtitles (temporary file in `cache\render`), clip volume, music (+ ducking), loudnorm.
4. `level_loudness` (when normalise is on): measure → exact gain ±12 dB + limiter.
5. `validate`: exists, duration, resolution / rotation, audio stream, full decode; warnings for black video, clipping, silence.
6. Promote: rename to the first free `<name>-<preset>-<unix>[-N].mp4`, or `…-failed-validation.mp4` when validation failed (kept, never under the normal name).
7. Result JSON → Timeline `Report` (checks, warnings, encoder, render seconds).

## 12. Install an AI model

`Models.tsx` Download (confirmation with size, licence, URL, SHA-256) or "Install from files…" → `submit_job("models.install", {id, files?})` → `models::install_job`: states `downloading` (curl.exe, resumable `.part` in `models\.downloads`) or copy → `verifying` (size + SHA-256) → `installing` (`models\<category>\<id>\<version>\`) → `validating` (OpenVINO IR: worker `validate_openvino_ir`; others: checksum only) → `ready` or a failure state. A crash mid-install is turned into `install_failed` at the next start.

## 13. Database backup and restore

Automatic at start-up (if none < 10 min old) and before every migration; manual via Data folder "Back up now" (`backup_database`; no CLI command). `db::backup`: `VACUUM INTO` → `PRAGMA integrity_check` on the copy (deleted if not "ok") → keep newest 10. Restore (manual): close the app, copy a backup over `database\memora.db`, delete `memora.db-wal` and `memora.db-shm`.

## 14. Cancel, timeouts, crashes

| Running | Cancel | Timeout |
|---|---|---|
| Queued job | removed immediately | — |
| FFmpeg (`proxy::run_ffmpeg`) | killed at the next progress line | none |
| ffprobe / loudness measurement (`process::wait_output`) | not cancellable (short) | 60 s / 600 s (assumed) |
| whisper-cli | killed within 200 ms | none |
| curl.exe | killed within 250 ms; `.part` kept for resume | none |
| AI worker job | `job.cancel` message; worker killed after 10 s without a reply, restarted on the next job | 60 s start-up handshake |

App exit or crash: the Job Object kills every child. Next start: `reconcile` (§1).

## 15. Command-line session (`memora-cli.exe`)

`cli::run` → `cli::open` (data root from settings or `MEMORA_DATA_ROOT`; `db::open` as in §1, start-up copy skipped if fresh) → command. Read commands print short text (`media`, `concepts`, `scenes`, `transcript`, `show-plan`, `timeline`, `options`). Edit commands call the same functions as the app (`lib::make_plan`, `lib::apply_plan_to`, `timeline::*`, `export::save_options`). `export` / `job` run the job inline through `lib::job_runner` with progress on stderr — the app should be closed first. Typical assistant session: `media` / `concepts` → `plan P --request "…"` → `show-plan` → `apply --keep …` → `options …` → `export P`.

## 16. Validate AI devices

`AiCenter.tsx` "Validate AI devices" → `submit_job("validate_devices")` → routed to the AI worker (`lib::job_runner`: kinds without a `media.` / `models.` prefix) → worker job `validate_devices` compiles a small test network on every OpenVINO device (CPU / GPU / NPU) and compares with NumPy → result table (compile ms, inference ms, precision, ok). Says nothing about real models; real models are load-tested by `models.install` (§12).

## 17. Music suggestions

`recommend_sounds(projectId)` / CLI `sounds P` → `lib::recommend_for`: plan style, on-screen concepts of the timeline clips (weighted by length), timeline length and speech share → `sounds::recommend` (style fit 50 %, content 30 %, length 10 %, talk-friendliness 10 %).
