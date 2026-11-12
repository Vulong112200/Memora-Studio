import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addClip, cancelJob, formatBytes, getTranscript, listMedia, listModels, setMediaMark, listScenes, onJobUpdated, removeMedia, submitJob,
  type Job, type Media, type ModelInfo, type Project, type ScoredScene, type TagScore, type Transcript,
} from "./api";
import { clock } from "./Timeline";
import { dateTime, L } from "./i18n";
import { active, btn, Card, field, fileName, jobLabel, prepareMedia, preparation, Progress, Thumb } from "./ui";

/** Offered in the file picker; FFprobe decides what is actually a video. */
const VIDEO_EXTENSIONS = ["mp4", "mov", "m4v", "mkv", "avi", "mts", "m2ts", "3gp", "webm", "wmv"];
/** Progress units per file in media.proxy / media.analyze / media.transcribe jobs (src-tauri). */
const STEP = 1000;
/** Background jobs that work through a list of media ids. */
const BACKGROUND = new Set(["media.proxy", "media.analyze", "media.transcribe", "media.tag"]);

/** Lower case without Vietnamese diacritics, so "xe dap" finds "xe đạp". */
const fold = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/đ/g, "d").replace(/Đ/g, "d").toLowerCase();

function matches(m: Media, q: string) {
  const f = fold(q.trim());
  if (!f) return true;
  return fold(m.path).includes(f) || (m.tags ?? []).some((t) => !t.negative && t.score >= 0.1 && (fold(t.vi).includes(f) || t.en.includes(f)));
}

const topTags = (tags: TagScore[] | null, n: number, min: number) =>
  (tags ?? []).filter((t) => !t.negative && t.score >= min).slice(0, n).map((t) => t.vi).join(", ");

interface ImportResult { imported: number; ids: number[]; failed: { path: string; error: string }[] }
interface BatchResult { failed: { id: number; error: string }[] }

function duration(ms: number) {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Size as the video is displayed (phone portrait clips are stored landscape + rotation). */
function resolution(m: Media) {
  return m.rotation % 180 === 0 ? `${m.width}×${m.height}` : `${m.height}×${m.width}`;
}

/** "40%" / "waiting" while a `kinds` job still has to process `m`, else null. */
function working(m: Media, job: Job | null, kinds: string[], finished: boolean) {
  if (!active(job) || !kinds.includes(job!.kind) || finished) return null;
  const i = ((job!.params as { ids?: number[] } | null)?.ids ?? []).indexOf(m.id);
  if (i < 0) return null;
  const pct = Math.round(((job!.done - i * STEP) / STEP) * 100);
  return pct > 0 && pct < 100 ? `${L("working", "đang xử lý")} ${pct}%` : L("waiting", "đang chờ");
}

/** Analysis state of one clip while `job` may be working on it. */
function status(m: Media, job: Job | null) {
  const w = working(m, job, ["media.proxy", "media.analyze"], m.sceneCount !== null);
  if (w) return w;
  if (m.sceneCount !== null) return `${m.sceneCount} ${L("scenes", "cảnh")}`;
  return m.proxy ? L("not analysed", "chưa phân tích") : "–";
}

function speech(m: Media, job: Job | null) {
  if (m.audioStreams === 0) return L("no audio", "không tiếng");
  return working(m, job, ["media.transcribe"], false) ?? (m.transcribed ? L("transcribed", "đã nhận dạng") : "–");
}

const pct = (x: number) => `${Math.round(x * 100)}`;

function SceneCard({ s, onPick }: { s: ScoredScene; onPick: () => void }) {
  const t = s.technical;
  const tone = t.total >= 0.7 ? "text-green-500" : t.total >= 0.4 ? "text-amber-500" : "text-red-500";
  return (
    <button className="w-40 shrink-0 rounded bg-zinc-900 p-1 text-left text-xs text-zinc-300 hover:bg-zinc-800" onClick={onPick}
      title={`${L("Sharpness", "Độ nét")} ${pct(t.sharpness)} · ${L("Exposure", "Phơi sáng")} ${pct(t.exposure)} · ${L("Brightness", "Độ sáng")} ${pct(t.brightness)} · ${L("Contrast", "Tương phản")} ${pct(t.contrast)} · ${L("Stability", "Độ ổn định")} ${pct(t.stability)}`}>
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
      setMessage(L(`Added ${clock(outMs - inMs)} to ${project.name}.`, `Đã thêm ${clock(outMs - inMs)} vào ${project.name}.`));
      onClipAdded();
    } catch (e) {
      setMessage(String(e));
    }
  }

  const small = "rounded border border-zinc-600 px-2 py-0.5 hover:bg-zinc-800";
  return (
    <div className="mb-4 rounded-xl bg-black">
      {m.proxy ? (
        <video ref={video} key={m.id} className="mx-auto max-h-96" src={convertFileSrc(m.proxy)} controls autoPlay />
      ) : (
        <div className="relative">
          {m.thumb && <img className="mx-auto max-h-96" src={convertFileSrc(m.thumb)} alt="" />}
          <p className="absolute inset-x-0 bottom-2 text-center text-sm text-zinc-300">{L("Preview is available once the video is analysed.", "Xem trước được khi video đã phân tích.")}</p>
        </div>
      )}
      <p className="truncate px-3 py-1.5 text-xs text-zinc-400" title={m.path}>
        {m.path} · {L("preview copy only; exports always use the original file", "chỉ là bản xem trước; khi xuất luôn dùng file gốc")}
      </p>
      {scenes.length > 0 && (
        <div className="px-3 pb-2">
          <p className="mb-1 text-xs text-zinc-500">
            {L("Scenes · technical score 0–100 (sharpness, exposure, brightness, contrast, stability). Low scores are hints, not exclusions. Click to select.",
              "Các cảnh · điểm kỹ thuật 0–100 (độ nét, phơi sáng, độ sáng, tương phản, độ ổn định). Điểm thấp chỉ là gợi ý. Bấm để chọn đoạn.")}
          </p>
          <div className="flex gap-2 overflow-x-auto pb-1">
            {scenes.map((s) => <SceneCard key={s.id} s={s} onPick={() => pick(s)} />)}
          </div>
        </div>
      )}
      {transcript && (
        <div className="px-3 pb-2 text-xs text-zinc-300">
          <p className="mb-1 text-zinc-500">
            {L("Speech", "Lời nói")} · {transcript.modelId}{transcript.language ? ` · ${transcript.language}` : ""} ·{" "}
            {L("machine transcription, may contain mistakes. Click a line to select it.", "nhận dạng tự động, có thể sai. Bấm một dòng để chọn đoạn đó.")}
          </p>
          {transcript.segments.length === 0 ? (
            <p className="text-zinc-500">{L("No speech found.", "Không có lời nói.")}</p>
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
          <button className={small} onClick={() => setIn(now())}>{L("Set In", "Đặt điểm vào")}</button>
          <span className="font-mono">{clock(inMs)}</span>
          <button className={small} onClick={() => setOut(now())}>{L("Set Out", "Đặt điểm ra")}</button>
          <span className="font-mono">{clock(outMs)}</span>
          <span className="text-zinc-500">({clock(Math.max(0, outMs - inMs))})</span>
          <button className="ml-auto rounded bg-zinc-100 px-3 py-1 font-medium text-zinc-900 disabled:opacity-40" disabled={!project || outMs <= inMs} onClick={add}>
            {project ? L(`Add to ${project.name}`, `Thêm vào ${project.name}`) : L("Open a project to add clips", "Mở một dự án để thêm clip")}
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
    <tr className={`cursor-pointer border-t border-zinc-200 dark:border-zinc-800 ${selected ? "bg-indigo-50 dark:bg-indigo-950/40" : "hover:bg-zinc-50 dark:hover:bg-zinc-800/50"}`} onClick={onSelect}>
      <td className="py-1.5 pr-2"><Thumb path={m.thumb} className="h-10 w-[72px]" /></td>
      <td className="max-w-56 truncate font-medium" title={m.path}>
        <button className="hover:underline" onClick={(e) => { e.stopPropagation(); onSelect(); }} aria-pressed={selected}>{fileName(m.path)}</button>
      </td>
      <td>{recorded && !isNaN(recorded.getTime()) ? dateTime(recorded) : "–"}</td>
      <td>{duration(m.durationMs)}</td>
      <td>{resolution(m)}{m.fps ? ` · ${Math.round(m.fps * 100) / 100} fps` : ""}</td>
      <td className="text-right">{formatBytes(m.size)}</td>
      <td className="pl-3 text-xs text-zinc-500">{status(m, job)}</td>
      <td className="pl-3 text-xs text-zinc-500">{speech(m, job)}</td>
      <td className="max-w-40 truncate pl-3 text-xs text-zinc-500" title={topTags(m.tags, 8, 0.05)}>
        {working(m, job, ["media.tag"], false) ?? (m.tags ? topTags(m.tags, 3, 0.05) || "–" : "–")}
      </td>
      <td className="pl-3 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
        <button className={`mr-2 text-sm ${m.mark.favorite ? "text-amber-500" : "text-zinc-400 hover:text-amber-500"}`} aria-pressed={m.mark.favorite}
          onClick={() => mark({ favorite: !m.mark.favorite })} title={L("Favorite: preferred by Smart edit", "Yêu thích: Dựng tự động ưu tiên")}>★</button>
        <button className={`mr-2 text-xs ${m.mark.excluded ? "text-red-600" : "text-zinc-400 hover:text-red-600"}`} aria-pressed={m.mark.excluded}
          onClick={() => mark({ excluded: !m.mark.excluded })} title={L("Exclude: never used by Smart edit", "Loại trừ: Dựng tự động không dùng")}>
          {m.mark.excluded ? L("excluded", "đã loại") : "⊘"}
        </button>
        {confirming ? (
          <>
            <button className="text-xs text-red-600 hover:underline" onClick={() => removeMedia(m.id).then(onRemoved).catch((e) => onError(String(e)))}>{L("Remove", "Gỡ")}</button>{" "}
            <button className={btn.link} onClick={() => setConfirming(false)}>{L("Keep", "Giữ")}</button>
          </>
        ) : (
          <button className={btn.link} onClick={() => setConfirming(true)}
            title={L("Removes it from the library and from project timelines. The file stays on disk.", "Gỡ khỏi thư viện và timeline các dự án. File vẫn còn trên ổ đĩa.")}>
            {L("Remove", "Gỡ")}
          </button>
        )}
      </td>
    </tr>
  );
}

export default function MediaLibrary({ project, version, onClipAdded, onSettings }: {
  project: Project | null; version: number; onClipAdded: () => void; onSettings: () => void;
}) {
  const [media, setMedia] = useState<Media[] | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [importJob, setImportJob] = useState<Job | null>(null);
  const [job, setJob] = useState<Job | null>(null); // latest proxy / analysis / tag / transcription job
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const load = useCallback(() => {
    listMedia().then(setMedia).catch((e) => setError(String(e)));
    listModels().then(setModels).catch(() => {});
  }, []);
  useEffect(load, [load, version]);

  useEffect(() => {
    const unlisten = onJobUpdated((j) => {
      if (j.kind === "media.import") {
        setImportJob(j);
        if (j.status === "completed") {
          // New videos get everything Smart edit needs, without the owner having to find each button.
          const ids = new Set((j.result as ImportResult).ids);
          Promise.all([listMedia(), listModels()])
            .then(([all, ms]) => { setMedia(all); return prepareMedia(all.filter((m) => ids.has(m.id)), ms); })
            .catch((e) => setError(String(e)));
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
  }, [load]);

  async function addVideos() {
    setError(null);
    const picked = await open({ multiple: true, title: L("Add videos", "Thêm video"), filters: [{ name: "Videos", extensions: VIDEO_EXTENSIONS }] });
    if (!picked || picked.length === 0) return;
    submitJob("media.import", { paths: picked }).then(setImportJob).catch((e) => setError(String(e)));
  }

  const importResult = importJob?.status === "completed" ? (importJob.result as ImportResult) : null;
  const jobResult = job?.status === "completed" ? (job.result as BatchResult) : null;
  const shown = media?.filter((m) => matches(m, query)) ?? [];
  const prep = preparation(media ?? [], models);
  const todo = prep.analyse.length + prep.tag.length + prep.transcribe.length;
  const current = media?.find((m) => m.id === selected);
  const busy = active(job) || active(importJob);

  return (
    <Card title={L("Video library", "Thư viện video")} actions={
      <>
        {todo > 0 && !busy && (
          <button className={btn.secondary} onClick={() => prepareMedia(media ?? [], models).catch((e) => setError(String(e)))}
            title={L("Scenes and image quality, then what is on screen (AI), then speech (AI)", "Cảnh và chất lượng hình, rồi nội dung hình (AI), rồi lời nói (AI)")}>
            {L(`Analyse (${todo} steps)`, `Phân tích (${todo} bước)`)}
          </button>
        )}
        <button className={btn.primary} disabled={active(importJob)} onClick={addVideos}>{L("+ Add videos", "+ Thêm video")}</button>
      </>
    }>
      <p className="text-xs text-zinc-500">
        {L("Videos are read in place: Memora Studio never copies, moves or modifies the originals. New videos are analysed automatically.",
          "Video được đọc tại chỗ: Memora Studio không sao chép, di chuyển hay sửa file gốc. Video mới được phân tích tự động.")}
        {prep.missingModels && media && media.length > 0 && (
          <> {L("AI models are not all installed, so some analysis is skipped:", "Chưa cài đủ model AI nên một số bước phân tích bị bỏ qua:")}{" "}
            <button className="text-indigo-600 hover:underline" onClick={onSettings}>{L("Settings › AI models", "Cài đặt › Model AI")}</button>.</>
        )}
      </p>
      {active(importJob) && (
        <p className="mt-2 text-sm text-zinc-500">
          {jobLabel("media.import")}… {importJob!.total > 0 && `${importJob!.done}/${importJob!.total}`}{" "}
          <button className={btn.link} onClick={() => cancelJob(importJob!.id).catch((e) => setError(String(e)))}>{L("Cancel", "Huỷ")}</button>
        </p>
      )}
      {active(job) && (
        <div className="mt-2 text-sm text-zinc-500">
          {jobLabel(job!.kind)}… {job!.total > 0 && `${Math.round((job!.done / job!.total) * 100)}%`}{" "}
          <button className={btn.link} onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>{L("Cancel", "Huỷ")}</button>
          <div className="mt-1 max-w-sm"><Progress value={job!.total ? job!.done / job!.total : 0} /></div>
        </div>
      )}
      {importJob?.status === "failed" && <p className="mt-2 text-sm text-red-600">{importJob.error}</p>}
      {job?.status === "failed" && <p className="mt-2 text-sm text-red-600">{job.error}</p>}
      {importResult && (
        <div className="mt-2 text-sm">
          <p className="text-zinc-500">{L(`Added or refreshed ${importResult.imported} video(s).`, `Đã thêm / cập nhật ${importResult.imported} video.`)}</p>
          {importResult.failed.map((f) => <p key={f.path} className="text-amber-600" title={f.path}>{fileName(f.path)}: {f.error}</p>)}
        </div>
      )}
      {jobResult?.failed.map((f) => <p key={f.id} className="mt-1 text-sm text-amber-600">#{f.id}: {f.error}</p>)}
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {media?.length === 0 && (
        <div className="mt-4 rounded-xl border border-dashed border-zinc-300 p-8 text-center dark:border-zinc-700">
          <p className="text-sm text-zinc-500">{L("No videos yet. Add the videos of a trip or an event to begin.", "Chưa có video. Thêm video của một chuyến đi hay sự kiện để bắt đầu.")}</p>
          <button className={`${btn.primary} mt-3`} onClick={addVideos}>{L("+ Add videos", "+ Thêm video")}</button>
        </div>
      )}
      {media && media.length > 0 && (
        <input className={`${field} mb-3 mt-3 w-full`} placeholder={L("Search by file name or what is on screen (xe đạp, beach, hoàng hôn…)", "Tìm theo tên file hoặc nội dung hình (xe đạp, biển, hoàng hôn…)")}
          value={query} onChange={(e) => setQuery(e.target.value)} aria-label={L("Search media", "Tìm video")} />
      )}
      {current && <Preview key={current.id} m={current} project={project} onClipAdded={onClipAdded} />}
      {media && media.length > 0 && shown.length === 0 && <p className="mt-2 text-sm text-zinc-500">{L("No match.", "Không có kết quả.")}</p>}
      {shown.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead className="text-left text-xs text-zinc-500">
              <tr>
                <th /><th className="py-1">File</th><th>{L("Recorded", "Ngày quay")}</th><th>{L("Length", "Độ dài")}</th><th>{L("Resolution", "Độ phân giải")}</th>
                <th className="text-right">{L("Size", "Dung lượng")}</th><th className="pl-3">{L("Analysis", "Phân tích")}</th><th className="pl-3">{L("Speech", "Lời nói")}</th>
                <th className="pl-3">{L("On screen", "Nội dung")}</th><th />
              </tr>
            </thead>
            <tbody>
              {shown.map((m) => (
                <MediaRow key={m.id} m={m} job={job} selected={m.id === selected} onSelect={() => setSelected(m.id === selected ? null : m.id)}
                  onRemoved={() => { if (m.id === selected) setSelected(null); load(); }} onError={setError} onChanged={load} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}
