import { useCallback, useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  cancelJob, listClips, listMedia, moveClip, onJobUpdated, removeClip, submitJob,
  type Clip, type ExportPreset, type ExportResult, type Job, type Media, type Project,
} from "./api";

const PRESETS: { id: ExportPreset; label: string }[] = [
  { id: "landscape", label: "Landscape 1920×1080 · 30 fps" },
  { id: "story", label: "Story 1080×1920 · 30 fps" },
  { id: "reels", label: "Reels 1080×1920 · 30 fps" },
  { id: "archive", label: "Archive · source size and frame rate" },
];
const linkClass = "text-xs text-zinc-500 hover:underline disabled:opacity-30";

export const clock = (ms: number) => {
  const t = ms / 1000;
  return `${Math.floor(t / 60)}:${(t % 60).toFixed(1).padStart(4, "0")}`;
};
const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

function Report({ r }: { r: ExportResult }) {
  const v = r.validation;
  return (
    <div className={`mt-3 rounded-lg border p-3 text-sm ${v.ok ? "border-green-600/40" : "border-red-600/60"}`}>
      <p className={`font-medium ${v.ok ? "text-green-600" : "text-red-600"}`}>
        {v.ok ? "Export finished and passed validation" : "Export finished but FAILED validation"}
      </p>
      <p className="mt-1 break-all font-mono text-xs">{r.path}</p>
      <p className="mt-1 text-xs text-zinc-500">
        {r.width}×{r.height} · {clock(r.durationMs)} · rendered in {r.renderSeconds} s ({r.encoder})
      </p>
      <ul className="mt-2 text-xs">
        {v.checks.map((c) => (
          <li key={c.name} className={c.ok ? "text-zinc-500" : "text-red-600"}>
            {c.ok ? "✓" : "✗"} {c.name}: {c.detail}
          </li>
        ))}
        {v.warnings.map((w) => <li key={w} className="text-amber-600">! {w}</li>)}
      </ul>
    </div>
  );
}

export default function Timeline({ project, version }: { project: Project; version: number }) {
  const [clips, setClips] = useState<Clip[]>([]);
  const [media, setMedia] = useState<Map<number, Media>>(new Map());
  const [preset, setPreset] = useState<ExportPreset>("landscape");
  const [transitionMs, setTransitionMs] = useState(0);
  const [fill, setFill] = useState(false);
  const [job, setJob] = useState<Job | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    listClips(project.id).then(setClips).catch((e) => setError(String(e)));
    listMedia().then((m) => setMedia(new Map(m.map((x) => [x.id, x])))).catch((e) => setError(String(e)));
  }, [project.id]);
  useEffect(load, [load, version]);

  useEffect(() => {
    const unlisten = onJobUpdated((j) => {
      if (j.kind === "media.export" && (j.params as { projectId?: number }).projectId === project.id) setJob(j);
    });
    return () => { unlisten.then((f) => f()); };
  }, [project.id]);

  const run = (p: Promise<unknown>) => p.then(load).catch((e) => setError(String(e)));
  const exporting = job && (job.status === "queued" || job.status === "running");
  const total = clips.reduce((s, c) => s + c.outMs - c.inMs, 0) - Math.max(0, clips.length - 1) * transitionMs;
  const result = job?.status === "completed" ? (job.result as ExportResult) : null;

  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-zinc-500">Timeline · {project.name}</h2>
        <div className="flex items-center gap-2">
          <select
            className="rounded-md border border-zinc-300 bg-transparent px-2 py-1.5 text-sm dark:border-zinc-700"
            value={preset}
            onChange={(e) => { const p = e.target.value as ExportPreset; setPreset(p); setFill(p === "story" || p === "reels"); }}
            aria-label="Export preset"
          >
            {PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
          </select>
          <select className="rounded-md border border-zinc-300 bg-transparent px-2 py-1.5 text-sm dark:border-zinc-700" value={transitionMs}
            onChange={(e) => setTransitionMs(Number(e.target.value))} aria-label="Transitions">
            <option value={0}>Hard cuts</option>
            <option value={500}>Crossfade 0.5 s</option>
            <option value={1000}>Crossfade 1 s</option>
          </select>
          <label className="flex items-center gap-1 text-xs text-zinc-500" title="Crop the centre to fill the frame instead of adding black bars">
            <input type="checkbox" checked={fill} onChange={(e) => setFill(e.target.checked)} /> Fill frame
          </label>
          <button
            className="rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900"
            disabled={clips.length === 0 || !!exporting}
            onClick={() => submitJob("media.export", { projectId: project.id, preset, transitionMs, fit: fill ? "fill" : "fit" }).then(setJob).catch((e) => setError(String(e)))}
          >
            Export
          </button>
        </div>
      </div>
      {clips.length === 0 ? (
        <p className="text-sm text-zinc-500">No clips yet. Select a video in the Media library, set In / Out and add it here.</p>
      ) : (
        <>
          <ol className="divide-y divide-zinc-200 dark:divide-zinc-800">
            {clips.map((c, i) => {
              const m = media.get(c.mediaId);
              return (
                <li key={c.id} className="flex items-center gap-3 py-1.5 text-sm">
                  <span className="w-5 text-right text-xs text-zinc-500">{i + 1}</span>
                  {m?.thumb ? <img className="h-9 w-16 rounded object-cover" src={convertFileSrc(m.thumb)} alt="" /> : <div className="h-9 w-16 rounded bg-zinc-200 dark:bg-zinc-800" />}
                  <span className="flex-1 truncate" title={m?.path}>{m ? fileName(m.path) : `media #${c.mediaId}`}</span>
                  <span className="font-mono text-xs text-zinc-500">{clock(c.inMs)} → {clock(c.outMs)}</span>
                  <span className="w-14 text-right font-mono text-xs">{clock(c.outMs - c.inMs)}</span>
                  <button className={linkClass} disabled={i === 0} onClick={() => run(moveClip(c.id, -1))} aria-label="Move earlier">↑</button>
                  <button className={linkClass} disabled={i === clips.length - 1} onClick={() => run(moveClip(c.id, 1))} aria-label="Move later">↓</button>
                  <button className={linkClass} onClick={() => run(removeClip(c.id))}>Remove</button>
                </li>
              );
            })}
          </ol>
          <p className="mt-2 text-right text-xs text-zinc-500">Total {clock(total)} · rendered from the original files</p>
        </>
      )}
      {exporting && (
        <div className="mt-3 text-sm text-zinc-500">
          {job!.status === "queued" ? "Waiting for other jobs…" : job!.done < 900 ? "Rendering…" : "Validating output…"}{" "}
          {job!.total > 0 && `${Math.round((job!.done / job!.total) * 100)}%`}{" "}
          <button className="text-xs hover:underline" onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>Cancel</button>
          <div className="mt-1 h-1.5 overflow-hidden rounded bg-zinc-200 dark:bg-zinc-800">
            <div className="h-full bg-zinc-900 transition-all dark:bg-zinc-100" style={{ width: `${job!.total ? (job!.done / job!.total) * 100 : 0}%` }} />
          </div>
        </div>
      )}
      {job?.status === "failed" && <p className="mt-2 text-sm text-red-600">{job.error}</p>}
      {job?.status === "cancelled" && <p className="mt-2 text-sm text-zinc-500">Export cancelled. No file was left behind.</p>}
      {result && <Report r={result} />}
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
    </section>
  );
}
