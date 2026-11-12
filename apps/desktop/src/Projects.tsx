import { useCallback, useEffect, useState } from "react";
import { createProject, deleteProject, listProjects, renameProject, type Project } from "./api";
import { dateTime, L } from "./i18n";
import { btn, Card, field } from "./ui";

const inputClass = `${field} px-3`;
const linkClass = btn.link;

function ProjectRow({ project, active, onOpen, onChanged, onError }: {
  project: Project;
  active: boolean;
  onOpen: () => void;
  onChanged: () => void;
  onError: (e: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(project.name);
  const [confirming, setConfirming] = useState(false);

  const run = (action: Promise<unknown>) => action.then(onChanged).catch((e) => onError(String(e)));

  if (editing) {
    return (
      <li className="flex items-center gap-2 py-2">
        <input
          className={`${inputClass} flex-1`}
          value={name}
          autoFocus
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setEditing(false)}
          aria-label={L("Project name", "Tên dự án")}
        />
        <button className={linkClass} onClick={() => run(renameProject(project.id, name)).then(() => setEditing(false))}>
          {L("Save", "Lưu")}
        </button>
        <button className={linkClass} onClick={() => { setName(project.name); setEditing(false); }}>
          {L("Cancel", "Huỷ")}
        </button>
      </li>
    );
  }

  return (
    <li className={`flex items-center gap-3 rounded-lg px-2 py-2 ${active ? "bg-indigo-50 dark:bg-indigo-950/40" : ""}`}>
      <div className="flex-1">
        <p className="text-sm font-medium">{project.name}{active && <span className="ml-2 text-xs text-indigo-600">{L("open", "đang mở")}</span>}</p>
        <p className="text-xs text-zinc-500">{L("Updated", "Cập nhật")} {dateTime(new Date(project.updatedAt))}</p>
      </div>
      {confirming ? (
        <>
          <span className="text-xs text-zinc-500">{L("Delete project? Original videos are not affected.", "Xoá dự án? Video gốc không bị ảnh hưởng.")}</span>
          <button className="text-xs text-red-600 hover:underline" onClick={() => run(deleteProject(project.id))}>
            {L("Delete", "Xoá")}
          </button>
          <button className={linkClass} onClick={() => setConfirming(false)}>{L("Keep", "Giữ")}</button>
        </>
      ) : (
        <>
          {!active && <button className={btn.secondary} onClick={onOpen}>{L("Open", "Mở")}</button>}
          <button className={linkClass} onClick={() => setEditing(true)}>{L("Rename", "Đổi tên")}</button>
          <button className={linkClass} onClick={() => setConfirming(true)}>{L("Delete", "Xoá")}</button>
        </>
      )}
    </li>
  );
}

export default function Projects({ active, onOpen }: { active: Project | null; onOpen: (p: Project | null) => void }) {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    listProjects().then(setProjects).catch((e) => setError(String(e)));
  }, []);
  useEffect(load, [load]);

  // Keep the open project in sync after rename / delete.
  useEffect(() => {
    if (!projects || !active) return;
    const now = projects.find((p) => p.id === active.id);
    if (!now) onOpen(null);
    else if (now.name !== active.name) onOpen(now);
  }, [projects, active, onOpen]);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    try {
      await createProject(name);
      setName("");
      load();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Card title={L("Projects", "Dự án")}>
      <form className="flex gap-2" onSubmit={create}>
        <input
          className={`${inputClass} flex-1`}
          placeholder={L("New project name, e.g. Đà Lạt 2026", "Tên dự án mới, ví dụ Đà Lạt 2026")}
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label={L("New project name", "Tên dự án mới")}
        />
        <button className={btn.primary} disabled={name.trim() === ""}>{L("Create", "Tạo")}</button>
      </form>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {projects?.length === 0 && <p className="mt-3 text-sm text-zinc-500">{L("No projects yet. A project is one video you want to make (a trip recap, a reel…).", "Chưa có dự án. Mỗi dự án là một video bạn muốn làm (tổng hợp chuyến đi, reels…).")}</p>}
      <ul className="mt-2 divide-y divide-zinc-200 dark:divide-zinc-800">
        {projects?.map((p) => (
          <ProjectRow
            key={p.id}
            project={p}
            active={p.id === active?.id}
            onOpen={() => onOpen(p)}
            onChanged={load}
            onError={setError}
          />
        ))}
      </ul>
    </Card>
  );
}
