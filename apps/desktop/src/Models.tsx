import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { cancelJob, formatBytes, listModels, onJobUpdated, removeModel, submitJob, type Job, type ModelInfo } from "./api";
import { L } from "./i18n";
import { btn, Card, fileName } from "./ui";

const linkClass = btn.link;
const WORKING = new Set(["downloading", "verifying", "installing", "validating"]);
const FAILED = new Set(["download_failed", "checksum_failed", "install_failed", "model_load_failed", "incompatible"]);

function size(m: ModelInfo) {
  const known = m.artifacts.every((a) => a.sizeBytes !== null);
  return known ? formatBytes(m.artifacts.reduce((s, a) => s + (a.sizeBytes ?? 0), 0)) : L("size not verified", "chưa rõ dung lượng");
}

function ModelRow({ m, job, onChanged }: { m: ModelInfo; job: Job | null; onChanged: () => void }) {
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const running = job && (job.status === "queued" || job.status === "running");
  const install = (files?: Record<string, string>) =>
    submitJob("models.install", { id: m.id, files }).then(() => { setConfirming(false); onChanged(); }).catch((e) => setError(String(e)));

  async function fromFiles() {
    setError(null);
    const picked = await open({ multiple: true, title: L(`Install ${m.name} from files`, `Cài ${m.name} từ file`) });
    if (!picked || picked.length === 0) return;
    const files: Record<string, string> = {};
    for (const a of m.artifacts) {
      const hit = picked.find((p) => fileName(p).toLowerCase() === a.file.toLowerCase()) ?? (m.artifacts.length === 1 && picked.length === 1 ? picked[0] : undefined);
      if (!hit) return setError(L("Select these files: ", "Hãy chọn các file: ") + m.artifacts.map((x) => x.file).join(", "));
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
          <button className={linkClass} onClick={() => cancelJob(job!.id).catch((e) => setError(String(e)))}>{L("Cancel", "Huỷ")}</button>
        ) : (
          <>
            <button className={linkClass} disabled={!!m.downloadBlocker} title={m.downloadBlocker ?? undefined} onClick={() => setConfirming(true)}>
              {FAILED.has(m.state) ? L("Retry download", "Tải lại") : L("Download", "Tải về")}
            </button>
            <button className={linkClass} onClick={fromFiles} title={L("Offline install: pick the model files you already have", "Cài offline: chọn file model bạn đã có")}>{L("Install from files…", "Cài từ file…")}</button>
            {m.state !== "not_installed" && !WORKING.has(m.state) && (
              <button className={linkClass} onClick={() => removeModel(m.id).then(onChanged).catch((e) => setError(String(e)))}>{L("Remove", "Gỡ")}</button>
            )}
          </>
        )}
      </div>
      {m.downloadBlocker && m.state === "not_installed" && <p className="mt-1 text-xs text-zinc-500">{m.downloadBlocker}</p>}
      {confirming && (
        <div className="mt-2 rounded-md border border-zinc-300 p-3 text-xs dark:border-zinc-700">
          <p className="font-medium">{L("Download", "Tải")} {m.name}?</p>
          <p>{L("Size", "Dung lượng")}: {size(m)} · {L("License", "Giấy phép")}: {m.license}</p>
          {m.artifacts.map((a) => <p key={a.file} className="break-all font-mono">{a.url}</p>)}
          <p className="mt-1 text-zinc-500">{L("Files are checked against their SHA-256 before installation.", "File được kiểm tra SHA-256 trước khi cài.")}</p>
          <div className="mt-2 flex gap-3">
            <button className="rounded bg-zinc-900 px-3 py-1 text-white dark:bg-zinc-100 dark:text-zinc-900" onClick={() => install()}>{L("Download", "Tải về")}</button>
            <button className={linkClass} onClick={() => setConfirming(false)}>{L("Cancel", "Huỷ")}</button>
          </div>
        </div>
      )}
      {m.error && <p className="mt-1 text-xs text-red-600">{m.error}</p>}
      {m.details?.devices && (
        <p className="mt-1 text-xs text-zinc-500">
          {L("Load test", "Thử nạp")}: {m.details.devices.map((d) => `${d.device} ${d.ok ? `ok (compile ${d.compileMs} ms, run ${d.inferMs} ms)` : `failed: ${d.error}`}`).join(" · ")}
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
    <Card title={L("AI models", "Model AI")}>
      <p className="text-xs text-zinc-500">
        {L("Optional, all run on this computer. Without them Memora still reads scenes and image quality; with them Smart edit also knows what is on screen (OpenCLIP) and what people say (Whisper). Nothing is downloaded without your confirmation.",
          "Không bắt buộc, tất cả chạy trên máy này. Không có model, Memora vẫn đọc được cảnh và chất lượng hình; có model, Dựng tự động biết thêm nội dung hình (OpenCLIP) và lời nói (Whisper). Không tải gì nếu bạn chưa đồng ý.")}
      </p>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      <ul className="mt-2 divide-y divide-zinc-200 dark:divide-zinc-800">
        {models.map((m) => <ModelRow key={m.id} m={m} job={jobs[m.id] ?? null} onChanged={load} />)}
      </ul>
    </Card>
  );
}
