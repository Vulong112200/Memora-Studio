# ADR 0006 — Photo editor (non-destructive, classical first, local AI by approved slices)

Status: **accepted for slice B1** (2026-11-12, owner: "thực hiện tiếp … xử lý chỉnh ảnh"); B1 implemented, awaiting review.
Generative "change the scenery by text" is deferred; order of work Video → UI → Photo. Model downloads (B2–B6)
need a separate approval (CLAUDE.md).

## Context
The owner wants to edit photos in Memora Studio: exposure / light, clarity without halos or noise, removing
unwanted people and objects, changing scenery, colours and the sky, removing freckles and acne, beautifying a
subject on request, and camera-like background blur. Rules that apply: originals are never modified; CPU fallback
is mandatory; GPU / NPU validated at runtime; one heavy model in memory, no AI during a render; record source,
licence and SHA-256 of every model; no cloud, no API keys; the frontend never runs FFmpeg or models.

Research (2026-11-11, licences read on the official pages; speeds are reported by others, not measured here):
most of the list is classical image processing (OpenCV + numpy, already in the worker); only object removal,
click selection, subject matte, depth and AI upscaling need models, and permissive models exist for each.
CodeFormer (S-Lab, non-commercial), GFPGAN (non-commercial parts, redraws faces), RMBG-1.4/2.0 (non-commercial),
SegFormer (NVIDIA research licence), Depth Anything V2 Base/Large (CC BY-NC) and FastSAM (AGPL) are rejected.

## Decision
1. **Separate photo table** (migration 10): `photos (id, path UNIQUE NOCASE, size, modified_unix, width, height,
   taken_at, edits_json, imported_at)` — the edit stack lives in the photo row (one table is enough for one stack). The video `media`
   table stays video-only. Round 1 formats: JPEG, PNG, TIFF, WebP (OpenCV, EXIF orientation honoured); HEIC/RAW later.
2. **Non-destructive edit stack**: `edits_json` is an ordered list of adjustments and AI layers. AI results
   (masks, removal patches, depth) are files in `<data root>\cache\photo\<id>\` referenced by the stack. Export
   renders original + stack to `<data root>\photos\exports\<name>-edit.jpg` via `.part` + rename, never replacing a file.
3. **Worker `photo.py`** (OpenCV + numpy) renders the stack. **Live preview lane**: a second worker process
   (reusing `worker::Worker::spawn`) answers slider previews at ≤ 1600 px directly, so previews are not queued
   behind long video jobs; AI steps remain normal jobs in the FIFO queue (one heavy model at a time).
4. **Classical tools first (slice B1, no model)**: exposure, contrast, highlights / shadows, whites / blacks,
   temperature / tint, vibrance / saturation, HSL per colour band, clarity (guided-filter base/detail), dehaze
   (dark channel), edge-masked sharpening (flat, noisy areas are not sharpened), luma / chroma denoise, vignette,
   crop / rotate / straighten, "Auto" (levels + grey-world white balance + mild CLAHE), looks shared with video.
5. **Local AI by slices, each after owner approval of `docs/PHOTO_MODEL_PROPOSAL.md`** (official URL, pinned
   revision, licence, training-data note, size, SHA-256) and a measured hardware check:

   | Slice | Feature | Model (licence) |
   |---|---|---|
   | B2 | select by click, remove object / person | MobileSAM (Apache-2.0), MI-GAN 512 (MIT code + weights); LaMa (Apache-2.0) only if needed |
   | B3 | blemish / acne / freckle removal, skin smoothing, face brighten — identity kept | YuNet (MIT, in OpenCV) + MediaPipe Face Landmarker (Apache-2.0, to check) + frequency separation |
   | B4 | portrait bokeh (lens blur by depth, click focus) | Depth Anything V2 **Small** (Apache-2.0), BiRefNet-lite (MIT) |
   | B5 | sky selection by click and region recolour (the classical sky enhancement is already in B1) | masks from B2 + guided-filter refine |
   | B6 | AI detail / upscale / denoise (tiled, strength slider) | Real-ESRGAN general-x4v3 (BSD-3), NAFNet-SIDD w32 (MIT) |
   | later | change scenery by text | SD 1.5 inpainting via OpenVINO GenAI (OpenRAIL-M) — deferred by the owner |

   Models are ONNX converted to OpenVINO IR with `ov.convert_model` in the worker venv where an official ONNX
   exists (no torch); MobileSAM needs the existing dev-only torch venv for export. GPU → CPU fallback as in tagging.

## Owner clarification (2026-11-12): sky
"Sky" means making the photo's **own** sky cinematic, clearer and richer while staying natural — not replacing it
with another picture; reference images are mood examples only. B1 therefore has a classical sky mask (column-run
heuristic, initial thresholds checked on the owner's stills, "Show sky area" overlay) and `enhance_sky` (cloud
texture, deeper bright sky, richer colour, optional warmth). Sky replacement is dropped.

## Defaults taken while questions are open
- EXIF: exports do not carry the camera EXIF yet (OpenCV drops it); the date is read by a small EXIF reader and kept
  in the database. Adding Pillow to the worker venv to copy EXIF waits for the owner's approval.
- Photos are added as files (multi-select in the dialog), like videos; folder scanning was not added.

## Consequences
- B1 gives a useful editor with no download; AI features arrive one reviewed slice at a time.
- A second worker process costs memory only while the photo editor is open (stopped when it closes).
- Speeds for models on the Arc 140T are unknown until measured; nothing is promised before that.
