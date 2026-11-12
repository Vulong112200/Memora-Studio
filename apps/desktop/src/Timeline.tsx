import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addPhotoClips, cancelJob, getExportOptions, listClips, listJoins, listMedia, listPhotos, listSounds, moveClip, onJobUpdated, previewJoin, recommendSounds,
  removeClip, setClipCaption, setClipRange, setClipTransition, setExportOptions, submitJob,
  type Clip, type ExportOptions, type ExportPreset, type ExportResult, type Job, type Join, type Media, type Photo, type Project, type SoundRecommendation, type Track,
} from "./api";
import { L } from "./i18n";
import { btn, Card, field, fileName, Progress, Thumb } from "./ui";

export const PRESETS: { id: ExportPreset; label: string }[] = [
  { id: "landscape", label: L("Landscape 16:9 · 1920×1080", "Ngang 16:9 · 1920×1080") },
  { id: "story", label: L("Story 9:16 · 1080×1920", "Story 9:16 · 1080×1920") },
  { id: "reels", label: L("Reels 9:16 · 1080×1920", "Reels 9:16 · 1080×1920") },
  { id: "archive", label: L("Archive · source size and frame rate", "Lưu trữ · giữ kích thước và tốc độ khung gốc") },
];
const TRANSITIONS: [string, string][] = [
  ["auto", L("Automatic (by recording time)", "Tự động (theo thời gian quay)")], ["fade", L("Cross dissolve", "Hoà mờ")],
  ["fadeblack", L("Dip to black", "Mờ qua đen")], ["dissolve", L("Film dissolve (grain)", "Hoà hạt (film)")],
  ["smoothleft", L("Smooth slide", "Trượt mượt")], ["slideleft", L("Slide", "Trượt")], ["wipeleft", L("Wipe", "Quét")], ["circleopen", L("Circle open", "Mở tròn")],
];
const COLORS: [string, string][] = [
  ["none", L("Natural (no grade)", "Tự nhiên")], ["warm", L("Warm", "Ấm")], ["cool", L("Cool", "Lạnh")], ["vivid", L("Vivid", "Rực rỡ")],
  ["cinematic", L("Cinematic (teal & orange)", "Điện ảnh")], ["vintage", L("Vintage", "Cổ điển")], ["bw", L("Black & white", "Đen trắng")],
];
const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"];
const label = "flex items-center gap-2 text-xs text-zinc-500";

export const clock = (ms: number) => {
  const t = ms / 1000;
  return `${Math.floor(t / 60)}:${(t % 60).toFixed(1).padStart(4, "0")}`;
};

export function Report({ r }: { r: ExportResult }) {
  const v = r.validation;
  const [copied, setCopied] = useState(false);
  return (
    <div className={`mt-3 rounded-xl border p-3 text-sm ${v.ok ? "border-green-600/40 bg-green-50 dark:bg-green-950/20" : "border-red-600/60"}`}>
      <p className={`font-medium ${v.ok ? "text-green-700 dark:text-green-400" : "text-red-600"}`}>
        {v.ok ? L("✓ Export finished and passed validation", "✓ Đã xuất xong và qua kiểm tra") : L("Export finished but FAILED validation", "Đã xuất nhưng KHÔNG qua kiểm tra")}
      </p>
      <p className="mt-1 flex flex-wrap items-center gap-2">
        <span className="break-all font-mono text-xs">{r.path}</span>
        <button className={btn.link} onClick={() => navigator.clipboard.writeText(r.path).then(() => setCopied(true)).catch(() => {})}>
          {copied ? L("copied", "đã chép") : L("copy path", "chép đường dẫn")}
        </button>
      </p>
      <p className="mt-1 text-xs text-zinc-500">
        {r.width}×{r.height} · {clock(r.durationMs)} · {L("rendered in", "dựng trong")} {r.renderSeconds} s ({r.encoder})
      </p>
      <ul className="mt-2 text-xs">
        {v.checks.map((c) => <li key={c.name} className={c.ok ? "text-zinc-500" : "text-red-600"}>{c.ok ? "✓" : "✗"} {c.name}: {c.detail}</li>)}
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
      placeholder={L("Caption (optional)", "Chú thích (tuỳ chọn)")} value={text} maxLength={200} aria-label={L("Clip caption", "Chú thích clip")}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => { if (text !== (clip.caption ?? "")) setClipCaption(clip.id, text).then(onSaved).catch((e) => onError(String(e))); }} />
  );
}

const minutes = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.round(ms / 1000) % 60).padStart(2, "0")}`;

/** Free music library: recommendations for this video, mood filter, streamed preview, choose. */
function MusicLibrary({ projectId, videoMs, version, current, onUse, onClose }: {
  projectId: number | null; videoMs?: number; version: number; current: string | null; onUse: (t: Track) => void; onClose: () => void;
}) {
  const [tracks, setTracks] = useState<Track[]>([]);
  const [recs, setRecs] = useState<SoundRecommendation[]>([]);
  const [mood, setMood] = useState("all");
  const [playing, setPlaying] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement>(null);
  useEffect(() => { listSounds().then(setTracks).catch((e) => setError(String(e))); }, []);
  useEffect(() => { recommendSounds(projectId, videoMs).then(setRecs).catch((e) => setError(String(e))); }, [projectId, videoMs, version]);
  useEffect(() => () => audio.current?.pause(), []);

  function toggle(t: Track) {
    const a = audio.current!;
    if (playing === t.id) { a.pause(); setPlaying(null); return; }
    a.src = t.url;
    a.play().then(() => setPlaying(t.id)).catch(() => setError(L("Preview needs an internet connection.", "Nghe thử cần kết nối internet.")));
  }
  const moods = ["all", ...Array.from(new Set(tracks.flatMap((t) => t.moods))).sort()];
  const byId = new Map(tracks.map((t) => [t.id, t]));
  const row = (t: Track, why?: string[]) => (
    <li key={t.id} className={`flex items-center gap-2 py-1 text-xs ${current === t.id ? "text-green-600" : ""}`}>
      <button className="w-6 rounded border border-zinc-300 text-center dark:border-zinc-700" onClick={() => toggle(t)} aria-label={`${L("Preview", "Nghe thử")} ${t.title}`}>
        {playing === t.id ? "■" : "▶"}
      </button>
      <span className="min-w-0 flex-1 truncate" title={t.credit}>
        <span className="font-medium">{t.title}</span> <span className="text-zinc-500">· {t.moods.join(", ")} · {t.energy} · {minutes(t.durationMs)}</span>
        {why && <span className="block truncate text-zinc-500">{why.join(" · ")}</span>}
      </span>
      <button className="rounded border border-zinc-300 px-2 dark:border-zinc-700" onClick={() => onUse(t)}>{current === t.id ? L("Chosen", "Đã chọn") : L("Use", "Dùng")}</button>
    </li>
  );
  return (
    <div className="mt-2 rounded-md border border-zinc-200 p-2 dark:border-zinc-800">
      <audio ref={audio} onEnded={() => setPlaying(null)} />
      <div className="flex items-center justify-between">
        <p className="text-xs font-semibold text-zinc-500">{L("Free music library", "Thư viện nhạc miễn phí")} · {tracks.length}</p>
        <button className={btn.link} onClick={onClose}>{L("Close", "Đóng")}</button>
      </div>
      <p className="mt-1 text-xs text-zinc-500">
        {L("Music by Scott Buckley (scottbuckley.com.au) and Kevin MacLeod (incompetech.com), free under CC BY 4.0 — the required credit is added at the end of the video; also put it in the post caption. Preview streams from the internet; a chosen track is downloaded once (checked) and kept in the data folder's music folder.",
          "Nhạc của Scott Buckley (scottbuckley.com.au) và Kevin MacLeod (incompetech.com), miễn phí theo CC BY 4.0 — dòng ghi công bắt buộc được thêm ở cuối video; hãy ghi cả trong phần mô tả bài đăng. Nghe thử cần internet; bài được chọn tải về một lần (có kiểm tra) và giữ trong thư mục music.")}
      </p>
      {recs.length > 0 && (
        <>
          <p className="mt-2 text-xs font-semibold text-zinc-500">{L("Recommended for this video", "Gợi ý cho video này")}</p>
          <ul>{recs.map((r) => byId.get(r.id) && row(byId.get(r.id)!, r.reasons))}</ul>
        </>
      )}
      <div className="mt-2 flex flex-wrap gap-1">
        {moods.map((m) => (
          <button key={m} className={`rounded-full border px-2 text-xs ${mood === m ? "border-zinc-900 dark:border-zinc-100" : "border-zinc-300 text-zinc-500 dark:border-zinc-700"}`}
            onClick={() => setMood(m)}>{m === "all" ? L("all", "tất cả") : m}</button>
        ))}
      </div>
      <ul className="mt-1 max-h-56 overflow-y-auto">{tracks.filter((t) => mood === "all" || t.moods.includes(mood)).map((t) => row(t))}</ul>
      {error && <p className="text-xs text-red-600">{error}</p>}
    </div>
  );
}

/** Look & sound panel (docs/Production.md §6.3): saved per project; `photos` = photo video (no clip sound, speech or captions). */
export function LookAndSound({ o, projectId, videoMs, photos, version, onChange, onError }: {
  o: ExportOptions; projectId: number | null; videoMs?: number; photos?: boolean; version: number; onChange: (o: ExportOptions) => void; onError: (e: string) => void;
}) {
  const [library, setLibrary] = useState(false);
  const [trackNames, setTrackNames] = useState<Map<string, Track>>(new Map());
  useEffect(() => { listSounds().then((t) => setTrackNames(new Map(t.map((x) => [x.id, x])))).catch(() => {}); }, []);
  const set = (change: Partial<ExportOptions>) => onChange({ ...o, ...change });
  const titles = (change: Partial<ExportOptions["titles"]>) => set({ titles: { ...o.titles, ...change } });
  async function pickMusic() {
    try {
      const picked = await open({ multiple: false, title: L("Background music (your own file)", "Nhạc nền (file của bạn)"), filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
      if (typeof picked === "string") set({ music: { path: picked, library: null, volume: o.music?.volume ?? 0.35, duck: o.music?.duck ?? true } });
    } catch (e) {
      onError(String(e));
    }
  }
  const lengths: [number, string][] = [[0, L("Hard cuts", "Cắt thẳng")], [300, L("Short · 0.3 s", "Ngắn · 0.3 s")], [500, L("Medium · 0.5 s", "Vừa · 0.5 s")],
    [700, L("Medium · 0.7 s", "Vừa · 0.7 s")], [800, L("Slow · 0.8 s", "Chậm · 0.8 s")], [1200, L("Very slow · 1.2 s", "Rất chậm · 1.2 s")]];
  return (
    <div className="mt-4 grid gap-4 rounded-xl border border-zinc-200 p-3 md:grid-cols-3 dark:border-zinc-800">
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-zinc-500">{L("Picture", "Hình ảnh")}</p>
        <label className={label}>{L("Transitions", "Chuyển cảnh")}
          <select className={field} value={o.transitionMs} onChange={(e) => set({ transitionMs: Number(e.target.value) })}>
            {lengths.concat(lengths.some(([v]) => v === o.transitionMs) ? [] : [[o.transitionMs, `${o.transitionMs / 1000} s`]])
              .map(([v, name]) => <option key={v} value={v}>{name}</option>)}
          </select>
        </label>
        {o.transitionMs > 0 && (
          <label className={label}>{L("Style", "Kiểu")}
            <select className={field} value={o.transition} onChange={(e) => set({ transition: e.target.value })}>
              {TRANSITIONS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
            </select>
          </label>
        )}
        <label className={label}>{L("Colour", "Màu")}
          <select className={field} value={o.color} onChange={(e) => set({ color: e.target.value })}>
            {COLORS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
          </select>
        </label>
        <label className={label}><input type="checkbox" checked={o.fill} onChange={(e) => set({ fill: e.target.checked })} /> {L("Fill the frame (crop centre, no black bars)", "Lấp đầy khung (cắt giữa, không viền đen)")}</label>
        <label className={label}><input type="checkbox" checked={o.fade} onChange={(e) => set({ fade: e.target.checked })} /> {L("Fade in from / out to black", "Mờ dần đầu / cuối video")}</label>
      </div>
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-zinc-500">{L("Sound", "Âm thanh")}</p>
        {!photos && (
          <label className={label}>{L("Clip sound", "Tiếng gốc")} {Math.round(o.originalVolume * 100)}%
            <input type="range" min={0} max={1.5} step={0.05} value={o.originalVolume} onChange={(e) => set({ originalVolume: Number(e.target.value) })} />
          </label>
        )}
        {o.music ? (
          <>
            {o.music.library ? (
              <p className="text-xs" title={trackNames.get(o.music.library)?.credit}>♪ {trackNames.get(o.music.library)?.title ?? o.music.library} · {L("free library", "thư viện miễn phí")}</p>
            ) : (
              <p className="truncate text-xs" title={o.music.path}>♪ {fileName(o.music.path)} · {L("your file (kept as is)", "file của bạn (giữ nguyên)")}</p>
            )}
            <p className="text-xs">
              <button className={btn.link} onClick={() => setLibrary(true)}>{L("Library…", "Thư viện…")}</button>{" "}
              <button className={btn.link} onClick={pickMusic}>{L("Own file…", "File riêng…")}</button>{" "}
              <button className={btn.link} onClick={() => set({ music: null })}>{L("Remove", "Bỏ nhạc")}</button>
            </p>
            <label className={label}>{L("Music", "Nhạc")} {Math.round(o.music.volume * 100)}%
              <input type="range" min={0} max={1.5} step={0.05} value={o.music.volume} onChange={(e) => set({ music: { ...o.music!, volume: Number(e.target.value) } })} />
            </label>
            {!photos && <label className={label}><input type="checkbox" checked={o.music.duck} onChange={(e) => set({ music: { ...o.music!, duck: e.target.checked } })} /> {L("Lower the music while people talk", "Giảm nhạc khi có người nói")}</label>}
          </>
        ) : (
          <p className="text-xs">
            <button className={btn.link} onClick={() => setLibrary(true)}>{L("+ Free music library…", "+ Thư viện nhạc miễn phí…")}</button>{" "}
            <button className={btn.link} onClick={pickMusic}>{L("+ Your own file…", "+ File nhạc của bạn…")}</button>
          </p>
        )}
        {library && (
          <MusicLibrary projectId={projectId} videoMs={videoMs} version={version} current={o.music?.library ?? null} onClose={() => setLibrary(false)}
            onUse={(t) => set({ music: { path: "", library: t.id, volume: o.music?.volume ?? 0.35, duck: o.music?.duck ?? true } })} />
        )}
        <label className={label}><input type="checkbox" checked={o.normalize} onChange={(e) => set({ normalize: e.target.checked })} /> {L("Even loudness (−16 LUFS, phone / social level)", "Cân âm lượng (−16 LUFS, chuẩn điện thoại / mạng xã hội)")}</label>
      </div>
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-zinc-500">{L("Text", "Chữ")}</p>
        <input className={`${field} w-full`} placeholder={L("Opening title", "Tiêu đề mở đầu")} value={o.titles.title} maxLength={80} onChange={(e) => titles({ title: e.target.value })} aria-label={L("Opening title", "Tiêu đề mở đầu")} />
        <input className={`${field} w-full`} placeholder={L("Subtitle (place, date…)", "Dòng phụ (địa điểm, ngày…)")} value={o.titles.subtitle} maxLength={80} onChange={(e) => titles({ subtitle: e.target.value })} aria-label={L("Subtitle", "Dòng phụ")} />
        <input className={`${field} w-full`} placeholder={L("Ending text", "Chữ kết thúc")} value={o.titles.ending} maxLength={80} onChange={(e) => titles({ ending: e.target.value })} aria-label={L("Ending text", "Chữ kết thúc")} />
        {!photos && (
          <>
            <label className={label}><input type="checkbox" checked={o.titles.captions} onChange={(e) => titles({ captions: e.target.checked })} /> {L("Subtitles from speech (transcribed videos)", "Phụ đề từ lời nói (video đã nhận dạng)")}</label>
            <p className="text-xs text-zinc-500">{L("Per-clip captions: type them next to each clip.", "Chú thích từng clip: gõ bên cạnh mỗi clip.")}</p>
          </>
        )}
      </div>
    </div>
  );
}

/** Seconds offered for a photo on the timeline. */
const PHOTO_SECONDS = [1, 2, 3, 4, 5, 6, 8, 10, 15, 20];
const JOIN_LENGTHS = [0, 300, 500, 800, 1000, 1200, 1500, 2000];
const transitionName = (id: string) => TRANSITIONS.find(([t]) => t === id)?.[1] ?? id;
/** Opens a photo in the Photos page (App listens). */
export const openPhoto = (id: number) => window.dispatchEvent(new CustomEvent("memora:open-photo", { detail: id }));

/** Visual timeline: one block per clip (width follows its length), the transition at every join. Click a join to change
 *  it for that place only or to preview it. */
function Strip({ project, clips, joins, media, photos, preset, onChanged, onError }: {
  project: Project; clips: Clip[]; joins: Join[]; media: Map<number, Media>; photos: Map<number, Photo>; preset: ExportPreset;
  onChanged: () => void; onError: (e: string) => void;
}) {
  const [selected, setSelected] = useState<number | null>(null);
  const [preview, setPreview] = useState<{ index: number; url: string | null; busy: boolean } | null>(null);
  const join = selected !== null ? joins[selected] : undefined;
  const into = selected !== null ? clips[selected + 1] : undefined;
  useEffect(() => { if (selected !== null && selected >= joins.length) setSelected(null); }, [selected, joins.length]);

  function setJoin(style: string | null, ms: number) {
    if (!into) return;
    setPreview(null);
    setClipTransition(into.id, style, ms).then(onChanged).catch((e) => onError(String(e)));
  }
  function play(index: number) {
    setPreview({ index, url: null, busy: true });
    previewJoin(project.id, index, preset)
      .then((path) => setPreview({ index, url: convertFileSrc(path), busy: false }))
      .catch((e) => { setPreview(null); onError(String(e)); });
  }
  return (
    <div className="mb-3">
      <ol className="flex items-stretch overflow-x-auto rounded-xl border border-zinc-200 bg-zinc-50 p-2 dark:border-zinc-800 dark:bg-zinc-950">
        {clips.map((c, i) => {
          const ms = c.outMs - c.inMs;
          const thumb = c.photoId !== null ? photos.get(c.photoId)?.thumb : media.get(c.mediaId ?? -1)?.thumb;
          return (
            <li key={c.id} className="flex shrink-0 items-stretch">
              {i > 0 && (
                <button
                  className={`mx-0.5 flex w-11 flex-col items-center justify-center rounded text-[10px] leading-tight ${selected === i - 1 ? "bg-indigo-600 text-white" : c.transition ? "bg-amber-100 text-amber-800 dark:bg-amber-900/50 dark:text-amber-200" : "bg-zinc-200 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300"}`}
                  onClick={() => setSelected(selected === i - 1 ? null : i - 1)}
                  title={`${transitionName(joins[i - 1]?.[0] ?? "")} · ${((joins[i - 1]?.[1] ?? 0) / 1000).toFixed(1)} s`}>
                  <span>{joins[i - 1]?.[1] ? "⟷" : "|"}</span>
                  <span>{((joins[i - 1]?.[1] ?? 0) / 1000).toFixed(1)}s</span>
                </button>
              )}
              <div className="relative h-16 overflow-hidden rounded" style={{ width: Math.min(260, Math.max(56, (ms / 1000) * 14)) }}
                title={c.photoId !== null ? fileName(photos.get(c.photoId)?.path ?? "") : fileName(media.get(c.mediaId ?? -1)?.path ?? "")}>
                <Thumb path={thumb} className="h-full w-full" />
                <span className="absolute left-0.5 top-0.5 rounded bg-black/70 px-1 text-[10px] text-white">{c.photoId !== null ? "🖼 " : ""}{i + 1}</span>
                <span className="absolute bottom-0.5 right-0.5 rounded bg-black/70 px-1 text-[10px] text-white">{(ms / 1000).toFixed(1)}s</span>
              </div>
            </li>
          );
        })}
      </ol>
      {selected !== null && join && into && (
        <div className="mt-2 flex flex-wrap items-center gap-2 rounded-xl border border-zinc-200 p-2 text-xs dark:border-zinc-800">
          <span className="font-medium">{L(`Join ${selected + 1} → ${selected + 2}`, `Chỗ nối ${selected + 1} → ${selected + 2}`)}:</span>
          <span>{join[1] ? `${transitionName(join[0])} · ${(join[1] / 1000).toFixed(1)} s` : L("hard cut", "cắt thẳng")}</span>
          <span className="text-zinc-500">{into.transition ? L("(set here)", "(chỉnh riêng chỗ này)") : L("(project setting)", "(theo cài đặt chung)")}</span>
          <select className={field} value={into.transition ?? ""} aria-label={L("Transition style", "Kiểu chuyển cảnh")}
            onChange={(e) => setJoin(e.target.value || null, into.transitionMs ?? join[1])}>
            <option value="">{L("Project setting", "Theo cài đặt chung")}</option>
            {TRANSITIONS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
          </select>
          {into.transition && (
            <select className={field} value={into.transitionMs ?? 0} aria-label={L("Transition length", "Độ dài chuyển cảnh")}
              onChange={(e) => setJoin(into.transition, Number(e.target.value))}>
              {JOIN_LENGTHS.map((ms) => <option key={ms} value={ms}>{ms === 0 ? L("Hard cut", "Cắt thẳng") : `${ms / 1000} s`}</option>)}
            </select>
          )}
          <button className={btn.secondary} disabled={preview?.busy} onClick={() => play(selected)}>
            {preview?.busy ? L("Rendering…", "Đang dựng…") : L("▶ Preview this join", "▶ Xem thử chỗ nối")}
          </button>
          {preview?.url && preview.index === selected && (
            <video key={preview.url} className="mt-2 max-h-72 w-full rounded bg-black" src={preview.url} autoPlay controls />
          )}
        </div>
      )}
    </div>
  );
}

/** Pick library photos to append to the timeline (in click order). */
function AddPhotos({ project, onAdded, onClose, onError }: { project: Project; onAdded: () => void; onClose: () => void; onError: (e: string) => void }) {
  const [photos, setPhotos] = useState<Photo[] | null>(null);
  const [picked, setPicked] = useState<number[]>([]);
  const [seconds, setSeconds] = useState(4);
  useEffect(() => { listPhotos().then(setPhotos).catch((e) => onError(String(e))); }, [onError]);
  return (
    <div className="mt-2 rounded-xl border border-zinc-200 p-2 dark:border-zinc-800">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="font-medium">{L("Add photos to the timeline", "Thêm ảnh vào timeline")}</span>
        <span className="text-zinc-500">{L("Click in the order you want; each photo uses its edits from the Photos page.", "Bấm theo thứ tự muốn đặt; mỗi ảnh dùng bản đã chỉnh ở trang Ảnh.")}</span>
        <select className={field} value={seconds} onChange={(e) => setSeconds(Number(e.target.value))} aria-label={L("Seconds per photo", "Số giây mỗi ảnh")}>
          {PHOTO_SECONDS.map((n) => <option key={n} value={n}>{n} s</option>)}
        </select>
        <button className={btn.primary} disabled={picked.length === 0}
          onClick={() => addPhotoClips(project.id, picked, seconds * 1000).then(() => { onAdded(); onClose(); }).catch((e) => onError(String(e)))}>
          {L(`Add ${picked.length} photos`, `Thêm ${picked.length} ảnh`)}
        </button>
        <button className={btn.link} onClick={onClose}>{L("Close", "Đóng")}</button>
      </div>
      {photos?.length === 0 && <p className="mt-2 text-xs text-zinc-500">{L("No photos yet: add them on the Photos page first.", "Chưa có ảnh: thêm ảnh ở trang Ảnh trước.")}</p>}
      <ul className="mt-2 flex max-h-48 flex-wrap gap-1 overflow-y-auto">
        {photos?.map((p) => (
          <li key={p.id}>
            <button className={`relative block overflow-hidden rounded border-2 ${picked.includes(p.id) ? "border-indigo-500" : "border-transparent"}`}
              onClick={() => setPicked(picked.includes(p.id) ? picked.filter((x) => x !== p.id) : [...picked, p.id])} title={fileName(p.path)}>
              <Thumb path={p.thumb} className="h-16 w-16" />
              {picked.includes(p.id) && <span className="absolute left-0.5 top-0.5 rounded-full bg-indigo-600 px-1 text-[10px] text-white">{picked.indexOf(p.id) + 1}</span>}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function Timeline({ project, version }: { project: Project; version: number }) {
  const [clips, setClips] = useState<Clip[]>([]);
  const [media, setMedia] = useState<Map<number, Media>>(new Map());
  const [photos, setPhotos] = useState<Map<number, Photo>>(new Map());
  const [joins, setJoins] = useState<Join[]>([]);
  const [adding, setAdding] = useState(false);
  const [preset, setPreset] = useState<ExportPreset>("landscape");
  const [options, setOptions] = useState<ExportOptions | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    listClips(project.id).then(setClips).catch((e) => setError(String(e)));
    listJoins(project.id).then(setJoins).catch((e) => setError(String(e)));
    listMedia().then((m) => setMedia(new Map(m.map((x) => [x.id, x])))).catch((e) => setError(String(e)));
    listPhotos().then((p) => setPhotos(new Map(p.map((x) => [x.id, x])))).catch((e) => setError(String(e)));
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
    setExportOptions(project.id, o).then(() => { setError(null); listJoins(project.id).then(setJoins).catch(() => {}); }).catch((e) => setError(String(e)));
  }

  const run = (p: Promise<unknown>) => p.then(load).catch((e) => setError(String(e)));
  const exporting = job && (job.status === "queued" || job.status === "running");
  // Same joins as the export (project transition, per-clip choices, clamped by the neighbours).
  const total = clips.reduce((s, c) => s + c.outMs - c.inMs, 0) - joins.reduce((s, j) => s + j[1], 0);
  const result = job?.status === "completed" ? (job.result as ExportResult) : null;

  return (
    <Card title={`${L("Timeline", "Timeline")} · ${project.name}`} actions={
      <>
        <select className={field} value={preset} aria-label={L("Export format", "Định dạng xuất")}
          onChange={(e) => {
            const p = e.target.value as ExportPreset;
            setPreset(p);
            if (options) change({ ...options, fill: p === "story" || p === "reels" });
          }}>
          {PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
        </select>
        <button className={btn.primary} disabled={clips.length === 0 || !!exporting || !options}
          onClick={() => submitJob("media.export", { projectId: project.id, preset, options }).then(setJob).catch((e) => setError(String(e)))}>
          {L("Export video", "Xuất video")}
        </button>
      </>
    }>
      {exporting && (
        <div className="mb-3 text-sm text-zinc-500">
          {job!.status === "queued" ? L("Waiting for other jobs…", "Đang chờ việc khác…") : job!.done < 900 ? L("Rendering from the original files…", "Đang dựng từ file gốc…") : L("Checking the result…", "Đang kiểm tra kết quả…")}{" "}
          {job!.total > 0 && `${Math.round((job!.done / job!.total) * 100)}%`}{" "}
          <button className={btn.link} onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>{L("Cancel", "Huỷ")}</button>
          <div className="mt-1"><Progress value={job!.total ? job!.done / job!.total : 0} /></div>
        </div>
      )}
      {job?.status === "failed" && <p className="mb-2 text-sm text-red-600">{job.error}</p>}
      {job?.status === "cancelled" && <p className="mb-2 text-sm text-zinc-500">{L("Export cancelled. No file was left behind.", "Đã huỷ xuất. Không để lại file dở.")}</p>}
      {result && <Report r={result} />}
      {clips.length > 0 && <Strip project={project} clips={clips} joins={joins} media={media} photos={photos} preset={preset} onChanged={load} onError={setError} />}
      <p className="text-xs">
        <button className={btn.link} onClick={() => setAdding(!adding)}>{L("+ Add photos…", "+ Thêm ảnh…")}</button>
      </p>
      {adding && <AddPhotos project={project} onAdded={load} onClose={() => setAdding(false)} onError={setError} />}
      {clips.length === 0 ? (
        <p className="text-sm text-zinc-500">{L("No clips yet. Use Smart edit (step 2), or select a video in the Library, set In / Out and add it here, or add photos.",
          "Chưa có clip. Dùng Dựng tự động (bước 2), hoặc chọn video trong Thư viện, đặt điểm vào / ra rồi thêm vào đây, hoặc thêm ảnh.")}</p>
      ) : (
        <>
          <ol className="mt-2 max-h-[28rem] divide-y pr-3 divide-zinc-200 overflow-y-auto dark:divide-zinc-800">
            {clips.map((c, i) => {
              const m = c.mediaId !== null ? media.get(c.mediaId) : undefined;
              const ph = c.photoId !== null ? photos.get(c.photoId) : undefined;
              return (
                <li key={c.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5 text-sm">
                  <span className="w-5 text-right text-xs text-zinc-500">{i + 1}</span>
                  <Thumb path={ph ? ph.thumb : m?.thumb} />
                  <span className="min-w-0 flex-1 basis-40 truncate" title={ph?.path ?? m?.path}>
                    {ph ? `🖼 ${fileName(ph.path)}` : m ? fileName(m.path) : `#${c.mediaId ?? c.photoId}`}
                  </span>
                  <CaptionField clip={c} onSaved={load} onError={setError} />
                  {c.photoId !== null ? (
                    <>
                      <button className={btn.link} onClick={() => openPhoto(c.photoId!)}>{L("Edit photo", "Chỉnh ảnh")}</button>
                      <select className={field} value={c.outMs - c.inMs} aria-label={L("Seconds on screen", "Số giây hiện")}
                        onChange={(e) => run(setClipRange(c.id, 0, Number(e.target.value)))}>
                        {PHOTO_SECONDS.concat(PHOTO_SECONDS.includes((c.outMs - c.inMs) / 1000) ? [] : [(c.outMs - c.inMs) / 1000])
                          .map((n) => <option key={n} value={n * 1000}>{n} s</option>)}
                      </select>
                    </>
                  ) : (
                    <span className="font-mono text-xs text-zinc-500">{clock(c.inMs)} → {clock(c.outMs)}</span>
                  )}
                  <span className="w-14 text-right font-mono text-xs">{clock(c.outMs - c.inMs)}</span>
                  <button className={btn.link} disabled={i === 0} onClick={() => run(moveClip(c.id, -1))} aria-label={L("Move earlier", "Lên trước")}>↑</button>
                  <button className={btn.link} disabled={i === clips.length - 1} onClick={() => run(moveClip(c.id, 1))} aria-label={L("Move later", "Xuống sau")}>↓</button>
                  <button className={btn.link} onClick={() => run(removeClip(c.id))}>{L("Remove", "Bỏ")}</button>
                </li>
              );
            })}
          </ol>
          <p className="mt-2 text-right text-xs text-zinc-500">{L("About", "Khoảng")} {clock(total)} · {L("rendered from the original files", "dựng từ file gốc")}</p>
        </>
      )}
      {options && <LookAndSound o={options} projectId={project.id} version={version + clips.length} onChange={change} onError={setError} />}
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
    </Card>
  );
}
