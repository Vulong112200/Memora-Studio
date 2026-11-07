# ADR 0001 — Phase 0 decisions

**Date:** 2026-11-07  
**Status:** Accepted (project owner)  
**Context:** `docs/DISCOVERY_REPORT.md`

| # | Decision | Rationale |
|---|---|---|
| 1 | **Data root `D:\MemoraStudio\`** on the reference machine; user-selectable in the app. App binaries stay in Program Files. | C: has ~82 GB free; D: ~190 GB unused. Cache/proxies/models do not need to be on C:. Same NVMe disk, no speed difference. |
| 2 | **Roadmap follows the Phase 0 spec** (Phase 1–7, approval gate between phases). Phase 1 split into 1a Environment Setup and 1b Feasibility spikes. | Removes the conflict with `Production.md` v1.1; keeps Phase 0 strictly read-only. |
| 3 | **Phase 1a installs**: rustup (stable MSVC), FFmpeg, Python 3.13 venv with OpenVINO, OpenCV (headless), PySceneDetect, numpy. **No AI models, no torch.** | Required to validate the stack; model downloads remain a separate approval in Phase 5. |
| 4 | **Installation via winget**, approved packages only: `Rustlang.Rustup`, `Gyan.FFmpeg`. | Simple to update and uninstall; permissions restricted in `.claude/settings.json`. |
| 5 | **FFmpeg GPL build (Gyan)** for development and internal use. | Includes QSV, libx264, libass, freetype. GPL/LGPL must be revisited before any distribution. |
| 6 | **AI Worker uses Python 3.13** (already installed) in an isolated venv; system Python 3.14 is not used. Fall back to 3.12 only if a required wheel is missing. | Avoids an extra install and unverified 3.14 wheels. |
| 7 | **OpenCLIP runtime = OpenVINO.** PyTorch is used only at development time for export and as a consistency reference. | Much smaller worker; one runtime for Intel CPU/GPU. |
| 8 | **whisper.cpp runs as a separate native process** controlled by Rust. | Simpler packaging; switching CPU/GPU backend is swapping a binary. |
| 9 | **Memory: the app limits itself**; Windows pagefile (fixed 2 GB) is not changed. Monitor commit charge, one heavy model at a time, no AI in parallel with rendering. | Must also work on other users' machines; no system changes. |
| 10 | **Test media**: synthetic FFmpeg sources plus non-private samples provided by the user in `D:\MemoraStudio\test-media\`. Personal folders are never scanned. | Needed for Vietnamese/English speech and phone-footage validation. |

## Amendment 2026-11-07 — data root moved to `E:\Memora Data\`
Supersedes the location in decisions 1 and 10. The project owner partitions drives by purpose: D: ("Software") is for software, E: ("Data") is for data. The data root was moved (copied, verified by file count/bytes and SHA-256 for test media) to `E:\Memora Data\`, outside the repository `E:\Memora Studio\`. Stored media paths in the database were rewritten. Everything else in decisions 1 and 10 is unchanged (user-selectable, nothing on C:, test media in `<data root>\test-media\`).

## Deferred
Stable Windows 11 test machine, antivirus product, background-music source, PySceneDetect decoding path (Phase 4), model downloads (Phase 5).
