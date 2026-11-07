# Progress log

## 2026-11-07 — end of day

### Done (all verified with tests and/or the real app)

| Phase | Status | Evidence |
|---|---|---|
| 0 Discovery | Done | `docs/DISCOVERY_REPORT.md`, ADR 0001 |
| 1a Environment | Done | `docs/DEPENDENCY_MATRIX.md` (Rust, FFmpeg Gyan GPL, Python 3.13 venv) |
| 1b Feasibility spikes | Done | `docs/HARDWARE_VALIDATION.md`, ADR 0002 (embedded Python worker) |
| 2 Desktop foundation | Done | Tauri 2 + React shell, settings/data root, JSON logs, hardware info, SQLite + migrations, projects, job manager, AI worker lifecycle (Job Object, restart, protocol v1) |
| 3 Media Engine | Done | Import + FFprobe metadata, thumbnails, proxies (full QSV / hybrid / libx264), preview player, timeline (trim/concat), export presets rendered from originals, §8.3 validation |
| 4 Classical analysis | Done | ADR 0003: worker analyses proxies (PySceneDetect, OpenCV metrics, shake, dHash); Rust deterministic technical scoring + duplicate flags; scene strip in preview |
| 5 Local AI | **In progress** | Slice 5.1 Model Manager done (see below). No model downloaded, torch not installed. |

Data root moved to `E:\Memora Data\` (ADR 0001 amendment). `D:\MemoraStudio\` still exists as a verified copy — the owner deletes it manually (the agent's tool blocks deleting that path).

Tests: 40 Rust tests (unit + real FFmpeg + real Python worker integration) and `python workers/ai-worker/analysis.py` self-check — all passing at end of day.

#### Slice 5.1 Model Manager (done, awaiting review)
- `apps/desktop/src-tauri/models.json`: manifests for OpenCLIP ViT-B/32, Whisper base, Whisper small. **No URL/checksum filled → downloads disabled** with an explanation in the UI.
- `models.rs`: install pipeline download (Windows built-in `curl.exe`, resumable) or offline copy → size + SHA-256 check → install to `<data root>\models\<category>\<id>\<version>\` → load validation → state in SQLite (migration 5). Removal. States per docs/Production.md §4.3.
- Worker job `validate_openvino_ir`: compiles an IR on CPU (or listed devices) and runs one inference.
- UI card "AI models": state, Download (confirmation with name/size/license/source), Install from files…, Remove, Cancel.
- Tests: real `file://` download through curl.exe, checksum failure, offline install, load-validation failure, and an offline OpenVINO-IR install validated by the real worker.

### Not done yet / next steps

1. **Model proposal (needs owner approval before any download):** for each model the official source, pinned revision, license, size and SHA-256:
   - OpenCLIP ViT-B/32 (checkpoint candidate `laion2b_s34b_b79k`) — license review; export to OpenVINO IR needs **dev-time torch in a separate venv (separate approval)**; output-consistency check vs PyTorch CPU.
   - whisper.cpp (pinned release, CPU build) + Whisper base multilingual (ggml); small on demand.
2. After approval: fill models.json, OpenCLIP embedding job (scene frames → embeddings cache, text search), whisper.cpp transcription job (Vietnamese + English test coverage), CPU/GPU benchmarks, optional NPU experiment.
3. Phase 6 Editing Intelligence (needs approval): split long continuous scenes into candidate windows (all DJI samples are single long shots), clip selection, presets, Editing Plan review UI.
4. Phase 7 Product completion (needs approval).

### Known limitations carried forward
- Score thresholds in `scoring.rs` are initial assumptions; tune on real libraries.
- Export letterboxes mismatched aspect ratios (no smart crop); HDR (PQ/HLG) not tone-mapped; export bitrate generous (~29 Mbit/s at 1080p).
- No FFprobe/FFmpeg timeout for hung processes; job list is in memory only; worker is not stopped when idle.
- Download cancellation (curl kill) is not covered by an automated test.
- Packaging: `analysis.py` must ship next to `memora_worker.py`; installer not built yet; FFmpeg GPL licence must be revisited before distribution.
- Pending owner decisions: OpenVINO telemetry opt-out, deleting spike artifacts (~1.9 GB in repo build dirs, ~1.6 GB in `E:\Memora Data\cache\spike`), UI language (English now).

### How to resume
- Read `CLAUDE.md` (status + rules) and this file.
- Tests: `cargo test` in `apps/desktop/src-tauri`; build: `npm run tauri build -- --no-bundle` in `apps/desktop`.
- Recreate dev venv: `py -3.13 -m venv workers\ai-worker\.venv` then `pip install --no-deps -r workers\ai-worker\requirements.lock.txt` (inside the venv).
- Frontend deps: `npm ci` in `apps/desktop`.
