# ADR 0004 — Local AI observations and the Editing Core

Status: accepted by the owner's "continue to complete the project" (2026-11-08), awaiting review.

## Context
Phase 5 adds speech and visual-concept observations; Phase 6 turns all observations into an editable plan.
The owner's footage is long continuous action-cam shots (DJI, 30 s – 5 min, one scene each), wind-heavy audio,
Vietnamese speech.

## Decisions

### Speech (slice 5.2 / 5.4)
- whisper.cpp `whisper-cli.exe` b5454 (CPU) as a separate process under the Job Object; audio extracted from the
  ORIGINAL to a temporary 16 kHz mono WAV.
- Default decoding hallucinated on wind (small: "Cảm ơn các bạn." for 5 minutes). Mitigations, all measured on owner
  footage (docs/MODEL_PROPOSAL.md §4): `-mc 0 -sns`, Silero VAD (`--vad`, used whenever installed), and dropping runs of
  ≥ 4 identical consecutive segments. Whisper small + VAD is the recommended setting (5 min audio → 33 s on 8 threads).
- Transcripts are observations shown with a "may contain mistakes" note; nothing is rendered from them yet.

### Visual concepts (slice 5.3)
- OpenCLIP ViT-B/32 image encoder as OpenVINO IR + text embeddings precomputed for a fixed bilingual taxonomy (49 tags)
  → no tokenizer / text encoder / torch at runtime; follows the model card's "fixed taxonomy" advice.
- Tags are sampled every 2 s of the proxy (not one frame per scene): long single shots need time-local concepts.
- Device: GPU first, CPU fallback, decided at runtime by compiling and running the model (NPU not used by default).

### Editing Core (Phase 6)
- `editing.rs`, pure and deterministic: same inputs → same plan.
- Candidates: scenes cut into style-length windows. Per window, separate scores (technical from the window's own sample
  frames, semantic from tags vs the style's favoured concepts, speech coverage, favorite) and human-readable reasons.
- Selection: greedy by weighted rank with diversity (same concept ×0.85, same file ×0.9, near an already chosen window
  ×0.6), duplicate scenes skipped, user-excluded files never used, ties → farthest from chosen clips (even coverage).
- Structure: chronological, then the style's opening (establishing shot / strongest hook) and ending clip.
- Five styles (§6.3) define target, aspect, clip range, transitions, music, titles, pacing, opening and ending.
  Music and titles are explicitly "not available yet" (open requirements: licensed music source; libass/freetype).
- Smart mode is the default: plan → review (drop / reorder) → apply to the editable timeline → export.
  "Apply and export" is the Auto path. Recaps = date range (last month / 6 months / year / custom) on generation.
- Export gained centre-crop fill (vertical presets) and crossfades (xfade + acrossfade, clamped to half the shortest clip).

## Revision 2026-11-08 (owner review of the first real exports)
Feedback: one continuous shot was cut into several near-identical parts; conversations were cut mid-sentence;
too few effects; no titles, no music, few user options. Changes, following common editing practice:
- No sentence is ever cut: window edges move to speech boundaries (extend up to 3× the style's max clip length,
  else shrink to exclude the speech, else drop the window).
- No jump cuts inside one unchanged shot: a further window from a used scene needs a different picture (dHash
  distance > 12/64) or a different main concept; at most 1 + scene length/60 s clips per scene; clips of one file
  ≤ 1.5 s apart are merged into one continuous clip. Plans may end below the target rather than repeat content.
- Bad-shot detectors (OpenCLIP v2 tag file, same IR): finger over the lens, blurry, ground, too dark → rank penalty
  (up to −80 %), ⚠ reason, never the opening hook. Found on owner footage: a finger shot had technical score 99.
- Export `Options` (saved per project, set from the style on Apply, all editable): 7 transitions (xfade), 6 colour
  looks, fade in/out, fill/crop, blurred background instead of black bars for mismatched aspect ratios, clip sound
  level, user music (looped, faded, sidechain-ducked under the clips' sound), EBU R128 loudness (−16 LUFS), opening
  title + subtitle, ending text, per-clip captions and speech subtitles via a temporary ASS file (libass, Segoe UI,
  title-safe margins; vertical video keeps text above the bottom 18 %). Vietnamese rendering verified on frames.
- Titles: the app suggests "<main concept> · <dates>"; there is no text-generating AI in the app (no LLM in the MVP).

## Consequences / known limits
- Weights, thresholds and taxonomy are initial assumptions; tune on a larger real library.
- Centre crop only (no subject-aware crop); English-trained CLIP with a fixed tag list (no free-text visual search).
- No bundled music library (the user's own files only), no GPS/location yet; Whisper GPU not evaluated.
