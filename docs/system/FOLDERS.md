# Folder structure

Last verified: 2026-11-09 against commit `9ac182c` + uncommitted changes. Index: [README.md](README.md). Kept current by the `sync-docs` skill.

## 1. Repository (`D:\Memora Studio\`)

Code and documents only — never user data or media (data lives under the data root, section 2).

```
Memora Studio/
├─ CLAUDE.md                     rules for Claude Code + status (read first)
├─ .claude/
│  ├─ settings.json              permission allow/deny rules (no installs, no env/registry changes)
│  ├─ commands/phase-0-discovery.md   slash command used in Phase 0
│  └─ skills/sync-docs/          skill + checker that keep docs/system in step with the code
├─ apps/desktop/                 THE APP (Tauri 2)
│  ├─ index.html, vite.config.ts, tsconfig*.json, package.json, package-lock.json
│  ├─ src/                       React + TypeScript frontend            → FRONTEND.md
│  │  ├─ main.tsx, App.tsx, api.ts, index.css
│  │  └─ Projects.tsx, EditingPlan.tsx, Timeline.tsx, MediaLibrary.tsx, PhotoEditor.tsx, AiCenter.tsx, Models.tsx, ui.tsx, i18n.ts
│  ├─ public/                    (empty)
│  ├─ dist/                      frontend build output (generated)
│  └─ src-tauri/                 Rust backend                            → BACKEND.md
│     ├─ Cargo.toml, Cargo.lock, build.rs, tauri.conf.json
│     ├─ capabilities/default.json   Tauri permissions (core:default, dialog:allow-open)
│     ├─ icons/                  app icons
│     ├─ models.json             AI model manifests (embedded)
│     ├─ sounds.json             free music library catalog (embedded)
│     ├─ src/                    library crate `memora_desktop_lib`: one file per module
│     │  └─ bin/memora-cli.rs    command-line tool entry point
│     ├─ gen/                    Tauri generated schemas
│     └─ target/                 cargo build output (debug/, release/memora-desktop.exe + memora-cli.exe)
├─ workers/ai-worker/            Python AI worker                        → WORKER.md
│  ├─ memora_worker.py           protocol v1 + job table
│  ├─ analysis.py, tagging.py, photo.py   algorithms (each has a self-check: `python <file>.py`)
│  ├─ requirements.lock.txt      pinned packages
│  └─ .venv/                     dev runtime (not committed)
├─ tools/clip-export/            dev-only OpenCLIP → OpenVINO IR exporter (torch venv, not shipped)
├─ spikes/                       throwaway Phase 1b experiments (not product code)
└─ docs/
   ├─ system/                    THIS documentation set (code-level)
   ├─ APP_OVERVIEW.md            one-file summary
   ├─ PROGRESS.md                day log
   ├─ IMPROVEMENT_BACKLOG.md, IMPROVEMENT_AUDIT.md
   ├─ Production.md, Memora_Studio_Phase_0_Technical_Spec_and_Claude_Prompt.md   specifications
   ├─ DISCOVERY_REPORT.md, DEPENDENCY_MATRIX.md, HARDWARE_VALIDATION.md, MODEL_PROPOSAL.md
   └─ adr/0001…0005-*.md         architecture decisions
```

`.gitignore` keeps models (`*.bin`, `*.onnx`), media, `*.db`, `.venv*`, `target/`, `node_modules/` out of Git.

## 2. Data root (owner: `E:\Memora Data\`)

Created by `settings::prepare_data_root`; everything the app writes lives here.

```
<data root>\
├─ database\
│  ├─ memora.db (+ -wal, -shm)             SQLite                                   → DATABASE.md
│  └─ backups\memora-<UTC>-v<schema>-<label>.db   verified copies, last 10 kept (startup | before-upgrade | manual)
├─ logs\memora.log.YYYY-MM-DD              daily JSON-lines logs
├─ runtime\
│  ├─ ffmpeg\                              optional bundled ffmpeg/ffprobe (else PATH / WinGet)
│  ├─ whisper.cpp\b5454\                   whisper-cli.exe, whisper.dll, ggml*.dll
│  └─ ai-worker\python\, app\              packaged worker (future installer; dev uses the repo venv)
├─ models\
├─ music\<track id>.mp3                    free library music, downloaded once and kept (verified on every export)
│  ├─ <category>\<id>\<version>\           installed models (vision\openclip-vit-b32\2\, speech\whisper-small\1\ …)
│  └─ .downloads\<id>-<version>\           resumable downloads (*.part), kept after a crash
├─ cache\                                  the ONLY folder the webview may load files from
│  ├─ thumbs\<media id>.jpg                thumbnails (180 px short side)
│  ├─ proxy\<media id>.mp4                 540p preview proxies (H.264, ≤ 30 fps)
│  ├─ frames\<media id>\<scene>.jpg        scene representative frames
│  ├─ audio\<media id>.wav|.json           temporary whisper input/output (deleted after each file)
│  ├─ render\titles-<pid>-<nanos>.ass      temporary title/subtitle files (deleted after render)
│  ├─ render\part-<pid>-<nanos>-<i>.mp4    parts of a long export (> 10 clips), deleted after render
│  ├─ downloads\sounds\                    old location of library music (swept at start-up; no longer written)
│  └─ photo\<photo id>\thumb.jpg | before.jpg | preview-<n>.jpg   photo editor thumbnails and previews
├─ photos\exports\<name>-edit[-N].jpg      edited photos (never overwritten; written as .jpg.part, then renamed)
├─ projects\<project id>\exports\
│  ├─ <name>-<preset>-<unix>[-N].mp4       finished exports (never overwritten)
│  ├─ …-failed-validation.mp4              kept, but never under the normal name
│  └─ *.unvalidated.mp4, *.part, *.level.mp4   in-progress files (removed at next start if left over)
└─ test-media\                             owner samples + generated portrait clips (dev)
```

Temporary files are removed at start-up by `proxy::remove_leftovers` (see [FLOWS.md](FLOWS.md) §1). Original videos stay wherever the user keeps them; the database stores their absolute paths.
