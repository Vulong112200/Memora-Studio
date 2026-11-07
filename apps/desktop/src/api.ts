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
}

/** Trimmed segment on a project timeline (src-tauri/src/timeline.rs). */
export interface Clip {
  id: number;
  projectId: number;
  mediaId: number;
  position: number;
  inMs: number;
  outMs: number;
}

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
}

export const listScenes = (mediaId: number) => invoke<ScoredScene[]>("list_scenes", { mediaId });
export const listMedia = () => invoke<Media[]>("list_media");
export const removeMedia = (id: number) => invoke<void>("remove_media", { id });

export const getSettings =() => invoke<SettingsView>("get_settings");
export const setDataRoot = (path: string) => invoke<SettingsView>("set_data_root", { path });
export const getHardwareInfo = () => invoke<HardwareInfo>("get_hardware_info");
export const listProjects = () => invoke<Project[]>("list_projects");
export const createProject = (name: string) => invoke<Project>("create_project", { name });
export const renameProject = (id: number, name: string) => invoke<Project>("rename_project", { id, name });
export const deleteProject = (id: number) => invoke<void>("delete_project", { id });

/** Bytes → "18.3 GB" (binary units, as Windows displays them). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024 ** 3) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}
