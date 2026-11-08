"""Checks that docs/system/* still describe the code: every module, Tauri command, job kind, migration,
table, React file, worker job and CLI command is documented, frontend wrappers call real commands, and
relative links resolve. Stdlib only. Run from anywhere:  python .claude/skills/sync-docs/check_docs.py
Exit code 1 when something is missing (the list says what and where to add it)."""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "apps/desktop/src-tauri/src"
UI = ROOT / "apps/desktop/src"
SYS = ROOT / "docs/system"
DOCS = ["README.md", "ARCHITECTURE.md", "FOLDERS.md", "BACKEND.md", "FRONTEND.md", "DATABASE.md", "WORKER.md", "FLOWS.md"]


def read(p: Path) -> str:
    return p.read_text(encoding="utf-8") if p.exists() else ""


def code_facts() -> dict:
    lib = read(SRC / "lib.rs")
    handler = re.search(r"generate_handler!\[(.*?)\]", lib, re.S)
    jobs = re.search(r"UI_JOB_KINDS:[^=]*=\s*&\[(.*?)\];", lib, re.S)
    db = read(SRC / "db.rs")
    migrations = re.search(r"const MIGRATIONS: &\[&str\] = &\[(.*?)\n\];", db, re.S)
    worker = read(ROOT / "workers/ai-worker/memora_worker.py")
    worker_jobs = re.search(r"JOBS = \{(.*?)\}", worker, re.S)
    cli = read(SRC / "cli.rs")
    dispatch = re.search(r"fn dispatch\(.*?\n}\n", cli, re.S)
    return {
        "modules": re.findall(r"^\s*(?:pub )?mod (\w+);", lib, re.M) + [f"bin/{p.stem}" for p in (SRC / "bin").glob("*.rs")],
        "commands": re.findall(r"\b([a-z_]+)\b", handler.group(1)) if handler else [],
        "job_kinds": re.findall(r'"([\w.]+)"', jobs.group(1)) if jobs else [],
        "migrations": [int(n) for n in re.findall(r"^\s*// (\d+):", migrations.group(1), re.M)] if migrations else [],
        "tables": sorted(set(re.findall(r"CREATE TABLE (\w+)", db))),
        "components": sorted(p.name for p in UI.glob("*.tsx") if p.name != "main.tsx"),
        "invokes": sorted(set(re.findall(r'invoke<[^>]*>\("(\w+)"', read(UI / "api.ts")) + re.findall(r'invoke\("(\w+)"', read(UI / "api.ts")))),
        "worker_jobs": re.findall(r'"([\w.]+)":', worker_jobs.group(1)) if worker_jobs else [],
        "cli_commands": re.findall(r'^\s{8}"([a-z-]+)" =>', dispatch.group(0), re.M) if dispatch else [],
    }


def main() -> int:
    f = code_facts()
    doc = {name: read(SYS / name) for name in DOCS}
    problems: list[str] = []

    def need(cond: bool, msg: str) -> None:
        if not cond:
            problems.append(msg)

    for name, text in doc.items():
        need(bool(text), f"docs/system/{name} is missing")
        need("Last verified:" in text, f"docs/system/{name}: no 'Last verified:' line")
    be, fe, dbd, wk, fl = doc["BACKEND.md"], doc["FRONTEND.md"], doc["DATABASE.md"], doc["WORKER.md"], doc["FLOWS.md"]
    for m in f["modules"]:
        need(f"### {m}.rs" in be, f"BACKEND.md: no section '### {m}.rs'")
    for c in f["commands"]:
        need(f"`{c}`" in be, f"BACKEND.md: Tauri command `{c}` not documented")
    for k in f["job_kinds"]:
        need(f"`{k}`" in be, f"BACKEND.md: job kind `{k}` not documented")
        need(k in fl, f"FLOWS.md: job kind {k} has no flow")
    for n in f["migrations"]:
        need(re.search(rf"^\|\s*{n}\s*\|", dbd, re.M) is not None, f"DATABASE.md: migration {n} has no table row '| {n} |'")
    for t in f["tables"]:
        need(t in dbd, f"DATABASE.md: table {t} not documented")
    for c in f["components"]:
        need(f"### {c}" in fe, f"FRONTEND.md: no section '### {c}'")
    for i in f["invokes"]:
        need(i in f["commands"], f"api.ts invokes '{i}', which is not a registered Tauri command")
        need(i in fe, f"FRONTEND.md: wrapper for command {i} not documented")
    for j in f["worker_jobs"]:
        need(f"`{j}`" in wk, f"WORKER.md: worker job `{j}` not documented")
    for c in f["cli_commands"]:
        need(f"`{c}" in be, f"BACKEND.md: CLI command `{c}` not documented (cli.rs section)")

    # Relative links in the system docs and the overview resolve.
    for path in [*(SYS / n for n in DOCS), ROOT / "docs/APP_OVERVIEW.md"]:
        for target in re.findall(r"\]\(([^)#\s]+)(?:#[^)]*)?\)", read(path)):
            if "://" not in target and not (path.parent / target).exists():
                problems.append(f"{path.relative_to(ROOT)}: broken link {target}")

    head = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    stale = [n for n, t in doc.items() if head and head not in t]
    counts = ", ".join(f"{len(v)} {k}" for k, v in f.items())
    print(f"code: {counts}")
    if stale:
        print(f"note: 'Last verified' does not name HEAD {head} in: {', '.join(stale)} (update after checking them)")
    if problems:
        print(f"{len(problems)} problem(s):")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("docs/system matches the code structure.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
