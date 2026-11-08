# Memora Studio — Frontend

The React webview of the Tauri app: one scrolling page of cards that call Rust commands through typed wrappers and follow background jobs through `job-updated` events.

Last verified: 2026-11-08 against commit `b751673`.

Index of all system docs: [README.md](README.md). The `sync-docs` skill (`.claude/skills/sync-docs/`) keeps this file current; run it after any change under `apps/desktop/` outside `src-tauri/`.

Sources: [apps/desktop/src/](../../apps/desktop/src/) — [main.tsx](../../apps/desktop/src/main.tsx), [App.tsx](../../apps/desktop/src/App.tsx), [api.ts](../../apps/desktop/src/api.ts), [Projects.tsx](../../apps/desktop/src/Projects.tsx), [EditingPlan.tsx](../../apps/desktop/src/EditingPlan.tsx), [Timeline.tsx](../../apps/desktop/src/Timeline.tsx), [MediaLibrary.tsx](../../apps/desktop/src/MediaLibrary.tsx), [AiCenter.tsx](../../apps/desktop/src/AiCenter.tsx), [Models.tsx](../../apps/desktop/src/Models.tsx), [index.css](../../apps/desktop/src/index.css). Backend counterpart: [BACKEND.md](BACKEND.md).

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

- CSP: `img-src 'self' data: asset: http://asset.localhost`; `media-src 'self' asset: http://asset.localhost https://incompetech.com`; `style-src 'self' 'unsafe-inline'`. Images/videos load only from the asset protocol (plus music previews from incompetech.com).
- Asset protocol scope is empty in config; Rust adds `<data root>\cache` at runtime, so `convertFileSrc` works only for thumbs, proxies and scene frames.
- Window: 1200×800, min 900×600.

---

## Page layout

Single page, no router. [App.tsx](../../apps/desktop/src/App.tsx) renders `<main>` (full-height, `p-6`) with a header ("Memora Studio" + **Refresh** link) and a grid `max-w-5xl gap-4 lg:grid-cols-2`. Every card except the two hardware cards spans both columns (`lg:col-span-2`).

| # | Card | Render condition | `key` | Props |
|---|---|---|---|---|
| 1 | `DataFolder` (in App.tsx) | `settings` loaded | — | `settings`, `onSaved` (sets settings, then `refresh()`) |
| 2 | `Projects` | `settings.dataRoot && !settings.databaseError` | `settings.dataRoot` | `active={project}`, `onOpen={setProject}` |
| 3 | `EditingPlan` ("Smart edit") | same + `project` open | `` `${dataRoot}-${project.id}` `` | `project`, `onApplied` → bump `timelineVersion` |
| 4 | `Timeline` | same + `project` open | `` `${dataRoot}-${project.id}` `` | `project`, `version={timelineVersion}` |
| 5 | `MediaLibrary` | data root + no DB error | `settings.dataRoot` | `project` (may be null), `onClipAdded` → bump `timelineVersion` |
| 6 | `AiCenter` | always | — | none |
| 7 | `Models` ("AI models") | data root + no DB error | `settings.dataRoot` | none |
| 8 | `Hardware` → two cards "System" and "Graphics" (one column each on `lg`) | `hw` loaded, else "Detecting hardware…" | — | `hw` |

- **Remount keys**: changing the data root switches database, so every DB-backed card is keyed on `settings.dataRoot` and loses its local state. Switching project remounts `EditingPlan` and `Timeline` (key includes `project.id`).
- **Cross-card refresh**: App holds `timelineVersion` (number). `EditingPlan.onApplied` and `MediaLibrary.onClipAdded` increment it; `Timeline` re-runs `load()` and `getExportOptions()` when `version` changes. Nothing else is shared; e.g. Smart edit does not see library changes until it remounts or regenerates (it refreshes its media map after each Generate).
- **Open project**: lives only in App state (`project: Project | null`), not persisted; it is lost on restart or data-root change. `Projects` keeps it in sync after rename/delete (calls `onOpen(now)` / `onOpen(null)`).
- **App state**: `settings`, `hw`, `error`, `project`, `timelineVersion` in `App`; everything else is local `useState` per card. No context, no store.
- `refresh()` (on mount, header **Refresh**, after saving the data root) calls `getSettings()` and `getHardwareInfo()`; errors go to one red line under the header.

### App.tsx (components inside)

| Component | Purpose |
|---|---|
| `Card({title, children})` | Shared card shell: `rounded-xl border … bg-white p-5 dark:…` + uppercase `h2`. Used only inside App.tsx; other files repeat the same classes inline. |
| `Row({label, value, warn})` | Label/value line; `warn` → `font-medium text-amber-600`. |
| `DataFolder` | Data-root input. First run (no `dataRoot`): explanation paragraph, button "Use this folder"; later "Change". Disabled while saving, when empty, or unchanged. `setDataRoot(path)` → `onSaved`. Shows Current / Log folder ("not active (console only)" when `logDir` null), `databaseError` (red), `restartRequired` → amber "Restart Memora Studio to write logs to the new folder." Renders `Backup` when a data root is set and the DB opened. |
| `Backup` | "Back up now" → `backupDatabase()`; green "Saved <path>" or red error. Static text: auto backup at start, last 10 kept in `database\backups`, restore instructions (copy over `database\memora.db`, delete `-wal`/`-shm`). |
| `Hardware({hw})` | "System": Windows name/edition/version/build, CPU + logical processors, memory free/total/load, commit free/limit (amber when `commitAvailable < LOW_COMMIT_BYTES`), data drive free/total. "Graphics": per GPU dedicated / shared ("part of system RAM") / driver; "No hardware GPU detected…" when empty; note that AI devices are validated by the worker. |

Constant: `LOW_COMMIT_BYTES = 8 * 1024 ** 3` (8 GiB; Production.md §9.2 small pagefile).

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
| `setClipCaption(id, caption)` | `set_clip_caption` | `{ id, caption }` | `void` |
| `listSounds()` | `list_sounds` | — | `Track[]` |
| `recommendSounds(projectId)` | `recommend_sounds` | `{ projectId }` | `SoundRecommendation[]` |
| `getExportOptions(projectId)` | `get_export_options` | `{ projectId }` | `ExportOptions` |
| `setExportOptions(projectId, options)` | `set_export_options` | `{ projectId, options }` | `void` |
| `submitJob(kind, params?)` | `submit_job` | `{ kind, params }` | `Job` (queued) |
| `listJobs()` | `list_jobs` | — | `Job[]` (in-memory, this session) |
| `cancelJob(id)` | `cancel_job` | `{ id }` | `void` |
| `getWorkerInfo()` | `get_worker_info` | — | `WorkerInfo \| null` (null = worker not running) |

`submit_job` accepts only `UI_JOB_KINDS` in lib.rs: `validate_devices`, `media.import`, `media.proxy`, `media.export`, `media.analyze`, `media.transcribe`, `media.tag`, `models.install`. Kinds and params the UI actually submits:

| Kind | Submitted by | Params |
|---|---|---|
| `validate_devices` | AiCenter | none |
| `media.import` | MediaLibrary | `{ paths: string[] }` |
| `media.analyze` | MediaLibrary (auto after import, button) | `{ ids, quality: "balanced" }` |
| `media.tag` | MediaLibrary | `{ ids }` |
| `media.transcribe` | MediaLibrary | `{ ids, model: "whisper-base" \| "whisper-small", language: "auto" }` |
| `media.export` | Timeline, EditingPlan ("Apply and export") | `{ projectId, preset, options: ExportOptions }` |
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
| `Plan` | `editing::Plan` | `style, wishes, targetMs, totalMs, transitionMs, exportPreset, clips[], skipped[]` (why files were not used), `candidates, suggestedTitle` |
| `ExportOptions` | `export::Options` | `fill, transitionMs, transition, color, fade, originalVolume, music{path, library \| null, volume, duck} \| null, titles{title,subtitle,ending,captions}, normalize` |
| `Track` | `sounds::Track` (sounds.json) | `id, title, artist, moods[], energy, durationMs, url, license, credit` (Rust also has `sha256, sizeBytes, licenseUrl`) |
| `SoundRecommendation` | `sounds::Recommendation` | `id, score, reasons[]` |
| `TagScore` | `tagging::TagScore` | `en, vi, score, negative` (negative = bad-shot detector) |
| `Transcript` | `transcribe::Transcript` | `mediaId, modelId, language, segments[]{startMs,endMs,text}, createdAt` (times in the original file) |
| `Clip` | `timeline::Clip` | `id, projectId, mediaId, position, inMs, outMs, caption` |
| `ExportPreset` | `export::Preset` | `"landscape" \| "story" \| "reels" \| "archive"` |
| `ExportResult` | `media.export` job result JSON | `path, preset, encoder, renderSeconds, durationMs, width, height, validation{ok, checks[]{name,ok,detail}, warnings[]}` (Rust also sends `transitionMs, options, music, loudness`) |
| `ModelInfo` | `models::ModelView` (+ flattened `Manifest`) | `id, name, version, category, description, license, minimumRamGb, artifacts[]{file,url,sha256,sizeBytes}, runtimes[]{runtime,devices,validated}, notes, state, error, path, details{devices?[]}, downloadBlocker` |
| `ScoredScene` | `scoring::ScoredScene` (+ flattened `Scene`) | `id, mediaId, index, startMs, endMs, shake, framePath, samples[]`, `technical{sharpness,exposure,brightness,contrast,stability,total}` (0–1), `flags[], duplicates[], tags[]` |

Local result types (not in api.ts): `ImportResult { imported, ids, failed[]{path,error} }` and `BatchResult { failed[]{id,error} }` in MediaLibrary.tsx.

---

## Components

### Projects.tsx

- **Purpose**: list, create, rename, delete, open projects.
- **Props**: `active: Project | null`, `onOpen(p: Project | null)`.
- **State**: `projects: Project[] | null`, `name` (new-project input), `error`. `ProjectRow`: `editing`, `name`, `confirming`.
- **Sub-components**: `ProjectRow({project, active, onOpen, onChanged, onError})`.
- **Commands**: `listProjects`, `createProject`, `renameProject`, `deleteProject`.
- **Events**: none.
- **Actions**:
  - **Create** (form submit; disabled when name blank) → `createProject(name)`, clear input, reload.
  - **Open** (hidden on the active row) → `onOpen(p)`; active row shows green "open".
  - **Rename** → inline input (autofocus; Escape cancels) → **Save** `renameProject` then reload / **Cancel** restores name.
  - **Delete** → confirm text "Delete project? Original videos are not affected." → red **Delete** `deleteProject` / **Keep**.
  - Effect: after every reload, if the active project is gone → `onOpen(null)`; if renamed → `onOpen(updated)`.
- **Messages**: "No projects yet." when empty; errors red under the form. Rows show "Updated <locale date-time>".
- **Constants**: `inputClass`, `linkClass`; placeholder "New project name, e.g. Đà Lạt 2026".

### EditingPlan.tsx

- **Purpose**: Smart mode (Production.md §6.4): free-text request + style + date range → generate an Editing Plan, review/reorder/drop clips, apply to the timeline (optionally export).
- **Props**: `project: Project`, `onApplied()`.
- **State**: `styles`, `styleId` (default `"travel_cinematic"`), `rangeId` (default `"all"`), `from`, `to` (custom dates), `plan`, `order: number[]` (indexes into `plan.clips` kept, in display order), `media: Map<id, Media>`, `message`, `busy`, `request` (text), `reading: RequestReading | null`.
- **Sub-components**: `RequestBox({text, onChange, reading})` — textarea (2 rows) + "Understood:" chips, or amber "Nothing recognised yet. Words that work: …" when `understood` is empty.
- **Helpers**: `range(id, from, to)` → `{}` for all; custom → given dates; `1/6/12` → `from` = today minus N months, `to` = today (both `toISOString().slice(0,10)`). `show(p)` sets plan, resets `order` to all clips, sets `styleId = p.style`.
- **Commands**: `listStyles`, `listMedia`, `getPlan`, `interpretRequest`, `generatePlan`, `applyPlan`, `submitJob("media.export")`.
- **Events**: none (export progress is shown by Timeline, which listens for `media.export` of this project).
- **Lifecycle**: on mount / project change: load styles, media map, saved plan (`getPlan`); restores `request` from `plan.wishes.request`. While typing, `interpretRequest(request)` runs after a 250 ms debounce; empty text clears `reading`.
- **Actions**:
  - Style `<select>` (`name · N s`), date-range `<select>` (`RANGES`), two `<input type="date">` when "Custom dates…".
  - **Generate plan / Generate again** (disabled while `busy`) → `generatePlan(project.id, styleId, from, to, request.trim() || undefined)` → `show()`, then refresh the media map.
  - Per clip: **↑** / **↓** reorder within `order` (disabled at ends), **Drop** removes from `order`. **Restore dropped clips** resets `order` to all.
  - **Apply to timeline** (disabled when nothing kept) → `applyPlan(project.id, order)` → `onApplied()` → message "Timeline replaced with N clips and the style's look set (<color> colour, title "<title>"). Adjust anything in Timeline › Look & sound before exporting."
  - **Apply and export** → same, then `submitJob("media.export", { projectId, preset: plan.exportPreset, options })` with the returned options → message "…export started with the style's look (see Timeline)." Tooltip names "Reels 9:16 (centre crop)" or "Landscape 16:9".
- **Display**: style summary line (pacing, clip min–max s, crossfade ms or "hard cuts", "vertical 9:16" when preset is reels else "16:9", opening/ending, music and titles text). Plan header: "K of N clips kept · <total> of <target> target · chosen from C candidate windows" + score legend (T technical · S visual concepts · V speech · ★ favorite). Total = sum of kept durations − (kept−1) × `plan.transitionMs`. "Made from your request: “…”" when the plan has a request. `plan.skipped[]` lines amber. Rows: number, thumbnail, file name + `clock(in)–clock(out)`, reasons (truncated, full in tooltip), `T S V` percentages + ★. List `max-h-96` scrolls.
- **Messages**: "The style and dates named in your request replace the selection below." when `reading.style` is set; "Nothing to use: analyse videos in the Media library first (and tag / transcribe them for better choices)." when the plan has no clips; all errors and results in one grey `message` line.
- **Constants**: `RANGES` (`all` Whole library, `1` Last month, `6` Last six months, `12` Last year, `custom` Custom dates…); `EXAMPLE` (Vietnamese sample request used in the placeholder: "Video dọc 30 giây, nhiều cảnh cây cổ thụ và đường mòn, bớt xe đạp, bỏ selfie, tông ấm, nhạc nhẹ, tiêu đề: Đà Lạt"); `WORDS` (vocabulary hint: length, dọc/ngang, nhiều/bớt/bỏ + concept, colour tones, music, phụ đề, tiêu đề:, kết thúc bằng:, month/year, mood styles); `buttonClass`; helpers `iso`, `fileName`, `pct`. Imports `clock` from Timeline.tsx.

### Timeline.tsx

- **Purpose**: the project timeline (ordered trimmed clips), per-clip captions, Look & sound export options, export with progress and validation report.
- **Props**: `project: Project`, `version: number` (from App `timelineVersion`).
- **State**: `clips`, `media: Map<id, Media>`, `preset: ExportPreset` (default `"landscape"`), `options: ExportOptions | null`, `job` (latest `media.export` of this project), `error`.
- **Exports**: `clock(ms)` → `m:ss.s` (e.g. `1:05.3`); also used by EditingPlan and MediaLibrary.
- **Sub-components**:
  - `Report({r: ExportResult})` — green/red border; "Export finished and passed validation" or "…but FAILED validation"; output path; `W×H · duration · rendered in S s (encoder)`; checks `✓`/`✗ name: detail`; warnings `! …` amber.
  - `CaptionField({clip, onSaved, onError})` — input (max 200, placeholder "Caption (optional)"); saves with `setClipCaption` on blur only if changed.
  - `MusicLibrary({projectId, version, current, onUse, onClose})` — free library browser: `listSounds`, `recommendSounds(projectId)` (re-run when `version` changes), "Recommended for this video" list with reasons, mood filter chips (`all` + sorted distinct moods), track rows (▶/■ preview via a hidden `<audio>` streaming `t.url`; **Use** / "Chosen"). Preview failure → "Preview needs an internet connection."; audio paused on unmount. Credit text: Kevin MacLeod (incompetech.com), CC BY 4.0, credit added at the end; chosen track downloaded only while exporting and deleted after.
  - `LookAndSound({o, projectId, version, onChange, onError})` — three columns:
    - Picture: Transitions select (0 Hard cuts, 300, 500, 600, 800, 1200 ms; a non-standard current value is appended as an extra option), Style select (`TRANSITIONS`, only when `transitionMs > 0`), Colour select (`COLORS`), checkboxes "Fill the frame (crop centre, no black bars)" (`fill`) and "Fade in from / out to black" (`fade`).
    - Sound: "Clip sound N%" range 0–1.5 step 0.05 (`originalVolume`); music block: library track (`♪ title · free library`) or own file (`♪ name · your file (kept as is)`), links **Library…** / **Own file…** / **Remove**, "Music N%" range, checkbox "Lower the music while people talk" (`duck`); without music: **+ Free music library…** / **+ Your own file…**; checkbox "Even loudness (−16 LUFS, phone / social level)" (`normalize`).
    - Text: Opening title / Subtitle / Ending text inputs (max 80), checkbox "Subtitles from speech (transcribed videos)" (`titles.captions`), hint about per-clip captions.
    - Own file: `open({ multiple: false, filters: Audio AUDIO_EXTENSIONS })` → `music = { path, library: null, volume: prev ?? 0.35, duck: prev ?? true }`. Library pick → `music = { path: "", library: t.id, … }`.
- **Commands**: `listClips`, `listMedia`, `getExportOptions`, `setExportOptions`, `moveClip`, `removeClip`, `setClipCaption`, `listSounds`, `recommendSounds`, `submitJob("media.export")`, `cancelJob`.
- **Events**: `job-updated` filtered to `kind === "media.export" && params.projectId === project.id` (so exports started from Smart edit appear here too).
- **Lifecycle**: `load()` (clips + media map) on mount and when `version` changes; `getExportOptions` on project/version change — if `o.fill` is true and preset is still `landscape`, preset switches to `reels`.
- **Actions**:
  - Export-format `<select>` (`PRESETS`) → sets preset and saves options with `fill = (preset is story or reels)`.
  - **Export** (disabled with no clips, while exporting, or before options load) → `submitJob("media.export", { projectId, preset, options })`.
  - Per clip: caption field, **↑** / **↓** `moveClip(id, ±1)`, **Remove** `removeClip(id)`; each reloads.
  - Every Look & sound change → `change(o)`: optimistic `setOptions` + `setExportOptions` (error clears on success).
  - **Cancel** during export → `cancelJob`.
- **Display**: rows with number, thumbnail, file name, caption, `in → out`, duration. Footer "Total <clock> · rendered from the original files", where total = Σ durations − (n−1) × min(`transitionMs`, half of the shortest clip). `LookAndSound` gets `version + clips.length` so recommendations refresh when clips change.
- **Messages**: "No clips yet. Use Smart edit above, or select a video in the Media library, set In / Out and add it here."; export progress (see below); failed → red `job.error`; cancelled → "Export cancelled. No file was left behind."; completed → `Report`; errors red at the bottom.
- **Constants**: `PRESETS` (landscape "Landscape 16:9 · 1920×1080", story "Story 9:16 · 1080×1920", reels "Reels 9:16 · 1080×1920", archive "Archive · source size and frame rate"); `TRANSITIONS` (fade Cross dissolve, fadeblack Dip to black, dissolve Film dissolve (grain), smoothleft Smooth slide, slideleft Slide, wipeleft Wipe, circleopen Circle open — FFmpeg xfade names); `COLORS` (none Natural, warm, cool, vivid, cinematic teal & orange, vintage, bw Black & white); `AUDIO_EXTENSIONS` = mp3, m4a, aac, wav, flac, ogg, opus; class strings `linkClass`, `field`, `label`; `minutes(ms)` → `m:ss`.

### MediaLibrary.tsx

- **Purpose**: import videos, run background analysis/tagging/transcription, browse/search the library, preview a clip with scenes and transcript, set In/Out and add to the open project, favorite/exclude/remove media.
- **Props**: `project: Project | null`, `onClipAdded()`.
- **State**: `media: Media[] | null`, `importJob`, `job` (latest background job: proxy/analyze/transcribe/tag), `selected: number | null`, `error`, `speechModel` (default `"whisper-small"`), `query`.
- **Sub-components**:
  - `SceneCard({s, onPick})` — scene frame (`convertFileSrc(framePath)`), `clock(start)–clock(end)`, total technical score 0–100 coloured (≥70 green, ≥40 amber, else red), top 3 positive tags (score ≥ 0.1, Vietnamese names, sky), negative tags ≥ 0.15 as `⚠ vi` (amber), `flags` joined. Tooltip lists the five sub-scores.
  - `Preview({m, project, onClipAdded})` — `<video controls autoPlay>` of the proxy (`key={m.id}`), or thumbnail + "Preview is available once the proxy is created."; path line "preview proxy only; exports always use the original file"; scene strip (when `sceneCount !== null`, via `listScenes`); transcript panel (when `transcribed`, via `getTranscript`) with model, language, warning about machine transcription, "No speech found." for zero segments, clickable lines; In/Out bar (only with a proxy): **Set In** / **Set Out** from `video.currentTime`, In/Out/length clocks, **Add to <project>** (disabled without project or when out ≤ in; label "Open a project to add clips") → `addClip` → "Added <len> to <project>." + `onClipAdded()`. Clicking a scene or transcript line sets In/Out to its range and seeks.
  - `MediaRow({m, job, selected, onSelect, onRemoved, onError, onChanged})` — table row (click selects/deselects): thumbnail, file name, recorded time (`sourceCreatedAt` local, "–" if invalid), length `m:ss`, display resolution (swapped when `rotation % 180 !== 0`) + fps, `video / audio` codec ("no audio"), size, Analysis status, Speech status, Concepts (top 3 tags ≥ 0.05; tooltip top 8). Actions cell (clicks do not select): **★** favorite toggle, **⊘**/"excluded" toggle (`setMediaMark` with merged mark), **Remove** → confirm **Remove** (`removeMedia`) / **Keep** (tooltip: removed from library and timelines, file stays on disk).
- **Helpers**: `fold(s)` (NFD, strip diacritics, `đ/Đ→d`, lowercase); `matches(m, q)` (folded path, or positive tag with score ≥ 0.1 matching folded `vi` or `en`); `topTags(tags, n, min)`; `duration`, `resolution`; `working(m, job, kinds, finished)` → `"working N%"` / `"waiting"` / null; `status(m, job)` → working, or `"N scene(s)"`, `"not analysed"` (has proxy), `"–"`; `speech(m, job)` → `"no audio"`, working, `"transcribed"`, `"–"`; `active(job)`.
- **Commands**: `listMedia`, `submitJob` (`media.import`, `media.analyze`, `media.tag`, `media.transcribe`), `cancelJob`, `listScenes`, `getTranscript`, `addClip`, `setMediaMark`, `removeMedia`.
- **Events**: `media.import` → `importJob`; on completion reload and auto-submit `media.analyze` for the imported `ids`. Kinds in `BACKGROUND` → `job`; reload when the job leaves `running` or when `floor(done / STEP)` advances (one file finished).
- **Actions**:
  - **Add videos** (disabled while an import is active) → `open({ multiple: true, title: "Add videos", filters: Videos VIDEO_EXTENSIONS })` → `media.import { paths }`.
  - **Analyze N videos** (when some have `sceneCount === null` and no background job is active) → `media.analyze { ids, quality: "balanced" }` (creates missing proxies first).
  - **Tag N videos** (`tags === null`) → `media.tag { ids }`.
  - Speech model `<select>` (Whisper base (faster) / Whisper small (more accurate)) + **Transcribe N videos** (has audio, not transcribed) → `media.transcribe { ids, model, language: "auto" }`.
  - Search input (shown when the library is non-empty; placeholder "Search by file name or visual concept (xe đạp, beach, hoàng hôn…)").
  - **Cancel** links on the import line and the background-job line.
- **Messages**: privacy note (videos read in place; proxies/frames in the data folder's cache); "Reading metadata… done/total"; `JOB_LABEL[kind]` + percent; failed import/job red; import result "Added or refreshed N video(s)." + per-file failures amber (`name: error`); batch `failed[]` amber `#id: error`; "No videos yet."; "No match.".
- **Constants**: `VIDEO_EXTENSIONS` = mp4, mov, m4v, mkv, avi, mts, m2ts, 3gp, webm, wmv (FFprobe decides what is a video); `STEP = 1000`; `BACKGROUND` = media.proxy, media.analyze, media.transcribe, media.tag; `JOB_LABEL` ("Creating preview proxies…", "Analysing scenes and image quality…", "Transcribing speech (whisper.cpp, on this computer)…", "Recognising visual concepts (OpenCLIP, on this computer)…"); `linkClass`, `buttonClass`.

### AiCenter.tsx

- **Purpose**: AI worker status, device validation, and the log of non-media, non-model jobs.
- **Props**: none.
- **State**: `jobs: Job[]`, `worker: WorkerInfo | null`, `error`.
- **Sub-components**: `DeviceTable({devices})` (Device, Result "works"/error, Precision, Compile ms, Inference ms; row tooltip = device name); `JobRow({job})` (`#id kind`, status + `done/total` while running, progress bar, **Cancel** while queued/running, red error, `DeviceTable` when `result.devices` exists).
- **Commands**: `listJobs`, `getWorkerInfo`, `submitJob("validate_devices")`, `cancelJob`.
- **Events**: every `job-updated` upserts the job into the list and re-reads `getWorkerInfo()`.
- **Actions**: **Validate AI devices** (disabled while a `validate_devices` job is queued/running).
- **Display**: "AI worker running · Python X · OpenVINO Y|unavailable · devices a, b|none" or "AI worker not running (starts automatically when a job needs it)."; `runtimeError` amber. List shows jobs whose kind does not start with `media.` or `models.`, newest first (in practice `validate_devices`).
- **Constants**: `ACTIVE = {queued, running}`.

### Models.tsx

- **Purpose**: "AI models" card — model manifests with install state, download (with confirmation), offline install from files, remove.
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
- **Display**: name · category · size · runtimes; description; state text (underscores → spaces; green `ready`, red failed states, grey otherwise) + install percent while running; `downloadBlocker` text when not installed; `m.error` red; "Load test: DEVICE ok (compile X ms, run Y ms) · …" from `details.devices`. Intro: models are optional, nothing downloads without confirmation.
- **Constants**: `WORKING` = downloading, verifying, installing, validating; `FAILED` = download_failed, checksum_failed, install_failed, model_load_failed, incompatible; `linkClass`, `fileName`.

---

## Job progress in the UI

| Job kind | Shown in | Reading of `done/total` | Cancel |
|---|---|---|---|
| `validate_devices` (and any other non-`media.`/`models.` kind) | AiCenter `JobRow` | `done/total` text + bar `done/total×100%` | **Cancel** per row |
| `media.import` | MediaLibrary "Reading metadata… done/total" | files read / file count | link on that line |
| `media.analyze`, `media.proxy`, `media.tag`, `media.transcribe` | MediaLibrary job line (`JOB_LABEL` + overall %) and per-row Analysis / Speech / Concepts cells | `total = files × 1000`; file at index `i` in `params.ids` is at `(done − i×1000)/1000`; 1–99 % → "working N%", otherwise "waiting" (until its row data changes) | link on the job line |
| `media.export` | Timeline (filtered by `params.projectId`) | `total = 1000`: Rust reports 0–50 music download, 50–900 render, 900–1000 validation; UI shows "Waiting for other jobs…" (queued), "Rendering…" (`done < 900`), "Validating output…" (≥ 900), percent and bar | **Cancel** next to the label |
| `models.install` | Models row state text + percent | `done/total×100` | **Cancel** replaces row buttons |

- Only one job runs at a time (FIFO in Rust), so a queued job just shows "queued"/"waiting".
- `MediaLibrary.job` keeps only the most recent background job; Analyze/Tag/Transcribe buttons are hidden while it is active.
- Jobs list is in memory in Rust; `AiCenter` loads `listJobs()` on mount, others only see events after they mount.
- Cancellation leaves no output file (`.part` + rename in Rust); Timeline says so explicitly.

---

## Styling

- **Tailwind v4** via `@tailwindcss/vite`; [index.css](../../apps/desktop/src/index.css) is only `@import "tailwindcss";` and `:root { color-scheme: light dark; }`. No config file, no custom theme, no `@apply`, no CSS modules.
- **Dark mode**: Tailwind's default `dark:` variant = `prefers-color-scheme` (follows Windows). No toggle. `color-scheme` makes native controls (selects, date inputs, scrollbars) follow too.
- **Palette** (zinc neutrals + status colours):
  - Page `bg-zinc-50 / dark:bg-zinc-950`, text `zinc-900 / zinc-100`; cards `bg-white border-zinc-200 / dark:bg-zinc-900 dark:border-zinc-800`; secondary text `text-zinc-500`.
  - Errors `text-red-600`; warnings and "needs attention" `text-amber-600` (amber-400/500 on the dark preview); success/ready/open `text-green-600`; tag chips in scene cards `text-sky-300`.
  - Preview area is always dark (`bg-black`, zinc-300/400 text) regardless of theme.
- **Shared class strings** (repeated per file, not centralised):
  - Card: `rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900`; card title `mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500`.
  - Primary button: `rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900` (`buttonClass` in MediaLibrary).
  - Secondary button: `rounded-md border border-zinc-300 px-3 py-1.5 text-sm … dark:border-zinc-700` (`buttonClass` in EditingPlan).
  - Link-button: `linkClass = "text-xs text-zinc-500 hover:underline"` (+ disabled variants in Timeline/Models).
  - Inputs/selects: `rounded-md border border-zinc-300 bg-transparent px-… text-sm dark:border-zinc-700` (`inputClass`, `field`).
  - Thumbnails `h-9 w-16 rounded object-cover` with a grey placeholder div; progress bar `h-1.5 rounded bg-zinc-200` + inner `bg-zinc-900 dark:bg-zinc-100 transition-all`.
  - Lists `divide-y divide-zinc-200 dark:divide-zinc-800`; times/paths `font-mono`.
- **Icons**: Unicode only — `★` favorite, `⊘` exclude, `↑ ↓` reorder, `▶ ■` preview, `♪` music, `⚠` negative tag, `✓ ✗ !` validation, `·` separators, `…` ellipsis.
- **Responsive**: grid `lg:grid-cols-2`; Look & sound `md:grid-cols-3`; library table in `overflow-x-auto`.
- No component library; native `<select>`, `<input type=date|range|checkbox>`, `<video>`, `<audio>`, `<table>`. Accessibility: `aria-label` on unlabeled inputs and arrow buttons, `aria-pressed` on toggles, `title` tooltips for details.

---

## Conventions and how to add UI

- **Add a backend call**: add the Rust command to `generate_handler!` (lib.rs), then one wrapper in [api.ts](../../apps/desktop/src/api.ts): `export const fooBar = (projectId: number) => invoke<T>("foo_bar", { projectId });` with camelCase arg keys and a TS interface mirroring the serde (`rename_all = "camelCase"`) struct; reference the Rust file in a `/** */` comment. Components never import `invoke` directly.
- **Add a job kind**: add it to `UI_JOB_KINDS` in lib.rs, submit with `submitJob(kind, params)`, and filter `onJobUpdated` by `j.kind` (and by a param such as `projectId` when several instances exist). For list jobs follow the 1000-units-per-file convention so `working()` keeps working. Decide whether AiCenter should show it (it hides `media.*` and `models.*`).
- **Add a card**: new `src/<Name>.tsx` with a default-exported component using the card class string; mount it in `App.tsx` inside `<div className="lg:col-span-2">`, gated on `settings?.dataRoot && !settings.databaseError` if it touches the DB, keyed on `settings.dataRoot` (plus `project.id` if per project). Share refreshes through App state (like `timelineVersion`), not a store.
- **Errors**: every call ends in `.catch((e) => setError(String(e)))` (or `setMessage`); show `{error && <p className="mt-2 text-sm text-red-600">{error}</p>}` in the card. Amber for non-fatal warnings, grey `text-zinc-500` for info/empty states. Destructive actions use an inline confirm (Delete/Remove → confirm + Keep), never `window.confirm`.
- **Files**: pickers via `@tauri-apps/plugin-dialog` `open()` with explicit extension filters; local images/videos only via `convertFileSrc` on paths under `<data root>\cache` (asset scope + CSP block anything else).
- **Rules**: the frontend never runs FFmpeg or loads models (CLAUDE.md); exports always render from originals — UI copy says so.
- **Checks**: `npm run build` (tsc strict) in `apps/desktop`; there are no frontend unit tests. UI screenshots: capture only the app window (PrintWindow).
- **Docs to update** after a UI change: this file; [FLOWS.md](FLOWS.md) if a user flow changes; [BACKEND.md](BACKEND.md) for new commands/jobs; [../APP_OVERVIEW.md](../APP_OVERVIEW.md) for user-visible features; [../PROGRESS.md](../PROGRESS.md) day log. Run the `sync-docs` skill.
