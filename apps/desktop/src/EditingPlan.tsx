import { useEffect, useState } from "react";
import {
  applyPlan, generatePlan, getPlan, interpretRequest, listClips, listMedia, listPhotos, listStyles, submitJob,
  type EditingStyle, type Media, type Photo, type Plan, type Project, type RequestReading,
} from "./api";
import { clock } from "./Timeline";
import { L } from "./i18n";
import { btn, Card, field, fileName, Thumb } from "./ui";

const RANGES = [
  { id: "all", label: L("Whole library", "Cả thư viện") },
  { id: "1", label: L("Last month", "Tháng vừa qua") },
  { id: "6", label: L("Last six months", "Sáu tháng qua") },
  { id: "12", label: L("Last year", "Một năm qua") },
  { id: "custom", label: L("Custom dates…", "Chọn ngày…") },
];
const iso = (d: Date) => d.toISOString().slice(0, 10);
const pct = (x: number) => Math.round(x * 100);
const EXAMPLE = "Video tổng hợp cả chuyến đi, nhiều cảnh đường rừng và qua cầu, bỏ selfie, tông ấm, không nhạc, tiêu đề: Nam Cát Tiên";
const WORDS = L(
  "length (30 giây, 5 phút) · dọc / ngang · nhiều / bớt / bỏ + what is on screen (đường rừng, qua cầu, lên dốc, cây cổ thụ…) · tông ấm, lạnh, rực rỡ, cổ điển, đen trắng · nhạc nhẹ, sôi động, không nhạc · phụ đề · tiêu đề: … · kết thúc bằng: … · tháng 2/2026 · tổng hợp, hoài niệm, tài liệu, tối giản, điện ảnh",
  "độ dài (30 giây, 5 phút) · dọc / ngang · nhiều / bớt / bỏ + nội dung (đường rừng, qua cầu, lên dốc, cây cổ thụ…) · tông ấm, lạnh, rực rỡ, cổ điển, đen trắng · nhạc nhẹ, sôi động, không nhạc · phụ đề · tiêu đề: … · kết thúc bằng: … · tháng 2/2026 · tổng hợp, hoài niệm, tài liệu, tối giản, điện ảnh",
);

/** Free-text request, read offline by keywords (request.rs); shows what it understood before anything is made. */
function RequestBox({ text, onChange, reading }: { text: string; onChange: (t: string) => void; reading: RequestReading | null }) {
  return (
    <div className="mb-3">
      <textarea className={`${field} w-full px-3 py-2`} rows={3} value={text} onChange={(e) => onChange(e.target.value)}
        placeholder={L(`Describe the video you want, e.g. “${EXAMPLE}”`, `Mô tả video bạn muốn, ví dụ “${EXAMPLE}”`)}
        aria-label={L("Describe the video you want", "Mô tả video bạn muốn")} />
      {text.trim() !== "" && reading && (
        reading.understood.length > 0 ? (
          <p className="mt-1 text-xs text-zinc-500">
            {L("Understood:", "Đã hiểu:")} {reading.understood.map((u) => <span key={u} className="mr-1 inline-block rounded bg-zinc-100 px-1.5 py-0.5 dark:bg-zinc-800">{u}</span>)}
          </p>
        ) : (
          <p className="mt-1 text-xs text-amber-600">{L("Nothing recognised yet. Words that work:", "Chưa nhận ra từ nào. Các từ dùng được:")} {WORDS}.</p>
        )
      )}
      {text.trim() === "" && <p className="mt-1 text-xs text-zinc-500">{L("Words that work:", "Các từ dùng được:")} {WORDS}.</p>}
    </div>
  );
}

function range(id: string, from: string, to: string): { from?: string; to?: string } {
  if (id === "custom") return { from: from || undefined, to: to || undefined };
  if (id === "all") return {};
  const d = new Date();
  d.setMonth(d.getMonth() - Number(id));
  return { from: iso(d), to: iso(new Date()) };
}

/** Smart mode (docs/Production.md §6.4): generate an Editing Plan, review it, apply it to the timeline. */
export default function EditingPlan({ project, onApplied }: { project: Project; onApplied: () => void }) {
  const [styles, setStyles] = useState<EditingStyle[]>([]);
  const [styleId, setStyleId] = useState("journey_recap");
  const [rangeId, setRangeId] = useState("all");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [plan, setPlan] = useState<Plan | null>(null);
  const [order, setOrder] = useState<number[]>([]); // indexes into plan.clips the user keeps, in order
  const [media, setMedia] = useState<Map<number, Media>>(new Map());
  const [photos, setPhotos] = useState<Map<number, Photo>>(new Map());
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [request, setRequest] = useState("");
  const [reading, setReading] = useState<RequestReading | null>(null);
  const [replacing, setReplacing] = useState<{ clips: number; thenExport: boolean } | null>(null);

  useEffect(() => {
    listStyles().then(setStyles).catch((e) => setMessage(String(e)));
    listMedia().then((m) => setMedia(new Map(m.map((x) => [x.id, x])))).catch((e) => setMessage(String(e)));
    listPhotos().then((p) => setPhotos(new Map(p.map((x) => [x.id, x])))).catch((e) => setMessage(String(e)));
    getPlan(project.id).then((p) => {
      show(p);
      if (p?.wishes?.request) setRequest(p.wishes.request);
    }).catch((e) => setMessage(String(e)));
  }, [project.id]);

  // Read the request as the user types (cheap, offline); a short pause avoids work on every key.
  useEffect(() => {
    if (request.trim() === "") return setReading(null);
    const t = setTimeout(() => interpretRequest(request).then(setReading).catch((e) => setMessage(String(e))), 250);
    return () => clearTimeout(t);
  }, [request]);

  function show(p: Plan | null) {
    setPlan(p);
    setOrder(p ? p.clips.map((_, i) => i) : []);
    if (p) setStyleId(p.style);
  }

  const style = styles.find((s) => s.id === styleId);

  async function generate() {
    setBusy(true);
    setMessage(null);
    try {
      const r = range(rangeId, from, to);
      show(await generatePlan(project.id, styleId, r.from, r.to, request.trim() || undefined));
      listMedia().then((m) => setMedia(new Map(m.map((x) => [x.id, x]))));
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  }

  /** Applying replaces the timeline: ask first when it already has clips. */
  async function apply(thenExport: boolean, confirmed = false) {
    if (!plan) return;
    try {
      if (!confirmed) {
        const existing = (await listClips(project.id)).length;
        if (existing > 0) return setReplacing({ clips: existing, thenExport });
      }
      setReplacing(null);
      const options = await applyPlan(project.id, order);
      if (thenExport) await submitJob("media.export", { projectId: project.id, preset: plan.exportPreset, options });
      onApplied();
    } catch (e) {
      setMessage(String(e));
    }
  }

  const move = (pos: number, delta: number) => {
    const o = [...order];
    const [x] = o.splice(pos, 1);
    o.splice(pos + delta, 0, x);
    setOrder(o);
  };
  const kept = order.map((i) => plan!.clips[i]);
  const unchanged = plan !== null && order.length === plan.clips.length && order.every((x, i) => x === i);
  // The plan's own total uses the export's transition rules; after changes this is an estimate.
  const total = unchanged ? plan!.totalMs : kept.reduce((s, c) => s + c.outMs - c.inMs, 0) - Math.max(0, kept.length - 1) * (plan?.transitionMs ?? 0);

  return (
    <Card title={`${L("Smart edit", "Dựng tự động")} · ${project.name}`}>
      <RequestBox text={request} onChange={setRequest} reading={reading} />
      {reading?.style && <p className="-mt-2 mb-2 text-xs text-zinc-500">{L("The style and dates named in your request replace the selection below.", "Kiểu dựng và ngày trong câu mô tả sẽ thay cho lựa chọn bên dưới.")}</p>}
      <div className="flex flex-wrap items-center gap-2">
        <select className={field} value={styleId} onChange={(e) => setStyleId(e.target.value)} aria-label={L("Editing style", "Kiểu dựng")}>
          {styles.map((s) => <option key={s.id} value={s.id}>{s.name} · {Math.round(s.targetMs / 1000)} s</option>)}
        </select>
        <select className={field} value={rangeId} onChange={(e) => setRangeId(e.target.value)} aria-label={L("Recording dates", "Ngày quay")}>
          {RANGES.map((r) => <option key={r.id} value={r.id}>{r.label}</option>)}
        </select>
        {rangeId === "custom" && (
          <>
            <input type="date" className={field} value={from} onChange={(e) => setFrom(e.target.value)} aria-label={L("From", "Từ")} />
            <input type="date" className={field} value={to} onChange={(e) => setTo(e.target.value)} aria-label={L("To", "Đến")} />
          </>
        )}
        <button className={btn.primary} disabled={busy} onClick={generate}>
          {busy ? L("Choosing scenes…", "Đang chọn cảnh…") : plan ? L("Make the plan again", "Tạo lại kế hoạch") : L("Make a plan", "Tạo kế hoạch")}
        </button>
      </div>
      {style && (
        <p className="mt-2 text-xs text-zinc-500">
          {style.pacing}. {L("Clips", "Clip")} {style.clipMinMs / 1000}–{style.clipMaxMs / 1000} s · {style.exportPreset === "reels" ? L("vertical 9:16", "dọc 9:16") : "16:9"} ·{" "}
          {L("opens with", "mở đầu")}: {style.opening.toLowerCase()} · {L("ends with", "kết thúc")}: {style.ending.toLowerCase()}.
        </p>
      )}
      {plan && (
        <div className="mt-4">
          <p className="text-sm">
            <span className="font-medium">{L(`${kept.length} of ${plan.clips.length} clips`, `${kept.length} / ${plan.clips.length} clip`)}</span>{" "}
            · {clock(total)} {L("of", "/")} {clock(plan.targetMs)} · <span className="text-zinc-500">{L(`chosen from ${plan.candidates} candidate windows`, `chọn từ ${plan.candidates} đoạn ứng viên`)}</span>
          </p>
          <p className="text-xs text-zinc-500">{L("Scores: T technical · S what is on screen · V speech · ★ favorite. Videos you exclude in the library are never used.", "Điểm: T kỹ thuật · S nội dung hình · V lời nói · ★ yêu thích. Video đã loại trừ trong thư viện không bao giờ được dùng.")}</p>
          {plan.wishes?.request && <p className="text-xs text-zinc-500">{L("Made from your request:", "Tạo từ mô tả của bạn:")} “{plan.wishes.request}”</p>}
          {(plan.notes ?? []).map((s) => <p key={s} className="text-xs text-indigo-600 dark:text-indigo-400">{s}</p>)}
          {plan.skipped.map((s) => <p key={s} className="text-xs text-amber-600">{s}</p>)}
          {plan.clips.length === 0 && <p className="mt-2 text-sm text-zinc-500">{L("Nothing to use: analyse the videos in the Library first.", "Chưa có gì để dùng: hãy phân tích video trong Thư viện trước.")}</p>}
          <ol className="mt-2 max-h-[28rem] divide-y pr-3 divide-zinc-200 overflow-y-auto dark:divide-zinc-800">
            {order.map((i, pos) => {
              const c = plan.clips[i];
              const ph = c.photoId != null ? photos.get(c.photoId) : undefined;
              const m = c.photoId != null ? undefined : media.get(c.mediaId);
              const file = ph ? fileName(ph.path) : m ? fileName(m.path) : "";
              return (
                <li key={i} className="flex items-center gap-3 py-1.5 text-sm">
                  <span className="w-5 text-right text-xs text-zinc-500">{pos + 1}</span>
                  <Thumb path={ph ? ph.thumb : m?.thumb} />
                  <div className="min-w-0 flex-1">
                    <p className="truncate" title={ph?.path ?? m?.path}>{ph ? "🖼 " : ""}{c.tags[0] ?? (file || `#${c.mediaId}`)} <span className="font-mono text-xs text-zinc-500">{file} {ph ? `${(c.outMs - c.inMs) / 1000} s` : `${clock(c.inMs)}–${clock(c.outMs)}`}</span></p>
                    <p className="truncate text-xs text-zinc-500" title={c.reasons.join(" · ")}>{c.reasons.join(" · ")}</p>
                  </div>
                  <span className="whitespace-nowrap font-mono text-xs text-zinc-500" title={L("technical / on screen / speech / favorite", "kỹ thuật / nội dung / lời nói / yêu thích")}>
                    T{pct(c.scores.technical)} S{pct(c.scores.semantic)} V{pct(c.scores.speech)}{c.scores.favorite ? " ★" : ""}
                  </span>
                  <button className={btn.link} disabled={pos === 0} onClick={() => move(pos, -1)} aria-label={L("Move earlier", "Lên trước")}>↑</button>
                  <button className={btn.link} disabled={pos === order.length - 1} onClick={() => move(pos, 1)} aria-label={L("Move later", "Xuống sau")}>↓</button>
                  <button className={btn.link} onClick={() => setOrder(order.filter((x) => x !== i))}>{L("Drop", "Bỏ")}</button>
                </li>
              );
            })}
          </ol>
          {order.length < plan.clips.length && (
            <button className={`${btn.link} mt-1`} onClick={() => setOrder(plan.clips.map((_, i) => i))}>{L("Restore dropped clips", "Lấy lại các clip đã bỏ")}</button>
          )}
          {replacing ? (
            <div className="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm dark:border-amber-800 dark:bg-amber-950/30">
              <span>{L(`The timeline already has ${replacing.clips} clips. Replace them with this plan?`, `Timeline đang có ${replacing.clips} clip. Thay bằng kế hoạch này?`)}</span>
              <button className={btn.primary} onClick={() => apply(replacing.thenExport, true)}>{L("Replace", "Thay")}</button>
              <button className={btn.secondary} onClick={() => setReplacing(null)}>{L("Cancel", "Huỷ")}</button>
            </div>
          ) : (
            <div className="mt-3 flex flex-wrap gap-2">
              <button className={btn.primary} disabled={order.length === 0} onClick={() => apply(false)}>{L("Use this plan →", "Dùng kế hoạch này →")}</button>
              <button className={btn.secondary} disabled={order.length === 0} onClick={() => apply(true)}
                title={plan.exportPreset === "reels" ? "Reels 9:16" : "Landscape 16:9"}>
                {L("Use and export now", "Dùng và xuất luôn")}
              </button>
            </div>
          )}
        </div>
      )}
      {message && <p className="mt-2 text-sm text-red-600">{message}</p>}
    </Card>
  );
}
