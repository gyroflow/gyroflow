"""Reproducible synthetic spatial motion and local DAVIS evaluation inputs.

Synthetic footage is generated here, with no external image assets. DAVIS
frames remain local and require a separate download from the official source.
All sequences are evaluated at an explicitly assigned 30 fps.
"""

import argparse
import copy
import json
from pathlib import Path
import subprocess
import cv2
import numpy as np

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--ffmpeg", type=Path, required=True)
parser.add_argument("--output-dir", type=Path, required=True)
parser.add_argument("--davis-dir", type=Path)
parser.add_argument("--base-preset", type=Path, required=True)
args = parser.parse_args()
out = args.output_dir.resolve()
out.mkdir(parents=True, exist_ok=True)
cv2.setNumThreads(4)
width, height, fps, count = 640, 360, 30, 90


def texture(seed):
    random = np.random.default_rng(seed)
    image = cv2.GaussianBlur(
        random.integers(30, 225, (height, width, 3), dtype=np.uint8), (0, 0), 0.8
    )
    for i in range(240):
        xy = tuple(random.integers([5, 5], [width - 5, height - 5]).tolist())
        color = tuple(random.integers(20, 235, 3).tolist())
        cv2.circle(image, xy, int(random.integers(2, 13)), color, -1, cv2.LINE_AA)
    for y in range(45, height, 60):
        for x in range(25, width, 90):
            cv2.putText(
                image,
                f"{x}:{y}",
                (x, y),
                cv2.FONT_HERSHEY_SIMPLEX,
                0.36,
                (250, 250, 250),
                1,
                cv2.LINE_AA,
            )
    return image


base = texture(45)
other = texture(811)
grid_y, grid_x = np.mgrid[:height, :width].astype(np.float32)
camera = np.array([[480, 0, width / 2], [0, 480, height / 2], [0, 0, 1.0]])
template = json.loads(args.base_preset.read_text(encoding="utf-8"))


def presets(case, w, h, focal, note):
    for name, strength, method in [
        ("camera-only", 0.0, 2),
        ("local", 1.0, 2),
        ("gmflow-local", 1.0, 3),
    ]:
        preset = copy.deepcopy(template)
        preset["calibration_data"].update(
            name=note,
            camera_model=note,
            calib_dimension={"w": w, "h": h},
            orig_dimension={"w": w, "h": h},
        )
        preset["calibration_data"]["fisheye_params"]["camera_matrix"] = [
            [focal, 0, w / 2],
            [0, focal, h / 2],
            [0, 0, 1],
        ]
        preset["stabilization"]["optical_stabilization_strength"] = strength
        preset["synchronization"].update(of_method=method, processing_resolution=h)
        preset["output"].update(
            output_folder=str(out), output_filename=f"{case}-{name}.mp4"
        )
        (out / f"{case}-{name}-preset.gyroflow").write_text(
            json.dumps(preset, indent=2), encoding="utf-8"
        )


def encode(path, frames, size):
    command = [
        str(args.ffmpeg.resolve()),
        "-v",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "bgr24",
        "-s",
        f"{size[0]}x{size[1]}",
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
        str(path),
    ]
    with subprocess.Popen(command, stdin=subprocess.PIPE) as process:
        for frame in frames:
            process.stdin.write(frame.tobytes())
        process.stdin.close()
        if process.wait() != 0:
            raise RuntimeError(f"Encoding failed: {path}")


manifest = {"fps_assumption": fps, "texture_seeds": {"background": 45, "alternate": 811}, "cases": {}}
for case in [
    "local-jitter",
    "moving-subject",
    "high-shake",
    "sparse-features",
    "scene-cut",
]:
    truth = []

    def frames():
        for i in range(count):
            t = i / fps
            scene = other.copy() if case == "scene-cut" and i >= 45 else base.copy()
            if case == "sparse-features":
                scene[:] = 128
                scene[110:250, 180:460] = base[110:250, 180:460]
            if case == "moving-subject":
                x = int(100 + 320 * i / (count - 1))
                scene[100:245, x : x + 110] = other[100:245, 200:310]
                cv2.rectangle(scene, (x, 100), (x + 110, 245), (35, 180, 240), 3)
            amplitude = 3.0 if case == "high-shake" else 1.0
            angles = np.deg2rad(
                amplitude
                * np.array(
                    [
                        np.sin(2 * np.pi * 3.1 * t),
                        1.5 * np.sin(2 * np.pi * 2.3 * t),
                        1.2 * np.sin(2 * np.pi * 4.3 * t),
                    ]
                )
            )
            rotation = cv2.Rodrigues(angles)[0]
            homography = camera @ rotation @ np.linalg.inv(camera)
            image = cv2.warpPerspective(
                scene, homography, (width, height), borderMode=cv2.BORDER_REFLECT101
            )
            # Smoothly varying distortion cannot be represented by one camera
            # rotation or homography. Inverse remap coefficients are recorded.
            dx = 4.0 * np.sin(2 * np.pi * 5.1 * t)
            dy = 2.5 * np.sin(2 * np.pi * 4.7 * t)
            map_x = (grid_x + dx * np.sin(grid_y / height * np.pi * 2)).astype(
                np.float32
            )
            map_y = (grid_y + dy * np.sin(grid_x / width * np.pi * 2)).astype(
                np.float32
            )
            image = cv2.remap(
                image, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REFLECT101
            )
            truth.append(
                {
                    "frame": i,
                    "rotation_vector": angles.tolist(),
                    "inverse_local_dx": dx,
                    "inverse_local_dy": dy,
                    "scene": int(case == "scene-cut" and i >= 45),
                }
            )
            yield image

    encode(out / f"{case}.mp4", frames(), (width, height))
    (out / f"{case}-truth.json").write_text(
        json.dumps(truth, indent=2), encoding="utf-8"
    )
    presets(case, width, height, 480, "Known synthetic pinhole, spatial jitter fixture")
    manifest["cases"][case] = {
        "source": "procedural",
        "frames": count,
        "camera": "known 480 px focal length",
        "local_model": "sinusoidal inverse remap, coefficients in truth JSON",
    }
    print(case, flush=True)

contacts = []
if args.davis_dir:
    for case in ["bmx-trees", "drift-chicane", "horsejump-high"]:
        paths = sorted((args.davis_dir / case).glob("*.jpg"))
        if not paths:
            continue
        first = cv2.imread(str(paths[0]))
        h, w = first.shape[:2]
        encode(out / f"{case}.mp4", (cv2.imread(str(path)) for path in paths), (w, h))
        presets(case, w, h, 0.9 * w, "Uncalibrated DAVIS sequence, approximate pinhole")
        manifest["cases"][case] = {
            "source": "DAVIS 2017 trainval 480p",
            "frames": len(paths),
            "camera": "unknown, assumed 0.9 * width focal length",
            "ground_truth": "none for motion",
        }
        for i in [0, len(paths) // 2]:
            frame = cv2.resize(cv2.imread(str(paths[i])), (426, 240))
            cv2.putText(
                frame,
                f"{case} frame {i}",
                (10, 25),
                cv2.FONT_HERSHEY_SIMPLEX,
                0.5,
                (255, 255, 255),
                1,
                cv2.LINE_AA,
            )
            contacts.append(frame)
        print(case, flush=True)
if contacts:
    cv2.imwrite(
        str(out / "davis-contact.jpg"),
        np.vstack([np.hstack(contacts[i : i + 2]) for i in range(0, len(contacts), 2)]),
    )
(out / "cases.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
