"""Probe: does a thread blocked on stdin (open pipe, no data) block work on the main thread?

Run: .venv/Scripts/python.exe probe_stdin_block.py
Spawns itself as a child with stdin=PIPE kept open and reports which steps finish.
"""
import subprocess
import sys
import time

if len(sys.argv) > 1:  # child
    import threading

    def say(s):
        sys.stdout.write(s + "\n")
        sys.stdout.flush()

    threading.Thread(target=lambda: sys.stdin.readline(), daemon=True).start()
    time.sleep(0.2)  # let the reader block first
    say("after-reader-start")
    if sys.argv[1] == "numpy":
        import numpy  # noqa: F401
        say("numpy imported")
    if sys.argv[1] in ("openvino", "infer"):
        import openvino as ov
        say("openvino imported")
        core = ov.Core()
        say("core created: " + ",".join(core.available_devices))
        if sys.argv[1] == "infer":
            import numpy as np
            import openvino.opset13 as ops
            x = ops.parameter([1, 4], np.float32)
            m = ov.Model([ops.relu(x)], [x])
            for dev in core.available_devices:
                r = core.compile_model(m, dev).create_infer_request().infer({0: np.ones((1, 4), np.float32)})
                say(f"infer ok on {dev}")
    say("done")
    sys.exit(0)

for case in ("plain", "numpy", "openvino", "infer"):
    p = subprocess.Popen([sys.executable, "-u", __file__, case], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         text=True)
    import threading
    lines = []
    th = threading.Thread(target=lambda: lines.extend(iter(p.stdout.readline, "")), daemon=True)
    th.start()
    th.join(20)
    hung = th.is_alive()
    p.kill()
    print(f"{case:9} {'HUNG' if hung else 'ok  '} lines={lines and [l.strip() for l in lines]}")
