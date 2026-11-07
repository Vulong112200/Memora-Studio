"""Spike 1b-2: OpenVINO compile + inference on CPU / GPU / NPU with an in-code model.

No downloads. Run with the worker venv:
    spikes/ai-worker/.venv/Scripts/python.exe spikes/openvino-devices/run.py
The model is a small MLP (static shape) roughly the size of a CLIP projection head,
so this proves runtime/device execution, not real model compatibility or speed.
"""
import json
import sys
import time

import numpy as np
import openvino as ov
import openvino.opset13 as ops

BATCH, DIM, HIDDEN, LAYERS, RUNS = 8, 768, 3072, 4, 50
rng = np.random.default_rng(0)
weights = [(rng.standard_normal((DIM if i == 0 else HIDDEN, HIDDEN if i < LAYERS - 1 else DIM)) * 0.02)
           .astype(np.float32) for i in range(LAYERS)]

x = ops.parameter([BATCH, DIM], np.float32, name="x")
h = x
for i, w in enumerate(weights):
    h = ops.matmul(h, ops.constant(w), False, False)
    if i < LAYERS - 1:
        h = ops.relu(h)
model = ov.Model([h], [x], "mlp_spike")

inp = rng.standard_normal((BATCH, DIM)).astype(np.float32)
ref = inp
for i, w in enumerate(weights):
    ref = ref @ w
    if i < LAYERS - 1:
        ref = np.maximum(ref, 0)

core = ov.Core()
results = {"openvino": ov.__version__, "devices": {}}
for dev in core.available_devices:
    r = {"name": core.get_property(dev, "FULL_DEVICE_NAME")}
    try:
        r["optimization_capabilities"] = list(core.get_property(dev, "OPTIMIZATION_CAPABILITIES"))
    except Exception:
        pass
    try:
        t = time.perf_counter()
        compiled = core.compile_model(model, dev)
        r["compile_s"] = round(time.perf_counter() - t, 3)
        req = compiled.create_infer_request()
        req.infer({0: inp})  # warm-up
        t = time.perf_counter()
        for _ in range(RUNS):
            out = req.infer({0: inp})[0]
        r["infer_ms_avg"] = round((time.perf_counter() - t) / RUNS * 1000, 3)
        r["inference_precision"] = str(compiled.get_property("INFERENCE_PRECISION_HINT"))
        r["max_abs_err_vs_numpy"] = float(np.max(np.abs(out - ref)))
        r["rel_err_vs_numpy"] = float(np.linalg.norm(out - ref) / np.linalg.norm(ref))
        r["ok"] = r["rel_err_vs_numpy"] < 1e-2
    except Exception as e:
        r["ok"] = False
        r["error"] = f"{type(e).__name__}: {e}"[:500]
    results["devices"][dev] = r

print(json.dumps(results, indent=2))
if len(sys.argv) > 1:
    open(sys.argv[1], "w").write(json.dumps(results, indent=2))
