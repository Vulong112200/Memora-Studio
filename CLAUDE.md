# Memora Studio — Claude Code rules

Windows desktop app for organizing, analyzing, editing, and exporting personal videos.
Local-first, offline-capable, AI-optional. Claude Code is a development assistant, never a runtime dependency.

## Language
- Reply to the user in **Vietnamese**.
- Write code, comments, commit messages, and project docs in **English**.

## System documentation (keep it true)
- Code-level docs live in `docs/system/` (index `docs/system/README.md`): ARCHITECTURE, FOLDERS, BACKEND, FRONTEND, DATABASE, WORKER, FLOWS. Read the relevant ones before changing code — they are faster than re-reading every module.
- **After any change to the app's structure or behaviour** (module, Tauri command, job kind, migration/SQL, React component, api.ts wrapper, worker job, CLI command, flow, folder, dependency, config), **run the `sync-docs` skill** (`.claude/skills/sync-docs/SKILL.md`) before reporting the work as done or committing. Its checker `python .claude/skills/sync-docs/check_docs.py` must pass.
- Source of truth: code > manifests > `docs/system` > ADRs > PROGRESS / APP_OVERVIEW.

## Source documents (read before any work)
- `docs/Memora_Studio_Phase_0_Technical_Spec_and_Claude_Prompt.md` — Phase 0 spec, roadmap (Phase 0–7), approval gates. **Authoritative for phase gates.**
- `docs/Production.md` — full technical specification (v1.1). Plays the role of `PRODUCT_SPEC.md`.
- Where the two disagree, report the conflict; do not silently pick one.

## Status
- Phase 0 (read-only discovery) is **done**: `docs/DISCOVERY_REPORT.md`, decisions in `docs/adr/0001-phase0-decisions.md`.
- Phase 1a done (`docs/DEPENDENCY_MATRIX.md`). Phase 1b spikes done (`docs/HARDWARE_VALIDATION.md`, ADR 0002 accepted: embedded Python worker).
- Phase 2: Slice 2.1: `apps/desktop` (Tauri 2 + React + TS + Tailwind v4), Rust modules `settings` (data root, first-run, `%APPDATA%\dev.memora.studio\settings.json`), `logging` (JSON daily files in `<data root>\logs`), `hardware` (registry + GlobalMemoryStatusEx + DXGI). Single crate with modules for now; split into `crates/` when a second consumer appears. Slice 2.2 done, awaiting review: `db` (rusqlite bundled, WAL, append-only migrations via `PRAGMA user_version`, refuses newer schema) and `projects` (CRUD). Slice 2.3 done (ADR 0002 accepted): `process` (Job Object + CREATE_NO_WINDOW), `jobs` (FIFO, one at a time, progress/cancel, `job-updated` events, in-memory), `worker` (spawn on demand, protocol v1 handshake, restart after crash; resolves `<data root>\runtime\ai-worker\` else dev venv `workers/ai-worker/.venv`), real worker `workers/ai-worker/memora_worker.py` (job `validate_devices`; `diagnostic.*` test-only, not exposed to UI). **Phase 2 complete.**
- Phase 3 (Media Engine) approved ("tiếp tục" after the Phase 3 question). Slice 3.1 done, awaiting review: `media` module (migration 2 `media` table, path UNIQUE NOCASE, raw FFprobe JSON kept; job `media.import` runs FFprobe in Rust via the Job Object; ffprobe resolved `<data root>\runtime\ffmpeg\` → PATH → winget Links), dialog plugin (`dialog:allow-open`), Media library UI. User samples in `test-media`: 4 DJI clips, HEVC Main10 10-bit 2688×1512 50 fps + AAC — 10-bit HEVC must be handled by thumbnails/proxies. Slice 3.2 done: `proxy` module (thumbnails `cache\thumbs\<id>.jpg` on import, proxies `cache\proxy\<id>.mp4` 540p/≤30fps H.264 via job `media.proxy`; full-QSV path for unrotated sources, libx264 fallback and for rotated sources; `.part` files + rename; asset protocol scoped to `<data root>\cache` only), preview player in the Media library. Measurements in `docs/HARDWARE_VALIDATION.md` §6a. Slice 3.3 done: `timeline` (migration 3 `project_clips`, ordered trimmed segments, cascades on project/media delete), `export` (presets Story/Reels/Landscape/Archive, letterbox fit, silence for clips without audio, renders from ORIGINALS via hybrid QSV decode → libx264 fallback, `.part` + rename, §8.3 validation: exists/duration/resolution/audio/full decode with `-xerror`, black-frame + clipping warnings), Timeline UI + Set In/Out in preview, jobs log completion/failure. **Phase 3 complete.**
- Phase 4 approved ("Tiếp theo thôi"). Done, awaiting review — decisions in `docs/adr/0003-classical-analysis.md`: worker `analysis.py` (one OpenCV pass over the PROXY: PySceneDetect AdaptiveDetector, phase-correlation shake, sample-frame metrics + dHash, scene JPEGs in `cache\frames\<id>\`; self-check `python analysis.py`), Rust `analysis` (migration 4 `media_analysis` + `scenes`, job `media.analyze` = ensure proxy + worker `analyze_video`), `scoring` (pure deterministic technical score + flags + dHash duplicates, thresholds are initial assumptions), scene strip in preview (click → In/Out). Portrait test clips (phone-style display matrix, generated from user footage) in `E:\Memora Data\test-media\generated\`. **Phase 4 complete.**
- Phase 5 approved ("duyệt Phase 5") — model downloads and torch install are NOT approved yet. Slice 5.1 Model Manager done, awaiting review: `models.json` manifests (no URL/checksum → downloads disabled), `models.rs` (curl.exe resumable download / offline copy → size + SHA-256 → versioned install → load validation; migration 5 `models`), worker `validate_openvino_ir`, "AI models" card. Model proposal approved 2026-11-08 (`docs/MODEL_PROPOSAL.md`): whisper.cpp b5454 runtime in `<data root>\runtime\whisper.cpp\b5454\`, Whisper base/small staged, OpenCLIP exported to IR by `tools/clip-export/` (dev-only torch venv). Slice 5.2 Transcription done, awaiting review: `transcribe` module (migration 6 `transcripts`, job `media.transcribe`, anti-hallucination flags). Slices 5.3 (OpenCLIP tagging, migration 7) and 5.4 (Silero VAD) done. Phase 6 Editing Core done (`editing.rs`, migration 8, Smart edit UI, ADR 0004) plus export crossfades/centre-crop and date-range recaps — awaiting review. Owner review round 1 done (ADR 0004 revision: sentence-safe cuts, bad-shot detection via OpenCLIP v2 tags, export Options with transitions/colour/music/titles, migration 9). Free music library done (ADR 0005: verified catalog `sounds.json`, download on export then delete, credit burned in, second loudness pass). Reliability batch done (`docs/IMPROVEMENT_AUDIT.md` R2–R6: DB backups `database\backups\` via VACUUM INTO, start-up reconciliation of interrupted installs/temp files, `process::spawn`/`wait_output` timeouts, worker cancel grace, export validated before promotion and never overwriting, data-root switch refused while jobs run). Plan variety: per-concept scene cap, `MAX_CONCEPT_SHARE` 35 %, order by recording time. `editing::Wishes` (prefer/less/avoid concepts, length, preset, colour, title, ending, subtitles, music) stored with the plan and honoured on apply. Request box (owner decision: offline keywords, no LLM/cloud): `request.rs` + Smart edit textarea with "Understood:" read-back. `memora-cli` (`src/cli.rs`, `src/bin/memora-cli.rs`) shares `lib::job_runner`/`make_plan`/`apply_plan_to` with the app. 82 Rust tests. App overview: `docs/APP_OVERVIEW.md`. Open: packaging (FFmpeg licence review first), audit items R1 relink, E4 per-join transitions, M1 HDR, V1 versions. Day log and next steps: `docs/PROGRESS.md`. Repo: https://github.com/Vulong112200/Memora-Studio (branch `main`).
- Editing for the owner: use `apps/desktop/src-tauri/target/release/memora-cli.exe` (`help` lists commands; text output, cheap) instead of driving the GUI. Read with `media`/`concepts`/`scenes`/`transcript`, edit with `plan --request "…"` (same offline reader as the app's request box) or `plan --prefer/--less/--avoid`, `apply --keep`, `clip-*`, `options`, then `export`. The owner should close the app before `export`/`job`. Test on a copy via `MEMORA_DATA_ROOT` when changing code.
- Screenshots for UI checks: capture only the app window (PrintWindow), never the whole screen — other windows on the user's desktop must not be captured.
- App code now lives in `apps/desktop` (approved by user's "continue" into Phase 2). Test: `cargo test` in `apps/desktop/src-tauri`; build: `npm run tauri build -- --no-bundle` in `apps/desktop`.
- Worker rule from 1b: never block on stdin in a background thread (Windows import deadlock) — single-threaded worker, `PeekNamedPipe` polling during jobs.
- Installed and approved: winget packages `Rustlang.Rustup`, `Gyan.FFmpeg` (nothing else system-wide); Python 3.13 venvs `spikes/ai-worker/.venv` and `workers/ai-worker/.venv` (openvino, opencv-python-headless, scenedetect, numpy, pinned in `requirements.lock.txt`). Never pip-install into system Python. `spikes/` is throwaway code.
- Git: repo https://github.com/Vulong112200/Memora-Studio, branch `main`. Commit/push only when the owner asks.
- Still forbidden without separate approval: downloading AI models, installing torch, changing drivers / env vars / Windows settings (incl. pagefile), admin commands, scanning personal folders, uploading media.

## Key decisions (ADR 0001)
- Data root: `E:\Memora Data\` (user-selectable; moved from D: on 2026-11-07 — D: is the owner's software drive, E: the data drive). App binaries only on C:. Never put data in the repo folder `E:\Memora Studio\`.
- FFmpeg: Gyan GPL build for dev/internal use; revisit license before distribution.
- OpenCLIP runs on OpenVINO; PyTorch is dev-time export only. whisper.cpp is a separate native process.
- App limits its own memory (fixed 2 GB pagefile): monitor commit charge, one heavy model, no AI during render.
- Test media: synthetic + user samples in `E:\Memora Data\test-media\`.

## Approval gates
Every phase transition (Phase 1 Environment Setup → Phase 7 Product Completion) needs explicit user approval.
The user controls software installation, model downloads, driver changes, file access, and major architecture decisions.

## Architecture rules (all phases)
- Separate **AI Model** (OpenCLIP, Whisper) / **Inference Runtime** (OpenVINO, whisper.cpp, PyTorch, ONNX Runtime) / **Execution Device** (CPU, Arc GPU, NPU). OpenVINO is a runtime, not a model or provider. Do not force every model through OpenVINO.
- CPU fallback is mandatory. GPU, NPU, and QSV must be validated at runtime, never assumed.
- Arc 140T uses shared system RAM (128 MB dedicated). Never treat shared memory as VRAM.
- NPU is optional and experimental. Windows detecting it does not prove runtime support.
- No mandatory cloud APIs or API keys, no CUDA, no local LLM in the MVP.
- Never modify original media. Never render final output from proxies.
- AI produces observations; Editing Core decides; FFmpeg renders.
- Frontend never runs FFmpeg or loads models directly; Rust backend orchestrates processes.
- Record model sources, licenses, and checksums. Never trust unverified URLs.
- **Never invent benchmark numbers.** Label estimates and assumptions clearly. Separate verified from theoretical compatibility.
- One major module per reviewed change. Tests for every processing module.
