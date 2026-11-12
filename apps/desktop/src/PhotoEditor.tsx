import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  autoPhoto, cancelJob, exportPhoto, importPhotos, listPhotos, onJobUpdated, previewPhoto, recommendSounds, removePhoto, setPhotoEdits, submitJob,
  type ExportOptions, type ExportPreset, type ExportResult, type Job, type Photo, type PhotoEdits,
} from "./api";
import { dateTime, L } from "./i18n";
import { clock, LookAndSound, PRESETS, Report } from "./Timeline";
import { btn, Card, field, fileName, Progress, Thumb } from "./ui";

/** Offered in the file picker (src-tauri photos::EXTENSIONS). */
const PHOTO_EXTENSIONS = ["jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp"];

type Slider = [key: string, label: string, min?: number, max?: number];
const GROUPS: [string, Slider[]][] = [
  [L("Light", "Ánh sáng"), [["exposure", L("Exposure", "Phơi sáng")], ["contrast", L("Contrast", "Tương phản")], ["highlights", L("Highlights", "Vùng sáng")],
    ["shadows", L("Shadows", "Vùng tối")], ["whites", L("Whites", "Điểm trắng")], ["blacks", L("Blacks", "Điểm đen")]]],
  [L("Colour", "Màu sắc"), [["temperature", L("Temperature", "Nhiệt độ màu")], ["tint", L("Tint", "Sắc thái")], ["vibrance", L("Vibrance", "Độ tươi")],
    ["saturation", L("Saturation", "Độ bão hoà")]]],
  [L("Sky", "Bầu trời"), [["sky", L("Enhance the sky", "Làm đẹp bầu trời"), 0, 100], ["skyWarmth", L("Sky cool ↔ warm", "Trời lạnh ↔ ấm")]]],
  [L("Detail", "Chi tiết"), [["clarity", L("Clarity", "Độ rõ")], ["dehaze", L("Dehaze", "Khử mờ sương")], ["sharpen", L("Sharpen (edges only)", "Làm nét (chỉ đường viền)"), 0, 100],
    ["denoise", L("Noise reduction", "Khử nhiễu"), 0, 100]]],
  [L("Effects & frame", "Hiệu ứng & khung"), [["vignette", L("Vignette", "Viền tối")], ["straighten", L("Straighten (°)", "Cân thẳng (°)"), -15, 15]]],
];
const LOOKS: [string, string][] = [
  ["none", L("No look", "Không")], ["warm", L("Warm", "Ấm")], ["cool", L("Cool", "Lạnh")], ["vivid", L("Vivid", "Rực rỡ")],
  ["cinematic", L("Cinematic", "Điện ảnh")], ["vintage", L("Vintage", "Cổ điển")], ["bw", L("Black & white", "Đen trắng")],
];
const BANDS: [string, string][] = [
  ["red", L("Red", "Đỏ")], ["orange", L("Orange", "Cam")], ["yellow", L("Yellow", "Vàng")], ["green", L("Green", "Xanh lá")],
  ["aqua", L("Aqua", "Xanh ngọc")], ["blue", L("Blue", "Xanh dương")], ["purple", L("Purple", "Tím")], ["magenta", L("Magenta", "Hồng tím")],
];
const CROPS: [string, string][] = [["free", L("Original", "Gốc")], ["1:1", "1:1"], ["4:5", "4:5"], ["3:2", "3:2"], ["16:9", "16:9"], ["9:16", "9:16"]];

const num = (e: PhotoEdits, k: string) => (typeof e[k] === "number" ? (e[k] as number) : 0);

/** Live preview without flooding the photo worker: one request in flight, the latest edits win. */
function usePreview(id: number | null, edits: PhotoEdits, showMask: boolean, onError: (e: string) => void) {
  const [url, setUrl] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const want = useRef<{ id: number; edits: PhotoEdits; showMask: boolean } | null>(null);
  const running = useRef(false);
  const pump = useCallback(async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    while (want.current) {
      const w = want.current;
      want.current = null;
      try {
        const path = await previewPhoto(w.id, w.edits, w.showMask, false);
        setUrl(convertFileSrc(path));
      } catch (e) {
        onError(String(e));
      }
    }
    running.current = false;
    setBusy(false);
  }, [onError]);
  useEffect(() => {
    if (id === null) return;
    want.current = { id, edits, showMask };
    const t = setTimeout(pump, 80);
    return () => clearTimeout(t);
  }, [id, edits, showMask, pump]);
  return { url, busy };
}

function Editor({ photo, onChanged }: { photo: Photo; onChanged: () => void }) {
  const [edits, setEdits] = useState<PhotoEdits>(photo.edits ?? {});
  const [history, setHistory] = useState<PhotoEdits[]>([]);
  const [showMask, setShowMask] = useState(false);
  const [before, setBefore] = useState<string | null>(null);
  const [showBefore, setShowBefore] = useState(false);
  const [band, setBand] = useState("blue");
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [working, setWorking] = useState<string | null>(null);
  const onError = useCallback((e: string) => setMessage({ ok: false, text: e }), []);
  const { url, busy } = usePreview(photo.id, edits, showMask, onError);
  const saved = useRef(JSON.stringify(photo.edits ?? {}));

  // Save shortly after the last change; each save is one undo step.
  useEffect(() => {
    const t = setTimeout(() => {
      const json = JSON.stringify(edits);
      if (json === saved.current) return;
      setHistory((h) => [...h.slice(-49), JSON.parse(saved.current)]);
      saved.current = json;
      setPhotoEdits(photo.id, edits).then(onChanged).catch((e) => onError(String(e)));
    }, 500);
    return () => clearTimeout(t);
  }, [edits, photo.id, onChanged, onError]);

  async function holdBefore(on: boolean) {
    setShowBefore(on);
    if (on && !before) {
      try {
        setBefore(convertFileSrc(await previewPhoto(photo.id, {}, false, true)));
      } catch (e) {
        onError(String(e));
      }
    }
  }
  const set = (k: string, v: number | string | Record<string, number[]>) => setEdits((e) => ({ ...e, [k]: v }));
  const hsl = (edits.hsl ?? {}) as Record<string, number[]>;
  const setHsl = (i: number, v: number) => {
    const cur = [...(hsl[band] ?? [0, 0, 0])];
    cur[i] = v;
    set("hsl", { ...hsl, [band]: cur });
  };
  async function run(label: string, f: () => Promise<void>) {
    setWorking(label);
    setMessage(null);
    try {
      await f();
    } catch (e) {
      onError(String(e));
    } finally {
      setWorking(null);
    }
  }
  const undo = () => {
    const prev = history[history.length - 1];
    if (!prev) return;
    setHistory((h) => h.slice(0, -1));
    saved.current = JSON.stringify(prev);
    setEdits(prev);
    setPhotoEdits(photo.id, prev).then(onChanged).catch((e) => onError(String(e)));
  };
  const range = (k: string, label: string, min = -100, max = 100) => (
    <label key={k} className="block text-xs text-zinc-500" onDoubleClick={() => set(k, 0)} title={L("Double-click to reset", "Bấm đúp để đặt lại")}>
      <span className="flex justify-between"><span>{label}</span><span className="font-mono">{num(edits, k)}</span></span>
      <input type="range" className="w-full accent-indigo-600" min={min} max={max} step={1} value={num(edits, k)} onChange={(e) => set(k, Number(e.target.value))} />
    </label>
  );

  return (
    <div className="flex min-h-0 flex-col gap-4 lg:flex-row">
      <div className="min-w-0 flex-1">
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <button className={btn.primary} disabled={!!working} onClick={() => run(L("Analysing…", "Đang phân tích…"), async () => setEdits({ ...edits, ...(await autoPhoto(photo.id)) }))}
            title={L("Suggested light, colour, detail and sky values; adjust them after", "Gợi ý ánh sáng, màu, chi tiết, bầu trời; chỉnh lại sau")}>
            ✦ {L("Auto", "Tự động")}
          </button>
          <button className={btn.secondary} disabled={history.length === 0} onClick={undo}>↶ {L("Undo", "Hoàn tác")}</button>
          <button className={btn.secondary} onClick={() => setEdits({})}>{L("Reset", "Đặt lại")}</button>
          <button className={btn.secondary} onPointerDown={() => holdBefore(true)} onPointerUp={() => holdBefore(false)} onPointerLeave={() => showBefore && holdBefore(false)}>
            {L("Hold: original", "Giữ: xem ảnh gốc")}
          </button>
          <label className="flex items-center gap-1 text-xs text-zinc-500">
            <input type="checkbox" checked={showMask} onChange={(e) => setShowMask(e.target.checked)} /> {L("Show sky area", "Hiện vùng trời")}
          </label>
          <button className={`${btn.primary} ml-auto`} disabled={!!working}
            onClick={() => run(L("Exporting full size…", "Đang xuất ảnh cỡ gốc…"), async () => {
              await setPhotoEdits(photo.id, edits);
              const path = await exportPhoto(photo.id);
              setMessage({ ok: true, text: path });
            })}>
            {L("Export photo", "Xuất ảnh")}
          </button>
        </div>
        <div className="relative flex min-h-[24rem] items-center justify-center rounded-xl bg-zinc-950">
          {(showBefore ? before : url) ? (
            <img className="max-h-[65vh] max-w-full object-contain" src={(showBefore ? before : url)!} alt="" />
          ) : (
            <p className="text-sm text-zinc-400">{L("Preparing preview…", "Đang chuẩn bị xem trước…")}</p>
          )}
          {showBefore && <span className="absolute left-3 top-3 rounded bg-black/70 px-2 py-0.5 text-xs text-white">{L("Original", "Ảnh gốc")}</span>}
          {(busy || working) && <span className="absolute right-3 top-3 rounded bg-black/70 px-2 py-0.5 text-xs text-white">{working ?? L("Updating…", "Đang cập nhật…")}</span>}
        </div>
        <p className="mt-1 truncate text-xs text-zinc-500" title={photo.path}>
          {fileName(photo.path)} · {photo.width}×{photo.height} · {L("the original is never changed; exports go to the data folder's photos\\exports", "ảnh gốc không bao giờ bị sửa; ảnh xuất nằm trong thư mục dữ liệu photos\\exports")}
        </p>
        {message && (
          <p className={`mt-1 flex flex-wrap items-center gap-2 text-sm ${message.ok ? "text-green-600" : "text-red-600"}`}>
            {message.ok ? `✓ ${L("Saved", "Đã lưu")} ${message.text}` : message.text}
            {message.ok && <button className={btn.link} onClick={() => navigator.clipboard.writeText(message.text).catch(() => {})}>{L("copy path", "chép đường dẫn")}</button>}
          </p>
        )}
      </div>
      <div className="w-full shrink-0 space-y-4 overflow-y-auto lg:max-h-[75vh] lg:w-72">
        {GROUPS.map(([title, sliders]) => (
          <div key={title}>
            <p className="mb-1 text-xs font-semibold uppercase tracking-wide text-zinc-500">{title}</p>
            {sliders.map(([k, label, min, max]) => range(k, label, min, max))}
            {title === GROUPS[1][0] && (
              <>
                <label className="mt-1 flex items-center justify-between text-xs text-zinc-500">{L("Look", "Tông màu")}
                  <select className={field} value={(edits.look as string) ?? "none"} onChange={(e) => set("look", e.target.value)}>
                    {LOOKS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
                  </select>
                </label>
                <details className="mt-2 text-xs text-zinc-500">
                  <summary className="cursor-pointer">{L("One colour (HSL)", "Từng màu (HSL)")}</summary>
                  <div className="mt-1 flex flex-wrap gap-1">
                    {BANDS.map(([id, name]) => (
                      <button key={id} className={`rounded-full border px-2 ${band === id ? "border-indigo-500 text-indigo-600" : "border-zinc-300 dark:border-zinc-700"}`} onClick={() => setBand(id)}>{name}</button>
                    ))}
                  </div>
                  {[L("Hue", "Sắc độ"), L("Saturation", "Bão hoà"), L("Lightness", "Độ sáng")].map((name, i) => (
                    <label key={name} className="block">
                      <span className="flex justify-between"><span>{name}</span><span className="font-mono">{hsl[band]?.[i] ?? 0}</span></span>
                      <input type="range" className="w-full accent-indigo-600" min={-100} max={100} value={hsl[band]?.[i] ?? 0} onChange={(e) => setHsl(i, Number(e.target.value))} />
                    </label>
                  ))}
                </details>
              </>
            )}
            {title === GROUPS[2][0] && (
              <p className="text-xs text-zinc-500">{L("Brings out the photo's own sky (clouds, depth, colour); nothing is pasted in. Tick “Show sky area” to check what counts as sky.",
                "Làm nổi chính bầu trời trong ảnh (vân mây, chiều sâu, màu); không ghép trời khác. Tick “Hiện vùng trời” để xem vùng được coi là trời.")}</p>
            )}
            {title === GROUPS[4][0] && (
              <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-zinc-500">
                <button className={btn.secondary} onClick={() => set("rotate", (num(edits, "rotate") + 270) % 360)} aria-label={L("Rotate left", "Xoay trái")}>⟲</button>
                <button className={btn.secondary} onClick={() => set("rotate", (num(edits, "rotate") + 90) % 360)} aria-label={L("Rotate right", "Xoay phải")}>⟳</button>
                <label className="flex items-center gap-1">{L("Crop", "Cắt")}
                  <select className={field} value={(edits.crop as string) ?? "free"} onChange={(e) => set("crop", e.target.value)}>
                    {CROPS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
                  </select>
                </label>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

const VIDEO_KEY = "memora.photoVideo";
const PHOTO_SECONDS = [2, 3, 4, 5, 6, 8, 10];
const VIDEO_DEFAULTS: ExportOptions = {
  fill: false, transitionMs: 800, transition: "fade", color: "none", fade: true, originalVolume: 0, music: null,
  titles: { title: "", subtitle: "", ending: "", captions: false }, normalize: true,
};
type VideoSettings = { options: ExportOptions; photoMs: number; preset: ExportPreset; name: string };

function loadVideoSettings(): VideoSettings | null {
  try {
    const v = JSON.parse(localStorage.getItem(VIDEO_KEY) ?? "null");
    return v && v.options ? { ...v, options: { ...VIDEO_DEFAULTS, ...v.options } } : null;
  } catch {
    return null;
  }
}

/** Video from photos: the picked photos in order (with their edits), each shown `photoMs`, joined by the chosen transition,
 *  with music and titles — job `media.photo_video`, same export pipeline as projects. */
function PhotoVideo({ photos, picked, setPicked }: { photos: Photo[]; picked: number[]; setPicked: (ids: number[]) => void }) {
  const stored = useState(loadVideoSettings)[0];
  const [s, setS] = useState<VideoSettings>(stored ?? { options: VIDEO_DEFAULTS, photoMs: 3000, preset: "landscape", name: "" });
  const [job, setJob] = useState<Job | null>(null);
  const [error, setError] = useState<string | null>(null);
  const byId = new Map(photos.map((p) => [p.id, p]));
  const items = picked.map((id) => byId.get(id)).filter((p): p is Photo => !!p);
  const join = items.length > 1 ? Math.min(s.options.transitionMs, s.photoMs / 2) : 0;
  const total = items.length * s.photoMs - Math.max(0, items.length - 1) * join;

  useEffect(() => { try { localStorage.setItem(VIDEO_KEY, JSON.stringify(s)); } catch { /* convenience only */ } }, [s]);
  // First time: suggest a library track that suits a photo video (the user can change or remove it).
  useEffect(() => {
    if (stored) return;
    recommendSounds(null, 60000)
      .then((r) => r[0] && setS((x) => (x.options.music ? x : { ...x, options: { ...x.options, music: { path: "", library: r[0].id, volume: 1, duck: false } } })))
      .catch(() => {});
  }, [stored]);
  useEffect(() => {
    const unlisten = onJobUpdated((j) => setJob((cur) => (cur && cur.id === j.id ? j : cur)));
    return () => { unlisten.then((f) => f()); };
  }, []);

  const move = (i: number, d: number) => {
    const next = [...picked];
    [next[i], next[i + d]] = [next[i + d], next[i]];
    setPicked(next);
  };
  const byDate = (ids: number[]) => [...ids].sort((a, b) => (byId.get(a)?.takenAt ?? "").localeCompare(byId.get(b)?.takenAt ?? "") || a - b);
  const running = job && (job.status === "queued" || job.status === "running");
  const result = job?.status === "completed" ? (job.result as ExportResult) : null;

  return (
    <div>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="font-medium">{L(`${items.length} photos`, `${items.length} ảnh`)} · {clock(total)}</span>
        <span className="text-zinc-500">{L("Click photos above to add or remove them; the video follows this order.", "Bấm ảnh ở trên để thêm / bỏ; video theo đúng thứ tự này.")}</span>
        <button className={btn.link} onClick={() => setPicked(byDate(photos.map((p) => p.id)))}>{L("All photos", "Tất cả ảnh")}</button>
        <button className={btn.link} onClick={() => setPicked(byDate(picked))}>{L("Sort by date taken", "Xếp theo ngày chụp")}</button>
        <button className={btn.link} onClick={() => setPicked([])}>{L("Clear", "Bỏ chọn hết")}</button>
      </div>
      {/* Timeline: one block per photo, the transition between them. */}
      <ol className="mt-2 flex items-center overflow-x-auto rounded-xl border border-zinc-200 p-2 dark:border-zinc-800">
        {items.length === 0 && <li className="p-4 text-sm text-zinc-500">{L("No photos picked yet.", "Chưa chọn ảnh nào.")}</li>}
        {items.map((p, i) => (
          <li key={p.id} className="flex shrink-0 items-center">
            {i > 0 && (
              <span className="mx-1 rounded bg-indigo-50 px-1 text-[10px] text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300" title={s.options.transition}>
                {join > 0 ? `⟷ ${(join / 1000).toFixed(1)}s` : "|"}
              </span>
            )}
            <div className="group relative w-24">
              <Thumb path={p.thumb} className="aspect-square w-full rounded-lg" />
              <span className="absolute left-1 top-1 rounded bg-black/70 px-1 text-[10px] text-white">{i + 1} · {s.photoMs / 1000}s</span>
              <span className="absolute inset-x-0 bottom-0 hidden justify-center gap-2 rounded-b-lg bg-black/70 py-0.5 text-xs text-white group-hover:flex">
                <button disabled={i === 0} onClick={() => move(i, -1)} aria-label={L("Earlier", "Lên trước")}>◀</button>
                <button onClick={() => setPicked(picked.filter((x) => x !== p.id))} aria-label={L("Leave out", "Bỏ ra")}>✕</button>
                <button disabled={i === items.length - 1} onClick={() => move(i, 1)} aria-label={L("Later", "Ra sau")}>▶</button>
              </span>
            </div>
          </li>
        ))}
      </ol>
      <div className="mt-3 flex flex-wrap items-end gap-3 text-xs text-zinc-500">
        <label className="flex flex-col gap-1">{L("Each photo", "Mỗi ảnh")}
          <select className={field} value={s.photoMs} onChange={(e) => setS({ ...s, photoMs: Number(e.target.value) })}>
            {PHOTO_SECONDS.map((n) => <option key={n} value={n * 1000}>{n} s</option>)}
          </select>
        </label>
        <label className="flex flex-col gap-1">{L("Format", "Khổ video")}
          <select className={field} value={s.preset} onChange={(e) => setS({ ...s, preset: e.target.value as ExportPreset })}>
            {PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
          </select>
        </label>
        <label className="flex flex-col gap-1">{L("File name", "Tên file")}
          <input className={field} value={s.name} maxLength={80} placeholder={L("Photo video", "Video ảnh")} onChange={(e) => setS({ ...s, name: e.target.value })} />
        </label>
        <button className={btn.primary} disabled={items.length === 0 || !!running}
          onClick={() => {
            setError(null);
            submitJob("media.photo_video", { photoIds: items.map((p) => p.id), photoMs: s.photoMs, preset: s.preset, options: s.options, name: s.name || L("Photo video", "Video ảnh") })
              .then(setJob).catch((e) => setError(String(e)));
          }}>
          🎬 {L("Create video", "Tạo video")}
        </button>
      </div>
      <p className="mt-1 text-xs text-zinc-500">{L("Each photo is used with its edits from the editor. The photo files are never changed.", "Mỗi ảnh dùng bản đã chỉnh trong trình chỉnh ảnh. File ảnh gốc không bị thay đổi.")}</p>
      <LookAndSound o={s.options} projectId={null} videoMs={total} photos version={0} onChange={(o) => setS({ ...s, options: o })} onError={setError} />
      {running && (
        <div className="mt-3 text-sm">
          {job!.status === "queued" ? L("Waiting for other jobs…", "Đang chờ việc khác…") : job!.done < 200 ? L("Preparing the photos…", "Đang chuẩn bị ảnh…") : L("Rendering the video…", "Đang dựng video…")}{" "}
          {job!.total > 0 && `${Math.round((job!.done / job!.total) * 100)}%`}{" "}
          <button className={btn.link} onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>{L("Cancel", "Huỷ")}</button>
          <div className="mt-1"><Progress value={job!.total ? job!.done / job!.total : 0} /></div>
        </div>
      )}
      {job?.status === "failed" && <p className="mt-2 text-sm text-red-600">{job.error}</p>}
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {result && <Report r={result} />}
    </div>
  );
}

/** Photos page: library of photos (read in place) and the editor (ADR 0006, slice B1 — classical tools, no model). */
export default function PhotoEditor() {
  const [photos, setPhotos] = useState<Photo[] | null>(null);
  // Opened from a timeline clip ("Edit photo"): that photo first.
  const [selected, setSelected] = useState<number | null>(() => Number(localStorage.getItem("memora.photo")) || null);
  const [importing, setImporting] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<number | null>(null);
  const [video, setVideo] = useState(false);
  const [picked, setPicked] = useState<number[]>([]);
  const load = useCallback(() => { listPhotos().then(setPhotos).catch((e) => setMessage(String(e))); }, []);
  useEffect(load, [load]);
  // Photos are analysed in the background (quality + content) so Smart edit can place them between video clips.
  const analysing = useRef(false);
  useEffect(() => {
    const ids = photos?.filter((p) => !p.analyzed).map((p) => p.id) ?? [];
    if (ids.length === 0 || analysing.current) return;
    analysing.current = true;
    submitJob("media.photo_analyze", { ids }).catch(() => { analysing.current = false; });
  }, [photos]);
  useEffect(() => {
    const unlisten = onJobUpdated((j) => {
      if (j.kind === "media.photo_analyze" && (j.status === "completed" || j.status === "failed" || j.status === "cancelled")) {
        load();
        if (j.status !== "completed") return; // failed / cancelled: retried on the next visit, not in a loop
        analysing.current = false;
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, [load]);

  async function add() {
    const picked = await open({ multiple: true, title: L("Add photos", "Thêm ảnh"), filters: [{ name: "Photos", extensions: PHOTO_EXTENSIONS }] });
    if (!picked || picked.length === 0) return;
    setImporting(true);
    setMessage(null);
    try {
      const r = await importPhotos(picked);
      load();
      if (r.ids.length) setSelected(r.ids[0]);
      setMessage(r.failed.length ? r.failed.map((f) => `${fileName(f.path)}: ${f.error}`).join(" · ") : null);
    } catch (e) {
      setMessage(String(e));
    } finally {
      setImporting(false);
    }
  }
  const current = photos?.find((p) => p.id === selected) ?? null;

  return (
    <Card title={L("Photos", "Ảnh")} actions={
      <span className="flex gap-2">
        {photos && photos.length > 0 && (
          <button className={video ? btn.primary : btn.secondary} onClick={() => setVideo(!video)}>
            {video ? L("← Back to editing", "← Quay lại chỉnh ảnh") : L("🎬 Make a video", "🎬 Tạo video từ ảnh")}
          </button>
        )}
        <button className={video ? btn.secondary : btn.primary} disabled={importing} onClick={add}>{importing ? L("Adding…", "Đang thêm…") : L("+ Add photos", "+ Thêm ảnh")}</button>
      </span>
    }>
      {message && <p className="mb-2 text-sm text-amber-600">{message}</p>}
      {photos?.length === 0 && (
        <div className="rounded-xl border border-dashed border-zinc-300 p-8 text-center dark:border-zinc-700">
          <p className="text-sm text-zinc-500">{L("No photos yet. Photos are read in place and never changed; edits are saved separately and can be undone.",
            "Chưa có ảnh. Ảnh được đọc tại chỗ và không bao giờ bị sửa; các chỉnh sửa lưu riêng và hoàn tác được.")}</p>
          <button className={`${btn.primary} mt-3`} onClick={add}>{L("+ Add photos", "+ Thêm ảnh")}</button>
        </div>
      )}
      {photos && photos.length > 0 && (
        <div className="flex flex-col gap-4">
          <ul className="flex gap-2 overflow-x-auto pb-1">
            {photos.map((p) => (
              <li key={p.id} className="group relative w-28 shrink-0">
                <button className={`block w-full overflow-hidden rounded-lg border-2 ${(video ? picked.includes(p.id) : p.id === selected) ? "border-indigo-500" : "border-transparent"}`}
                  onClick={() => (video ? setPicked(picked.includes(p.id) ? picked.filter((x) => x !== p.id) : [...picked, p.id]) : setSelected(p.id))}
                  title={`${fileName(p.path)}${p.takenAt ? ` · ${dateTime(new Date(p.takenAt))}` : ""}`}>
                  <Thumb path={p.thumb} className="aspect-square w-full" />
                  {p.edits && Object.keys(p.edits).length > 0 && <span className="absolute bottom-1 left-1 rounded bg-indigo-600 px-1 text-[10px] text-white">✎</span>}
                  {video && picked.includes(p.id) && <span className="absolute left-1 top-1 rounded-full bg-indigo-600 px-1.5 text-xs text-white">{picked.indexOf(p.id) + 1}</span>}
                </button>
                {confirming === p.id ? (
                  <span className="absolute inset-x-0 bottom-0 flex justify-center gap-2 rounded-b-lg bg-black/80 py-1 text-xs">
                    <button className="text-red-400" onClick={() => removePhoto(p.id).then(() => { setConfirming(null); if (selected === p.id) setSelected(null); setPicked((x) => x.filter((i) => i !== p.id)); load(); }).catch((e) => setMessage(String(e)))}>{L("Remove", "Gỡ")}</button>
                    <button className="text-zinc-300" onClick={() => setConfirming(null)}>{L("Keep", "Giữ")}</button>
                  </span>
                ) : (
                  <button className="absolute right-1 top-1 hidden rounded bg-black/60 px-1 text-xs text-white group-hover:block" onClick={() => setConfirming(p.id)}
                    title={L("Remove from the library (the file stays on disk)", "Gỡ khỏi thư viện (file vẫn còn trên ổ đĩa)")}>✕</button>
                )}
              </li>
            ))}
          </ul>
          <div className="min-w-0 flex-1">
            {video ? <PhotoVideo photos={photos} picked={picked} setPicked={setPicked} /> : current ? <Editor key={current.id} photo={current} onChanged={load} /> : (
              <p className="p-8 text-center text-sm text-zinc-500">{L("Pick a photo on the left to edit it.", "Chọn một ảnh bên trái để chỉnh.")}</p>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}
