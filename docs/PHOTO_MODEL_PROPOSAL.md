# Photo model proposal (ADR 0006, slices B2–B6)

**Status 2026-11-12:** owner approved the recommended set ("Tải cả bộ"). Downloaded, size + SHA-256 matched, then the
load spike on this PC: YuNet (OpenCV), Face Landmarker mesh (OpenVINO CPU), EfficientSAM-Ti (OpenVINO), Depth Anything
V2 Small and MODNet (OpenVINO GPU, CPU fallback) work; MI-GAN pipeline v2 works in OpenCV DNN (OpenVINO CPU rejects it,
GPU output wrong); **Real-ESRGAN TFLite fails in both OpenVINO and OpenCV → not installed** (B6 open: needs another
ONNX build, e.g. the Qualcomm export, after a new check). Installed through the Model Manager (models.json).

Metadata collected 2026-11-12 from the Hugging Face model API (`?blobs=true`), GitHub release pages / HEAD
requests, the Google Cloud Storage HEAD response and the official READMEs / LICENSE files. **No model file was
downloaded.** SHA-256 values below are the publishers' Hugging Face LFS digests; the Model Manager re-verifies
size + SHA-256 after download. "unverified" = not stated on the source page; it must be checked in a spike.

## 1. Purpose

Local, offline AI photo editing for the Photos page (ADR 0006), running in the existing Python worker
(`openvino` 2026.x, `opencv-python-headless`, `numpy`; no torch at runtime) on CPU with optional Arc GPU:

| Slice | Feature | Needs |
|---|---|---|
| B3 | blemish retouch / skin smoothing (identity kept) | face box + skin mask that excludes eyes, brows, lips |
| B2 | click/box select + object removal | promptable segmentation + inpainting |
| B4 | portrait background blur | depth map + subject (person) matte |
| B5 | sky selection by click | reuses the B2 segmenter (no new model) |
| B6 | upscale / denoise | super-resolution; denoise |

All recommended files are ONNX or TFLite. OpenVINO reads both directly (`core.read_model` / `ov.convert_model`;
the TFLite frontend has shipped since OpenVINO 2023.0), so **nothing in the recommended set needs a torch export**.

## 2. Recommended minimal set

| # | Slice | Model | File | Bytes | Licence (code / weights) | ONNX/TFLite publisher |
|---|---|---|---|---|---|---|
| 1 | B3 | YuNet 2023mar | `face_detection_yunet_2023mar.onnx` | 232,589 | MIT / MIT | **official** (OpenCV) |
| 2 | B3 | MediaPipe Face Landmarker (Face Mesh V2) | `face_landmarker.task` (float16, v1) | 3,758,596 | Apache-2.0 / Apache-2.0 (model card) | **official** (Google) |
| 3 | B2, B5 | EfficientSAM-Ti | `efficientsam_ti_encoder.onnx` + `efficientsam_ti_decoder.onnx` | 24,799,761 + 16,565,728 | Apache-2.0 / Apache-2.0 | author-hosted (exported by K. Wada, in the authors' HF Space) |
| 4 | B2 | MI-GAN 512 Places2 pipeline | `migan_pipeline_v2.onnx` | 28,079,181 | MIT / MIT (`LICENSE-WEIGHTS`) | **official** (first author, linked from the official README) |
| 5 | B4 | Depth Anything V2 Small | `onnx/model.onnx` | 99,060,839 | Apache-2.0 (Small only) | third-party (`onnx-community`, Transformers.js export) |
| 6 | B4 | MODNet (photographic portrait matting) | `onnx/model.onnx` | 25,888,640 | Apache-2.0 / Apache-2.0 | third-party (`Xenova`, Transformers.js export) |
| 7 | B6 | Real-ESRGAN realesr-general-x4v3 | `realesr_general_x4v3.tflite` (fp16) | 3,549,456 | BSD-3-Clause | third-party (HF org `litert-community`, re-authored graph) |

**Total: 201,934,790 B (≈ 193 MiB).** Denoise in the MVP: classical OpenCV `fastNlMeansDenoisingColored`
(already installed) plus the mild denoising Real-ESRGAN general does; NAFNet stays optional (§3.6).

## 3. Per-feature tables

### 3.1 Face detection (B3)

| Field | YuNet 2023mar |
|---|---|
| Source | https://huggingface.co/opencv/face_detection_yunet (OpenCV's official HF mirror of `opencv_zoo/models/face_detection_yunet`) |
| File | `face_detection_yunet_2023mar.onnx` (also `_int8.onnx` 100,416 B, `_int8bq.onnx` 122,489 B) |
| Revision | HF commit `3cc26e7f1014a5ee5d74a42acee58bafc9d0a310` |
| Licence | MIT, Copyright (c) 2020 Shiqi Yu — `LICENSE` in the same repo |
| Size | 232,589 B |
| SHA-256 | `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4` |
| I/O | fixed input shape in the file, but OpenCV DNN infers on the actual image size (README, opencv_zoo issue #44). Output via `cv2.FaceDetectorYN`: box + 5 landmarks + score. Opset: unverified |
| Runtime | OpenCV DNN (`cv2.FaceDetectorYN`, OpenCV ≥ 4.8 for 2023mar — version requirement unverified on page); no OpenVINO needed |
| Notes | README: detects faces **between ~10×10 and 300×300 px** → downscale large photos before detection. WIDER Face val AP 0.884 / 0.866 / 0.750 (README) |

### 3.2 Face landmarks → skin mask (B3)

| Field | MediaPipe Face Landmarker |
|---|---|
| Source | https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task (linked from the official guide https://developers.google.com/edge/mediapipe/solutions/vision/face_landmarker) |
| File | `face_landmarker.task` — a bundle of BlazeFace short-range (192×192), Face Mesh V2 (256×256 → 478 3-D landmarks) and blendshapes (1×146×2) |
| Revision | path version `float16/1`; `Last-Modified: 2023-05-03`; ETag / MD5 `b0e7274907a1644404fef66b28dd6d85` (`latest` currently serves the same object) |
| Licence | MediaPipe code Apache-2.0; Face Mesh V2 model card: "LICENSED UNDER Apache License, Version 2.0" (card text read via a search mirror — the official PDF could not be text-extracted here; re-read after download) |
| Size | 3,758,596 B |
| SHA-256 | not published (GCS publishes MD5 + CRC32C only) — compute after download, pin in `models.json` |
| I/O | Face Mesh V2: 256×256 face crop → 478 landmarks. Tensor names / layout: unverified |
| Without the `mediapipe` package | the `.task` is a bundle archive; the inner `.tflite` files (names reportedly `face_detector.tflite`, `face_landmarks_detector.tflite`, `face_blendshapes.tflite` — unverified) can be extracted with `zipfile` and the landmark model loaded by the OpenVINO TFLite frontend. We would crop with YuNet instead of BlazeFace. **OpenVINO op coverage for this graph is unverified → spike.** |
| Skin mask | fill the face-oval polygon, subtract eye, brow and lip polygons (standard Face Mesh index sets) — no face-parsing model needed |

Alternative (not recommended): BiSeNet face parsing (zllrunning/face-parsing.PyTorch, code MIT, 19 classes incl. skin/brows/eyes/lips, ~13.3 M params). Weights are on Google Drive (no official ONNX); a TFLite port exists at `litert-community/BiSeNet-Face-Parsing-LiteRT` @ `62996ba4ebcfd9a44b45f1dd5325dcd7e4f09955`, `faceparsing.tflite` 52,639,204 B, SHA-256 `23923022a8f4149fe1f5df5426a18f3a202dd605454539ed33c5d76f3035e907`, input 1×3×512×512. Risk: trained on **CelebAMask-HQ**, whose dataset terms are non-commercial research/education (see §4).

### 3.3 Click / box segmentation (B2, B5)

| Field | EfficientSAM-Ti (recommended) | MobileSAM (alternative) |
|---|---|---|
| Source | https://huggingface.co/spaces/yunyangx/EfficientSAM (authors' Space; official README: "ONNX version … including separate encoder and decoder is available on the Hugging Face Space (thanks to @wkentaro … onnx export)") | ONNX: https://huggingface.co/Acly/MobileSAM (third party: Acly). Weights: github.com/ChaoningZhang/MobileSAM `weights/mobile_sam.pt` |
| Files | `efficientsam_ti_encoder.onnx`, `efficientsam_ti_decoder.onnx` (also combined `efficientsam_ti.onnx` 41,365,520 B) | `mobile_sam_image_encoder.onnx`, `sam_mask_decoder_single.onnx` (or `_multi`) |
| Revision | Space commit `d8dbb1eee73bfb3392aa6f6e8944aeb13f3f4036` | `0d3b403339b4674a82493d5e97964dd78089ddc8` |
| Licence | Apache-2.0 (repo `LICENSE`, Space card) | MobileSAM Apache-2.0; HF ONNX repo tagged MIT (mismatch — Apache-2.0 terms apply to the weights) |
| Size | 24,799,761 + 16,565,728 = 41,365,489 B | 28,157,093 + 16,501,323 = 44,658,416 B |
| SHA-256 | enc `84ed466ffcc5c1f8d08409bc34a23bb364ab2c15e402cb12d4335a42be0e0951`; dec `a62f8fa5ea080447c0689418d69e58f1e83e0b7adf9c142e2bd9bcc8045c0b11` | enc `580f5fb648ea1062c0aabc26217aed56921985f03f0cbbd852bba81d760cc749`; dec `93915fc7c993ab9d59ab8c9ccd3bce37f7509c81ab4150a74abd4d2abbd8570d` |
| I/O | points + labels (box = 2 points) → mask logits + IoU; Space code resizes longest side to 1024, `ToTensor()` without mean/std (TorchScript path). ONNX input names / opset: unverified | SAM-style: image encoder 1024 input; decoder takes embeddings + point coords/labels. Opset unverified (MobileSAM README recommends onnx 1.12) |
| Compute | params: unverified | encoder ~5 M params, total 9.66 M (MobileSAM README) |
| OpenVINO | unverified — spike (OpenVINO has had an EfficientSAM notebook converting from PyTorch, not from this ONNX) | unverified — spike |

Other candidates checked:
- **SAM 2.1 hiera-tiny** (facebookresearch/sam2, Apache-2.0, 38.9 M params): ONNX only from `onnx-community/sam2.1-hiera-tiny-ONNX` @ `814a066640debee5a91e70aa401fb8e17e030503` (no licence tag); fp32 `vision_encoder.onnx` + `.onnx_data` 134,084,864 B, decoder 20,958,208 B. ~3.7× the download of EfficientSAM-Ti → keep as upgrade path.
- **SlimSAM-77** (`Xenova/slimsam-77-uniform` @ `5850ab45…`, Apache-2.0): encoder 23,276,014 B + decoder 16,557,892 B. Viable fallback.
- **EdgeSAM** — rejected (§4).

### 3.4 Object removal / inpainting (B2)

| Field | MI-GAN pipeline (recommended) | LaMa big-lama (optional, large areas) |
|---|---|---|
| Source | https://huggingface.co/andraniksargsyan/migan (Andranik Sargsyan = MI-GAN first author; linked from github.com/Picsart-AI-Research/MI-GAN README) | https://huggingface.co/Carve/LaMa-ONNX (third party: Carve / Carve-Photos fork of advimman/lama) |
| File | `migan_pipeline_v2.onnx` (raw generator `migan.onnx` 29,546,882 B also present) | `lama_fp32.onnx` (recommended by publisher) |
| Revision | `406830d0fa60666da0071c342ad2fbc8f30c5c64` (repo modified 2026-09-14 → pin) | `c3c0c9e468934d62e79c329e35d82dd09ff8c444` |
| Licence | code MIT (`LICENSE`), weights MIT (`LICENSE-WEIGHTS`), HF repo MIT | advimman/lama Apache-2.0; HF repo apache-2.0 |
| Size | 28,079,181 B | 208,044,816 B |
| SHA-256 | `6f1f3530a1a2324b19752018ce756088b07973cda8d7d890034ace5c8a48c40b` | `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6` |
| I/O | inputs `image` (uint8 RGB) + `mask` (uint8, **255 = keep, 0 = fill**, binary); pipeline crops around the mask, resizes to 512, blends back; arbitrary resolution (README). Opset unverified | fixed 512×512, opset 17, custom `FourierUnitJIT` (FFT) |
| Compute | params/FLOPs: unverified (repo has `scripts/calculate_flops`) | unverified |
| OpenVINO | unverified — pipeline contains dynamic crop/resize logic → spike; onnxruntime is the publisher's reference | FFT/DFT ops → OpenVINO support unverified → spike before approving |

### 3.5 Portrait background blur (B4)

| Field | Depth Anything V2 Small (depth) | MODNet (person matte) |
|---|---|---|
| Source | weights: https://huggingface.co/depth-anything/Depth-Anything-V2-Small (`.pth` only). ONNX: https://huggingface.co/onnx-community/depth-anything-v2-small | official: github.com/ZHKKKe/MODNet (ONNX export script only, weights on Google Drive). ONNX: https://huggingface.co/Xenova/modnet |
| File | `onnx/model.onnx` (single file, no external data) | `onnx/model.onnx` |
| Revision | `4472b7362082ad9968fee890ca0f1e5aca36b93d` (card points to newer `onnx-community/depth-anything-v2-small-ONNX` @ `c3b67641…`, split `.onnx` + `.onnx_data` 98,798,080 B) | `fa2fa546052fba4c08921230a26cc69a333fca12` |
| Licence | Small: Apache-2.0 (Base/Large/Giant: CC-BY-NC-4.0) | Apache-2.0 ("code, models, and demos") |
| Size | 99,060,839 B (fp16 49,642,442 B) | 25,888,640 B (fp16 12,984,781 B) |
| SHA-256 | `afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c` | `07c308cf0fc7e6e8b2065a12ed7fc07e1de8febb7dc7839d7b7f15dd66584df9` |
| I/O | relative inverse depth; Transformers.js export (H, W multiples of 14, 518 default — unverified for this file) | portrait alpha matte; input size/normalisation in `preprocessor_config.json` — unverified |
| Compute | 24.8 M params (official README) | unverified |
| OpenVINO | unverified for this ONNX; OpenVINO notebooks convert Depth Anything from PyTorch (`depth-anything`) | unverified |

Why both: depth gives graded lens blur for any scene; MODNet keeps hair/shoulder edges sharp for people. For
non-person subjects the B2 segmenter supplies the subject mask.

### 3.6 Upscale / denoise (B6)

| Field | Real-ESRGAN general-x4v3 (recommended) | NAFNet-SIDD-width32 (optional denoise) |
|---|---|---|
| Official weights | github.com/xinntao/Real-ESRGAN release `v0.2.5.0`, `realesr-general-x4v3.pth` 4,885,111 B (no digest published); BSD-3-Clause | github.com/megvii-research/NAFNet (MIT), weights on Google Drive / Baidu; no official ONNX |
| Proposed file | `realesr_general_x4v3.tflite` from https://huggingface.co/litert-community/real-esrgan-x4v3-litert | `nafnet_sidd_width32_fp16.tflite` from https://huggingface.co/litert-community/NAFNet-SIDD-width32-LiteRT |
| Revision | `0a6fc35ba3e34e80b53f43089c1d2b612d33bea9` | `59984b5234b24ccf2f324ce0ac49cd640b9d5b2c` |
| Licence | BSD-3-Clause (card states weights are the official release, only the op graph re-authored) | MIT |
| Size | 3,549,456 B | 62,454,048 B |
| SHA-256 | `6d597b46d812ada42bdde641ec6e78c0ef0b78ff5a825fcfd74b9a1b3bcbe9a4` | `f8fbaa422411683c53e802cf7cc7cf9be0a0de00886ad4af057232e26b172a0c` |
| I/O | fp16; input `[1,128,128,3]` NHWC RGB 0–1 → output `[1,3,512,512]` **NCHW** — fixed size, tile with overlap | fp16; `[1,3,256,256]` RGB 0–1 → same shape; fixed size, tile |
| Compute | ~1.2 M params (card; Qualcomm card: 1.21 M) | unverified |
| Publisher | HF org `litert-community`; conversion records link to a third-party contributor (john-rocky/edge-compat). Graph re-authored (PReLU and PixelShuffle rewritten), card reports corr 1.000 vs original | same contributor; LayerNorm re-authored for fp16 safety. Card: TFLite **XNNPACK (CPU) declined the graph** — OpenVINO CPU behaviour unverified |
| OpenVINO | TFLite frontend — unverified for this graph → spike | unverified → spike; MVP uses OpenCV non-local means instead |

ONNX alternative for Real-ESRGAN: Qualcomm AI Hub (`qualcomm/Real-ESRGAN-General-x4v3` @ `deb4baad8e4992332b996e066e06766110a9559a`, card licence BSD-3-Clause) links `real_esrgan_general_x4v3-onnx-float.zip` v0.66.0 on `qaihub-public-assets.s3.us-west-2.amazonaws.com`, 4,540,895 B (HEAD), ETag `ed2635507dce0a7aefb2f4d89a9275c7`, no SHA-256 published, input 128×128 fixed, contents of the zip unverified. Dynamic-shape ONNX would need a torch export of the official `.pth` (Real-ESRGAN `scripts/pytorch2onnx.py`) — avoidable.

## 4. Rejected models

| Model | Reason |
|---|---|
| EdgeSAM (chongzhou96/EdgeSAM, HF `chongzhou/EdgeSAM`) | **S-Lab License 1.0**: "Redistribution and use for non-commercial purpose …" |
| CodeFormer (sczhou/CodeFormer) | **S-Lab License 1.0** (non-commercial); also generative face redraw changes identity |
| GFPGAN (TencentARC/GFPGAN) | Apache-2.0 *except* third-party parts: StyleGAN2 ops under the **NVIDIA Source Code License** ("non-commercially … research or evaluation purposes only") and code from DFDNet under **CC BY-NC-SA 4.0**; uses a StyleGAN2 face prior = redraws faces (identity drift) — against ADR 0006 "identity kept" |
| RMBG-1.4 / RMBG-2.0 (BRIA) | non-commercial licence |
| Depth Anything V2 Base / Large / Giant | CC-BY-NC-4.0 (only Small is Apache-2.0) |
| `jonathandinu/face-parsing` (SegFormer) | card: "License: non-commercial research and educational purposes"; 340 MB |
| BiRefNet lite (ZhengPeng7/BiRefNet, MIT) | licence fine, but heavy: official release `v1` `BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx` 224,005,088 B (same size as `onnx-community/BiRefNet_lite-ONNX` `model.onnx`, SHA-256 `5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333`), 2K variant 331,082,421 B; decoder uses deformable convolution (OpenVINO import unverified). MODNet + depth cover B4 at ~1/2 the size — keep BiRefNet as a quality upgrade |
| SAM 2.1 tiny | not rejected — deferred as upgrade (size, see §3.3) |

## 5. Open questions (owner decision)

1. **Training-data terms.** MI-GAN and LaMa are trained on Places2 (Places365), BiSeNet on CelebAMask-HQ; both
   datasets are published for non-commercial research. The model licences (MIT / Apache-2.0) do not restrict use,
   but whether dataset terms carry over to weights is a legal question — acceptable for personal use; needs review
   before distribution (together with the pending FFmpeg licence review).
2. **Third-party conversions.** Depth Anything (onnx-community), MODNet (Xenova), Real-ESRGAN / NAFNet
   (litert-community) are conversions by others; SHA-256 pins the exact bytes, but output fidelity must be checked
   against the publisher's sample outputs in the spike. Accept, or prefer torch-based self-export in the existing
   dev-only `tools/clip-export` style venv?
3. **OpenVINO compatibility spike** (before wiring UI): load each file with `core.read_model` on CPU and GPU, run
   one image, compare with onnxruntime/TFLite reference where available. Highest risk: MediaPipe landmark TFLite,
   MI-GAN pipeline (dynamic crop), NAFNet fp16 TFLite, LaMa FFT. Fallback runtime for ONNX that OpenVINO rejects:
   OpenCV DNN (already installed) — adding onnxruntime would be a new dependency needing approval.
4. MediaPipe `.task` has no published SHA-256 — accept "compute after download, then pin" (MD5 from GCS is
   available as a cross-check)?
5. NAFNet now or later? MVP proposal: classical denoise only; NAFNet after the spike shows CPU works.
6. LaMa (208 MB) only if MI-GAN quality is insufficient on large removals — download on demand?
