# Memora Studio — Improvement Backlog for Claude Code

**Based on:** Application Overview snapshot, 2026-11-08  
**Commit:** `5f6bb1a` on `main`  
**Status:** Proposed improvements, not approved for implementation.

## 1. Instructions for Claude Code

Review this document together with the current source code, `CLAUDE.md`, `Production.md`, Phase 0 specification, ADRs, and relevant technical documents.

This document contains potential improvements, not confirmed bugs.

For every item:

1. Inspect the current implementation before making conclusions.
2. Classify it as:
   - `CONFIRMED_ISSUE`
   - `NEEDS_INVESTIGATION`
   - `IMPROVEMENT`
   - `NOT_APPLICABLE`
3. Provide source-code evidence, file paths, and relevant line numbers.
4. Explain user impact and technical risk.
5. Recommend a solution and acceptance criteria.
6. Identify required tests and possible database or compatibility impact.

**Important restrictions:**

- Start in read-only mode.
- Do not modify source code before explicit approval.
- Do not install dependencies or download models.
- Do not change drivers, environment variables, Windows settings, or permissions.
- Do not scan personal folders or upload media.
- Do not start Phase 7 packaging or installer work.
- Never modify original media.
- Final exports must always use original media, never proxies.
- Preserve local-first, offline-capable, AI-optional architecture.
- No cloud APIs, API keys, CUDA, or local LLM.
- CPU fallback remains mandatory.
- Prefer small, focused changes over broad refactoring.

## 2. P0 — Reliability and Data Safety

### 2.1 Missing or moved media

Investigate:
- What happens when an original video is moved, renamed, or deleted?
- What happens when an external drive is disconnected?
- Can projects still open when some source files are unavailable?
- Can users relink missing files without losing edits?

Potential improvements:
- Explicit media availability states.
- Relink workflow.
- Preserve project metadata when source files are temporarily unavailable.
- Never silently substitute proxies for missing originals.

### 2.2 SQLite backup and restore

Investigate SQLite WAL backup behaviour.

Potential improvements:
- Safe database backup while the application is running.
- Backup integrity validation.
- Restore workflow with pre-restore backup.
- Recovery instructions for corrupted databases.

Do not assume copying only `memora.db` is sufficient while WAL is active.

### 2.3 Job interruption and crash recovery

Investigate:
- In-memory job history.
- Stale jobs after app crashes.
- Temporary file cleanup.
- Process termination during shutdown.

Potential improvements:
- Explicit interrupted job state.
- Safe startup reconciliation.
- Preserve successful exports.
- Do not automatically resume non-idempotent jobs without validation.

### 2.4 Process cancellation

Verify that cancelling a job terminates all child processes:

- FFmpeg
- FFprobe
- whisper-cli
- Python AI worker

Test cancellation during import, proxy generation, analysis, tagging, transcription, rendering, and validation.

No orphan process should remain after cancellation or application shutdown.

### 2.5 Export safety

Investigate:
- Disk-full handling.
- Permission errors.
- Existing output filename collisions.
- `.part` cleanup.
- Crash during final rename.
- Whether validation happens before promoting temporary output.

Never overwrite a successful export without explicit user consent.

### 2.6 Data-root management

Investigate changing the configured data root after the application has already been used.

If data migration is supported, it should have:
- Copy.
- Verification.
- Switch.
- Rollback.

Never automatically delete the old data root.

## 3. P1 — Editing Core Quality

### 3.1 Separate quality dimensions

Current technical scoring includes sharpness, exposure, brightness, contrast, and stability.

Evaluate whether the system should keep these dimensions separate from:

- Semantic relevance.
- Speech relevance.
- User preference.
- Memory value.
- Story contribution.

Do not automatically introduce new scoring weights without evidence and approval.

Important memories should not be discarded solely because they are technically imperfect.

### 3.2 Clip diversity

Investigate whether generated plans contain too many visually similar clips.

Evaluate:
- Perceptual similarity.
- Same-file repetition.
- Same-concept repetition.
- Distribution across recording dates.
- Distribution across source videos.

Potential improvement: generate a diversity report explaining the selected clips.

### 3.3 Story structure

Current logic primarily uses chronological ordering with opening and ending selection.

Evaluate deterministic story roles:

1. Hook.
2. Establishing scene.
3. Journey.
4. Highlights.
5. Closing.

Prefer existing tags, timestamps, transcripts, and scores. Do not introduce an LLM dependency.

### 3.4 Transition compatibility

Investigate whether consecutive clips have suitable visual and audio continuity.

Evaluate:
- Lighting changes.
- Motion direction.
- Similarity between adjacent scenes.
- Abrupt audio changes.
- Transition duration and clip boundaries.

### 3.5 Speech-safe editing

Test:
- Overlapping transcript segments.
- Very long speech segments.
- Low-confidence transcription.
- Segments outside scene boundaries.
- Missing transcript.

Ensure plan generation always terminates within bounded work.

## 4. P1 — Media Engine and Export

### 4.1 HDR

Current documented limitation: HDR is not tone-mapped.

Evaluate:
- HDR detection.
- Colour metadata reporting.
- Warning when exporting HDR sources to SDR.
- Future tone-mapping support.

### 4.2 Audio

Investigate:
- A/V synchronization.
- Loudness and true peak.
- Silence and clipping.
- Multiple audio streams.
- Silent videos.
- Music ducking and speech continuity.

An audio stream should not be required if silent export is intentionally supported.

### 4.3 Framing

Current behaviour uses centre crop, padding, or blurred background.

Potential improvements:
- Manual crop positioning.
- Persistent per-clip crop settings.
- Preview/render consistency.

Subject-aware crop can remain a later enhancement.

### 4.4 Encoder and validation

Investigate:
- QSV fallback behaviour.
- Actual output FPS.
- Pixel format.
- Rotation metadata.
- Colour metadata.
- Audio/video stream validation.
- A/V synchronization.

The export report should describe the encoder actually used, including fallback reasons.

## 5. P1 — AI Model and Worker

### 5.1 Model validation levels

Distinguish:

- `loadable`: model loads.
- `runnable`: inference completes.
- `numerically_valid`: output shape and values are valid.
- `functional_validated`: representative functional tests pass.
- `performance_measured`: device performance has been measured.

Do not mark a model validated merely because inference succeeds on zero input.

### 5.2 OpenCLIP taxonomy

Current documented taxonomy: 48 concept tags and 4 negative detectors.

Evaluate:
- Independent taxonomy versioning.
- User correction of incorrect tags.
- Custom tags as a future feature.
- Model and taxonomy provenance.

### 5.3 Whisper

Evaluate:
- Confidence or uncertainty display.
- Manual transcript correction.
- Transcript-to-preview navigation.
- Vietnamese and English quality tests.
- Handling of low-confidence transcript segments.

### 5.4 Worker lifecycle

The worker is currently not stopped when idle.

Measure:
- Baseline memory.
- Peak memory during analysis.
- Peak memory during tagging.
- Peak memory during transcription.
- Memory after jobs complete.
- Idle memory after 5–10 minutes.

Only then consider idle timeout or model unloading.

Keep the current one-heavy-model-at-a-time policy unless separately approved.

## 6. P1/P2 — Data Versioning

Investigate whether results record enough provenance for:

- Media fingerprint version.
- Scene detection version.
- Scoring algorithm version.
- OpenCLIP model version.
- Tag taxonomy version.
- Whisper model/runtime version.
- Editing algorithm version.
- Export pipeline version.

The objective is to identify stale results and avoid unnecessary re-analysis.

Do not discard user data during migrations.

## 7. P2 — UI/UX

Evaluate these improvements based on actual workflow friction.

### 7.1 Workflow navigation

Potential workflow:

1. Library.
2. Review.
3. Create.
4. Edit.
5. Export.

These may be sections or separate navigation destinations. Do not introduce a router or state library without a clear need.

### 7.2 Explainable Editing Plan

Show:
- Why a clip was selected.
- Why a clip was skipped.
- Relevant score components.
- Story role, if available.
- Whether a user mark influenced selection.
- Whether analysis or AI data was unavailable.

### 7.3 Editing usability

Evaluate:
- Undo/redo.
- Autosave.
- Save status.
- Keyboard shortcuts.
- Batch selection.
- Better loading and error states.
- More useful preview layout.

### 7.4 UI language

UI language remains an owner decision. Do not translate the interface without approval.

## 8. P2 — Quality Benchmark

Propose regression tests for:

- Media compatibility.
- Scene detection.
- Blur, shake, black frame, duplicates.
- Vietnamese and English transcription.
- OpenCLIP tag quality.
- Clip diversity.
- Chronological order.
- Speech-safe cuts.
- A/V synchronization.
- Render integrity.
- Cancellation and crash recovery.
- Disk-full and permission errors.
- Memory and CPU usage.

Prefer synthetic or licensed test media. Do not commit personal videos to Git.

Also propose a repeatable human evaluation rubric:

- Important moments retained.
- Redundant clips reduced.
- Pacing feels appropriate.
- Cuts do not interrupt important actions.
- Opening is engaging.
- Ending feels intentional.
- Result feels like a coherent story.

## 9. Documentation discrepancies

Verify the six discrepancies reported in the Application Overview:

1. 49 bilingual tags in documentation versus 48 concepts + 4 negatives in code.
2. `openclip-vit-b32` has `validated: false` despite recorded measurements.
3. VAD comment in `transcribe.rs` says approval is pending although VAD is implemented.
4. `NO_MUSIC` / `NO_TITLES` constants conflict with export capabilities.
5. `PROGRESS.md` contains stale next steps.
6. `HARDWARE_VALIDATION.md` contains stale OpenCLIP/whisper.cpp status.

Verify each discrepancy against current code before editing documentation.

## 10. P3 — Future Features

These are ideas, not implementation commitments:

- Storyboard / visual story map.
- Audio-aware editing.
- Manual and subject-aware reframing.
- Memory timeline.
- Six-month and yearly recaps.
- Custom tags.
- Export queue and custom presets.
- GPS/location grouping, subject to privacy design.

Do not add a local LLM merely to support these features.

## 11. Packaging Gate — Not Approved

Phase 7 packaging remains unapproved.

Before requesting approval, prepare a separate readiness report covering:

- FFmpeg GPL distribution obligations.
- Third-party dependency licenses.
- Model licenses.
- Embedded Python redistribution.
- Clean-machine installation.
- Upgrade and uninstall behaviour.
- Preservation of user data.
- Offline operation.

Do not build an installer as part of this review.

## 12. Required Claude Code Output

After the read-only audit, provide:

### A. Executive summary
- Top 5 confirmed risks.
- Top 5 highest-value improvements.
- Current internal-use readiness.
- Items required before packaging approval.

### B. Findings table

| ID | Priority | Classification | Area | Evidence | User impact | Recommendation |
|---|---|---|---|---|---|---|

### C. Proposed backlog

For each accepted item:
- Goal.
- Scope.
- Out of scope.
- Acceptance criteria.
- Tests.
- Dependencies.
- Database/data impact.
- Complexity: S/M/L.
- Suggested implementation order.

### D. Documentation reconciliation

Identify stale or conflicting documents and recommend one authoritative source for each fact.

### E. Approval gate

Ask the owner which findings should be approved for implementation.

**Do not begin implementation before explicit approval.**

---

## Final principle

The objective is not to make Memora Studio more complicated.

The objective is to make it safer for personal memories, more consistent in the videos it creates, easier to understand, and more dependable on the verified Windows hardware.

**Inspect first. Prove the issue. Propose the smallest safe change. Wait for approval.**