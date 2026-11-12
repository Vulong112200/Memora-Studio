"""Photo editor rendering (ADR 0006, slice B1): classical, deterministic, no model, original never written.

render_file(path, edits, max_side, out) reads the original, applies the edit stack and writes a JPEG.
Edit values are numbers from -100 to 100 (0 = unchanged); unknown keys are ignored, so older stacks still load.
Radii scale with the image size, so a 1600 px preview looks like the full-size export.
The "sky" tools enhance the photo's own sky (owner 2026-11-12: cinematic but natural, never a pasted sky).

Self-check: python photo.py
"""
import os
import time

import cv2
import numpy as np

KEYS = ("exposure", "contrast", "highlights", "shadows", "whites", "blacks", "temperature", "tint", "vibrance",
        "saturation", "clarity", "dehaze", "sharpen", "denoise", "sky", "skyWarmth", "vignette", "straighten")
# HSL bands: centre hue in degrees; each edit is [hue shift, saturation, lightness].
BANDS = {"red": 0, "orange": 30, "yellow": 60, "green": 120, "aqua": 180, "blue": 225, "purple": 270, "magenta": 315}
LOOKS = ("none", "warm", "cool", "vivid", "cinematic", "bw", "vintage")
CROPS = {"free": None, "1:1": 1.0, "4:5": 0.8, "3:2": 1.5, "16:9": 16 / 9, "9:16": 9 / 16}
REF_SIDE = 1600  # radii below are tuned for this size and scaled with the image

_cache = {}


def read(path):
    """BGR uint8 with the EXIF orientation applied (np.fromfile: Unicode paths work on Windows)."""
    img = cv2.imdecode(np.fromfile(path, np.uint8), cv2.IMREAD_COLOR)
    if img is None:
        raise ValueError(f"not a readable image: {path}")
    return img


def write_jpeg(path, rgb, quality):
    ok, buf = cv2.imencode(".jpg", cv2.cvtColor(_u8(rgb), cv2.COLOR_RGB2BGR), [cv2.IMWRITE_JPEG_QUALITY, int(quality)])
    if not ok:
        raise ValueError("JPEG encoding failed")
    buf.tofile(path)


def _u8(x):
    return (np.clip(x, 0, 1) * 255 + 0.5).astype(np.uint8)


def load(path, max_side):
    """Float RGB 0..1, at most max_side px (0 = full size). The last two decodes are cached for live previews."""
    st = os.stat(path)
    key = (path, st.st_mtime_ns, st.st_size, max_side)
    if key not in _cache:
        img = read(path)
        h, w = img.shape[:2]
        if max_side and max(h, w) > max_side:
            s = max_side / max(h, w)
            img = cv2.resize(img, (max(1, round(w * s)), max(1, round(h * s))), interpolation=cv2.INTER_AREA)
        while len(_cache) >= 2:
            _cache.pop(next(iter(_cache)))
        _cache[key] = (cv2.cvtColor(img, cv2.COLOR_BGR2RGB).astype(np.float32) / 255, (w, h))
    rgb, size = _cache[key]
    return rgb.copy(), size


def guided(I, p, r, eps):
    """He et al. guided filter (box-filter form): edge-preserving smoothing of p guided by I (both 2-D float32)."""
    k = (2 * r + 1, 2 * r + 1)
    m = lambda a: cv2.boxFilter(a, -1, k)
    mi, mp = m(I), m(p)
    a = (m(I * p) - mi * mp) / (m(I * I) - mi * mi + eps)
    b = mp - a * mi
    return m(a) * I + m(b)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def gray(x):
    return x[..., 0] * 0.2126 + x[..., 1] * 0.7152 + x[..., 2] * 0.0722


# ------------------------------------------------------------------ geometry

def geometry(x, e):
    turns = int(e.get("rotate", 0)) // 90 % 4
    if turns:
        x = np.ascontiguousarray(np.rot90(x, -turns))  # clockwise
    a = float(np.clip(float(e.get("straighten", 0) or 0), -15, 15))
    if a:
        h, w = x.shape[:2]
        t = np.radians(abs(a))
        s = np.cos(t) + np.sin(t) * max(w / h, h / w)  # zoom so no empty corners show
        M = cv2.getRotationMatrix2D((w / 2, h / 2), -a, s)
        x = cv2.warpAffine(x, M, (w, h), flags=cv2.INTER_CUBIC, borderMode=cv2.BORDER_REFLECT)
    ratio = CROPS.get(e.get("crop", "free"))
    if ratio:
        # ponytail: centred crop of a fixed aspect; a draggable crop box comes with the next UI round.
        h, w = x.shape[:2]
        cw, ch = (w, round(w / ratio)) if w / h < ratio else (round(h * ratio), h)
        y0, x0 = (h - ch) // 2, (w - cw) // 2
        x = x[y0:y0 + ch, x0:x0 + cw]
    return x


# ------------------------------------------------------------------ sky

def sky_mask(x):
    """Soft 0..1 mask of the sky. Classical (initial thresholds, checked on stills of the owner's footage; a click
    selection comes with slice B2): sky-like pixels (bright enough, not foliage, and smooth, blue or cloud-white) that form an
    unbroken run from the top of each column — a tree line or the horizon ends the sky, so water and paths below it
    are never taken."""
    h, w = x.shape[:2]
    s = 512 / max(h, w)
    small = cv2.resize(x, (max(1, round(w * s)), max(1, round(h * s))), interpolation=cv2.INTER_AREA)
    sh = small.shape[0]
    g = gray(small)
    r, gg, b = small[..., 0], small[..., 1], small[..., 2]
    hsv = cv2.cvtColor(small, cv2.COLOR_RGB2HSV)
    sat, val = hsv[..., 1], hsv[..., 2]
    texture = cv2.boxFilter(np.abs(cv2.Laplacian(cv2.GaussianBlur(g, (0, 0), 1.0), cv2.CV_32F)), -1, (5, 5))
    hue = hsv[..., 0]
    foliage = (hue > 50) & (hue < 165) & (sat > 0.15) & (gg > b)  # yellow-green to green; pale cyan sky is not foliage
    blue = (hue > 180) & (hue < 255) & (sat > 0.12)
    cloud = (sat < 0.3) & (val > 0.6)  # clouds are textured but bright and nearly colourless
    skyish = ((val > 0.25) & ~foliage & ((texture < 0.025) | blue | cloud)).astype(np.uint8)
    skyish = cv2.morphologyEx(skyish, cv2.MORPH_CLOSE, np.ones((9, 1), np.uint8)).astype(bool)  # thin wires, birds
    first = skyish.argmax(axis=0)
    starts = skyish.any(axis=0) & (first < sh // 10)  # the sky run must begin near the top edge
    above = np.arange(sh)[:, None] < first[None, :]
    run = np.cumprod(skyish | above, axis=0).astype(bool) & starts[None, :]
    # The skyline is smooth at the scale of a tree trunk or a white shirt: narrow downward spikes of the run are cut
    # back to the running median of the neighbouring columns.
    bottom = np.where(starts, run.sum(axis=0), 0).astype(np.float32)
    win = max(5, small.shape[1] // 8) | 1
    med = np.median(np.lib.stride_tricks.sliding_window_view(np.pad(bottom, win // 2, mode="edge"), win), axis=1)
    run = (np.arange(sh)[:, None] < np.minimum(bottom, med * 1.1 + 3)[None, :]) & skyish
    hard = cv2.morphologyEx(run.astype(np.uint8), cv2.MORPH_OPEN, np.ones((3, 3), np.uint8))
    if hard.mean() < 0.01:
        return np.zeros((h, w), np.float32)
    soft = np.clip(guided(g, hard.astype(np.float32), 4, 1e-2), 0, 1)
    soft *= cv2.dilate(hard, np.ones((7, 7), np.uint8))  # soft edge only next to the sky, never a leak
    return np.clip(cv2.resize(soft.astype(np.float32), (w, h), interpolation=cv2.INTER_LINEAR), 0, 1)


def enhance_sky(x, strength, warmth, k, mask):
    """Cinematic but natural: cloud detail, slightly deeper highlights, richer colour, optional warmth — only in the sky."""
    m = (mask * strength)[..., None]
    lab = cv2.cvtColor(np.clip(x, 0, 1), cv2.COLOR_RGB2Lab)
    L = lab[..., 0] / 100
    base = guided(L, L, max(2, int(20 * k)), 0.01)
    L = L + 0.9 * np.tanh((L - base) * 5) / 5  # cloud texture
    L = L - 0.1 * L * L  # graduated-filter feel: bright sky a little deeper
    lab[..., 0] = np.clip(L, 0, 1) * 100
    lab[..., 1:] *= 1.35
    lab[..., 1] += 3 * warmth
    lab[..., 2] += 10 * warmth
    sky = cv2.cvtColor(lab, cv2.COLOR_Lab2RGB)
    return x * (1 - m) + sky * m


# ------------------------------------------------------------------ tone and colour

def dehaze(x, amt, k):
    """Dark-channel prior (He et al.), refined with the guided filter; negative amounts add haze."""
    A = np.percentile(x.reshape(-1, 3), 99.9, axis=0).clip(0.3, 1).astype(np.float32)
    if amt < 0:
        return x * (1 + 0.5 * amt) - A * 0.5 * amt
    size = max(3, int(15 * k)) | 1
    dark = cv2.erode((x / A).min(axis=2), np.ones((size, size), np.uint8))
    t = 1 - 0.9 * amt * dark
    t = np.clip(guided(gray(x), t.astype(np.float32), max(4, int(30 * k)), 1e-3), 0.25, 1)
    return (x - A) / t[..., None] + A


def hsl(x, bands):
    hsv = cv2.cvtColor(np.clip(x, 0, 1), cv2.COLOR_RGB2HSV)  # float: H 0..360, S and V 0..1
    H, S, V = hsv[..., 0], hsv[..., 1], hsv[..., 2]
    dh, ds, dv = np.zeros_like(H), np.zeros_like(H), np.zeros_like(H)
    for name, centre in BANDS.items():
        hs, sa, li = (np.clip(float(v), -100, 100) / 100 for v in (list(bands.get(name, [0, 0, 0])) + [0, 0, 0])[:3])
        if not (hs or sa or li):
            continue
        d = np.abs((H - centre + 180) % 360 - 180)
        wgt = np.clip(1 - d / 45, 0, 1) * smoothstep(0.05, 0.2, S)  # greys are not coloured
        dh += wgt * hs * 30
        ds += wgt * sa
        dv += wgt * li * 0.3
    hsv[..., 0] = (H + dh) % 360
    hsv[..., 1] = np.clip(S * (1 + ds), 0, 1)
    hsv[..., 2] = np.clip(V * (1 + dv), 0, 1)
    return cv2.cvtColor(hsv, cv2.COLOR_HSV2RGB)


def look(x, name):
    if name == "warm":
        return x * np.array([1.04, 1.0, 0.95], np.float32)
    if name == "cool":
        return x * np.array([0.96, 1.0, 1.05], np.float32)
    if name == "vivid":
        g = gray(x)[..., None]
        return g + (x - g) * 1.25
    if name == "cinematic":
        L = gray(x)[..., None]
        teal, orange = np.array([-0.03, 0.01, 0.04], np.float32), np.array([0.04, 0.01, -0.03], np.float32)
        return x + teal * (1 - L) ** 2 + orange * L ** 2
    if name == "bw":
        return np.repeat(gray(x)[..., None], 3, axis=2)
    if name == "vintage":
        g = gray(x)[..., None]
        return (g + (x - g) * 0.85) * np.array([1.03, 1.0, 0.92], np.float32) * 0.94 + 0.05
    return x


def render(x, e, show_mask=False):
    """Apply the edit stack to float RGB x (already resized); returns float RGB 0..1."""
    v = lambda k: float(np.clip(float(e.get(k, 0) or 0), -100, 100)) / 100
    x = geometry(x, e)
    k = max(x.shape[:2]) / REF_SIDE
    if v("denoise") > 0:
        n = v("denoise")
        ycc = cv2.cvtColor(np.clip(x, 0, 1), cv2.COLOR_RGB2YCrCb)
        for c in (1, 2):  # colour noise: strong smoothing of chroma, edges kept by the luma guide
            ycc[..., c] = guided(ycc[..., 0], ycc[..., c], max(1, int(4 * k * n + 1)), 1e-3)
        ycc[..., 0] = guided(ycc[..., 0], ycc[..., 0], max(1, int(2 * k)), (0.03 * n) ** 2)
        x = cv2.cvtColor(ycc, cv2.COLOR_YCrCb2RGB)
    t, tn = v("temperature"), v("tint")
    if t or tn:
        f = np.array([1 + 0.2 * t, 1 - 0.12 * tn, 1 - 0.2 * t], np.float32)
        x = x * (f / (f @ np.array([0.2126, 0.7152, 0.0722], np.float32)))
    if v("exposure"):
        x = (np.clip(x, 0, None) ** 2.2 * 2 ** (2 * v("exposure"))) ** (1 / 2.2)
    if v("dehaze"):
        x = dehaze(np.clip(x, 0, None), v("dehaze"), k)
    # Highlight roll-off instead of a hard clip (brightening never burns out skies).
    x = np.where(x > 0.85, 0.85 + 0.15 * np.tanh((x - 0.85) / 0.15), x).astype(np.float32)

    if any(v(n) for n in ("shadows", "highlights", "blacks", "whites", "contrast", "clarity", "sharpen", "saturation", "vibrance")):
        lab = cv2.cvtColor(np.clip(x, 0, 1), cv2.COLOR_RGB2Lab)
        L = lab[..., 0] / 100
        if v("shadows") or v("highlights"):
            base = guided(L, L, max(2, int(16 * k)), 0.01)  # local brightness, so detail stays
            sh, hi = v("shadows"), v("highlights")
            w_sh = np.clip(1 - base / 0.5, 0, 1) ** 2
            w_hi = np.clip((base - 0.5) / 0.5, 0, 1) ** 2
            L = L + 0.35 * sh * w_sh * (1 - L if sh > 0 else L) + 0.35 * hi * w_hi * (1 - L if hi > 0 else L)
        b, wh = v("blacks"), v("whites")
        if b > 0:
            L = 0.1 * b + L * (1 - 0.1 * b)
        elif b < 0:
            L = (L + 0.1 * b) / (1 + 0.1 * b)
        if wh:
            L = L / (1 - 0.1 * wh)
        c = v("contrast")
        if c > 0:
            s = lambda y: 1 / (1 + np.exp(-(y - 0.5) * 8))
            L = L + c * ((s(L) - s(0)) / (s(1) - s(0)) - L)
        elif c < 0:
            L = 0.5 + (L - 0.5) * (1 + 0.5 * c)
        if v("clarity"):
            base = guided(L, L, max(2, int(24 * k)), 0.02)
            L = L + v("clarity") * 1.2 * np.tanh((L - base) * 4) / 4  # bounded, so no halos
        L = L.astype(np.float32)
        if v("sharpen") > 0:
            blur = cv2.GaussianBlur(L, (0, 0), max(0.8, 1.0 * k))
            smooth = cv2.GaussianBlur(L, (0, 0), max(1.5, 2.0 * k))
            grad = cv2.magnitude(cv2.Sobel(smooth, cv2.CV_32F, 1, 0), cv2.Sobel(smooth, cv2.CV_32F, 0, 1))
            edges = smoothstep(0.02, 0.08, grad / max(k, 0.25))  # flat or noisy areas are not sharpened
            L = L + v("sharpen") * 1.5 * (L - blur) * edges
        lab[..., 0] = np.clip(L, 0, 1) * 100
        sat, vib = v("saturation"), v("vibrance")
        if sat or vib:
            a, bb = lab[..., 1], lab[..., 2]
            chroma = np.sqrt(a * a + bb * bb)
            hue = np.degrees(np.arctan2(bb, a)) % 360
            skin = np.clip(1 - np.abs(hue - 55) / 25, 0, 1)  # protect skin tones from vibrance
            f = (1 + sat) * (1 + vib * np.clip(1 - chroma / 60, 0, 1) * (1 - 0.6 * skin))
            lab[..., 1:] *= np.clip(f, 0, 3)[..., None]
        x = cv2.cvtColor(lab, cv2.COLOR_Lab2RGB)

    if e.get("hsl"):
        x = hsl(x, e["hsl"])
    mask = None
    if v("sky") > 0 or show_mask:
        mask = sky_mask(x)
    if v("sky") > 0:
        x = enhance_sky(x, v("sky"), v("skyWarmth"), k, mask)
    x = look(x, e.get("look", "none"))
    if v("vignette"):
        h, w = x.shape[:2]
        yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
        r = np.sqrt(((xx - w / 2) / (w / 2)) ** 2 + ((yy - h / 2) / (h / 2)) ** 2) / np.sqrt(2)
        x = x * (1 - 0.6 * v("vignette") * smoothstep(0.35, 1.0, r))[..., None]
    if show_mask and mask is not None:
        m = (0.55 * mask)[..., None]
        x = x * (1 - m) + np.array([1.0, 0.25, 0.6], np.float32) * m
    return np.clip(x, 0, 1).astype(np.float32)


def local_offset():
    """This computer's UTC offset as "+07:00" (cameras write local time without a zone)."""
    z = time.strftime("%z") or "+0000"
    return f"{z[:3]}:{z[3:5]}"


def taken(path):
    """EXIF DateTimeOriginal of a JPEG as ISO with its zone, "2026-02-20T10:15:00+07:00": OffsetTimeOriginal (0x9011)
    when the camera wrote it, else this computer's offset (photos are usually sorted where they were taken). None
    without EXIF. Minimal reader: APP1 Exif -> IFD0 -> Exif IFD (0x8769) -> 0x9003 / 0x9011."""
    import struct
    try:
        with open(path, "rb") as f:
            data = f.read(256 * 1024)
        i = data.find(b"Exif\0\0")
        if not data.startswith(b"\xff\xd8") or i < 0:
            return None
        t = data[i + 6:]
        e = "<" if t[:2] == b"II" else ">"

        def entries(off):
            n = struct.unpack(e + "H", t[off:off + 2])[0]
            for k in range(n):
                tag, typ, cnt, val = struct.unpack(e + "HHII", t[off + 2 + 12 * k: off + 14 + 12 * k])
                yield tag, cnt, val

        ifd0 = struct.unpack(e + "I", t[4:8])[0]
        sub = next((v for tag, _, v in entries(ifd0) if tag == 0x8769), None)
        exif = {tag: (c, v) for tag, c, v in entries(sub)} if sub else {}
        if exif.get(0x9003, (0, 0))[0] < 19:
            return None
        val = exif[0x9003][1]
        s = t[val:val + 19].decode("ascii")
        if s[4] != ":" or s[13] != ":":
            return None
        zone = local_offset()
        if 0x9011 in exif and exif[0x9011][0] >= 6:
            o = t[exif[0x9011][1]:exif[0x9011][1] + 6].decode("ascii")
            if o[0] in "+-" and o[3] == ":":
                zone = o
        return f"{s[0:4]}-{s[5:7]}-{s[8:10]}T{s[11:19]}{zone}"
    except Exception:
        return None


ANALYZE_SIDE = 960  # same scale as the 540p video proxies, so sharpness compares with video scenes


def analyze(path, edits, model_dir=None, devices=("GPU", "CPU")):
    """Observations of a photo as edited (the Editing Core decides): technical metrics of analysis.frame_metrics
    at 960 px, and OpenCLIP tags / cues (tagging.tag_image) when a model folder is given."""
    import analysis

    x, (w, h) = load(path, ANALYZE_SIDE)
    y = cv2.cvtColor(_u8(render(x, edits or {})), cv2.COLOR_RGB2BGR)  # 8-bit BGR, as video frames
    r = {"width": w, "height": h, "metrics": analysis.frame_metrics(cv2.cvtColor(y, cv2.COLOR_BGR2GRAY))}
    if model_dir:
        import tagging

        r["tags"] = tagging.tag_image(y, model_dir, devices)
    return r


def render_file(path, edits, max_side, out, quality=92, show_mask=False, info=False):
    t = time.perf_counter()
    x, (w, h) = load(path, max_side)
    y = render(x, edits or {}, show_mask)
    write_jpeg(out, y, quality)
    r = {"width": w, "height": h, "outWidth": y.shape[1], "outHeight": y.shape[0], "ms": round((time.perf_counter() - t) * 1000)}
    if info:
        r["taken"] = taken(path)
    return r


def auto(path):
    """Suggested starting values (shown on the sliders, the user adjusts): exposure to a mid-grey median, grey-world
    white balance at half strength (keeps a sunset's mood), contrast for flat photos, recovery of clipped ends."""
    x, _ = load(path, REF_SIDE)
    lum = gray(x)
    med = float(np.median(lum))
    stops = np.log2((0.46 ** 2.2) / max(med, 0.02) ** 2.2)
    e = {"exposure": round(float(np.clip(stops / 2 * 100 * 0.7, -60, 60)))}
    hsv = cv2.cvtColor(x, cv2.COLOR_RGB2HSV)
    mid = (lum > 0.15) & (lum < 0.85) & (hsv[..., 1] < 0.25)  # near-neutral only: foliage is not a grey card
    if mid.mean() > 0.02:
        r, g, b = (float(x[..., i][mid].mean()) for i in range(3))
        e["temperature"] = round(float(np.clip((b - r) / (0.2 * (b + r)), -0.5, 0.5)) * 50)
        e["tint"] = round(float(np.clip((g - (r + b) / 2) / (0.12 * g), -0.4, 0.4)) * 50)
    p2, p98 = np.percentile(lum, [2, 98])
    e["contrast"] = round(float(np.clip((0.75 - (p98 - p2)) * 100, 0, 30)))
    if np.percentile(lum, 99.5) > 0.97:
        e["highlights"] = -25
    if np.percentile(lum, 5) < 0.06:
        e["shadows"] = 20
    e.update({"clarity": 10, "vibrance": 15, "sharpen": 20})
    sky = float(sky_mask(x).mean())
    if sky > 0.08:
        e["sky"] = 35
    return {"edits": e, "skyShare": round(sky, 3)}


# ------------------------------------------------------------------ self-check

if __name__ == "__main__":
    import tempfile

    rng = np.random.default_rng(0)
    h, w = 300, 400
    img = np.zeros((h, w, 3), np.float32)
    img[:120] = np.linspace([0.45, 0.65, 0.95], [0.75, 0.85, 1.0], 120)[:, None, :]  # smooth blue sky on top
    img[120:] = [0.2, 0.45, 0.15]  # green ground
    img[120:] += rng.normal(0, 0.06, (h - 120, w, 1)).astype(np.float32)  # foliage texture
    img = np.clip(img, 0, 1)
    d = tempfile.mkdtemp()
    src, out = os.path.join(d, "ảnh gốc.jpg"), os.path.join(d, "kết quả.jpg")
    write_jpeg(src, img, 98)
    base, _ = load(src, 0)

    same = render(base, {})
    assert np.abs(same - base).max() < 0.06, "neutral stack keeps the picture (roll-off only touches > 0.85)"
    assert np.abs(same[150:] - base[150:]).max() < 1e-3, "midtones untouched by a neutral stack"
    assert render(base, {"exposure": 50}).mean() > base.mean() + 0.05
    assert render(base, {"exposure": -50}).mean() < base.mean() - 0.05
    assert render(base, {"exposure": 100}).max() <= 1.0
    warm = render(base, {"temperature": 60})
    assert warm[..., 0].mean() > base[..., 0].mean() and warm[..., 2].mean() < base[..., 2].mean()
    assert gray(render(base, {"saturation": -100})[:100]).std() >= 0 and np.abs(np.diff(render(base, {"saturation": -100})[60, 200])).max() < 0.02, "no colour left"

    m = sky_mask(base)
    assert m[:100].mean() > 0.8 and m[150:].mean() < 0.1, f"sky found on top only: {m[:100].mean():.2f} / {m[150:].mean():.2f}"
    sky = render(base, {"sky": 100})
    assert np.abs(sky[200:] - base[200:]).max() < 0.03, "ground unchanged by sky tools"
    assert np.abs(sky[:100] - base[:100]).mean() > 0.01, "sky enhanced"

    flat = np.full((h, w, 3), 0.5, np.float32) + rng.normal(0, 0.01, (h, w, 1)).astype(np.float32)
    flat[:, 200:] += 0.3  # one hard edge
    s = render(flat, {"sharpen": 100})
    noise_gain = (s[:, :150] - flat[:, :150]).std()
    edge_gain = np.abs(s[:, 195:205] - flat[:, 195:205]).max()
    assert noise_gain < 0.004 and edge_gain > 0.03, f"edge sharpened, flat noise left alone: {noise_gain:.4f} / {edge_gain:.3f}"

    assert render(base, {"rotate": 90}).shape[:2] == (w, h)
    assert render(base, {"crop": "1:1"}).shape[:2] == (h, h)
    assert render(base, {"straighten": 5}).shape == base.shape
    for name in LOOKS:
        assert render(base, {"look": name}).shape == base.shape
    assert render(base, {"hsl": {"blue": [0, -100, 0]}})[:100].std(axis=2).mean() < base[:100].std(axis=2).mean()
    for key in ("clarity", "dehaze", "denoise", "vignette", "shadows", "highlights", "whites", "blacks", "contrast", "vibrance"):
        for val in (-100, 100):
            r = render(base, {key: val})
            assert np.isfinite(r).all() and r.shape == base.shape, key

    info = render_file(src, {"exposure": 20, "sky": 50}, 200, out)
    assert os.path.exists(out) and info["outWidth"] == 200, info
    a = auto(src)
    assert a["skyShare"] > 0.2 and a["edits"]["sky"] == 35, a
    assert taken(src) is None and render_file(src, {}, 100, out, info=True)["taken"] is None, "no EXIF: no date"
    print("photo.py self-check OK", info, a)
