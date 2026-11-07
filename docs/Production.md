# Memora Studio
## Technical Specification & Implementation Blueprint

**Version:** 1.2 (Phase 0 decisions applied: data root on D:, roadmap aligned with Phase 0 spec, OpenVINO runtime for OpenCLIP, FFmpeg GPL build, Python 3.13 worker, memory policy)  
**Target platform:** Windows 11 x64  
**Reference development machine:** ASUS Zenbook 14 UX3405CA — Intel Core Ultra 9 285H, Intel Arc 140T iGPU, Intel AI Boost NPU, 32 GB RAM, 1 TB NVMe SSD (see §3.1)  
**Operating mode:** Online + Offline, Local-first  
**Development assistant:** Claude Code (Claude Team subscription)  
**Core principle:** The application must remain fully usable without any cloud AI service.

> **Implementation gate:** Phase 0 (Review & Discovery) is complete — see `docs/DISCOVERY_REPORT.md` and `docs/adr/0001-phase0-decisions.md`. Each further phase (§16) starts only after explicit approval by the project owner.

---

# 1. Product Vision

Memora Studio is a local-first desktop application for automatically organizing, analyzing, editing, and exporting personal videos.

Primary use cases:

1. Automatically edit daily-life videos.
2. Create travel videos and social media stories.
3. Generate short highlight videos from long trips.
4. Create six-month and yearly memory recap videos.
5. Organize personal video libraries by date, event, and location.

The product must provide more than basic clip concatenation. It should identify useful moments, remove redundant footage, build a coherent sequence, select music, control pacing, and produce polished videos.

## 1.1 Core requirements

- No mandatory cloud AI.
- No mandatory Claude subscription at runtime.
- Original video files must never be modified.
- All editing decisions must be stored in a project.
- Video rendering must use deterministic processing.
- AI analysis must be optional and replaceable.
- All model downloads must be user-controlled.
- Offline mode must support previously installed models.
- Export must remain available even when AI models are unavailable.

# 2. Architecture Decisions

## 2.1 Selected technology stack

| Layer | Technology | Status |
|---|---|---|
| Desktop shell | Tauri 2 | Selected — integration validated in Phase 1b |
| Frontend | React + TypeScript | Selected |
| UI styling | Tailwind CSS | Selected |
| Backend orchestration | Rust | Selected |
| Video processing | FFmpeg + FFprobe | Selected — GPL build (Gyan) for development and internal use; license review before any distribution |
| Image processing | OpenCV | Selected |
| Scene detection | PySceneDetect | Selected |
| Visual understanding | OpenCLIP | **Candidate** — pending license, conversion and benchmark review |
| Speech recognition | whisper.cpp | Selected (CPU baseline) |
| AI acceleration | OpenVINO | **Optional runtime**, chosen per model after benchmark |
| Database | SQLite | Selected |
| Data serialization | JSON | Selected |
| Local AI worker | Python 3.13, isolated venv | Selected — packaging strategy decided in Phase 1b (§13) |
| Logging | Structured JSON logs | Selected |
| Testing | Vitest + Rust tests + Python pytest | Selected |

Do not introduce cloud-dependent infrastructure into the core application.

## 2.2 High-level architecture

```text
┌──────────────────────────────────────────────────────────┐
│                React + TypeScript UI                     │
│     Dashboard · Library · Editor · AI Center · Export    │
└────────────────────────────┬─────────────────────────────┘
                             │ Tauri commands / events
┌────────────────────────────▼─────────────────────────────┐
│                 Tauri + Rust Backend                     │
│    Project · Jobs · Storage · Resource Manager           │
└──────┬──────────────┬───────────────┬──────────────┬─────┘
       │              │               │              │
┌──────▼──────┐ ┌─────▼───────┐ ┌─────▼───────┐ ┌────▼────────┐
│ Media Engine│ │ AI Worker   │ │ Editing Core│ │ Model       │
│             │ │ (process)   │ │             │ │ Manager     │
│ FFmpeg      │ │ OpenCV      │ │ Scoring     │ │ Download    │
│ FFprobe     │ │ OpenCLIP    │ │ Rules       │ │ Verify      │
│ Quick Sync* │ │ Whisper     │ │ Storytelling│ │ Install     │
└──────┬──────┘ └─────┬───────┘ └─────┬───────┘ └────┬────────┘
       │              │               │              │
┌──────▼──────────────▼───────────────▼──────────────▼─────┐
│                     Local Storage                        │
│   SQLite · Original Media (read-only) · Proxy · Models   │
│   · Cache                                                │
└──────────────────────────────────────────────────────────┘

* Quick Sync is used only after runtime capability validation (§8.4).
```

## 2.3 Architectural rules

1. Frontend must not directly execute FFmpeg commands.
2. Frontend must not directly load AI models.
3. Rust backend owns process orchestration, including the AI Worker lifecycle.
4. AI Worker owns model inference.
5. Media Engine owns video decoding, encoding, and rendering.
6. Editing Core owns editing decisions.
7. Model Manager owns downloading, verifying, installing, and removing models.
8. All long-running tasks must support progress reporting and cancellation.
9. Every AI module must have a non-AI fallback where practical.
10. No module may assume that an Intel GPU or NPU is always available.
11. AI Model, Inference Runtime, and Execution Device are separate concepts (§2.4). No module may hardcode a model–runtime–device combination.

## 2.4 Model / Runtime / Device separation

Three independent concepts:

| Concept | Examples | Owned by |
|---|---|---|
| **AI Model** | OpenCLIP ViT-B/32, Whisper base, Whisper small | Model Manager (files, versions, licenses) |
| **Inference Runtime** | OpenVINO, whisper.cpp (PyTorch only at development time for model export) | AI Worker / native runtime |
| **Execution Device** | Intel CPU, Intel Arc GPU, Intel NPU | Resource Manager (detection, validation) |

Rules:

- Each model declares which runtimes it supports; each runtime declares which devices it supports.
- The selected combination is a **configuration result** from capability detection and benchmarks, not a code path.
- OpenVINO is one runtime among several. Do not force every model through OpenVINO. Each model uses the runtime that works best and is validated on the target machine (e.g. OpenCLIP → OpenVINO; Whisper → whisper.cpp).
- Editing Core consumes only normalized analysis results (`observation`, `confidence`, `modelId`, `modelVersion`, `runtime`, `device`). Changing a model or runtime must not require changes to Editing Core.

# 3. Local AI Strategy

## 3.1 Verified hardware profile

Collected from the reference development machine:

| Component | Value |
|---|---|
| Laptop | ASUS Zenbook 14 UX3405CA |
| CPU | Intel Core Ultra 9 285H (Arrow Lake-H, Core Ultra Series 2), 16 cores: 6 P + 8 E + 2 LP E, 16 threads |
| GPU | Intel Arc 140T integrated, 8 Xe-cores |
| GPU dedicated memory | 128 MB |
| GPU shared memory | 18,307 MB (system RAM, **not** VRAM) |
| NPU | Intel AI Boost (13 TOPS nominal) |
| Graphics driver | 32.0.101.8860 |
| NPU driver | 32.0.100.4023 |
| RAM | 32 GB |
| SSD | Micron NVMe, 1 TB |
| OS | Windows 11 Home Single Language, Build 26300 |

Interpretation rules:

- **Shared GPU memory is not dedicated VRAM.** GPU workloads consume system RAM and compete with the application, the AI Worker, and FFmpeg. Memory budgets must count GPU allocations against total system RAM.
- **NPU presence in Windows is not proof of a working inference backend.** The NPU is treated as unverified until OpenVINO device discovery, model compilation, and an inference smoke test succeed (§3.4).
- **Windows build 26300** must be recorded in the hardware validation report. Driver/runtime compatibility on this build is validated, not assumed.
- Hardware values above are the reference machine only. The application still performs runtime capability detection on every machine (§9.3).

## 3.2 Runtime and device selection

Selection is per model, not global:

1. Detect available devices and runtimes (§9.3).
2. For each installed model, try the validated combinations in the order recorded in its manifest (§4.2), for example:
   - OpenCLIP: OpenVINO GPU → OpenVINO CPU.
   - Whisper: whisper.cpp CPU (baseline) → experimental GPU path only if validated.
3. A combination is enabled only after it passes a load test and an output-consistency check against the CPU reference.
4. If no AI combination is available, use rule-based / classical computer vision fallback.

Never make the application depend on successful GPU or NPU initialization.

## 3.3 Model selection

### Model A: OpenCLIP ViT-B/32 (candidate, not yet locked)

Purpose:
- Understand visual content.
- Classify scenes using text prompts.
- Identify broad concepts such as beach, mountain, food, city, nature, architecture, and people.
- Support semantic clip selection.

Candidate:
- Architecture: ViT-B/32, OpenCLIP family.
- Candidate checkpoint: `laion2b_s34b_b79k` — **not final**.

Runtime decision (ADR 0001): the shipped runtime is **OpenVINO** (CPU by default, GPU after validation). PyTorch is used only at development time to export the model to ONNX / OpenVINO IR and as the reference for consistency checks; it is not shipped in the worker.

Before this model is locked in (Phase 5), the following must be produced:

1. **License review** of code and checkpoint weights, including redistribution terms and training-data terms.
2. **Conversion feasibility**: export to ONNX, then OpenVINO IR; record any unsupported operators.
3. **Benchmark** on the reference machine: OpenVINO CPU vs OpenVINO GPU (PyTorch CPU as a development-time reference only) — latency per batch, throughput, peak RAM, model load time.
4. **Output consistency**: embedding cosine similarity between runtimes against the PyTorch CPU reference on a fixed image set.
5. Recorded download size and checksum from the official source.

If the license or conversion review fails, evaluate an alternative checkpoint or model and document the decision.

Implementation rules:
- Do not analyze every frame.
- Analyze representative frames only.
- Process small batches.
- Cache embeddings.
- Allow users to define custom semantic categories.
- Do not use similarity scores as absolute truth.
- Store confidence, model version, runtime, and device with every result.

OpenCLIP must be optional. If unavailable, the application must continue with metadata, image metrics, and user-defined rules.

### Model B: Whisper multilingual

Purpose:
- Transcribe speech.
- Detect speech segments.
- Generate subtitles.
- Help identify meaningful spoken moments.

Runtime:
- whisper.cpp.
- **CPU is the guaranteed baseline and is validated first.**
- GPU acceleration (OpenVINO encoder or other whisper.cpp GPU backends) is a **separate experiment**. It has not been proven on the reference machine and must not block the CPU pipeline.

Model policy:

| Model | Purpose | Recommendation |
|---|---|---|
| Whisper base multilingual | Initial transcription | Default |
| Whisper small multilingual | Better transcription quality | Optional, downloaded on demand |
| Quantized variants | Reduce memory usage | Preferred where quality remains acceptable |

Do not automatically install both models. Start with base. Allow small to be downloaded later when users need improved transcription.

Vietnamese and English must be included in test coverage.

Integration decision (ADR 0001): whisper.cpp runs as a separate native process controlled by the Rust backend, not inside the Python AI Worker. CPU and GPU variants are separate binaries under `runtime/whisper/<backend>/`.

### Model C: OpenCV

Purpose:
- Brightness and exposure analysis.
- Blur and sharpness detection.
- Black frame detection.
- Motion estimation.
- Basic image quality analysis.
- Perceptual duplicate detection support.

OpenCV is a core dependency and must work without any downloaded AI model.

### Model D: PySceneDetect

Purpose:
- Detect scene boundaries.
- Divide long videos into smaller shots.
- Identify changes in visual content.
- Provide candidate segments for later analysis.

Use FFmpeg for decoding and video splitting. PySceneDetect must work without any downloaded AI model.

### Model E: Optional MediaPipe

Purpose:
- Basic person or face presence detection.
- Support framing and composition analysis.

Not required for the first MVP. Do not implement face recognition or identity matching.

## 3.4 NPU policy

The NPU is an **optional, experimental** execution device.

Phase 1b checks, without installing or modifying system drivers:

- NPU driver version (recorded: 32.0.100.4023).
- OpenVINO NPU plugin availability.
- OpenVINO device discovery (`NPU` listed or not).
- Model compilation and operator compatibility for each candidate model.
- Actual inference performance vs CPU and GPU.

NPU support is never a release requirement. If any check fails, the NPU is marked unavailable and nothing else changes.

## 3.5 No Local LLM in MVP

Do not install Ollama, Llama, Qwen, or another general-purpose LLM in the initial release.

Reason:
- Core editing tasks do not require a general-purpose language model.
- An LLM adds memory pressure and setup complexity.
- The Editing Core and Rule Engine should remain deterministic.

A Local LLM may be considered in a later release for natural-language editing instructions.

# 4. AI Model Manager

Create a dedicated module named `model-manager`.

## 4.1 MVP scope

The MVP implements only:

- The **model manifest** format (§4.2) and verified manifests for approved models.
- The **install pipeline** in the backend: download (with resume) → SHA-256 verify → install into versioned directory → load validation → smoke test.
- Offline installation from a local file.
- Removal of an installed model.
- A minimal status view (state, size, error, retry/cancel/remove) in AI Center.

Deferred until after MVP:

- Full model browsing/catalog UI.
- Automatic update checks.
- Compatibility scoring UI beyond the detected runtime/device list.

Every download requires explicit user confirmation showing the model name, size, license, and source.

## 4.2 Model manifest

Each model must have a manifest. Runtimes and devices are listed per model (§2.4).

Example schema:

```json
{
  "id": "openclip-vit-b32",
  "name": "OpenCLIP ViT-B/32",
  "version": "1.0",
  "category": "vision",
  "required": false,
  "artifacts": [
    {
      "format": "onnx",
      "downloadSource": "OFFICIAL_MODEL_SOURCE",
      "revision": "PINNED_REVISION",
      "checksumSha256": "REQUIRED_SHA256",
      "sizeBytes": null
    }
  ],
  "runtimes": [
    { "runtime": "openvino", "devices": ["GPU", "CPU"], "validated": false }
  ],
  "license": "VERIFY_BEFORE_DISTRIBUTION",
  "minimumRamGb": 8
}
```

The example is a schema, not a real downloadable manifest. Claude Code must fill in verified URLs, revisions, licenses, checksums, and sizes before enabling downloads. `validated` becomes `true` only from recorded benchmark/smoke-test results.

Never trust an unverified model URL or checksum.

## 4.3 Model states

```text
NOT_INSTALLED
      |
      v
DOWNLOADING
      |
      v
VERIFYING
      |
      v
INSTALLING
      |
      v
VALIDATING
      |
      v
READY
```

Failure states:

```text
DOWNLOAD_FAILED
CHECKSUM_FAILED
INSTALL_FAILED
MODEL_LOAD_FAILED
INCOMPATIBLE
```

The UI must allow retry, cancel, and remove where appropriate.

## 4.4 Storage layout

The application binaries live in the installation directory (default `C:\Program Files\Memora Studio\`, a few hundred MB). All data lives under a **user-selectable data root**. On the reference machine the data root is `E:\Memora Data\` (ADR 0001, amended): drive C: has ~82 GB free; drive E: is the owner's data drive (D: is reserved for software). The data root must be changeable in settings; nothing requires data on C:.

```text
<DataRoot>\                     e.g. E:\Memora Data\
├── runtime/
│   ├── ffmpeg/
│   ├── whisper/<backend>/
│   └── ai-worker/<version>/
├── models/
│   ├── vision/
│   │   └── openclip-vit-b32/
│   ├── speech/
│   │   ├── whisper-base/
│   │   └── whisper-small/
│   └── optional/
├── projects/
├── cache/             proxy/, frames/, thumbs/ — size-capped, LRU eviction
├── logs/
├── database/
└── test-media/        development only: user-provided, non-private samples
```

Do not store models inside the application installation directory.

Application upgrades must not require model downloads again.

# 5. Video Analysis Pipeline

## 5.1 Processing stages

```text
IMPORT
  |
  v
METADATA EXTRACTION
  |
  v
SCENE DETECTION
  |
  v
FRAME SAMPLING
  |
  v
TECHNICAL ANALYSIS
  |
  v
VISUAL SEMANTIC ANALYSIS
  |
  v
OPTIONAL AUDIO ANALYSIS
  |
  v
CLIP SCORING
  |
  v
EDITING PLAN
```

## 5.2 Metadata extraction

Use FFprobe to extract:

- Duration.
- Width and height.
- Frame rate.
- Codec.
- Audio streams.
- Creation timestamp where available.
- Rotation/orientation.
- File size.
- Video bitrate.

Preserve original metadata in the media database.

## 5.3 Frame sampling

Do not feed complete videos into AI models.

Recommended initial strategy:

- Analyze frames near scene boundaries.
- Analyze one representative frame per scene.
- Add extra samples for unusually long scenes.
- Use a configurable sampling interval.
- Avoid processing nearly identical adjacent frames.

Allow users to configure analysis quality:

- Fast.
- Balanced.
- Detailed.

## 5.4 Technical scoring

Create independent metrics:

| Metric | Meaning |
|---|---|
| Sharpness | Image clarity |
| Brightness | Overall brightness |
| Exposure | Overexposure and underexposure |
| Contrast | Visual contrast |
| Blur | Motion or focus blur |
| Camera shake | Motion instability |
| Black frame | Invalid or blank visual segment |
| Duplicate similarity | Redundant footage |

Do not automatically discard a clip because it is blurry or shaky.

A technically imperfect clip may contain an important memory.

# 6. Editing Core and Rule Engine

This is the most important application-specific module.

AI models provide observations. They must not directly control rendering.

The Editing Core converts analysis results into an explicit Editing Plan. It is independent of which model, runtime, or device produced the observations (§2.4).

## 6.1 Scoring architecture

Separate technical quality from emotional and storytelling value.

```text
Technical Score
      |
      v
Visual Score
      |
      v
Semantic Score
      |
      v
Memory Value
      |
      v
Storytelling Score
```

Do not combine these into a single score too early.

Example:

A blurry video showing a birthday celebration may have low technical quality but high memory value.

## 6.2 Clip selection

The Rule Engine should consider:

- Visual quality.
- Semantic relevance.
- Scene diversity.
- Duplicate content.
- Duration.
- Camera movement.
- Audio relevance.
- Event importance.
- User favorites.
- User exclusions.

The user must be able to override every automated decision.

## 6.3 Editing styles

Implement these presets:

1. Travel Cinematic.
2. Social Story/Reels.
3. Memory/Nostalgia.
4. Travel Documentary.
5. Clean Minimal.

Each preset must define:

- Target duration.
- Aspect ratio.
- Clip duration range.
- Transition rules.
- Music behavior.
- Text/title behavior.
- Pacing.
- Opening and ending structure.

## 6.4 Editing modes

- Auto: generate and export an automatic video.
- Smart: generate an Editing Plan and let the user review it.
- Manual Assist: recommend useful clips while the user edits.

Smart mode should be the default for the first release.

# 7. Media Library and Memory Timeline

Create a local media library.

Store:

- Original file reference.
- File hash.
- Date and time.
- Duration.
- Resolution.
- Scene boundaries.
- Technical metrics.
- Semantic tags.
- AI analysis status.
- User favorites.
- Project usage history.

Support:

- Date filtering.
- Month and year grouping.
- Event grouping.
- Manual tags.
- Location metadata where available.
- Search by visual concepts.
- Duplicate detection.

Never require a cloud account.

## 7.1 Long-term memory projects

Users should be able to select:

- Last month.
- Last six months.
- Last year.
- Custom date range.
- Selected trips.
- Selected events.

Generate multiple outputs from the same source library:

- 30-second Story.
- 60-second Reel.
- 3-minute memory video.
- Longer documentary-style video.

# 8. Media Engine and Rendering

Use FFmpeg for all final media processing.

## 8.1 Requirements

- Read original files.
- Create proxy files for preview.
- Trim clips.
- Concatenate clips.
- Normalize audio.
- Apply transitions.
- Add titles and overlays.
- Mix background music.
- Export final video.
- Validate output.

Do not render final output from low-resolution proxies.

## 8.2 Export presets

| Preset | Resolution | FPS | Format |
|---|---|---|---|
| Story | 1080 × 1920 | 30 | MP4 H.264 + AAC |
| Reels | 1080 × 1920 | 30 | MP4 H.264 + AAC |
| Landscape | 1920 × 1080 | 30 | MP4 H.264 + AAC |
| Archive | Preserve source where practical | Source-aware | MP4 |

Do not upscale every video to 4K or force all source footage to 60 FPS.

## 8.3 Export validation

After rendering, verify:

- File exists.
- File can be decoded.
- Duration is valid.
- Resolution is correct.
- Audio is present when expected.
- No unexpected black frames.
- No unexpected audio clipping.
- No missing media segments.

A failed validation must be reported to the user.

## 8.4 FFmpeg build and hardware acceleration validation

Do not assume QSV encoders exist. Availability depends on the exact FFmpeg build, its license configuration, and the installed Intel driver.

Build decision (ADR 0001): the Gyan GPL build (includes QSV, `libx264`, `libass`, `libfreetype`) is used for development and internal use. GPL obligations apply if the binary is ever distributed; an LGPL build (with OpenH264 as software fallback) must be reconsidered before any public or commercial release.

Phase 1b must record:

- FFmpeg/FFprobe version, build source, and `configuration` flags.
- Build license (LGPL vs GPL) and its implications for distribution.
- Output of `ffmpeg -hwaccels`, `ffmpeg -encoders`, `ffmpeg -decoders` filtered for `qsv`.
- An **actual test encode** for each candidate encoder (`h264_qsv`, `hevc_qsv`, `av1_qsv`) on a synthetic source, checking success, speed, and decodability of the output.
- Quick Sync decode test on representative source formats (H.264, HEVC, phone footage with rotation).

Runtime rules:

- At startup (cached per driver/FFmpeg version), the Media Engine probes hardware encoders with a short test encode.
- An encoder is used only if the probe succeeded.
- Software encoding (`libx264` or the license-compatible equivalent) is always available as fallback.
- A failed hardware encode mid-job falls back to software and is logged.

# 9. Resource Management

The reference machine has 32 GB RAM shared between CPU, integrated GPU, and NPU.

## 9.1 Resource policy

- Do not load every model simultaneously.
- Process vision and speech workloads sequentially by default.
- Limit inference batch size.
- Limit CPU worker threads.
- Monitor memory usage, including GPU shared memory.
- Support cancellation.
- Release model memory after long tasks.
- Use cache to avoid repeated inference.
- Use proxy files for editing preview.
- Keep the application responsive during processing.

## 9.2 Hardware profiles

| Profile | Conditions | Behavior |
|---|---|---|
| CPU Safe | GPU unsupported or unvalidated | CPU-only inference |
| Intel Accelerated | OpenVINO GPU validated for a model | GPU acceleration for that model only |
| NPU Experimental | NPU validated for a model, user opt-in | NPU for that model only |
| Low Resource | Memory pressure | Smaller batches, reduced sampling |
| Offline | No network | Installed models only |

Initial memory policy:

- Application UI and backend must remain responsive.
- AI inference must use configurable memory limits.
- Do not reserve all available RAM for AI.
- Prefer one heavy model at a time.
- Treat GPU memory as shared system RAM; the 128 MB dedicated memory is not a usable model budget.
- The reference machine has a fixed 2 GB pagefile, so the commit limit is only about RAM + 2 GB. The application does not change Windows settings (ADR 0001); instead it:
  - monitors commit charge and shared GPU memory, not only physical RAM;
  - enforces a configurable memory budget for the AI Worker;
  - never runs AI inference in parallel with final rendering;
  - loads at most one heavy model at a time.

## 9.3 Hardware detection

Detect:

- CPU model and instruction set.
- RAM.
- GPU model, dedicated and shared memory (separately).
- Graphics driver version.
- NPU presence and driver version.
- Windows version and build.
- Available disk space.
- OpenVINO device list (CPU / GPU / NPU).
- FFmpeg hardware encoder probe results.
- Runtime compatibility per model.

Display the detected devices and the selected runtime/device per model in AI Center.

# 10. AI Provider Abstraction

Implements the separation in §2.4. Do not model OpenVINO as a "provider" alongside OpenCLIP.

```text
ModelProvider            (what is computed)
├── VisionEmbeddingProvider   e.g. OpenCLIP
├── SpeechProvider            e.g. Whisper
└── FutureCloudProvider       disabled by default

InferenceRuntime         (how it is computed)
├── OpenVINORuntime
├── WhisperCppRuntime
└── OnnxRuntime               only if justified by benchmarks

PyTorch is a development-time export tool, not a shipped runtime.

ExecutionDevice          (where it is computed)
├── CPU
├── IntelGPU
└── IntelNPU                  experimental
```

A ModelProvider is bound to one (Runtime, Device) pair at load time, chosen by §3.2.

Every ModelProvider must expose:

- Availability.
- Model version.
- Active runtime and device.
- Health check.
- Inference.
- Progress.
- Cancellation.
- Error information.

The application must not fail because a provider, runtime, or device is unavailable.

Cloud providers must remain disabled by default.

# 11. Offline and Online Behavior

## 11.1 Online mode

Online mode may be used for:

- Downloading models (with user confirmation).
- Downloading application updates.
- Checking model updates.
- Optional cloud AI features in future versions.

## 11.2 Offline mode

Offline mode must support:

- Importing media.
- Metadata extraction.
- Scene detection.
- OpenCV analysis.
- Installed local model inference.
- Editing Plan generation.
- Preview.
- Rendering.
- Export.
- Project management.

No cloud authentication may be required for these features.

If a model is not installed, explain which features are unavailable and continue using supported fallbacks.

# 12. Security and Privacy

- Original media files must remain untouched.
- Do not upload media automatically.
- Do not upload frames or audio to cloud services.
- Do not collect personal video metadata for analytics.
- Restrict file access to user-selected locations.
- Validate downloaded model files.
- Do not execute arbitrary scripts from model repositories.
- Never bypass application or operating-system permissions.
- Do not install or modify system drivers.
- Do not store Claude credentials in the application database.
- Do not use Claude Team subscription credentials as a runtime authentication mechanism.

# 13. Python AI Worker: Packaging and Lifecycle

Packaging the Python worker is a first-class design concern, decided in Phase 1b, not at release time. The worker targets **Python 3.13** in an isolated venv; the system Python (3.14) is never used.

## 13.1 Packaging requirements

- Independent of any system Python; the user must not install Python.
- Pinned dependencies with a lockfile; reproducible builds.
- Installed under `runtime/ai-worker/`, versioned with the application, separate from `models/`.
- CPU-only dependency set by default; no CUDA packages.
- Works offline after installation.

Options to prototype and compare in Phase 1b:

| Option | Notes to evaluate |
|---|---|
| Embedded Python distribution + pinned wheels | Size, update path, path handling |
| Frozen executable (e.g. PyInstaller) | Startup time, antivirus false positives, code signing |
| Managed environment created on first run (e.g. uv) | Requires network on first run — conflicts with offline install unless wheels are bundled |

Record installer size, startup time, and Windows Defender behavior for each option. Do not invent these numbers.

## 13.2 Lifecycle

- **Spawn:** Rust backend starts the worker on demand, not at application launch.
- **IPC:** versioned JSON messages over stdio (or a local-only channel); no network ports exposed externally.
- **Handshake:** worker reports protocol version, available runtimes, and devices on start.
- **Health:** heartbeat; backend restarts a hung worker with backoff.
- **Progress & cancellation:** every request carries a job ID; cancel is honored between batches.
- **Memory:** worker unloads models after idle timeout; one heavy model at a time.
- **Shutdown:** graceful stop request, then forced termination. On Windows, attach the worker to a Job Object so it dies with the application.
- **Crash isolation:** a worker crash fails the job, not the application. Logs go to `logs/`.

whisper.cpp runs as its own native process under the same lifecycle rules (ADR 0001).

# 14. Claude Code Development Rules

Create a root `CLAUDE.md` with the following rules:

1. Read all architecture and product documents before implementation.
2. **Do not start a phase (§16) without explicit approval from the project owner.**
3. Follow the module boundaries defined in this specification.
4. Do not replace local processing with cloud APIs.
5. Do not add mandatory API keys.
6. **Do not download models without explicit user approval of the model, size, license, and source.**
7. Do not install or modify system drivers or system-level components without explicit user approval.
8. Do not introduce NVIDIA CUDA dependencies.
9. Always implement CPU fallback.
10. Do not assume OpenVINO, GPU, NPU, or QSV support; validate at runtime.
11. Do not modify original media.
12. Do not generate final output from proxy files.
13. Do not hardcode model download URLs without verifying their official sources.
14. Record model licenses and checksums.
15. Do not invent benchmark numbers; record only measured results with method and date.
16. Implement tests for every processing module.
17. Keep UI responsive during background jobs.
18. Do not implement multiple major modules in one unreviewed change.
19. Update documentation when architecture changes.

Claude Code must act as a development assistant, not as an application runtime dependency.

Use the user's locally installed Claude Code CLI for development tasks only. Do not embed Team account authentication or distribute Claude credentials with the application.

# 15. Recommended Repository Structure

```text
memora-studio/
├── CLAUDE.md
├── README.md
├── docs/
│   ├── PRODUCT_SPEC.md
│   ├── ARCHITECTURE.md
│   ├── DISCOVERY_REPORT.md
│   ├── HARDWARE_VALIDATION.md
│   ├── DEPENDENCY_MATRIX.md
│   ├── AI_LOCAL_SPEC.md
│   ├── MODEL_MANAGER_SPEC.md
│   ├── MEDIA_ENGINE_SPEC.md
│   ├── EDITING_CORE_SPEC.md
│   ├── DATABASE_SPEC.md
│   ├── SECURITY_PRIVACY.md
│   ├── TESTING_STRATEGY.md
│   ├── BUILD_DEPLOYMENT.md
│   ├── ROADMAP.md
│   └── adr/
├── spikes/                  # Phase 1b throwaway prototypes
├── apps/
│   └── desktop/
├── crates/
│   ├── app-core/
│   ├── project-manager/
│   ├── media-library/
│   ├── media-engine/
│   ├── model-manager/
│   ├── ai-orchestrator/
│   ├── editing-core/
│   └── resource-manager/
├── workers/
│   └── ai-worker/
├── models/
│   └── manifest/
├── tests/
│   ├── fixtures/
│   ├── golden-media/
│   ├── integration/
│   └── performance/
└── scripts/
```

This is the logical target structure. Claude Code may adjust implementation details when justified, but must preserve the module boundaries.

# 16. Development Roadmap

Roadmap and phase gates follow `docs/Memora_Studio_Phase_0_Technical_Spec_and_Claude_Prompt.md` (§12, §14). Every phase transition requires explicit approval from the project owner. Decisions taken so far: `docs/adr/0001-phase0-decisions.md`.

## Phase 0: Review & Discovery — DONE (read-only)

- Specification review, read-only environment inspection, feasibility matrix, proposed corrections.
- Deliverable: `docs/DISCOVERY_REPORT.md`.

## Phase 1a: Environment Setup — approved

- Install via winget (approved packages only): `Rustlang.Rustup` (stable MSVC toolchain), `Gyan.FFmpeg` (GPL build).
- Create the data root `D:\MemoraStudio\` (§4.4).
- Create the isolated Python 3.13 venv for the worker spike; install `openvino`, `opencv-python-headless`, `scenedetect`, `numpy`; pin versions. No torch, no AI models.
- Deliverable: `docs/DEPENDENCY_MATRIX.md`.
- Acceptance: `rustc`, `cargo`, `ffmpeg`, `ffprobe` report versions; OpenVINO imports inside the venv.

## Phase 1b: Feasibility spikes

Throwaway code in `spikes/`. No model downloads.

1. FFmpeg capability check per §8.4: build info, QSV encoders/decoders, real test encodes on synthetic `testsrc2` input.
2. OpenVINO runtime discovery: device list (CPU / GPU / NPU); compile and run a small model built in code (no download) on each device; NPU checks per §3.4.
3. Integration spike: minimal Tauri 2 app that spawns the Python worker (JSON-lines over stdio), reports progress, cancels a job, survives a worker crash, runs FFmpeg as a child process with progress and cancellation, and leaves no orphan processes on exit (Job Object).
4. Worker packaging comparison per §13.1: size, startup time, antivirus behavior.

Deliverables: `docs/HARDWARE_VALIDATION.md` (measured results only), updated feasibility matrix, `docs/adr/0002-worker-packaging.md`.
Acceptance: each spike has a re-runnable script; all results are measured, not estimated.

## Phase 2: Desktop Foundation

Tauri + React shell, SQLite integration, structured logging, project manager, background job manager, hardware detection (§9.3), AI Worker lifecycle (§13.2).

## Phase 3: Media Engine

Media import, FFprobe metadata extraction, thumbnail/proxy generation, preview, basic trim/concatenation, first validated export (§8.3). Uses user-provided samples in `E:\Memora Data\test-media\` plus synthetic media.

## Phase 4: Classical Analysis

Scene detection, frame sampling, image metrics, perceptual duplicate detection, deterministic scoring. Decide the PySceneDetect decoding path (§17) — decided in ADR 0003: analyse the preview proxy with OpenCV in the worker; scores computed in Rust.

## Phase 5: Local AI

Model proposal **[approval]**: source, pinned revision, license, size, checksum per model. Then Model Manager install pipeline (§4.1), whisper.cpp base on CPU (Vietnamese + English), OpenCLIP export (dev-time torch) to OpenVINO with output consistency check, CPU/GPU benchmarks, optional NPU and Whisper GPU experiments, analysis caching.

## Phase 6: Editing Intelligence

Editing Plan, clip selection, story structure, editing styles, Smart review workflow.

## Phase 7: Product Completion

Timeline, templates, Story/Reels presets, travel recap, six-month/yearly recap, packaging, offline validation, FFmpeg license review before any distribution.

# 17. Open Requirements

Identified in Phase 0; to be resolved in the listed phase.

| Topic | Gap | Phase |
|---|---|---|
| Background music | No music source, licensing policy, or beat detection component | 6 |
| Vietnamese text | Titles/subtitles need fonts with Vietnamese diacritics; FFmpeg build must include `libfreetype` and `libass` | 1b (verify), 6 |
| Location | GPS extraction from phone videos; offline reverse geocoding source | 3 |
| Phone footage | HEVC 10-bit, HDR (tonemapping), variable frame rate normalization | 3 |
| Perceptual hashing | Library not chosen (OpenCV `img_hash` requires contrib, or numpy dHash/pHash) | 4 |
| Import hashing | Full SHA-256 of large videos is slow; use size + mtime + partial hash first, full hash lazily | 3 |
| Determinism | Defined as: same Editing Plan + same encoder settings → same timeline, duration and content. Bit-identical output is not required (hardware encoders differ by driver) | 2 |
| Scene detection decoding | PySceneDetect decodes via OpenCV/PyAV, not the FFmpeg CLI. Choose: OpenCV decoding in the worker, or FFmpeg-extracted frames | 4 |
| Frame transfer | Large data (frames, audio) passed between processes by file path, never through stdio | 1b |
| Protocol compatibility | App ↔ worker protocol version handshake across updates | 2 |
| Test environment | A stable (non-Insider) Windows 11 machine for validation; the dev machine runs build 26300 | 7 |
