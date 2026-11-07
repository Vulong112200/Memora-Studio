# ADR 0002 — AI Worker packaging and IPC

**Date:** 2026-11-07  
**Status:** Accepted (project owner, 2026-11-07)  
**Evidence:** `docs/HARDWARE_VALIDATION.md` §3, §5

## Decision

1. **Packaging: embedded Python 3.13 (official python.org embeddable distribution) + pinned site-packages**, installed under `<DataRoot>\runtime\ai-worker\<version>\`. Worker code ships as plain `.py` files next to it.
2. **IPC: JSON-lines over stdio, protocol version in `hello`.** Worker is **single-threaded**: blocking stdin read only when idle, `PeekNamedPipe` polling between batches during a job.
3. **Lifecycle:** Rust spawns the worker on demand inside a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (also for FFmpeg and whisper.cpp), `CREATE_NO_WINDOW` from the GUI app, health/crash detection with restart.

## Rationale

| Criterion | Embedded Python | PyInstaller |
|---|---|---|
| Suite result | 11/11 | 11/11 |
| Warm handshake | ~444 ms (≈ dev venv) | ~593 ms |
| Interpreter binary | Signed by Python Software Foundation | Unsigned PyInstaller bootloader (common antivirus false-positive source) |
| Same interpreter as development | Yes — identical behavior | Frozen import system; needed `--collect-all openvino`, hidden-import risk |
| Updating worker code | Replace `.py` files | Rebuild whole bundle |
| Size | Larger unless pruned (433 MB incl. everything) | Smaller, only imported modules |

The size gap is reducible later (remove unused packages, `opencv` headless only, strip tests/caches). Antivirus behavior must still be tested on a machine with Defender enabled before release (open item).

## Rejected
- **Stdin reader thread in the worker** — deadlocks imports on Windows (measured).
- **Managed environment created on first run (uv/pip)** — needs network on first start; conflicts with offline install.
