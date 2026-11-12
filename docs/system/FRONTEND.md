# Memora Studio — Frontend

The React webview of the Tauri app: a sidebar with five pages (Library, Projects, Photos, Jobs, Settings), a "Next step" banner and a job bar; cards call Rust commands through typed wrappers and follow background jobs through `job-updated` events. Bilingual (Vietnamese default, English).

Last verified: 2026-11-12 against commit `7eeb0bd` + uncommitted changes.

Index of all system docs: [README.md](README.md). The `sync-docs` skill (`.claude/skills/sync-docs/`) keeps this file current; run it after any change under `apps/desktop/` outside `src-tauri/`.

Sources: [apps/desktop/src/](../../apps/desktop/src/) — [main.tsx](../../apps/desktop/src/main.tsx), [App.tsx](../../apps/desktop/src/App.tsx), [api.ts](../../apps/desktop/src/api.ts), [Projects.tsx](../../apps/desktop/src/Projects.tsx), [EditingPlan.tsx](../../apps/desktop/src/EditingPlan.tsx), [Timeline.tsx](../../apps/desktop/src/Timeline.tsx), [MediaLibrary.tsx](../../apps/desktop/src/MediaLibrary.tsx), [PhotoEditor.tsx](../../apps/desktop/src/PhotoEditor.tsx), [AiCenter.tsx](../../apps/desktop/src/AiCenter.tsx), [Models.tsx](../../apps/desktop/src/Models.tsx), [ui.tsx](../../apps/desktop/src/ui.tsx), [i18n.ts](../../apps/desktop/src/i18n.ts), [index.css](../../apps/desktop/src/index.css). Backend counterpart: [BACKEND.md](BACKEND.md).

---

## Build and tooling

### Scripts ([package.json](../../apps/desktop/package.json))

| Script | Command | Use |
|---|---|---|
| `dev` | `vite` | Dev server on port 1420 (started by `tauri dev` via `beforeDevCommand`). |
| `build` | `tsc && vite build` | Type-check then bundle to `dist/` (run by `tauri build` via `beforeBuildCommand`). |
| `preview` | `vite preview` | Serve `dist/` (not used by Tauri). |
| `tauri` | `tauri` | Tauri CLI. Project build: `npm run tauri build -- --no-bundle`. |

Package: `"name": "desktop"`, `"version": "0.1.0"`, `"type": "module"`, private. No test runner, linter or formatter is configured for the frontend; type-checking (`tsc`) is the only static check.

### Vite ([vite.config.ts](../../apps/desktop/vite.config.ts))

- Plugins: `@vitejs/plugin-react`, `@tailwindcss/vite`.
- `clearScreen: false` (keeps Rust errors visible).
- `server.port: 1420`, `strictPort: true` (Tauri `devUrl` is `http://localhost:1420`).
- `server.host` = `TAURI_DEV_HOST` env var or `false`; HMR over `ws` on port 1421 only when that var is set.
- `server.watch.ignored: ["**/src-tauri/**"]`.
- `import process from "node:process"` with `@ts-expect-error` (no `@types/node` installed).

### TypeScript ([tsconfig.json](../../apps/desktop/tsconfig.json), [tsconfig.node.json](../../apps/desktop/tsconfig.node.json))

- `target`/`lib`: ES2020 + DOM; `module: ESNext`; `moduleResolution: bundler`; `jsx: react-jsx`; `noEmit`; `isolatedModules`; `allowImportingTsExtensions`; `resolveJsonModule`.
- Strictness: `strict`, `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch` — an unused import or variable fails `npm run build`.
- `include: ["src"]`; `tsconfig.node.json` (composite) covers `vite.config.ts` only.
- [vite-env.d.ts](../../apps/desktop/src/vite-env.d.ts): `/// <reference types="vite/client" />`.

### Entry

- [index.html](../../apps/desktop/index.html): `<title>Memora Studio</title>`, `<div id="root">`, module script `/src/main.tsx`.
- [main.tsx](../../apps/desktop/src/main.tsx): `ReactDOM.createRoot(#root).render(<React.StrictMode><App/></React.StrictMode>)`, imports `index.css`. StrictMode double-runs effects in dev; every effect that subscribes to events returns an unsubscribe.
- `public/` is empty. `README.md` is the untouched Tauri template text.

### Dependencies (installed versions from `node_modules/*/package.json`)

| Package | Range | Installed | Why |
|---|---|---|---|
| `@tauri-apps/api` | ^2 | 2.12.4 | `invoke` (commands), `listen` (events), `convertFileSrc` (asset URLs for cache files). |
| `@tauri-apps/plugin-dialog` | ^2.8.4 | 2.8.4 | `open()` file pickers (videos, audio, model files). Capability `dialog:allow-open`. |
| `react` / `react-dom` | ^19.1.0 | 19.3.1 | UI. |
| dev `@tailwindcss/vite` | ^4.3.5 | 4.3.5 | Tailwind v4 Vite plugin (no `tailwind.config`, no PostCSS config). |
| dev `tailwindcss` | ^4.3.5 | 4.3.5 | Utility CSS. |
| dev `@tauri-apps/cli` | ^2 | 2.12.4 | `tauri dev` / `tauri build`. |
| dev `@types/react` / `@types/react-dom` | ^19.1.x | 19.3.0 / 19.3.0 | Types. |
| dev `@vitejs/plugin-react` | ^6.0.2 | 6.2.1 | JSX transform / fast refresh. |
| dev `typescript` | ~6.0.3 | 6.0.3 | `tsc` in `build`. |
| dev `vite` | ^8.0.16 | 8.3.6 | Dev server / bundler. |

No router, no state library, no component library, no icon package, no data-fetching library.

### Webview security that affects the UI ([tauri.conf.json](../../apps/desktop/src-tauri/tauri.conf.json))

- CSP: `img-src 'self' data: asset: http://asset.localhost`; `media-src 'self' asset: http://asset.localhost https://incompetech.com https://www.scottbuckley.com.au`; `style-src 'self' 'unsafe-inline'`. Images/videos load only from the asset protocol (plus music previews from incompetech.com and scottbuckley.com.au).
- Asset protocol scope is empty in config; Rust adds `<data root>\cache` at runtime, so `convertFileSrc` works only for thumbs, proxies and scene frames.
- Window: 1200×800, min 900×600.

---

## Page layout

No router: [App.tsx](../../apps/desktop/src/App.tsx) keeps `page: "library" | "project" | "photos" | "jobs" | "settings"` in state (remembered in `localStorage` `memora.page`). Layout: full-height column = left `nav` (w-52: "Memora Studio", five page buttons with Unicode icons, the open project, language switch Tiếng Việt / English) + scrolling `main` (`max-w-6xl`) + `JobBar` at the bottom. Without a data root (or with a DB error) only Settings is enabled and shown.

| Page | Content | Render condition / `key` |
|---|---|---|
| Library | `NextStep`, `MediaLibrary` (`project`, `version`, `onClipAdded` → bump, `onSettings` → Settings page) | data root + DB; key `settings.dataRoot` |
| Projects | `NextStep`, `ProjectPage`: `Projects` card, then for the open project a 3-step bar (1 Videos → goes to Library, 2 Smart edit, 3 Edit & export) and `EditingPlan` (step 2, `onApplied` → bump + step 3) or `Timeline` (step 3, `version`) | data root + DB; key `settings.dataRoot`, steps keyed on `project.id` |
| Photos | `PhotoEditor` (no Next-step banner) | data root + DB; key `settings.dataRoot` |
| Jobs | `AiCenter` (all jobs of the session + worker + device check) | always |
| Settings | `DataFolder` (+ `Backup`), `Models`, `Hardware` (System, Graphics) | always (`Models` needs the DB) |

- **Open project**: `project` in App state, its id remembered in `localStorage` `memora.project` and reopened at start (`listProjects`). `Projects` keeps it in sync after rename/delete.
- **Refresh**: App holds `version` (number); it is bumped by every job that leaves queued/running (`onJobUpdated`), by `MediaLibrary.onClipAdded` and `EditingPlan.onApplied`. `NextStep`, `MediaLibrary` and `Timeline` reload when it changes. No context, no store.
- `refresh()` (mount, after saving the data root) calls `getSettings()` and `getHardwareInfo()`; errors go to one red line at the top of `main`.

### App.tsx (components inside)

| Component | Purpose |
|---|---|
| `Row({label, value, warn})` | Label/value line; `warn` → `font-medium text-amber-600`. |
| `DataFolder` | Data-root input. First run (no `dataRoot`): explanation, button "Use this folder"; later "Change". `setDataRoot(path)` → `onSaved`. Shows Current / Log folder, `databaseError` (red), `restartRequired` (amber). Renders `Backup` when the DB opened. |
| `Backup` | "Back up now" → `backupDatabase()`; green "Saved <path>" or red error; text about the automatic backups and the manual restore steps. |
| `Hardware({hw})` | "System" (Windows, CPU, memory, commit — amber below `LOW_COMMIT_BYTES` —, data drive) and "Graphics" (per GPU dedicated / shared memory, driver; AI devices are validated by the worker). |
| `NextStep({project, version, go})` | Indigo banner "Next step" computed from `listMedia`, `listModels`, `listClips(project)`, `getPlan(project)`: no videos → open Library; a job running → wait; missing analysis steps (`ui::preparation`) → **Analyse now** (`ui::prepareMedia`); no project → open Projects; no clips → Smart edit (step 2); else → Edit & export (step 3). Link "Install AI models for better choices" when a model is missing. Not shown on Jobs / Settings. |
| `ProjectPage` | `Projects` (folded in a native `<details>` "Project: … · change, create, rename…" while a project is open) + step bar + step 2 / 3 content (see table above). |

Constants: `LOW_COMMIT_BYTES = 8 * 1024 ** 3` (8 GiB; Production.md §9.2 small pagefile). Helpers `remember(key, value)` / `recall(key)` wrap `localStorage` in try/catch.

### ui.tsx (shared pieces)

| Export | Purpose |
|---|---|
| `btn.primary` / `btn.secondary` / `btn.link`, `field` | Class strings (indigo primary button, bordered secondary, small link-button, input/select). |
| `Card({title, actions?, children})` | Card shell (`rounded-2xl … shadow-sm`), uppercase title, optional right-aligned actions. |
| `fileName(path)`, `Thumb({path, className?})`, `Progress({value 0..1})` | File name from a path; cache thumbnail via `convertFileSrc` or grey placeholder; indigo progress bar. |
| `active(job)`, `jobLabel(kind)` | Queued/running test; bilingual label per job kind. |
| `useJobs()` | `listJobs()` + upsert on every `job-updated`. |
| `JobBar({onOpen})` | Bottom bar while jobs are queued/running: first job label, % and bar, "+N waiting" (→ Jobs page), **Cancel**. Hidden when idle. |
| `preparation(media, models)` | Ids still to analyse (`sceneCount === null`), tag (`tags === null`, only when `openclip-vit-b32` is ready) and transcribe (audio, not transcribed, only when `whisper-small` — else `whisper-base` — is ready); `missingModels`. |
| `prepareMedia(media, models)` | Submits, in this order, `media.analyze {ids, quality: "balanced"}`, `media.tag {ids}`, `media.transcribe {ids, model, language: "auto"}` for what `preparation` lists (the FIFO queue runs them one after the other, so tagging finds the proxies). Returns the number of steps. |

### i18n.ts

`lang` = `localStorage` `memora.lang` (`"en"` → English, anything else → Vietnamese, also when storage is blocked); sets `document.documentElement.lang`. `L(en, vi)` returns the string of the current language — every UI string is written in both languages where it is used (no key table). `dateTime(d)` formats dates in the UI language (`vi-VN` / `en-GB`, short date + time). `setLang(l)` stores and reloads the window (module-level lists such as `PRESETS` are built at import). Backend texts (style names and descriptions, plan reasons, errors, validation checks) stay as Rust sends them.

---

## api.ts

[api.ts](../../apps/desktop/src/api.ts) is the only file that calls `invoke`/`listen`. Rust registers 33 commands (`generate_handler!` in `src-tauri/src/lib.rs`); api.ts has exactly one wrapper per command. Argument keys are camelCase (Tauri converts to the Rust snake_case parameters); all Rust errors arrive as rejected promises with a `String`.

### Command wrappers (33)

| Wrapper | Command | Args | Returns |
|---|---|---|---|
| `getSettings()` | `get_settings` | — | `SettingsView` |
| `setDataRoot(path)` | `set_data_root` | `{ path }` | `SettingsView` |
| `backupDatabase()` | `backup_database` | — | `string` (backup file path) |
| `getHardwareInfo()` | `get_hardware_info` | — | `HardwareInfo` |
| `listProjects()` | `list_projects` | — | `Project[]` |
| `createProject(name)` | `create_project` | `{ name }` | `Project` |
| `renameProject(id, name)` | `rename_project` | `{ id, name }` | `Project` |
| `deleteProject(id)` | `delete_project` | `{ id }` | `void` |
| `listMedia()` | `list_media` | — | `Media[]` |
| `removeMedia(id)` | `remove_media` | `{ id }` | `void` |
| `listScenes(mediaId)` | `list_scenes` | `{ mediaId }` | `ScoredScene[]` |
| `getTranscript(mediaId)` | `get_transcript` | `{ mediaId }` | `Transcript \| null` |
| `setMediaMark(mediaId, mark)` | `set_media_mark` | `{ mediaId, mark }` | `void` |
| `listStyles()` | `list_styles` | — | `EditingStyle[]` |
| `generatePlan(projectId, style, from?, to?, request?)` | `generate_plan` | `{ projectId, style, from, to, request }` (missing → `null`) | `Plan` |
| `interpretRequest(text)` | `interpret_request` | `{ text }` | `RequestReading` |
| `getPlan(projectId)` | `get_plan` | `{ projectId }` | `Plan \| null` |
| `applyPlan(projectId, keep)` | `apply_plan` | `{ projectId, keep }` (indexes into `plan.clips`, in order) | `ExportOptions` (style look applied) |
| `listModels()` | `list_models` | — | `ModelInfo[]` |
| `removeModel(id)` | `remove_model` | `{ id }` | `void` |
| `listClips(projectId)` | `list_clips` | `{ projectId }` | `Clip[]` |
| `addClip(projectId, mediaId, inMs, outMs)` | `add_clip` | `{ projectId, mediaId, inMs, outMs }` | `Clip` |
| `removeClip(id)` | `remove_clip` | `{ id }` | `void` |
| `moveClip(id, delta)` | `move_clip` | `{ id, delta }` | `void` |
| `addPhotoClips(projectId, photoIds, ms?)` | `add_photo_clips` | `{ projectId, photoIds, ms }` | `Clip[]` |
| `setClipRange(id, inMs, outMs)` | `set_clip_range` | `{ id, inMs, outMs }` | `Clip` |
| `setClipTransition(id, transition, transitionMs)` | `set_clip_transition` | `{ id, transition \| null, transitionMs }` | `Clip` |
| `listJoins(projectId)` | `list_joins` | `{ projectId }` | `Join[]` (`[name, ms]` per join) |
| `previewJoin(projectId, index, preset)` | `preview_join` | `{ projectId, index, preset }` | `string` (cache file) |
| `setClipCaption(id, caption)` | `set_clip_caption` | `{ id, caption }` | `void` |
| `listSounds()` | `list_sounds` | — | `Track[]` |
| `recommendSounds(projectId, videoMs?)` | `recommend_sounds` | `{ projectId \| null, videoMs \| null }` | `SoundRecommendation[]` (null project = photo video) |
| `getExportOptions(projectId)` | `get_export_options` | `{ projectId }` | `ExportOptions` |
| `setExportOptions(projectId, options)` | `set_export_options` | `{ projectId, options }` | `void` |
| `submitJob(kind, params?)` | `submit_job` | `{ kind, params }` | `Job` (queued) |
| `listJobs()` | `list_jobs` | — | `Job[]` (in-memory, this session) |
| `cancelJob(id)` | `cancel_job` | `{ id }` | `void` |
| `getWorkerInfo()` | `get_worker_info` | — | `WorkerInfo \| null` (null = worker not running) |
| `listPhotos()` | `list_photos` | — | `Photo[]` |
| `importPhotos(paths)` | `import_photos` | `{ paths }` | `PhotoImport { ids, failed[] }` |
| `setPhotoEdits(id, edits)` | `set_photo_edits` | `{ id, edits }` (`PhotoEdits` or `null`) | `void` |
| `previewPhoto(id, edits, showMask, before)` | `preview_photo` | `{ id, edits, showMask, before }` | `string` (cache file of a ≤ 1600 px preview) |
| `autoPhoto(id)` | `auto_photo` | `{ id }` | `PhotoEdits` (suggested values) |
| `exportPhoto(id)` | `export_photo` | `{ id }` | `string` (exported file) |
| `removePhoto(id)` | `remove_photo` | `{ id }` | `void` |

`submit_job` accepts only `UI_JOB_KINDS` in lib.rs: `validate_devices`, `media.import`, `media.proxy`, `media.export`, `media.analyze`, `media.transcribe`, `media.tag`, `models.install`. Kinds and params the UI actually submits:

| Kind | Submitted by | Params |
|---|---|---|
| `validate_devices` | AiCenter | none |
| `media.import` | MediaLibrary | `{ paths: string[] }` |
| `media.analyze` | `ui::prepareMedia` (after import, Library **Analyse**, Next step **Analyse now**) | `{ ids, quality: "balanced" }` |
| `media.tag` | `ui::prepareMedia` (OpenCLIP ready) | `{ ids }` |
| `media.transcribe` | `ui::prepareMedia` (a Whisper model ready; small preferred) | `{ ids, model: "whisper-base" \| "whisper-small", language: "auto" }` |
| `media.export` | Timeline, EditingPlan ("Use and export now") | `{ projectId, preset, options: ExportOptions }` |
| `models.install` | Models | `{ id, files?: Record<artifactFile, absolutePath> }` (no `files` = download) |
| `media.proxy` | never by the UI (analysis creates missing proxies) | — |

### Event helper

`onJobUpdated(f: (job: Job) => void): Promise<UnlistenFn>` = `listen<Job>("job-updated", e => f(e.payload))`. Rust emits the full `Job` on every state/progress change. Pattern used by every subscriber:

```ts
useEffect(() => {
  const unlisten = onJobUpdated((j) => { /* filter by j.kind */ });
  return () => { unlisten.then((f) => f()); };
}, [deps]);
```

### Helper

`formatBytes(bytes)`: < 1 GiB → `"<n> MB"` (rounded MiB), else `"<n.n> GB"` (GiB, binary units as Windows shows).

### Types

| Type | Backend source | Purpose / key fields |
|---|---|---|
| `JobStatus` | `jobs::Status` (lowercase) | `"queued" \| "running" \| "completed" \| "failed" \| "cancelled"` |
| `Job` | `jobs::Job` | `id, kind, params, status, done, total, result, error` — `params`/`result` are `unknown`, cast per kind in components |
| `WorkerInfo` | worker `hello` message | `protocol, python, openvino \| null, devices[], startupMs, runtimeError?` |
| `DeviceValidation` | `validate_devices` result `devices[]` | `device, name, ok, compileMs?, inferMs?, precision?, relError?, error?` |
| `SettingsView` | lib.rs `SettingsView` | `dataRoot \| null, defaultDataRoot, logDir \| null, restartRequired, databaseError \| null` |
| `Project` | `projects::Project` | `id, name, createdAt, updatedAt` |
| `HardwareInfo` | `hardware::HardwareInfo` | `os{name,edition,displayVersion,build}`, `cpu{name,logicalProcessors}`, `memory{totalPhysical,availablePhysical,commitLimit,commitAvailable,loadPercent}`, `gpus[]{name,vendorId,dedicatedVideoMemory,sharedSystemMemory,driverVersion}`, `dataDisk \| null` |
| `Media` | `media::Media` (+ flattened `Probe`) | `id, path, size, durationMs, width, height` (coded), `rotation` (CCW degrees), `fps, videoCodec, audioCodec, audioStreams, bitrate, sourceCreatedAt, importedAt, thumb, proxy` (absolute cache paths), `sceneCount` (null = not analysed), `transcribed`, `tags: TagScore[] \| null` (null = not tagged), `mark` |
| `Mark` | `editing::Mark` | `favorite, excluded` (user overrides for Smart edit) |
| `EditingStyle` | `editing::Style` | `id, name, targetMs, exportPreset, clipMinMs, clipMaxMs, clipIdealMs, transitionMs, music, titles, pacing, opening, ending` (Rust also sends `weights, favor, openWith, endWith, transition, color, captions`, not typed) |
| `PlanClip` | `editing::PlanClip` | `mediaId, sceneIndex, inMs, outMs, scores{technical,semantic,speech,favorite}, rank, tags[], reasons[]` |
| `Wishes` | `editing::Wishes` | `prefer[], less[], avoid[]` (tag `en` names), `targetMs, preset, color, title, ending, captions, music` (all nullable), `request` (raw text) |
| `RequestReading` | `request::Request` | `style, from, to` (nullable), `wishes`, `understood[]` (human-readable chips) |
| `Plan` | `editing::Plan` | `style, wishes, targetMs, totalMs, transitionMs, exportPreset, clips[], skipped[]` (why files were not used), `candidates, suggestedTitle`, `notes?` (how the plan was built) |
| `ExportOptions` | `export::Options` | `fill, transitionMs, transition, color, fade, originalVolume, music{path, library \| null, volume, duck} \| null, titles{title,subtitle,ending,captions}, normalize` |
| `Track` | `sounds::Track` (sounds.json) | `id, title, artist, moods[], energy, durationMs, url, license, credit` (Rust also has `sha256, sizeBytes, licenseUrl`) |
| `SoundRecommendation` | `sounds::Recommendation` | `id, score, reasons[]` |
| `TagScore` | `tagging::TagScore` | `en, vi, score, negative` (negative = bad-shot detector) |
| `Transcript` | `transcribe::Transcript` | `mediaId, modelId, language, segments[]{startMs,endMs,text}, createdAt` (times in the original file) |
| `Clip` | `timeline::Clip` | `id, projectId, mediaId, position, inMs, outMs, caption` |
| `ExportPreset` | `export::Preset` | `"landscape" \| "story" \| "reels" \| "archive"` |
| `ExportResult` | `media.export` job result JSON | `path, preset, encoder, renderSeconds, durationMs, width, height, validation{ok, checks[]{name,ok,detail}, warnings[]}` (Rust also sends `transitionMs, options, music, loudness`) |
| `ModelInfo` | `models::ModelView` (+ flattened `Manifest`) | `id, name, version, category, description, license, minimumRamGb, artifacts[]{file,url,sha256,sizeBytes}, runtimes[]{runtime,devices,validated}, notes, state, error, path, details{devices?[]}, downloadBlocker` |
| `Photo` | `photos::Photo` | `id, path, size, width, height, takenAt, edits: PhotoEdits \| null, importedAt, thumb` |
| `PhotoEdits` | photo.py edit stack | numbers −100..100 by key, plus `look`, `crop`, `rotate`, `hsl: {band: [hue, sat, light]}` |
| `ScoredScene` | `scoring::ScoredScene` (+ flattened `Scene`) | `id, mediaId, index, startMs, endMs, shake, framePath, samples[]`, `technical{sharpness,exposure,brightness,contrast,stability,total}` (0–1), `flags[], duplicates[], tags[]` |

Local result types (not in api.ts): `ImportResult { imported, ids, failed[]{path,error} }` and `BatchResult { failed[]{id,error} }` in MediaLibrary.tsx.

---

## Components

### Projects.tsx

- **Purpose**: list, create, rename, delete, open projects (`Card` "Projects / Dự án").
- **Props**: `active: Project | null`, `onOpen(p: Project | null)`.
- **State**: `projects: Project[] | null`, `name` (new-project input), `error`. `ProjectRow`: `editing`, `name`, `confirming`.
- **Sub-components**: `ProjectRow({project, active, onOpen, onChanged, onError})` (active row highlighted indigo with "open").
- **Commands**: `listProjects`, `createProject`, `renameProject`, `deleteProject`.
- **Events**: none.
- **Actions**: **Create** (form; disabled when blank) → `createProject`, reload. **Open** (secondary button, hidden on the active row) → `onOpen(p)`. **Rename** → inline input (Escape cancels) → **Save** / **Cancel**. **Delete** → inline confirm → **Delete** / **Keep**. After every reload the open project is updated or cleared.
- **Messages**: empty list explains what a project is; errors red under the form.

### EditingPlan.tsx

- **Photos** (2026-11-12): plan rows with `photoId` show the photo thumbnail (🖼, seconds) from `listPhotos`.

- **Purpose**: Smart mode (Production.md §6.4), step 2 of a project: free-text request + style + date range → Editing Plan, review / reorder / drop clips, use it (replaces the timeline), optionally export at once.
- **Props**: `project: Project`, `onApplied()` (App bumps `version` and opens step 3).
- **State**: `styles`, `styleId` (default `"journey_recap"`), `rangeId` (default `"all"`), `from`, `to`, `plan`, `order: number[]`, `media: Map<id, Media>`, `message` (errors), `busy`, `request`, `reading`, `replacing: {clips, thenExport} | null`.
- **Sub-components**: `RequestBox({text, onChange, reading})` — 3-row textarea; "Understood:" chips, amber "Nothing recognised yet. Words that work: …", or the word list while empty.
- **Helpers**: `range(id, from, to)` (custom dates, or today minus 1/6/12 months via `toISOString`); `show(p)`.
- **Commands**: `listStyles`, `listMedia`, `getPlan`, `interpretRequest` (250 ms debounce), `generatePlan`, `listClips`, `applyPlan`, `submitJob("media.export")`.
- **Actions**: style and date-range selects (+ two date inputs); **Make a plan / Make the plan again** ("Choosing scenes…" while busy); per clip **↑ ↓ Drop**; **Restore dropped clips**; **Use this plan →** and **Use and export now** — when the timeline already has clips, an amber inline confirm "The timeline already has N clips. Replace them with this plan?" (**Replace** / **Cancel**) comes first; then `applyPlan(project.id, order)` (+ `media.export` with the returned options and `plan.exportPreset`) and `onApplied()`.
- **Display**: style summary (pacing, clip min–max, 16:9 / vertical, opening, ending — backend text). Plan header "K of N clips · total of target · chosen from C candidate windows" (total = `plan.totalMs` while the order is unchanged, else Σ kept − (kept−1) × `transitionMs` as an estimate), score legend, request, `plan.notes` (indigo), `plan.skipped` (amber). Rows: number, `Thumb`, first tag (else file name), file name + in–out, reasons, T/S/V + ★. List scrolls (`max-h-[28rem]`).
- **Constants**: `RANGES`, `EXAMPLE` (Vietnamese trip-recap request), `WORDS` (bilingual vocabulary hint incl. đường rừng, qua cầu, lên dốc), `iso`, `pct`. Imports `clock` from Timeline.tsx.

### Timeline.tsx

- **Purpose**: step 3 of a project: the timeline (ordered video clips and photos) as a visual strip with every transition, per-join transition choice and preview, per-clip captions, Look & sound, export with progress and validation report.
- **Props**: `project: Project`, `version: number`.
- **State**: `clips`, `media`, `photos` (map), `joins`, `adding` (photo picker open), `preset` (default `"landscape"`), `options`, `job` (latest `media.export` of this project), `error`.
- **Exports**: `clock(ms)` → `m:ss.s` (also used by EditingPlan and MediaLibrary); `PRESETS`, `Report`, `LookAndSound` (used by PhotoEditor); `openPhoto(id)` (dispatches `memora:open-photo`).
- **Sub-components**:
  - `Report({r})` — green (passed) or red (failed validation) box, output path + **copy path** (`navigator.clipboard`), size · duration · render time (encoder), checks ✓/✗, warnings.
  - `CaptionField({clip, onSaved, onError})` — saved with `setClipCaption` on blur when changed.
  - `Strip({project, clips, joins, media, photos, preset, onChanged, onError})` — horizontal blocks (width `ms/1000 × 14` px, 56–260), join chips (selected = indigo, set on the clip = amber); join panel: style select (`Project setting` / `TRANSITIONS`) and length (`JOIN_LENGTHS`, when set) → `setClipTransition`; **▶ Preview this join** → `previewJoin` → `<video>`.
  - `AddPhotos({project, onAdded, onClose, onError})` — library photos (`listPhotos`), click order badges, seconds (`PHOTO_SECONDS`), **Add N photos** → `addPhotoClips`.
  - Photo rows: 🖼 name, **Edit photo** (`openPhoto`), seconds select → `setClipRange(id, 0, ms)`.
  - `MusicLibrary({projectId | null, videoMs?, version, current, onUse, onClose})` — free library: recommendations (`recommendSounds`), mood chips, ▶/■ streamed preview, **Use** / "Chosen"; licence/credit text.
  - `LookAndSound({o, projectId | null, videoMs?, photos?, version, onChange, onError})` (exported; `photos` hides clip sound, ducking and speech subtitles) — Picture: transition length (0 cuts, 300, 500, 700, 800, 1200 ms; other values appended), transition style (`TRANSITIONS`, when length > 0), colour (`COLORS`), fill, fade. Sound: clip sound %, music (library / own file / remove, level, duck), even loudness. Text: opening title, subtitle, ending, speech subtitles.
- **Commands**: `listClips`, `listJoins`, `listMedia`, `listPhotos`, `getExportOptions`, `setExportOptions`, `moveClip`, `removeClip`, `setClipCaption`, `setClipRange`, `setClipTransition`, `addPhotoClips`, `previewJoin`, `listSounds`, `recommendSounds`, `submitJob("media.export")`, `cancelJob`.
- **Events**: `job-updated` filtered to `media.export` of this project (exports started from Smart edit appear too).
- **Actions**: card actions = export-format select (`PRESETS`; sets `fill` for story/reels) + **Export video**; per clip caption, **↑ ↓ Remove**; every Look & sound change saved at once (`setExportOptions`); **Cancel** during export.
- **Display**: export progress and report at the top of the card; rows with number, `Thumb`, file, caption, in → out, length; footer "About <total> · rendered from the original files" (estimate: Σ − (n−1) × min(transition, half the shortest clip); the export clamps per join and `auto` varies the length).
- **Constants**: `PRESETS`, `TRANSITIONS` (`auto` Automatic (by recording time), `fade` Cross dissolve, `fadeblack` Dip to black, `dissolve` Film dissolve (grain), `smoothleft`, `slideleft`, `wipeleft`, `circleopen`), `COLORS`, `AUDIO_EXTENSIONS`, `label`, `minutes(ms)` — all labels bilingual.

### MediaLibrary.tsx

- **Purpose**: the Library page: import videos (analysed automatically), one-click analysis of what is missing, search, preview with scenes and transcript, In/Out and add to the open project, favorite / exclude / remove.
- **Props**: `project: Project | null`, `version`, `onClipAdded()`, `onSettings()` (opens Settings for missing AI models).
- **State**: `media`, `models`, `importJob`, `job` (latest proxy/analyze/tag/transcribe job), `selected`, `error`, `query`.
- **Sub-components**: `SceneCard({s, onPick})` (frame, range, technical score coloured, top tags, ⚠ negative tags, flags); `Preview({m, project, onClipAdded})` (proxy video or thumbnail, path line, scene strip, transcript lines, Set In / Set Out, **Add to <project>**); `MediaRow` (thumbnail, file, recorded, length, resolution + fps, size, Analysis, Speech, On screen; ★, ⊘, Remove with inline confirm).
- **Helpers**: `fold`, `matches`, `topTags`, `duration`, `resolution`, `working`, `status`, `speech` (bilingual texts).
- **Commands**: `listMedia`, `listModels`, `submitJob` (`media.import`, and through `ui::prepareMedia` `media.analyze`, `media.tag`, `media.transcribe`), `cancelJob`, `listScenes`, `getTranscript`, `addClip`, `setMediaMark`, `removeMedia`.
- **Events**: `media.import` → `importJob`; on completion `prepareMedia` for the imported ids (analysis, then tagging and speech when the models are installed). `BACKGROUND` kinds → `job`; reload when a file finishes or the job ends.
- **Actions**: **+ Add videos** (card action, also in the empty state); **Analyse (N steps)** (when `preparation` lists work and nothing runs; tooltip lists the three steps); search input; Cancel links on job lines. The speech model is chosen automatically (Whisper small when installed, else base).
- **Messages**: privacy note + "New videos are analysed automatically"; link "Settings › AI models" when a model is missing; job label + % + progress bar; import result and per-file failures; empty state with a button; "No match.".
- **Constants**: `VIDEO_EXTENSIONS`, `STEP = 1000`, `BACKGROUND`.

### PhotoEditor.tsx

- **Purpose**: the Photos page (ADR 0006, slice B1): add photos (read in place), pick one, edit with classical tools, export a full-size copy; or make a video from picked photos. Original files are never written.
- **Props**: none (keyed on the data root).
- **State** (`PhotoEditor`): `photos`, `selected` (initially `localStorage` `memora.photo`, set by "Edit photo" on a timeline clip), `analysing` (ref: one `media.photo_analyze` submit per visit for photos with `analyzed` false; `job-updated` of that kind reloads the list), `importing`, `message`, `confirming` (remove), `video` (video mode), `picked` (ordered photo ids). (`PhotoVideo`): `s: {options: ExportOptions, photoMs, preset, name}` (`localStorage` `memora.photoVideo`), `job`, `error`. (`Editor`): `edits`, `history` (≤ 50 undo steps), `showMask`, `before` (URL), `showBefore`, `band` (HSL band), `message`, `working`.
- **Sub-components / hooks**: `usePreview(id, edits, showMask, onError)` — 80 ms after a change, one `previewPhoto` in flight, the latest edits win, returns `{url, busy}`; `Editor({photo, onChanged})`; `PhotoVideo({photos, picked, setPicked})` — timeline strip (photo blocks with order and seconds, transition length between them, ◀ ✕ ▶), seconds per photo, format, name, `LookAndSound` (photos), **Create video** (`submitJob("media.photo_video")`), progress / cancel, `Report`; first use pre-selects the top `recommendSounds(null, …)` track.
- **Commands**: `listPhotos`, `importPhotos`, `setPhotoEdits`, `previewPhoto`, `autoPhoto`, `exportPhoto`, `removePhoto`, `recommendSounds`, `submitJob("media.photo_video")`, `cancelJob`. Imports `PRESETS`, `LookAndSound`, `Report`, `clock` from `Timeline.tsx`.
- **Actions**: **+ Add photos** (dialog, `PHOTO_EXTENSIONS`; first imported photo selected; per-file failures shown); thumbnail grid (✎ badge when edited; ✕ on hover → inline **Remove** / **Keep**); **✦ Auto** (merges `autoPhoto` values into the sliders); **↶ Undo**; **Reset**; **Hold: original** (press and hold shows `before`); **Show sky area** (mask overlay); **Export photo** (saves edits, then `exportPhoto`; path + copy link). Sliders save 500 ms after the last change (one undo step per save); double-click a slider label resets it.
- **Panels** (`GROUPS`): Light (exposure, contrast, highlights, shadows, whites, blacks) · Colour (temperature, tint, vibrance, saturation, look select `LOOKS`, "One colour (HSL)" with 8 `BANDS` × hue / saturation / lightness) · Sky (enhance 0–100, cool ↔ warm; note that the photo's own sky is enhanced, nothing pasted) · Detail (clarity, dehaze, sharpen 0–100 edges only, noise reduction 0–100) · Effects & frame (vignette, straighten ±15°, rotate ⟲ ⟳, crop `CROPS`).
- **Constants**: `PHOTO_EXTENSIONS`, `GROUPS`, `LOOKS`, `BANDS`, `CROPS` — labels bilingual.

### AiCenter.tsx

- **Purpose**: the Jobs page: every job of this session (newest first), AI worker status, device validation.
- **Props**: none.
- **State**: `jobs`, `worker`, `error`.
- **Sub-components**: `DeviceTable({devices})`; `JobRow({job})` (`#id` + `jobLabel(kind)`, status, `Progress`, **Cancel**, error, device table).
- **Commands**: `listJobs`, `getWorkerInfo`, `submitJob("validate_devices")`, `cancelJob`.
- **Events**: every `job-updated` upserts the job and re-reads `getWorkerInfo()`.
- **Actions**: **Validate AI devices** (card action; disabled while one runs).
- **Constants**: `ACTIVE = {queued, running}`.

### Models.tsx

- **Purpose**: "AI models" card (Settings page) — model manifests with install state, download (with confirmation), offline install from files, remove.
- **Props**: none.
- **State**: `models: ModelInfo[]`, `jobs: Record<modelId, Job>` (latest `models.install` per model), `error`. `ModelRow`: `confirming`, `error`.
- **Sub-components**: `ModelRow({m, job, onChanged})`; helper `size(m)` → total `formatBytes` when every artifact has `sizeBytes`, else "size not verified".
- **Commands**: `listModels`, `removeModel`, `submitJob("models.install")`, `cancelJob`.
- **Events**: `models.install` only → store by `params.id`, then reload the list (state transitions are stored in the DB).
- **Actions**:
  - **Download** / **Retry download** (when state is in `FAILED`) — disabled with tooltip when `downloadBlocker` is set → confirmation box (name, size, license, artifact URLs, "Files are checked against their SHA-256 before installation.") → **Download** `models.install { id }` / **Cancel**.
  - **Install from files…** → `open({ multiple: true })`; each manifest artifact is matched by case-insensitive file name (a single artifact + single pick matches regardless of name); missing → "Select these files: …" → `models.install { id, files }`.
  - **Remove** (when state is not `not_installed` and not in `WORKING`) → `removeModel(id)`.
  - **Cancel** replaces the buttons while the install job is queued/running.
- **Display**: name · category · size · runtimes; description; state text (underscores → spaces; green `ready`, red failed states, grey otherwise) + install percent while running; `downloadBlocker` text when not installed; `m.error` red; "Load test: DEVICE ok (compile X ms, run Y ms) · …" from `details.devices`. Intro: models are optional and local, what OpenCLIP / Whisper add to Smart edit, nothing downloads without confirmation. All labels bilingual.
- **Constants**: `WORKING` = downloading, verifying, installing, validating; `FAILED` = download_failed, checksum_failed, install_failed, model_load_failed, incompatible; `linkClass` (= `btn.link`); `fileName` from ui.tsx.

---

## Job progress in the UI

| Job kind | Shown in | Reading of `done/total` | Cancel |
|---|---|---|---|
| any | `JobBar` (bottom, every page) and Jobs page `JobRow` | first queued/running job: label, %, bar; "+N waiting" | **Cancel** |
| `media.import` | Library "Reading video metadata… done/total" | files read / file count | link on that line |
| `media.analyze`, `media.proxy`, `media.tag`, `media.transcribe` | Library job line (label + % + bar) and per-row Analysis / Speech / On screen cells | `total = files × 1000`; file at index `i` is at `(done − i×1000)/1000`; 1–99 % → "working N%", else "waiting" | link on the job line |
| `media.export` | Timeline (filtered by `params.projectId`) | `total = 1000`: 0–50 music download, 50–900 render, 900–1000 validation; "Waiting for other jobs…", "Rendering from the original files…", "Checking the result…" | **Cancel** |
| `models.install` | Models row state + percent | `done/total×100` | **Cancel** replaces row buttons |

- Only one job runs at a time (FIFO in Rust): "Analyse" submits up to three jobs that run one after the other.
- Jobs list is in memory in Rust; `useJobs()` loads `listJobs()` on mount.
- Cancellation leaves no output file (`.part` + rename in Rust); Timeline says so.

---

## Styling

- **Tailwind v4** via `@tailwindcss/vite`; [index.css](../../apps/desktop/src/index.css) is only `@import "tailwindcss";` and `:root { color-scheme: light dark; }`. No config file, no `@apply`.
- **Dark mode** follows Windows (`prefers-color-scheme`); no toggle.
- **Palette**: zinc neutrals; **indigo** accent (primary buttons, active nav item, open project, step bar, "Next step" banner, progress bars); red errors, amber warnings / confirmations, green success. The video preview is always dark.
- **Shared classes** live in [ui.tsx](../../apps/desktop/src/ui.tsx) (`btn`, `field`, `Card`, `Thumb`, `Progress`); a few local class strings remain (`label` in Timeline, preview buttons in MediaLibrary).
- **Icons**: Unicode only — nav `▦ ✎ ⟳ ⚙`, `★` favorite, `⊘` exclude, `↑ ↓`, `▶ ■`, `♪`, `⚠`, `✓ ✗ !`.
- **Responsive**: step bar `sm:grid-cols-3`, Settings `lg:grid-cols-2`, Look & sound `md:grid-cols-3`, library table in `overflow-x-auto`.
- Native controls only; `aria-label` on unlabeled inputs and arrow buttons, `aria-pressed` on toggles.

---

## Conventions and how to add UI

- **Add a backend call**: add the Rust command to `generate_handler!` (lib.rs), then one wrapper in [api.ts](../../apps/desktop/src/api.ts): `export const fooBar = (projectId: number) => invoke<T>("foo_bar", { projectId });` with camelCase arg keys and a TS interface mirroring the serde (`rename_all = "camelCase"`) struct; reference the Rust file in a `/** */` comment. Components never import `invoke` directly.
- **Add a job kind**: add it to `UI_JOB_KINDS` in lib.rs, submit with `submitJob(kind, params)`, and filter `onJobUpdated` by `j.kind` (and by a param such as `projectId` when several instances exist). For list jobs follow the 1000-units-per-file convention so `working()` keeps working. Add a label in `ui::jobLabel`.
- **Add a card**: new `src/<Name>.tsx` with a default-exported component wrapped in `ui::Card`; mount it on a page in `App.tsx` (or add a page to `nav`), gated on `ready` (data root + DB) if it touches the DB, keyed on `settings.dataRoot`. Share refreshes through App `version`, not a store. Write every string as `L("English", "Tiếng Việt")`.
- **Errors**: every call ends in `.catch((e) => setError(String(e)))` (or `setMessage`); show `{error && <p className="mt-2 text-sm text-red-600">{error}</p>}` in the card. Amber for non-fatal warnings, grey `text-zinc-500` for info/empty states. Destructive actions use an inline confirm (Delete/Remove → confirm + Keep), never `window.confirm`.
- **Files**: pickers via `@tauri-apps/plugin-dialog` `open()` with explicit extension filters; local images/videos only via `convertFileSrc` on paths under `<data root>\cache` (asset scope + CSP block anything else).
- **Rules**: the frontend never runs FFmpeg or loads models (CLAUDE.md); exports always render from originals — UI copy says so.
- **Checks**: `npm run build` (tsc strict) in `apps/desktop`; there are no frontend unit tests. UI screenshots: capture only the app window (PrintWindow).
- **Docs to update** after a UI change: this file; [FLOWS.md](FLOWS.md) if a user flow changes; [BACKEND.md](BACKEND.md) for new commands/jobs; [../APP_OVERVIEW.md](../APP_OVERVIEW.md) for user-visible features; [../PROGRESS.md](../PROGRESS.md) day log. Run the `sync-docs` skill.
