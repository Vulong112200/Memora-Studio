# ADR 0005 — Free music library (download on use, delete after)

Status: requested by the owner (2026-11-08), implemented, awaiting review.

## Context
The owner wants free background music from the internet without storing a whole library: download a track
only when a video needs it, insert it, then delete the download. Audio files the owner picks from the computer
must never be deleted. The owner also wants to listen before choosing and to get suggestions per video.
Project rules: never trust unverified URLs; record source, licence and checksum; no mandatory accounts/API keys.

## Decision
- Curated catalog `apps/desktop/src-tauri/sounds.json`: 22 tracks by Kevin MacLeod (incompetech.com), licence
  Creative Commons BY 4.0 (verified on incompetech.com/music/royalty-free/licenses/ on 2026-11-08: "Free … Requires
  that you credit the music"). Each track was downloaded once at development time to record size + SHA-256 and
  probe it (MP3 audio, duration), then deleted. No account or API key is involved.
- Mood / energy labels are the developer's assumptions, not measurements — hence streamed preview in the app.
- Export job: a library track is downloaded with curl.exe (Job Object) into `<data root>\cache\downloads\sounds\`,
  size + SHA-256 verified (mismatch → not used), rendered, and deleted when the job ends (RAII `TempFile`, any
  outcome). User files are referenced by path and never copied, modified or deleted.
- Licence credit `"<Title>" Kevin MacLeod (incompetech.com) · Licensed under Creative Commons: By Attribution 4.0`
  is burned in during the last 4 s of every export that uses a library track.
- Preview: `<audio>` streams the track URL directly; the webview CSP allows `media-src https://incompetech.com` only.
- Recommendations (`sounds::recommend`, deterministic): style fit 50 %, on-screen concepts of the timeline clips
  (OpenCLIP tags) 30 %, track at least as long as the video 10 %, low-energy music when > 30 % of the video is speech 10 %.
- Loudness: the single-pass `loudnorm` left real exports at −19.6 LUFS; a second pass now measures the render
  (EBU R128) and applies the exact gain with a −1 dBTP limiter, audio only (picture copied): −16.4 LUFS measured.

## Consequences
- Library music needs internet at export time; offline exports must use the owner's own files (clear error message).
- The catalog is fixed in the app; adding tracks means verifying and recording them the same way.
