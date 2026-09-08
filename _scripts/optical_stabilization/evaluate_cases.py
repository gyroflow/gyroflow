"""Independent SIFT registration of spatially distributed background patches.

Reports temporal acceleration in output pixels, its spatially varying component,
registration coverage, and local scale. These are diagnostic proxies, not a
perceptual score or ground-truth motion error. Synthetic scene cuts are measured
as separate shots; DAVIS clips use adjacent-frame registration.
"""

import argparse
import json
from pathlib import Path
import cv2
import numpy as np

cv2.setNumThreads(4)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("directory", type=Path)
parser.add_argument("--cases", nargs="+", required=True)
parser.add_argument("--variants", nargs="+", default=["input", "camera-only", "local"])
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
detector = cv2.SIFT_create(nfeatures=5000, contrastThreshold=0.02)
matcher = cv2.BFMatcher()


def features(frame, mask=None):
    gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
    return detector.detectAndCompute(gray, mask)


def measure(case, variant):
    path = args.directory / (
        f"{case}.mp4" if variant == "input" else f"{case}-{variant}.mp4"
    )
    video = cv2.VideoCapture(str(path))
    source = cv2.VideoCapture(str(args.directory / f"{case}.mp4"))
    valid, reference = source.read()
    if not valid or not video.isOpened():
        raise RuntimeError(f"Missing video: {path}")
    h, w = reference.shape[:2]
    points = np.array(
        [(x * w, y * h) for y in [0.2, 0.5, 0.8] for x in [0.18, 0.39, 0.61, 0.82]],
        dtype=np.float32,
    )
    mask = np.full((h, w), 255, np.uint8)
    if case == "moving-subject":
        mask[95:250, 95:215] = 0
    keypoints, descriptors = features(reference, mask)
    pairwise = case in ["bmx-trees", "drift-chicane", "horsejump-high"]
    positions, scales, counts, invalid = [], [], [], []
    index, black, border_black = 0, 0, []
    while True:
        ok, frame = video.read()
        if not ok:
            break
        gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
        black += int(gray.max() <= 2)
        border = np.concatenate(
            [
                gray[:3].ravel(),
                gray[-3:].ravel(),
                gray[:, :3].ravel(),
                gray[:, -3:].ravel(),
            ]
        )
        border_black.append(float((border <= 2).mean()))
        if case == "scene-cut" and index == 45:
            source.set(cv2.CAP_PROP_POS_FRAMES, 45)
            ok, reference = source.read()
            if not ok:
                raise RuntimeError("Missing second shot")
            keypoints, descriptors = features(reference)
        current, values = features(frame)
        if pairwise and index == 0:
            keypoints, descriptors = current, values
            index += 1
            continue
        candidates = (
            matcher.knnMatch(descriptors, values, k=2)
            if descriptors is not None and values is not None
            else []
        )
        matches = [
            m[0]
            for m in candidates
            if len(m) == 2 and m[0].distance < 0.7 * m[1].distance
        ]
        src = np.float32([keypoints[m.queryIdx].pt for m in matches]).reshape(-1, 2)
        dst = np.float32([current[m.trainIdx].pt for m in matches]).reshape(-1, 2)
        field = np.full((len(points), 2), np.nan)
        frame_scales, frame_counts = [], []
        if len(matches) >= 15:
            # A loose global gate excludes wrong descriptors and independently
            # moving subjects without forcing a globally rigid field.
            transform, inliers = cv2.findHomography(src, dst, cv2.RANSAC, 8.0)
            if transform is not None:
                gate = inliers.ravel() > 0
                src, dst = src[gate], dst[gate]
                for i, point in enumerate(points):
                    near = (np.abs(src[:, 0] - point[0]) < w * 0.21) & (
                        np.abs(src[:, 1] - point[1]) < h * 0.25
                    )
                    if near.sum() < 8:
                        continue
                    affine, accepted = cv2.estimateAffine2D(
                        src[near],
                        dst[near],
                        method=cv2.RANSAC,
                        ransacReprojThreshold=2.0,
                        maxIters=2000,
                    )
                    if affine is None or accepted.sum() < 6:
                        continue
                    field[i] = affine @ np.r_[point, 1.0]
                    frame_scales.append(
                        float(np.sqrt(abs(np.linalg.det(affine[:, :2]))))
                    )
                    frame_counts.append(int(accepted.sum()))
        positions.append(field - points if pairwise else field)
        scales.extend(frame_scales)
        counts.extend(frame_counts)
        invalid.append(int(np.isnan(field[:, 0]).sum()))
        index += 1
        if pairwise:
            keypoints, descriptors = current, values
    video.release()
    source.release()
    positions = np.asarray(positions)
    acceleration = np.diff(positions, n=1 if pairwise else 2, axis=0)
    if case == "scene-cut":
        acceleration[43:45] = np.nan
    valid = np.isfinite(acceleration).all(axis=2)
    rms = (
        float(np.sqrt(np.mean(np.sum(acceleration[valid] ** 2, axis=1))))
        if valid.any()
        else None
    )
    spatial = []
    for frame in acceleration:
        ok = np.isfinite(frame).all(axis=1)
        if ok.sum() >= 8:
            design = np.c_[points[ok] / [w, h], np.ones(ok.sum())]
            linear = np.linalg.lstsq(design, frame[ok], rcond=None)[0]
            spatial.extend(np.sum((frame[ok] - design @ linear) ** 2, axis=1).tolist())
    return {
        "case": case,
        "variant": variant,
        "file": path.name,
        "mode": (
            "adjacent-frame local affine velocity difference"
            if pairwise
            else "fixed-reference local affine acceleration"
        ),
        "frames": index,
        "black_frames": black,
        "median_black_border_fraction": float(np.median(border_black)),
        "jitter_rms_px": rms,
        "spatial_jitter_rms_px": float(np.sqrt(np.mean(spatial))) if spatial else None,
        "acceleration_observation_fraction": float(valid.mean()),
        "median_registration_inliers_per_patch": (
            float(np.median(counts)) if counts else None
        ),
        "median_linear_scale": float(np.median(scales)) if scales else None,
        "missing_patches_per_frame": invalid,
    }


results = []
for case in args.cases:
    for variant in args.variants:
        result = measure(case, variant)
        results.append(result)
        print(
            json.dumps(
                {k: v for k, v in result.items() if k != "missing_patches_per_frame"}
            ),
            flush=True,
        )
        args.output.write_text(
            json.dumps(results, indent=2, allow_nan=False), encoding="utf-8"
        )
