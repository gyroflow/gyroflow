"""Measure rendered-video jitter using image registration independent of GMFlow.

The default metric is RMS second differences of nine fixed reference points.
With --pairwise, it measures differences of the adjacent-frame displacement
field at nine image positions. Both use output pixels, include zoom jitter,
and approximate the scene with a homography. They do not measure local warping.
"""

import argparse
import json
from pathlib import Path

import cv2
import numpy as np


def measure(reference, path, pairwise=False):
    capture = cv2.VideoCapture(str(path))
    if not capture.isOpened():
        raise RuntimeError(f"Cannot open {path}")
    detector = cv2.SIFT_create(nfeatures=3000)
    gray = cv2.cvtColor(reference, cv2.COLOR_BGR2GRAY)
    height, width = gray.shape
    mask = np.zeros_like(gray)
    mask[height // 6 : -height // 6, width // 6 : -width // 6] = 255
    keypoints, descriptors = detector.detectAndCompute(gray, mask)
    points = np.array(
        [[x, y] for y in [0.3, 0.5, 0.7] for x in [0.3, 0.5, 0.7]], np.float32
    )
    points *= [width, height]
    matcher = cv2.BFMatcher()
    trajectories, inliers, scales, rejected = [], [], [], []
    index = 0
    black = 0
    while True:
        ok, frame = capture.read()
        if not ok:
            break
        gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
        black += int(gray.max() <= 2)
        current, values = detector.detectAndCompute(gray, None)
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
            pair[0]
            for pair in candidates
            if len(pair) == 2 and pair[0].distance < 0.7 * pair[1].distance
        ]
        transform, accepted = None, None
        if len(matches) >= 20:
            source = np.float32([keypoints[m.queryIdx].pt for m in matches])
            target = np.float32([current[m.trainIdx].pt for m in matches])
            transform, accepted = cv2.findHomography(source, target, cv2.RANSAC, 2.0)
        if transform is None or accepted.sum() < 20 or not np.isfinite(transform).all():
            trajectories.append(np.full((9, 2), np.nan))
            rejected.append(index)
        else:
            projected = cv2.perspectiveTransform(points[None], transform)[0]
            trajectories.append(projected - points if pairwise else projected)
            inliers.append(int(accepted.sum()))
            scales.append(float(np.sqrt(abs(np.linalg.det(transform[:2, :2])))))
        index += 1
        if pairwise:
            keypoints, descriptors = current, values
    capture.release()
    positions = np.asarray(trajectories)
    acceleration = np.diff(positions, n=1 if pairwise else 2, axis=0)
    usable = np.isfinite(acceleration).all(axis=(1, 2))
    rms = (
        float(np.sqrt(np.mean(np.sum(acceleration[usable] ** 2, axis=2))))
        if usable.any()
        else None
    )
    return {
        "file": path.name,
        "frames": index,
        "black_frames": black,
        "mode": (
            "adjacent-frame homography velocity differences"
            if pairwise
            else "fixed-reference homography second differences"
        ),
        "registration_failed_frames": rejected,
        "jitter_rms_px": rms,
        "median_registration_inliers": float(np.median(inliers)) if inliers else None,
        "median_scale": float(np.median(scales)) if scales else None,
        "reference_points": points.tolist(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("videos", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--pairwise", action="store_true")
    args = parser.parse_args()
    capture = cv2.VideoCapture(str(args.reference))
    ok, reference = capture.read()
    capture.release()
    if not ok:
        raise RuntimeError(f"Cannot read reference {args.reference}")
    results = [measure(reference, video, args.pairwise) for video in args.videos]
    args.output.write_text(
        json.dumps(results, indent=2, allow_nan=False), encoding="utf-8"
    )
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
