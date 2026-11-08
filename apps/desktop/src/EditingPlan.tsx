import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  applyPlan, generatePlan, getPlan, interpretRequest, listMedia, listStyles, submitJob,
  type EditingStyle, type Media, type Plan, type Project, type RequestReading,
} from "./api";
import { clock } from "./Timeline";

const RANGES = [
  { id: "all", label: "Whole library" },
  { id: "1", label: "Last month" },
  { id: "6", label: "Last six months" },
  { id: "12", label: "Last year" },
  { id: "custom", label: "Custom dates…" },
];
const iso = (d: Date) => d.toISOString().slice(0, 10);
const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;
const pct = (x: number) => Math.round(x * 100);
const buttonClass = "rounded-md border border-zinc-300 px-3 py-1.5 text-sm disabled:opacity-40 dark:border-zinc-700";
const EXAMPLE = "Video dọc 30 giây, nhiều cảnh cây cổ thụ và đường mòn, bớt xe đạp, bỏ selfie, tông ấm, nhạc nhẹ, tiêu đề: Đà Lạt";
const WORDS = "length (30 giây, 1 phút) · dọc / ngang · nhiều / bớt / bỏ + what is on screen · tông ấm, lạnh, rực rỡ, cổ điển, đen trắng · nhạc nhẹ, sôi động, không nhạc · phụ đề · tiêu đề: … · kết thúc bằng: … · tháng 2/2026, năm 2025 · hoài niệm, tài liệu, tối giản, điện ảnh";

/** Free-text request, read offline by keywords (request.rs); shows what it understood before anything is made. */
function RequestBox({ text, onChange, reading }: { text: string; onChange: (t: string) => void; reading: RequestReading | null }) {
  return (
    <div className="mb-3">
      <textarea
        className="w-full rounded-md border border-zinc-300 bg-transparent px-3 py-2 text-sm dark:border-zinc-700"
        rows={2}
        value={text}
        placeholder={`Describe the video you want, e.g. “${EXAMPLE}”`}
        onChange={(e) => onChange(e.target.value)}
        aria-label="Describe the video you want"
      />
      {text.trim() !== "" && reading && (
        reading.understood.length > 0 ? (
          <p className="mt-1 text-xs text-zinc-500">
            Understood: {reading.understood.map((u) => <span key={u} className="mr-1 inline-block rounded bg-zinc-100 px-1.5 py-0.5 dark:bg-zinc-800">{u}</span>)}
          </p>
        ) : (
          <p className="mt-1 text-xs text-amber-600">Nothing recognised yet. Words that work: {WORDS}.</p>
        )
      )}
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
  const [styleId, setStyleId] = useState("travel_cinematic");
  const [rangeId, setRangeId] = useState("all");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [plan, setPlan] = useState<Plan | null>(null);
  const [order, setOrder] = useState<number[]>([]); // indexes into plan.clips the user keeps, in order
  const [media, setMedia] = useState<Map<number, Media>>(new Map());
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [request, setRequest] = useState("");
  const [reading, setReading] = useState<RequestReading | null>(null);

  useEffect(() => {
    listStyles().then(setStyles).catch((e) => setMessage(String(e)));
    listMedia().then((m) => setMedia(new Map(m.map((x) => [x.id, x])))).catch((e) => setMessage(String(e)));
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

  async function apply(thenExport: boolean) {
    if (!plan) return;
    try {
      const options = await applyPlan(project.id, order);
      onApplied();
      if (thenExport) {
        await submitJob("media.export", { projectId: project.id, preset: plan.exportPreset, options });
        setMessage(`Timeline replaced with ${order.length} clips; export started with the style's look (see Timeline).`);
      } else {
        setMessage(`Timeline replaced with ${order.length} clips and the style's look set (${options.color} colour, title "${options.titles.title}"). Adjust anything in Timeline › Look & sound before exporting.`);
      }
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
  const total = kept.reduce((s, c) => s + c.outMs - c.inMs, 0) - Math.max(0, kept.length - 1) * (plan?.transitionMs ?? 0);

  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">Smart edit · {project.name}</h2>
      <RequestBox text={request} onChange={setRequest} reading={reading} />
      {reading?.style && <p className="-mt-2 mb-2 text-xs text-zinc-500">The style and dates named in your request replace the selection below.</p>}
      <div className="flex flex-wrap items-center gap-2">
        <select className="rounded-md border border-zinc-300 bg-transparent px-2 py-1.5 text-sm dark:border-zinc-700" value={styleId}
          onChange={(e) => setStyleId(e.target.value)} aria-label="Editing style">
          {styles.map((s) => <option key={s.id} value={s.id}>{s.name} · {Math.round(s.targetMs / 1000)} s</option>)}
        </select>
        <select className="rounded-md border border-zinc-300 bg-transparent px-2 py-1.5 text-sm dark:border-zinc-700" value={rangeId}
          onChange={(e) => setRangeId(e.target.value)} aria-label="Recording dates">
          {RANGES.map((r) => <option key={r.id} value={r.id}>{r.label}</option>)}
        </select>
        {rangeId === "custom" && (
          <>
            <input type="date" className="rounded-md border border-zinc-300 bg-transparent px-2 py-1 text-sm dark:border-zinc-700" value={from} onChange={(e) => setFrom(e.target.value)} aria-label="From" />
            <input type="date" className="rounded-md border border-zinc-300 bg-transparent px-2 py-1 text-sm dark:border-zinc-700" value={to} onChange={(e) => setTo(e.target.value)} aria-label="To" />
          </>
        )}
        <button className="rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900" disabled={busy} onClick={generate}>
          {plan ? "Generate again" : "Generate plan"}
        </button>
      </div>
      {style && (
        <p className="mt-2 text-xs text-zinc-500">
          {style.pacing}. Clips {style.clipMinMs / 1000}–{style.clipMaxMs / 1000} s · {style.transitionMs ? `${style.transitionMs} ms crossfades` : "hard cuts"} ·
          {" "}{style.exportPreset === "reels" ? "vertical 9:16" : "16:9"} · opens with: {style.opening.toLowerCase()} · ends with: {style.ending.toLowerCase()}.
          {" "}{style.music} {style.titles}
        </p>
      )}
      {plan && (
        <div className="mt-3">
          <p className="text-xs text-zinc-500">
            {kept.length} of {plan.clips.length} clips kept · {clock(total)} of {clock(plan.targetMs)} target · chosen from {plan.candidates} candidate
            windows. Scores: T technical · S visual concepts · V speech · ★ favorite. Videos you exclude in the library are never used.
          </p>
          {plan.wishes?.request && <p className="text-xs text-zinc-500">Made from your request: “{plan.wishes.request}”</p>}
          {plan.skipped.map((s) => <p key={s} className="text-xs text-amber-600">{s}</p>)}
          {plan.clips.length === 0 && <p className="mt-2 text-sm text-zinc-500">Nothing to use: analyse videos in the Media library first (and tag / transcribe them for better choices).</p>}
          <ol className="mt-2 max-h-96 divide-y divide-zinc-200 overflow-y-auto dark:divide-zinc-800">
            {order.map((i, pos) => {
              const c = plan.clips[i];
              const m = media.get(c.mediaId);
              return (
                <li key={i} className="flex items-center gap-3 py-1.5 text-sm">
                  <span className="w-5 text-right text-xs text-zinc-500">{pos + 1}</span>
                  {m?.thumb ? <img className="h-9 w-16 rounded object-cover" src={convertFileSrc(m.thumb)} alt="" /> : <div className="h-9 w-16 rounded bg-zinc-200 dark:bg-zinc-800" />}
                  <div className="min-w-0 flex-1">
                    <p className="truncate" title={m?.path}>{m ? fileName(m.path) : `media #${c.mediaId}`} <span className="font-mono text-xs text-zinc-500">{clock(c.inMs)}–{clock(c.outMs)}</span></p>
                    <p className="truncate text-xs text-zinc-500" title={c.reasons.join(" · ")}>{c.reasons.join(" · ")}</p>
                  </div>
                  <span className="whitespace-nowrap font-mono text-xs text-zinc-500" title="technical / concepts / speech / favorite">
                    T{pct(c.scores.technical)} S{pct(c.scores.semantic)} V{pct(c.scores.speech)}{c.scores.favorite ? " ★" : ""}
                  </span>
                  <button className="text-xs text-zinc-500 hover:underline disabled:opacity-30" disabled={pos === 0} onClick={() => move(pos, -1)} aria-label="Move earlier">↑</button>
                  <button className="text-xs text-zinc-500 hover:underline disabled:opacity-30" disabled={pos === order.length - 1} onClick={() => move(pos, 1)} aria-label="Move later">↓</button>
                  <button className="text-xs text-zinc-500 hover:underline" onClick={() => setOrder(order.filter((x) => x !== i))}>Drop</button>
                </li>
              );
            })}
          </ol>
          {order.length < plan.clips.length && (
            <button className="mt-1 text-xs text-zinc-500 hover:underline" onClick={() => setOrder(plan.clips.map((_, i) => i))}>Restore dropped clips</button>
          )}
          <div className="mt-3 flex flex-wrap gap-2">
            <button className={buttonClass} disabled={order.length === 0} onClick={() => apply(false)}>Apply to timeline</button>
            <button className={buttonClass} disabled={order.length === 0} onClick={() => apply(true)}
              title={`Replaces the timeline and exports with ${plan.exportPreset === "reels" ? "Reels 9:16 (centre crop)" : "Landscape 16:9"}`}>
              Apply and export
            </button>
          </div>
        </div>
      )}
      {message && <p className="mt-2 text-sm text-zinc-500">{message}</p>}
    </section>
  );
}
