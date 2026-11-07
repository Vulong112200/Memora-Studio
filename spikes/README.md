# Phase 1b spikes (throwaway)

Results: `docs/HARDWARE_VALIDATION.md`. Run from the repo root in PowerShell.
If `ffmpeg`/`cargo` are not on PATH yet (terminal opened before install):
`$env:Path = "$env:USERPROFILE\.cargo\bin;$env:LOCALAPPDATA\Microsoft\WinGet\Links;$env:Path"`

| Spike | Command |
|---|---|
| FFmpeg / QSV | `spikes\ai-worker\.venv\Scripts\python.exe spikes\ffmpeg-qsv\run.py` (set `$env:SPIKE_SECONDS=60` for long clips) |
| OpenVINO devices | `spikes\ai-worker\.venv\Scripts\python.exe spikes\openvino-devices\run.py` |
| stdin deadlock probe | `spikes\ai-worker\.venv\Scripts\python.exe spikes\ai-worker\probe_stdin_block.py` |
| Orchestrator | `cargo build --release --manifest-path spikes\orchestrator\Cargo.toml` then `spikes\orchestrator\target\release\orchestrator-spike.exe spikes\ai-worker\.venv\Scripts\python.exe spikes\ai-worker\worker.py` |
| Orphan test | same exe with a third argument `hold`, then `taskkill /F` it and check that no `python`/`ffmpeg` remains |
| Tauri smoke | `cd spikes\tauri-smoke; npm install; npm run tauri build -- --no-bundle` |

Outputs go to `E:\Memora Data\cache\spike\`.
