"""Memora Studio AI worker. JSON-lines over stdio, protocol v1. Single-threaded by design.

Messages in:  health | job.start {id, kind, params} | job.cancel {id} | shutdown
Messages out: hello | health.ok | job.progress {id, done, total} | job.result {id, result}
              | job.cancelled {id} | job.error {id, error} | error

Why single-threaded: on Windows a thread blocked reading stdin makes `import numpy` /
`import openvino` on another thread hang (docs/HARDWARE_VALIDATION.md §3). stdin is read with a
blocking call only while idle; during a job it is polled with PeekNamedPipe between batches.
Large data (frames, audio) is passed by file path, never over stdio.
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
_t_start = time.perf_counter()
_FD = sys.stdin.fileno()
_k32 = ctypes.WinDLL("kernel32", use_last_error=True)
_k32.PeekNamedPipe.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p,
                               ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
_HIN = msvcrt.get_osfhandle(_FD)
_buf = b""
_pending = deque()


class ParentGone(Exception):
    pass


class Cancelled(Exception):
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
    """Complete messages from stdin. Raises ParentGone on EOF."""
    global _buf
    if block or _stdin_has_data():
        chunk = os.read(_FD, 65536)
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


class Job:
    """Handle passed to job functions: progress reporting and cooperative cancellation."""

    def __init__(self, jid, params):
        self.id, self.params = jid, params or {}

    def progress(self, done, total):
        send({"type": "job.progress", "id": self.id, "done": done, "total": total})

    def check_cancel(self):
        """Call between batches. Answers health probes, queues other messages, raises on cancel."""
        cancel = False
        for m in poll(block=False):
            if m.get("type") == "job.cancel" and m.get("id") == self.id:
                cancel = True
            elif m.get("type") == "health":
                send({"type": "health.ok", "id": m.get("id"), "busy": self.id})
            else:
                _pending.append(m)
        if cancel:
            raise Cancelled


def runtime_info():
    try:
        import openvino as ov
        return {"openvino": ov.__version__.split("-")[0], "devices": ov.Core().available_devices}
    except Exception as e:  # the worker must start even without OpenVINO
        return {"openvino": None, "devices": [], "runtimeError": f"{type(e).__name__}: {e}"}


# ---------------------------------------------------------------- jobs

def validate_devices(job):
    """Compile and run a small in-code model on every OpenVINO device; compare with NumPy FP32.
    Proves the runtime/device path works on this machine; says nothing about real models."""
    import numpy as np
    import openvino as ov
    import openvino.opset13 as ops

    rng = np.random.default_rng(0)
    w1 = (rng.standard_normal((768, 3072)) * 0.02).astype(np.float32)
    w2 = (rng.standard_normal((3072, 768)) * 0.02).astype(np.float32)
    x = ops.parameter([8, 768], np.float32)
    model = ov.Model([ops.matmul(ops.relu(ops.matmul(x, ops.constant(w1), False, False)), ops.constant(w2),
                                 False, False)], [x])
    data = rng.standard_normal((8, 768)).astype(np.float32)
    ref = np.maximum(data @ w1, 0) @ w2

    core = ov.Core()
    devices = core.available_devices
    results = []
    job.progress(0, len(devices))
    for i, dev in enumerate(devices):
        job.check_cancel()
        r = {"device": dev, "name": core.get_property(dev, "FULL_DEVICE_NAME")}
        try:
            t = time.perf_counter()
            compiled = core.compile_model(model, dev)
            r["compileMs"] = round((time.perf_counter() - t) * 1000, 1)
            req = compiled.create_infer_request()
            req.infer({0: data})  # warm-up
            runs = 20
            t = time.perf_counter()
            for _ in range(runs):
                out = req.infer({0: data})[0]
            r["inferMs"] = round((time.perf_counter() - t) / runs * 1000, 3)
            r["precision"] = str(compiled.get_property("INFERENCE_PRECISION_HINT")).replace("<Type: '", "").rstrip("'>")
            r["relError"] = float(np.linalg.norm(out - ref) / np.linalg.norm(ref))
            r["ok"] = r["relError"] < 1e-2
        except Exception as e:
            r["ok"] = False
            r["error"] = f"{type(e).__name__}: {e}"[:300]
        results.append(r)
        job.progress(i + 1, len(devices))
    return {"openvino": ov.__version__.split("-")[0], "devices": results}


def analyze_video(job):
    """Classical analysis (scenes, sample-frame metrics, shake, dHash). params: path, framesDir, quality."""
    import analysis

    p = job.params
    return analysis.analyze(p["path"], p["framesDir"], p.get("quality", "balanced"),
                            progress=job.progress, check_cancel=job.check_cancel)


def tag_video(job):
    """OpenCLIP tags every everyMs of the proxy. params: path, modelDir, everyMs, devices."""
    import tagging

    p = job.params
    return tagging.tag_video(p["path"], p["modelDir"], p.get("everyMs", 2000), p.get("devices", ["GPU", "CPU"]),
                             progress=job.progress, check_cancel=job.check_cancel)


def validate_openvino_ir(job):
    """Model Manager load validation: read an OpenVINO IR, compile on each requested device (default
    CPU) and run one inference on zeros. Dynamic dims become 1, or 224 for image height/width."""
    import numpy as np
    import openvino as ov

    core = ov.Core()
    model = core.read_model(job.params["xml"])
    feeds, inputs = {}, []
    for i, inp in enumerate(model.inputs):
        ps = inp.get_partial_shape()
        rank = ps.rank.get_length()
        dims = [d.get_length() if d.is_static else (224 if rank == 4 and k >= 2 else 1) for k, d in enumerate(ps)]
        feeds[i] = np.zeros(dims, dtype=inp.get_element_type().to_dtype())
        inputs.append({"name": inp.get_any_name(), "shape": dims})
    results = []
    for dev in job.params.get("devices", ["CPU"]):
        job.check_cancel()
        r = {"device": dev}
        try:
            t = time.perf_counter()
            compiled = core.compile_model(model, dev)
            r["compileMs"] = round((time.perf_counter() - t) * 1000, 1)
            t = time.perf_counter()
            out = compiled.create_infer_request().infer(feeds)
            r["inferMs"] = round((time.perf_counter() - t) * 1000, 2)
            r["outputs"] = [list(v.shape) for v in out.values()]
            r["ok"] = all(np.isfinite(v).all() for v in out.values())
        except Exception as e:
            r["ok"], r["error"] = False, f"{type(e).__name__}: {e}"[:300]
        results.append(r)
    if not any(r["ok"] for r in results):
        raise RuntimeError("model failed on every device: " + "; ".join(r.get("error", "non-finite output") for r in results))
    return {"validation": "openvino-ir", "openvino": ov.__version__.split("-")[0], "inputs": inputs, "devices": results}


def photo_render(job):
    """Photo editor (ADR 0006): render an edit stack. params: path, edits, maxSide (0 = full size), out, quality,
    showMask, info (also return the EXIF date)."""
    import photo

    p = job.params
    return photo.render_file(p["path"], p.get("edits") or {}, int(p.get("maxSide", 0)), p["out"], int(p.get("quality", 92)),
                             bool(p.get("showMask")), bool(p.get("info")))


def photo_auto(job):
    """Photo editor: suggested starting values for one photo. params: path."""
    import photo

    return photo.auto(job.params["path"])


def diagnostic_steps(job):
    """Test-only: N steps of 50 ms with progress and cancellation."""
    steps = int(job.params.get("steps", 10))
    for i in range(steps):
        job.check_cancel()
        time.sleep(0.05)
        job.progress(i + 1, steps)
    return {"steps": steps}


def diagnostic_crash(job):
    """Test-only: simulate a native crash inside inference."""
    os._exit(3)


def diagnostic_block(job):
    """Test-only: one long call that never checks for cancellation (like a slow model compile)."""
    time.sleep(float(job.params.get("seconds", 30)))
    return {}


JOBS = {
    "validate_devices": validate_devices,
    "analyze_video": analyze_video,
    "validate_openvino_ir": validate_openvino_ir,
    "tag_video": tag_video,
    "photo_render": photo_render,
    "photo_auto": photo_auto,
    "diagnostic.steps": diagnostic_steps,
    "diagnostic.crash": diagnostic_crash,
    "diagnostic.block": diagnostic_block,
}


def handle(msg):
    kind, jid = msg.get("type"), msg.get("id")
    if kind == "health":
        send({"type": "health.ok", "id": jid})
    elif kind == "job.cancel":
        pass  # job already finished
    elif kind != "job.start":
        send({"type": "error", "id": jid, "error": f"unknown message type: {kind}"})
    elif msg.get("kind") not in JOBS:
        send({"type": "job.error", "id": jid, "error": f"unknown job kind: {msg.get('kind')}"})
    else:
        try:
            result = JOBS[msg["kind"]](Job(jid, msg.get("params")))
            send({"type": "job.result", "id": jid, "result": result})
        except Cancelled:
            send({"type": "job.cancelled", "id": jid})
        except ParentGone:
            raise
        except Exception as e:  # a failed job must not kill the worker
            send({"type": "job.error", "id": jid, "error": f"{type(e).__name__}: {e}"[:500]})


def main():
    send({"type": "hello", "protocol": PROTOCOL, "pid": os.getpid(), "python": sys.version.split()[0],
          **runtime_info(), "startupMs": round((time.perf_counter() - _t_start) * 1000)})
    try:
        while True:
            if not _pending:
                _pending.extend(poll(block=True))
                continue
            msg = _pending.popleft()
            if msg.get("type") == "shutdown":
                return
            handle(msg)
    except ParentGone:
        pass  # parent closed stdin: exit quietly


if __name__ == "__main__":
    main()
