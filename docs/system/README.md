# Memora Studio — System documentation

Last verified: 2026-11-08 against commit `b751673`.

Detailed, code-level documentation of how the app is built and how it works. It is written for the
developer assistant (Claude Code) and for people changing the code. **It must match the code**: after
any change to modules, commands, jobs, tables, components, flows or folders, run the `sync-docs`
skill ([.claude/skills/sync-docs/SKILL.md](../../.claude/skills/sync-docs/SKILL.md)); its checker
`python .claude/skills/sync-docs/check_docs.py` lists what the docs are missing.

## Which file holds what

| File | Holds | Update when |
|---|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Principles, process model, layers, configuration, decisions (ADRs), versions | Architecture rule, new process/runtime, dependency or config change, new ADR |
| [FOLDERS.md](FOLDERS.md) | Repository tree and data-folder tree, what each folder/file is for | New/renamed/removed folder or important file, new data-root path |
| [BACKEND.md](BACKEND.md) | Every Rust module: types, functions, constants, commands, jobs, tests | Any change in `apps/desktop/src-tauri/` |
| [FRONTEND.md](FRONTEND.md) | Every React file, `api.ts` wrappers, UI behaviour, styling | Any change in `apps/desktop/src/` or frontend tooling |
| [DATABASE.md](DATABASE.md) | Migrations, tables, JSON column shapes, readers/writers, backups | New migration, new SQL, changed JSON stored in a column |
| [WORKER.md](WORKER.md) | Python AI worker: protocol, jobs, analysis/tagging algorithms, model export | Any change in `workers/ai-worker/`, `worker.rs`, `tools/clip-export/` |
| [FLOWS.md](FLOWS.md) | End-to-end flows across UI → command → job → process → DB → files | Any change that alters what happens when the user does something |

Summaries and history live elsewhere:

| File | Role |
|---|---|
| [../../CLAUDE.md](../../CLAUDE.md) | Rules for Claude Code + one-paragraph status. Read first every session. |
| [../APP_OVERVIEW.md](../APP_OVERVIEW.md) | One-file overview of the whole app (summary of these docs). |
| [../PROGRESS.md](../PROGRESS.md) | Day log: what was done, verified, still open. |
| [../IMPROVEMENT_AUDIT.md](../IMPROVEMENT_AUDIT.md) | Audit findings and their status. |
| [../adr/](../adr/) | Architecture decisions (why). |
| [../Production.md](../Production.md), [Phase 0 spec](../Memora_Studio_Phase_0_Technical_Spec_and_Claude_Prompt.md) | Product specification and phase gates. |

## Source of truth

**Code > manifests (`models.json`, `sounds.json`) > these system docs > ADRs > PROGRESS / overview.**
When a doc disagrees with the code, the code wins and the doc is fixed (or the disagreement is reported
when the code looks wrong).

## Writing rules

- English; dense tables and bullets; relative links to sources (`../../apps/desktop/src-tauri/src/x.rs`).
- Facts from the code only. Never invent numbers; label assumptions ("initial value", "assumed").
- Headings the checker relies on: `### <module>.rs` in BACKEND.md, `### <File>.tsx` in FRONTEND.md,
  every Tauri command name and job kind written in backticks in BACKEND.md, every migration number in
  DATABASE.md, every worker job name in WORKER.md.
- Update the `Last verified` line of every file you touch.
