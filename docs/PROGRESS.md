# Progress log

## 2026-11-07 — end of day

### Done (all verified with tests and/or the real app)

| Phase | Status | Evidence |
|---|---|---|
| 0 Discovery | Done | `docs/DISCOVERY_REPORT.md`, ADR 0001 |
| 1a Environment | Done | `docs/DEPENDENCY_MATRIX.md` (Rust, FFmpeg Gyan GPL, Python 3.13 venv) |
| 1b Feasibility spikes | Done | `docs/HARDWARE_VALIDATION.md`, ADR 0002 (embedded Python worker) |
| 2 Desktop foundation | Done | Tauri 2 + React shell, settings/data root, JSON logs, hardware info, SQLite + migrations, projects, job manager, AI worker lifecycle (Job Object, restart, protocol v1) |
| 3 Media Engine | Done | Import + FFprobe metadata, thumbnails, proxies (full QSV / hybrid / libx264), preview player, timeline (trim/concat), export presets rendered from originals, §8.3 validation |
| 4 Classical analysis | Done | ADR 0003: worker analyses proxies (PySceneDetect, OpenCV metrics, shake, dHash); Rust deterministic technical scoring + duplicate flags; scene strip in preview |
| 5 Local AI | **In progress** | Slice 5.1 Model Manager done (see below). No model downloaded, torch not installed. |

Data root moved to `E:\Memora Data\` (ADR 0001 amendment). `D:\MemoraStudio\` still exists as a verified copy — the owner deletes it manually (the agent's tool blocks deleting that path).

Tests: 40 Rust tests (unit + real FFmpeg + real Python worker integration) and `python workers/ai-worker/analysis.py` self-check — all passing at end of day.

#### Slice 5.1 Model Manager (done, awaiting review)
- `apps/desktop/src-tauri/models.json`: manifests for OpenCLIP ViT-B/32, Whisper base, Whisper small. **No URL/checksum filled → downloads disabled** with an explanation in the UI.
- `models.rs`: install pipeline download (Windows built-in `curl.exe`, resumable) or offline copy → size + SHA-256 check → install to `<data root>\models\<category>\<id>\<version>\` → load validation → state in SQLite (migration 5). Removal. States per docs/Production.md §4.3.
- Worker job `validate_openvino_ir`: compiles an IR on CPU (or listed devices) and runs one inference.
- UI card "AI models": state, Download (confirmation with name/size/license/source), Install from files…, Remove, Cancel.
- Tests: real `file://` download through curl.exe, checksum failure, offline install, load-validation failure, and an offline OpenVINO-IR install validated by the real worker.

#### 2026-11-08 — model proposal approved, downloads done
See `docs/MODEL_PROPOSAL.md` §4: whisper.cpp b5454 runtime installed in `<data root>\runtime\whisper.cpp\b5454\`, Whisper base/small verified and pre-staged, OpenCLIP checkpoint verified in `downloads\staging`. OpenCLIP exported to OpenVINO IR + fixed bilingual tag embeddings (`tools/clip-export/`), verified vs PyTorch, checksums in `models.json`. Next: install both models in the app, then the whisper.cpp transcription job and the OpenCLIP scene-tagging job.

#### Slice 5.2 Transcription (done, awaiting review)
- `transcribe.rs`: job `media.transcribe` (params ids, model whisper-base|whisper-small, language auto|vi|en): audio from the ORIGINAL → 16 kHz mono WAV in `cache\audio` (deleted after) → `whisper-cli.exe` (`<data root>\runtime\whisper.cpp\b5454\`, Job Object, `-pp` progress, cancel kills it) → migration 6 `transcripts` (segments JSON, cascades with media, cleared when the original changes). Files without audio are stored as "no speech".
- Hallucination mitigation from real footage: `-mc 0 -sns` + drop runs of ≥4 identical segments. Silero VAD proposed (MODEL_PROPOSAL §4), not downloaded.
- UI: Media library "Speech" column, model picker + "Transcribe N videos", transcript list in the preview (click → seek + In/Out).
- Tests: parse/store/cascade, loop filter, missing model, real whisper.cpp end-to-end incl. cancellation (44 Rust tests).
- Next: OpenCLIP scene tagging job (slice 5.3).

#### Slices 5.3–5.4, Phase 6 and first Phase 7 items (done, awaiting review) — decisions in ADR 0004
- 5.3 `tagging.rs` + worker `tagging.py`: job `media.tag`, OpenCLIP IR every 2 s of the proxy (GPU → CPU fallback), migration 7 `media_tags`; tags per clip and per scene in the UI; library search by concept (Vietnamese without diacritics works). Real proxy: 150 samples in 13 s on GPU.
- 5.4 Silero VAD (`silero-vad` in models.json, verified SHA-256, staged in `models\.downloads`) used automatically by transcription when installed.
- Phase 6 `editing.rs`: five styles, candidate windows, separate scores + reasons, diversity/duplicates/exclusions, story structure, deterministic plan stored per project (migration 8 `editing_plans`, `media_marks`). UI "Smart edit": style + date range → plan review (drop / reorder) → Apply to timeline / Apply and export. Library ★ favorite / ⊘ exclude.
- Export: `fit: fill` centre crop for vertical output, `transitionMs` crossfades; Timeline export has both options.
- Verified: 54 Rust tests (real FFmpeg crossfade + fill, real worker tagging with the exported IR, real whisper.cpp with VAD), `python tagging.py`, and the debug app on the real library (Smart edit generated a 20-clip Travel Cinematic plan from 96 candidates).

#### Owner review round 1 (2026-11-08) — see ADR 0004 "Revision"
- Editing: sentence-safe cuts, no chopping of unchanged shots, merged near clips, bad-shot detection (OpenCLIP v2 → owner must "Install from files" from `downloads\openclip-export-v2` and re-run "Tag").
- Export: transitions, colour looks, fades, blurred background, music with ducking, loudness, titles/captions/subtitles; per-project options and per-clip captions (migration 9).
- Verified: 61 Rust tests; real renders from the owner's footage (Travel Cinematic 85 s and Reels 30 s, both valid) inspected frame by frame; release app UI checked (Look & sound panel filled from the style).

#### Free music library (2026-11-08) — ADR 0005
- `sounds.json` (22 verified CC BY 4.0 tracks, Kevin MacLeod / incompetech.com) + `sounds.rs`: streamed preview, per-video recommendations, download-on-export with SHA-256 check and deletion afterwards; the owner's own audio files are never touched. Credit burned in at the end.
- Second loudness pass (measure → exact gain + limiter): real export −19.6 → −16.4 LUFS.
- Verified: 64 Rust tests + real download/verify/delete test, real export with library music (credit frame inspected, download folder empty afterwards), release app: library opens, preview plays, recommendation shown.

#### Reliability batch + plan variety (2026-11-08) — audit `docs/IMPROVEMENT_AUDIT.md`, batches 1–5 approved
- R6: `set_data_root` refused while a job is queued/running; the new database is opened before settings are saved (a folder whose database fails to open is not saved).
- R3: start-up / folder-switch reconciliation: model rows left in a working state → `install_failed` "Interrupted…" (Remove visible again); app temporaries removed (`cache\render|audio|downloads\sounds`, `.part`, `.level.mp4`, `.unvalidated.mp4`); resumable model downloads kept.
- R4: `process::spawn` (child killed if it cannot join the Job Object — 6 call sites), `process::wait_output` with timeout/cancel (ffprobe 60 s, loudness measure 600 s — assumed limits); AI worker killed 10 s after an ignored cancel and restarted on the next job (test job `diagnostic.block`).
- R5: export renders to `<name>.unvalidated.mp4`, then levels and validates it, then promotes it to a free name (`-2`, `-3` … — an earlier export is never replaced); a failed validation is kept as `…-failed-validation.mp4`; no second (libx264) render after disk-full / permission errors; levelled temp file cleaned on rename failure.
- R2: database copied with `VACUUM INTO` + `integrity_check` to `database\backups\` at every start and before any migration (migration refused if that copy fails); last 10 kept; "Back up now" + restore steps in the Data folder card.
- Plan variety (owner feedback: walk up the path to the big tree dropped, too much cycling): the per-scene cap now counts per main concept, one concept fills at most 35 % of the target while other content exists (`MAX_CONCEPT_SHARE`, initial value), same-day clips ordered by recording time. On a copy of the real library, Travel Cinematic: cycling 7 clips (~55 s of 85 s) → 4 clips (26 s); the path walk (0007, 0–7 s) now included; 8 concepts instead of 4.
- Verified: 72 Rust tests (3 ignored manual ones), `tsc --noEmit`.

#### Command-line tool + editing wishes (2026-11-08, owner approved)
- `memora-cli.exe` (`src/bin/memora-cli.rs` → `src/cli.rs`): the engine without the window, same data folder / database / Editing Core / export as the app (`MEMORA_DATA_ROOT` overrides the folder). Library (`status`, `media`, `concepts`, `scenes`, `transcript`, `job`), projects and plans (`plan … --prefer --avoid --target`, `show-plan`, `apply --keep`), timeline (`clip-add|trim|remove|move|caption`), `options`, `sounds`, `export`. Close the app before export/job commands. `memora-cli help` lists everything.
- Editing wishes (`editing::Wishes`, stored with the plan): preferred concepts raise rank up to ×1.5 (reason "wanted: …"), avoided concepts leave out windows where they score ≥ 0.25, optional length. Not in the app UI yet.
- Shared code moved, not duplicated: `lib::job_runner` (app queue + CLI), `export::load_options/save_options`, `lib::recommend_for`, `timeline::set_range`. Start-up database copy skipped when one is < 10 min old (the CLI opens the database per command).
- Request box (owner decision 2026-11-08: offline keywords, no language model / cloud — Claude API option declined for now): `request.rs` reads a Vietnamese or English sentence (length, dọc/ngang, nhiều/bớt/bỏ + concepts from the library's tag names, nature group, colour, music energy / no music, subtitles, title, ending, dates, style words) into `Wishes` + style + dates and reads it back ("Understood: …"). Smart edit card has the box with live read-back; `generate_plan` takes `request`; applying a plan honours preset/colour/title/ending/subtitles/music (`lib::apply_plan_to` picks the best library track of the wished energy). CLI: `understand TEXT`, `plan P --request "…"`. Short single words need diacritics (biến ≠ biển).
- Verified (request box): 82 Rust tests; on the copy, the owner-style sentence → social_story 30 s vertical, big tree as hook, path walk kept, cycling one 2 s clip ("less wanted"), 21 selfie windows left out, warm look, calm library track, title "Đà Lạt"; release build.
- Verified: 76 Rust tests; on a COPY of the data folder: `plan 1 --style travel_cinematic --target 45 --prefer "đường mòn,cây cổ thụ,thuyền"` → 6 clips incl. the path walk and the big tree, `apply`, `options --color warm --title …`, `export` → valid 1920×1080 39.9 s file (hybrid-qsv, 48 s render); release build of app + CLI.

#### Nam Cát Tiên trip edit + fixes from using the app (2026-11-08, owner request)
- Library: 25 DJI Osmo Action 4 clips of 20/02/2026 (≈72 min) analysed (13 min), tagged on GPU (9 min) and transcribed with whisper-base vi (2.5 min) through `memora-cli job`.
- Outputs (from the originals): project #4 "Nam Cát Tiên – tổng hợp" (`journey_recap`, 26 clips, 3:20, 1920×1080, Blue Feather, own sound 0.35, −16.1 LUFS) and project #5 "Nam Cát Tiên – Reels" (13 hand-picked clips, 33.5 s, 1080×1920 centre crop, Achaidh Cheide, own sound off, −16.3 LUFS). Both validated; frames inspected. ★ on #1, #3, #10, #12, #16 for the year recap.
- App changes: `journey_recap` style + request words; own sound lowered to 0.35 under music (measured DJI level would keep the music ducked); Editing Core ignores Whisper lines with implausible timing (`transcribe::Segment::timing_plausible`); export renders timelines over 10 clips in parts (`render_parts`, FFmpeg peak 11 GB → 5.2 GB on the 26-clip recap); CLI `mark`.
- Findings still open (tagging misses some fingers, no visual-appeal score, centre crop, large files): `docs/IMPROVEMENT_AUDIT.md` "Found while editing Nam Cát Tiên".
- Verified: 84 Rust tests (new: journey order, stretched Whisper lines, own sound under music, render in parts with real FFmpeg); release build of app + CLI.

#### Owner review round 2 of the Nam Cát Tiên edit (2026-11-09): recap 7/10, reel 7/10
- Owner: library music poor ("thà không có nhạc"), reel music "như cướp biển", riding highlights missing, raised-camera road shots replaced by tilted-down handlebar shots, reel did not cover the trip; allowed 2 GB for better free music.
- Camera cue: OpenCLIP model v3 (same IR, `cues` with "view ahead" prompt pair; checked on 850 frames), exported with the existing dev venv, installed with `memora-cli job models.install`, library re-tagged (4 min on GPU). Editing Core leaves out tilted-down windows and ranks by the view; same-concept diversity penalty capped at 3; request aliases (đạp xe → bicycle); own sound back to 1.0 without music.
- Music: 10 Scott Buckley tracks (CC BY 4.0; track pages checked, files measured) added to `sounds.json`; library music kept in `<data root>\music\` (ADR 0005 amendment); CSP allows their previews; UI text updated.
- Outputs: recap `projects\4\exports\…-1794157312.mp4` (32 clips, 3:23, no music, own sound, −16.4 LUFS) and reel `projects\5\exports\…-1794157516.mp4` (18 clips, 46 s, "Simplicity", −16.0 LUFS); frames of both outputs inspected; 7 of 22 automatic reel clips replaced by hand (visual appeal still not measured, audit T7).
- Verified: 86 Rust tests, `python tagging.py`, `tsc --noEmit`, release build.

#### Owner review 3 of the Nam Cát Tiên recap (2026-11-11): forest trails, length, transitions
- Owner: no forest paths / uphill / small bridges / downhill through the leaves; 3 min too short (~5 min or what the footage needs); smoother transitions.
- Cause (real database, read-only): the v3 "view ahead" cue scored 1 362 of 1 566 riding samples < 0.1 (handlebars always in frame, no "landscape far ahead" under the canopy) → 466 windows left out; the 240 s plan ran out of candidates at 213 s.
- OpenCLIP model **v4** (same IR, `openclip_tags.json` only; dev venv, IR vs PyTorch cosine ≥ 0.99999): scene cues "forest trail", "small bridge", "slope", thresholds from frame boards of 1 010 frames ("leaf-covered path" / "ground only" prompts did not separate, left out). Installed with `memora-cli job models.install`, 25 files re-tagged (7.4 min, GPU).
- Editing Core: tilted windows kept on forest trails, bridge / slope / forest trail become the clip's concept; `journey_recap` length from the footage (4.2 s per minute, 90–360 s) with a plan note; coverage pass (one clip per scene type with ≥ 20 s, one per 8 min of recording time); request words "đường rừng", "qua cầu", "lên/xuống dốc".
- Export: per-join transitions, each clamped by its two neighbours (audit E4 fixed); new `auto` transition (default of `journey_recap`): short cross dissolve inside a shot, 0.7 s between places, dip to black ≥ 20 min apart. Found: xfade `dissolve` is the grainy one — `auto` uses `fade`.
- Output: project #4 `projects\4\exports\Nam Cát Tiên – tổng hợp-landscape-1794413683.mp4` (41 clips, 4:30 of a 4:58 auto target, 13 forest trail + 3 slope + 1 bridge clips, valid, 1.9 GB); frame board inspected (the one "bridge" clip shows people by the stream: cue false positive).
- Verified: 89 Rust tests (new: auto joins / per-join clamp, forest trails + coverage + auto length, request words), real export validated.

#### UI / UX rework (2026-11-11, owner: "not clear what to do next")
- Sidebar with Library / Projects / Jobs / Settings, "Next step" banner (add videos → analyse → project → Smart edit → edit & export), job bar on every page, project as three steps (Videos → Smart edit → Edit & export), confirm before a plan replaces a timeline, copy path of an export.
- One "Analyse" for everything Smart edit uses (scenes, then OpenCLIP tags, then Whisper — small when installed), also run automatically after importing; models missing → link to Settings.
- Bilingual UI (owner decision): Vietnamese default, English in the sidebar; `L(en, vi)` at each string (`i18n.ts`); shared pieces in `ui.tsx`. Backend texts (style descriptions, plan reasons, "Understood" chips, errors) still English/mixed.
- Verified: `tsc --noEmit`, release build, app window screenshots (Library, Projects step 2 and step 3 on the real library); no frontend unit tests.

#### Photos in Smart edit (2026-11-12, owner: "tiếp tục thực hiện") — slice 2 (+ request words of slice 3), awaiting review
- Photo times now carry their zone (EXIF OffsetTimeOriginal, else this PC's offset); `export::unix_ms` reads zones — photos and UTC video times line up.
- Migration 12 `photo_analysis`; worker `photo_analyze` (edited photo at 960 px: `frame_metrics` + OpenCLIP `tag_image`, ≈ 0.14 s per photo on the GPU after a 1.4 s first compile, measured on this PC); job `media.photo_analyze`, submitted automatically when the Photos page opens.
- Editing Core: `gather_photos`, `pick_photos` (taken during the trip ± 3 h, not blurry / dark / overexposed / bad shot / avoided, one per near-identical picture, ≤ 25 % of the length, spread over the trip), `place_photos` (after the clip recorded just before), photo joins in the plan length; `apply` adds photo clips. Shares and margins are initial values.
- Request box: "có ảnh / thêm ảnh", "nhiều ảnh", "ít ảnh", "chỉ video / không dùng ảnh", "mỗi ảnh N giây" (not read as the video length); "bỏ ảnh tự sướng" stays the selfie concept.
- Verified: 99 Rust tests (new: zones, photo picking and placement, request words), `python tagging.py`, `python photo.py`, `tsc`, release build. Real data: the 6 test stills (file times set to their frame moments in the trip videos) re-imported and analysed in the app; `memora-cli plan 6 --request "… có cả ảnh …"` → 44 clips 4:26 incl. 6 photos, each right after the clip of the moment it shows; `apply 6` + `export 6` → 265.8 s 1920×1080, hybrid QSV 214 s, validation passed (clip-peak warning as before); frame board checked. Project #4 untouched.

#### Mixed timeline, visual strip, per-join transitions, join preview (2026-11-12, owner: "có timeline và các đoạn chuyển cảnh thì càng dễ để tôi quản lý" + "vừa dùng video vừa dựng kèm hình ảnh") — slice 1 of 3, awaiting review
- Migration 11: `project_clips` rebuilt — `media_id` nullable, `photo_id`, CHECK one of them, `transition` / `transition_ms` per clip (transition into it). Existing clips kept (test `mixed_timeline_migration_keeps_clips`); the start-up backup is taken before migrating.
- `timeline.rs`: `add_photo`, `set_transition`, photo lengths 1–20 s. `export.rs`: `joins_with` (per-clip overrides), `timeline_joins`, photo clips rendered as stills in `export_job` (job worker), `preview_join` (half size, proxies). 5 new commands (`add_photo_clips`, `set_clip_range`, `set_clip_transition`, `list_joins`, `preview_join`); CLI `timeline` shows every join.
- UI: Timeline step strip (blocks + join chips), join panel (style / length / ▶ preview), "+ Add photos…", photo rows with seconds and "Edit photo" (opens the Photos page on that photo).
- Verified: 96 Rust tests (new: migration, timeline photos + transitions, real mixed export + join preview), `tsc --noEmit`, release build; in the app on a new test project #6 "Thử timeline video + ảnh" (2 DJI clips + 2 photos): strip, photo picker, join preview (~3 s), export 17.2 s 1920×1080 validated (`projects\6\exports`), frame board checked. Project #4 untouched (its strip shows the 41 clips and auto joins).
- Known: photo dates are file times or EXIF local time without zone, videos are UTC → `auto` may put a dip-to-black between a photo and a video; fixed in slice 2 (photo times) together with placing photos by date in Smart edit.

#### Video from photos (2026-11-12, owner: "tạo video từ ảnh tĩnh … chỉnh ảnh ở phần xử lý ảnh rồi đem kết quả đó qua làm video") — awaiting review
- Owner choices: still photos, no motion; photo effects and music; edits made on the Photos page carry into the video.
- `export.rs`: still segments (`STILL`, looped image input, no hardware decode) and `finish` shared by project exports and photo videos. `photos::video_job` = job `media.photo_video` (photos rendered with their edits at 2880 px into the render cache, then `export::finish`). `recommend_sounds` without a project suggests music for a photo video.
- UI: Photos page **🎬 Make a video**: pick in order, timeline strip (photo blocks + transition lengths, reorder / leave out), seconds per photo, format, Look & sound panel reused from projects, suggested library track pre-selected once, progress, validation report.
- Verified: 93 Rust tests (new `stills_are_looped_and_never_hardware_decoded`, `photo_video_with_real_worker`), `tsc --noEmit`, release build; in the app: 6 test photos → `E:\Memora Data\photos\exports\Video ảnh-landscape.mp4` (14.0 s, 1920×1080, rendered in 10 s with hybrid QSV, validation passed); frame board checked (crossfades, edited photo, music credit at the end).
- Next (owner 2026-11-12): a visual timeline for project videos too — clip blocks, each transition visible and changeable, preview of a join.

#### Photo editor slice B1 (2026-11-12, owner: "thực hiện tiếp … chỉnh ảnh") — ADR 0006, no model, awaiting review
- Owner clarification: "sky" = make the photo's own sky cinematic, clearer, richer but natural — never a pasted sky; reference images are mood only.
- Worker `photo.py` (OpenCV + numpy, already installed): geometry (rotate, straighten, aspect crop), denoise, white balance, exposure, dehaze, highlight roll-off, shadows / highlights / whites / blacks, contrast, clarity (no halos), edge-only sharpening, vibrance (skin protected) / saturation, HSL × 8 bands, looks, vignette; **sky**: classical mask (sky-like pixels in an unbroken run from the top of each column, narrow spikes cut to the median skyline) + cloud detail, deeper bright sky, richer colour, warmth; **Auto** suggestions; minimal EXIF date reader. Self-check `python photo.py`.
- Mask tuned on frame boards of 6 stills from the owner's footage: first version leaked into trees, water and people; column-run + skyline-median version keeps to the sky (trunks / shirts no longer taken). Speed on this PC: 2688×1512 full render 1.2–1.4 s, 1600 px preview 0.45 s with Auto values (single runs).
- Rust `photos.rs` (migration 10 `photos`, edit stack in the row), second worker process `photos::Lane` for previews, 7 commands (`list_photos`, `import_photos`, `set_photo_edits`, `preview_photo`, `auto_photo`, `export_photo`, `remove_photo`); exports `photos\exports\<name>-edit[-N].jpg`, never replacing a file. UI: new "Photos / Ảnh" page (strip of photos, large preview, slider panel, Auto, Undo, Reset, hold for original, show sky area, export).
- Defaults while questions are open: exported photos do not carry the camera EXIF yet (Pillow in the worker venv needs approval); photos are added as files.
- Test stills (frames of the owner's videos) in `E:\Memora Data\test-media\photos\` and imported in the library; one export `photos\exports\7_3-edit.jpg` (original hash unchanged).
- Verified: 91 Rust tests (new: photo rows / edits / dates, real worker import → preview → auto → two exports), `python photo.py`, `tsc --noEmit`, release build, app window screenshots of the Photos page.
- Next (needs approval of model downloads): B2 select + remove (MobileSAM, MI-GAN), B3 retouch (YuNet + landmarks), B4 bokeh (Depth Anything V2 Small, BiRefNet-lite), B6 AI upscale / denoise.

### Still open (owner decisions or later work)
- Packaging / installer: needs the FFmpeg GPL licence review before any distribution and an embedded Python runtime (ADR 0002); not built.
- Bundled music library (user's own files work now), subject-aware crop, text animation presets, GPS/location, month/year grouping in the library, Whisper GPU experiment.
- Offline: no network use except the Model Manager's explicit, user-confirmed downloads (all models are installed locally now).

### Not done yet / next steps

1. **Model proposal (needs owner approval before any download):** for each model the official source, pinned revision, license, size and SHA-256:
   - OpenCLIP ViT-B/32 (checkpoint candidate `laion2b_s34b_b79k`) — license review; export to OpenVINO IR needs **dev-time torch in a separate venv (separate approval)**; output-consistency check vs PyTorch CPU.
   - whisper.cpp (pinned release, CPU build) + Whisper base multilingual (ggml); small on demand.
2. After approval: fill models.json, OpenCLIP embedding job (scene frames → embeddings cache, text search), whisper.cpp transcription job (Vietnamese + English test coverage), CPU/GPU benchmarks, optional NPU experiment.
3. Phase 6 Editing Intelligence (needs approval): split long continuous scenes into candidate windows (all DJI samples are single long shots), clip selection, presets, Editing Plan review UI.
4. Phase 7 Product completion (needs approval).

### Known limitations carried forward
- Score thresholds in `scoring.rs` are initial assumptions; tune on real libraries.
- Export letterboxes mismatched aspect ratios (no smart crop); HDR (PQ/HLG) not tone-mapped; export bitrate generous (~29 Mbit/s at 1080p).
- No FFprobe/FFmpeg timeout for hung processes; job list is in memory only; worker is not stopped when idle.
- Download cancellation (curl kill) is not covered by an automated test.
- Packaging: `analysis.py` must ship next to `memora_worker.py`; installer not built yet; FFmpeg GPL licence must be revisited before distribution.
- Pending owner decisions: OpenVINO telemetry opt-out, deleting spike artifacts (~1.9 GB in repo build dirs, ~1.6 GB in `E:\Memora Data\cache\spike`), UI language (English now).

### How to resume
- Read `CLAUDE.md` (status + rules) and this file.
- Tests: `cargo test` in `apps/desktop/src-tauri`; build: `npm run tauri build -- --no-bundle` in `apps/desktop`.
- Recreate dev venv: `py -3.13 -m venv workers\ai-worker\.venv` then `pip install --no-deps -r workers\ai-worker\requirements.lock.txt` (inside the venv).
- Frontend deps: `npm ci` in `apps/desktop`.
