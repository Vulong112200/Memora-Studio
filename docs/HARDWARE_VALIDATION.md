# Hardware Validation — Phase 1b results

**Date:** 2026-11-07  
**Machine:** ASUS Zenbook 14 UX3405CA, Core Ultra 9 285H, Arc 140T (driver 32.0.101.8860), AI Boost NPU (driver 32.0.100.4023), 32 GB RAM, Windows 11 build 26300.9711  
**Software:** FFmpeg 9.0.2 (Gyan full_build), OpenVINO 2026.4.1, Python 3.13.17, Rust 1.99.1, Tauri 2.12.5  
All numbers below were **measured** with the scripts in `spikes/` (re-run instructions: `spikes/README.md`). Single runs, synthetic input, machine otherwise in normal use — treat throughput as indicative, not as a benchmark.

## 1. FFmpeg encode / decode (`spikes/ffmpeg-qsv/run.py`)

Input: `testsrc2` synthetic video, 30 fps (simple content — real footage will encode slower). Every output was checked with ffprobe (codec, profile, resolution, exact frame count). Raw results: `E:\Memora Data\cache\spike\ffmpeg*\results.json`.

| Test | Works | 10 s clip fps | 60 s clip fps | Output |
|---|---|---|---|---|
| encode `libx264` medium (software) | ✅ | 208.8 | 196.8 | H.264 High 1920×1080 |
| encode `h264_qsv` | ✅ | 156.8 | 339.5 | H.264 High 1920×1080 |
| encode `hevc_qsv` | ✅ | 220.0 | 373.4 | HEVC Main 1920×1080 |
| encode `hevc_qsv` Main10 (10-bit source) | ✅ | 186.8 | 294.1 | HEVC Main 10, yuv420p10le |
| encode `av1_qsv` | ✅ | 197.8 | 379.2 | AV1 Main 1920×1080 |
| encode `libsvtav1` preset 8 (software) | ✅ | 159.0 | 187.4 | AV1 Main 1920×1080 |
| encode `h264_qsv` vertical 1080×1920 | ✅ | 201.6 | 349.3 | H.264 High 1080×1920 |
| full-HW decode + encode H.264 (QSV surfaces) | ✅ | 207.8 | 380.5 | H.264 High 1920×1080 |
| decode H.264 software | ✅ | 1300.5 | 1885.9 | — |
| decode `h264_qsv` | ✅ | 288.5 | 595.3 | — |
| decode HEVC 10-bit software | ✅ | 345.1 | 360.0 | — |
| decode `hevc_qsv` 10-bit | ✅ | 332.8 | 784.2 | — |

Findings:
- All QSV encoders listed by the build **work** on Arc 140T, including AV1 and HEVC 10-bit.
- On 10 s clips QSV is not faster than software: hardware session start-up (~0.5–1 s) dominates. QSV pays off on longer renders (~1.7× libx264 on 60 s 1080p here).
- Software H.264 decode to system memory is **faster** than `h264_qsv` decode with copy-back. Use QSV decode only in a full-hardware pipeline (decode → scale → encode on QSV surfaces), or for HEVC 10-bit where QSV was ~2.2× faster.
- Policy stays: probe at runtime, software fallback always available.

## 2. OpenVINO devices (`spikes/openvino-devices/run.py`)

In-code MLP, static shape [8×768] → 3072 → 3072 → 3072 → 768 (no download). Output compared with a NumPy float32 reference.

| Device | Compile | Avg inference (50 runs) | Inference precision | Rel. error vs NumPy | Result |
|---|---|---|---|---|---|
| CPU | 0.041 s | 2.339 ms | FP32 | 1.2e-6 | ✅ |
| GPU (Arc 140T) | 2.599 s | 0.683 ms | FP16 (default) | 5.9e-4 | ✅ |
| NPU (AI Boost) | 0.517 s | 1.827 ms | FP16 | 6.2e-4 | ✅ |

Findings:
- All three devices **compile and execute** an OpenVINO model with correct results. This proves runtime/device execution only — **not** compatibility or speed of OpenCLIP/Whisper (Phase 5).
- GPU and NPU default to FP16; results differ from FP32 at ~1e-3 relative level. Phase 5 must check embedding consistency for real models.
- GPU compile time is seconds → use OpenVINO model caching (`CACHE_DIR` under `<DataRoot>\cache\`) in the worker.

## 3. Process orchestration (`spikes/orchestrator`, `spikes/ai-worker/worker.py`)

Rust (std + serde_json + windows-sys) drives the Python worker over JSON-lines stdio and FFmpeg as a child process. 11/11 scenarios pass:

| Scenario | Result |
|---|---|
| Spawn + handshake (`hello` with protocol, OpenVINO version, devices) | ✅ ~430–640 ms warm |
| Job progress events + result | ✅ |
| `health` answered while a job is running | ✅ |
| Cancel mid-job | ✅ acknowledged within one batch (~50 ms for 50 ms batches) |
| Real OpenVINO job inside worker — CPU (complete), GPU (cancelled mid-run), NPU (complete) | ✅ |
| Worker crash (exit 3) detected, restart, health OK | ✅ restart ~440–510 ms |
| FFmpeg child: `-progress pipe:1` parsed, graceful cancel with `q` | ✅ exit 0 ~410–470 ms after `q` |
| Graceful worker shutdown | ✅ ~40 ms |
| Orchestrator force-killed (`taskkill /F`) → worker launcher, Python interpreter and FFmpeg all terminated (Job Object `KILL_ON_JOB_CLOSE`) | ✅ no orphans |

**Critical finding — Windows stdin deadlock.** With a thread blocked reading stdin (open pipe, no data), `import numpy` / `import openvino` on the main thread **hangs** (`spikes/ai-worker/probe_stdin_block.py`: plain Python ok; numpy, openvino, inference all hang). Therefore the worker is **single-threaded**: blocking stdin read only while idle; during a job, stdin is polled with `PeekNamedPipe` between batches. Do not introduce a stdin reader thread in the real worker.

Other notes:
- The venv `python.exe` is a launcher that spawns the real interpreter (two processes); the Job Object covers both.
- PowerShell pipes add a UTF-8 BOM; the worker decodes with `utf-8-sig`.
- Not tested: console-window flashing when a GUI (Tauri) app spawns console children — the real backend should pass `CREATE_NO_WINDOW`.

## 4. Tauri 2 build (`spikes/tauri-smoke`)

| Item | Result |
|---|---|
| Scaffold `create-tauri-app` (vanilla-ts), Tauri 2.12.5 | ✅ |
| Release build (`tauri build --no-bundle`, cold) | ✅ 2 min 53 s, VS Build Tools 2026 + Rust 1.99.1 |
| Executable size | 4.2 MB |
| Launch: window created, 1 WebView2 child process, working set 26.4 MB, graceful close exit 0 | ✅ |
| Installer bundling (MSI/NSIS) | Not tested (would download WiX/NSIS) |

## 5. Worker packaging (`spikes/packaging`, outputs in `E:\Memora Data\cache\spike\packaging`)

Same orchestrator suite run twice per variant (run 1 = cold file cache):

| Variant | Size on disk | Handshake run 1 / run 2 | Suite |
|---|---|---|---|
| Dev venv (reference) | 423 MB | 467 / 432 ms | 11/11 |
| Embedded Python 3.13.17 (official zip, `python.exe` signed by Python Software Foundation) + copied site-packages | 433 MB | 2299 / 444 ms | 11/11 |
| PyInstaller 6.22.3 `--onedir --collect-all openvino` | 294 MB, 1056 files | 3149 / 593 ms | 11/11 |

Caveats:
- PyInstaller only bundles modules the worker imports; this spike worker does not import OpenCV/PySceneDetect, so its size is not comparable yet. The embedded bundle contains everything.
- **Antivirus behavior could not be tested**: Windows Defender real-time protection is off on this machine.
- Embedded zip SHA-256 `9E35319BFFEDAB8B36AD44D8F377CB4F1733BCEC257A0B6D04D94F01486DDB38` (computed, not yet compared with a python.org published hash).

Recommendation: `docs/adr/0002-worker-packaging.md`.

## 6. Updated feasibility matrix

| Component | CPU | Intel GPU | NPU | Status |
|---|---|---|---|---|
| FFmpeg software encode/decode | Verified | — | — | Verified |
| FFmpeg `h264_qsv` / `hevc_qsv` / `av1_qsv` encode | — | Verified | — | Verified |
| FFmpeg QSV decode (H.264, HEVC 10-bit) | — | Verified | — | Verified |
| OpenVINO runtime (in-code model) | Verified | Verified | Verified | Verified |
| OpenVINO inside worker process, cancellable | Verified | Verified | Verified | Verified |
| OpenCV / PySceneDetect | Detected but untested | — | — | Installed, not exercised |
| OpenCLIP ViT-B/32 via OpenVINO | Theoretically supported | Theoretically supported | Theoretically supported | Unknown (Phase 5) |
| whisper.cpp | Theoretically supported | Theoretically supported | Unknown | Unknown (Phase 5) |
| Tauri 2 + WebView2 | Verified | — | — | Verified |
| Rust ↔ Python worker IPC + Job Object | Verified | — | — | Verified |

## 6a. Phase 3 — preview proxies on real footage (2026-11-07)

Source: user samples, DJI HEVC Main 10 (yuv420p10le), 2688×1512, 50 fps, AAC, ~80 Mbit/s. Target: 960×540, 30 fps, H.264, AAC 96k.

Manual benchmark on the 39.7 s clip (wall clock, one run each):

| Pipeline | Time |
|---|---|
| QSV decode → `vpp_qsv` scale/fps → `h264_qsv` | 4.4 s |
| software decode → scale → `h264_qsv` | 24.4 s |
| software decode → scale → `libx264 -preset veryfast` | 21.9 s |

Software 10-bit HEVC decode is the bottleneck. In the app (`media.proxy` job, QSV path), all 4 clips (7 min 4 s of footage, 4.2 GB) were converted in ~42 s of encode time: 3.9 s / 6.0 s / 27.3 s (the 4 min 59 s clip) / 4.5 s. Single run, not averaged.

Rotation: `-hwaccel qsv` + `vpp_qsv` does **not** apply the display matrix (output was squashed on a synthetic `-display_rotation 90` clip); the software path autorotates correctly. The app therefore uses software for rotated sources. `vpp_qsv transpose` gave ambiguous results and is not used until tested on real phone footage.

### Export (slice 3.3)

QSV decode with `-hwaccel_output_format qsv` keeps frames on the GPU, so CPU filters need `vpp_qsv=format=nv12,hwdownload,format=nv12`; FFmpeg autorotate cannot run on GPU surfaces, so the app uses `-noautorotate`, applies `transpose` itself and removes the display matrix (`sidedata=mode=delete:type=DISPLAYMATRIX`). Rotation matches software autorotate (automated test compares frames for a 90° source).

20 s segment of the DJI clip → 1920×1080 30 fps letterboxed, `h264_qsv -global_quality 22`, one run each:

| Decode | Time |
|---|---|
| hybrid (QSV decode, CPU filters) | 7.0 s |
| software | 28.8 s |

App E2E (UI-driven): 2-clip timeline (5.05 s trimmed with Set In/Out on the playing proxy + 39.68 s full clip) → Landscape export rendered in 11.4 s (hybrid-qsv), validation passed; output 44.747 s vs 44.734 s expected, 1920×1080, 30 fps, AAC 48 kHz stereo, 162.5 MB (~29 Mbit/s — ICQ 22 is generous; tune in Phase 7 if file size matters).

## 7. Disk usage of spikes

| Path | Size |
|---|---|
| `spikes/tauri-smoke/src-tauri/target` | 1447 MB |
| `spikes/packaging/.venv-pyi` | 325 MB |
| `spikes/tauri-smoke/node_modules` | 75 MB |
| `spikes/orchestrator/target` | 43 MB |
| `E:\Memora Data\cache\spike` | 1551 MB |
| `%USERPROFILE%\.cargo\registry` (C:) | 347 MB |

All spike outputs are throwaway and can be deleted after Phase 1b review.
