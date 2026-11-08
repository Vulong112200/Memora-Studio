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

#### 2026-11-08 — model proposal approved, downloads done
See `docs/MODEL_PROPOSAL.md` §4: whisper.cpp b5454 runtime installed in `<data root>\runtime\whisper.cpp\b5454\`, Whisper base/small verified and pre-staged, OpenCLIP checkpoint verified in `downloads\staging`. OpenCLIP exported to OpenVINO IR + fixed bilingual tag embeddings (`tools/clip-export/`), verified vs PyTorch, checksums in `models.json`. Next: install both models in the app, then the whisper.cpp transcription job and the OpenCLIP scene-tagging job.

#### Slice 5.2 Transcription (done, awaiting review)
- `transcribe.rs`: job `media.transcribe` (params ids, model whisper-base|whisper-small, language auto|vi|en): audio from the ORIGINAL → 16 kHz mono WAV in `cache\audio` (deleted after) → `whisper-cli.exe` (`<data root>\runtime\whisper.cpp\b5454\`, Job Object, `-pp` progress, cancel kills it) → migration 6 `transcripts` (segments JSON, cascades with media, cleared when the original changes). Files without audio are stored as "no speech".
- Hallucination mitigation from real footage: `-mc 0 -sns` + drop runs of ≥4 identical segments. Silero VAD proposed (MODEL_PROPOSAL §4), not downloaded.
- UI: Media library "Speech" column, model picker + "Transcribe N videos", transcript list in the preview (click → seek + In/Out).
- Tests: parse/store/cascade, loop filter, missing model, real whisper.cpp end-to-end incl. cancellation (44 Rust tests).
- Next: OpenCLIP scene tagging job (slice 5.3).

#### Slices 5.3–5.4, Phase 6 and first Phase 7 items (done, awaiting review) — decisions in ADR 0004
- 5.3 `tagging.rs` + worker `tagging.py`: job `media.tag`, OpenCLIP IR every 2 s of the proxy (GPU → CPU fallback), migration 7 `media_tags`; tags per clip and per scene in the UI; library search by concept (Vietnamese without diacritics works). Real proxy: 150 samples in 13 s on GPU.
- 5.4 Silero VAD (`silero-vad` in models.json, verified SHA-256, staged in `models\.downloads`) used automatically by transcription when installed.
- Phase 6 `editing.rs`: five styles, candidate windows, separate scores + reasons, diversity/duplicates/exclusions, story structure, deterministic plan stored per project (migration 8 `editing_plans`, `media_marks`). UI "Smart edit": style + date range → plan review (drop / reorder) → Apply to timeline / Apply and export. Library ★ favorite / ⊘ exclude.
- Export: `fit: fill` centre crop for vertical output, `transitionMs` crossfades; Timeline export has both options.
- Verified: 54 Rust tests (real FFmpeg crossfade + fill, real worker tagging with the exported IR, real whisper.cpp with VAD), `python tagging.py`, and the debug app on the real library (Smart edit generated a 20-clip Travel Cinematic plan from 96 candidates).

### Still open (owner decisions or later work)
- Packaging / installer: needs the FFmpeg GPL licence review before any distribution and an embedded Python runtime (ADR 0002); not built.
- Background music (no licensed offline source), burned-in titles (libass/freetype validation), subject-aware crop, GPS/location, month/year grouping in the library, Whisper GPU experiment.
- Offline: no network use except the Model Manager's explicit, user-confirmed downloads (all models are installed locally now).

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
