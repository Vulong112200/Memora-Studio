import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  cancelJob, getExportOptions, listClips, listMedia, listSounds, moveClip, onJobUpdated, recommendSounds, removeClip, setClipCaption, setExportOptions, submitJob,
  type Clip, type ExportOptions, type ExportPreset, type ExportResult, type Job, type Media, type Project, type SoundRecommendation, type Track,
} from "./api";

const PRESETS: { id: ExportPreset; label: string }[] = [
  { id: "landscape", label: "Landscape 16:9 · 1920×1080" },
  { id: "story", label: "Story 9:16 · 1080×1920" },
  { id: "reels", label: "Reels 9:16 · 1080×1920" },
  { id: "archive", label: "Archive · source size and frame rate" },
];
const TRANSITIONS: [string, string][] = [
  ["fade", "Cross dissolve"], ["fadeblack", "Dip to black"], ["dissolve", "Film dissolve (grain)"],
  ["smoothleft", "Smooth slide"], ["slideleft", "Slide"], ["wipeleft", "Wipe"], ["circleopen", "Circle open"],
];
const COLORS: [string, string][] = [
  ["none", "Natural (no grade)"], ["warm", "Warm"], ["cool", "Cool"], ["vivid", "Vivid"],
  ["cinematic", "Cinematic (teal & orange)"], ["vintage", "Vintage"], ["bw", "Black & white"],
];
const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"];
const linkClass = "text-xs text-zinc-500 hover:underline disabled:opacity-30";
const field = "rounded-md border border-zinc-300 bg-transparent px-2 py-1.5 text-sm dark:border-zinc-700";
const label = "flex items-center gap-2 text-xs text-zinc-500";

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

/** Per-clip caption: saved when the field loses focus. */
function CaptionField({ clip, onSaved, onError }: { clip: Clip; onSaved: () => void; onError: (e: string) => void }) {
  const [text, setText] = useState(clip.caption ?? "");
  useEffect(() => setText(clip.caption ?? ""), [clip.caption]);
  return (
    <input className="w-48 rounded border border-transparent bg-transparent px-1 text-xs hover:border-zinc-300 focus:border-zinc-400 dark:hover:border-zinc-700"
      placeholder="Caption (optional)" value={text} maxLength={200} aria-label="Clip caption"
      onChange={(e) => setText(e.target.value)}
      onBlur={() => { if (text !== (clip.caption ?? "")) setClipCaption(clip.id, text).then(onSaved).catch((e) => onError(String(e))); }} />
  );
}

const minutes = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.round(ms / 1000) % 60).padStart(2, "0")}`;

/** Free music library: recommendations for this video, mood filter, streamed preview, choose. */
function MusicLibrary({ projectId, version, current, onUse, onClose }: {
  projectId: number; version: number; current: string | null; onUse: (t: Track) => void; onClose: () => void;
}) {
  const [tracks, setTracks] = useState<Track[]>([]);
  const [recs, setRecs] = useState<SoundRecommendation[]>([]);
  const [mood, setMood] = useState("all");
  const [playing, setPlaying] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement>(null);
  useEffect(() => { listSounds().then(setTracks).catch((e) => setError(String(e))); }, []);
  useEffect(() => { recommendSounds(projectId).then(setRecs).catch((e) => setError(String(e))); }, [projectId, version]);
  useEffect(() => () => audio.current?.pause(), []);

  function toggle(t: Track) {
    const a = audio.current!;
    if (playing === t.id) { a.pause(); setPlaying(null); return; }
    a.src = t.url;
    a.play().then(() => setPlaying(t.id)).catch(() => setError("Preview needs an internet connection."));
  }
  const moods = ["all", ...Array.from(new Set(tracks.flatMap((t) => t.moods))).sort()];
  const byId = new Map(tracks.map((t) => [t.id, t]));
  const row = (t: Track, why?: string[]) => (
    <li key={t.id} className={`flex items-center gap-2 py-1 text-xs ${current === t.id ? "text-green-600" : ""}`}>
      <button className="w-6 rounded border border-zinc-300 text-center dark:border-zinc-700" onClick={() => toggle(t)} aria-label={`Preview ${t.title}`}>
        {playing === t.id ? "■" : "▶"}
      </button>
      <span className="min-w-0 flex-1 truncate" title={t.credit}>
        <span className="font-medium">{t.title}</span> <span className="text-zinc-500">· {t.moods.join(", ")} · {t.energy} energy · {minutes(t.durationMs)}</span>
        {why && <span className="block truncate text-zinc-500">{why.join(" · ")}</span>}
      </span>
      <button className="rounded border border-zinc-300 px-2 dark:border-zinc-700" onClick={() => onUse(t)}>{current === t.id ? "Chosen" : "Use"}</button>
    </li>
  );
  return (
    <div className="mt-2 rounded-md border border-zinc-200 p-2 dark:border-zinc-800">
      <audio ref={audio} onEnded={() => setPlaying(null)} />
      <div className="flex items-center justify-between">
        <p className="text-xs font-semibold text-zinc-500">Free music library · {tracks.length} tracks</p>
        <button className={linkClass} onClick={onClose}>Close</button>
      </div>
      <p className="mt-1 text-xs text-zinc-500">
        Music by Kevin MacLeod (incompetech.com), free under CC BY 4.0 — the required credit is added at the end of the video.
        Preview streams from the internet; a chosen track is downloaded only while exporting and deleted right after.
      </p>
      {recs.length > 0 && (
        <>
          <p className="mt-2 text-xs font-semibold text-zinc-500">Recommended for this video</p>
          <ul>{recs.map((r) => byId.get(r.id) && row(byId.get(r.id)!, r.reasons))}</ul>
        </>
      )}
      <div className="mt-2 flex flex-wrap gap-1">
        {moods.map((m) => (
          <button key={m} className={`rounded-full border px-2 text-xs ${mood === m ? "border-zinc-900 dark:border-zinc-100" : "border-zinc-300 text-zinc-500 dark:border-zinc-700"}`}
            onClick={() => setMood(m)}>{m}</button>
        ))}
      </div>
      <ul className="mt-1 max-h-56 overflow-y-auto">{tracks.filter((t) => mood === "all" || t.moods.includes(mood)).map((t) => row(t))}</ul>
      {error && <p className="text-xs text-red-600">{error}</p>}
    </div>
  );
}

/** Look & sound panel (docs/Production.md §6.3): every option is saved per project. */
function LookAndSound({ o, projectId, version, onChange, onError }: {
  o: ExportOptions; projectId: number; version: number; onChange: (o: ExportOptions) => void; onError: (e: string) => void;
}) {
  const [library, setLibrary] = useState(false);
  const [trackNames, setTrackNames] = useState<Map<string, Track>>(new Map());
  useEffect(() => { listSounds().then((t) => setTrackNames(new Map(t.map((x) => [x.id, x])))).catch(() => {}); }, []);
  const set = (change: Partial<ExportOptions>) => onChange({ ...o, ...change });
  const titles = (change: Partial<ExportOptions["titles"]>) => set({ titles: { ...o.titles, ...change } });
  async function pickMusic() {
    try {
      const picked = await open({ multiple: false, title: "Background music (your own file)", filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
      if (typeof picked === "string") set({ music: { path: picked, library: null, volume: o.music?.volume ?? 0.35, duck: o.music?.duck ?? true } });
    } catch (e) {
      onError(String(e));
    }
  }
  return (
    <div className="mt-3 grid gap-4 rounded-lg border border-zinc-200 p-3 md:grid-cols-3 dark:border-zinc-800">
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-zinc-500">Picture</p>
        <label className={label}>Transitions
          <select className={field} value={o.transitionMs} onChange={(e) => set({ transitionMs: Number(e.target.value) })}>
            {[[0, "Hard cuts"], [300, "Short · 0.3 s"], [500, "Medium · 0.5 s"], [600, "Medium · 0.6 s"], [800, "Slow · 0.8 s"], [1200, "Very slow · 1.2 s"]]
              .concat([0, 300, 500, 600, 800, 1200].includes(o.transitionMs) ? [] : [[o.transitionMs, `${o.transitionMs / 1000} s`]])
              .map(([v, name]) => <option key={v} value={v}>{name}</option>)}
          </select>
        </label>
        {o.transitionMs > 0 && (
          <label className={label}>Style
            <select className={field} value={o.transition} onChange={(e) => set({ transition: e.target.value })}>
              {TRANSITIONS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
            </select>
          </label>
        )}
        <label className={label}>Colour
          <select className={field} value={o.color} onChange={(e) => set({ color: e.target.value })}>
            {COLORS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
          </select>
        </label>
        <label className={label}><input type="checkbox" checked={o.fill} onChange={(e) => set({ fill: e.target.checked })} /> Fill the frame (crop centre, no black bars)</label>
        <label className={label}><input type="checkbox" checked={o.fade} onChange={(e) => set({ fade: e.target.checked })} /> Fade in from / out to black</label>
      </div>
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-zinc-500">Sound</p>
        <label className={label}>Clip sound {Math.round(o.originalVolume * 100)}%
          <input type="range" min={0} max={1.5} step={0.05} value={o.originalVolume} onChange={(e) => set({ originalVolume: Number(e.target.value) })} />
        </label>
        {o.music ? (
          <>
            {o.music.library ? (
              <p className="text-xs" title={trackNames.get(o.music.library)?.credit}>♪ {trackNames.get(o.music.library)?.title ?? o.music.library} · free library
                <span className="block text-zinc-500">Downloaded only while exporting, then deleted; credit added at the end.</span>
              </p>
            ) : (
              <p className="truncate text-xs" title={o.music.path}>♪ {fileName(o.music.path)} · your file (kept as is)</p>
            )}
            <p className="text-xs">
              <button className={linkClass} onClick={() => setLibrary(true)}>Library…</button>{" "}
              <button className={linkClass} onClick={pickMusic}>Own file…</button>{" "}
              <button className={linkClass} onClick={() => set({ music: null })}>Remove</button>
            </p>
            <label className={label}>Music {Math.round(o.music.volume * 100)}%
              <input type="range" min={0} max={1.5} step={0.05} value={o.music.volume} onChange={(e) => set({ music: { ...o.music!, volume: Number(e.target.value) } })} />
            </label>
            <label className={label}><input type="checkbox" checked={o.music.duck} onChange={(e) => set({ music: { ...o.music!, duck: e.target.checked } })} /> Lower the music while people talk</label>
          </>
        ) : (
          <p className="text-xs">
            <button className={linkClass} onClick={() => setLibrary(true)}>+ Free music library…</button>{" "}
            <button className={linkClass} onClick={pickMusic} title="Music you own or may use. Looped to the video length and faded out at the end.">+ Your own file…</button>
          </p>
        )}
        {library && (
          <MusicLibrary projectId={projectId} version={version} current={o.music?.library ?? null} onClose={() => setLibrary(false)}
            onUse={(t) => set({ music: { path: "", library: t.id, volume: o.music?.volume ?? 0.35, duck: o.music?.duck ?? true } })} />
        )}
        <label className={label}><input type="checkbox" checked={o.normalize} onChange={(e) => set({ normalize: e.target.checked })} /> Even loudness (−16 LUFS, phone / social level)</label>
      </div>
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-zinc-500">Text</p>
        <input className={`${field} w-full`} placeholder="Opening title" value={o.titles.title} maxLength={80} onChange={(e) => titles({ title: e.target.value })} aria-label="Opening title" />
        <input className={`${field} w-full`} placeholder="Subtitle (place, date…)" value={o.titles.subtitle} maxLength={80} onChange={(e) => titles({ subtitle: e.target.value })} aria-label="Subtitle" />
        <input className={`${field} w-full`} placeholder="Ending text" value={o.titles.ending} maxLength={80} onChange={(e) => titles({ ending: e.target.value })} aria-label="Ending text" />
        <label className={label}><input type="checkbox" checked={o.titles.captions} onChange={(e) => titles({ captions: e.target.checked })} /> Subtitles from speech (transcribed videos)</label>
        <p className="text-xs text-zinc-500">Per-clip captions: type them next to each clip below.</p>
      </div>
    </div>
  );
}

export default function Timeline({ project, version }: { project: Project; version: number }) {
  const [clips, setClips] = useState<Clip[]>([]);
  const [media, setMedia] = useState<Map<number, Media>>(new Map());
  const [preset, setPreset] = useState<ExportPreset>("landscape");
  const [options, setOptions] = useState<ExportOptions | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    listClips(project.id).then(setClips).catch((e) => setError(String(e)));
    listMedia().then((m) => setMedia(new Map(m.map((x) => [x.id, x])))).catch((e) => setError(String(e)));
  }, [project.id]);
  useEffect(load, [load, version]);
  useEffect(() => {
    getExportOptions(project.id).then((o) => { setOptions(o); if (o.fill) setPreset((p) => (p === "landscape" ? "reels" : p)); }).catch((e) => setError(String(e)));
  }, [project.id, version]);

  useEffect(() => {
    const unlisten = onJobUpdated((j) => {
      if (j.kind === "media.export" && (j.params as { projectId?: number }).projectId === project.id) setJob(j);
    });
    return () => { unlisten.then((f) => f()); };
  }, [project.id]);

  function change(o: ExportOptions) {
    setOptions(o);
    setExportOptions(project.id, o).then(() => setError(null)).catch((e) => setError(String(e)));
  }

  const run = (p: Promise<unknown>) => p.then(load).catch((e) => setError(String(e)));
  const exporting = job && (job.status === "queued" || job.status === "running");
  const overlap = Math.min(options?.transitionMs ?? 0, ...clips.map((c) => (c.outMs - c.inMs) / 2));
  const total = clips.reduce((s, c) => s + c.outMs - c.inMs, 0) - Math.max(0, clips.length - 1) * Math.max(0, overlap);
  const result = job?.status === "completed" ? (job.result as ExportResult) : null;

  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-zinc-500">Timeline · {project.name}</h2>
        <div className="flex items-center gap-2">
          <select className={field} value={preset} aria-label="Export format"
            onChange={(e) => {
              const p = e.target.value as ExportPreset;
              setPreset(p);
              if (options) change({ ...options, fill: p === "story" || p === "reels" });
            }}>
            {PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
          </select>
          <button
            className="rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900"
            disabled={clips.length === 0 || !!exporting || !options}
            onClick={() => submitJob("media.export", { projectId: project.id, preset, options }).then(setJob).catch((e) => setError(String(e)))}
          >
            Export
          </button>
        </div>
      </div>
      {clips.length === 0 ? (
        <p className="text-sm text-zinc-500">No clips yet. Use Smart edit above, or select a video in the Media library, set In / Out and add it here.</p>
      ) : (
        <>
          <ol className="divide-y divide-zinc-200 dark:divide-zinc-800">
            {clips.map((c, i) => {
              const m = media.get(c.mediaId);
              return (
                <li key={c.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5 text-sm">
                  <span className="w-5 text-right text-xs text-zinc-500">{i + 1}</span>
                  {m?.thumb ? <img className="h-9 w-16 rounded object-cover" src={convertFileSrc(m.thumb)} alt="" /> : <div className="h-9 w-16 rounded bg-zinc-200 dark:bg-zinc-800" />}
                  <span className="min-w-0 flex-1 basis-40 truncate" title={m?.path}>{m ? fileName(m.path) : `media #${c.mediaId}`}</span>
                  <CaptionField clip={c} onSaved={load} onError={setError} />
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
      {options && <LookAndSound o={options} projectId={project.id} version={version + clips.length} onChange={change} onError={setError} />}
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
