# Model proposal (Phase 5) — approved 2026-11-08 (all items)

Metadata collected 2026-11-08 from the Hugging Face and GitHub APIs (no model file downloaded).
SHA-256 values are the publishers' LFS / release digests; the Model Manager re-verifies them after download.

## 1. Speech: whisper.cpp runtime + Whisper ggml models

| Item | Source (pinned) | License | Size | SHA-256 |
|---|---|---|---|---|
| whisper.cpp CPU x64 binaries `whisper-bin-x64.zip` | github.com/ggml-org/whisper.cpp release `b5454` (= v1.9.5, 2026-10-06) | MIT | 8,928,640 B | `6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb` |
| `ggml-base.bin` (multilingual, default) | huggingface.co/ggerganov/whisper.cpp @ `5359861c739e955e79d9a303bcbc70fb988958b1` | MIT (weights: OpenAI Whisper, MIT) | 147,951,465 B | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |
| `ggml-small.bin` (on demand) | same repo / revision | MIT | 487,601,967 B | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` |

- Runtime install target: `<data root>\runtime\whisper.cpp\b5454\` (like FFmpeg: resolved by Rust, run through the Job Object).
- CPU build only. The release has no Vulkan/SYCL build for Windows x64; a GPU build would mean compiling ourselves — out of scope until CPU numbers are measured.
- Vietnamese quality of `base` is unknown — measured on our test clips, not assumed.

## 2. Vision: OpenCLIP ViT-B/32 (`laion2b_s34b_b79k`)

| Item | Source (pinned) | License | Size | SHA-256 |
|---|---|---|---|---|
| `open_clip_model.safetensors` | huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K @ `1a25a446712ba5ee05982a381eed697ef9b435cf` | MIT | 605,143,316 B | `ac4f8c4b88af6d963118cbf40ad93176d092abbedfcb752601ae1866352656e6` |

- safetensors chosen over `open_clip_pytorch_model.bin` (no pickle code execution).
- **Model-card caveats (owner decision):** the LAION card says *"Any deployed use case of the model — whether commercial or not — is currently out of scope"* and recommends task-specific testing with a fixed class taxonomy; trained/evaluated on **English only**. MIT licence permits use, the card is a recommendation, not a licence term. Consequences for us:
  - text search works in English; Vietnamese queries need translation or a fixed bilingual tag list (proposed: fixed tag taxonomy, matches the card's advice);
  - no face recognition / person identification (already out of scope).
- Export to OpenVINO IR needs **dev-time PyTorch**: a separate venv `tools/clip-export/.venv` (never the worker venv, never system Python) with `torch 2.15.0` CPU wheel, `open-clip-torch 4.2.1` (MIT), `openvino 2026.4.1` (same as worker). Output: `openclip_image.xml/.bin` + `openclip_text.xml/.bin`, installed via the existing "Install from files" path; checksum of the exported IR recorded in `models.json`.
- Consistency check: IR vs PyTorch CPU embeddings on test frames (cosine ≥ 0.999 assumed as pass threshold).

## 3. What approval covers (please answer per item)

1. Download whisper.cpp `b5454` CPU zip + `ggml-base.bin` (~157 MB total).
2. `ggml-small.bin` (~488 MB) — now, or later on demand.
3. Install torch CPU + open-clip-torch in a separate dev venv (~1 GB disk) and download the OpenCLIP checkpoint (~605 MB) for IR export.
4. Accept the OpenCLIP model-card caveats (English-only, fixed tag taxonomy approach).

## 4. Execution log (2026-11-08)

- Owner approved all items ("download everything needed for offline use, check the packages are trustworthy").
- Downloaded from the pinned URLs into `E:\Memora Data\downloads\staging\`; **all SHA-256 values match** the publishers' digests above.
- Antivirus: Windows Defender is disabled on this machine because McAfee is the registered real-time scanner; McAfee has no
  command-line scan, so files were only checked on access (no detection). Attestation check against GitHub skipped (API rate limit).
  whisper.cpp binaries are unsigned (normal for its CI builds); provenance = official release asset + matching GitHub digest.
- whisper.cpp runtime: only `whisper-cli.exe`, `whisper.dll`, `ggml*.dll` copied to `E:\Memora Data\runtime\whisper.cpp\b5454\`
  (no test/SDL/llama/parakeet executables). Smoke test: 39.7 s Vietnamese clip audio, base model, 4 CPU threads → 11.2 s wall time, language auto-detected as vi.
- Whisper base/small: URLs + checksums in `models.json`; verified files pre-staged in `<data root>\models\.downloads\<id>-1\`,
  so "Download" in the app verifies and installs them without network.
- OpenCLIP checkpoint + config in `E:\Memora Data\downloads\staging\`. Export venv `tools/clip-export/.venv` installed by the
  owner (torch 2.15.0+cpu, open_clip_torch 4.2.1, openvino 2026.4.1; `--only-binary :all:`); pinned in `tools/clip-export/requirements.lock.txt`.
- Export (`tools/clip-export/export_openclip.py`, offline): image encoder IR (FP16 weights, 176 MB) + `openclip_tags.json`
  (49 bilingual tags, text embeddings precomputed → no tokenizer/text encoder/torch at runtime). Output in
  `E:\Memora Data\downloads\openclip-export\`, checksums in `models.json`, install via "Install from files".
- IR vs PyTorch CPU on 8 real scene frames: min cosine 0.9999988, top-tag agreement 100 %.
- Tag sanity check on the same frames (visually confirmed): bicycle rides → "xe đạp" 0.97–1.0, forest with a big tree →
  "cây cổ thụ" 1.0. First tag list lacked bicycle/trail/tree and mislabelled them (motorbike, temple) — taxonomy matters.
- Measured image-encoder speed, batch 1, mean of 30 runs after warm-up (2026-11-08, this machine):
  CPU compile 321 ms / 22.6 ms per frame; GPU (Arc 140T) compile 1932 ms / 3.8 ms; NPU compile 7667 ms / 11.1 ms.
  NPU works for this model but remains experimental (ADR 0001); CPU stays the fallback.
- Whisper on real footage (DJI_..._0004, 299 s, bike ride with wind noise, Vietnamese speech), 8 threads, CPU:
  base 26.4 s, small 53.0 s wall time (default decoding flags). Default decoding **hallucinates on noise**: small
  output "Cảm ơn các bạn." for the whole 5 minutes. With `-mc 0 -sns` small produced the real conversation
  (≈ readable Vietnamese) except a 14× "Cảm ơn các bạn." loop over the first 30 s of wind; base stays weaker
  (loops "Màu tiên rửi"). The app uses `-mc 0 -sns` and drops runs of ≥4 identical consecutive segments.
- Proposed (not downloaded, needs approval): Silero VAD for whisper.cpp `--vad`, `ggml-silero-v6.2.0.bin` from
  huggingface.co/ggml-org/whisper-vad @ `9ffd54a1e1ee413ddf265af9913beaf518d1639b`, MIT, 885,098 B,
  SHA-256 `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` — skips non-speech before decoding.
