# ADR 0003 — Classical analysis: where it runs and what it reads

**Date:** 2026-11-07
**Status:** Accepted (implemented in Phase 4; project owner approved Phase 4)
**Resolves:** "PySceneDetect decoding path" (docs/Production.md §17, deferred in ADR 0001)

## Decision
1. **Analysis reads the preview proxy, not the original.** Proxies are H.264 540p ≤30 fps; timestamps map 1:1 to the original. Rendering still always reads originals (§8.1).
2. **One sequential OpenCV decode per clip in the AI worker** (`workers/ai-worker/analysis.py`): every frame (downscaled to 320 px) is fed to PySceneDetect's `AdaptiveDetector.process_frame`; global motion by `cv2.phaseCorrelate` at 160 px; a sample frame every 2 / 1 / 0.5 s (fast / balanced / detailed) gets metrics and a dHash. One representative JPEG per scene goes to `<data root>\cache\frames\<media id>\`.
3. **The worker stores observations only; scores are computed in Rust** (`scoring.rs`) on read, as pure functions. Changing the formula needs no re-analysis.
4. **Missing proxies are created first** by the `media.analyze` job (Rust), then the worker job `analyze_video` runs.

## Why
- Software decode of the user's 10-bit HEVC 2.7K footage is the bottleneck (~2× real time, `HARDWARE_VALIDATION.md` §6a); the proxy decodes far faster and already exists for preview.
- OpenCV/PySceneDetect are already in the approved worker venv; no new dependency.
- Keeping AI/vision observation separate from decisions follows "AI produces observations; Editing Core decides".

## Measured (single runs, not averaged)
- 5 min proxy (8 975 frames): 35 s, ~256 fps, in a standalone run.
- App, `balanced`: all 6 library clips (~7.5 min) incl. worker start in ~51 s.

## Known limits
- Sharpness (Laplacian variance) is resolution-dependent: only comparable between proxies (540p).
- dHash is not rotation-invariant; the same footage rotated is not a duplicate.
- Continuous shots give one long scene; per-window scoring of long scenes is left to Phase 6 clip selection.
- Score thresholds in `scoring.rs` are initial assumptions, not validated on a broad footage set.
- Packaging: `analysis.py` must ship next to `memora_worker.py` in `runtime\ai-worker\app\`.
