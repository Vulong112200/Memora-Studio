# Memora Studio

## Phase 0: Project Review & Technical Discovery

**Version:** 1.0\
**Status:** Ready for first Claude Code review\
**Platform:** Windows 11 x64\
**Principles:** Local-first, offline-capable, AI-optional

## 1. Product Overview

Memora Studio is a Windows desktop application for organizing,
analyzing, editing, and exporting personal videos.

Primary use cases: - Daily-life video editing. - Travel videos and
social media Stories/Reels. - Highlight videos from long trips. -
Six-month and yearly memory recaps. - Personal video organization by
date, event, and location.

Core principles: 1. No mandatory cloud AI or API key. 2. Claude Code is
a development assistant, not a runtime dependency. 3. Original media
must never be modified. 4. Editing decisions must be stored in an
explicit project/Editing Plan. 5. Rendering must be deterministic and
reproducible. 6. AI analysis is optional and replaceable. 7. Model
downloads and installation require user approval. 8. Offline operation
must work with installed dependencies and models. 9. Basic editing and
export must work without AI models.

## 2. Verified Development Hardware

  Component              Verified information
  ---------------------- ---------------------------------
  Laptop                 ASUS Zenbook 14 UX3405CA
  CPU                    Intel Core Ultra 9 285H
  Logical processors     16
  GPU                    Intel Arc 140T integrated GPU
  Dedicated GPU memory   128 MB reported by DXDiag
  Shared GPU memory      18,307 MB reported by DXDiag
  NPU                    Intel AI Boost
  Graphics driver        32.0.101.8860
  NPU driver             32.0.100.4023
  RAM                    32 GB
  SSD                    Micron NVMe, approximately 1 TB
  OS                     Windows 11 Home Single Language
  Build                  26300
  DirectX                12
  GPU driver model       WDDM 3.2

### Hardware cautions

-   Intel Arc 140T is an integrated GPU using shared system memory.
-   Do not interpret a Task Manager label such as "16GB" as 16 GB
    dedicated VRAM.
-   DXDiag reports 128 MB dedicated and 18,307 MB shared graphics
    memory. These values do not guarantee that all shared memory is
    available to AI workloads.
-   WMI AdapterRAM reported 2,147,479,552 bytes. Do not use this field
    alone to infer usable GPU memory.
-   Windows detecting Intel AI Boost does not prove that a model/runtime
    can execute on the NPU.
-   Do not update graphics or NPU drivers automatically.

## 3. Proposed Technology Stack

This is a proposed stack to evaluate, not an instruction to install
everything immediately.

  -----------------------------------------------------------------------
  Layer                   Proposed technology     Responsibility
  ----------------------- ----------------------- -----------------------
  Desktop shell           Tauri 2                 Windows desktop
                                                  application

  Frontend                React + TypeScript      UI, library, editor, AI
                                                  Center

  Backend                 Rust                    Process orchestration,
                                                  jobs, storage

  Video processing        FFmpeg + FFprobe        Decode, encode, trim,
                                                  render

  Image processing        OpenCV                  Image metrics and frame
                                                  analysis

  Scene detection         PySceneDetect           Scene boundaries

  Visual understanding    OpenCLIP ViT-B/32       Optional semantic
                                                  analysis

  Speech recognition      whisper.cpp             Optional transcription

  Intel acceleration      OpenVINO                Evaluate CPU/GPU/NPU
                                                  inference

  Database                SQLite                  Projects and metadata

  AI worker               Python process or       Isolated model
                          packaged worker         inference
  -----------------------------------------------------------------------

### Important architecture separation

Keep these concepts independent:

-   **AI Model:** OpenCLIP, Whisper.
-   **Inference Runtime:** OpenVINO, whisper.cpp, ONNX Runtime, or
    native CPU runtime.
-   **Execution Device:** Intel CPU, Intel Arc GPU, or Intel NPU.

Do not treat OpenVINO as a model. Do not force every model to use
OpenVINO. Select runtime and device per model based on verified
compatibility and measured performance.

### High-level architecture

``` text
React + TypeScript UI
        |
Tauri + Rust Backend
        |
  +-----+------------------+
  |                        |
AI Worker               Media Engine
OpenCV                  FFmpeg
OpenCLIP                FFprobe
Whisper                 Proxy / Render
Runtime adapters        Audio processing
  |                        |
  +-----------+------------+
              |
       Local Storage
 SQLite / Projects / Models / Cache
```

The Editing Core and Rule Engine must be independent of model inference.
AI provides observations; the Editing Core makes explicit editing
decisions; FFmpeg performs media operations.

## 4. Local AI Strategy

### Classical processing

These features must work without downloaded AI models: - FFprobe
metadata extraction. - Scene detection. - Frame sampling. - Brightness
and exposure analysis. - Sharpness and blur estimation. - Black-frame
detection. - Motion estimation. - Perceptual duplicate detection. -
Basic clip scoring and deterministic selection rules.

### OpenCLIP ViT-B/32

Potential uses: - Identify broad concepts such as beach, mountain, food,
city, nature, architecture, and people. - Add semantic tags to clips. -
Support semantic search. - Help select clips relevant to a travel or
memory recap.

Before adopting a checkpoint, verify its exact revision, license,
redistribution conditions, official source, checksum, ONNX export
feasibility, OpenVINO compatibility, CPU/GPU performance, and peak
memory use.

### Whisper

Proposed runtime: whisper.cpp.

-   Initial candidate: Whisper base multilingual.
-   Optional quality upgrade: Whisper small multilingual.
-   Validate Vietnamese and English transcription.
-   Benchmark CPU first.
-   Test Intel GPU acceleration only if a compatible implementation is
    verified.
-   NPU support is optional and must not be assumed.
-   Do not automatically install both models.

### NPU policy

Treat Intel AI Boost as an experimental execution target. Validate
driver/runtime compatibility, OpenVINO NPU discovery, model/operator
compatibility, inference results, latency, and memory use. NPU support
is not a release requirement.

### No Local LLM in MVP

Do not install a general-purpose local LLM in the initial release.
Revisit this only when a concrete user-facing feature requires
natural-language reasoning.

## 5. Video Analysis and Editing

Pipeline:

``` text
IMPORT
  -> METADATA EXTRACTION
  -> SCENE DETECTION
  -> FRAME SAMPLING
  -> TECHNICAL ANALYSIS
  -> OPTIONAL VISUAL ANALYSIS
  -> OPTIONAL AUDIO ANALYSIS
  -> CLIP SCORING
  -> EDITING PLAN
  -> REVIEW / RENDER
```

Do not send entire videos into AI models. Analyze representative frames
near scene boundaries, avoid near-duplicate frames, provide
Fast/Balanced/Detailed modes, and cache results by source identity,
configuration, model version, and runtime version.

Keep scoring dimensions separate: - Technical quality. - Visual
composition. - Semantic relevance. - Audio relevance. - Memory value. -
Storytelling value.

Do not discard a clip solely because it is blurry, dark, or shaky. It
may contain an important memory.

### Editing Core

The Editing Core converts metadata and analysis results into a
structured Editing Plan. The Rule Engine considers visual quality,
semantic relevance, scene diversity, duplicates, duration, camera
movement, audio relevance, event importance, favorites, and user
exclusions.

Users must be able to override automated decisions.

Editing styles: 1. Travel Cinematic. 2. Social Story/Reels. 3.
Memory/Nostalgia. 4. Travel Documentary. 5. Clean Minimal.

Editing modes: - Auto: generate an automatic video. - Smart: generate an
Editing Plan for review. - Manual Assist: recommend clips during
editing.

Smart mode is the recommended initial default.

## 6. Media Engine

Use FFmpeg for: - Reading source media. - Proxy generation. - Trimming
and concatenation. - Audio normalization and mixing. - Transitions. -
Titles and overlays. - Final export and validation.

Never render final output from low-resolution proxies.

Investigate actual FFmpeg support for: - Intel Quick Sync decoding. -
`h264_qsv`. - `hevc_qsv`. - `av1_qsv`.

Do not assume these encoders exist. Test real encoding/decoding and keep
software encoding such as `libx264` as a fallback.

Initial export presets:

  Preset      Resolution     Frame rate                 Format
  ----------- -------------- -------------------------- -----------------
  Story       1080 × 1920    30 FPS                     MP4 H.264 + AAC
  Reels       1080 × 1920    30 FPS                     MP4 H.264 + AAC
  Landscape   1920 × 1080    30 FPS                     MP4 H.264 + AAC
  Archive     Source-aware   Preserve where practical   MP4

Do not upscale every video to 4K or force all footage to 60 FPS.
Validate exported file existence, decodability, duration, resolution,
expected audio streams, unexpected black frames, and audio clipping.

## 7. Model Manager

Eventually support: - Model catalog and version. - Verified download
source. - License information. - SHA-256 checksum. - Estimated download
size. - User-approved download. - Interrupted-download recovery. -
Checksum verification. - Installation and validation. - Removal of
unused models. - Offline installation from local files.

Model states:

``` text
NOT_INSTALLED -> DOWNLOADING -> VERIFYING
              -> INSTALLING -> VALIDATING -> READY
```

Failure states: `DOWNLOAD_FAILED`, `CHECKSUM_FAILED`, `INSTALL_FAILED`,
`MODEL_LOAD_FAILED`, `INCOMPATIBLE`.

Do not download or install models during Phase 0. Store models outside
the application installation directory so application upgrades do not
require downloading them again.

## 8. Resource Management

The target system has 32 GB RAM and an integrated Intel Arc GPU sharing
system memory.

Requirements: - Do not load all models simultaneously. - Run heavy
vision and speech workloads sequentially by default. - Limit inference
batch size and CPU threads. - Monitor physical RAM, committed memory,
shared GPU memory, and pagefile. - Support cancellation. - Release model
memory when practical. - Cache analysis results. - Keep the UI
responsive. - Avoid competing AI inference and heavy rendering unless
resource limits permit it.

Do not allocate all available RAM to AI. Resource limits must be
configurable and measured.

## 9. Python Worker and Packaging Risks

Validate the proposed Rust orchestration + Python AI worker design
early.

Evaluate: - Process startup time. - Rust/Python IPC protocol. -
Structured request/response messages. - Progress events and
cancellation. - Crash recovery. - Python dependency isolation. - Windows
packaging and distribution. - Model paths independent of application
installation. - Offline startup.

Do not assume a development Python environment can be shipped directly
to end users. Recommend a packaging approach and document trade-offs
before implementation.

## 10. Offline and Privacy

Offline mode must support importing media, metadata extraction, scene
detection, classical image analysis, installed local model inference,
Editing Plan generation, preview, rendering, export, and project
management.

Privacy requirements: - Never upload media automatically. - Never upload
frames or audio automatically. - Never modify original media. - Restrict
access to user-selected locations. - Validate downloaded model files. -
Do not execute arbitrary scripts from model repositories. - Do not store
Claude credentials in the application database. - Do not use Claude Team
credentials as application runtime authentication.

## 11. Proposed Repository Structure

This is a target logical structure, not a requirement to create it
during Phase 0.

``` text
memora-studio/
├── CLAUDE.md
├── README.md
├── docs/
│   ├── PRODUCT_SPEC.md
│   ├── ARCHITECTURE.md
│   ├── AI_LOCAL_SPEC.md
│   ├── MODEL_MANAGER_SPEC.md
│   ├── MEDIA_ENGINE_SPEC.md
│   ├── EDITING_CORE_SPEC.md
│   ├── DATABASE_SPEC.md
│   ├── SECURITY_PRIVACY.md
│   ├── TESTING_STRATEGY.md
│   ├── BUILD_DEPLOYMENT.md
│   ├── HARDWARE_VALIDATION.md
│   └── ROADMAP.md
├── apps/desktop/
├── crates/
│   ├── app-core/
│   ├── project-manager/
│   ├── media-library/
│   ├── media-engine/
│   ├── model-manager/
│   ├── ai-orchestrator/
│   ├── editing-core/
│   └── resource-manager/
├── workers/ai-worker/
├── models/manifest/
├── tests/
└── scripts/
```

## 12. Development Roadmap

### Phase 0: Project Review and Technical Discovery

-   Read and review the specification.
-   Inspect the development environment.
-   Verify available tools and runtimes.
-   Evaluate architecture risks.
-   Produce a feasibility report and proposed corrections.
-   Do not install or download anything.

### Phase 1: Environment Setup

After approval, install only required development tools and establish
reproducible Rust, Node.js, Python, and FFmpeg environments.

### Phase 2: Desktop Foundation

Build the Tauri + React shell, SQLite integration, structured logging,
project manager, and background job manager.

### Phase 3: Media Engine

Implement media import, metadata extraction, thumbnail/proxy generation,
preview, basic trim/concatenation, and first validated export.

### Phase 4: Classical Analysis

Implement scene detection, frame sampling, image metrics, duplicate
detection, and deterministic scoring.

### Phase 5: Local AI

Implement Model Manager, OpenCLIP, Whisper, CPU/GPU benchmarking,
optional NPU investigation, and analysis caching.

### Phase 6: Editing Intelligence

Implement Editing Plan, clip selection, story structure, editing styles,
and Smart review workflow.

### Phase 7: Product Completion

Implement timeline, templates, Story/Reels presets, travel recap,
six-month/yearly recap, packaging, and offline validation.

------------------------------------------------------------------------

# 13. FIRST PROMPT FOR CLAUDE CODE

The following is the first task. It is intentionally limited to
inspection and planning.

``` text
# MEMORA STUDIO
# PHASE 0: PROJECT REVIEW & TECHNICAL DISCOVERY

You are acting as the Lead Software Architect and Senior Windows AI Infrastructure Engineer for Memora Studio.

Your first responsibility is to understand the project, review its architecture, and validate the development environment.

IMPORTANT: This is an inspection and planning task only. Do not start implementing the application.

## 1. Project Context

Memora Studio is a Windows desktop application for automatically organizing, analyzing, editing, and exporting personal videos.

Primary use cases:
- Daily-life video editing.
- Travel video creation.
- Social media Stories and Reels.
- Six-month and yearly memory recaps.
- Long-term personal video organization.

Core principles:
- Local-first.
- Offline-capable.
- AI-optional.
- No mandatory cloud APIs.
- No mandatory Claude API billing.
- Original media must never be modified.
- Editing decisions must be deterministic and reproducible.
- Local AI models must be independently installable and replaceable.

The application must remain usable even when every optional AI model is unavailable.

## 2. Read Existing Documentation

Inspect the current project directory.

Read:
- `docs/PRODUCT_SPEC.md`
- Any other existing architecture or project documentation.
- Existing source code, if present.

Do not assume the documentation is completely correct.

Identify:
- Contradictory requirements.
- Outdated hardware information.
- Unsupported technical assumptions.
- Unnecessary dependencies.
- Missing implementation details.
- Potential performance problems.
- Packaging and deployment risks.

Do not silently rewrite the original specification.

Produce recommendations and proposed changes separately.

## 3. Actual Development Hardware

Device:
- ASUS Zenbook 14 UX3405CA.

CPU:
- Intel Core Ultra 9 285H.
- 16 logical processors.

GPU:
- Intel Arc 140T integrated GPU.
- Dedicated memory: 128 MB reported by DXDiag.
- Shared system memory: 18,307 MB reported by DXDiag.
- Graphics driver: 32.0.101.8860.

NPU:
- Intel AI Boost.
- NPU driver: 32.0.100.4023.

RAM:
- 32 GB.

Storage:
- Micron NVMe SSD, approximately 1 TB.

Operating system:
- Windows 11 Home Single Language.
- Build 26300.

Important:
The Intel Arc GPU uses shared system memory. Never assume it has 16 GB dedicated VRAM.

The presence of an NPU device in Windows does not prove that a particular model or runtime can execute on it.

## 4. Architecture Review

Review:
- Tauri 2.
- React.
- TypeScript.
- Rust.
- FFmpeg.
- FFprobe.
- SQLite.
- Python AI Worker.
- OpenCV.
- PySceneDetect.
- OpenCLIP.
- whisper.cpp.
- OpenVINO.

Evaluate:
1. Rust-to-Python process communication.
2. Python environment management.
3. AI Worker packaging and distribution.
4. FFmpeg process management.
5. Hardware acceleration.
6. Model installation and versioning.
7. Application updates without reinstalling models.
8. Offline operation.
9. Memory and CPU resource management.

Do not introduce additional technologies without explaining why they are necessary.

## 5. Local AI Feasibility

Evaluate independently:

Visual understanding:
- OpenCLIP ViT-B/32.
- Exact checkpoint and license.
- Official download source.
- ONNX export feasibility.
- OpenVINO compatibility.
- Intel Arc GPU inference.
- CPU fallback.

Speech recognition:
- whisper.cpp.
- Whisper base multilingual.
- Whisper small multilingual.
- CPU inference.
- Possible Intel GPU acceleration.
- Vietnamese transcription quality.

Classical processing:
- OpenCV.
- PySceneDetect.
- Perceptual hashing.
- Blur and sharpness detection.
- Brightness and exposure analysis.
- Scene segmentation.

Intel acceleration:
- OpenVINO CPU.
- OpenVINO GPU.
- OpenVINO NPU.
- Intel Level Zero.
- Intel OpenCL.
- FFmpeg Intel Quick Sync.

Do not assume every component supports every device.

Separate verified compatibility from theoretical compatibility.

## 6. Read-Only Environment Inspection

Inspect the existing environment.

You may execute safe, read-only commands to detect:
- Windows version.
- CPU.
- GPU.
- RAM.
- Disk capacity and free space.
- Graphics driver.
- NPU driver.
- Installed Rust toolchain.
- Installed Node.js.
- Installed npm or pnpm.
- Installed Python.
- Installed FFmpeg.
- Existing OpenVINO installation.
- Existing AI runtimes.

If available, check:
- `rustc --version`
- `cargo --version`
- `node --version`
- `npm --version`
- `python --version`
- `ffmpeg -version`
- `ffmpeg -encoders`
- `ffmpeg -hwaccels`

For OpenVINO, check whether the existing environment can discover CPU, GPU, and NPU devices.

For other components, only inspect existing installations.

Do not install missing dependencies during this phase.

If a tool is unavailable, record it as missing instead of installing it.

## 7. Strict Safety Restrictions

During this phase:
- Do not install software.
- Do not download AI models.
- Do not download large dependencies.
- Do not modify Windows drivers.
- Do not modify environment variables.
- Do not change Windows settings.
- Do not execute administrator commands.
- Do not modify system configuration.
- Do not initialize a new application project.
- Do not generate application source code.
- Do not modify original documentation.
- Do not delete or overwrite files.
- Do not access personal video folders.
- Do not upload any media to cloud services.

Only inspect and report.

If a diagnostic command may modify the system, do not execute it without asking first.

## 8. Architecture Corrections

Separate:
- AI Model.
- Inference Runtime.
- Execution Device.

For example:
Model: OpenCLIP.
Runtime: OpenVINO or another validated runtime.
Device: Intel CPU or Intel Arc GPU.

Do not treat OpenVINO as an AI model or model-specific provider.

Do not force all AI models to use OpenVINO.

Choose the best runtime for each model based on compatibility and measured performance.

Keep NPU support optional.

## 9. Required Deliverables

Present a report in your response first.

Do not modify project files until the user approves.

The report must include:

A. Current Environment Report
- Hardware.
- Software versions.
- Installed development tools.
- Available runtimes.
- Missing dependencies.

B. Specification Review
- Incorrect assumptions.
- Contradictory requirements.
- Technical risks.
- Missing requirements.
- Recommended corrections.

C. AI Feasibility Matrix

Use this table:

| Component | CPU | Intel GPU | NPU | Status | Notes |
|---|---|---|---|---|---|

Use only these statuses:
- Verified.
- Detected but untested.
- Theoretically supported.
- Unsupported.
- Unknown.

D. Recommended Architecture
- Final proposed technology stack.
- AI Worker architecture.
- Model Manager architecture.
- Media Engine architecture.
- Runtime and device abstraction.
- Storage layout.

E. Resource and Storage Plan
- Estimated model storage.
- Estimated cache storage.
- Memory management strategy.
- Recommended model installation order.

Clearly label estimates and assumptions.

F. Implementation Roadmap
For each phase, provide:
- Objective.
- Scope.
- Dependencies.
- Deliverables.
- Acceptance criteria.
- Risks.

G. Questions Requiring User Approval
List decisions that cannot safely be made without the user's confirmation.

## 10. Final Instruction

Do not implement the application in this task.

Do not install or download anything.

Do not assume a detected hardware device is compatible with every model.

Do not invent benchmark results.

Do not silently change the architecture.

First understand the project, inspect the environment, identify risks, and produce a technically justified implementation plan.

Wait for explicit user approval before proceeding to the next phase.
```

------------------------------------------------------------------------

## 14. Approval Gates

Claude Code must wait for explicit approval between major phases.

  Phase                           Permission
  ------------------------------- ------------------------------------
  Phase 0: Review and Discovery   Read-only inspection and reporting
  Phase 1: Environment Setup      Explicit approval required
  Phase 2: Desktop Foundation     Explicit approval required
  Phase 3: Media Engine           Explicit approval required
  Phase 4: Classical Analysis     Explicit approval required
  Phase 5: Local AI               Explicit approval required
  Phase 6: Editing Intelligence   Explicit approval required
  Phase 7: Product Completion     Explicit approval required

The user remains in control of software installation, model downloads,
driver changes, file access, and major architecture decisions.

## 15. Final Architectural Principle

Memora Studio must be a deterministic video editor with optional local
AI intelligence.

The core application must never depend on Claude, cloud APIs, or an
always-running AI model.

AI models improve understanding and selection. The Editing Core decides
what to do. FFmpeg performs the actual editing and rendering.

**The first milestone is not an AI video editor. It is a reliable local
desktop foundation that can import, understand, edit, and export video
without requiring cloud services.**
