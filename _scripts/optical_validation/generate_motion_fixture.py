# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate an original, deterministic no-gyro video and its exact camera path."""
from pathlib import Path
import argparse
import hashlib
import json
import math
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--ffmpeg", required=True)
parser.add_argument("--output", required=True)
parser.add_argument("--cut-at", type=int)
args = parser.parse_args()
out = Path(args.output).resolve()
out.mkdir(parents=True, exist_ok=True)
video = out / "translated-318x238.mp4"
if video.exists():
    raise SystemExit("Fixture already exists; inspect it before replacing")

width, height, fps, count = 318, 238, 30, 120
scene_width, scene_height = 400, 320
if args.cut_at is not None and not 0 < args.cut_at < count:
    raise SystemExit('Cut must fall inside the clip')
def make_scene(seed):
    scene = bytearray(scene_width * scene_height)
    for y in range(scene_height):
        for x in range(scene_width):
            value = ((x // 4) * 73856093) ^ ((y // 4) * 19349663) ^ (seed * 83492791)
            value ^= value >> 13
            scene[y * scene_width + x] = ((value * 1274126177) & 0xFFFFFFFF) >> 24
    return scene
scenes = [make_scene(0), make_scene(41)]

command = [args.ffmpeg, "-hide_banner", "-loglevel", "error", "-n",
           "-f", "rawvideo", "-pixel_format", "gray", "-video_size", f"{width}x{height}",
           "-framerate", str(fps), "-i", "pipe:0", "-an", "-c:v", "libx264",
           "-crf", "18", "-preset", "veryfast", "-pix_fmt", "yuv420p", str(video)]
path = []
raw_hash = hashlib.sha256()
with (out / "encode.stderr.log").open("wb") as log:
    encoder = subprocess.Popen(command, stdin=subprocess.PIPE, stderr=log)
    try:
        for frame in range(count):
            after_cut = args.cut_at is not None and frame >= args.cut_at
            local_frame = frame - args.cut_at if after_cut else frame
            scene = scenes[int(after_cut)]
            x = round(32 + local_frame * 0.1 + 7 * math.sin(2 * math.pi * local_frame / 9))
            y = round(32 + 4 * math.sin(2 * math.pi * local_frame / 13))
            pixels = b"".join(scene[(y + row) * scene_width + x:(y + row) * scene_width + x + width]
                              for row in range(height))
            assert len(pixels) == width * height
            raw_hash.update(pixels)
            encoder.stdin.write(pixels)
            path.append({"frame": frame, "timestamp_ms": frame * 1000 / fps,
                         "camera_origin": [x, y], "intended_pan_x": 32 + local_frame * 0.1, "scene": int(after_cut)})
    finally:
        encoder.stdin.close()
    if encoder.wait(timeout=60) != 0:
        raise SystemExit("FFmpeg encoding failed; inspect encode.stderr.log")

lens = {"name": "Synthetic perspective 318x238", "camera_brand": "Synthetic",
        "camera_model": "Deterministic translated crop", "lens_model": "Pinhole",
        "calibrated_by": "Original test fixture", "calibrator_version": "1.6.3",
        "calib_dimension": {"w": width, "h": height}, "orig_dimension": {"w": width, "h": height},
        "fps": fps, "input_horizontal_stretch": 1, "input_vertical_stretch": 1,
        "global_shutter": True, "distortion_model": "opencv_standard",
        "fisheye_params": {"camera_matrix": [[400, 0, width / 2], [0, 400, height / 2], [0, 0, 1]],
                           "distortion_coeffs": [0, 0, 0, 0, 0]}}
(out / "lens.json").write_text(json.dumps(lens, indent=2) + "\n")
(out / "motion.json").write_text(json.dumps({"width": width, "height": height, "fps": fps,
    "frames": path, "scene_cut_frames": [args.cut_at] if args.cut_at is not None else [], "raw_frames_sha256": raw_hash.hexdigest(),
    "scope": "Original translated texture with deliberate pan plus jitter; no gyro telemetry or rolling shutter. "
             "This is a rendering smoke fixture, not representative-footage validation."}, indent=2) + "\n")
print(json.dumps({"video": str(video), "frames": count, "duration_seconds": count / fps,
                  "sha256": hashlib.sha256(video.read_bytes()).hexdigest()}))
