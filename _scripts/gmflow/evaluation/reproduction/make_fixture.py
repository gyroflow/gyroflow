"""Generate a known-camera fixture and identical presets for both flow methods."""

import json
import argparse
from pathlib import Path
import subprocess
import cv2
import numpy as np

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument(
    "--ffmpeg",
    type=Path,
    required=True,
    help="Path to an FFmpeg executable with libx264",
)
parser.add_argument("--output-dir", type=Path, required=True)
args = parser.parse_args()
out = args.output_dir.resolve()
out.mkdir(parents=True, exist_ok=True)
width, height, fps, count = 960, 540, 30, 90
camera = np.array([[720, 0, width / 2], [0, 720, height / 2], [0, 0, 1.0]])
rng = np.random.default_rng(45)
base = cv2.GaussianBlur(
    rng.integers(30, 210, (height, width, 3), dtype=np.uint8), (0, 0), 1.3
)
for i in range(180):
    xy = tuple(rng.integers([5, 5], [width - 5, height - 5]).tolist())
    color = tuple(rng.integers(20, 235, 3).tolist())
    cv2.circle(base, xy, int(rng.integers(3, 25)), color, -1, cv2.LINE_AA)
for y in range(80, height, 90):
    for x in range(50, width, 180):
        cv2.putText(
            base,
            f"{x}:{y}",
            (x, y),
            cv2.FONT_HERSHEY_SIMPLEX,
            0.6,
            (245, 245, 245),
            2,
            cv2.LINE_AA,
        )
ffmpeg = args.ffmpeg.resolve()
args = [
    str(ffmpeg),
    "-v",
    "error",
    "-y",
    "-f",
    "rawvideo",
    "-pix_fmt",
    "bgr24",
    "-s",
    f"{width}x{height}",
    "-r",
    str(fps),
    "-i",
    "pipe:0",
    "-an",
    "-c:v",
    "libx264",
    "-crf",
    "16",
    "-pix_fmt",
    "yuv420p",
    str(out / "known-camera.mp4"),
]
truth = []
with subprocess.Popen(args, stdin=subprocess.PIPE) as encoder:
    for i in range(count):
        t = i / fps
        angles = np.deg2rad(
            [
                1.0 * np.sin(2 * np.pi * 3.1 * t),
                1.5 * np.sin(2 * np.pi * 2.3 * t),
                1.2 * np.sin(2 * np.pi * 4.3 * t),
            ]
        )
        rotation = cv2.Rodrigues(angles)[0]
        image = cv2.warpPerspective(
            base,
            camera @ rotation @ np.linalg.inv(camera),
            (width, height),
            flags=cv2.INTER_LINEAR,
            borderMode=cv2.BORDER_REFLECT101,
        )
        encoder.stdin.write(image.tobytes())
        truth.append({"timestamp_ms": t * 1000, "rotation_vector": angles.tolist()})
    encoder.stdin.close()
    if encoder.wait() != 0:
        raise RuntimeError("Video encoding failed")
(out / "known-camera-truth.json").write_text(
    json.dumps(truth, indent=2), encoding="utf-8"
)


def lens(w, h, focal, name):
    return {
        "name": name,
        "camera_brand": "Evaluation",
        "camera_model": name,
        "calibrator_version": "1.6.3",
        "calib_dimension": {"w": w, "h": h},
        "orig_dimension": {"w": w, "h": h},
        "input_horizontal_stretch": 1,
        "input_vertical_stretch": 1,
        "distortion_model": "opencv_standard",
        "global_shutter": True,
        "fisheye_params": {
            "camera_matrix": [[focal, 0, w / 2], [0, focal, h / 2], [0, 0, 1]],
            "distortion_coeffs": [0, 0, 0, 0, 0],
        },
    }


for case, calibration in [
    ("known-camera", lens(width, height, 720, "Known synthetic pinhole")),
    ("comparison1", lens(854, 480, 640, "Approximate pinhole, uncalibrated real clip")),
]:
    (out / f"{case}-lens.json").write_text(
        json.dumps(calibration, indent=2), encoding="utf-8"
    )
    for method, name in [(2, "dis"), (3, "gmflow")]:
        preset = {
            "version": 4,
            "calibration_data": calibration,
            "gyro_source": {"integration_method": 3},
            "stabilization": {
                "method": "Default",
                "smoothing_params": [{"name": "smoothness", "value": 0.5}],
                "fov": 1.0,
                "adaptive_zoom_window": 2.0,
                "max_zoom": 150,
                "frame_readout_time": 0,
                "lens_correction_amount": 1.0,
            },
            "synchronization": {
                "do_autosync": True,
                "of_method": method,
                "pose_method": 3,
                "offset_method": 0,
                "every_nth_frame": 1,
                "processing_resolution": 540,
                "max_sync_points": 1,
                "time_per_syncpoint": 1,
                "search_size": 0.1,
                "auto_sync_points": False,
                "initial_offset": 0,
                "initial_offset_inv": False,
                "calc_initial_fast": False,
            },
            "output": {
                "codec": "H.264/AVC",
                "use_gpu": False,
                "audio": False,
                "bitrate": 12,
                "output_folder": str(out),
                "output_filename": f"{case}-{name}.mp4",
                "interpolation": "Bicubic",
            },
        }
        (out / f"{case}-{name}-preset.gyroflow").write_text(
            json.dumps(preset, indent=2), encoding="utf-8"
        )
        if case == "comparison1":
            preset["synchronization"]["pose_method"] = 0
            preset["output"]["output_filename"] = f"{case}-{name}-essential.mp4"
            (out / f"{case}-{name}-essential-preset.gyroflow").write_text(
                json.dumps(preset, indent=2), encoding="utf-8"
            )
print(
    f"Prepared {count} frames, two flow methods, and explicit camera assumptions in {out}"
)
