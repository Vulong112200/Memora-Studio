import { useCallback, useEffect, useState, type ReactNode } from "react";
import { formatBytes, getHardwareInfo, getSettings, setDataRoot, type HardwareInfo, type Project, type SettingsView } from "./api";
import Projects from "./Projects";
import Timeline from "./Timeline";
import EditingPlan from "./EditingPlan";
import AiCenter from "./AiCenter";
import MediaLibrary from "./MediaLibrary";
import Models from "./Models";

/** Warn when less than this much commit charge is left (fixed small pagefile, docs/Production.md §9.2). */
const LOW_COMMIT_BYTES = 8 * 1024 ** 3;

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">{title}</h2>
      {children}
    </section>
  );
}

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
    <Card title="Data folder">
      {!settings.dataRoot && (
        <p className="mb-3 text-sm">
          Choose where Memora Studio stores projects, cache, proxies, models and logs. Original videos are never
          copied or modified. Pick a drive with plenty of free space.
        </p>
      )}
      <div className="flex gap-2">
        <input
          className="flex-1 rounded-md border border-zinc-300 bg-transparent px-3 py-2 font-mono text-sm dark:border-zinc-700"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          spellCheck={false}
          aria-label="Data folder path"
        />
        <button
          className="rounded-md bg-zinc-900 px-4 py-2 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900"
          onClick={save}
          disabled={saving || path.trim() === "" || path === settings.dataRoot}
        >
          {settings.dataRoot ? "Change" : "Use this folder"}
        </button>
      </div>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {settings.dataRoot && <Row label="Current" value={<code>{settings.dataRoot}</code>} />}
      <Row label="Log folder" value={settings.logDir ? <code>{settings.logDir}</code> : "not active (console only)"} />
      {settings.databaseError && <p className="mt-2 text-sm text-red-600">{settings.databaseError}</p>}
      {settings.restartRequired && (
        <p className="mt-2 text-sm text-amber-600">Restart Memora Studio to write logs to the new folder.</p>
      )}
    </Card>
  );
}

function Hardware({ hw }: { hw: HardwareInfo }) {
  const m = hw.memory;
  return (
    <>
      <Card title="System">
        <Row label="Windows" value={`${hw.os.name} ${hw.os.edition} ${hw.os.displayVersion} (build ${hw.os.build})`} />
        <Row label="CPU" value={`${hw.cpu.name} · ${hw.cpu.logicalProcessors} logical processors`} />
        <Row label="Memory" value={`${formatBytes(m.availablePhysical)} free of ${formatBytes(m.totalPhysical)} (${m.loadPercent}% used)`} />
        <Row
          label="Commit (RAM + pagefile)"
          value={`${formatBytes(m.commitAvailable)} free of ${formatBytes(m.commitLimit)}`}
          warn={m.commitAvailable < LOW_COMMIT_BYTES}
        />
        {hw.dataDisk && (
          <Row label="Data drive" value={`${formatBytes(hw.dataDisk.free)} free of ${formatBytes(hw.dataDisk.total)}`} />
        )}
      </Card>
      <Card title="Graphics">
        {hw.gpus.length === 0 && <p className="text-sm">No hardware GPU detected. CPU processing will be used.</p>}
        {hw.gpus.map((g) => (
          <div key={g.name} className="mb-2">
            <p className="text-sm font-medium">{g.name}</p>
            <Row label="Dedicated video memory" value={formatBytes(g.dedicatedVideoMemory)} />
            <Row label="Shared system memory" value={`${formatBytes(g.sharedSystemMemory)} (part of system RAM)`} />
            <Row label="Driver" value={g.driverVersion ?? "unknown"} />
          </div>
        ))}
        <p className="text-xs text-zinc-500">
          AI acceleration devices (OpenVINO CPU / GPU / NPU) are validated by the AI worker, not assumed from this list.
        </p>
      </Card>
    </>
  );
}

export default function App() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [hw, setHw] = useState<HardwareInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [project, setProject] = useState<Project | null>(null);
  const [timelineVersion, setTimelineVersion] = useState(0);

  const refresh = useCallback(() => {
    getSettings().then(setSettings).catch((e) => setError(String(e)));
    getHardwareInfo().then(setHw).catch((e) => setError(String(e)));
  }, []);
  useEffect(refresh, [refresh]);

  return (
    <main className="min-h-screen bg-zinc-50 p-6 text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
      <header className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Memora Studio</h1>
        <button className="text-sm text-zinc-500 hover:underline" onClick={refresh}>
          Refresh
        </button>
      </header>
      {error && <p className="mb-4 text-sm text-red-600">{error}</p>}
      <div className="grid max-w-5xl gap-4 lg:grid-cols-2">
        {settings && (
          <div className="lg:col-span-2">
            <DataFolder settings={settings} onSaved={(s) => { setSettings(s); refresh(); }} />
          </div>
        )}
        {settings?.dataRoot && !settings.databaseError && (
          <>
            {/* key remounts the lists when the data root (and therefore the database) changes */}
            <div className="lg:col-span-2">
              <Projects key={settings.dataRoot} active={project} onOpen={setProject} />
            </div>
            {project && (
              <div className="lg:col-span-2">
                <EditingPlan key={`${settings.dataRoot}-${project.id}`} project={project} onApplied={() => setTimelineVersion((v) => v + 1)} />
              </div>
            )}
            {project && (
              <div className="lg:col-span-2">
                <Timeline key={`${settings.dataRoot}-${project.id}`} project={project} version={timelineVersion} />
              </div>
            )}
            <div className="lg:col-span-2">
              <MediaLibrary key={settings.dataRoot} project={project} onClipAdded={() => setTimelineVersion((v) => v + 1)} />
            </div>
          </>
        )}
        <div className="lg:col-span-2">
          <AiCenter />
        </div>
        {settings?.dataRoot && !settings.databaseError && (
          <div className="lg:col-span-2">
            <Models key={settings.dataRoot} />
          </div>
        )}
        {hw ? <Hardware hw={hw} /> : <p className="text-sm text-zinc-500">Detecting hardware…</p>}
      </div>
    </main>
  );
}
