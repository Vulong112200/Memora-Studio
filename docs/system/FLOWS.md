# Processing flows

Last verified: 2026-11-12 against commit `7eeb0bd` + uncommitted changes. Index: [README.md](README.md). Kept current by the `sync-docs` skill.

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

Library page `MediaLibrary.tsx` "+ Add videos" (file dialog) → `submit_job("media.import", {paths})` → `media::import_job`, per file:
1. `media::run_ffprobe` (60 s timeout) → `parse_probe` (first real video stream, duration, rotation from display matrix, fps, recording time).
2. `media::upsert` by path (case-insensitive). If the path was known and size/mtime changed: analysis, transcript, tags and cache files are cleared.
3. Thumbnail (`proxy` thumbnail at 10 % of the clip).
4. UI reloads the library and runs `ui::prepareMedia` for the new ids: `media.analyze`, then `media.tag` (OpenCLIP ready) and `media.transcribe` (Whisper small, else base, ready) — queued one after the other. The same three steps run from the Library **Analyse (N steps)** button and the "Next step" banner **Analyse now**.

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

0. UI: Projects page → open a project → step 2 "Smart edit" (the "Next step" banner leads there once the videos are analysed).
1. `EditingPlan.tsx` request box: while typing (250 ms pause) `interpret_request(text)` → `request::parse(text, lib::vocabulary)` → "Understood: …" chips (or a hint listing words that work). Offline keywords only.
2. "Make a plan" → `generate_plan(projectId, style, from, to, request)` → `lib::make_plan`:
   - parse the request; its style and dates (if any) replace the selection; its wishes go into `editing::Wishes` (prefer / less / avoid concepts, length, preset, colour, title, ending, subtitles, music, request text);
   - `editing::gather(from, to)`: library files recorded in the range, with scored scenes, tags, transcript, marks (★ / ⊘); `editing::gather_photos(from, to)`: library photos taken in the range with their analysis;
   - `editing::plan_with_photos(style, sources, photos, wishes)` (photos: see §20):
     1. skip excluded / unanalysed files (reason in `skipped`);
     2. candidate windows per scene (whole scene if ≤ max, else ideal-length windows); windows where the camera points down at the handlebars / ground ("view ahead" cue < 0.1, model v3) are left out and counted, except on a forest trail (model v4 cue ≥ 0.9), where a small bridge / slope / forest trail becomes the clip's concept; `journey_recap` picks its length from the footage when none is asked and first takes one clip per scene type and per 8 min of recording time; transcript lines with implausible timing (> 3 s + 1 s per word: Whisper stretched them over noise) are ignored; `fit_speech` moves edges out of the remaining sentences (extend ≤ 3× max, else shrink, else drop; ≤ 4 passes);
     3. windows showing an avoided concept (score ≥ 0.25) are left out and counted;
     4. scores: technical (window samples), semantic (top tag + style favourites), speech share, favourite; rank = style weights × scores × bad-shot penalty (≤ −80 %) × prefer boost (≤ ×1.5) × less cut (≥ ×0.5);
     5. greedy pick while the target has room: no overlap in a file, no duplicate scenes, per-scene **per-concept** cap (1 + scene/60 s), dHash novelty, concepts within their 35 % share first, diversity multipliers, tie-breaks;
     6. story order: recording time, then opening clip (style `open_with` or the hook) first, ending clip last; consecutive clips ≤ 1.5 s apart merged; suggested title;
   - `editing::save` → `editing_plans.plan_json` (with the wishes).
3. The card shows clips with reasons and T/S/V/★ scores; the user can reorder / drop (frontend only).

## 9. Apply the plan

"Use this plan →" / "Use and export now" (inline confirm first when the timeline already has clips) → `apply_plan(projectId, keep)` → `lib::apply_plan_to`:
`editing::apply` replaces `project_clips` in one transaction (keep order) → `editing::styled_options` (style look, then wishes: preset → fill, colour, title or suggested title, ending, subtitles, "no music"; with music and no subtitles the clips' own sound goes down to 0.35) → wished music energy → best library track from `lib::recommend_for` → `export::save_options` (`project_settings`). "Use and export now" then submits `media.export` with the plan's preset; the UI opens step 3 "Edit & export" (Timeline).

## 10. Manual timeline editing

Media library preview (proxy) → "Set In" / "Set Out" → `add_clip`; scene / transcript click sets In/Out. Timeline card (step 3):
- **Strip** (visual timeline): one block per clip (width from its length, 🖼 = photo) and a chip at every join with the length from `list_joins` (= `export::timeline_joins`: project transition, per-clip choices, clamped; amber = set on that join). Click a chip → join panel: style (`Project setting` clears it, or one of `TRANSITIONS`) and length (0–2 s) → `set_clip_transition(clip after the join)`; **▶ Preview this join** → `preview_join(projectId, index, preset)` → a few seconds around the join at half size (proxies) played in a `<video>`.
- **+ Add photos…** → photo picker (library photos, click order, seconds per photo) → `add_photo_clips`. Photo rows: seconds select → `set_clip_range(id, 0, ms)`; **Edit photo** → `memora:open-photo` event → App opens the Photos page with that photo selected (`memora.photo`).
- Rows: `set_clip_caption` (on blur), `move_clip ±1`, `remove_clip`; Look & sound panel → `set_export_options` on every change (checked by `Options::check`), then joins reloaded. Total = clip lengths − joins. CLI adds `clip-trim` (`timeline::set_range`); CLI `timeline` lists each join (`↳ name s`, "set on this clip").

## 11. Export

`submit_job("media.export", {projectId, preset, options})` → `export::export_job`:
1. Load project name + timeline clips + media + transcripts; fail if empty or **any original file (video or photo) is missing** (never a proxy fallback); photo clips rendered with their edits by the job worker (`photos::still`, 2880 px, `cache\render\still-*.jpg`, deleted afterwards); each clip's own transition kept on its segment; `Options::check`.
2. Library music: `sounds::fetch` uses `music\<id>.mp3` when it passes the size + SHA-256 check, else downloads it there once (kept); credit line added.
3. `export::render` → staging file `<name>-<preset>-<unix>.unvalidated.mp4` via `.part`: `hybrid-qsv` then `libx264` (no retry on disk-full / permission errors); filter graph: trim, fit / fill / blurred background, xfade, colour look, fades, ASS titles/captions/subtitles (temporary file in `cache\render`), clip volume, music (+ ducking), loudnorm. Timelines over 10 clips: parts of ≤ 10 clips rendered from the originals into `cache\render` (picture, own sound, transitions), then one pass joins them with the same transition and adds look, text, music and loudness (bounds FFmpeg memory; parts deleted afterwards).
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

`cli::run` → `cli::open` (data root from settings or `MEMORA_DATA_ROOT`; `db::open` as in §1, start-up copy skipped if fresh) → command. Read commands print short text (`media`, `concepts`, `scenes`, `transcript`, `show-plan`, `timeline`, `options`). `mark MEDIA --favorite on|off --exclude on|off` sets ★ / ⊘ like the library buttons. Edit commands call the same functions as the app (`lib::make_plan`, `lib::apply_plan_to`, `timeline::*`, `export::save_options`). `export` / `job` run the job inline through `lib::job_runner` with progress on stderr — the app should be closed first. Typical assistant session: `media` / `concepts` → `plan P --request "…"` → `show-plan` → `apply --keep …` → `options …` → `export P`.

## 16. Validate AI devices

`AiCenter.tsx` "Validate AI devices" → `submit_job("validate_devices")` → routed to the AI worker (`lib::job_runner`: kinds without a `media.` / `models.` prefix) → worker job `validate_devices` compiles a small test network on every OpenVINO device (CPU / GPU / NPU) and compares with NumPy → result table (compile ms, inference ms, precision, ok). Says nothing about real models; real models are load-tested by `models.install` (§12).

## 17. Music suggestions

Photo video: `recommend_sounds(null, videoMs)` → `sounds::recommend("memory_nostalgia", no tags, length, no speech)`; the first time the panel opens the top track is pre-selected (changeable). Project: `recommend_sounds(projectId)` / CLI `sounds P` → `lib::recommend_for`: plan style, on-screen concepts of the timeline clips (weighted by length), timeline length and speech share → `sounds::recommend` (style fit 50 %, content 30 %, length 10 %, talk-friendliness 10 %).

## 18. Edit a photo (ADR 0006, slice B1)

Photos page (`PhotoEditor.tsx`) "+ Add photos" (file dialog, `photos::EXTENSIONS`) → `import_photos(paths)` → `photos::import`: row (`upsert`, edits kept) → photo worker `photo_render` 480 px thumbnail + EXIF date → `set_info`. Select a photo → sliders change the edit stack → after 80 ms `preview_photo(id, edits, showMask, false)` (one request in flight, latest wins) → photo worker renders ≤ 1600 px to a new `preview-<n>.jpg` → shown. 500 ms after the last change `set_photo_edits` saves it (each save is one undo step). **Auto** → `auto_photo(id)` → suggested values merged onto the sliders. **Hold: original** → `preview_photo(…, before = true)` (`before.jpg`, once). **Show sky area** → the sky mask overlaid in pink. **Export photo** → `set_photo_edits` then `export_photo(id)` → full-size render to `photos\exports\<name>-edit[-N].jpg` (`.part` + rename); the original is never written. Remove (✕ on a thumbnail, confirm) → row and cache folder deleted, file kept. Opening the Photos page submits `media.photo_analyze` for photos not analysed yet (once per visit; the list reloads when it finishes).

## 19. Video from photos

Photos page **🎬 Make a video** → clicking thumbnails picks photos in order (number badge); **All photos** / **Sort by date taken** / **Clear**. The timeline strip shows each photo (number, seconds; ◀ ✕ ▶ to move or leave out) and the transition between photos. Settings: seconds per photo, format (`PRESETS`), file name, and the project's Look & sound panel without clip sound / ducking / speech subtitles (transition, colour, fill, fades, music, loudness, titles); kept in `localStorage` `memora.photoVideo`. **Create video** → `submit_job("media.photo_video", {photoIds, photoMs, preset, options, name})` → `photos::video_job`: each photo rendered with its edits (worker `photo_render`, 2880 px) into `cache\render\` → still segments → `export::finish` (library music, render, loudness, validation) → `photos\exports\<name>-<preset>[-N].mp4`, stills deleted. Progress and the validation report shown under the panel; photos are never written.

## 20. Photos in Smart edit (2026-11-12)

1. Photos page → `media.photo_analyze` for photos without observations → `photos::analyze_job` → worker `photo_analyze` (edited photo at 960 px: frame metrics + OpenCLIP tags / cues) → `photo_analysis`.
2. "Make a plan" → `lib::make_plan` → `editing::gather_photos` → `plan_with_photos`: photos taken during the trip (recording span of the analysed videos ± 3 h) that are not blurry / dark / overexposed / a bad shot / an avoided concept, one per near-identical picture, up to 25 % of the length (request "ít ảnh" 12 %, "nhiều ảnh" 40 %, "chỉ video" none), spread over the trip; each shown 4 s ("mỗi ảnh N giây"); placed after the last clip recorded before it was taken. Plan notes say how many were used and why others were not.
3. The plan card shows photo rows (🖼, seconds); "Use this plan" → `editing::apply` → `timeline::add_photo` for photo clips; export renders them with their edits (§11).

