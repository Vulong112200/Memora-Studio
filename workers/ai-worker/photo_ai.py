"""Local AI photo tools (ADR 0006 slices B2–B4). Models come from the Model Manager; their paths arrive with each
job in `MODELS` (missing key = that tool is off and its edits do nothing):

- face: YuNet 2023mar ONNX (OpenCV FaceDetectorYN) — faces in the 4 rotations (selfies taken lying down).
- landmarks: MediaPipe Face Landmarker .task → face mesh TFLite (OpenVINO CPU), 478 points on an eye-aligned crop.
- sam: [encoder, decoder] EfficientSAM-Ti ONNX (OpenVINO CPU) — click to select an object.
- inpaint: MI-GAN pipeline v2 ONNX (OpenCV DNN, CPU). OpenVINO: CPU plug-in rejects the graph, GPU output is wrong
  (spike 2026-11-12, docs/PHOTO_MODEL_PROPOSAL.md).
- depth: Depth Anything V2 Small ONNX, matte: MODNet ONNX (OpenVINO GPU, CPU fallback) — background blur.

Observations and fills only; the user decides what to apply (sliders, clicks). Self-check: python photo_ai.py
"""
import hashlib
import math
import os
import tempfile
import zipfile

import cv2
import numpy as np

MODELS = {}
_compiled = {}
_memo = {}  # expensive results by (kind, image key, args); a few entries, cleared when it grows


def _u8(x):
    return (np.clip(x, 0, 1) * 255 + 0.5).astype(np.uint8)


def image_key(x):
    return hashlib.blake2b(np.ascontiguousarray(x[::8, ::8]).tobytes() + str(x.shape).encode(), digest_size=12).hexdigest()


def memo(key, make):
    if key not in _memo:
        if len(_memo) > 24:
            _memo.clear()
        _memo[key] = make()
    return _memo[key]


def compiled(path, devices=("CPU",), shape=None):
    """OpenVINO model compiled on the first device that accepts it (kept for the worker's lifetime)."""
    import openvino as ov

    key = (str(path), tuple(devices), tuple(shape) if shape else None)
    if key not in _compiled:
        core = ov.Core()
        # Compiled blobs cached on disk: the second session skips the 10–15 s first compile (measured 2026-11-12).
        core.set_property({"CACHE_DIR": os.path.join(tempfile.gettempdir(), "memora-openvino-cache")})
        m = core.read_model(str(path))
        if shape:
            m.reshape({m.inputs[0]: list(shape)})
        err = None
        for dev in devices:
            if dev not in core.available_devices:
                continue
            try:
                _compiled[key] = core.compile_model(m, dev)
                break
            except Exception as e:  # next device
                err = e
        if key not in _compiled:
            raise RuntimeError(f"No device could run {os.path.basename(str(path))}: {err}")
    return _compiled[key]


def available():
    return {k: bool(MODELS.get(k)) for k in ("face", "landmarks", "sam", "inpaint", "depth", "matte")}


# ------------------------------------------------------------------ faces

DETECT_SIDE = 1280
_ROT = (None, cv2.ROTATE_90_CLOCKWISE, cv2.ROTATE_180, cv2.ROTATE_90_COUNTERCLOCKWISE)


def faces(x):
    """[(box (x, y, w, h), 5 landmarks [[x, y]…] (right eye, left eye, nose, mouth right, mouth left), score)] in
    pixels of float RGB x."""
    if not MODELS.get("face"):
        return []

    def find():
        h, w = x.shape[:2]
        s = min(1.0, DETECT_SIDE / max(h, w))
        small = cv2.resize(x, (max(1, round(w * s)), max(1, round(h * s))), interpolation=cv2.INTER_AREA) if s < 1 else x
        img = cv2.cvtColor(_u8(small), cv2.COLOR_RGB2BGR)
        H, W = img.shape[:2]
        back = {
            None: lambda u, v: (u, v),
            cv2.ROTATE_90_CLOCKWISE: lambda u, v: (v, H - 1 - u),
            cv2.ROTATE_180: lambda u, v: (W - 1 - u, H - 1 - v),
            cv2.ROTATE_90_COUNTERCLOCKWISE: lambda u, v: (W - 1 - v, u),
        }
        found = []
        for rot in _ROT:
            im = img if rot is None else cv2.rotate(img, rot)
            det = cv2.FaceDetectorYN.create(str(MODELS["face"]), "", (im.shape[1], im.shape[0]), 0.75, 0.3, 50)
            _, res = det.detect(im)
            f = back[rot]
            for r in (res if res is not None else []):
                corners = [f(r[0], r[1]), f(r[0] + r[2], r[1]), f(r[0], r[1] + r[3]), f(r[0] + r[2], r[1] + r[3])]
                xs, ys = [c[0] for c in corners], [c[1] for c in corners]
                found.append(((min(xs) / s, min(ys) / s, (max(xs) - min(xs)) / s, (max(ys) - min(ys)) / s),
                              [[a / s, b / s] for a, b in (f(r[4 + 2 * i], r[5 + 2 * i]) for i in range(5))], float(r[14])))
        found.sort(key=lambda f: -f[2])
        kept = []
        for f in found:  # the same face seen in two rotations: the more confident one
            if all(_iou(f[0], k[0]) < 0.3 for k in kept):
                kept.append(f)
        return kept

    return memo(("faces", image_key(x), MODELS["face"]), find)


def _iou(a, b):
    iw = max(0, min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0]))
    ih = max(0, min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1]))
    return iw * ih / max(1e-6, a[2] * a[3] + b[2] * b[3] - iw * ih)


# Face mesh index sets (MediaPipe face_mesh_connections).
OVAL = [10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 152, 148, 176, 149, 150, 136,
        172, 58, 132, 93, 234, 127, 162, 21, 54, 103, 67, 109]
LIPS = [61, 146, 91, 181, 84, 17, 314, 405, 321, 375, 291, 409, 270, 269, 267, 0, 37, 39, 40, 185]
EYE_L = [263, 249, 390, 373, 374, 380, 381, 382, 362, 398, 384, 385, 386, 387, 388, 466]
EYE_R = [33, 7, 163, 144, 145, 153, 154, 155, 133, 173, 157, 158, 159, 160, 161, 246]
BROW_L = [276, 283, 282, 295, 285, 300, 293, 334, 296, 336]
BROW_R = [46, 53, 52, 65, 55, 70, 63, 105, 66, 107]
MESH_SIDE = 256


def _mesh_model():
    task = MODELS.get("landmarks")
    if not task:
        return None
    st = os.stat(task)
    out = os.path.join(tempfile.gettempdir(), f"memora-face-mesh-{st.st_size}-{int(st.st_mtime)}.tflite")
    if not os.path.exists(out):
        with zipfile.ZipFile(task) as z:
            data = z.read(next(n for n in z.namelist() if "landmarks" in n))
        with open(out + ".part", "wb") as f:
            f.write(data)
        os.replace(out + ".part", out)
    return compiled(out, ("CPU",), (1, MESH_SIDE, MESH_SIDE, 3))


def mesh(x, face):
    """478 face points (pixels of x) on a crop turned so the eyes are level, or None when the landmark model is not
    installed or does not see a face there."""
    model = _mesh_model()
    if model is None:
        return None
    (bx, by, bw, bh), lm, _ = face
    re, le = np.array(lm[0]), np.array(lm[1])
    angle = math.degrees(math.atan2(le[1] - re[1], le[0] - re[0]))
    centre = (float(bx + bw / 2), float(by + bh / 2))
    side = 1.5 * max(bw, bh)
    M = cv2.getRotationMatrix2D(centre, angle, MESH_SIDE / side)
    M[:, 2] += np.array([MESH_SIDE / 2, MESH_SIDE / 2]) - np.array(centre)
    crop = cv2.warpAffine(_u8(x), M, (MESH_SIDE, MESH_SIDE), flags=cv2.INTER_AREA, borderMode=cv2.BORDER_REPLICATE)
    out = list(model(crop.astype(np.float32)[None] / 255).values())
    pts = max(out, key=lambda v: v.size).reshape(-1, 3)[:, :2]
    score = next((float(v.ravel()[0]) for v in out if v.size == 1 and v.ndim == 4), 1.0)
    if score < 0:  # logit: no face in the crop
        return None
    inv = cv2.invertAffineTransform(M)
    return pts @ inv[:, :2].T + inv[:, 2]


def skin_mask(x, face, pts):
    """Soft skin mask (float, size of x) of one face: the mesh's face oval without eyes, brows and lips; pixels far
    from the skin colour sampled on the cheeks (glasses, hair strands) left out. Returns (mask, eye distance)."""
    h, w = x.shape[:2]
    m = np.zeros((h, w), np.uint8)
    if pts is not None:
        ed = float(np.linalg.norm(pts[EYE_L].mean(0) - pts[EYE_R].mean(0))) + 1e-3
        cv2.fillPoly(m, [cv2.convexHull(pts[OVAL].astype(np.int32))], 255)
        grow = max(1, int(0.06 * ed))
        for part in (EYE_L, EYE_R, BROW_L, BROW_R, LIPS):
            hole = np.zeros_like(m)
            cv2.fillPoly(hole, [cv2.convexHull(pts[part].astype(np.int32))], 255)
            m[cv2.dilate(hole, np.ones((2 * grow + 1, 2 * grow + 1), np.uint8)) > 0] = 0
        cheeks = np.concatenate([pts[[50, 101, 118, 117, 123, 187, 205]], pts[[280, 330, 347, 346, 352, 411, 425]]])
    else:  # no landmark model: ellipse from the 5 YuNet points
        (bx, by, bw, bh), lm, _ = face
        re, le, nose, mr, ml = [np.array(p, np.float32) for p in lm]
        ed = float(np.linalg.norm(le - re)) + 1e-3
        mouth, eyes = (mr + ml) / 2, (re + le) / 2
        up = eyes - mouth
        angle = float(np.degrees(np.arctan2(up[0], -up[1])))
        centre = (eyes + mouth) / 2
        cv2.ellipse(m, (int(centre[0]), int(centre[1])), (int(1.05 * ed), int(1.55 * np.linalg.norm(up))), angle, 0, 360, 255, -1)
        for eye in (re, le):
            cv2.circle(m, (int(eye[0]), int(eye[1])), int(0.3 * ed), 0, -1)
        mw = float(np.linalg.norm(mr - ml))
        cv2.ellipse(m, (int(mouth[0]), int(mouth[1])), (int(0.65 * mw + 2), int(0.35 * mw + 2)), angle, 0, 360, 0, -1)
        cheeks = np.array([nose])
    ycc = cv2.cvtColor(_u8(x), cv2.COLOR_RGB2YCrCb).astype(np.float32)
    sample = np.zeros((h, w), bool)
    for p in cheeks:
        cv2.circle(sample.view(np.uint8), (int(p[0]), int(p[1])), max(2, int(0.12 * ed)), 1, -1)
    sample &= m > 0
    if sample.sum() > 20:
        cr, cb = ycc[..., 1], ycc[..., 2]
        mcr, mcb = np.median(cr[sample]), np.median(cb[sample])
        tol = max(9.0, 3.0 * float(np.median(np.abs(cr[sample] - mcr)) + np.median(np.abs(cb[sample] - mcb))))
        m[(np.abs(cr - mcr) > tol) | (np.abs(cb - mcb) > tol)] = 0
    m = cv2.morphologyEx(m, cv2.MORPH_OPEN, np.ones((3, 3), np.uint8))
    return cv2.GaussianBlur(m.astype(np.float32) / 255, (0, 0), max(1.0, 0.05 * ed)), ed


def retouch(x, blemish, smooth, light):
    """Per face: small dark / red spots (acne, marks) filled from the skin around them, skin evened and smoothed with
    its fine texture kept (frequency separation), optional gentle brightening. Eyes, brows and lips untouched; the
    face is not redrawn (identity unchanged)."""
    found = faces(x)
    if not found:
        return x
    out = x.copy()
    for face in found:
        pts = mesh(x, face)
        mask, ed = skin_mask(x, face, pts)
        ys, xs = np.where(mask > 0.02)
        if len(xs) == 0:
            continue
        pad = int(0.2 * ed) + 2
        x0, y0 = max(0, xs.min() - pad), max(0, ys.min() - pad)
        x1, y1 = min(x.shape[1], xs.max() + pad), min(x.shape[0], ys.max() + pad)
        crop, mk = out[y0:y1, x0:x1], mask[y0:y1, x0:x1]
        res = crop
        if blemish > 0:
            bgr = cv2.cvtColor(_u8(res), cv2.COLOR_RGB2BGR)
            lab = cv2.cvtColor(bgr, cv2.COLOR_BGR2Lab)
            k = max(3, int(0.22 * ed) | 1)
            L, A = lab[..., 0].astype(np.int16), lab[..., 1].astype(np.int16)
            dark = cv2.medianBlur(lab[..., 0], k).astype(np.int16) - L
            red = A - cv2.medianBlur(lab[..., 1], k).astype(np.int16)
            thr = 12 - 7 * blemish  # grey levels: stronger finds fainter spots
            spots = (((dark > thr) | (red > thr * 0.8)) & (mk > 0.5)).astype(np.uint8)
            n, lbl, st, _ = cv2.connectedComponentsWithStats(spots, connectivity=8)
            lo, hi = (0.012 * ed) ** 2, (0.14 * ed) ** 2
            bw, bh = st[1:, cv2.CC_STAT_WIDTH], st[1:, cv2.CC_STAT_HEIGHT]
            keep = np.zeros(n, bool)
            keep[1:] = (st[1:, cv2.CC_STAT_AREA] >= lo) & (st[1:, cv2.CC_STAT_AREA] <= hi) & (np.maximum(bw, bh) <= 3 * np.minimum(bw, bh) + 2)
            sm = keep[lbl].astype(np.uint8) * 255
            if sm.any():
                sm = cv2.dilate(sm, np.ones((3, 3), np.uint8), iterations=max(1, int(0.015 * ed)))
                res = cv2.cvtColor(cv2.inpaint(bgr, sm, max(2, int(0.03 * ed)), cv2.INPAINT_TELEA), cv2.COLOR_BGR2RGB).astype(np.float32) / 255
        if smooth > 0:
            from photo import guided

            r = max(2, int(0.07 * ed))
            base = np.dstack([guided(res[..., c], res[..., c], r, 0.0015) for c in range(3)])
            fine = res - cv2.GaussianBlur(res, (0, 0), max(0.8, 0.012 * ed))  # pores: kept
            smoothed = base + 0.25 * (res - base) + 0.6 * fine * smooth
            res = res + (smoothed - res) * (mk[..., None] * 0.85 * smooth)
        if light > 0:
            lifted = res + 0.12 * light * (1 - res) * np.array([1.0, 0.97, 0.94], np.float32)
            res = res + (lifted - res) * mk[..., None]
        out[y0:y1, x0:x1] = np.clip(res, 0, 1)
    return out


# ------------------------------------------------------------------ select and remove

SAM_SIDE = 1024


def select(x, points, labels=None):
    """Object mask (bool, size of x) from clicks (points 0..1; label 1 = this, 0 = not this), EfficientSAM-Ti."""
    if not MODELS.get("sam") or not points:
        return np.zeros(x.shape[:2], bool)
    enc_path, dec_path = MODELS["sam"]
    h, w = x.shape[:2]
    s = min(1.0, SAM_SIDE / max(h, w))
    sw, sh = max(1, round(w * s)), max(1, round(h * s))
    small = cv2.resize(x, (sw, sh), interpolation=cv2.INTER_AREA) if s < 1 else x
    emb = memo(("sam-emb", image_key(x), enc_path), lambda: list(compiled(enc_path)(np.ascontiguousarray(small.transpose(2, 0, 1)[None], np.float32)).values())[0])
    pts = np.array([[[[p[0] * sw, p[1] * sh] for p in points]]], np.float32)
    lab = np.array([[labels or [1] * len(points)]], np.float32)
    out = list(compiled(dec_path)([emb, pts, lab, np.array([sh, sw], np.int64)]).values())
    masks, iou = out[0], out[1]
    best = int(np.argmax(iou[0, 0]))
    m = (masks[0, 0, best] > 0).astype(np.uint8)
    return cv2.resize(m, (w, h), interpolation=cv2.INTER_NEAREST) > 0


def inpaint(x, hole):
    """Fills `hole` (bool) from its surroundings with MI-GAN (None when the model is not installed)."""
    if not MODELS.get("inpaint") or not hole.any():
        return None
    key = ("inpaint-net", MODELS["inpaint"])
    net = _compiled.get(key) or _compiled.setdefault(key, cv2.dnn.readNet(str(MODELS["inpaint"])))
    keep = np.where(hole, 0, 255).astype(np.uint8)
    net.setInput(np.ascontiguousarray(_u8(x).transpose(2, 0, 1)[None]), "image")
    net.setInput(keep[None, None].copy(), "mask")
    res = net.forward()[0].transpose(1, 2, 0).clip(0, 255).astype(np.float32) / 255
    soft = cv2.GaussianBlur(hole.astype(np.float32), (0, 0), 1.2)[..., None]
    return x * (1 - soft) + res * soft


def remove(x, objects, painted=None):
    """Removes selected objects (`objects` = [{points, labels}] clicks, each grown a little to take shadows and edges)
    and painted areas (`painted`, bool) with MI-GAN. Returns None when the inpainting model is not installed."""
    hole = np.zeros(x.shape[:2], bool) if painted is None else painted.copy()
    grow = max(2, int(0.012 * max(x.shape[:2])))
    for o in objects or []:
        m = select(x, o.get("points") or [], o.get("labels"))
        if m.any():
            hole |= cv2.dilate(m.astype(np.uint8), np.ones((2 * grow + 1, 2 * grow + 1), np.uint8)) > 0
    if not hole.any():
        return x
    return memo(("removed", image_key(x), image_key(hole[..., None].astype(np.float32))), lambda: inpaint(x, hole))


# ------------------------------------------------------------------ background blur

def _run(path, x, mean, std, side, multiple):
    h, w = x.shape[:2]
    s = side / (min(h, w) if multiple == 32 else max(h, w))
    nh, nw = max(multiple, round(h * s / multiple) * multiple), max(multiple, round(w * s / multiple) * multiple)
    inp = ((cv2.resize(x, (nw, nh), interpolation=cv2.INTER_AREA) - mean) / std).transpose(2, 0, 1)[None].astype(np.float32)
    out = list(compiled(path, ("GPU", "CPU"), (1, 3, nh, nw))(inp).values())[0]
    return cv2.resize(np.squeeze(out).astype(np.float32), (w, h), interpolation=cv2.INTER_LINEAR)


def depth(x):
    """Relative nearness 0 (far) .. 1 (near), Depth Anything V2 Small (inverse depth)."""
    def make():
        d = _run(MODELS["depth"], x, np.array([0.485, 0.456, 0.406], np.float32), np.array([0.229, 0.224, 0.225], np.float32), 518, 14)
        lo, hi = np.percentile(d, 2), np.percentile(d, 98)
        return np.clip((d - lo) / max(1e-6, hi - lo), 0, 1)
    return memo(("depth", image_key(x)), make)


def matte(x):
    """People matte 0..1, MODNet."""
    return memo(("matte", image_key(x)), lambda: np.clip(_run(MODELS["matte"], x, 0.5, 0.5, 512, 32), 0, 1))


def bokeh(x, strength):
    """Camera-like background blur: the subject (people from the matte when the photo shows faces, else the nearest
    depth) stays sharp; the rest gets a disc (lens) blur that grows with distance. Subject colours do not bleed into
    the blur (normalised convolution). Needs the depth model; the matte is used when installed."""
    if not MODELS.get("depth") or strength <= 0:
        return x
    h, w = x.shape[:2]
    near = depth(x)
    if MODELS.get("matte") and faces(x):
        subject = matte(x)
        focus = float(np.median(near[subject > 0.5])) if (subject > 0.5).any() else float(np.percentile(near, 90))
    else:
        focus = float(np.percentile(near, 92))
        subject = np.clip((near - (focus - 0.12)) / 0.08, 0, 1)
    subject = cv2.GaussianBlur(subject.astype(np.float32), (0, 0), max(1.0, 0.002 * max(h, w)))
    amount = np.clip((focus - near) / max(0.15, focus), 0, 1) * (1 - subject)  # 0 sharp .. 1 most blurred
    radius = strength * 0.025 * max(h, w)  # lens blur radius at full strength
    s = min(1.0, 14 / max(1.0, radius))  # blur on a smaller copy: same look, much faster
    sw, sh = max(1, round(w * s)), max(1, round(h * s))
    small = cv2.resize(x, (sw, sh), interpolation=cv2.INTER_AREA)
    bg = cv2.resize(1 - subject, (sw, sh), interpolation=cv2.INTER_AREA)[..., None]
    layers = [x]
    for k in (1 / 3, 2 / 3, 1.0):
        r = max(1, int(round(radius * s * k)))
        disc = np.zeros((2 * r + 1, 2 * r + 1), np.float32)
        cv2.circle(disc, (r, r), r, 1, -1)
        disc /= disc.sum()
        num = cv2.filter2D(small * bg, -1, disc, borderType=cv2.BORDER_REFLECT)
        den = cv2.filter2D(bg, -1, disc, borderType=cv2.BORDER_REFLECT)[..., None]
        layers.append(cv2.resize(num / np.maximum(den, 1e-3), (w, h), interpolation=cv2.INTER_CUBIC))
    t = amount * 3
    out = np.zeros_like(x)
    for i in range(3):
        wgt = np.clip(1 - np.abs(t - i), 0, 1)[..., None]
        out += layers[i] * wgt
    out += layers[3] * np.clip(t - 2, 0, 1)[..., None]
    return np.clip(out, 0, 1).astype(np.float32)


if __name__ == "__main__":
    img = np.full((120, 160, 3), 0.5, np.float32)
    assert faces(img) == [] and select(img, [[0.5, 0.5]]).sum() == 0, "no models: tools off"
    assert inpaint(img, np.ones((120, 160), bool)) is None and remove(img, [], None) is img and bokeh(img, 1.0) is img
    assert retouch(img, 1, 1, 1) is img
    print("photo_ai self-check OK (no models)", available())
