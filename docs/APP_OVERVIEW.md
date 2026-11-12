# Memora Studio — Application Overview

Snapshot of the app as built, **2026-11-11** (forest-trail cues, auto length and transitions, new bilingual UI), commit `9ac182c` + uncommitted changes on `main` (first written at `5f6bb1a`, refreshed after the music library, reliability batch, plan variety, command-line tool, request box and the Nam Cát Tiên editing session: `journey_recap`, long renders in parts, quieter own sound under music, stretched-transcript filter, `mark`).
One-file reference for setup, stack, AI models, processing logic and UI. **Code-level details** (every module, component, table, worker job and flow) are in [system/README.md](system/README.md); this file is the summary. Facts come from the code and the documents below. Measured numbers are quoted as written in the source documents. Anything that is an assumption is labelled as such.

| Source | Role |
|---|---|
| [Phase 0 spec](Memora_Studio_Phase_0_Technical_Spec_and_Claude_Prompt.md) | Roadmap and approval gates (authoritative for gates) |
| [Production.md](Production.md) | Full technical specification (v1.2) |
| [DISCOVERY_REPORT.md](DISCOVERY_REPORT.md) | Phase 0 findings |
| [DEPENDENCY_MATRIX.md](DEPENDENCY_MATRIX.md) | Installed tools and versions |
| [HARDWARE_VALIDATION.md](HARDWARE_VALIDATION.md) | Phase 1b / Phase 3 measurements |
| [MODEL_PROPOSAL.md](MODEL_PROPOSAL.md) | Model sources, licences, checksums, measurements |
| [PROGRESS.md](PROGRESS.md) | Day log, open items |
| [IMPROVEMENT_AUDIT.md](IMPROVEMENT_AUDIT.md) | Audit of the improvement backlog, findings and their status |
| [adr/](adr/) | ADR 0001–0006 |

---

## 1. Status

| Phase | State |
|---|---|
| 0 Discovery | Done |
| 1a Environment / 1b Spikes | Done (ADR 0002: embedded Python worker) |
| 2 Desktop shell (settings, logging, hardware, DB, projects, jobs, worker) | Done |
| 3 Media Engine (import, thumbnails, proxies, timeline, export) | Done |
| 4 Classical analysis (scenes, quality, shake, duplicates) | Done |
| 5 Local AI (Model Manager, transcription, OpenCLIP tagging, Silero VAD) | Done, awaiting review |
| 6 Editing Core (smart edit, styles, crossfades, recaps, titles, music) | Done, owner review round 1 done, awaiting review |
| Free music library (ADR 0005) | Done |
| Reliability batch (audit R2–R6) | Done |
| Plan variety, editing wishes, request box, `memora-cli` | Done |
| 7 Product completion (packaging, installer, …) | **Not approved yet** |

Still forbidden without separate approval: downloading models beyond the approved proposal, installing torch outside the dev export venv, driver / env var / Windows-settings changes, admin commands, scanning personal folders, uploading media.

## 2. Product and principles

- Windows desktop app to organise, analyse, edit and export personal videos.
- **Local-first, offline-capable, AI-optional.** No cloud APIs or API keys, no CUDA, no local LLM.
- **Original media is never modified.** Final output is always rendered from originals, **never from proxies**.
- **AI produces observations → Editing Core decides → FFmpeg renders.**
- The frontend never runs FFmpeg or loads models. The Rust backend orchestrates every process.
- Model ≠ runtime ≠ device. OpenCLIP runs on OpenVINO. Whisper runs on whisper.cpp. PyTorch is dev-time only, for export.
- CPU fallback is mandatory. GPU, NPU and QSV are validated at runtime, never assumed.
- The app limits its own memory (fixed 2 GB pagefile): one heavy model at a time, no AI during render.

## 3. Target hardware and verified environment

| Item | Value |
|---|---|
| Machine | ASUS Zenbook 14 UX3405CA |
| CPU | Intel Core Ultra 9 285H (16 cores / 16 threads) |
| GPU | Intel Arc 140T, driver 32.0.101.8860. 128 MB dedicated; uses **shared system RAM** (not VRAM) |
| NPU | Intel AI Boost, driver 32.0.100.4023 (optional, experimental) |
| RAM | 32 GB (31.36 GB usable), **fixed 2,048 MB pagefile** |
| OS | Windows 11 build 26300.9711 |
| Drives | C: binaries, D: software, **E: data** (`E:\Memora Data\`) |

| Tool | Version |
|---|---|
| FFmpeg / ffprobe | 9.0.2 (Gyan **GPL** build, dev/internal use; licence must be reviewed before distribution) |
| OpenVINO | 2026.4.1 |
| Python | 3.13.17 (venvs only, never system Python) |
| Rust | 1.99.1 |
| Tauri | 2.12.5 |
| whisper.cpp | b5454 (= v1.9.5), CPU build |

System-wide installs (winget): `Rustlang.Rustup`, `Gyan.FFmpeg`. Nothing else.

## 4. Tech stack and languages

| Layer | Language / tech |
|---|---|
| Desktop shell + backend | **Rust** (edition 2024), **Tauri 2** |
| Frontend | **TypeScript 6 + React 19**, Vite 8, **Tailwind CSS v4** |
| AI / analysis worker | **Python 3.13** (OpenVINO, OpenCV, PySceneDetect, NumPy) |
| Speech | whisper.cpp native exe (`whisper-cli.exe`) |
| Media | FFmpeg / ffprobe (external processes) |
| Database | SQLite via `rusqlite` (bundled), WAL |
| Model export (dev only) | Python + PyTorch + open_clip → OpenVINO IR |

### 4.1 Rust crates (`apps/desktop/src-tauri/Cargo.toml`, resolved from Cargo.lock)

| Crate | Resolved | Notes |
|---|---|---|
| tauri | 2.12.5 | feature `protocol-asset` |
| tauri-build | 2.7.4 | build dependency |
| tauri-plugin-dialog | 2.8.4 | file pickers |
| serde / serde_json | 1.0.229 / 1.0.154 | |
| tracing / tracing-subscriber / tracing-appender | 0.1.44 / 0.3.23 / 0.2.5 | JSON + env-filter |
| rusqlite | 0.37.0 | bundled SQLite (libsqlite3-sys 0.35.0) |
| sha2 | 0.10.9 | model checksums |
| windows | 0.62.2 | Job Objects, DXGI, registry, memory status, file system |

Package `memora-desktop` 0.1.0, lib `memora_desktop_lib`. Release profile: `codegen-units=1`, `lto`, `opt-level=3`, `panic=abort`, `strip`.

### 4.2 npm packages (`apps/desktop/package.json`)

| Package | Installed |
|---|---|
| react / react-dom | 19.3.1 |
| @tauri-apps/api | 2.12.4 |
| @tauri-apps/plugin-dialog | 2.8.4 |
| @tauri-apps/cli (dev) | 2.12.4 |
| vite (dev) | 8.3.6 |
| tailwindcss + @tailwindcss/vite (dev) | 4.3.5 |
| @vitejs/plugin-react (dev) | 6.2.1 |
| typescript (dev) | 6.0.3 |

No router, state library, UI kit, icon library or frontend test framework.

### 4.3 Worker packages (`workers/ai-worker/requirements.lock.txt`)

```
click==8.5.1            colorama==0.4.6         numpy==2.5.4
opencv-python-headless==5.0.0.93                openvino==2026.4.1
openvino-telemetry==2025.2.0                    platformdirs==4.14.5
scenedetect==0.7.2      tqdm==4.70.1
```

`scenedetect` declares `opencv-python`, which is replaced by the headless build. `pip check` warns, but only about metadata.

### 4.4 Dev-only export venv (`tools/clip-export/requirements.lock.txt`, 40 pins)

Key pins: `torch==2.15.0`, `torchvision==0.29.2`, `open_clip_torch==4.2.1`, `openvino==2026.4.1`, `timm==1.0.31`, `safetensors==0.8.0`, `huggingface_hub==2.4.0`, `pillow==13.0.0`.

## 5. Repository layout

| Path | Contents |
|---|---|
| `apps/desktop/` | The app: React frontend `src/`, Rust backend `src-tauri/` |
| `apps/desktop/src-tauri/models.json` | Model manifests (embedded at build time) |
| `apps/desktop/src-tauri/sounds.json` | Free music library catalog (32 verified CC BY 4.0 tracks: Kevin MacLeod + Scott Buckley, ADR 0005) |
| `apps/desktop/src-tauri/src/bin/memora-cli.rs` | Command-line tool (thin wrapper over `src/cli.rs`) |
| `workers/ai-worker/` | Production Python worker: `memora_worker.py`, `analysis.py`, `tagging.py` |
| `tools/clip-export/` | Dev-time OpenCLIP → OpenVINO IR exporter |
| `spikes/` | Throwaway Phase 1b spikes (protocol, QSV, OpenVINO devices, orchestrator, packaging, Tauri smoke) |
| `docs/` | Specs, ADRs, reports |
| `.claude/` | Claude Code commands and permissions |
| `CLAUDE.md` | Project rules for Claude Code |

`.gitignore` excludes models (`*.bin`, `*.onnx`), media, `*.db`, `.venv*`, `target/`, `node_modules/`. No data lives in the repo.

## 6. Configuration and data layout

### 6.1 Settings

- File: `%APPDATA%\dev.memora.studio\settings.json`. Single field: `{"dataRoot": "<path>" | null}`. A missing or unreadable file falls back to defaults.
- The default suggestion is `%LOCALAPPDATA%\MemoraStudio`. The owner uses **`E:\Memora Data\`**.
- The data root must be absolute and must not be inside the exe folder.

### 6.2 Data root layout

```
<data root>\
  database\memora.db                    SQLite (WAL)
  database\backups\memora-*.db          verified copies: start-up (if none < 10 min old), before migrations, "Back up now"; last 10 kept
  logs\memora.log.YYYY-MM-DD            daily JSON-lines logs
  runtime\ffmpeg\                       optional bundled ffmpeg/ffprobe
  runtime\whisper.cpp\b5454\            whisper-cli.exe, whisper.dll, ggml*.dll
  runtime\ai-worker\python\, app\       embedded Python worker (packaging target)
  models\<category>\<id>\<version>\     installed models
  models\.downloads\<id>-<ver>\         resumable downloads (.part)
  cache\thumbs\<id>.jpg                 thumbnails
  cache\proxy\<id>.mp4                  540p preview proxies
  cache\frames\<id>\<i>.jpg             scene representative frames
  cache\audio\<id>.wav                  temporary 16 kHz audio for whisper
  cache\render\titles-*.ass             temporary subtitle/title files
  music\<id>.mp3                        library music, downloaded once and kept (verified on every export)
  projects\<id>\exports\*.mp4           final exports (never overwritten: -2, -3 …; failed validation → *-failed-validation.mp4)
  projects\<id>\exports\*.unvalidated.mp4  render in progress (renamed only after validation)
  test-media\                           user + synthetic samples (dev)
```

### 6.3 External tool resolution

| Tool | Order (first existing wins) |
|---|---|
| ffmpeg / ffprobe | `<data root>\runtime\ffmpeg\` → `PATH` → `%LOCALAPPDATA%\Microsoft\WinGet\Links\` (dev) → bare name |
| whisper-cli | `<data root>\runtime\whisper.cpp\b5454\whisper-cli.exe` |
| AI worker | `<data root>\runtime\ai-worker\python\python.exe` + `app\memora_worker.py` → dev venv `workers/ai-worker/.venv\Scripts\python.exe` |

### 6.4 Tauri configuration (`tauri.conf.json`, `capabilities/default.json`)

| Setting | Value |
|---|---|
| productName / version / identifier | Memora Studio / 0.1.0 / `dev.memora.studio` |
| Window | 1200×800, minimum 900×600 |
| Dev server | `http://localhost:1420` (strict port; HMR 1421 when `TAURI_DEV_HOST` is set) |
| CSP | `default-src 'self'; img-src 'self' data: asset: http://asset.localhost; media-src 'self' asset: http://asset.localhost https://incompetech.com https://www.scottbuckley.com.au; style-src 'self' 'unsafe-inline'` |
| Asset protocol | Enabled with empty static scope. At runtime only `<data root>\cache` (recursive) is allowed |
| Capabilities | `core:default`, `dialog:allow-open` |
| Plugins | `tauri-plugin-dialog` only |

### 6.5 Logging

- `tracing`, level from env `MEMORA_LOG` (EnvFilter syntax), default `info`.
- Human-readable output to stderr always. Daily JSON files in `<data root>\logs` once a data root exists.
- A changed data root moves file logging only after a restart.

## 7. Architecture

```
React UI (WebView2)                 memora-cli.exe (cli.rs, text output)
   │ invoke() wrappers (api.ts)        │ same functions, runs one job at a time itself
   ▼                     ▲ "job-updated"▼
Tauri commands (lib.rs, 33 commands) ──► JobManager (jobs.rs) ──► lib::job_runner (shared with the CLI)
                                          FIFO, ONE job at a time, in memory
                                          │
          ┌───────────────────────────────┼─────────────────────────────┐
          ▼                               ▼                             ▼
   FFmpeg / ffprobe                whisper-cli.exe              Python AI worker
   (import, proxy, export,         (media.transcribe)           (protocol v1, JSON lines)
    audio extraction)                                           analyze_video, tag_video,
                                                                validate_devices,
                                                                validate_openvino_ir
          └───────── every child: Windows Job Object (KILL_ON_JOB_CLOSE) + CREATE_NO_WINDOW ─┘

SQLite (db.rs) ◄── media, analysis, transcripts, tags, plans, timeline, models
```

### 7.1 Rust modules (`apps/desktop/src-tauri/src/`)

| Module | Responsibility |
|---|---|
| `main.rs` / `lib.rs` | Entry, `AppState`, commands, `job_runner` (job routing, shared with the CLI), `make_plan`, `apply_plan_to`, `recommend_for`, `vocabulary`, start-up `reconcile` |
| `cli.rs` + `bin/memora-cli.rs` | Command-line tool: the engine without the window (section 15) |
| `request.rs` | Request box: offline keyword reader for a Vietnamese/English sentence (section 15) |
| `sounds.rs` | Free music library: catalog, recommendations, download-verify-delete on export |
| `settings.rs` | settings.json, data root preparation |
| `logging.rs` | tracing setup |
| `hardware.rs` | Registry (OS, CPU), `GlobalMemoryStatusEx`, DXGI GPUs, disk free space |
| `db.rs` | SQLite open, WAL, migrations, verified backups (`VACUUM INTO` + `integrity_check`) |
| `projects.rs` | Project CRUD (name 1–200 chars) |
| `process.rs` | Job Object + CREATE_NO_WINDOW; `spawn` (kills a child that cannot join the Job Object), `wait_output` (timeout + cancel) |
| `jobs.rs` | Job queue, progress, cancellation, `busy()` |
| `worker.rs` | Python worker spawn, handshake, restart; killed 10 s after an ignored cancel |
| `media.rs` | ffprobe, library, job `media.import` |
| `proxy.rs` | Thumbnails, proxies, FFmpeg helpers, job `media.proxy` |
| `analysis.rs` | Scene/sample storage, job `media.analyze` |
| `scoring.rs` | Pure technical score, flags, duplicates |
| `tagging.rs` | OpenCLIP tags, job `media.tag` |
| `transcribe.rs` | whisper.cpp, job `media.transcribe` |
| `models.rs` | Model Manager, job `models.install` |
| `timeline.rs` | Project clips (ordered trimmed segments) |
| `editing.rs` | Editing Core: styles, plan generation, apply |
| `export.rs` | Render from originals + validation, job `media.export` |

### 7.2 Tauri commands

`get_settings, set_data_root, backup_database, get_hardware_info, list_projects, create_project, rename_project, delete_project, list_media, remove_media, list_scenes, get_transcript, set_media_mark, list_styles, generate_plan (with optional request sentence), interpret_request, get_plan, apply_plan, list_models, remove_model, list_clips, add_clip, remove_clip, list_sounds, recommend_sounds, set_clip_caption, get_export_options, set_export_options, move_clip, submit_job, list_jobs, cancel_job, get_worker_info`

`set_data_root` is refused while a job is queued or running; the new database is opened before settings are saved.

### 7.4 Crash safety

- At start-up (and on a data-folder switch, when no job runs): model rows stuck in `downloading/verifying/installing/validating` become `install_failed` "Interrupted…"; app temporaries are removed (`cache\render|audio|downloads\sounds`, `.part`, `.level.mp4`, `.unvalidated.mp4`). Resumable model downloads are kept.
- Timeouts (assumed values, not measured): ffprobe 60 s, loudness measurement 600 s, worker cancel grace 10 s.

### 7.3 Job system

- UI-submittable kinds: `validate_devices, media.import, media.proxy, media.export, media.analyze, media.transcribe, media.tag, models.install`.
- Routing: `models.*` → models.rs; `media.*` → its module; any other kind → Python worker. `diagnostic.*` is test-only and not exposed to the UI.
- Statuses: `queued, running, completed, failed, cancelled`. Job = `{id, kind, params, status, done, total, result, error}`.
- One executor thread (`job-executor`). Cancel: a queued job is cancelled immediately; a running job is flagged and the runner polls the flag.
- Only the latest 50 finished jobs are kept, in memory only.
- Media jobs use 1000 progress units per file, collect failures per file and continue. The result is `{done, failed}`.

## 8. Database (`db.rs`)

`<data root>\database\memora.db`. `journal_mode=WAL`, `foreign_keys=ON`, busy timeout 5 s. Migrations are append-only and tracked by `PRAGMA user_version`, each in its own transaction. The app **refuses a database with a newer schema**.

| # | Tables / change | Key columns |
|---|---|---|
| 1 | `projects` | id, name (CHECK 1–200), created_at, updated_at |
| 2 | `media` | path (UNIQUE NOCASE), size, modified_unix, duration_ms, width, height, rotation, fps, video/audio codec, audio_streams, bitrate, source_created_at, **probe_json** (raw ffprobe), imported_at |
| 3 | `project_clips` | project_id (CASCADE), media_id (CASCADE), position, in_ms ≥ 0, out_ms > in_ms |
| 4 | `media_analysis`, `scenes` | analysis version/quality; scenes idx, start/end_ms, shake, frame_path, samples_json |
| 5 | `models` | id, version, state, error, path, details (no row = not installed) |
| 6 | `transcripts` | media_id, model_id, language, segments_json |
| 7 | `media_tags` | media_id, model_id, device, tags_json |
| 8 | `media_marks`, `editing_plans` | favorite, excluded; project_id, style, plan_json |
| 9 | `project_clips.caption`, `project_settings` | caption TEXT; export_json per project |

Timestamps are ISO-8601 UTC strings.

## 9. Media pipeline

### 9.1 Import (`media.import`, params `{paths}`)

- Picker filter: mp4, mov, m4v, mkv, avi, mts, m2ts, 3gp, webm, wmv.
- ffprobe: `-v error -print_format json -show_format -show_streams`.
- Parsing: the first video stream that is not an attached picture. Duration from the format, else the stream (must be > 0). Rotation from the display matrix, else legacy `tags.rotate`, normalised to 0..359. fps from `avg_frame_rate`, else `r_frame_rate`. Recording date from `com.apple.quicktime.creationdate`, else `creation_time`.
- Re-importing a known path whose size or mtime changed clears its analysis, transcript, tags and cache.
- A thumbnail is generated on every import. The UI then submits analysis, tagging and transcription (when their models are installed) for the new files automatically.

### 9.2 Thumbnails and proxies (`proxy.rs`)

| Item | Setting |
|---|---|
| Thumbnail | Frame at 10 % of the duration, short side 180, `-q:v 4` |
| Proxy | Short side **540**, never upscaled, even dimensions, rotation applied; fps capped at **30** when the source is above 30.5; H.264 + AAC 96k stereo; `+faststart` |
| Attempt 1 `qsv` (unrotated only) | Full GPU: `-hwaccel qsv`, `vpp_qsv`, `h264_qsv -preset veryfast -global_quality 28` |
| Attempt 2 `hybrid` | QSV decode `-noautorotate` → `hwdownload` → manual transpose → display matrix removed → `h264_qsv` |
| Attempt 3 `libx264` | `-preset veryfast -crf 26`, yuv420p |
| Safety | Written as `.part`, then renamed. Existing proxies are kept |

The user's DJI samples are HEVC Main10 10-bit 2688×1512 50 fps. Proxies and thumbnails handle 10-bit sources.

## 10. Classical analysis and scoring

### 10.1 Worker `analysis.py` (one OpenCV pass over the **proxy**, never the original)

| Parameter | Value |
|---|---|
| Sample interval | fast 2.0 s / **balanced 1.0 s** (default) / detailed 0.5 s |
| Scene detection | PySceneDetect `AdaptiveDetector(min_scene_len = fps·0.6)` on frames 320 px wide |
| Global motion | `cv2.phaseCorrelate` on 160 px grey frames; shift kept only if response > 0.1 |
| Shake | std of (shift − 0.5 s moving average) / 160; 0 if fewer than 5 shifts |
| Sample metrics | sharpness = Laplacian variance; brightness = mean/255; contrast = std/255; clipLow ≤ 10; clipHigh ≥ 245; black = mean < 20 and std < 10; 64-bit dHash (9×8) |
| Scene frame | Sample nearest the midpoint, JPEG 320 px wide, quality 85 |
| Self-check | `python analysis.py` (synthetic scenes; asserts cuts ±70 ms, black, sharpness and shake ratios, dHash) |

Known gap (`ponytail:` comment): a scene shorter than the sample interval may get no sample.

### 10.2 `scoring.rs` (pure, computed when read, never stored)

> **The thresholds are initial assumptions**, not validated on a broad set of footage.

Per-sample values are medians over the scene.

| Component | Formula | Weight |
|---|---|---|
| sharpness | clamp((log10(lapVar) − 1.5) / 2) | 0.35 |
| exposure | clamp(1 − 4·(clipLow + clipHigh)) | 0.20 |
| brightness | clamp(1 − max(0, \|b − 0.45\| − 0.2) / 0.25) | 0.15 |
| contrast | clamp(c / 0.18) | 0.10 |
| stability | clamp(1 − shake / 0.02) | 0.20 |

The total is 0 when the scene has no samples or is black (more than 50 % black samples).

Flags: `not sampled`, `black`, `blurry` (sharpness < 0.3), `shaky` (stability < 0.4), `dark` (brightness < 0.2), `overexposed` (clipped > 0.15 and high > low), `duplicate`.
Duplicates: dHash Hamming distance ≤ **6** of 64, compared across the whole library.

## 11. AI models and runtimes

### 11.1 Models (`models.json`)

| id | Version | Category | Runtime / device | Files (bytes) | Min RAM | Licence | Source | SHA-256 | `validated` |
|---|---|---|---|---|---|---|---|---|---|
| `openclip-vit-b32` | 4 | vision | OpenVINO IR / GPU → CPU | `openclip_image.xml` 491,391; `openclip_image.bin` 175,698,576; `openclip_tags.json` 322,909 (v4: 48 tags, 4 bad-shot detectors, cues view ahead / forest trail / small bridge / slope) | 8 GB | MIT (open_clip); LAION card: English-only, research-oriented | Local export of `laion/CLIP-ViT-B-32-laion2B-s34B-b79K@1a25a44…` (offline install only, no URL) | yes | false |
| `whisper-base` | 1 | speech | whisper.cpp / CPU | `ggml-base.bin` 147,951,465 | 4 GB | MIT | HF pinned `@5359861c…` | yes | false |
| `whisper-small` | 1 | speech | whisper.cpp / CPU | `ggml-small.bin` 487,601,967 | 8 GB | MIT | HF pinned `@5359861c…` | yes | false |
| `silero-vad` | 1 | speech | whisper.cpp / CPU | `ggml-silero-v6.2.0.bin` 885,098 | 1 GB | MIT | HF pinned `@9ffd54a1…` | yes | true |

Runtime binary: whisper.cpp **b5454** `whisper-bin-x64.zip` (8,928,640 B, MIT, SHA-256 recorded in MODEL_PROPOSAL). CPU build only; the release has no Vulkan/SYCL build for Windows.

### 11.2 Model Manager (`models.rs`)

- Pipeline: download with `curl.exe --location --fail --retry 3 --continue-at -` (resumable `.part`), or copy from local files → size + **SHA-256** check → versioned install to `models\<category>\<id>\<version>\` → load validation (OpenVINO IR models compile and run one inference on zeros in the worker via `validate_openvino_ir`).
- States: `not_installed, downloading, verifying, installing, validating, ready, download_failed, checksum_failed, install_failed, model_load_failed`, plus computed `update_available`.
- A manifest without a URL or checksum disables download. Nothing downloads without user confirmation in the UI.

### 11.3 Transcription (`transcribe.rs`, job `media.transcribe`)

- Params: `{ids, model: "whisper-base"|"whisper-small", language: "auto"|"vi"|"en"}`. The UI picks Whisper small when installed, else base, language auto.
- Audio is extracted to `cache\audio\<id>.wav` (16 kHz mono PCM) and deleted afterwards. Progress split: extraction 15 %, whisper 85 %.
- Command: `whisper-cli -oj -np -pp -mc 0 -sns -l <lang> -t <threads>`. Threads = half the logical CPUs, clamped to 1–8.
- **Anti-hallucination**: `-mc 0` (no context carry-over), `-sns` (suppress non-speech), Silero VAD (`--vad -vm ggml-silero-v6.2.0.bin`) whenever `silero-vad` is installed, and runs of **≥ 4 identical consecutive segments are dropped**.
- A file with no audio gets an empty transcript. The UI warns that it is machine transcription.

### 11.4 Tagging (`tagging.rs` + worker `tagging.py`, job `media.tag`)

- Needs `openclip-vit-b32` in state `ready`. Missing proxies are created first. Reads the proxy.
- Samples every **2000 ms**. Preprocessing: short side to 224 (bicubic), centre crop, CLIP mean/std.
- Device: the first of `["GPU","CPU"]` that compiles and gives finite output. The NPU is not used by default. Devices that failed are reported as `fallbacks`.
- Image embedding is L2-normalised, then multiplied against precomputed text embeddings × logitScale, softmax, **top-5** kept.
- Fixed bilingual taxonomy (English/Vietnamese): **48 concept tags + 4 negative "bad shot" detectors** (finger over the lens, blurry shot, camera pointing at the ground, too dark). Prompt template `"a photo of {}."`.
- `summarize(start, end, min)` gives the mean probability per tag over a time range; Editing Core uses min 0.1.

### 11.5 OpenCLIP export (dev only, `tools/clip-export/export_openclip.py`)

`python export_openclip.py <checkpoint.safetensors> <out dir> [frames dir]`, run offline (`HF_HUB_OFFLINE=1`). Checkpoint: ViT-B/32 `laion2b_s34b_b79k`, `open_clip_model.safetensors` (605,143,316 B; safetensors chosen to avoid pickle). Steps: `ov.convert_model(model.visual)` → [1,3,224,224] → FP16 weights. Text embeddings are precomputed into `openclip_tags.json`. Check: cosine ≥ 0.999 vs PyTorch and 100 % top-tag agreement.

### 11.6 Python worker protocol v1 (`memora_worker.py` ↔ `worker.rs`)

- JSON lines over stdio (stdin decoded as `utf-8-sig`). Large data is passed as file paths.
- **Single-threaded.** Blocking stdin read only while idle. During a job it polls with `PeekNamedPipe`, because a thread blocked on stdin deadlocks `import numpy/openvino` on Windows.
- Start: `python -u memora_worker.py` in the Job Object. The worker must send `{"type":"hello","protocol":1,pid,python,openvino,devices,startupMs}` within **60 s**, otherwise it is killed. If OpenVINO fails to import, the worker still starts and reports `runtimeError`.
- In: `health`, `job.start {id,kind,params}`, `job.cancel {id}`, `shutdown`. Out: `hello`, `health.ok`, `job.progress`, `job.result`, `job.cancelled`, `job.error`, `error`.
- Started on demand, restarted after a crash on the next job. The app polls every 100 ms.

| Worker job | Purpose |
|---|---|
| `validate_devices` | Compile a test MLP on every OpenVINO device; compile/infer ms, precision, rel. error (ok < 1e-2) |
| `analyze_video` | `analysis.analyze(proxy, framesDir, quality)` |
| `tag_video` | `tagging.tag_video(proxy, modelDir, everyMs=2000, devices=[GPU,CPU])` |
| `validate_openvino_ir` | Model Manager load test |
| `diagnostic.steps` / `diagnostic.crash` | Tests only |

## 12. Editing Core (`editing.rs`)

Deterministic, with no LLM. Inputs: scenes + scores, tags, transcripts, user marks (★ favorite, ⊘ exclude).

### 12.1 Styles

| id | Target | Preset | Clip min / ideal / max | Transition | Weights [tech, sem, speech, fav] | Look | Speech captions | Opening → ending |
|---|---|---|---|---|---|---|---|---|
| `travel_cinematic` | 90 s | landscape | 3 / 4.5 / 6 s | 600 ms fade | .45 .40 .05 .10 | cinematic | no | landscape → sunset/sea/sky |
| `social_story` | 30 s | reels (9:16) | 1.5 / 2 / 3 s | hard cuts | .40 .40 .00 .20 | vivid | no | strongest clip (hook) → people |
| `memory_nostalgia` | 180 s | landscape | 3 / 5 / 7 s | 800 ms fade | .20 .40 .20 .20 | warm | no | chronological → family/friends |
| `travel_documentary` | 300 s | landscape | 5 / 8 / 15 s | 300 ms fade | .30 .25 .35 .10 | none | yes | landscape/city → chronological |
| `journey_recap` | 300 s (from the footage when no length is asked: 4.2 s per minute, 1:30–6:00) | landscape | 4 / 6 / 9 s | 700 ms auto (cross dissolve, dip to black between parts of the day) | .40 .35 .10 .15 | warm | no | chronological (whole trip in order) |
| `clean_minimal` | 60 s | landscape | 3 / 4 / 5 s | hard cuts | .60 .25 .00 .15 | none | no | chronological |

### 12.2 Plan generation

1. **Gather**: media filtered by recording day (date range: whole library, last month / 6 months / year, custom). Skipped files get a reason: "excluded by you", "not analysed yet", "no scene long enough for this style".
2. **Candidate windows**: scenes shorter than min are dropped. Scenes up to max are used whole. Longer scenes are cut into ideal-length windows.
3. **Sentence-safe cuts (`fit_speech`)**: transcript lines longer than 3 s + 1 s per word are ignored (Whisper stretches short lines over wind/road noise). An edge that falls inside speech is extended to the whole segment (clip ≤ 3×max, inside the scene), or shrunk to exclude it (clip ≥ min), or the window is dropped. At most 4 iterations.
4. **Scores**: technical (samples in the window), semantic (0.5 if untagged; otherwise 0.5·top + 0.5·min(1, 2·favoured-concept score)), speech (fraction of the window covered by speech), favorite (0/1). Weighted by the style. ★ files are always in the plan (their best moment first, more from long ones; looked at every half clip length) unless nothing in them is usable.
5. **Bad-shot penalty**: rank × (1 − min(0.8, 2·Σ negative-tag scores)), i.e. up to −80 %.
6. **Greedy selection** while total + min ≤ target. Overlaps in the same file and duplicate scenes are skipped. Per scene, **per main concept**, at most 1 + scene_ms/60,000 clips (a walk up a path and the tree at its end, in one continuous shot, count separately). A second clip from the same scene needs dHash distance > 12 **or** a different dominant concept.
   Diversity multiplier = 0.85^(same concept) × 0.9^(same file) × 0.6^(same file within 3×ideal).
   **Concept share**: while other content is available, one main concept fills at most `MAX_CONCEPT_SHARE` = 35 % of the target (initial value); it only fills up the rest when nothing else is left. Owner feedback 2026-11-08: a recap that was mostly cycling dropped the walk to the big tree.
7. **Story structure**: chronological order by full recording time (then media, in), then the opening clip (style's `open_with`, score > 0.2, or the hook) moves first and the ending clip (`end_with`) moves last.
8. **merge_near**: consecutive clips from one file ≤ 1.5 s apart are merged.
9. **Suggested title**: most frequent Vietnamese main tag + date range, e.g. "Xe đạp · 20 – 27/02/2026".

### 12.3 Wishes (user requests on top of a style)

`editing::Wishes`, stored in the plan JSON (`plan_with(style, sources, wishes)`):

| Field | Effect |
|---|---|
| `prefer` | Concepts to show more of: rank × up to 1.5 (reason "wanted: …") |
| `less` | Concepts to show less of: rank × down to 0.5 (reason "less wanted: …") |
| `avoid` | Windows where an avoided concept scores ≥ 0.25 are left out (counted in `skipped`) |
| `target_ms` | Length instead of the style's target |
| `preset` | Export preset instead of the style's (reels → fill / centre crop) |
| `color`, `title`, `ending`, `captions` | Applied to the export options on Apply |
| `music` | `none` removes music; `low` / `medium` / `high` / `any` = best-suited library track of that energy, chosen for the applied timeline (`lib::apply_plan_to`) |
| `request` | The sentence the wishes came from (request box) |

Concept names match tag `en` or `vi`, case-insensitive.

### 12.4 Apply

- The user can reorder or drop clips in the UI (frontend only), then **Use this plan** replaces `project_clips` in one transaction.
- Styled export options are saved too: fill = (plan preset is reels), the style's transition, colour (or the wished one), fade on, normalise on, speech captions per style (or wish), the wished title, or the suggested title only if none is set. Existing music and texts are kept unless a wish changes them.
- **Apply and export** also submits `media.export` with the plan's preset.

## 13. Timeline and export

### 13.1 Timeline (`timeline.rs`)

Ordered trimmed segments of a project. `out_ms` is clamped to the media duration; a clip must be at least 100 ms. Caption up to 200 chars. Moving a clip swaps it with its neighbour. Rows cascade on project or media delete.

### 13.2 Export (`export.rs`, job `media.export {projectId, preset, options}`)

Renders from the **original files**. The job fails if the timeline is empty or an original is missing.

| Preset | Size | FPS |
|---|---|---|
| `landscape` | 1920×1080 | 30 |
| `story` | 1080×1920 | 30 |
| `reels` | 1080×1920 | 30 |
| `archive` | First clip's upright size (never scaled) | First clip's fps, ≤ 60 (default 30) |

| Encoder (in order, fallback logged) | Settings |
|---|---|
| `hybrid-qsv` | QSV decode, `h264_qsv -preset medium -global_quality 22`, nv12 |
| `libx264` | Software decode, `-preset medium -crf 20`, yuv420p |
| Audio | AAC 192k, 48 kHz stereo; `+faststart` |

**Options**

| Option | Values / behaviour |
|---|---|
| Framing | `fill` = centre crop. Otherwise pad if the aspect ratio is within 5 %, else the clip sits on a **blurred, darkened copy of itself** (no black bars). Subject-aware crop is not implemented |
| Transition | 0 / 300 / 500 / 800 / 1200 ms; `fade, dissolve, fadeblack, smoothleft, slideleft, wipeleft, circleopen` via `xfade` + `acrossfade`. Clamped to half of the shortest clip |
| Colour look | `none, warm, cool, vivid, cinematic, vintage, bw` (fixed filter strings) |
| Fade | From/to black, 600 ms (≤ total/4), audio too |
| Clip volume | 0–2 (UI 0–1.5) |
| Music | User file (mp3, m4a, aac, wav, flac, ogg, opus) **or a free library track** (downloaded once into `music\`, SHA-256 checked on every export, credit burned in at the end), looped and trimmed, 1.5 s fade-out, volume (default 0.35), optional **ducking** (`sidechaincompress` under the clips' sound) |
| Normalise | `loudnorm I=-16 TP=-1.5 LRA=11` in the render, then a second pass: measure (ebur128) → exact gain (±12 dB) + limiter at −1 dBTP |
| Text | Opening title + subtitle (0–4 s), per-clip captions (4 s labels), speech subtitles from transcripts, ending text (last 3.5 s). Burned in with ASS, Segoe UI Semibold. User text is neutralised (`\`→`/`, `{}`→`()`) |

Duration = Σ clip lengths − (N−1)·transition.
**Long timelines** (> 10 clips, `MAX_INPUTS`): parts of ≤ 10 clips are rendered from the originals into `cache\render` at higher quality (QSV `global_quality 16` / x264 `crf 14`, AAC 320k; picture, own sound, transitions), then one pass joins them with the same transition and adds look, text, music and loudness. Measured on a 26-clip recap: FFmpeg peak 11 GB → 5.2 GB working set, same duration, render 121 s → 149 s.
With music and no speech subtitles, applying a plan lowers the clips' own sound to 0.35 (DJI own sound ≈ −25 dBFS would otherwise keep the music ducked the whole video).
Output: rendered to `<name>-<preset>-<unix>.unvalidated.mp4` (via `.part`), levelled, **validated, then renamed** to the first free name `<name>-<preset>-<unix>.mp4` (`-2`, `-3` … — an earlier export is never replaced). A file that fails validation is kept as `…-failed-validation.mp4`. Disk-full / permission errors are not retried with the second encoder.

**Validation** (progress 900–1000 of 1000):
- Checks: exists, size > 0; duration within ±(200 + 100·segments) ms; resolution = target and rotation 0; has audio; full decode with `-xerror`.
- Warnings: black segments (`blackdetect d=1`), clipping (max ≥ −0.1 dB), silence (mean ≤ −80 dB).
- The report shows output path, size, duration, render seconds, encoder, checks and warnings.

## 14. User interface (`apps/desktop/src/`)

Reworked 2026-11-11 (owner: "not clear what to do next").

- **Layout**: left sidebar with four pages — **Library**, **Projects**, **Jobs**, **Settings** — plus the open project and a language switch; a **"Next step"** banner (Library / Projects) says what to do now with one button; a **job bar** at the bottom shows the running job on every page. No router (page in React state, remembered per computer with the open project). `job-updated` is the only backend event the UI listens to.
- **Language** (owner decision 2026-11-11): bilingual, Vietnamese by default, English selectable; every string is written in both languages where it is used (`L(en, vi)` in `i18n.ts`). Texts that come from Rust (style descriptions, plan reasons, errors, validation checks) are not translated yet. Search folds Vietnamese diacritics.
- **Styling**: Tailwind v4 utilities; shared pieces in `ui.tsx` (`Card`, buttons, `Thumb`, `Progress`, job helpers). Zinc neutrals with an indigo accent; light/dark follows Windows. Unicode icons.

| Page / part | File | What it does |
|---|---|---|
| Next step banner | `App.tsx` | From the library and project state: add videos → analyse (missing steps) → create/open a project → Smart edit → edit & export; link to install AI models when one is missing |
| Library | `MediaLibrary.tsx` | **+ Add videos** (new videos are analysed, tagged and transcribed automatically when the models are installed), **Analyse (N steps)** for anything missing, search by name/tag, table (thumbnail, recorded, length, resolution, size, analysis, speech, on screen), ★ / ⊘ / Remove; selected row → proxy preview, scene strip, transcript, Set In / Out → "Add to {project}" |
| Projects | `Projects.tsx` + `App.tsx` `ProjectPage` | Create / open / rename / delete; for the open project a 3-step bar: 1 Videos (→ Library), 2 Smart edit, 3 Edit & export |
| Step 2 Smart edit | `EditingPlan.tsx` | Request box with "Understood: …" and word list, style + dates → **Make a plan**; plan notes (auto length, coverage), clip list with T/S/V/★, reasons, ↑/↓/Drop; **Use this plan →** (asks before replacing an existing timeline) or **Use and export now**; then opens step 3 |
| Step 3 Edit & export | `Timeline.tsx` | Format + **Export video**, progress, validation report with **copy path**; clips with caption, ↑/↓/Remove; **Look & sound** (transitions incl. Automatic, colour, fill, fades / clip sound, music library or own file, ducking, loudness / titles, subtitles) |
| Photos | `PhotoEditor.tsx` | Add photos (read in place); strip of thumbnails; large preview; **Auto**, Undo, Reset, hold for original, show sky area; sliders for light, colour (+ looks, HSL), sky, detail, effects & frame; **Export photo** (full size, `photos\exports`) — see section 14a |
| Jobs | `AiCenter.tsx` | Every job of the session with progress and Cancel, AI worker status, Validate AI devices |
| Settings | `App.tsx`, `Models.tsx` | Data folder + backup, AI models (download with confirmation, install from files, remove), system and graphics |

All Tauri calls go through typed wrappers in `src/api.ts`.

## 14a. Photo editor (ADR 0006, slice B1)

- Photos are read in place and never written; the edit stack (JSON) is stored in the `photos` row (migration 10); exports are new files `<data root>\photos\exports\<name>-edit[-N].jpg`.
- Rendering: worker `photo.py` (OpenCV + numpy, no model) in a **second worker process** (`photos::Lane`), so previews (≤ 1600 px) never wait behind queued video jobs. Radii scale with size, so previews match the full-size export.
- Tools: rotate / straighten / aspect crop, noise reduction, white balance, exposure, dehaze, highlight roll-off, shadows / highlights / whites / blacks, contrast, clarity, edge-only sharpening, vibrance / saturation, HSL per colour, looks, vignette, **sky enhancement** (the photo's own sky: cloud detail, depth, colour, warmth — owner 2026-11-12: cinematic but natural, nothing pasted), **Auto**.
- Measured on this PC (single runs): 2688×1512 full render 1.2–1.4 s; 1600 px preview ≈ 0.45 s with Auto values.
- **AI photo tools** (2026-11-12, local models approved by the owner): click a person / object to remove it (EfficientSAM + MI-GAN), remove by painting, face retouch (spots and marks removed, skin smoothed with its texture kept, brightened; eyes, brows, lips untouched — YuNet + Face Mesh), background blur like a camera (Depth Anything V2 Small + MODNet). Auto adds face retouch; batch auto-edit and auto on import; moved photos found again in a folder. AI upscale not yet (Real-ESRGAN file does not load).
- **Photos in Smart edit** (2026-11-12, migration 12): photos are analysed when the Photos page opens (sharpness / exposure / near-duplicates and OpenCLIP content of the edited photo). Plans take photos taken during the trip, leave out blurry, dark, duplicate and avoided ones, use up to a quarter of the length and place each photo after the clip recorded just before it. Request words: "có ảnh", "nhiều ảnh", "ít ảnh", "chỉ video", "mỗi ảnh 3 giây".
- **Photos in project timelines** (2026-11-12, migration 11): Timeline step **+ Add photos…** puts library photos (with their edits) between video clips; each photo has its own seconds and an **Edit photo** link. The timeline is shown as a strip of clip blocks with every transition; any join can get its own transition and **▶ Preview this join** renders a few seconds around it from the proxies.
- **Video from photos** (2026-11-12): **🎬 Make a video** on the Photos page — pick photos in order, timeline strip with the transitions, seconds per photo, format, Look & sound (transition, colour, music with a suggested track, titles). Job `media.photo_video` renders each photo with its edits, then the normal export (validation included) to `photos\exports\<name>-<preset>.mp4`. Owner: no motion (Ken Burns) wanted.
- Not yet (need model-download approval): select and remove objects / people, blemish retouch, portrait bokeh, AI upscale / denoise; generative scenery deferred by the owner. Exports do not carry camera EXIF yet.

## 15. Request box and command-line tool

### 15.1 Request box (`request.rs`)

Owner decision 2026-11-08: **offline keyword reading** — no language model, no network, no API key (a Claude API option was offered and declined for now). A Vietnamese or English sentence becomes a style, dates and `Wishes`; the UI reads back "Understood: …" before anything is made. Unknown words are ignored and the box says so.

| Recognised | Examples |
|---|---|
| Length | `30 giây`, `1 phút 30 giây`, `45s`, `2 min` |
| Frame | `dọc`, `reels`, `tiktok`, `9:16` (→ reels + social style unless another style is named) · `ngang`, `16:9` |
| Concepts | `nhiều` / `thêm` (default) · `bớt` / `ít` / `less` · `bỏ` / `không` / `tránh` / `no` + any concept of the library's tag names (en or vi), per clause; `thiên nhiên` / `nature` = the landscape concepts |
| Colour | `tông ấm`, `lạnh`, `rực rỡ`, `cổ điển`, `đen trắng` |
| Music | `nhạc nhẹ`, `nhạc sôi động`, `nhạc vui`, `có nhạc`, `không nhạc` |
| Text | `phụ đề` / `không phụ đề`, `tiêu đề: …` (or quoted), `kết thúc bằng: …` |
| Dates | `tháng 2/2026`, `tháng 2 năm 2026`, `02/2026`, `năm 2025`, `từ 01/02/2026 đến 15/02/2026` |
| Style | `điện ảnh`/`cinematic`, `hoài niệm`, `tài liệu`/`vlog`, `tối giản`, `sôi động`, `tổng hợp`/`hành trình`/`cả chuyến đi`/`recap` (journey_recap) |

Rules: needles with diacritics match exactly ("âm" ≠ "ấm"); without diacritics they also match accented text. Concept names without diacritics match only when unambiguous (several words or ≥ 5 letters: "thuyen" yes, "bien" no). Title/ending text is taken first so its words never become wishes; longer concept names win ("bãi biển" is beach, not also sea).

### 15.2 Command-line tool (`memora-cli.exe`)

Built next to the app (`target\release\memora-cli.exe`). Same data folder (`MEMORA_DATA_ROOT` overrides), database, Editing Core and export; results show up in the app. Short text output — the token-cheap way for an assistant to edit for the owner. Close the app before `export` / `job`.

| Group | Commands |
|---|---|
| Library | `status`, `media`, `concepts`, `scenes MEDIA`, `transcript MEDIA`, `mark MEDIA [--favorite on|off] [--exclude on|off]`, `job KIND [JSON]` |
| Plans | `styles`, `understand TEXT`, `plan P --request "…"`, `plan P --style ID [--from --to --target --prefer --less --avoid]`, `show-plan P`, `apply P [--keep 0,2,1]` |
| Timeline | `timeline P`, `clip-add P MEDIA IN OUT`, `clip-trim CLIP IN OUT`, `clip-remove`, `clip-move CLIP ±1`, `clip-caption CLIP TEXT` (seconds) |
| Output | `options P [--color --transition --transition-ms --fill --fade --normalize --share --volume --title --subtitle --ending --captions --music --music-library --music-volume --duck]`, `sounds [P]`, `export P [--preset]` |

## 16. Measured performance (single runs, indicative, not benchmarks)

Quoted from HARDWARE_VALIDATION, MODEL_PROPOSAL and ADR 0003.

| What | Result |
|---|---|
| Encode 60 s clip, fps | libx264 196.8 · h264_qsv 339.5 · hevc_qsv 373.4 · full-HW H.264 380.5 |
| Decode HEVC 10-bit, 60 s, fps | software 360.0 · hevc_qsv 784.2 |
| OpenVINO test MLP (compile / avg infer) | CPU 0.041 s / 2.339 ms · GPU 2.599 s / 0.683 ms · NPU 0.517 s / 1.827 ms |
| Worker orchestration | handshake ~430–640 ms warm, cancel ~50 ms, crash restart ~440–510 ms |
| Tauri release | exe 4.2 MB, working set 26.4 MB |
| Proxy, 39.7 s DJI clip | QSV full 4.4 s · software + h264_qsv 24.4 s · libx264 21.9 s |
| Export 20 s segment | hybrid 7.0 s vs software 28.8 s |
| App end-to-end export | 11.4 s, 162.5 MB (~29 Mbit/s at 1080p) |
| Analysis | 5 min proxy (8 975 frames) in 35 s (~256 fps); 6 clips (~7.5 min) in ~51 s |
| OpenCLIP image encoder, batch 1 (compile / per frame) | CPU 321 ms / 22.6 ms · GPU 1932 ms / 3.8 ms · NPU 7667 ms / 11.1 ms |
| Tagging | 150 samples in 13 s on GPU |
| Whisper, 299 s clip, 8 threads CPU | base 26.4 s · small 53.0 s · small + VAD 33 s |
| Whisper smoke test | 39.7 s Vietnamese clip, base, 4 threads → 11.2 s, language detected as vi |

## 17. Build, run, test

| Task | Command (directory) |
|---|---|
| Dev run | `npm run tauri dev` (`apps/desktop`) |
| Release build, no installer (app + `memora-cli.exe`) | `npm run tauri build -- --no-bundle` (`apps/desktop`) |
| Rust tests (86 at `9ac182c` + uncommitted changes, 3 manual ones ignored) | `cargo test` (`apps/desktop/src-tauri`) |
| Plans on a real library (manual) | `MEMORA_DB=<copy of memora.db> cargo test real_library_plans -- --ignored --nocapture` |
| CLI on a copy of the data folder | `MEMORA_DATA_ROOT=<copy> memora-cli …` |
| Worker self-checks | `python analysis.py`, `python tagging.py` (`workers/ai-worker`, using its venv) |
| Frontend type check | `npm run build` (runs `tsc` then `vite build`) |

UI screenshots: capture the app window only (PrintWindow), never the whole screen.

## 18. Known limitations, open items, pending decisions

**Limitations**
- Scoring thresholds, `MAX_CONCEPT_SHARE` 35 % and the timeouts are assumptions.
- No subject-aware or per-clip manual crop (only centre crop or blurred background).
- HDR is not detected or tone-mapped (audit M1).
- Moved / renamed originals cannot be relinked (audit R1).
- One transition length for the whole timeline, clamped by the shortest clip (audit E4).
- Speech-safe cuts can stretch clips to 3× the style maximum (owner rule; open question for low-speech styles).
- The request box only understands keywords (section 15.1).
- Exports are ~29 Mbit/s at 1080p.
- FFmpeg renders have no timeout (only ffprobe / loudness measurement do).
- The job list is in memory only.
- The worker is not stopped when idle (audit A4: measure first).
- No automated test for download cancel.

Audit status: [IMPROVEMENT_AUDIT.md](IMPROVEMENT_AUDIT.md).

**Open (Phase 7, needs approval)**
- Packaging / installer: FFmpeg GPL licence review first, embedded Python worker.
- Bundled music, subject-aware crop, text animation, GPS, month/year grouping, Whisper GPU experiment.

**Decided**
- OpenVINO telemetry: opted out 2026-11-12 (owner left it to the assistant): consent file `%LOCALAPPDATA%\Intel Corporation\openvino_telemetry` = `0`. Only the dev-time model converter (`ov.convert_model`, `tools/clip-export`) uses it; the app's inference does not.
- Spike artefacts deleted (2026-11-12 clean-up).

## 19. Doc / code discrepancies found (not fixed)

| # | Where | Discrepancy |
|---|---|---|
| 1 | MODEL_PROPOSAL, ADR 0004 vs `tools/clip-export/export_openclip.py` | Docs say 49 bilingual tags; code has 48 + 4 negatives |
| 2 | `models.json` | `openclip-vit-b32` has `validated: false`, though CPU/GPU/NPU measurements are recorded |
| 3 | `apps/desktop/src-tauri/src/transcribe.rs:61` | `ponytail:` comment says VAD is "pending model approval"; VAD is implemented and used |
| 4 | `apps/desktop/src-tauri/src/editing.rs` | Style text constants `NO_MUSIC` / `NO_TITLES` say music and titles are unavailable; export implements both |
| 5 | `docs/PROGRESS.md` | The "Not done yet / next steps" block is stale (model proposal and Phase 6 are already done) |
| 6 | `docs/HARDWARE_VALIDATION.md` §6 | OpenCLIP and whisper.cpp still listed as "Unknown (Phase 5)"; numbers are in MODEL_PROPOSAL §4 |
