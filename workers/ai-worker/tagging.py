"""OpenCLIP scene tagging (Phase 5): image encoder as OpenVINO IR + precomputed bilingual tag embeddings
(openclip_tags.json, made by tools/clip-export/export_openclip.py). Samples the preview proxy every
`every_ms`, returns the top tags per sample. Observations only; the Editing Core decides what to use.

Self-check: python tagging.py
"""
import json
import time
from pathlib import Path

import cv2
import numpy as np

TOP_K = 5
_compiled = {}  # (xml, device) -> compiled model, kept for the worker's lifetime


def preprocess(bgr, p):
    """Same as open_clip: shortest side → 224 (bicubic), center crop, CLIP mean/std. Returns 1x3x224x224."""
    rgb = cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB)
    h, w = rgb.shape[:2]
    s = p["size"] / min(h, w)
    rgb = cv2.resize(rgb, (max(p["size"], round(w * s)), max(p["size"], round(h * s))), interpolation=cv2.INTER_CUBIC)
    h, w = rgb.shape[:2]
    y, x = (h - p["size"]) // 2, (w - p["size"]) // 2
    img = rgb[y:y + p["size"], x:x + p["size"]].astype(np.float32) / 255.0
    img = (img - np.array(p["mean"], np.float32)) / np.array(p["std"], np.float32)
    return img.transpose(2, 0, 1)[None]


def compile_model(xml, devices):
    """First device that compiles and gives finite output wins (GPU first; CPU is the fallback)."""
    import openvino as ov

    core = ov.Core()
    errors = []
    for dev in devices:
        if dev not in core.available_devices:
            errors.append(f"{dev}: not available")
            continue
        key = (str(xml), dev)
        try:
            if key not in _compiled:
                m = core.compile_model(str(xml), dev)
                if not np.isfinite(m(np.zeros((1, 3, 224, 224), np.float32))[0]).all():
                    raise RuntimeError("non-finite output")
                _compiled[key] = m
            return _compiled[key], dev, errors
        except Exception as e:  # fall through to the next device
            errors.append(f"{dev}: {type(e).__name__}: {e}"[:200])
    raise RuntimeError("No device could run the model: " + "; ".join(errors))


class Tagger:
    """The image encoder compiled once plus the tag / cue embeddings of one model folder."""

    def __init__(self, model_dir, devices=("GPU", "CPU")):
        model_dir = Path(model_dir)
        self.meta = json.loads((model_dir / "openclip_tags.json").read_text(encoding="utf-8"))
        self.text = np.array([t["embedding"] for t in self.meta["tags"]], np.float32)
        self.scale = float(self.meta["logitScale"])
        self.cues = self.meta.get("cues", [])  # v3+: camera / scene cues, each a prompt and its contrast
        self.cue_pairs = [np.array([c["embedding"], c["contrast"]], np.float32) for c in self.cues]
        self.model, self.device, self.fallbacks = compile_model(model_dir / "openclip_image.xml", devices)

    def sample(self, bgr, at_ms):
        v = self.model(preprocess(bgr, self.meta["preprocess"]))[0][0]
        v = v / np.linalg.norm(v)
        logits = self.text @ v * self.scale
        prob = np.exp(logits - logits.max())
        prob /= prob.sum()
        top = prob.argsort()[::-1][:TOP_K]
        return {"atMs": at_ms, "tags": [[int(k), round(float(prob[k]), 4)] for k in top],
                "cues": [round(cue_score(pair, v, self.scale), 4) for pair in self.cue_pairs]}

    def result(self, samples, every_ms, seconds):
        return {
            "model": self.meta["model"], "device": self.device, "fallbacks": self.fallbacks, "everyMs": every_ms,
            "seconds": round(seconds, 2),
            "tags": [{"en": t["en"], "vi": t["vi"], "negative": bool(t.get("negative"))} for t in self.meta["tags"]],
            "cues": [{"en": c["en"], "vi": c["vi"]} for c in self.cues],
            "samples": samples,
        }


def tag_image(bgr, model_dir, devices=("GPU", "CPU")):
    """Same result shape as tag_video with one sample at 0 ms (photos)."""
    t0, tagger = time.perf_counter(), Tagger(model_dir, devices)
    return tagger.result([tagger.sample(bgr, 0)], 0, time.perf_counter() - t0)


def tag_video(path, model_dir, every_ms=2000, devices=("GPU", "CPU"), progress=None, check_cancel=None):
    tagger = Tagger(model_dir, devices)
    cap = cv2.VideoCapture(str(path))
    if not cap.isOpened():
        raise RuntimeError(f"Cannot open {path}")
    duration_ms = cap.get(cv2.CAP_PROP_FRAME_COUNT) / max(cap.get(cv2.CAP_PROP_FPS), 1e-6) * 1000
    times = list(range(0, max(int(duration_ms), 1), every_ms))
    samples, t0 = [], time.perf_counter()
    for i, at in enumerate(times):
        if check_cancel and i % 10 == 0:
            check_cancel()
        cap.set(cv2.CAP_PROP_POS_MSEC, at)
        ok, frame = cap.read()
        if not ok:
            continue
        samples.append(tagger.sample(frame, at))
        if progress:
            progress(i + 1, len(times))
    cap.release()
    return tagger.result(samples, every_ms, time.perf_counter() - t0)


def cue_score(pair, v, scale):
    """Probability of the cue prompt against its contrast (two-way softmax) for one normalised image embedding."""
    a, b = (pair @ v * scale).astype(np.float64)
    return float(1.0 / (1.0 + np.exp(min(b - a, 700.0))))


if __name__ == "__main__":
    p = {"size": 224, "mean": [0.48145466, 0.4578275, 0.40821073], "std": [0.26862954, 0.26130258, 0.27577711]}
    for shape in [(540, 960, 3), (960, 540, 3), (100, 100, 3)]:
        x = preprocess(np.full(shape, 128, np.uint8), p)
        assert x.shape == (1, 3, 224, 224) and x.dtype == np.float32, (shape, x.shape)
    assert abs(float(preprocess(np.full((300, 400, 3), 255, np.uint8), p)[0, 0, 0, 0]) - (1 - p["mean"][0]) / p["std"][0]) < 1e-5
    pair = np.array([[1.0, 0.0], [0.0, 1.0]], np.float32)
    assert cue_score(pair, np.array([1.0, 0.0], np.float32), 100.0) > 0.99
    assert cue_score(pair, np.array([0.0, 1.0], np.float32), 100.0) < 0.01
    assert abs(cue_score(pair, np.array([0.7071, 0.7071], np.float32), 100.0) - 0.5) < 1e-3
    print("tagging self-check OK")
