import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  backupDatabase, formatBytes, getHardwareInfo, getPlan, getSettings, listClips, listMedia, listModels, listProjects, onJobUpdated, setDataRoot,
  type HardwareInfo, type Media, type ModelInfo, type Project, type SettingsView,
} from "./api";
import Projects from "./Projects";
import Timeline from "./Timeline";
import EditingPlan from "./EditingPlan";
import AiCenter from "./AiCenter";
import MediaLibrary from "./MediaLibrary";
import Models from "./Models";
import PhotoEditor from "./PhotoEditor";
import { L, lang, setLang } from "./i18n";
import { btn, Card, JobBar, prepareMedia, preparation, useJobs, active, field } from "./ui";

/** Warn when less than this much commit charge is left (fixed small pagefile, docs/Production.md §9.2). */
const LOW_COMMIT_BYTES = 8 * 1024 ** 3;

type Page = "library" | "project" | "photos" | "jobs" | "settings";
type Step = 1 | 2 | 3;

/** Per-computer UI conveniences (open page / project); the app works the same without them. */
const remember = (key: string, value: string | null) => {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch { /* storage blocked */ }
};
const recall = (key: string) => {
  try { return localStorage.getItem(key); } catch { return null; }
};

function Row({ label, value, warn }: { label: string; value: ReactNode; warn?: boolean }) {
  return (
    <div className="flex justify-between gap-4 py-1 text-sm">
      <span className="text-zinc-500">{label}</span>
      <span className={warn ? "font-medium text-amber-600" : "text-right"}>{value}</span>
    </div>
  );
}

function DataFolder({ settings, onSaved }: { settings: SettingsView; onSaved: (s: SettingsView) => void }) {
  const [path, setPath] = useState(settings.dataRoot ?? settings.defaultDataRoot);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      onSaved(await setDataRoot(path));
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card title={L("Data folder", "Thư mục dữ liệu")}>
      {!settings.dataRoot && (
        <p className="mb-3 text-sm">
          {L("Choose where Memora Studio stores projects, cache, proxies, models and logs. Original videos are never copied or modified. Pick a drive with plenty of free space.",
            "Chọn nơi Memora Studio lưu dự án, bộ đệm, bản xem trước, model và nhật ký. Video gốc không bao giờ bị sao chép hay sửa. Nên chọn ổ còn nhiều dung lượng.")}
        </p>
      )}
      <div className="flex gap-2">
        <input className={`${field} flex-1 font-mono`} value={path} onChange={(e) => setPath(e.target.value)} spellCheck={false}
          aria-label={L("Data folder path", "Đường dẫn thư mục dữ liệu")} />
        <button className={btn.primary} onClick={save} disabled={saving || path.trim() === "" || path === settings.dataRoot}>
          {settings.dataRoot ? L("Change", "Đổi") : L("Use this folder", "Dùng thư mục này")}
        </button>
      </div>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {settings.dataRoot && <Row label={L("Current", "Hiện tại")} value={<code>{settings.dataRoot}</code>} />}
      <Row label={L("Log folder", "Thư mục nhật ký")} value={settings.logDir ? <code>{settings.logDir}</code> : L("not active (console only)", "chưa bật (chỉ console)")} />
      {settings.databaseError && <p className="mt-2 text-sm text-red-600">{settings.databaseError}</p>}
      {settings.restartRequired && (
        <p className="mt-2 text-sm text-amber-600">{L("Restart Memora Studio to write logs to the new folder.", "Khởi động lại Memora Studio để ghi nhật ký vào thư mục mới.")}</p>
      )}
      {settings.dataRoot && !settings.databaseError && <Backup />}
    </Card>
  );
}

function Backup() {
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    try {
      setResult({ ok: true, text: `${L("Saved", "Đã lưu")} ${await backupDatabase()}` });
    } catch (e) {
      setResult({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="mt-3 border-t border-zinc-200 pt-3 text-sm dark:border-zinc-800">
      <div className="flex items-center justify-between gap-4">
        <span className="text-zinc-500">
          {L("Projects and edits are backed up when Memora Studio starts (last 10 kept in", "Dự án và bản dựng được sao lưu mỗi lần mở Memora Studio (giữ 10 bản gần nhất trong")} <code>database\backups</code>).
        </span>
        <button className={btn.secondary} onClick={run} disabled={busy}>{L("Back up now", "Sao lưu ngay")}</button>
      </div>
      {result && <p className={`mt-1 break-all ${result.ok ? "text-green-600" : "text-red-600"}`}>{result.text}</p>}
      <p className="mt-1 text-xs text-zinc-500">
        {L("To restore: close Memora Studio, copy a backup over", "Khôi phục: đóng Memora Studio, chép một bản sao lưu đè lên")} <code>database\memora.db</code>{" "}
        {L("and delete", "rồi xoá")} <code>memora.db-wal</code>, <code>memora.db-shm</code>.
      </p>
    </div>
  );
}

function Hardware({ hw }: { hw: HardwareInfo }) {
  const m = hw.memory;
  return (
    <>
      <Card title={L("System", "Hệ thống")}>
        <Row label="Windows" value={`${hw.os.name} ${hw.os.edition} ${hw.os.displayVersion} (build ${hw.os.build})`} />
        <Row label="CPU" value={`${hw.cpu.name} · ${hw.cpu.logicalProcessors} ${L("logical processors", "luồng")}`} />
        <Row label={L("Memory", "Bộ nhớ")} value={`${formatBytes(m.availablePhysical)} ${L("free of", "trống / ")} ${formatBytes(m.totalPhysical)} (${m.loadPercent}%)`} />
        <Row label={L("Commit (RAM + pagefile)", "Commit (RAM + pagefile)")} value={`${formatBytes(m.commitAvailable)} ${L("free of", "trống / ")} ${formatBytes(m.commitLimit)}`}
          warn={m.commitAvailable < LOW_COMMIT_BYTES} />
        {hw.dataDisk && <Row label={L("Data drive", "Ổ dữ liệu")} value={`${formatBytes(hw.dataDisk.free)} ${L("free of", "trống / ")} ${formatBytes(hw.dataDisk.total)}`} />}
      </Card>
      <Card title={L("Graphics", "Đồ hoạ")}>
        {hw.gpus.length === 0 && <p className="text-sm">{L("No hardware GPU detected. CPU processing will be used.", "Không thấy GPU. Sẽ xử lý bằng CPU.")}</p>}
        {hw.gpus.map((g) => (
          <div key={g.name} className="mb-2">
            <p className="text-sm font-medium">{g.name}</p>
            <Row label={L("Dedicated video memory", "Bộ nhớ video riêng")} value={formatBytes(g.dedicatedVideoMemory)} />
            <Row label={L("Shared system memory", "Bộ nhớ dùng chung")} value={`${formatBytes(g.sharedSystemMemory)} (${L("part of system RAM", "là một phần RAM")})`} />
            <Row label="Driver" value={g.driverVersion ?? L("unknown", "không rõ")} />
          </div>
        ))}
        <p className="text-xs text-zinc-500">
          {L("AI acceleration devices (OpenVINO CPU / GPU / NPU) are validated by the AI worker, not assumed from this list.",
            "Thiết bị tăng tốc AI (OpenVINO CPU / GPU / NPU) được AI worker kiểm tra thực tế, không suy ra từ danh sách này.")}
        </p>
      </Card>
    </>
  );
}

/** What to do next, from the state of the library and the open project (the owner did not know what came next). */
function NextStep({ project, version, go }: { project: Project | null; version: number; go: (p: Page, step?: Step) => void }) {
  const [state, setState] = useState<{ media: Media[]; models: ModelInfo[]; clips: number; plan: boolean } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = useJobs().some(active);
  useEffect(() => {
    Promise.all([listMedia(), listModels(), project ? listClips(project.id) : Promise.resolve([]), project ? getPlan(project.id) : Promise.resolve(null)])
      .then(([media, models, clips, plan]) => setState({ media, models, clips: clips.length, plan: !!plan }))
      .catch((e) => setError(String(e)));
  }, [project?.id, version]);
  if (!state) return error ? <p className="text-sm text-red-600">{error}</p> : null;

  const p = preparation(state.media, state.models);
  const todo = p.analyse.length + p.tag.length + p.transcribe.length;
  const prepare = () => prepareMedia(state.media, state.models).catch((e) => setError(String(e)));
  let text: string;
  let action: ReactNode = null;
  if (state.media.length === 0) {
    text = L("Start by adding the videos of your trip or event to the Library.", "Bắt đầu bằng cách thêm video của chuyến đi / sự kiện vào Thư viện.");
    action = <button className={btn.primary} onClick={() => go("library")}>{L("Open Library", "Mở Thư viện")}</button>;
  } else if (busy) {
    text = L("Memora is working (bar at the bottom). You can keep browsing; results appear when ready.", "Memora đang xử lý (thanh bên dưới). Bạn cứ xem tiếp, kết quả tự hiện khi xong.");
  } else if (todo > 0) {
    text = L(`${todo} analysis step(s) are missing (scenes, what is on screen, speech). Smart edit chooses much better after them.`,
      `Còn ${todo} bước phân tích chưa làm (cảnh, nội dung hình, lời nói). Dựng tự động sẽ chọn cảnh tốt hơn nhiều sau khi phân tích.`);
    action = <button className={btn.primary} onClick={prepare}>{L("Analyse now", "Phân tích ngay")}</button>;
  } else if (!project) {
    text = L("Your videos are ready. Create or open a project for the video you want to make.", "Video đã sẵn sàng. Tạo hoặc mở một dự án cho video bạn muốn làm.");
    action = <button className={btn.primary} onClick={() => go("project")}>{L("Open Projects", "Mở Dự án")}</button>;
  } else if (state.clips === 0) {
    text = L(`Describe the video you want for “${project.name}” and let Smart edit pick the scenes.`, `Mô tả video bạn muốn cho “${project.name}” rồi để Dựng tự động chọn cảnh.`);
    action = <button className={btn.primary} onClick={() => go("project", 2)}>{L("Smart edit", "Dựng tự động")}</button>;
  } else {
    text = L(`“${project.name}” has ${state.clips} clips. Check them, set look & sound, then export.`, `“${project.name}” đã có ${state.clips} clip. Kiểm tra, chỉnh hình & âm thanh rồi xuất video.`);
    action = <button className={btn.primary} onClick={() => go("project", 3)}>{L("Edit & export", "Chỉnh & xuất")}</button>;
  }
  return (
    <div className="mb-4 flex flex-wrap items-center gap-3 rounded-2xl border border-indigo-200 bg-indigo-50 px-4 py-3 text-sm dark:border-indigo-900 dark:bg-indigo-950/40">
      <span className="font-semibold text-indigo-700 dark:text-indigo-300">{L("Next step", "Bước tiếp theo")}</span>
      <span className="min-w-0 flex-1">{text}</span>
      {action}
      {p.missingModels && todo === 0 && state.media.length > 0 && !busy && (
        <button className={btn.link} onClick={() => go("settings")}>{L("Install AI models for better choices", "Cài model AI để chọn cảnh tốt hơn")}</button>
      )}
      {error && <span className="w-full text-red-600">{error}</span>}
    </div>
  );
}

/** One project as three steps: the library (shared by all projects), Smart edit, then timeline + export. */
function ProjectPage({ project, onOpen, step, setStep, version, bump, go }: {
  project: Project | null; onOpen: (p: Project | null) => void; step: Step; setStep: (s: Step) => void; version: number; bump: () => void; go: (p: Page) => void;
}) {
  const steps: [Step, string, string][] = [
    [1, L("Videos", "Video"), L("Library: add and analyse", "Thư viện: thêm & phân tích")],
    [2, L("Smart edit", "Dựng tự động"), L("Describe it, review the plan", "Mô tả, xem kế hoạch")],
    [3, L("Edit & export", "Chỉnh & xuất"), L("Timeline, look & sound, export", "Timeline, hình & tiếng, xuất")],
  ];
  return (
    <div className="space-y-4">
      {project ? (
        // With a project open the list folds away (native <details>; it stays mounted to keep the open project in sync).
        <details className="rounded-2xl border border-zinc-200 bg-white px-5 py-3 text-sm dark:border-zinc-800 dark:bg-zinc-900">
          <summary className="cursor-pointer text-zinc-500">{L(`Project: ${project.name} · change, create, rename…`, `Dự án: ${project.name} · đổi, tạo mới, đổi tên…`)}</summary>
          <div className="mt-3"><Projects active={project} onOpen={onOpen} /></div>
        </details>
      ) : (
        <Projects active={project} onOpen={onOpen} />
      )}
      {project && (
        <>
          <ol className="grid gap-2 sm:grid-cols-3">
            {steps.map(([n, title, sub]) => (
              <li key={n}>
                <button onClick={() => (n === 1 ? go("library") : setStep(n))}
                  className={`flex w-full items-center gap-3 rounded-xl border px-3 py-2 text-left ${step === n
                    ? "border-indigo-500 bg-indigo-50 dark:bg-indigo-950/40" : "border-zinc-200 bg-white hover:border-zinc-400 dark:border-zinc-800 dark:bg-zinc-900"}`}>
                  <span className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-sm font-semibold ${step === n ? "bg-indigo-600 text-white" : "bg-zinc-200 dark:bg-zinc-800"}`}>{n}</span>
                  <span><span className="block text-sm font-medium">{title}</span><span className="block text-xs text-zinc-500">{sub}</span></span>
                </button>
              </li>
            ))}
          </ol>
          {step === 2 && <EditingPlan key={project.id} project={project} onApplied={() => { bump(); setStep(3); }} />}
          {step === 3 && <Timeline key={project.id} project={project} version={version} />}
        </>
      )}
    </div>
  );
}

export default function App() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [project, setProject] = useState<Project | null>(null);
  const [page, setPage] = useState<Page>((recall("memora.page") as Page | null) ?? "library");
  const [step, setStep] = useState<Step>(2);
  const [version, setVersion] = useState(0);
  const bump = useCallback(() => setVersion((v) => v + 1), []);
  const ready = !!settings?.dataRoot && !settings.databaseError;

  const refresh = useCallback(() => {
    getSettings().then(setSettings).catch((e) => setError(String(e)));
    getHardwareInfo().then(setHw).catch((e) => setError(String(e)));
  }, []);
  useEffect(refresh, [refresh]);

  // Reopen the last project (per computer); a finished job refreshes the "next step" and the open lists.
  useEffect(() => {
    if (!ready) return;
    const id = Number(recall("memora.project"));
    if (id) listProjects().then((ps) => setProject(ps.find((p) => p.id === id) ?? null)).catch(() => {});
  }, [ready, settings?.dataRoot]);
  useEffect(() => {
    const unlisten = onJobUpdated((j) => { if (!active(j)) bump(); });
    return () => { unlisten.then((f) => f()); };
  }, [bump]);

  const go = (p: Page, s?: Step) => {
    setPage(p);
    remember("memora.page", p);
    if (s) setStep(s);
  };
  // "Edit photo" on a timeline clip: open it on the Photos page.
  useEffect(() => {
    const open = (e: Event) => { remember("memora.photo", String((e as CustomEvent<number>).detail)); setPage("photos"); remember("memora.page", "photos"); };
    window.addEventListener("memora:open-photo", open);
    return () => window.removeEventListener("memora:open-photo", open);
  }, []);
  const open = useCallback((p: Project | null) => {
    setProject(p);
    remember("memora.project", p ? String(p.id) : null);
  }, []);
  const current: Page = ready ? page : "settings";

  const nav: [Page, string, string][] = [
    ["library", "▦", L("Library", "Thư viện")],
    ["project", "✎", L("Projects", "Dự án")],
    ["photos", "◩", L("Photos", "Ảnh")],
    ["jobs", "⟳", L("Jobs", "Công việc")],
    ["settings", "⚙", L("Settings", "Cài đặt")],
  ];

  return (
    <div className="flex h-screen flex-col bg-zinc-50 text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
      <div className="flex min-h-0 flex-1">
        <nav className="flex w-52 shrink-0 flex-col border-r border-zinc-200 bg-white p-3 dark:border-zinc-800 dark:bg-zinc-900">
          <p className="mb-4 px-2 text-lg font-semibold">Memora <span className="text-indigo-600">Studio</span></p>
          {nav.map(([id, icon, name]) => (
            <button key={id} disabled={!ready && id !== "settings"} onClick={() => go(id)}
              className={`mb-1 flex items-center gap-3 rounded-lg px-3 py-2 text-left text-sm disabled:opacity-40 ${current === id
                ? "bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950/50 dark:text-indigo-300" : "hover:bg-zinc-100 dark:hover:bg-zinc-800"}`}>
              <span className="w-4 text-center">{icon}</span>{name}
            </button>
          ))}
          {project && ready && (
            <button className="mt-4 rounded-lg border border-zinc-200 px-3 py-2 text-left text-xs dark:border-zinc-800" onClick={() => go("project")}>
              <span className="block text-zinc-500">{L("Open project", "Dự án đang mở")}</span>
              <span className="block truncate font-medium">{project.name}</span>
            </button>
          )}
          <div className="mt-auto flex gap-1 px-2 text-xs">
            {(["vi", "en"] as const).map((l) => (
              <button key={l} className={`rounded px-2 py-1 ${lang === l ? "bg-zinc-200 font-medium dark:bg-zinc-800" : "text-zinc-500"}`} onClick={() => lang !== l && setLang(l)}>
                {l === "vi" ? "Tiếng Việt" : "English"}
              </button>
            ))}
          </div>
        </nav>
        <main className="min-w-0 flex-1 overflow-y-auto p-6">
          <div className={`mx-auto ${current === "photos" ? "max-w-[1800px]" : "max-w-6xl"}`}>
            {error && <p className="mb-4 text-sm text-red-600">{error}</p>}
            {ready && (current === "library" || current === "project") && <NextStep project={project} version={version} go={go} />}
            {/* key remounts the lists when the data root (and therefore the database) changes */}
            {current === "library" && ready && (
              <MediaLibrary key={settings!.dataRoot} project={project} version={version} onClipAdded={bump} onSettings={() => go("settings")} />
            )}
            {current === "project" && ready && (
              <ProjectPage key={settings!.dataRoot} project={project} onOpen={open} step={step} setStep={setStep} version={version} bump={bump} go={go} />
            )}
            {current === "photos" && ready && <PhotoEditor key={settings!.dataRoot} />}
            {current === "jobs" && <AiCenter />}
            {current === "settings" && (
              <div className="grid gap-4 lg:grid-cols-2">
                {settings && <div className="lg:col-span-2"><DataFolder settings={settings} onSaved={(s) => { setSettings(s); refresh(); }} /></div>}
                {ready && <div className="lg:col-span-2"><Models key={settings!.dataRoot} /></div>}
                {hw ? <Hardware hw={hw} /> : <p className="text-sm text-zinc-500">{L("Detecting hardware…", "Đang nhận diện phần cứng…")}</p>}
              </div>
            )}
          </div>
        </main>
      </div>
      <JobBar onOpen={() => go("jobs")} />
    </div>
  );
}
