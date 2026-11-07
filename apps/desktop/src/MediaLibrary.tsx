import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addClip, cancelJob, formatBytes, listMedia, listScenes, onJobUpdated, removeMedia, submitJob,
  type Job, type Media, type Project, type ScoredScene,
} from "./api";
import { clock } from "./Timeline";

/** Offered in the file picker; FFprobe decides what is actually a video. */
const VIDEO_EXTENSIONS = ["mp4", "mov", "m4v", "mkv", "avi", "mts", "m2ts", "3gp", "webm", "wmv"];
const linkClass = "text-xs text-zinc-500 hover:underline";
const buttonClass =
  "rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900";
/** Progress units per file in media.proxy / media.analyze jobs (src-tauri). */
const STEP = 1000;
/** Background jobs that work through a list of media ids. */
const BACKGROUND = new Set(["media.proxy", "media.analyze"]);

interface ImportResult { imported: number; ids: number[]; failed: { path: string; error: string }[] }
interface BatchResult { failed: { id: number; error: string }[] }

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;
const active = (j: Job | null) => !!j && (j.status === "queued" || j.status === "running");

function duration(ms: number) {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Size as the video is displayed (phone portrait clips are stored landscape + rotation). */
function resolution(m: Media) {
  return m.rotation % 180 === 0 ? `${m.width}×${m.height}` : `${m.height}×${m.width}`;
}

/** Analysis state of one clip while `job` may be working on it. */
function status(m: Media, job: Job | null) {
  const ids = (job?.params as { ids?: number[] } | null)?.ids ?? [];
  const i = ids.indexOf(m.id);
  if (active(job) && i >= 0 && m.sceneCount === null) {
    const pct = Math.round(((job!.done - i * STEP) / STEP) * 100);
    return pct > 0 && pct < 100 ? `working ${pct}%` : "waiting";
  }
  if (m.sceneCount !== null) return `${m.sceneCount} scene${m.sceneCount === 1 ? "" : "s"}`;
  return m.proxy ? "not analysed" : "–";
}

const pct = (x: number) => `${Math.round(x * 100)}`;

function SceneCard({ s, onPick }: { s: ScoredScene; onPick: () => void }) {
  const t = s.technical;
  const tone = t.total >= 0.7 ? "text-green-500" : t.total >= 0.4 ? "text-amber-500" : "text-red-500";
  return (
    <button className="w-40 shrink-0 rounded bg-zinc-900 p-1 text-left text-xs text-zinc-300 hover:bg-zinc-800" onClick={onPick}
      title={`Sharpness ${pct(t.sharpness)} · Exposure ${pct(t.exposure)} · Brightness ${pct(t.brightness)} · Contrast ${pct(t.contrast)} · Stability ${pct(t.stability)}`}>
      {s.framePath ? <img className="h-20 w-full rounded object-cover" src={convertFileSrc(s.framePath)} alt="" /> : <div className="h-20 rounded bg-zinc-800" />}
      <div className="mt-1 flex justify-between">
        <span className="font-mono">{clock(s.startMs)}–{clock(s.endMs)}</span>
        <span className={`font-semibold ${tone}`}>{pct(t.total)}</span>
      </div>
      {s.flags.length > 0 && <div className="mt-0.5 truncate text-amber-400">{s.flags.join(" · ")}</div>}
    </button>
  );
}

function Preview({ m, project, onClipAdded }: { m: Media; project: Project | null; onClipAdded: () => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const [inMs, setIn] = useState(0);
  const [outMs, setOut] = useState(m.durationMs);
  const [scenes, setScenes] = useState<ScoredScene[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const now = () => Math.round((video.current?.currentTime ?? 0) * 1000);

  useEffect(() => {
    if (m.sceneCount !== null) listScenes(m.id).then(setScenes).catch((e) => setMessage(String(e)));
  }, [m.id, m.sceneCount]);

  function pick(s: ScoredScene) {
    setIn(s.startMs);
    setOut(s.endMs);
    if (video.current) video.current.currentTime = s.startMs / 1000;
  }

  async function add() {
    if (!project) return;
    try {
      await addClip(project.id, m.id, inMs, outMs);
      setMessage(`Added ${clock(outMs - inMs)} to ${project.name}.`);
      onClipAdded();
    } catch (e) {
      setMessage(String(e));
    }
  }

  return (
    <div className="mt-3 rounded-lg bg-black">
      {m.proxy ? (
        <video ref={video} key={m.id} className="mx-auto max-h-96" src={convertFileSrc(m.proxy)} controls autoPlay />
      ) : (
        <div className="relative">
          {m.thumb && <img className="mx-auto max-h-96" src={convertFileSrc(m.thumb)} alt="" />}
          <p className="absolute inset-x-0 bottom-2 text-center text-sm text-zinc-300">Preview is available once the proxy is created.</p>
        </div>
      )}
      <p className="truncate px-3 py-1.5 text-xs text-zinc-400" title={m.path}>
        {m.path} · preview proxy only; exports always use the original file
      </p>
      {scenes.length > 0 && (
        <div className="px-3 pb-2">
          <p className="mb-1 text-xs text-zinc-500">
            Scenes · technical score 0–100 (sharpness, exposure, brightness, contrast, stability). Low scores are hints, not exclusions. Click to select.
          </p>
          <div className="flex gap-2 overflow-x-auto pb-1">
            {scenes.map((s) => <SceneCard key={s.id} s={s} onPick={() => pick(s)} />)}
          </div>
        </div>
      )}
      {m.proxy && (
        <div className="flex flex-wrap items-center gap-2 px-3 pb-2 text-xs text-zinc-300">
          <button className="rounded border border-zinc-600 px-2 py-0.5 hover:bg-zinc-800" onClick={() => setIn(now())}>Set In</button>
          <span className="font-mono">{clock(inMs)}</span>
          <button className="rounded border border-zinc-600 px-2 py-0.5 hover:bg-zinc-800" onClick={() => setOut(now())}>Set Out</button>
          <span className="font-mono">{clock(outMs)}</span>
          <span className="text-zinc-500">({clock(Math.max(0, outMs - inMs))})</span>
          <button
            className="ml-auto rounded bg-zinc-100 px-3 py-1 font-medium text-zinc-900 disabled:opacity-40"
            disabled={!project || outMs <= inMs}
            onClick={add}
            title={project ? undefined : "Open a project first"}
          >
            {project ? `Add to ${project.name}` : "Open a project to add clips"}
          </button>
          {message && <span className="w-full text-zinc-400">{message}</span>}
        </div>
      )}
    </div>
  );
}

function MediaRow({ m, job, selected, onSelect, onRemoved, onError }: {
  m: Media;
  job: Job | null;
  selected: boolean;
  onSelect: () => void;
  onRemoved: () => void;
  onError: (e: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const recorded = m.sourceCreatedAt ? new Date(m.sourceCreatedAt) : null;
  return (
    <tr
      className={`cursor-pointer border-t border-zinc-200 dark:border-zinc-800 ${selected ? "bg-zinc-100 dark:bg-zinc-800" : ""}`}
      onClick={onSelect}
    >
      <td className="py-1.5 pr-2">
        {m.thumb ? <img className="h-9 w-16 rounded object-cover" src={convertFileSrc(m.thumb)} alt="" /> : <div className="h-9 w-16 rounded bg-zinc-200 dark:bg-zinc-800" />}
      </td>
      <td className="max-w-56 truncate font-medium" title={m.path}>
        <button className="hover:underline" onClick={(e) => { e.stopPropagation(); onSelect(); }} aria-pressed={selected}>
          {fileName(m.path)}
        </button>
      </td>
      <td>{recorded && !isNaN(recorded.getTime()) ? recorded.toLocaleString() : "–"}</td>
      <td>{duration(m.durationMs)}</td>
      <td>{resolution(m)}{m.fps ? ` · ${Math.round(m.fps * 100) / 100} fps` : ""}</td>
      <td>{m.videoCodec}{m.audioCodec ? ` / ${m.audioCodec}` : " / no audio"}</td>
      <td className="text-right">{formatBytes(m.size)}</td>
      <td className="pl-3 text-xs text-zinc-500">{status(m, job)}</td>
      <td className="pl-3 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
        {confirming ? (
          <>
            <button className="text-xs text-red-600 hover:underline" onClick={() => removeMedia(m.id).then(onRemoved).catch((e) => onError(String(e)))}>
              Remove
            </button>{" "}
            <button className={linkClass} onClick={() => setConfirming(false)}>Keep</button>
          </>
        ) : (
          <button className={linkClass} onClick={() => setConfirming(true)} title="Removes it from the library and from project timelines. The file stays on disk.">
            Remove
          </button>
        )}
      </td>
    </tr>
  );
}

export default function MediaLibrary({ project, onClipAdded }: { project: Project | null; onClipAdded: () => void }) {
  const [media, setMedia] = useState<Media[] | null>(null);
  const [importJob, setImportJob] = useState<Job | null>(null);
  const [job, setJob] = useState<Job | null>(null); // latest proxy / analysis job
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    listMedia().then(setMedia).catch((e) => setError(String(e)));
  }, []);

  // Analysis creates missing proxies first, so one job covers both.
  const analyze = useCallback((ids: number[]) => {
    if (ids.length > 0) submitJob("media.analyze", { ids, quality: "balanced" }).then(setJob).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    load();
    const unlisten = onJobUpdated((j) => {
      if (j.kind === "media.import") {
        setImportJob(j);
        if (j.status === "completed") {
          load();
          analyze((j.result as ImportResult).ids);
        }
      } else if (BACKGROUND.has(j.kind)) {
        setJob((prev) => {
          // A file finished (or the job ended): refresh so its proxy / scenes show up.
          if (j.status !== "running" || (prev && Math.floor(prev.done / STEP) !== Math.floor(j.done / STEP))) load();
          return j;
        });
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, [load, analyze]);

  async function addVideos() {
    setError(null);
    const picked = await open({ multiple: true, title: "Add videos", filters: [{ name: "Videos", extensions: VIDEO_EXTENSIONS }] });
    if (!picked || picked.length === 0) return;
    submitJob("media.import", { paths: picked }).then(setImportJob).catch((e) => setError(String(e)));
  }

  const importResult = importJob?.status === "completed" ? (importJob.result as ImportResult) : null;
  const jobResult = job?.status === "completed" ? (job.result as BatchResult) : null;
  const notAnalysed = media?.filter((m) => m.sceneCount === null).map((m) => m.id) ?? [];
  const current = media?.find((m) => m.id === selected);

  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-zinc-500">Media library</h2>
        <div className="flex gap-2">
          {notAnalysed.length > 0 && !active(job) && (
            <button className="rounded-md border border-zinc-300 px-3 py-1.5 text-sm dark:border-zinc-700" onClick={() => analyze(notAnalysed)}>
              Analyze {notAnalysed.length} video{notAnalysed.length === 1 ? "" : "s"}
            </button>
          )}
          <button className={buttonClass} disabled={active(importJob)} onClick={addVideos}>Add videos</button>
        </div>
      </div>
      <p className="text-xs text-zinc-500">
        Videos are read in place. Memora Studio never copies, moves or modifies the originals. Small preview copies
        (proxies) and analysis frames are kept in the data folder's cache.
      </p>
      {active(importJob) && (
        <p className="mt-2 text-sm text-zinc-500">
          Reading metadata… {importJob!.total > 0 && `${importJob!.done}/${importJob!.total}`}{" "}
          <button className={linkClass} onClick={() => cancelJob(importJob!.id).catch((e) => setError(String(e)))}>Cancel</button>
        </p>
      )}
      {active(job) && (
        <p className="mt-2 text-sm text-zinc-500">
          {job!.kind === "media.analyze" ? "Analysing scenes and image quality…" : "Creating preview proxies…"}{" "}
          {job!.total > 0 && `${Math.round((job!.done / job!.total) * 100)}%`}{" "}
          <button className={linkClass} onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>Cancel</button>
        </p>
      )}
      {importJob?.status === "failed" && <p className="mt-2 text-sm text-red-600">{importJob.error}</p>}
      {job?.status === "failed" && <p className="mt-2 text-sm text-red-600">{job.error}</p>}
      {importResult && (
        <div className="mt-2 text-sm">
          <p className="text-zinc-500">Added or refreshed {importResult.imported} video{importResult.imported === 1 ? "" : "s"}.</p>
          {importResult.failed.map((f) => (
            <p key={f.path} className="text-amber-600" title={f.path}>{fileName(f.path)}: {f.error}</p>
          ))}
        </div>
      )}
      {jobResult?.failed.map((f) => (
        <p key={f.id} className="mt-1 text-sm text-amber-600">#{f.id}: {f.error}</p>
      ))}
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {current && <Preview key={current.id} m={current} project={project} onClipAdded={onClipAdded} />}
      {media?.length === 0 && <p className="mt-3 text-sm text-zinc-500">No videos yet.</p>}
      {media && media.length > 0 && (
        <table className="mt-3 w-full text-sm">
          <thead className="text-left text-xs text-zinc-500">
            <tr>
              <th /><th className="py-1">File</th><th>Recorded</th><th>Length</th><th>Resolution</th><th>Codec</th>
              <th className="text-right">Size</th><th className="pl-3">Analysis</th><th />
            </tr>
          </thead>
          <tbody>
            {media.map((m) => (
              <MediaRow
                key={m.id}
                m={m}
                job={job}
                selected={m.id === selected}
                onSelect={() => setSelected(m.id === selected ? null : m.id)}
                onRemoved={() => { if (m.id === selected) setSelected(null); load(); }}
                onError={setError}
              />
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
