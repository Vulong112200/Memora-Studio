# Dependency Matrix

Measured on the reference machine (see `docs/DISCOVERY_REPORT.md`). Updated: 2026-11-07. **Phase 1a complete.**

## Data root

`D:\MemoraStudio\` created with: `runtime/`, `models/`, `projects/`, `cache/`, `logs/`, `database/`, `test-media/` (ADR 0001). Moved to `E:\Memora Data\` on 2026-11-07 (ADR 0001 amendment).

## System tools

| Tool | Version | Status |
|---|---|---|
| Node.js / npm | 24.21.0 / 11.19.0 | Installed (pre-existing) |
| Git | 2.55.0 | Installed (pre-existing) |
| VS Build Tools | 2026 (18.10), Windows SDK 10.0.26100 | Installed (pre-existing) |
| WebView2 Runtime | 157.0.4341.42 | Installed (pre-existing) |
| Python (worker) | 3.13.17 | Installed (pre-existing), used via venv only |
| rustup | 1.29.3 (winget `Rustlang.Rustup`) | Installed — `C:\Users\vu\.cargo\bin`, home `C:\Users\vu\.rustup` |
| Rust toolchain | rustc 1.99.1 / cargo 1.99.1, `stable-x86_64-pc-windows-msvc` (default) | Installed |
| FFmpeg / FFprobe | 9.0.2-full_build (gyan.dev, winget `Gyan.FFmpeg`), 664 MB on disk | Installed — via `%LOCALAPPDATA%\Microsoft\WinGet\Links` on user PATH |

Terminals opened before installation do not see the new PATH entries; restart the terminal/VS Code.

## FFmpeg build capabilities (from `-version`, `-encoders`, `-decoders`, `-hwaccels`, `-filters`; no encode run yet)

| Item | Result |
|---|---|
| License flags | `--enable-gpl`, `--enable-version3` → GPLv3 binary (ADR 0001: dev/internal use) |
| Intel QSV | Built with `libvpl` (not legacy `libmfx`); hwaccel `qsv` listed |
| QSV encoders listed | `h264_qsv`, `hevc_qsv`, `av1_qsv`, `vp9_qsv`, `mjpeg_qsv`, `mpeg2_qsv` |
| QSV decoders listed | `h264`, `hevc`, `av1`, `vp9`, `vp8`, `vvc`, `vc1`, `mpeg2`, `mjpeg` (`_qsv`) |
| Other hwaccels | `d3d11va`, `d3d12va`, `dxva2`, `vulkan`, `opencl` (+ `cuda`, `amf`, `vaapi` — not applicable to this machine) |
| Software encoders | `libx264`, `libx265`, `libsvtav1`, `aac`, `aac_mf`; no `libopenh264` |
| Text / subtitles | `libfreetype`, `libharfbuzz`, `libfribidi`, `libass` → `drawtext`, `subtitles`, `ass` available; no `libfontconfig` (fonts must be passed by file path) |
| Other filters | `loudnorm`, `blackdetect`, `scdet`, `tonemap`, `zscale` |

"Listed" means compiled in — **not** that encoding works on this GPU. Real test encodes are Phase 1b.

## AI worker venv — `spikes/ai-worker/.venv` (Python 3.13.17)

Pinned in `spikes/ai-worker/requirements.lock.txt`. Measured venv size: **423.3 MB** (without torch).

| Package | Version | Purpose |
|---|---|---|
| openvino | 2026.4.1 | Inference runtime (CPU / GPU / NPU) |
| opencv-python-headless | 5.0.0.93 | Image metrics, frame decoding |
| scenedetect | 0.7.2 | Scene boundary detection |
| numpy | 2.5.4 | Arrays |
| click, colorama, tqdm, platformdirs | see lock file | scenedetect dependencies |
| openvino-telemetry | 2025.2.0 | OpenVINO dependency — see Notes |

## OpenVINO device discovery (measured)

`openvino.Core().available_devices`:

| Device | FULL_DEVICE_NAME | Status |
|---|---|---|
| CPU | Intel(R) Core(TM) Ultra 9 285H | Detected by runtime — inference not yet tested |
| GPU | Intel(R) Arc(TM) 140T GPU (16GB) (iGPU) | Detected by runtime — inference not yet tested |
| NPU | Intel(R) AI Boost | Detected by runtime — inference not yet tested |

Device discovery is not proof of model compatibility. Compile + inference tests are Phase 1b.

## Notes

- `scenedetect` 0.7.2 declares a dependency on `opencv-python` (GUI build). It was removed and only `opencv-python-headless` is kept, because both packages install the same `cv2` module and conflict. `pip check` therefore reports `scenedetect requires opencv-python` — a metadata warning only; `import scenedetect` and `import cv2` work.
- `openvino-telemetry` is installed as an OpenVINO dependency. Telemetry is not part of the app's privacy model (§12); opting out (`opt_in_out --opt_out`) is pending user approval, and the shipped worker must not send telemetry.
