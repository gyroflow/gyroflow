"""Render optical comparisons with the same saved camera analysis for both passes."""

import argparse
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gyroflow", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--cases", nargs="+", required=True)
    parser.add_argument("--working-directory", type=Path, default=Path.cwd())
    parser.add_argument("--variants", nargs="+", default=["local", "camera-only"])
    args = parser.parse_args()
    directory = args.directory.resolve()
    executable = args.gyroflow.resolve()
    for case in args.cases:
        for variant in args.variants:
            source = directory / f"{case}.mp4"
            preset = directory / f"{case}-{variant}-preset.gyroflow"
            if variant.endswith("camera-only"):
                analysis = "gmflow-local" if variant.startswith("gmflow-") else "local"
                source = directory / f"{case}-{analysis}.gyroflow"
                if not source.is_file():
                    raise RuntimeError(f"Render {analysis} first: {source}")
                output = json.loads(
                    (directory / f"{case}-camera-only-preset.gyroflow").read_text(
                        encoding="utf-8"
                    )
                )["output"]
                output["output_filename"] = f"{case}-{variant}.mp4"
                settings = {
                    "version": 4,
                    "stabilization": {"optical_stabilization_strength": 0.0},
                    "synchronization": {"do_autosync": False},
                    "output": output,
                }
                preset = directory / f"{case}-{variant}-replay.gyroflow"
                preset.write_text(json.dumps(settings, indent=2), encoding="utf-8")
            command = [
                str(executable),
                str(source),
                "--preset",
                str(preset),
                "--export-project",
                "4",
                "--no-gpu-decoding",
                "-f",
            ]
            started = time.monotonic()
            with (directory / f"{case}-{variant}.log").open(
                "w", encoding="utf-8"
            ) as log:
                completed = subprocess.run(
                    command,
                    cwd=args.working_directory,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )
            result = {
                "case": case,
                "variant": variant,
                "seconds": time.monotonic() - started,
                "exit_code": completed.returncode,
                "command": command,
            }
            print(json.dumps(result), flush=True)
            (directory / f"{case}-{variant}-run.json").write_text(
                json.dumps(result, indent=2), encoding="utf-8"
            )
            if completed.returncode:
                raise RuntimeError(f"Failed render: {case}/{variant}; inspect its log")


if __name__ == "__main__":
    main()
