"""Spike AI worker: JSON-lines over stdio, protocol v1. Single-threaded by design.

Messages in:  health | job.start {id, kind: steps|crash|ov_infer, ...} | job.cancel {id} | shutdown
Messages out: hello | health.ok | job.progress | job.result | job.cancelled | job.error | error

Why single-threaded: on Windows, a thread blocked reading stdin (open pipe, no data) makes
`import numpy` / `import openvino` on another thread hang (see probe_stdin_block.py).
So stdin is only read with a blocking call while idle; during a job it is polled with
PeekNamedPipe between batches. Large data (frames, audio) is passed by file path, never here.
"""
import ctypes
import json
import msvcrt
import os
import sys
import time
from collections import deque
from ctypes import wintypes

PROTOCOL = 1
t_start = time.perf_counter()
FD = sys.stdin.fileno()
_k32 = ctypes.WinDLL("kernel32", use_last_error=True)
_k32.PeekNamedPipe.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p,
                               ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
_HIN = msvcrt.get_osfhandle(FD)
_buf = b""


class ParentGone(Exception):
    pass


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def _stdin_has_data():
    avail = wintypes.DWORD()
    if not _k32.PeekNamedPipe(_HIN, None, 0, None, ctypes.byref(avail), None):
        return True  # broken pipe: let os.read report EOF
    return avail.value > 0


def poll(block):
    """Return complete messages read from stdin. Raise ParentGone on EOF."""
    global _buf
    if block or _stdin_has_data():
        chunk = os.read(FD, 65536)
        if not chunk:
            raise ParentGone
        _buf += chunk
    *lines, _buf = _buf.split(b"\n")
    msgs = []
    for line in lines:
        if line.strip():
            try:
                msgs.append(json.loads(line.decode("utf-8-sig")))
            except ValueError:
                send({"type": "error", "error": "invalid json"})
    return msgs


pending = deque()


def cancel_requested(jid):
    """Called between batches: handle control messages, queue everything else."""
    hit = False
    for m in poll(block=False):
        if m.get("type") == "job.cancel" and m.get("id") == jid:
            hit = True
        elif m.get("type") == "health":
            send({"type": "health.ok", "id": m.get("id"), "busy": jid})
        else:
            pending.append(m)
    return hit


def runtimes():
    try:
        import openvino as ov
        return {"openvino": ov.__version__.split("-")[0], "devices": ov.Core().available_devices}
    except Exception as e:  # worker must start even without OpenVINO
        return {"openvino": None, "devices": ["CPU"], "runtime_error": str(e)}


def job_steps(jid, msg):
    steps = int(msg.get("steps", 10))
    for i in range(steps):
        if cancel_requested(jid):
            return send({"type": "job.cancelled", "id": jid, "done": i})
        time.sleep(0.05)  # stand-in for one inference batch
        send({"type": "job.progress", "id": jid, "done": i + 1, "total": steps})
    send({"type": "job.result", "id": jid, "result": {"steps": steps}})


def job_ov_infer(jid, msg):
    """Real OpenVINO inference on the requested device, cancellable between iterations."""
    import numpy as np
    import openvino as ov
    import openvino.opset13 as ops

    device, iters = msg.get("device", "CPU"), int(msg.get("iterations", 20))
    rng = np.random.default_rng(0)
    x = ops.parameter([8, 768], np.float32)
    h = ops.relu(ops.matmul(x, ops.constant(rng.standard_normal((768, 3072)).astype(np.float32) * 0.02), False, False))
    h = ops.matmul(h, ops.constant(rng.standard_normal((3072, 768)).astype(np.float32) * 0.02), False, False)
    t = time.perf_counter()
    req = ov.Core().compile_model(ov.Model([h], [x]), device).create_infer_request()
    compile_ms = round((time.perf_counter() - t) * 1000)
    data = rng.standard_normal((8, 768)).astype(np.float32)
    t = time.perf_counter()
    for i in range(iters):
        if cancel_requested(jid):
            return send({"type": "job.cancelled", "id": jid, "done": i})
        req.infer({0: data})
        send({"type": "job.progress", "id": jid, "done": i + 1, "total": iters})
    send({"type": "job.result", "id": jid, "result": {
        "device": device, "compile_ms": compile_ms,
        "infer_ms_avg": round((time.perf_counter() - t) / iters * 1000, 3)}})


JOBS = {"steps": job_steps, "ov_infer": job_ov_infer}

send({"type": "hello", "protocol": PROTOCOL, "pid": os.getpid(), "python": sys.version.split()[0],
      **runtimes(), "startup_ms": round((time.perf_counter() - t_start) * 1000)})

try:
    while True:
        if not pending:
            pending.extend(poll(block=True))
            continue
        msg = pending.popleft()
        kind, jid = msg.get("type"), msg.get("id")
        if kind == "shutdown":
            break
        if kind == "health":
            send({"type": "health.ok", "id": jid})
        elif kind == "job.cancel":
            pass  # job already finished
        elif kind != "job.start":
            send({"type": "error", "id": jid, "error": f"unknown message type: {kind}"})
        elif msg.get("kind") == "crash":
            os._exit(3)  # simulate a native crash inside inference
        elif msg.get("kind") in JOBS:
            try:
                JOBS[msg["kind"]](jid, msg)
            except ParentGone:
                raise
            except Exception as e:  # a failed job must not kill the worker
                send({"type": "job.error", "id": jid, "error": f"{type(e).__name__}: {e}"[:500]})
        else:
            send({"type": "job.error", "id": jid, "error": f"unknown job kind: {msg.get('kind')}"})
except ParentGone:
    pass  # parent closed stdin: exit quietly
