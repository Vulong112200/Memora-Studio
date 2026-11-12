import { useEffect, useState } from "react";
import {
  cancelJob, getWorkerInfo, listJobs, onJobUpdated, submitJob,
  type DeviceValidation, type Job, type WorkerInfo,
} from "./api";
import { L } from "./i18n";
import { btn, Card, jobLabel, Progress } from "./ui";

const ACTIVE = new Set(["queued", "running"]);

function DeviceTable({ devices }: { devices: DeviceValidation[] }) {
  return (
    <table className="mt-2 w-full text-sm">
      <thead className="text-left text-xs text-zinc-500">
        <tr><th className="py-1">{L("Device", "Thiết bị")}</th><th>{L("Result", "Kết quả")}</th><th>{L("Precision", "Độ chính xác")}</th><th>Compile</th><th>Inference</th></tr>
      </thead>
      <tbody>
        {devices.map((d) => (
          <tr key={d.device} className="border-t border-zinc-200 dark:border-zinc-800" title={d.name}>
            <td className="py-1 font-medium">{d.device}</td>
            <td className={d.ok ? "text-green-600" : "text-red-600"}>{d.ok ? L("works", "chạy được") : (d.error ?? L("failed", "lỗi"))}</td>
            <td>{d.precision ?? "–"}</td>
            <td>{d.compileMs != null ? `${d.compileMs} ms` : "–"}</td>
            <td>{d.inferMs != null ? `${d.inferMs} ms` : "–"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function JobRow({ job }: { job: Job }) {
  const pct = job.total > 0 ? Math.round((job.done / job.total) * 100) : 0;
  const devices = (job.result as { devices?: DeviceValidation[] } | null)?.devices;
  return (
    <li className="py-2">
      <div className="flex items-center gap-3 text-sm">
        <span className="font-medium">#{job.id} {jobLabel(job.kind)}</span>
        <span className="text-zinc-500">{job.status}{job.status === "running" && job.total > 0 ? ` ${job.done}/${job.total}` : ""}</span>
        {ACTIVE.has(job.status) && (
          <button className="ml-auto text-xs text-zinc-500 hover:underline" onClick={() => cancelJob(job.id)}>{L("Cancel", "Huỷ")}</button>
        )}
      </div>
      {job.status === "running" && (
        <div className="mt-1"><Progress value={pct / 100} /></div>
      )}
      {job.error && <p className="mt-1 text-sm text-red-600">{job.error}</p>}
      {devices && <DeviceTable devices={devices} />}
    </li>
  );
}

export default function AiCenter() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [worker, setWorker] = useState<WorkerInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listJobs().then(setJobs).catch((e) => setError(String(e)));
    getWorkerInfo().then(setWorker);
    const unlisten = onJobUpdated((job) => {
      setJobs((prev) => {
        const i = prev.findIndex((j) => j.id === job.id);
        return i < 0 ? [...prev, job] : prev.map((j) => (j.id === job.id ? job : j));
      });
      getWorkerInfo().then(setWorker);
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  const busy = jobs.some((j) => j.kind === "validate_devices" && ACTIVE.has(j.status));

  return (
    <Card title={L("Jobs and AI worker", "Công việc và AI worker")} actions={
      <button className={btn.secondary} disabled={busy} onClick={() => submitJob("validate_devices").catch((e) => setError(String(e)))}>
        {L("Validate AI devices", "Kiểm tra thiết bị AI")}
      </button>
    }>
      <p className="text-sm text-zinc-500">
        {worker
          ? `${L("AI worker running", "AI worker đang chạy")} · Python ${worker.python} · OpenVINO ${worker.openvino ?? "–"} · ${L("devices", "thiết bị")} ${worker.devices.join(", ") || "–"}`
          : L("AI worker not running (starts automatically when a job needs it).", "AI worker chưa chạy (tự khởi động khi có việc cần).")}
      </p>
      {worker?.runtimeError && <p className="text-sm text-amber-600">{worker.runtimeError}</p>}
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      <ul className="mt-2 divide-y divide-zinc-200 dark:divide-zinc-800">
        {jobs.length === 0 && <li className="py-2 text-sm text-zinc-500">{L("No jobs in this session yet.", "Chưa có công việc nào trong phiên này.")}</li>}
        {[...jobs].reverse().map((j) => <JobRow key={j.id} job={j} />)}
      </ul>
    </Card>
  );
}
