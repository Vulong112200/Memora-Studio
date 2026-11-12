// Typed wrappers for Rust commands (src-tauri/src/lib.rs). Field names match serde camelCase.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type JobStatus = "queued" | "running" | "completed" | "failed" | "cancelled";

export interface Job {
  id: number;
  kind: string;
  params: unknown;
  status: JobStatus;
  done: number;
  total: number;
  result: unknown;
  error: string | null;
}

/** `hello` message of the running AI worker (workers/ai-worker/memora_worker.py). */
export interface WorkerInfo {
  protocol: number;
  python: string;
  openvino: string | null;
  devices: string[];
  startupMs: number;
  runtimeError?: string;
}

export interface DeviceValidation {
  device: string;
  name: string;
  ok: boolean;
  compileMs?: number;
  inferMs?: number;
  precision?: string;
  relError?: number;
  error?: string;
}

export const submitJob = (kind: string, params?: unknown) => invoke<Job>("submit_job", { kind, params });
export const listJobs = () => invoke<Job[]>("list_jobs");
export const cancelJob = (id: number) => invoke<void>("cancel_job", { id });
export const getWorkerInfo = () => invoke<WorkerInfo | null>("get_worker_info");
export const onJobUpdated = (f: (job: Job) => void): Promise<UnlistenFn> =>
  listen<Job>("job-updated", (e) => f(e.payload));

export interface SettingsView {
  dataRoot: string | null;
  defaultDataRoot: string;
  logDir: string | null;
  restartRequired: boolean;
  databaseError: string | null;
}

export interface Project {
  id: number;
  name: string;
  createdAt: string;
  updatedAt: string;
}

export interface HardwareInfo {
  os: { name: string; edition: string; displayVersion: string; build: string };
  cpu: { name: string; logicalProcessors: number };
  memory: {
    totalPhysical: number;
    availablePhysical: number;
    commitLimit: number;
    commitAvailable: number;
    loadPercent: number;
  };
  gpus: {
    name: string;
    vendorId: number;
    dedicatedVideoMemory: number;
    sharedSystemMemory: number;
    driverVersion: string | null;
  }[];
  dataDisk: { path: string; total: number; free: number } | null;
}

/** Library entry (src-tauri/src/media.rs). `rotation` is counter-clockwise degrees; width/height are coded size. */
export interface Media {
  id: number;
  path: string;
  size: number;
  durationMs: number;
  width: number;
  height: number;
  rotation: number;
  fps: number | null;
  videoCodec: string;
  audioCodec: string | null;
  audioStreams: number;
  bitrate: number | null;
  sourceCreatedAt: string | null;
  importedAt: string;
  /** Absolute paths of cached files; load with convertFileSrc. */
  thumb: string | null;
  proxy: string | null;
  /** Detected scenes; null until analysed. */
  sceneCount: number | null;
  /** True once speech was transcribed (possibly with no speech found). */
  transcribed: boolean;
  /** Visual concepts over the clip (OpenCLIP, best first); null until tagged. */
  tags: TagScore[] | null;
  /** User override used by the Editing Core. */
  mark: Mark;
}

export interface Mark { favorite: boolean; excluded: boolean }
export const setMediaMark = (mediaId: number, mark: Mark) => invoke<void>("set_media_mark", { mediaId, mark });

/** Editing style (src-tauri/src/editing.rs, docs/Production.md §6.3). */
export interface EditingStyle {
  id: string;
  name: string;
  targetMs: number;
  exportPreset: ExportPreset;
  clipMinMs: number;
  clipMaxMs: number;
  clipIdealMs: number;
  transitionMs: number;
  music: string;
  titles: string;
  pacing: string;
  opening: string;
  ending: string;
}

export interface PlanClip {
  mediaId: number;
  sceneIndex: number;
  inMs: number;
  outMs: number;
  scores: { technical: number; semantic: number; speech: number; favorite: number };
  rank: number;
  tags: string[];
  reasons: string[];
  /** A still photo (`mediaId` is then −photo id). */
  photoId?: number | null;
}

/** The user's requests on top of a style (editing.rs `Wishes`); concept names are tag `en` names. */
export interface Wishes {
  prefer: string[];
  less: string[];
  avoid: string[];
  targetMs: number | null;
  preset: ExportPreset | null;
  color: string | null;
  title: string | null;
  ending: string | null;
  captions: boolean | null;
  music: string | null;
  /** "none" | "fewer" | "yes" | "more"; null = photos used when there are some. */
  photos?: string | null;
  photoMs?: number | null;
  request: string;
}

/** What the request box read from a sentence (request.rs, offline keywords). */
export interface RequestReading {
  style: string | null;
  from: string | null;
  to: string | null;
  wishes: Wishes;
  understood: string[];
}

export interface Plan {
  style: string;
  wishes: Wishes;
  targetMs: number;
  totalMs: number;
  transitionMs: number;
  exportPreset: ExportPreset;
  clips: PlanClip[];
  skipped: string[];
  candidates: number;
  suggestedTitle: string;
  /** How the plan was built (length chosen from the footage, coverage clips). */
  notes?: string[];
}

export const listStyles = () => invoke<EditingStyle[]>("list_styles");
export const generatePlan = (projectId: number, style: string, from?: string, to?: string, request?: string) =>
  invoke<Plan>("generate_plan", { projectId, style, from: from ?? null, to: to ?? null, request: request ?? null });
export const interpretRequest = (text: string) => invoke<RequestReading>("interpret_request", { text });
export const getPlan = (projectId: number) => invoke<Plan | null>("get_plan", { projectId });
/** Replaces the timeline with the kept clips; returns the export options with the style's look applied. */
export const applyPlan = (projectId: number, keep: number[]) => invoke<ExportOptions>("apply_plan", { projectId, keep });

/** Look and sound of an export (src-tauri/src/export.rs `Options`). */
export interface ExportOptions {
  fill: boolean;
  transitionMs: number;
  transition: string;
  color: string;
  fade: boolean;
  originalVolume: number;
  /** `library` = free library track id (downloaded only while exporting); else `path` = the user's own file. */
  music: { path: string; library: string | null; volume: number; duck: boolean } | null;
  titles: { title: string; subtitle: string; ending: string; captions: boolean };
  normalize: boolean;
  /** Lighter file for sharing (video ≈ 10 Mbit/s); file name ends in `-share`. */
  share?: boolean;
}

export const getExportOptions = (projectId: number) => invoke<ExportOptions>("get_export_options", { projectId });
export const setExportOptions = (projectId: number, options: ExportOptions) => invoke<void>("set_export_options", { projectId, options });
/** Free music library track (src-tauri/sounds.json). */
export interface Track {
  id: string;
  title: string;
  artist: string;
  moods: string[];
  energy: "low" | "medium" | "high";
  durationMs: number;
  url: string;
  license: string;
  credit: string;
}
export interface SoundRecommendation { id: string; score: number; reasons: string[] }
export const listSounds = () => invoke<Track[]>("list_sounds");
/** `projectId` null = a photo video of `videoMs` (no tags or speech to match). */
export const recommendSounds = (projectId: number | null, videoMs?: number) =>
  invoke<SoundRecommendation[]>("recommend_sounds", { projectId, videoMs: videoMs ?? null });

export const setClipCaption = (id: number, caption: string) => invoke<void>("set_clip_caption", { id, caption });

/** Mean OpenCLIP probability of a tag over a time range (src-tauri/src/tagging.rs). */
export interface TagScore { en: string; vi: string; score: number; /** bad-shot detector, not content */ negative: boolean }

/** whisper.cpp transcript (src-tauri/src/transcribe.rs). Times are in the original file. */
export interface Transcript {
  mediaId: number;
  modelId: string;
  language: string | null;
  segments: { startMs: number; endMs: number; text: string }[];
  createdAt: string;
}

export const getTranscript = (mediaId: number) => invoke<Transcript | null>("get_transcript", { mediaId });

/** Trimmed segment on a project timeline (src-tauri/src/timeline.rs). */
/** Timeline clip: a video segment (`mediaId`) or a still photo (`photoId`, shown `outMs - inMs`). */
export interface Clip {
  id: number;
  projectId: number;
  mediaId: number | null;
  photoId: number | null;
  position: number;
  inMs: number;
  outMs: number;
  caption: string | null;
  /** Transition into this clip; null = the project's transition. */
  transition: string | null;
  transitionMs: number | null;
}
/** One join as exported: [xfade name, ms]. */
export type Join = [string, number];

export type ExportPreset = "landscape" | "story" | "reels" | "archive";

export interface ExportResult {
  path: string;
  preset: ExportPreset;
  encoder: string;
  renderSeconds: number;
  durationMs: number;
  width: number;
  height: number;
  validation: { ok: boolean; checks: { name: string; ok: boolean; detail: string }[]; warnings: string[] };
}

export const listClips = (projectId: number) => invoke<Clip[]>("list_clips", { projectId });
export const addClip = (projectId: number, mediaId: number, inMs: number, outMs: number) =>
  invoke<Clip>("add_clip", { projectId, mediaId, inMs, outMs });
export const removeClip = (id: number) => invoke<void>("remove_clip", { id });
export const moveClip = (id: number, delta: number) => invoke<void>("move_clip", { id, delta });
export const addPhotoClips = (projectId: number, photoIds: number[], ms?: number) =>
  invoke<Clip[]>("add_photo_clips", { projectId, photoIds, ms: ms ?? null });
export const setClipRange = (id: number, inMs: number, outMs: number) => invoke<Clip>("set_clip_range", { id, inMs, outMs });
export const setClipTransition = (id: number, transition: string | null, transitionMs: number) =>
  invoke<Clip>("set_clip_transition", { id, transition, transitionMs });
export const listJoins = (projectId: number) => invoke<Join[]>("list_joins", { projectId });
/** Low-resolution render of a few seconds around join `index` (cache file). */
export const previewJoin = (projectId: number, index: number, preset: ExportPreset) => invoke<string>("preview_join", { projectId, index, preset });

/** Model manifest + install state (src-tauri/src/models.rs, models.json). */
export interface ModelInfo {
  id: string;
  name: string;
  version: string;
  category: string;
  description: string;
  license: string;
  minimumRamGb: number;
  artifacts: { file: string; url: string | null; sha256: string | null; sizeBytes: number | null }[];
  runtimes: { runtime: string; devices: string[]; validated: boolean }[];
  notes: string;
  state: string;
  error: string | null;
  path: string | null;
  details: { devices?: { device: string; ok: boolean; compileMs?: number; inferMs?: number; error?: string }[] } | null;
  downloadBlocker: string | null;
}

export const listModels = () => invoke<ModelInfo[]>("list_models");
export const removeModel = (id: string) => invoke<void>("remove_model", { id });

/** Scene with observations and deterministic technical score (src-tauri/src/scoring.rs). */
export interface ScoredScene {
  id: number;
  mediaId: number;
  index: number;
  startMs: number;
  endMs: number;
  shake: number;
  framePath: string | null;
  samples: { atMs: number; sharpness: number; brightness: number; contrast: number; clipLow: number; clipHigh: number; black: boolean; dhash: string }[];
  technical: { sharpness: number; exposure: number; brightness: number; contrast: number; stability: number; total: number };
  flags: string[];
  duplicates: { mediaId: number; sceneIndex: number; distance: number }[];
  tags: TagScore[];
}

export const listScenes = (mediaId: number) => invoke<ScoredScene[]>("list_scenes", { mediaId });
export const listMedia = () => invoke<Media[]>("list_media");
export const removeMedia = (id: number) => invoke<void>("remove_media", { id });

export const getSettings =() => invoke<SettingsView>("get_settings");
export const setDataRoot = (path: string) => invoke<SettingsView>("set_data_root", { path });
/** Verified copy of the database in `<data root>\database\backups\` (also made automatically at every start). */
export const backupDatabase = () => invoke<string>("backup_database");
export const getHardwareInfo = () => invoke<HardwareInfo>("get_hardware_info");
export const listProjects = () => invoke<Project[]>("list_projects");
export const createProject = (name: string) => invoke<Project>("create_project", { name });
export const renameProject = (id: number, name: string) => invoke<Project>("rename_project", { id, name });
export const deleteProject = (id: number) => invoke<void>("delete_project", { id });

/** Photo editor (src-tauri/src/photos.rs, workers/ai-worker/photo.py). Edit values are -100..100 (0 = unchanged). */
export interface PhotoEdits {
  [key: string]: number | string | Record<string, number[]> | { points: number[][]; r: number }[] | { points: number[][]; labels?: number[] }[] | undefined;
  /** Objects removed by clicking on them (EfficientSAM + MI-GAN): click points 0..1 of the shown frame. */
  remove?: { points: number[][]; labels?: number[] }[];
  /** Painted areas to remove: points 0..1 of the shown frame, r = brush radius 0..1 of its long side. */
  erase?: { points: number[][]; r: number }[];
  look?: string;
  crop?: string;
  rotate?: number;
  hsl?: Record<string, number[]>;
}
export interface Photo {
  id: number;
  path: string;
  size: number;
  width: number | null;
  height: number | null;
  takenAt: string | null;
  edits: PhotoEdits | null;
  importedAt: string;
  thumb: string | null;
  /** Observations stored (job `media.photo_analyze`): Smart edit can use it. */
  analyzed: boolean;
  /** The file is no longer at `path` (moved or renamed): `relinkPhotos` finds it again. */
  missing: boolean;
}
export interface PhotoImport { ids: number[]; failed: { path: string; error: string }[] }
export const listPhotos = () => invoke<Photo[]>("list_photos");
/** `auto`: give new photos the suggested edits (worker `photo_auto`) right away. */
export const importPhotos = (paths: string[], auto = false) => invoke<PhotoImport>("import_photos", { paths, auto });
/** Suggested edits for the photos that have none yet; returns how many were edited. */
export const autoPhotos = (ids: number[]) => invoke<number>("auto_photos", { ids });
/** Photos moved to `folder` (same name and size) are found again; returns how many. */
export const relinkPhotos = (folder: string) => invoke<number>("relink_photos", { folder });
/** What the photo tools can do here: `face` = face detector installed (retouch works). */
export interface PhotoTools { face: boolean; mesh: boolean; select: boolean; inpaint: boolean; bokeh: boolean; matte: boolean }
export const photoTools = () => invoke<PhotoTools>("photo_tools");
export const setPhotoEdits = (id: number, edits: PhotoEdits | null) => invoke<void>("set_photo_edits", { id, edits });
/** Returns the cache file of a ≤ 1600 px preview (`before` = without edits). */
export const previewPhoto = (id: number, edits: PhotoEdits, showMask: boolean, before: boolean) =>
  invoke<string>("preview_photo", { id, edits, showMask, before });
export const autoPhoto = (id: number) => invoke<PhotoEdits>("auto_photo", { id });
export const exportPhoto = (id: number) => invoke<string>("export_photo", { id });
export const removePhoto = (id: number) => invoke<void>("remove_photo", { id });

/** Bytes → "18.3 GB" (binary units, as Windows displays them). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024 ** 3) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}
