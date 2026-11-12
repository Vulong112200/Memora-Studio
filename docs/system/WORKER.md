# AI worker

The Python AI worker process: how Rust starts it, protocol v1, its jobs, the analysis and tagging algorithms, and the dev-only OpenCLIP export.

Last verified: 2026-11-12 against commit `12706f3` + uncommitted changes. Index: [README.md](README.md). Kept current by the `sync-docs` skill ([.claude/skills/sync-docs](../../.claude/skills/sync-docs/SKILL.md)).

Sources: [memora_worker.py](../../workers/ai-worker/memora_worker.py), [analysis.py](../../workers/ai-worker/analysis.py),
[tagging.py](../../workers/ai-worker/tagging.py), [requirements.lock.txt](../../workers/ai-worker/requirements.lock.txt),
[worker.rs](../../apps/desktop/src-tauri/src/worker.rs), [process.rs](../../apps/desktop/src-tauri/src/process.rs),
[export_openclip.py](../../tools/clip-export/export_openclip.py), [ADR 0002](../adr/0002-worker-packaging.md).
Callers: [lib.rs](../../apps/desktop/src-tauri/src/lib.rs) (`job_runner`), [analysis.rs](../../apps/desktop/src-tauri/src/analysis.rs),
[tagging.rs](../../apps/desktop/src-tauri/src/tagging.rs), [models.rs](../../apps/desktop/src-tauri/src/models.rs).

## 1. Runtime, start-up, process rules

### Runtime resolution (`worker::resolve`)

Tried in order; the first pair where **both** files exist wins:

| # | Python | Script | Notes |
|---|---|---|---|
| 1 | `<data root>\runtime\ai-worker\python\python.exe` | `<data root>\runtime\ai-worker\app\memora_worker.py` | Packaged runtime (ADR 0002: embedded Python 3.13 + pinned site-packages) |
| 2 | `<repo>\workers\ai-worker\.venv\Scripts\python.exe` | `<repo>\workers\ai-worker\memora_worker.py` | Dev fallback; `<repo>` is derived from `CARGO_MANIFEST_DIR` at **compile time** (marked `ponytail:` — remove once the installer provisions `runtime\ai-worker`) |

Neither found → job fails with *"AI worker runtime not found. Expected <data folder>\runtime\ai-worker\ (python\python.exe and app\memora_worker.py)."* The dev venv is Python 3.13.17 (`pyvenv.cfg`).

### Start (`Worker::spawn`)

- Command: `<python> -u <script>` (unbuffered), stdin/stdout/stderr piped.
- `process::command` sets `CREATE_NO_WINDOW` (0x08000000); `process::spawn` assigns the child to the app's Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), and kills it at once if assignment fails. The Job Object handle is never closed, so every child dies with the app (crash or forced kill included). Same Job Object as FFmpeg/whisper.cpp (`lib::job_runner` creates one per runner).
- Started **on demand**: on the first job routed to the worker, not at app start. One worker per runner (the app's job queue; the CLI builds its own runner).
- Routing in `lib::job_runner`: kinds starting with `media.` or `models.` run in Rust; **every other kind goes straight to the worker** (UI allow-list `UI_JOB_KINDS` exposes only `validate_devices` of those). Rust jobs call worker jobs as sub-jobs through the same `ai` runner, reusing the parent job id.
- Two Rust threads per worker: one forwards stdout lines into a channel; one logs every stderr line as `tracing::warn!(target: "ai_worker")` (into the app's JSON log).
- The first message must be `hello` with `protocol == 1` within `HELLO_TIMEOUT` = 60 s; otherwise the process is killed and the job fails (mismatch / timeout / exited during start-up). The `hello` object is stored in `AppState.worker_info` (Tauri command `get_worker_info`), cleared when the worker dies.

### Single-threaded rule (ADR 0002, `docs/HARDWARE_VALIDATION.md` §3)

- On Windows, a thread blocked reading stdin makes `import numpy` / `import openvino` on another thread hang (measured in Phase 1b). Therefore the worker has **no reader thread**.
- Idle: blocking `os.read` on stdin (`poll(block=True)`).
- During a job: `Job.check_cancel()` polls stdin with `PeekNamedPipe` (kernel32 via ctypes) and only reads when bytes are available. A failing `PeekNamedPipe` (broken pipe) is treated as "data available" so `os.read` reports EOF.
- Jobs must call `job.check_cancel()` between batches; that is also when `health` is answered and other messages are queued (`_pending`).
- Large data (frames, audio) goes by file path, never over stdio.

## 2. Protocol v1 (JSON lines over stdio)

One JSON object per line, UTF-8 (input decoded as `utf-8-sig`). Ids are strings (Rust sends `job.id.to_string()`).

### Rust → worker

| type | Fields | Worker behaviour | Sent by Rust? |
|---|---|---|---|
| `job.start` | `id`, `kind`, `params` | Runs `JOBS[kind](Job(id, params))`; unknown kind → `job.error` *"unknown job kind: …"* | Yes (`worker::run_on`) |
| `job.cancel` | `id` | During the job: `check_cancel` raises `Cancelled` → `job.cancelled`. When idle: ignored | Yes, once, when the job is cancelled |
| `health` | `id` | `health.ok` (idle: `{id}`; during a job: `{id, busy: <job id>}`) | No (supported, unused) |
| `shutdown` | — | Worker exits its main loop | No (Rust relies on stdin EOF / Job Object) |
| other | — | `error` *"unknown message type: …"* | — |
| invalid JSON line | — | `error` *"invalid json"* | — |

### Worker → Rust

| type | Fields | Rust behaviour (`run_on`) |
|---|---|---|
| `hello` | `protocol` (1), `pid`, `python` (version), `openvino` (version or null), `devices` (OpenVINO `available_devices`), `runtimeError` (only if OpenVINO failed to import), `startupMs` | Checked in `spawn`; stored as worker info |
| `job.progress` | `id`, `done`, `total` | Forwarded to the job's progress callback |
| `job.result` | `id`, `result` | `Outcome::Completed(result)` |
| `job.cancelled` | `id` | `Outcome::Cancelled` |
| `job.error` | `id`, `error` (≤ 500 chars, `"<ExceptionType>: <msg>"`) | `Outcome::Failed(error)`; worker stays alive |
| `health.ok` | `id`, `busy`? | Ignored (id ≠ job id) |
| `error` | `id`?, `error` | Ignored |

- Messages whose `id` is not the running job's id are ignored. Non-JSON stdout lines are logged (`non-JSON output ignored`) and skipped.
- Rust polls the channel every 100 ms.

### Timeouts, cancel, crash, restart

| Situation | Behaviour |
|---|---|
| No `hello` within 60 s (`HELLO_TIMEOUT`) | Kill; job fails "AI worker did not start within 60 s." |
| Cancel requested | `job.cancel` sent once. If no final message within `CANCEL_GRACE` = 10 s (comment: assumed value, not measured), the worker is killed, the job ends `Cancelled`, and the worker is restarted on the next job |
| Worker exits mid-job (stdout closed) | Job fails "AI worker stopped unexpectedly (exit code …)."; worker dropped, `worker_info` cleared; next job spawns a new one |
| stdin write fails | Job fails "AI worker pipe closed: …"; worker dropped |
| Exception inside a job | `job.error`; worker keeps running |
| Parent closes stdin | Worker raises `ParentGone` and exits quietly |
| Long job, no cancel | No overall job timeout in `worker.rs` |

Tested end-to-end by `worker::tests::worker_lifecycle_end_to_end` (skipped when the dev venv is missing): progress, cancel, stuck-job kill within grace, unknown kind, crash + restart, `validate_devices` (CPU must validate).

## 3. Jobs (`JOBS` in memora_worker.py)

9 entries (6 product jobs, 3 test-only `diagnostic.*`).

| Kind | Params | Result | Called by |
|---|---|---|---|
| `validate_devices` | none | `{openvino, devices: [{device, name, compileMs, inferMs, precision, relError, ok, error?}]}` — compiles an in-code 768→3072→768 MLP (seeded weights, input 8×768) on every OpenVINO device, 20 timed runs after a warm-up, `ok` = relative error vs NumPy FP32 < 1e-2 | UI "AI" card (`AiCenter.tsx` → `submit_job("validate_devices")`), routed directly by `lib::job_runner` |
| `analyze_video` | `path` (proxy), `framesDir` (`cache\frames\<id>`), `quality` (`fast`/`balanced`/`detailed`, default `balanced`) | `analysis.analyze` output, §4 | `analysis::analyze_job` (job `media.analyze`, after `proxy::ensure_proxy`) |
| `validate_openvino_ir` | `xml` (IR path), `devices` (default `["CPU"]`; Rust does not send it) | `{validation: "openvino-ir", openvino, inputs: [{name, shape}], devices: [{device, compileMs, inferMs, outputs, ok, error?}]}`; raises if no device is `ok`. Dynamic dims → 1, or 224 for H/W of rank-4 inputs; input = zeros; `ok` = all outputs finite | `models::install_job` (job `models.install`, manifests with `validation: "openvino-ir"`); result stored in `models.details` |
| `tag_video` | `path` (proxy), `modelDir`, `everyMs` (default 2000; Rust sends `tagging::EVERY_MS` = 2000), `devices` (default `["GPU","CPU"]`; Rust does not send it) | `tagging.tag_video` output, §5 | `tagging::tag_job` (job `media.tag`, after `proxy::ensure_proxy`) |
| `photo_analyze` | `path`, `edits`, `modelDir` (null = metrics only), `devices` (default `["GPU","CPU"]`) | `{width, height, metrics: frame_metrics, tags?: tag_video shape with one sample}` | `photos::analyze_job` (job `media.photo_analyze`) |
| `photo_render` | `path`, `edits`, `maxSide` (0 = full size), `out`, `quality` (default 92), `showMask`, `info` | `{width, height, outWidth, outHeight, ms, taken?}` | `photos::Lane` (import thumbnails, previews, export) — the photo worker process, not the job queue |
| `photo_auto` | `path` | `{edits, skyShare}` | `lib::auto_photo` via `photos::Lane` |
| `diagnostic.steps` | `steps` (default 10) | `{steps}`; 50 ms per step with progress + cancel | Tests only |
| `diagnostic.crash` | — | `os._exit(3)` | Tests only |
| `diagnostic.block` | `seconds` (default 30) | `{}` after one sleep that never checks cancel | Tests only |

`diagnostic.*` are not in `UI_JOB_KINDS` nor in the CLI `job` allow-list, so only tests can submit them. whisper.cpp transcription is **not** a worker job (separate native process, `transcribe.rs`).

## 4. analysis.py — classical analysis

One sequential OpenCV decode of the proxy; observations only (scoring is in Rust `scoring.rs`). Decisions: [ADR 0003](../adr/0003-classical-analysis.md).

### Constants

| Name | Value | Use |
|---|---|---|
| `VERSION` | 1 | Stored in `media_analysis.version`; bump when metrics change |
| `INTERVALS` | `fast` 2.0 s, `balanced` 1.0 s, `detailed` 0.5 s | Seconds between sample frames; `step = max(1, round(interval × fps))` frames |
| `DETECT_WIDTH` | 320 px | Frame width fed to PySceneDetect |
| `MOTION_WIDTH` | 160 px | Grey frame width for phase correlation; also the shake normaliser |
| `MIN_MOTION_RESPONSE` | 0.1 | Phase-correlation response below this (flat frames: black, sky) → shift ignored |
| fps fallback | 30.0 | When OpenCV reports 0 |
| `AdaptiveDetector(min_scene_len=int(fps × 0.6))` | ≈ 0.6 s | Minimum scene length in frames |
| Cancel/progress cadence | every 30 frames | `check_cancel()`, `progress(min(n,total), total)` |
| Clipping | `clipLow` = share of pixels ≤ 10, `clipHigh` = share ≥ 245 | 8-bit grey |
| Black frame | mean < 20 **and** std < 10 | |
| dHash | 9×8 INTER_AREA, horizontal gradient bits → 64 bits, 16 hex chars | |
| Shake kernel | `k = max(3, int(fps/2) | 1)` (odd, ≈ 0.5 s) | Moving average |
| Representative JPEG | 320 px wide, JPEG quality 85, `<framesDir>\<index>.jpg` | |

### Algorithm (`analyze`)

1. Per frame: downscale to 320 wide → `AdaptiveDetector.process_frame` (cuts); further downscale to 160 wide grey float → `cv2.phaseCorrelate` with the previous frame; keep `(frame, dx, dy)` if response > 0.1.
2. Every `step` frames: `frame_metrics` on the full-size proxy frame in grey; keep the BGR frame in memory for possible representative use.
3. After the loop: `detector.post_process`; error if 0 frames decoded.
4. Scene bounds = `[0] + sorted unique cuts in (0, n) + [n]`.
5. Per scene: samples with `a ≤ frame < b`; representative = sample nearest the middle frame (none if the scene has no sample — `ponytail:` comment); shake = `shake_score` over shifts with `a < f < b`.
6. `shake_score`: < 5 shifts → 0.0; else std of `‖shift − moving average‖` divided by `MOTION_WIDTH` (fraction of frame width). Smooth pans low, handheld jitter high.

`frame_metrics` values: `sharpness` = variance of Laplacian (1 dp; resolution-dependent, compare only at proxy 540p), `brightness` = mean/255 (4 dp), `contrast` = std/255 (4 dp), `clipLow`, `clipHigh` (4 dp), `black`, `dhash`.

### Output

```json
{"version": 1, "quality": "balanced", "fps": 29.97, "frames": 900,
 "scenes": [{"index": 0, "startMs": 0, "endMs": 4000, "shake": 0.00123, "representative": 60,
             "framePath": "...\\frames\\12\\0.jpg",
             "samples": [{"atMs": 0, "sharpness": 123.4, "brightness": 0.45, "contrast": 0.2,
                          "clipLow": 0.0, "clipHigh": 0.0, "black": false, "dhash": "0f1e2d3c4b5a6978"}]}]}
```

Rust (`analysis::store`) keeps `version`, `quality`, and per scene `index`, `startMs`, `endMs`, `shake`, `framePath`, `samples`; `fps`, `frames`, `representative` are dropped. See [DATABASE.md](DATABASE.md) `scenes`.

### Self-check

`python analysis.py` (in the worker venv; needs ffmpeg on PATH or `%LOCALAPPDATA%\Microsoft\WinGet\Links\ffmpeg.exe`). Builds a 6 s 640×360 30 fps clip: 2 s `testsrc2`, 2 s black, 2 s blurred and crop-shaken `mandelbrot`. Asserts: 3 scenes; cuts within 70 ms of 2000 and 4000 ms; black scene samples all `black`, first scene none; median sharpness scene 1 > 5 × scene 3; shake scene 3 > 5 × scene 1 and black scene < scene 3 / 5; all frame JPEGs exist; dHash of a gradient vs its mirror differs in > 50 bits.

## 5. tagging.py — OpenCLIP tags

| Aspect | Behaviour |
|---|---|
| Model files | `<modelDir>\openclip_image.xml` (+ `.bin`) and `<modelDir>\openclip_tags.json` (from §6), installed by the Model Manager as `openclip-vit-b32` |
| Preprocess (`preprocess`) | BGR→RGB; shortest side → `size` (224) bicubic; centre crop 224×224; /255; `(x − mean) / std` from `openclip_tags.json`; CHW; batch of 1 → `1×3×224×224` float32 (matches open_clip) |
| Devices (`compile_model`) | Tries `devices` in order (default `GPU`, then `CPU`); skips devices not in `available_devices`; compiles and checks a zero input gives finite output; first success wins. Failures are collected as `fallbacks`. None works → `RuntimeError "No device could run the model: …"` |
| Cache | `_compiled[(xml, device)]` keeps compiled models for the worker's lifetime |
| Sampling | `duration_ms = frame_count / fps × 1000`; times `0, everyMs, 2·everyMs …` below the duration (at least t=0); seek with `CAP_PROP_POS_MSEC`; unreadable frames skipped; `check_cancel` every 10 samples; progress every sample |
| Scoring | Image embedding L2-normalised; `logits = text_embeddings · v × logitScale`; softmax over **all** tags (content + negative) |
| Camera cues (model v3) | For each `cues` entry of `openclip_tags.json` (`embedding` + `contrast`): `cue_score` = two-way softmax of the prompt against its contrast (float64, overflow-safe), 4 dp, per sample in `samples[].cues`. Cues: "view ahead" (v3: road and landscape ahead visible vs. camera tilted down at handlebars / ground); v4 adds "forest trail", "small bridge", "slope" (scene cues used by the Editing Core on trails). Kept out of the tag softmax so it is not diluted |
| Top-k | `TOP_K` = 5 per sample, `[index, probability (4 dp)]`, best first |

Output:

```json
{"model": "ViT-B-32 laion2b_s34b_b79k", "device": "GPU", "fallbacks": [], "everyMs": 2000, "seconds": 12.3,
 "tags": [{"en": "beach", "vi": "bãi biển", "negative": false}],
 "cues": [{"en": "view ahead", "vi": "thấy đường phía trước"}],
 "samples": [{"atMs": 0, "tags": [[0, 0.4123], [1, 0.2011]], "cues": [0.9731]}]}
```

Rust (`tagging::store`) keeps `device`, `tags`, `cues`, `samples` (see [DATABASE.md](DATABASE.md) `media_tags`).

Self-check: `python tagging.py` — preprocess output shape/dtype for 540×960, 960×540 and 100×100 inputs, the normalised value of a white pixel, and `cue_score` on unit vectors. Does not load a model.

**tagging.py** (2026-11-12): `Tagger(model_dir, devices)` holds the compiled encoder and embeddings; `sample(bgr, at_ms)` → one sample; `result(samples, every_ms, seconds)` → the shared result shape; `tag_video` and `tag_image(bgr, model_dir)` (photos, one sample at 0 ms) use it.

## 5b. photo_ai.py — local AI photo tools (ADR 0006 B2–B4, 2026-11-12)

`MODELS` (paths per job from `photos::with_models`; a missing key turns that tool off and its edits do nothing):
`face` YuNet ONNX (OpenCV `FaceDetectorYN`, the 4 rotations so lying-down selfies are found), `landmarks` MediaPipe
`.task` (the face-mesh TFLite is unzipped to `%TEMP%` and run on the OpenVINO CPU, 256×256 eye-aligned crop → 478
points), `sam` EfficientSAM-Ti encoder + decoder (OpenVINO CPU, long side 1024, embeddings memoised per image),
`inpaint` MI-GAN pipeline v2 (OpenCV DNN CPU: OpenVINO's CPU plug-in rejects it and its GPU output is wrong), `depth`
Depth Anything V2 Small and `matte` MODNet (OpenVINO GPU → CPU, ImageNet / 0.5 normalisation from the publishers'
preprocessor configs). OpenVINO compiled blobs are cached in `%TEMP%\memora-openvino-cache`.

- `retouch(x, blemish, smooth, light)`: per face, skin mask = mesh face oval minus eyes / brows / lips (YuNet ellipse
  fallback), gated by the cheek skin colour; spots darker or redder than their median surroundings, sized 1.2–14 % of the
  eye distance and not elongated → Telea inpaint; smoothing = guided-filter base + 25 % detail + fine texture kept;
  gentle warm lift. Identity unchanged (nothing generated).
- `remove(x, objects, painted)`: clicks → EfficientSAM masks (best IoU of 3) grown 1.2 % → union with painted strokes
  → MI-GAN fill, feathered; memoised per image + mask. `photo.erase` (classical) remains the fallback for strokes.
- `bokeh(x, strength)`: nearness from depth (2–98 % stretch); subject = MODNet matte when the photo has faces, else the
  nearest depth; lens (disc) blur in 3 levels growing with distance, on a reduced copy, normalised so subject colours do
  not bleed; radius 2.5 % of the long side at full strength.
- Measured on this PC (owner photos, 1600 px, single runs, warm): retouch ≈ 0.4 s, click-remove 0.1–0.2 s (cached
  mask), background blur ≈ 0.5 s, full-size (2560 px) removal 1.6 s; first use 10–15 s (model compile).
- Self-check `python photo_ai.py` (no models: every tool is a no-op).

## 5a. photo.py — photo editor (ADR 0006, slice B1)

Classical, deterministic, no model; the original is only read (`np.fromfile` + `cv2.imdecode`, EXIF orientation applied; Unicode paths work). Edit values −100..100, unknown keys ignored, radii scaled from a 1600 px reference so previews match full-size exports. Last two decodes cached for live previews.

Order of operations (`render`): geometry (rotate 90° steps, straighten with zoom so no corners show, centred aspect crop) → denoise (guided filter on luma, strong on chroma) → white balance (temperature / tint gains, luminance kept) → exposure (±2 stops in linear light) → dehaze (dark-channel prior + guided filter; negative adds haze) → highlight roll-off above 0.85 (no burnt skies) → Lab tone (only when a tone/colour slider is set): shadows / highlights weighted by a guided-filter local brightness, blacks / whites end points, S-curve contrast, clarity (bounded `tanh` detail, no halos), edge-masked unsharp mask (flat / noisy areas not sharpened), saturation and vibrance (skin hues protected) → HSL per 8 bands → **sky** → look (`none, warm, cool, vivid, cinematic, bw, vintage`) → vignette → optional sky-mask overlay.

**Sky** (owner 2026-11-12: enhance the photo's own sky, cinematic but natural, never paste one): `sky_mask` at 512 px — sky-like = bright, not foliage (green hues), and smooth or blue or cloud-white; kept only as an unbroken run from the top of each column starting in the top 10 %; narrow downward spikes (trunks, white shirts) cut to the running median skyline; guided-filter soft edge limited to the dilated hard mask. `enhance_sky`: cloud texture (bounded local contrast), slightly deeper bright sky, chroma × 1.35, optional warmth — blended by the mask × strength. Thresholds are initial values checked on stills from the owner's footage (frame boards); a click selection comes with slice B2.

`auto(path)`: exposure towards a mid-grey median (70 %, ±60), white balance from near-neutral pixels only at half strength, contrast for flat photos, highlights −25 / shadows +20 when clipped, clarity 10, vibrance 15, sharpen 20, sky 35 when the sky covers > 8 %. `taken(path)`: minimal EXIF reader (DateTimeOriginal + OffsetTimeOriginal, else `local_offset()` of this PC → `…+07:00`). Since 2026-11-12 `render` also applies `erase` (painted strokes) / `remove` (clicks) right after geometry, the face keys `blemish` / `smooth` / `faceLight`, and `bokeh` after the look (all via photo_ai.py); `auto` adds blemish 60 / smooth 35 / faceLight 15 when it finds faces. `analyze(path, edits, model_dir)`: the edited photo at `ANALYZE_SIDE = 960` px (8-bit BGR, the video-proxy scale) → `analysis.frame_metrics`, plus `tagging.tag_image` when a model folder is given (measured on this PC: ≈ 0.14 s per photo on the GPU after a 1.4 s first compile). Self-check `python photo.py` (neutral stack, exposure, white balance, sky mask on a synthetic scene, edge-only sharpening, geometry, looks, extremes, Unicode file names, auto, no-EXIF date).

## 6. tools/clip-export (dev-only)

- Runs only in `tools/clip-export/.venv` (torch venv; torch is never shipped; installing torch needs separate approval per CLAUDE.md). Sets `HF_HUB_OFFLINE=1`; reads a local verified checkpoint.
- Usage: `python export_openclip.py <checkpoint.safetensors> <out dir> [frames dir]`. Model `ViT-B-32`, `laion2b_s34b_b79k` (see `docs/MODEL_PROPOSAL.md`).
- Produces:
  - `openclip_image.xml/.bin` — `model.visual` converted with `ov.convert_model`, reshaped static `[1,3,224,224]`, saved with OpenVINO's default FP16 weight compression.
  - `openclip_tags.json`:

```json
{"model": "ViT-B-32 laion2b_s34b_b79k", "template": "a photo of {}.", "logitScale": <float>,
 "preprocess": {"size": 224, "resize": "shortest side, bicubic, center crop", "mean": [..3], "std": [..3]},
 "tags": [{"en": "beach", "vi": "bãi biển", "embedding": [floats, 6 dp]},
          {"en": "finger over the lens", "vi": "ngón tay che ống kính", "negative": true, "embedding": [..]}]}
```

  (`logitScale` = `model.logit_scale.exp()` of the checkpoint; embeddings are `encode_text` outputs, L2-normalised, rounded to 6 decimals.)
- Taxonomy (counted in code): **`TAGS` = 48** bilingual content concepts, prompted with `TEMPLATE` `"a photo of {}."`; **`NEGATIVE` = 4** bad-shot detectors with their own prompts (`finger over the lens`, `blurry shot`, `camera pointing at the ground`, `too dark`), flagged `"negative": true`. Total 52 entries, negatives last. Text embeddings L2-normalised.
- Consistency checks (CPU): up to 40 `*.jpg` from `[frames dir]` (recursive), else 8 random tensors; IR vs PyTorch image embeddings must have **min cosine ≥ 0.999** (comment: assumed threshold) and **top-tag agreement = 1.0**; prints `{images, realFrames, cosineMin, cosineMean, topTagAgreement}`.
- Output is installed via the Model Manager "Install from files" (job `models.install` with `files`), validated by `validate_openvino_ir`.
- Pinned in [tools/clip-export/requirements.lock.txt](../../tools/clip-export/requirements.lock.txt) (39 packages), key ones: `torch==2.15.0`, `torchvision==0.29.2`, `open_clip_torch==4.2.1`, `openvino==2026.4.1`, `numpy==2.5.4`, `pillow==13.0.0`, `safetensors==0.8.0`, `timm==1.0.31`, `huggingface_hub==2.4.0`.

## 7. Worker dependencies (`workers/ai-worker/requirements.lock.txt`)

| Package | Version | Why |
|---|---|---|
| numpy | 2.5.4 | Arrays (all jobs) |
| opencv-python-headless | 5.0.0.93 | Decode, resize, Laplacian, phase correlation, JPEG (analysis, tagging) |
| openvino | 2026.4.1 | Inference runtime (`validate_devices`, `validate_openvino_ir`, `tag_video`, `photo_analyze`, `hello`) |
| openvino-telemetry | 2025.2.0 | Dependency of openvino; used only by the model converter (`ov.convert_model`, dev-time `tools/clip-export`), not by inference. Opted out on this PC 2026-11-12 (`%LOCALAPPDATA%\Intel Corporation\openvino_telemetry` = `0`, via `python -m openvino_telemetry.opt_in_out --opt_out`) |
| scenedetect | 0.7.2 | `AdaptiveDetector` (analysis) |
| click | 8.5.1 | Dependency of scenedetect |
| colorama | 0.4.6 | Dependency (click/tqdm on Windows) |
| platformdirs | 4.14.5 | Dependency of scenedetect |
| tqdm | 4.70.1 | Dependency of scenedetect |

Python 3.13 (dev venv 3.13.17). Never pip-install into system Python. Stdlib only in `memora_worker.py` itself (`ctypes`, `msvcrt`, `json` …); heavy imports happen inside job functions.

## 8. How to add a worker job

1. **Python**: write `def my_job(job)` in `memora_worker.py` (or a module imported inside the function, like `analysis`/`tagging`, so start-up stays light). Read `job.params`; return a JSON-serialisable dict; raise on failure (becomes `job.error`).
2. **Never block on stdin** and never start a reader thread. Call `job.check_cancel()` between batches (each long native call delays cancel; Rust kills the worker after `CANCEL_GRACE` = 10 s). Report `job.progress(done, total)`. Pass large data by file path.
3. Add it to the `JOBS` dict. Bump `PROTOCOL` (both `memora_worker.py` and `worker::PROTOCOL`) only if the message format changes, not for a new job.
4. **Rust caller**: either a direct kind (not starting with `media.`/`models.` → routed to the worker by `lib::job_runner`; add it to `UI_JOB_KINDS` only if the UI may submit it), or a sub-job built inside a Rust job and run through the `ai` runner (pattern: `analysis::analyze_job`, `tagging::tag_job`, `models::install_job`). Validate the result in Rust before storing (serde struct, like `analysis::WorkerScene`).
5. **Tests**: an assert-based self-check under `if __name__ == "__main__"` in the Python module; a Rust unit test for storing/parsing the result; optionally an integration test that skips when `resolve(None)` fails (pattern: `worker::tests`).
6. New pip dependency → user approval first; pin it in `requirements.lock.txt`; it must also reach the packaged runtime.
7. **Docs**: this file (job table, algorithm/constants, dependencies), [DATABASE.md](DATABASE.md) if results are stored, [BACKEND.md](BACKEND.md) for the Rust caller, `docs/PROGRESS.md`.
