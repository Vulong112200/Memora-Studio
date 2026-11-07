import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { cancelJob, formatBytes, listModels, onJobUpdated, removeModel, submitJob, type Job, type ModelInfo } from "./api";

const linkClass = "text-xs text-zinc-500 hover:underline disabled:cursor-not-allowed disabled:opacity-40";
const WORKING = new Set(["downloading", "verifying", "installing", "validating"]);
const FAILED = new Set(["download_failed", "checksum_failed", "install_failed", "model_load_failed", "incompatible"]);
const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

function size(m: ModelInfo) {
  const known = m.artifacts.every((a) => a.sizeBytes !== null);
  return known ? formatBytes(m.artifacts.reduce((s, a) => s + (a.sizeBytes ?? 0), 0)) : "size not verified";
}

function ModelRow({ m, job, onChanged }: { m: ModelInfo; job: Job | null; onChanged: () => void }) {
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const running = job && (job.status === "queued" || job.status === "running");
  const install = (files?: Record<string, string>) =>
    submitJob("models.install", { id: m.id, files }).then(() => { setConfirming(false); onChanged(); }).catch((e) => setError(String(e)));

  async function fromFiles() {
    setError(null);
    const picked = await open({ multiple: true, title: `Install ${m.name} from files` });
    if (!picked || picked.length === 0) return;
    const files: Record<string, string> = {};
    for (const a of m.artifacts) {
      const hit = picked.find((p) => fileName(p).toLowerCase() === a.file.toLowerCase()) ?? (m.artifacts.length === 1 && picked.length === 1 ? picked[0] : undefined);
      if (!hit) return setError(`Select these files: ${m.artifacts.map((x) => x.file).join(", ")}`);
      files[a.file] = hit;
    }
    install(files);
  }

  const stateClass = m.state === "ready" ? "text-green-600" : FAILED.has(m.state) ? "text-red-600" : "text-zinc-500";
  return (
    <li className="py-2 text-sm">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-56 flex-1">
          <p className="font-medium">{m.name} <span className="text-xs font-normal text-zinc-500">· {m.category} · {size(m)} · {m.runtimes.map((r) => r.runtime).join(", ")}</span></p>
          <p className="text-xs text-zinc-500">{m.description}</p>
        </div>
        <span className={`text-xs ${stateClass}`}>
          {m.state.replace(/_/g, " ")}{running && job!.total > 0 ? ` ${Math.round((job!.done / job!.total) * 100)}%` : ""}
        </span>
        {running ? (
          <button className={linkClass} onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>Cancel</button>
        ) : (
          <>
            <button className={linkClass} disabled={!!m.downloadBlocker} title={m.downloadBlocker ?? undefined} onClick={() => setConfirming(true)}>
              {FAILED.has(m.state) ? "Retry download" : "Download"}
            </button>
            <button className={linkClass} onClick={fromFiles} title="Offline install: pick the model files you already have">Install from files…</button>
            {m.state !== "not_installed" && !WORKING.has(m.state) && (
              <button className={linkClass} onClick={() => removeModel(m.id).then(onChanged).catch((e) => setError(String(e)))}>Remove</button>
            )}
          </>
        )}
      </div>
      {m.downloadBlocker && m.state === "not_installed" && <p className="mt-1 text-xs text-zinc-500">{m.downloadBlocker}</p>}
      {confirming && (
        <div className="mt-2 rounded-md border border-zinc-300 p-3 text-xs dark:border-zinc-700">
          <p className="font-medium">Download {m.name}?</p>
          <p>Size: {size(m)} · License: {m.license}</p>
          {m.artifacts.map((a) => <p key={a.file} className="break-all font-mono">{a.url}</p>)}
          <p className="mt-1 text-zinc-500">Files are checked against their SHA-256 before installation.</p>
          <div className="mt-2 flex gap-3">
            <button className="rounded bg-zinc-900 px-3 py-1 text-white dark:bg-zinc-100 dark:text-zinc-900" onClick={() => install()}>Download</button>
            <button className={linkClass} onClick={() => setConfirming(false)}>Cancel</button>
          </div>
        </div>
      )}
      {m.error && <p className="mt-1 text-xs text-red-600">{m.error}</p>}
      {m.details?.devices && (
        <p className="mt-1 text-xs text-zinc-500">
          Load test: {m.details.devices.map((d) => `${d.device} ${d.ok ? `ok (compile ${d.compileMs} ms, run ${d.inferMs} ms)` : `failed: ${d.error}`}`).join(" · ")}
        </p>
      )}
      {error && <p className="mt-1 text-xs text-red-600">{error}</p>}
    </li>
  );
}

export default function Models() {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [jobs, setJobs] = useState<Record<string, Job>>({});
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => { listModels().then(setModels).catch((e) => setError(String(e))); }, []);

  useEffect(() => {
    load();
    const unlisten = onJobUpdated((j) => {
      if (j.kind !== "models.install") return;
      const id = (j.params as { id: string }).id;
      setJobs((prev) => ({ ...prev, [id]: j }));
      load(); // state changes (downloading → verifying → …) are stored in the database
    });
    return () => { unlisten.then((f) => f()); };
  }, [load]);

  return (
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="mb-1 text-sm font-semibold uppercase tracking-wide text-zinc-500">AI models</h2>
      <p className="text-xs text-zinc-500">
        Optional. Memora Studio works without them (metadata, scenes and image quality need no models). Nothing is downloaded without your confirmation.
      </p>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      <ul className="mt-2 divide-y divide-zinc-200 dark:divide-zinc-800">
        {models.map((m) => <ModelRow key={m.id} m={m} job={jobs[m.id] ?? null} onChanged={load} />)}
      </ul>
    </section>
  );
}
