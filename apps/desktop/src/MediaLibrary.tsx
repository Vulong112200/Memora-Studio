import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addClip, cancelJob, formatBytes, getTranscript, listMedia, setMediaMark, listScenes, onJobUpdated, removeMedia, submitJob,
  type Job, type Media, type Project, type ScoredScene, type TagScore, type Transcript,
} from "./api";
import { clock } from "./Timeline";

/** Offered in the file picker; FFprobe decides what is actually a video. */
const VIDEO_EXTENSIONS = ["mp4", "mov", "m4v", "mkv", "avi", "mts", "m2ts", "3gp", "webm", "wmv"];
const linkClass = "text-xs text-zinc-500 hover:underline";
const buttonClass =
  "rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900";
/** Progress units per file in media.proxy / media.analyze / media.transcribe jobs (src-tauri). */
const STEP = 1000;
/** Background jobs that work through a list of media ids. */
const BACKGROUND = new Set(["media.proxy", "media.analyze", "media.transcribe", "media.tag"]);
const JOB_LABEL: Record<string, string> = {
  "media.proxy": "Creating preview proxies…",
  "media.analyze": "Analysing scenes and image quality…",
  "media.transcribe": "Transcribing speech (whisper.cpp, on this computer)…",
  "media.tag": "Recognising visual concepts (OpenCLIP, on this computer)…",
};

/** Lower case without Vietnamese diacritics, so "xe dap" finds "xe đạp". */
const fold = (s: string) => s.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/đ/g, "d").replace(/Đ/g, "d").toLowerCase();

function matches(m: Media, q: string) {
  const f = fold(q.trim());
  if (!f) return true;
  return fold(m.path).includes(f) || (m.tags ?? []).some((t) => !t.negative && t.score >= 0.1 && (fold(t.vi).includes(f) || t.en.includes(f)));
}

const topTags = (tags: TagScore[] | null, n: number, min: number) =>
  (tags ?? []).filter((t) => !t.negative && t.score >= min).slice(0, n).map((t) => t.vi).join(", ");

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

/** "working 40%" / "waiting" while a `kinds` job still has to process `m`, else null. */
function working(m: Media, job: Job | null, kinds: string[], finished: boolean) {
  if (!active(job) || !kinds.includes(job!.kind) || finished) return null;
  const i = ((job!.params as { ids?: number[] } | null)?.ids ?? []).indexOf(m.id);
  if (i < 0) return null;
  const pct = Math.round(((job!.done - i * STEP) / STEP) * 100);
  return pct > 0 && pct < 100 ? `working ${pct}%` : "waiting";
}

/** Analysis state of one clip while `job` may be working on it. */
function status(m: Media, job: Job | null) {
  const w = working(m, job, ["media.proxy", "media.analyze"], m.sceneCount !== null);
  if (w) return w;
  if (m.sceneCount !== null) return `${m.sceneCount} scene${m.sceneCount === 1 ? "" : "s"}`;
  return m.proxy ? "not analysed" : "–";
}

function speech(m: Media, job: Job | null) {
  if (m.audioStreams === 0) return "no audio";
  return working(m, job, ["media.transcribe"], false) ?? (m.transcribed ? "transcribed" : "–");
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
      {s.tags.length > 0 && <div className="mt-0.5 truncate text-sky-300">{topTags(s.tags, 3, 0.1)}</div>}
      {s.tags.filter((t) => t.negative && t.score >= 0.15).map((t) => <div key={t.en} className="mt-0.5 truncate text-amber-400">⚠ {t.vi}</div>)}
      {s.flags.length > 0 && <div className="mt-0.5 truncate text-amber-400">{s.flags.join(" · ")}</div>}
    </button>
  );
}

function Preview({ m, project, onClipAdded }: { m: Media; project: Project | null; onClipAdded: () => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const [inMs, setIn] = useState(0);
  const [outMs, setOut] = useState(m.durationMs);
  const [scenes, setScenes] = useState<ScoredScene[]>([]);
  const [transcript, setTranscript] = useState<Transcript | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const now = () => Math.round((video.current?.currentTime ?? 0) * 1000);

  useEffect(() => {
    if (m.sceneCount !== null) listScenes(m.id).then(setScenes).catch((e) => setMessage(String(e)));
  }, [m.id, m.sceneCount]);

  useEffect(() => {
    if (m.transcribed) getTranscript(m.id).then(setTranscript).catch((e) => setMessage(String(e)));
  }, [m.id, m.transcribed]);

  function pick(s: { startMs: number; endMs: number }) {
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
      {transcript && (
        <div className="px-3 pb-2 text-xs text-zinc-300">
          <p className="mb-1 text-zinc-500">
            Speech · {transcript.modelId}{transcript.language ? ` · language ${transcript.language}` : ""} · machine transcription, may
            contain mistakes or text invented from background noise. Click a line to select it.
          </p>
          {transcript.segments.length === 0 ? (
            <p className="text-zinc-500">No speech found.</p>
          ) : (
            <div className="max-h-40 overflow-y-auto">
              {transcript.segments.map((s, i) => (
                <button key={i} className="block w-full rounded px-1 text-left hover:bg-zinc-800" onClick={() => pick(s)}>
                  <span className="mr-2 font-mono text-zinc-500">{clock(s.startMs)}</span>{s.text}
                </button>
              ))}
            </div>
          )}
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

function MediaRow({ m, job, selected, onSelect, onRemoved, onError, onChanged }: {
  m: Media;
  job: Job | null;
  selected: boolean;
  onSelect: () => void;
  onRemoved: () => void;
  onError: (e: string) => void;
  onChanged: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const mark = (change: Partial<typeof m.mark>) => setMediaMark(m.id, { ...m.mark, ...change }).then(onChanged).catch((e) => onError(String(e)));
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
      <td className="pl-3 text-xs text-zinc-500">{speech(m, job)}</td>
      <td className="max-w-40 truncate pl-3 text-xs text-zinc-500" title={topTags(m.tags, 8, 0.05)}>
        {working(m, job, ["media.tag"], false) ?? (m.tags ? topTags(m.tags, 3, 0.05) || "–" : "–")}
      </td>
      <td className="pl-3 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
        <button className={`mr-2 text-sm ${m.mark.favorite ? "text-amber-500" : "text-zinc-400 hover:text-amber-500"}`} aria-pressed={m.mark.favorite}
          onClick={() => mark({ favorite: !m.mark.favorite })} title="Favorite: preferred by Smart edit">★</button>
        <button className={`mr-2 text-xs ${m.mark.excluded ? "text-red-600" : "text-zinc-400 hover:text-red-600"}`} aria-pressed={m.mark.excluded}
          onClick={() => mark({ excluded: !m.mark.excluded })} title="Exclude: never used by Smart edit">{m.mark.excluded ? "excluded" : "⊘"}</button>
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
  const [speechModel, setSpeechModel] = useState("whisper-small");
  const [query, setQuery] = useState("");

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
  const notTagged = media?.filter((m) => m.tags === null).map((m) => m.id) ?? [];
  const shown = media?.filter((m) => matches(m, query)) ?? [];
  const notTranscribed = media?.filter((m) => m.audioStreams > 0 && !m.transcribed).map((m) => m.id) ?? [];
  const transcribe = (ids: number[]) =>
    submitJob("media.transcribe", { ids, model: speechModel, language: "auto" }).then(setJob).catch((e) => setError(String(e)));
  const current = media?.find((m) => m.id === selected);

  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-zinc-500">Media library</h2>
        <div className="flex flex-wrap justify-end gap-2">
          {notAnalysed.length > 0 && !active(job) && (
            <button className="rounded-md border border-zinc-300 px-3 py-1.5 text-sm dark:border-zinc-700" onClick={() => analyze(notAnalysed)}>
              Analyze {notAnalysed.length} video{notAnalysed.length === 1 ? "" : "s"}
            </button>
          )}
          {notTagged.length > 0 && !active(job) && (
            <button className="rounded-md border border-zinc-300 px-3 py-1.5 text-sm dark:border-zinc-700"
              onClick={() => submitJob("media.tag", { ids: notTagged }).then(setJob).catch((e) => setError(String(e)))}>
              Tag {notTagged.length} video{notTagged.length === 1 ? "" : "s"}
            </button>
          )}
          {notTranscribed.length > 0 && !active(job) && (
            <>
              <select className="rounded-md border border-zinc-300 bg-transparent px-2 text-sm dark:border-zinc-700" value={speechModel}
                onChange={(e) => setSpeechModel(e.target.value)} aria-label="Speech model" title="Speech model (install it in AI models first)">
                <option value="whisper-base">Whisper base (faster)</option>
                <option value="whisper-small">Whisper small (more accurate)</option>
              </select>
              <button className="rounded-md border border-zinc-300 px-3 py-1.5 text-sm dark:border-zinc-700" onClick={() => transcribe(notTranscribed)}>
                Transcribe {notTranscribed.length} video{notTranscribed.length === 1 ? "" : "s"}
              </button>
            </>
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
          {JOB_LABEL[job!.kind]}{" "}
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
        <input className="mt-3 w-full rounded-md border border-zinc-300 bg-transparent px-3 py-1.5 text-sm dark:border-zinc-700"
          placeholder="Search by file name or visual concept (xe đạp, beach, hoàng hôn…)" value={query}
          onChange={(e) => setQuery(e.target.value)} aria-label="Search media" />
      )}
      {media && media.length > 0 && shown.length === 0 && <p className="mt-2 text-sm text-zinc-500">No match.</p>}
      {shown.length > 0 && (
        <div className="overflow-x-auto">
        <table className="mt-3 w-full text-sm">
          <thead className="text-left text-xs text-zinc-500">
            <tr>
              <th /><th className="py-1">File</th><th>Recorded</th><th>Length</th><th>Resolution</th><th>Codec</th>
              <th className="text-right">Size</th><th className="pl-3">Analysis</th><th className="pl-3">Speech</th><th className="pl-3">Concepts</th><th />
            </tr>
          </thead>
          <tbody>
            {shown.map((m) => (
              <MediaRow
                key={m.id}
                m={m}
                job={job}
                selected={m.id === selected}
                onSelect={() => setSelected(m.id === selected ? null : m.id)}
                onRemoved={() => { if (m.id === selected) setSelected(null); load(); }}
                onError={setError}
                onChanged={load}
              />
            ))}
          </tbody>
        </table>
        </div>
      )}
    </section>
  );
}
