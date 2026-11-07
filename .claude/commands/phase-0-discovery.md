---
description: Phase 0 — read-only project review, environment inspection, and feasibility report (no installs, no file changes)
---

Execute the Phase 0 task defined in `docs/Memora_Studio_Phase_0_Technical_Spec_and_Claude_Prompt.md`, section **"13. FIRST PROMPT FOR CLAUDE CODE"**, exactly as written.

Also read `docs/Production.md` (the full specification, acting as `PRODUCT_SPEC.md`) and include it in the specification review. Report conflicts between the two documents.

Constraints (restated from the prompt and CLAUDE.md):
- Inspection and reporting only. Do not install, download, or modify anything. Do not create or edit project files.
- Only run read-only diagnostic commands. Record missing tools as missing.
- Do not invent benchmark results. Use only the statuses: Verified, Detected but untested, Theoretically supported, Unsupported, Unknown.

Write the full report (sections A–G) in **Vietnamese** in the response. Keep technical terms, commands, and versions in their original form.

End by waiting for explicit user approval before any next step.

$ARGUMENTS
