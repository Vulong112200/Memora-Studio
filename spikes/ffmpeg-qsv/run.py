"""Spike 1b-1: real FFmpeg encode/decode tests (software vs Intel QSV).

Stdlib only. Run with any Python 3.10+:
    python spikes/ffmpeg-qsv/run.py [OUT_DIR]
Default OUT_DIR: E:\\Memora Data\\cache\\spike\\ffmpeg
Writes results.json there and prints a table. Synthetic input only (testsrc2).
"""
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

LINKS = Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft" / "WinGet" / "Links"


def tool(name):
    found = shutil.which(name) or (LINKS / f"{name}.exe")
    if not Path(found).exists():
        sys.exit(f"{name} not found")
    return str(found)


FFMPEG, FFPROBE = tool("ffmpeg"), tool("ffprobe")
OUT = Path(sys.argv[1] if len(sys.argv) > 1 else r"E:\Memora Data\cache\spike\ffmpeg")
OUT.mkdir(parents=True, exist_ok=True)
SECONDS, FPS = int(os.environ.get("SPIKE_SECONDS", "10")), 30
FRAMES = SECONDS * FPS


def run(args):
    t = time.perf_counter()
    p = subprocess.run([FFMPEG, "-hide_banner", "-y", *args], capture_output=True, text=True, errors="replace")
    return p.returncode, time.perf_counter() - t, p.stderr.strip().splitlines()[-3:]


def probe(path):
    p = subprocess.run(
        [FFPROBE, "-v", "error", "-select_streams", "v:0", "-count_frames",
         "-show_entries", "stream=codec_name,profile,pix_fmt,width,height,nb_read_frames:format=duration",
         "-of", "json", str(path)],
        capture_output=True, text=True)
    if p.returncode:
        return None
    d = json.loads(p.stdout)
    s = d["streams"][0]
    return {**s, "duration": float(d["format"]["duration"]), "nb_read_frames": int(s["nb_read_frames"])}


def source(name, size, codec_args):
    path = OUT / name
    code, secs, tail = run(["-f", "lavfi", "-i", f"testsrc2=size={size}:rate={FPS}:duration={SECONDS}",
                            *codec_args, str(path)])
    if code:
        sys.exit(f"source {name} failed: {tail}")
    return path


SRC = source("src_1080p_h264.mp4", "1920x1080", ["-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p"])
SRC_V = source("src_1080x1920_h264.mp4", "1080x1920", ["-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p"])
SRC_10 = source("src_1080p_hevc10.mp4", "1920x1080", ["-c:v", "libx265", "-crf", "20", "-pix_fmt", "yuv420p10le",
                                                      "-x265-params", "log-level=error"])

# (name, input args, input file, output args, output ext). Output None -> decode-only to null.
TESTS = [
    ("enc libx264 (sw)",         [], SRC, ["-c:v", "libx264", "-preset", "medium", "-crf", "20"], "mp4"),
    ("enc h264_qsv",             [], SRC, ["-c:v", "h264_qsv", "-global_quality", "23"], "mp4"),
    ("enc hevc_qsv",             [], SRC, ["-c:v", "hevc_qsv", "-global_quality", "25"], "mp4"),
    ("enc hevc_qsv main10",      [], SRC_10, ["-c:v", "hevc_qsv", "-profile:v", "main10", "-pix_fmt", "p010le"], "mp4"),
    ("enc av1_qsv",              [], SRC, ["-c:v", "av1_qsv", "-global_quality", "25"], "mp4"),
    ("enc libsvtav1 (sw)",       [], SRC, ["-c:v", "libsvtav1", "-preset", "8", "-crf", "32"], "mp4"),
    ("enc h264_qsv vertical",    [], SRC_V, ["-c:v", "h264_qsv", "-global_quality", "23"], "mp4"),
    ("full-hw h264 dec+enc qsv", ["-hwaccel", "qsv", "-hwaccel_output_format", "qsv", "-c:v", "h264_qsv"], SRC,
     ["-c:v", "h264_qsv", "-global_quality", "23"], "mp4"),
    ("dec h264 (sw)",            [], SRC, None, None),
    ("dec h264_qsv",             ["-hwaccel", "qsv", "-c:v", "h264_qsv"], SRC, None, None),
    ("dec hevc 10-bit (sw)",     [], SRC_10, None, None),
    ("dec hevc_qsv 10-bit",      ["-hwaccel", "qsv", "-c:v", "hevc_qsv"], SRC_10, None, None),
]

results = []
for i, (name, in_args, src, out_args, ext) in enumerate(TESTS):
    if out_args is None:
        code, secs, tail = run([*in_args, "-i", str(src), "-f", "null", "-"])
        info, ok = None, code == 0
    else:
        out = OUT / f"t{i:02d}.{ext}"
        code, secs, tail = run([*in_args, "-i", str(src), *out_args, "-an", str(out)])
        info = probe(out) if code == 0 else None
        ok = bool(info) and info["nb_read_frames"] == FRAMES
    results.append({"test": name, "ok": ok, "exit": code, "seconds": round(secs, 2),
                    "fps": round(FRAMES / secs, 1) if ok else None, "output": info,
                    "error": None if ok else tail})

ver = subprocess.run([FFMPEG, "-hide_banner", "-version"], capture_output=True, text=True).stdout.splitlines()[0]
(OUT / "results.json").write_text(json.dumps({"ffmpeg": ver, "frames": FRAMES, "results": results}, indent=2))

print(ver)
print(f"{'test':28} {'ok':5} {'sec':>6} {'fps':>7}  output")
for r in results:
    o = r["output"]
    desc = f"{o['codec_name']} {o.get('profile','')} {o['pix_fmt']} {o['width']}x{o['height']} {o['nb_read_frames']}f" if o else ""
    print(f"{r['test']:28} {str(r['ok']):5} {r['seconds']:6} {str(r['fps']):>7}  {desc}")
    if r["error"]:
        print("    ", " | ".join(r["error"]))
