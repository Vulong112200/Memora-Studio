import { useCallback, useEffect, useState } from "react";
import { createProject, deleteProject, listProjects, renameProject, type Project } from "./api";

const inputClass = "rounded-md border border-zinc-300 bg-transparent px-3 py-1.5 text-sm dark:border-zinc-700";
const linkClass = "text-xs text-zinc-500 hover:underline";

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
          aria-label="Project name"
        />
        <button className={linkClass} onClick={() => run(renameProject(project.id, name)).then(() => setEditing(false))}>
          Save
        </button>
        <button className={linkClass} onClick={() => { setName(project.name); setEditing(false); }}>
          Cancel
        </button>
      </li>
    );
  }

  return (
    <li className="flex items-center gap-3 py-2">
      <div className="flex-1">
        <p className="text-sm font-medium">{project.name}{active && <span className="ml-2 text-xs text-green-600">open</span>}</p>
        <p className="text-xs text-zinc-500">Updated {new Date(project.updatedAt).toLocaleString()}</p>
      </div>
      {confirming ? (
        <>
          <span className="text-xs text-zinc-500">Delete project? Original videos are not affected.</span>
          <button className="text-xs text-red-600 hover:underline" onClick={() => run(deleteProject(project.id))}>
            Delete
          </button>
          <button className={linkClass} onClick={() => setConfirming(false)}>Keep</button>
        </>
      ) : (
        <>
          {!active && <button className={linkClass} onClick={onOpen}>Open</button>}
          <button className={linkClass} onClick={() => setEditing(true)}>Rename</button>
          <button className={linkClass} onClick={() => setConfirming(true)}>Delete</button>
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
    <section className="rounded-xl border border-zinc-200 bg-white p-5 dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">Projects</h2>
      <form className="flex gap-2" onSubmit={create}>
        <input
          className={`${inputClass} flex-1`}
          placeholder="New project name, e.g. Đà Lạt 2026"
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label="New project name"
        />
        <button
          className="rounded-md bg-zinc-900 px-4 py-1.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-zinc-100 dark:text-zinc-900"
          disabled={name.trim() === ""}
        >
          Create
        </button>
      </form>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
      {projects?.length === 0 && <p className="mt-3 text-sm text-zinc-500">No projects yet.</p>}
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
    </section>
  );
}
