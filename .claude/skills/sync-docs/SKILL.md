---
name: sync-docs
description: Bring Memora Studio's documentation back in step with the code after a change. Use after ANY change to the app's structure or behaviour (Rust module, Tauri command, job kind, migration or SQL, React component, api.ts wrapper, worker job, CLI command, flow, folder, dependency, config) and before reporting the work as done or committing. Also use when the user asks to "update the docs", "ghi nhận thay đổi vào tài liệu", or when docs and code seem to disagree.
---

# sync-docs — keep the system docs true

The documentation set in `docs/system/` (index: `docs/system/README.md`) describes the code in detail so
every session starts from the current state. This skill updates it after a change. The code is the source
of truth; never document something you have not read in the code.

## 1. Find what changed

```bash
git status --short                      # uncommitted work
git diff --stat HEAD                    # size of uncommitted changes
git log --oneline -5                    # recent commits not yet reflected (compare with "Last verified" lines)
```

If the docs were last verified at an older commit, also run `git diff --stat <that commit> HEAD`.

## 2. Map changes to documents

| Changed | Update |
|---|---|
| `apps/desktop/src-tauri/src/<module>.rs` | `docs/system/BACKEND.md` section `### <module>.rs` (types, functions, constants, commands, jobs, tables, processes, tests, notes) |
| New / removed module (`mod` in lib.rs) or `src/bin/*` | BACKEND.md module map + new section; FOLDERS.md tree |
| `generate_handler!` / `UI_JOB_KINDS` / `job_runner` in lib.rs | BACKEND.md command and job tables; FLOWS.md for the user-visible flow |
| `db.rs` `MIGRATIONS` or any SQL / JSON stored in a column | `docs/system/DATABASE.md` (migration row, table section, JSON shape, readers/writers) |
| `apps/desktop/src/*.tsx`, `api.ts`, `index.css` | `docs/system/FRONTEND.md` (`### <File>.tsx`, api.ts table) |
| `workers/ai-worker/*`, `worker.rs`, `tools/clip-export/*` | `docs/system/WORKER.md` |
| Behaviour the user can notice (what a button / job / command does, order of steps) | `docs/system/FLOWS.md` |
| `Cargo.toml`, `package.json`, `requirements.lock.txt`, `tauri.conf.json`, capabilities, settings, data-root paths | `docs/system/ARCHITECTURE.md` (config, versions); FOLDERS.md for paths |
| New architecture decision | new `docs/adr/NNNN-*.md`; ARCHITECTURE.md decisions table |
| `cli.rs` commands | BACKEND.md `### cli.rs`; FLOWS.md §15; `docs/APP_OVERVIEW.md` CLI table |
| Anything above | `docs/APP_OVERVIEW.md` (summary sections that mention it), the status paragraph in `CLAUDE.md` (one short clause, no detail), `docs/PROGRESS.md` day log (what changed + how it was verified) |

## 3. Edit

- Read the changed code first, then edit only the affected sections (small edits with the Edit tool; rewrite a section only when most of it changed).
- Keep the conventions of `docs/system/README.md`: English, tables/bullets, relative links, facts only, assumptions labelled.
- Keep the headings the checker uses: `### <module>.rs`, `### <File>.tsx`, backticked command / job / worker-job / CLI names, migration rows `| N |`.
- Update the `Last verified:` line of every file you touched to the current date and `git rev-parse --short HEAD` (say "+ uncommitted changes" when the work is not committed yet).
- Remove documentation of code that was deleted. Do not leave "TODO: document".
- If you find a doc / code disagreement you did not cause, fix the doc when the code is clearly intended; otherwise report it to the owner instead of guessing.

## 4. Check

```bash
python .claude/skills/sync-docs/check_docs.py
```

It lists every module, command, job kind, migration, table, component, api.ts wrapper, worker job and CLI
command missing from the docs, api.ts calls to commands that do not exist, and broken relative links. Fix
until it prints `docs/system matches the code structure.` The checker only sees structure — also reread the
FLOWS.md steps you touched against the code, since behaviour changes are not detectable by it.

## 5. Report

Tell the owner (in Vietnamese, per CLAUDE.md) in a few lines which documents were updated and anything you
could not verify. Docs are committed together with the code change when the owner asks for a commit.
