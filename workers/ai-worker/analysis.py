"""Classical video analysis (docs/Production.md §5.1–5.4). Observations only — no scoring here.

One sequential decode of the (proxy) video:
  * every frame (downscaled) goes to PySceneDetect's AdaptiveDetector -> scene cuts
  * global motion between frames by phase correlation -> camera-shake estimate
  * every `interval` seconds a sample frame -> sharpness, brightness, contrast, clipping, black, dHash
After the pass, each scene gets its samples, a representative frame (nearest the middle) saved as
JPEG, and a shake value. Times are in ms of the source, so they apply to the original file too.

Run `python analysis.py` for a self-check on synthetic video (needs ffmpeg on PATH or WinGet Links).
"""
import os

import cv2
import numpy as np
from scenedetect import AdaptiveDetector
from scenedetect.common import FrameTimecode

VERSION = 1  # bump when metrics change, so old results can be recomputed
INTERVALS = {"fast": 2.0, "balanced": 1.0, "detailed": 0.5}  # seconds between sample frames
DETECT_WIDTH = 320
MOTION_WIDTH = 160
MIN_MOTION_RESPONSE = 0.1


def dhash(gray):
    """64-bit difference hash as 16 hex chars (perceptual; robust to scaling and compression)."""
    small = cv2.resize(gray, (9, 8), interpolation=cv2.INTER_AREA)
    bits = (small[:, 1:] > small[:, :-1]).flatten()
    return "%016x" % int("".join("1" if b else "0" for b in bits), 2)


def frame_metrics(gray):
    """Technical observations of one frame (8-bit grey). Values are raw, not scores."""
    mean, std = float(gray.mean()), float(gray.std())
    return {
        # Variance of the Laplacian: higher = more fine detail (sharper). Scale-dependent: compare
        # only between frames analysed at the same resolution (proxies are 540p).
        "sharpness": round(float(cv2.Laplacian(gray, cv2.CV_64F).var()), 1),
        "brightness": round(mean / 255, 4),
        "contrast": round(std / 255, 4),
        "clipLow": round(float((gray <= 10).mean()), 4),
        "clipHigh": round(float((gray >= 245).mean()), 4),
        "black": mean < 20 and std < 10,
        "dhash": dhash(gray),
    }


def shake_score(shifts, fps):
    """Jitter of global motion: std of frame-to-frame shift minus its ~0.5 s moving average,
    as a fraction of frame width. Smooth pans score low, handheld jitter high."""
    if len(shifts) < 5:
        return 0.0
    s = np.asarray(shifts, dtype=np.float64)
    k = max(3, int(fps / 2) | 1)
    kernel = np.ones(k) / k
    smooth = np.stack([np.convolve(s[:, i], kernel, mode="same") for i in range(2)], axis=1)
    return round(float(np.linalg.norm(s - smooth, axis=1).std() / MOTION_WIDTH), 5)


def analyze(path, frames_dir, quality="balanced", progress=None, check_cancel=None):
    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise RuntimeError(f"cannot open video: {path}")
    fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT)) or 1
    step = max(1, round(INTERVALS[quality] * fps))
    detector = AdaptiveDetector(min_scene_len=int(fps * 0.6))
    cuts, samples, shifts = [], [], []  # shifts: (frame, dx, dy)
    keep = {}  # frame index -> full-size BGR frame, only for samples (representatives chosen later)
    prev = None
    n = 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        h, w = frame.shape[:2]
        small = cv2.resize(frame, (DETECT_WIDTH, max(2, round(h * DETECT_WIDTH / w))), interpolation=cv2.INTER_AREA)
        cuts += [c.frame_num for c in detector.process_frame(FrameTimecode(n, fps), small)]
        tiny = cv2.cvtColor(cv2.resize(small, (MOTION_WIDTH, max(2, round(h * MOTION_WIDTH / w)))), cv2.COLOR_BGR2GRAY)
        tiny = np.float32(tiny)
        if prev is not None:
            (dx, dy), response = cv2.phaseCorrelate(prev, tiny)
            if response > MIN_MOTION_RESPONSE:  # flat frames (black, sky) give meaningless shifts
                shifts.append((n, dx, dy))
        prev = tiny
        if n % step == 0:
            m = frame_metrics(cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY))
            samples.append({"frame": n, "atMs": round(n * 1000 / fps), **m})
            keep[n] = frame
        n += 1
        if n % 30 == 0:
            if check_cancel:
                check_cancel()
            if progress:
                progress(min(n, total), total)
    cuts += [c.frame_num for c in detector.post_process(FrameTimecode(n, fps))]
    cap.release()
    if n == 0:
        raise RuntimeError("video has no decodable frames")

    bounds = [0] + sorted(set(c for c in cuts if 0 < c < n)) + [n]
    os.makedirs(frames_dir, exist_ok=True)
    scenes = []
    for i, (a, b) in enumerate(zip(bounds, bounds[1:])):
        own = [s for s in samples if a <= s["frame"] < b]
        mid = (a + b) // 2
        # ponytail: a scene shorter than the sample interval may get no sample (no frame / metrics).
        rep =min(own, key=lambda s: abs(s["frame"] - mid)) if own else None
        frame_path = None
        if rep is not None:
            frame_path = os.path.join(frames_dir, f"{i}.jpg")
            img = keep[rep["frame"]]
            fh, fw = img.shape[:2]
            cv2.imwrite(frame_path, cv2.resize(img, (320, max(2, round(fh * 320 / fw))), interpolation=cv2.INTER_AREA),
                        [cv2.IMWRITE_JPEG_QUALITY, 85])
        scenes.append({
            "index": i,
            "startMs": round(a * 1000 / fps),
            "endMs": round(b * 1000 / fps),
            "shake": shake_score([(dx, dy) for f, dx, dy in shifts if a < f < b], fps),
            "representative": rep["frame"] if rep else None,
            "framePath": frame_path,
            "samples": [{k: v for k, v in s.items() if k != "frame"} for s in own],
        })
    return {"version": VERSION, "quality": quality, "fps": round(fps, 3), "frames": n, "scenes": scenes}


# ---------------------------------------------------------------- self-check

def _selfcheck():
    import shutil
    import subprocess
    import tempfile

    ffmpeg = shutil.which("ffmpeg") or os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\WinGet\Links\ffmpeg.exe")
    d = tempfile.mkdtemp(prefix="memora-analysis-")
    try:
        # 3 scenes with hard cuts at 2 s and 4 s: test pattern, black, shaky blurred noise.
        src = os.path.join(d, "cuts.mp4")
        subprocess.run([ffmpeg, "-v", "error", "-y",
                        "-f", "lavfi", "-i", "testsrc2=s=640x360:d=2:r=30",
                        "-f", "lavfi", "-i", "color=c=black:s=640x360:d=2:r=30",
                        "-f", "lavfi", "-i", "mandelbrot=s=640x360:r=30",
                        "-filter_complex",
                        "[2]trim=duration=2,setpts=PTS-STARTPTS,crop=560:300:x='40+30*sin(n*2.1)':y='30+25*cos(n*1.7)',"
                        "scale=640:360,gblur=sigma=4,setsar=1[s];[0]setsar=1[t];[1]setsar=1[k];[t][k][s]concat=n=3:v=1",
                        "-c:v", "libx264", "-pix_fmt", "yuv420p", src], check=True)
        r = analyze(src, os.path.join(d, "frames"), "detailed")
        sc = r["scenes"]
        assert len(sc) == 3, [(s["startMs"], s["endMs"]) for s in sc]
        assert abs(sc[1]["startMs"] - 2000) <= 70 and abs(sc[2]["startMs"] - 4000) <= 70, sc
        assert all(s["black"] for s in sc[1]["samples"]), "black scene detected as black"
        assert not any(s["black"] for s in sc[0]["samples"])
        sharp = lambda s: np.median([x["sharpness"] for x in s["samples"]])
        assert sharp(sc[0]) > 5 * sharp(sc[2]), (sharp(sc[0]), sharp(sc[2]))
        assert sc[2]["shake"] > 5 * max(sc[0]["shake"], 1e-4), (sc[0]["shake"], sc[2]["shake"])
        assert sc[1]["shake"] < sc[2]["shake"] / 5, "flat black frames must not look shaky"
        assert all(os.path.exists(s["framePath"]) for s in sc)
        a, b = dhash(np.tile(np.arange(64, dtype=np.uint8), (64, 1))), dhash(np.tile(np.arange(64, dtype=np.uint8)[::-1], (64, 1)))
        assert bin(int(a, 16) ^ int(b, 16)).count("1") > 50
        print("analysis self-check OK:", [(s["startMs"], s["endMs"], s["shake"]) for s in sc])
    finally:
        shutil.rmtree(d, ignore_errors=True)


if __name__ == "__main__":
    _selfcheck()
