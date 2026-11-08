# Architecture

Last verified: 2026-11-08 against commit `b751673`. Index: [README.md](README.md). Kept current by the `sync-docs` skill.

## 1. Principles (non-negotiable, from CLAUDE.md and ADR 0001)

- Windows desktop app; **local-first, offline-capable, AI-optional**. No mandatory cloud API, API key, CUDA or local LLM.
- **Originals are never modified.** Final renders always read originals, **never proxies**. Proxies are for preview and analysis only.
- **AI observes → Editing Core decides → FFmpeg renders.**
- The frontend never runs FFmpeg or loads models; the Rust backend orchestrates every process.
- Model ≠ runtime ≠ device: OpenCLIP on OpenVINO (GPU → CPU), Whisper on whisper.cpp (CPU), PyTorch dev-time only.
- CPU fallback mandatory; GPU / NPU / QSV validated at runtime, never assumed. Arc 140T memory is shared RAM, not VRAM.
- One heavy job at a time (fixed 2 GB pagefile). Claude Code is a development assistant, never a runtime dependency.
- The request box reads sentences with offline keywords (owner decision 2026-11-08), not a language model.

## 2. Process model

```
memora-desktop.exe (Tauri 2, Rust)                     memora-cli.exe (same Rust library, console)
 ├─ WebView2: React UI ── invoke() ──► Tauri commands      └─ cli.rs commands ─┐
 │        ▲ "job-updated" events                                               │
 │        └──────────── JobManager (jobs.rs): FIFO, one job at a time ◄────────┘ (CLI runs jobs inline)
 │                         └─ lib::job_runner → media / proxy / analysis / tagging / transcribe / export / models
 └─ Windows Job Object (KILL_ON_JOB_CLOSE): every child dies with the app, started with CREATE_NO_WINDOW
      ├─ ffmpeg.exe / ffprobe.exe       import, thumbnails, proxies, audio extraction, export, validation
      ├─ whisper-cli.exe (whisper.cpp)  transcription (+ Silero VAD)
      ├─ curl.exe                       model / library-music downloads (user-confirmed or export-time)
      └─ python memora_worker.py        AI worker, protocol v1 JSON lines: analysis, tagging, device/model validation
```

All persistent state is in SQLite (`<data root>\database\memora.db`) and files under the data root.

## 3. Layers

| Layer | Where | Responsibility |
|---|---|---|
| UI | `apps/desktop/src/` ([FRONTEND.md](FRONTEND.md)) | Cards, user actions, job progress. Calls only `api.ts` wrappers. |
| Commands | `lib.rs` ([BACKEND.md](BACKEND.md)) | Thin Tauri commands: validate, call modules, return JSON. |
| Jobs | `jobs.rs` + `lib::job_runner` | Long work: queued, progress, cancel, `job-updated` events. |
| Domain | `media`, `proxy`, `analysis`, `scoring`, `tagging`, `transcribe`, `editing`, `request`, `timeline`, `export`, `sounds`, `models`, `projects` | Logic; pure where possible (scoring, editing, request are pure and unit-tested). |
| Infrastructure | `db`, `process`, `worker`, `settings`, `logging`, `hardware` | Database, child processes, worker protocol, config, logs, hardware info. |
| AI worker | `workers/ai-worker/` ([WORKER.md](WORKER.md)) | Python + OpenCV + OpenVINO observations. |
| Data | SQLite + data root ([DATABASE.md](DATABASE.md), [FOLDERS.md](FOLDERS.md)) | Library, observations, plans, timelines, settings. |

## 4. Configuration

| Item | Value / location |
|---|---|
| App settings | `%APPDATA%\dev.memora.studio\settings.json` → `{"dataRoot": "<path>" \| null}` |
| Data root | owner: `E:\Memora Data\` (must be absolute, not inside the exe folder); default suggestion `%LOCALAPPDATA%\MemoraStudio` |
| CLI data root | the app setting, or env `MEMORA_DATA_ROOT` |
| Logging | env `MEMORA_LOG` (EnvFilter, default `info`); stderr + daily JSON files `<data root>\logs\memora.log.YYYY-MM-DD` |
| Tauri | identifier `dev.memora.studio`; window 1200×800 (min 900×600); CSP allows `asset:` images/media; asset scope = `<data root>\cache` only (set at runtime); capabilities `core:default`, `dialog:allow-open` |
| Tool lookup | ffmpeg/ffprobe: `<data root>\runtime\ffmpeg\` → PATH → WinGet Links; whisper-cli: `<data root>\runtime\whisper.cpp\b5454\`; worker: `<data root>\runtime\ai-worker\` → repo dev venv |
| Manifests | `models.json` (AI models), `sounds.json` (music library), both embedded at build time |

## 5. Versions (verified 2026-11-08)

| Component | Version |
|---|---|
| Rust / edition | 1.99.1 / 2024 |
| Tauri / tauri-plugin-dialog | 2.12.5 / 2.8.4 |
| rusqlite (bundled SQLite) | 0.37.0 |
| React / TypeScript / Vite / Tailwind | 19.3.1 / 6.0.3 / 8.3.6 / 4.3.5 |
| Python worker | 3.13.17, openvino 2026.4.1, opencv-python-headless 5.0.0.93, scenedetect 0.7.2, numpy 2.5.4 |
| FFmpeg | 9.0.2 Gyan **GPL** build (dev only; licence review before distribution) |
| whisper.cpp | b5454 (v1.9.5), CPU build |

Full dependency tables: [BACKEND.md](BACKEND.md), [FRONTEND.md](FRONTEND.md), [WORKER.md](WORKER.md).

## 6. Target hardware

ASUS Zenbook 14 UX3405CA · Core Ultra 9 285H (16 threads) · Arc 140T (128 MB dedicated, shared RAM) · AI Boost NPU (optional) · 32 GB RAM · fixed 2 GB pagefile · Windows 11 26300. Measurements: [../HARDWARE_VALIDATION.md](../HARDWARE_VALIDATION.md), [../MODEL_PROPOSAL.md](../MODEL_PROPOSAL.md).

## 7. Decisions

| ADR | Decision |
|---|---|
| [0001](../adr/0001-phase0-decisions.md) | Data root on the data drive, user-selectable; FFmpeg GPL for dev; Python 3.13 venv worker; OpenCLIP on OpenVINO; whisper.cpp separate process; app limits its own memory |
| [0002](../adr/0002-worker-packaging.md) | Embedded Python worker, JSON-lines stdio, single-threaded with PeekNamedPipe, Job Object |
| [0003](../adr/0003-classical-analysis.md) | Analysis on the 540p proxy in one OpenCV pass; observations stored, scores computed in Rust |
| [0004](../adr/0004-local-ai-and-editing-core.md) | whisper.cpp + VAD + loop filter; OpenCLIP fixed bilingual taxonomy; deterministic Editing Core; export looks |
| [0005](../adr/0005-free-music-library.md) | Free music library: verified catalog, download on export, delete after, credit burned in |
| PROGRESS 2026-11-08 | Reliability batch (audit R2–R6); per-concept caps; `memora-cli`; offline request box instead of a Claude API call |

## 8. Approval gates

Phase transitions, installs, model downloads, driver / env / Windows changes, scanning personal folders and uploading media need the owner's explicit approval (CLAUDE.md). Phase 7 (packaging) is not approved.
