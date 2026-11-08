# Memora Studio — Improvement Backlog Audit

**Audited:** [IMPROVEMENT_BACKLOG.md](IMPROVEMENT_BACKLOG.md), against HEAD `2b32635` (2026-11-08). The audit was read-only: no code was changed.
**Classification:** `CONFIRMED_ISSUE` · `NEEDS_INVESTIGATION` · `IMPROVEMENT` · `NOT_APPLICABLE`.
**Evidence paths:** relative to `apps/desktop/src-tauri/src/` unless another folder is named. `*.tsx` files are in `apps/desktop/src/`, and `memora_worker.py`/`tagging.py` are in `workers/ai-worker/`.
**Values marked "assumed"** (timeouts, tolerances) are initial proposals, not measurements.

Note: [APP_OVERVIEW.md](APP_OVERVIEW.md) predates commit `2b32635`. That commit added `sounds.rs` (free music library), a second loudness pass and more tests, so the overview needs a refresh (section D).

---

## A. Executive summary

**Top 5 confirmed risks**
1. **Changing the data root while a job runs corrupts data.** The runner keeps the *old* root's paths but writes to the shared DB handle, which now points at the *new* DB, using media ids from the old one (`lib.rs:373,379,390`). Settings are also saved before the new DB is known to open (`lib.rs:108→110`).
2. **There is no database backup at all, not even before schema migrations** (`db.rs`; grep finds no backup, `VACUUM INTO` or `integrity_check`). A failed migration or a corrupt file loses every project and edit.
3. **Moved or renamed originals cannot be relinked.** Re-importing at the new path creates a *new* row (`ON CONFLICT(path)`, `media.rs:174`). The old row keeps its clips and analysis. The plan generator also includes missing files (`editing.rs:148-168`). Export does fail correctly and never falls back to a proxy (`export.rs:601-602`).
4. **Exports are promoted before validation.** The `.part` file is renamed to the final name *before* `validate` runs (`proxy.rs:155-157` → `export.rs:669`), and a failed file stays at the final name. A same-second filename collision silently overwrites the earlier file (`export.rs:502-506`, `-y`, `fs::rename` replaces). A full disk triggers a second, full libx264 render (`export.rs:552-566`).
5. **Crash and cancel gaps:**
   - Model rows stuck in `downloading`/`installing`/… after a crash. Remove is hidden for those states (`models.rs:260-327`, `Models.tsx:54`).
   - Temp files are never cleaned up at startup.
   - Worker cancel is cooperative with no time limit (`worker.rs:168-172`).
   - Three spawns leak their child if Job Object assignment fails: `media.rs:128`, `export.rs:261`, `models.rs:193`.

**Top 5 highest-value improvements**
1. Media availability flag plus a relink command that keeps the media id.
2. Automatic DB backup before migrations, plus a manual backup.
3. Clamp each transition by its two neighbouring clips. Order clips by full timestamp, not by day + media id.
4. HDR detection with an export warning. The data is already in `probe_json`.
5. Stale-result detection: `media_analysis.version` is stored but never read. Store model versions with tags and transcripts, and add "Re-tag stale".

**Internal-use readiness:** usable by the owner today, with two habits: don't change the data root while a job runs, and don't move originals after import. **Not ready for packaging.** Items 1, 2 and 4 of the risks should be fixed first, and the Phase 7 readiness report (§11 of the backlog) is still missing.

---

## B. Findings

| ID | Pri | Classification | Area | Evidence | User impact | Recommendation |
|---|---|---|---|---|---|---|
| R1 | P0 | CONFIRMED_ISSUE | Missing/moved media | No availability column or check (`db.rs:20`, `media.rs:192-211`); re-import creates new row (`media.rs:174`); `gather` ignores existence (`editing.rs:148`); export fails correctly (`export.rs:601`) | Moving a folder or unplugging a drive loses the link between edits and the video | Computed `available` in `list_media` (no migration); `relink_media(id, path)` that checks size + duration and keeps the id; `gather` skips missing files with a reason; UI badge |
| R2 | P0 | CONFIRMED_ISSUE | DB backup | No backup, checkpoint or integrity code; migrations run on open (`db.rs:137-148`) | One bad migration or a corrupt file loses all projects | `VACUUM INTO database\backups\memora-<ts>-v<N>.db` before any pending migration and from a "Back up" button; `PRAGMA integrity_check` on the copy; keep the last N copies; restore = documented manual steps for now |
| R3 | P0 | CONFIRMED_ISSUE | Crash recovery | No startup cleanup (`lib.rs:354-420`); stuck model states (`models.rs:99`); jobs in memory (`jobs.rs:2`) | After a crash a model looks "downloading" forever and Remove is hidden; temp files pile up | Startup reconciliation: model working states → `install_failed` "interrupted"; delete `cache\render\*`, `cache\audio\*`, `*.part` and `*.level.mp4` in cache/exports; **keep** model download `.part` (resume). No job persistence (YAGNI) |
| R4 | P0 | CONFIRMED_ISSUE (partial) | Cancellation | Worker cancel has no timeout (`worker.rs:168`); ffprobe and `measure_lufs` cannot be cancelled and have no timeout (`media.rs:118` admits it); leak on assign failure (3 sites) | Cancel can hang the queue; a hung ffprobe blocks all jobs | One `process::spawn_assigned` helper that kills the child if assignment fails; worker cancel grace period (assumed 10 s) then kill + restart; ffprobe timeout (assumed 60 s). App exit is already covered by the Job Object (`process.rs:17-30`) |
| R5 | P0 | CONFIRMED_ISSUE | Export safety | Rename before validate (`export.rs:669`); collision (`:502-506`); disk-full retry (`:552-566`); `.level.mp4` leak (`:290`) | A broken file looks like a finished export; an earlier export can be overwritten | Validate the `.part`, then rename. On failure keep it as `…-failed-validation.mp4`; never take an existing name (add a `-2` suffix); no libx264 retry on disk-full or permission errors; clean up `.level.mp4` |
| R6 | P0 | CONFIRMED_ISSUE | Data-root change | Jobs capture the old root but share the DB Arc (`lib.rs:373-390`); settings saved first (`:108`) | DB rows written with wrong ids | `set_data_root` refuses while a job is queued or running; open the DB first, save settings only on success. **No** data migration feature (copy → verify → switch); document manual copy |
| E1 | P1 | IMPROVEMENT (owner decision) | Favorites / memory value | Favorite is just `w[3]·1` (`editing.rs:314,318`); greedy can drop it | A ★ clip can be missing from the plan | Optional rule "every ★ file with a valid window gets ≥ 1 clip". This is a behaviour rule, not a new weight. **Needs owner OK** |
| E2 | P1 | CONFIRMED_ISSUE (small) | Order & diversity | Same-day clips ordered by media id, not time (`editing.rs:143-145,333`); stored `rank` excludes diversity, though the doc comment says it includes it (`:189,451`); no date spread | Clips from one day can appear out of order | Sort key = full `source_created_at` + in_ms; store the diversity-adjusted rank or fix the comment. Diversity report: defer |
| E3 | P3 | IMPROVEMENT | Story roles | Only open/hook/end moves (`editing.rs:461-484`) | none proven | Defer until the owner reports pacing problems |
| E4 | P1 | CONFIRMED_ISSUE | Transitions | One clamp from the shortest clip (`export.rs:158-161`); plan total uses the unclamped value (`editing.rs:492`, `EditingPlan.tsx:93`) | One short clip turns every crossfade into a hard cut; plan and export durations disagree | Clamp each join to min(left, right)/2; shared helper for the plan total. Lighting/motion continuity: defer |
| E5 | P1 | IMPROVEMENT (tests) | Speech-safe | Bounded to 4 passes (`editing.rs:270-286`) ✓; untested: overlap, pass exhaustion, segment past scene end; misleading "no scene long enough" when `fit_speech` drops everything (`:434`) | Low | Add the missing tests; a distinct skip reason "every window would cut speech" |
| M1 | P1 | CONFIRMED_ISSUE | HDR | No colour parsing (`media.rs:17-32`); no tonemap anywhere; no output colour tags (`export.rs:421-426`) | HLG/PQ footage exports washed out with no warning | Read `color_transfer`/`color_primaries` from the stored `probe_json` (no migration); show an "HDR" badge and an export warning. Tone mapping: NEEDS_INVESTIGATION (filters in the Gyan build, first check the DJI samples' transfer) |
| M2 | P1 | Mixed | Audio | First stream only (`export.rs:331`); silent sources get `anullsrc`, so validation's "audio present" always passes (`:333,469`). The backlog's silent-export concern is **NOT_APPLICABLE**; no A/V duration check | Possible unnoticed sync drift | Add an audio-vs-video stream duration check (assumed tolerance 100 ms). Multiple streams: defer |
| M3 | P2 | IMPROVEMENT | Manual crop | No per-clip crop (`db.rs:37-44`); global centre crop (`export.rs:157`) | Heads cut off in 9:16 | Defer: needs a migration + UI + preview. Owner decides |
| M4 | P1 | CONFIRMED_ISSUE | Encoder report & validation | Fallback reason only in the log (`export.rs:562`); fps/pix_fmt/colour not validated (`:446`) | The report can't explain a slow render | Return `attempts:[{encoder, error}]` in the result and the UI report; validate fps and pix_fmt |
| A1 | P1 | CONFIRMED (partial) | Model validation | `validated` is never read (`models.rs:37`); IR check = zeros → finite (`memora_worker.py:194`); whisper models get "checksum only" | The UI word "ready" overstates what was checked | Show what was actually checked ("checksum", "load test on zeros"); fix the `validated` flags in `models.json`. Five-level system: not needed now |
| A2 | P1 | CONFIRMED_ISSUE | Taxonomy / tags | No taxonomy version; `model_id` without version (`tagging.rs:14,83`); "Tag N" covers only untagged files (`MediaLibrary.tsx:299`) | After a model update, old tags can't be refreshed from the UI | Store `openclip-vit-b32@2`; mark older tags stale; "Re-tag stale" button. User tag correction: defer |
| A3 | P2 | IMPROVEMENT | Whisper confidence | Only text/start/end (`transcribe.rs:23-27,51-56`); click-to-seek already exists (`MediaLibrary.tsx:113-117,164`) → that part is **NOT_APPLICABLE** | Low | `-ojf` token probabilities → show low-confidence segments in amber. Manual correction: defer |
| A4 | P1 | NEEDS_INVESTIGATION | Worker memory | Worker never stopped; OpenCLIP compiled cache kept for its lifetime (`tagging.py:15`); no commit-charge check (`hardware.rs` is display-only) | Under the 2 GB pagefile, the AI model may sit in RAM during export, against CLAUDE.md "no AI during render" | First measure the worker working set before/after tagging and during export (read-only). Simplest fix if confirmed: stop the worker before `media.export` / `media.transcribe` |
| V1 | P1 | CONFIRMED_ISSUE | Provenance | `media_analysis.version` never read (`analysis.rs:121`); tags/transcripts/plans/exports carry no algorithm or runtime version | Stale results can't be detected | Read the analysis version and flag stale results; store model@version + runtime (`whisper.cpp b5454`, VAD on/off) in the existing text columns; `algorithm: 1` in plan JSON. No migration |
| U1 | P2 | IMPROVEMENT | Workflow nav | One page of cards (`App.tsx`) | Long scroll | Defer; no router |
| U2 | P2 | IMPROVEMENT (small) | Explainable plan | Reasons and scores already shown; skipped *windows* not counted | Medium | Add counts: "N windows lost to: target / duplicate / same-scene / speech" |
| U3 | P2 | IMPROVEMENT | Usability | No undo, multi-select or shortcuts; batch buttons ignore search (`MediaLibrary.tsx:298-301`); media list has no loading state | Low–medium | Batch buttons act on filtered rows; loading state. Undo: defer |
| U4 | — | Owner decision | UI language | — | — | Unchanged |
| Q1 | P2 | IMPROVEMENT | Tests | 67 Rust tests; none for cancel mid-run (export/proxy/analysis/tag), disk errors, A/V sync | — | Add with R4/R5/M2; human evaluation rubric as a doc |
| P7 | — | Gate | Packaging | Not approved | — | Separate readiness report later, no installer |

---

## C. Proposed backlog (implementation order)

One reviewed change per module (CLAUDE.md). Complexity is S/M/L. Every item keeps: originals untouched, exports from originals, CPU fallback, no new dependencies.

| Order | Item | Module(s) | Size | DB impact | Acceptance / tests |
|---|---|---|---|---|---|
| 1 | R6 data-root guard | lib.rs | S | none | Test: `set_data_root` errors while a job is queued/running; a bad new root does not overwrite settings |
| 2 | R3 startup reconciliation | lib.rs + models.rs (+ small helper) | S | updates stuck model rows only | Test: rows in working states become `install_failed` "interrupted"; temp files removed; model `.part` kept |
| 3 | R4 spawn helper + worker cancel timeout + ffprobe timeout | process.rs, worker.rs, media.rs | S–M | none | Test: worker that ignores cancel is killed after the grace period and the next job works (diagnostic job); no leak path left (grep) |
| 4 | R5 export promote-after-validate, unique name, no retry on disk errors | export.rs (+ proxy.rs helper) | S–M | none | Tests: existing output name never replaced; failed validation → `-failed-validation.mp4`; permission error into a read-only dir fails once |
| 5 | R2 DB backup | db.rs (+ command) | S | new `database\backups\` folder | Test: backup taken before a pending migration and passes `integrity_check`; retention keeps N |
| 6 | R1 availability + relink | media.rs, editing.rs, MediaLibrary.tsx | M | `UPDATE media.path` only | Tests: relink keeps id/clips/analysis; rejects a different file (size/duration mismatch); plan skips missing files with a reason |
| 7 | E4 per-join transitions + E2 timestamp order | export.rs, editing.rs | S | none | Tests: one 0.4 s clip no longer zeroes the other joins; plan total == export duration; same-day clips in time order |
| 8 | M4 encoder attempts + fps/pix_fmt checks, M2 A/V duration check | export.rs, Timeline.tsx | S | none | Report lists the fallback reason; synthetic tests |
| 9 | M1 HDR detection + warning | media.rs, export.rs, UI | S | none (reads `probe_json`) | Test with a synthetic HLG-tagged clip |
| 10 | V1 + A2 versions, stale flags, re-tag | analysis.rs, tagging.rs, transcribe.rs, editing.rs, UI | M | text values only | Old-version rows flagged stale; "Re-tag stale" re-runs them |
| 11 | Docs reconciliation (section D) | docs + 2 code comments + models.json flags | S | none | — |
| — | A4 measurement first, then possibly "stop worker before export" | worker.rs | S | none | Measured numbers recorded in HARDWARE_VALIDATION |
| Deferred | E1 (owner rule), E3, M3, A3, U1–U3, diversity report, transcript/tag editing, data-root migration | — | — | — | Revisit on owner request |

---

## D. Documentation reconciliation

| Fact | Stale place | Authoritative source |
|---|---|---|
| Tag count 48 + 4 negatives | `MODEL_PROPOSAL.md:52`, `adr/0004:21` say 49 | `tools/clip-export/export_openclip.py` |
| OpenCLIP validated | `models.json:42` `false` | Measurements in MODEL_PROPOSAL §4 → set the flag consistently (A1) |
| VAD active | `transcribe.rs:61` ponytail comment | `transcribe.rs:141-143` code |
| Music and titles exist | `editing.rs:51-52` `NO_MUSIC`/`NO_TITLES`, shown at `EditingPlan.tsx:121` | `export.rs`, `sounds.rs` |
| Next steps | `PROGRESS.md:60-67` stale block; "64 tests" (now 67) | PROGRESS day log above it |
| OpenCLIP/Whisper on hardware | `HARDWARE_VALIDATION.md:108-109` "Unknown" | MODEL_PROPOSAL §4 |
| App snapshot | `APP_OVERVIEW.md` predates `2b32635` (no `sounds.rs`, no second loudness pass, test count) | Code at HEAD → refresh APP_OVERVIEW |

Rule going forward: **code > manifests (`models.json`, `sounds.json`) > ADRs > PROGRESS/overview.**

---

## Status (2026-11-08)

Owner approved batches 1–5. **Done:** R6, R3, R4, R5, R2 (details and tests in [PROGRESS.md](PROGRESS.md)). E2 (same-day order by recording time) was done with the owner's plan-variety feedback, together with a per-concept scene cap and a 35 % concept share. Everything else is still open.

## E. Approval gate
The owner chooses which items to implement, by ID or by batch number. Recommended first batch: **1–5 (R6, R3, R4, R5, R2)**. All are small, P0, and need no schema migration. No code is touched before that answer.

