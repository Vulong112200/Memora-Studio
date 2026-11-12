// Shared UI pieces (they were duplicated in every card) and the background-job helpers.
import { useEffect, useState, type ReactNode } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { cancelJob, listJobs, onJobUpdated, submitJob, type Job, type Media, type ModelInfo } from "./api";
import { L } from "./i18n";

export const btn = {
  primary:
    "rounded-lg bg-indigo-600 px-4 py-1.5 text-sm font-medium text-white shadow-sm hover:bg-indigo-500 disabled:cursor-not-allowed disabled:opacity-50",
  secondary:
    "rounded-lg border border-zinc-300 bg-white px-3 py-1.5 text-sm hover:bg-zinc-50 disabled:cursor-not-allowed disabled:opacity-40 dark:border-zinc-700 dark:bg-zinc-900 dark:hover:bg-zinc-800",
  link: "text-xs text-zinc-500 hover:text-zinc-900 hover:underline disabled:cursor-not-allowed disabled:opacity-30 dark:hover:text-zinc-100",
};
export const field = "rounded-lg border border-zinc-300 bg-transparent px-2 py-1.5 text-sm dark:border-zinc-700";

export function Card({ title, actions, children }: { title: ReactNode; actions?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm dark:border-zinc-800 dark:bg-zinc-900">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-zinc-500">{title}</h2>
        {actions && <div className="flex flex-wrap items-center justify-end gap-2">{actions}</div>}
      </div>
      {children}
    </section>
  );
}

export const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

export function Thumb({ path, className = "h-9 w-16" }: { path: string | null | undefined; className?: string }) {
  return path
    ? <img className={`${className} rounded object-cover`} src={convertFileSrc(path)} alt="" />
    : <div className={`${className} rounded bg-zinc-200 dark:bg-zinc-800`} />;
}

export function Progress({ value }: { value: number }) {
  return (
    <div className="h-1.5 overflow-hidden rounded bg-zinc-200 dark:bg-zinc-800">
      <div className="h-full bg-indigo-600 transition-all" style={{ width: `${Math.max(0, Math.min(1, value)) * 100}%` }} />
    </div>
  );
}

export const active = (j: Job | null | undefined) => !!j && (j.status === "queued" || j.status === "running");

export function jobLabel(kind: string) {
  const labels: Record<string, [string, string]> = {
    "media.import": ["Reading video metadata", "Đọc thông tin video"],
    "media.proxy": ["Creating preview copies", "Tạo bản xem trước"],
    "media.analyze": ["Analysing scenes and image quality", "Phân tích cảnh và chất lượng hình"],
    "media.tag": ["Recognising what is on screen (AI)", "Nhận diện nội dung hình (AI)"],
    "media.transcribe": ["Transcribing speech (AI)", "Nhận dạng lời nói (AI)"],
    "media.export": ["Exporting video", "Xuất video"],
    "media.photo_video": ["Making a video from photos", "Tạo video từ ảnh"],
    "media.photo_analyze": ["Analysing photos (quality, content)", "Phân tích ảnh (chất lượng, nội dung)"],
    "models.install": ["Installing an AI model", "Cài model AI"],
    validate_devices: ["Checking AI devices", "Kiểm tra thiết bị AI"],
  };
  const l = labels[kind];
  return l ? L(l[0], l[1]) : kind;
}

/** All jobs of this session, kept current from `job-updated` events. */
export function useJobs(): Job[] {
  const [jobs, setJobs] = useState<Job[]>([]);
  useEffect(() => {
    listJobs().then(setJobs).catch(() => {});
    const unlisten = onJobUpdated((job) =>
      setJobs((prev) => (prev.some((j) => j.id === job.id) ? prev.map((j) => (j.id === job.id ? job : j)) : [...prev, job])));
    return () => { unlisten.then((f) => f()); };
  }, []);
  return jobs;
}

/** Running and waiting jobs, always visible at the bottom of the window. */
export function JobBar({ onOpen }: { onOpen: () => void }) {
  const jobs = useJobs().filter(active);
  if (jobs.length === 0) return null;
  const [j, ...rest] = jobs;
  return (
    <div className="flex items-center gap-3 border-t border-zinc-200 bg-white px-4 py-2 text-sm dark:border-zinc-800 dark:bg-zinc-900">
      <span className="h-2 w-2 animate-pulse rounded-full bg-indigo-600" />
      <span className="font-medium">{jobLabel(j.kind)}</span>
      <span className="text-zinc-500">{j.status === "queued" ? L("waiting", "đang chờ") : j.total > 0 ? `${Math.round((j.done / j.total) * 100)}%` : ""}</span>
      <div className="w-40"><Progress value={j.total ? j.done / j.total : 0} /></div>
      {rest.length > 0 && <button className={btn.link} onClick={onOpen}>{L(`+${rest.length} waiting`, `+${rest.length} việc đang chờ`)}</button>}
      <button className={`${btn.link} ml-auto`} onClick={() => cancelJob(j.id).catch(() => {})}>{L("Cancel", "Huỷ")}</button>
    </div>
  );
}

const ready = (models: ModelInfo[], id: string) => models.some((m) => m.id === id && m.state === "ready");

/** What is still missing before Smart edit can choose well. */
export function preparation(media: Media[], models: ModelInfo[]) {
  const tagger = ready(models, "openclip-vit-b32");
  const speech = ready(models, "whisper-small") ? "whisper-small" : ready(models, "whisper-base") ? "whisper-base" : null;
  return {
    analyse: media.filter((m) => m.sceneCount === null).map((m) => m.id),
    tag: tagger ? media.filter((m) => m.tags === null).map((m) => m.id) : [],
    transcribe: speech ? media.filter((m) => m.audioStreams > 0 && !m.transcribed).map((m) => m.id) : [],
    speech,
    missingModels: !tagger || !speech,
  };
}

/** One click for everything Smart edit uses: scenes + quality, then visual concepts, then speech. The job queue runs
 *  one job at a time in order, so tagging starts after the proxies exist. Steps without an installed model are skipped. */
export async function prepareMedia(media: Media[], models: ModelInfo[]) {
  const p = preparation(media, models);
  if (p.analyse.length) await submitJob("media.analyze", { ids: p.analyse, quality: "balanced" });
  if (p.tag.length) await submitJob("media.tag", { ids: p.tag });
  if (p.transcribe.length) await submitJob("media.transcribe", { ids: p.transcribe, model: p.speech, language: "auto" });
  return p.analyse.length + p.tag.length + p.transcribe.length;
}
