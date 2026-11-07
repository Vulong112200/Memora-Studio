# Phase 0 Discovery Report

**Date:** 2026-11-07  
**Scope:** Read-only inspection (CIM/WMI, registry, `--version`, `pip list`). Nothing was installed, downloaded, or modified.  
**Decisions taken afterwards:** `docs/adr/0001-phase0-decisions.md`.

## A. Environment

### Hardware (measured)

| Item | Measured value |
|---|---|
| Device | ASUSTeK ASUS Zenbook 14 UX3405CA |
| CPU | Intel Core Ultra 9 285H, 16 cores / 16 threads |
| RAM | 31.36 GB usable; 11.42 GB free at inspection time |
| GPU | Intel Arc 140T, driver 32.0.101.8860 (2026-06-25). Driver name contains "(16GB)" — a label, not VRAM |
| GPU shared memory in use | ≈ 2.7 GB at inspection time (DXDiag figure of 18,307 MB not re-measured) |
| NPU | Intel AI Boost, status OK, driver 32.0.100.4023; `npu_driver_compiler.dll` present in DriverStore |
| Disk | Micron NVMe 954 GB, partitions: C: 82 GB free, D: 190 GB free, E: 222 GB free, F: 95 GB free |
| Pagefile | Fixed 2,048 MB |
| OS | Windows 11 Home Single Language, build 26300.9711, DisplayVersion 26H2 |

### Software

| Tool | Status |
|---|---|
| Node.js / npm | v24.21.0 / 11.19.0 |
| Git | 2.55.0 |
| Python | 3.14.7 (default, `C:\Python314`, only numpy 2.5.4 + pillow 12.3.0); 3.13.17 (no relevant packages) |
| Visual Studio Build Tools | 2026 (18.10), VC x64 tools present; Windows SDK 10.0.26100 |
| WebView2 Runtime | 157.0.4341.42 |
| Level Zero | `ze_loader.dll` 1.28.2, `ze_intel_gpu64.dll` (driver) |
| OpenCL | ICD loader 3.0.8; Intel driver registers OpenCL |
| Intel media runtime | `libvpl.dll` 2.16, `libmfx64-gen.dll` (driver) |
| LongPathsEnabled | 1 |
| Rust / Cargo / rustup | **Missing** |
| FFmpeg / FFprobe | **Missing** |
| OpenVINO | **Missing** |
| OpenCV, PySceneDetect, open_clip, torch, onnx, whisper.cpp | Missing |
| CMake, pnpm, uv | Missing (not required) |
| Windows Defender real-time protection | Off (possibly third-party antivirus) |

## B. Specification review

### Contradictions
1. Phase 0 scope: `Production.md` v1.1 included installs and model downloads in Phase 0; the Phase 0 spec defines Phase 0 as read-only. Phase numbering also differed. → Resolved: Phase 0 spec roadmap adopted.
2. PySceneDetect decodes through OpenCV/PyAV, not the FFmpeg CLI → two decoders with different codec coverage and licenses. → Open (Phase 4).
3. "Deterministic rendering" cannot mean bit-identical output across encoders/drivers. → Redefined as same Plan → same timeline.
4. `libx264` fallback is GPL; FFmpeg licensing was not addressed. → Resolved for development (GPL), revisit before distribution.

### Unsupported assumptions
- PyTorch as shipped OpenCLIP CPU runtime adds significant worker size. → Resolved: dev-time only.
- Wheel availability for Python 3.14 unverified. → Resolved: worker uses Python 3.13 venv.
- Build 26300 / 26H2 appears to be an Insider build; behavior may differ from stable Windows 11.
- NPU, QSV, OpenVINO GPU: drivers present, no runtime exercised.

### Risks

| Risk | Level | Note |
|---|---|---|
| Fixed 2 GB pagefile | High | Commit limit ≈ RAM + 2 GB; out-of-memory failures instead of slowdowns |
| Drive C: 82 GB free | Medium | Proxies/cache can reach tens to hundreds of GB → data root on D: |
| Python worker packaging | High | Antivirus false positives, size, update coupling; Defender currently off on dev machine |
| Phone footage (HEVC 10-bit, HDR, VFR) | Medium | Not covered by spec |
| Full SHA-256 of large videos on import | Medium | Slow on large libraries |
| Frames over stdio | Medium | Pass file paths instead |
| Proxy playback in WebView2 | Low | Proxies should be H.264 |

### Missing requirements
Listed in `Production.md` §17 (music, Vietnamese fonts, GPS, phone footage, perceptual hashing, import hashing, protocol versioning, stable-Windows test machine).

## C. AI feasibility matrix

No runtime is installed, so nothing is **Verified**.

| Component | CPU | Intel GPU | NPU | Status | Notes |
|---|---|---|---|---|---|
| GPU driver + Level Zero | — | Detected but untested | — | Detected but untested | ze_loader 1.28.2 |
| OpenCL | — | Detected but untested | — | Detected but untested | ICD 3.0.8 |
| Intel media runtime (oneVPL) | — | Detected but untested | — | Detected but untested | libvpl 2.16 |
| NPU driver + compiler | — | — | Detected but untested | Detected but untested | No runtime exercised |
| FFmpeg software encode/decode | Theoretically supported | — | — | Unknown | Not installed |
| FFmpeg `h264_qsv` / `hevc_qsv` | — | Theoretically supported | — | Unknown | Depends on build |
| FFmpeg `av1_qsv` | — | Theoretically supported | — | Unknown | Needs real test encode |
| FFmpeg QSV decode | — | Theoretically supported | — | Unknown | |
| OpenVINO runtime | Theoretically supported | Theoretically supported | Theoretically supported | Unknown | Not installed |
| OpenCV metrics | Theoretically supported | Theoretically supported (OpenCL T-API, not needed) | Unsupported | Unknown | |
| PySceneDetect | Theoretically supported | Unsupported | Unsupported | Unknown | CPU only |
| Perceptual hashing | Theoretically supported | — | Unsupported | Unknown | Library not chosen |
| OpenCLIP ViT-B/32 – PyTorch | Theoretically supported | Theoretically supported (Intel XPU build) | Unsupported | Unknown | Dev-time only |
| OpenCLIP ViT-B/32 – ONNX → OpenVINO | Theoretically supported | Theoretically supported | Theoretically supported (static shapes) | Unknown | Export + consistency check needed |
| whisper.cpp base/small | Theoretically supported | Theoretically supported (Vulkan / SYCL / OpenVINO encoder) | Unknown | Unknown | No NPU backend in whisper.cpp |
| Whisper via OpenVINO GenAI (alternative) | Theoretically supported | Theoretically supported | Theoretically supported | Unknown | Not added to stack |
| Vietnamese quality (Whisper base) | — | — | — | Unknown | Needs samples |

## E. Resource estimates (not measured)

| Item | Estimate | Basis |
|---|---|---|
| Whisper base (ggml) | ~142 MB | whisper.cpp README |
| Whisper small (ggml) | ~466 MB | whisper.cpp README |
| OpenCLIP ViT-B/32 | ~600 MB FP32, ~300 MB FP16 IR | ~151M parameters × 4 / 2 bytes |
| MVP models total | ~0.75–1.3 GB | |
| FFmpeg | ~100–200 MB | build dependent |
| Python worker without torch | a few hundred MB | to be measured in Phase 1b |
| CLIP embedding | ~2 KB per frame | 512 × float32 |
| 720p H.264 proxy at ~2.5 Mbps | ~1.1 GB per hour of footage | → per-project proxies, size-capped LRU cache |

Recommended model installation order: none (classical pipeline first) → Whisper base (CPU) → OpenCLIP ViT-B/32 (after license review) → Whisper small (optional) → GPU/NPU experiments.
