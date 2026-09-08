"""Export the pinned UniMatch GMFlow checkpoint for Gyroflow's optional backend.

SPDX-License-Identifier: GPL-3.0-or-later
The network implementation and weights belong to the UniMatch authors; this
script imports their implementation instead of vendoring or modifying it.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import urllib.request

COMMIT = "da140fac169d58fba4ffde9c4ef10c906fb5040b"
WEIGHTS_SHA256 = "4e7b215d5a25dc3b41bd2fb55cbbfe1b715a9c279bdcc8ad30bc31afe69421ca"
WEIGHTS_URL = "https://s3.eu-central-1.amazonaws.com/avg-projects/unimatch/pretrained/gmflow-scale2-regrefine6-mixdata-train320x576-4e7b215d.pth"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--unimatch-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    source = args.unimatch_dir.resolve()
    commit = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    if commit != COMMIT:
        raise ValueError(f"Expected UniMatch commit {COMMIT}, got {commit}")
    subprocess.run(
        ["git", "-C", str(source), "diff", "--exit-code", "HEAD", "--", "unimatch"],
        check=True,
    )
    sys.path.insert(0, str(source))
    import numpy as np
    import torch
    import onnx
    import onnxsim
    import onnxruntime as ort
    from unimatch.unimatch import UniMatch

    destination = args.output_dir.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    weights = destination / WEIGHTS_URL.rsplit("/", 1)[-1]
    if not weights.exists():
        # A failed transfer is never accepted as an existing checkpoint.
        temporary = weights.with_suffix(".download")
        urllib.request.urlretrieve(WEIGHTS_URL, temporary)
        if hashlib.sha256(temporary.read_bytes()).hexdigest() != WEIGHTS_SHA256:
            raise ValueError("Downloaded checkpoint failed SHA-256 verification")
        temporary.replace(weights)
    if hashlib.sha256(weights.read_bytes()).hexdigest() != WEIGHTS_SHA256:
        raise ValueError("Checkpoint failed SHA-256 verification")

    torch.set_num_threads(8)
    torch.manual_seed(45)
    network = UniMatch(num_scales=2, upsample_factor=4, reg_refine=True, task="flow")
    state = torch.load(weights, map_location="cpu", weights_only=True)
    network.load_state_dict(state["model"], strict=True)
    network.eval()

    class Flow(torch.nn.Module):
        def __init__(self, net):
            super().__init__()
            self.net = net

        def forward(self, image0, image1):
            return self.net(
                image0,
                image1,
                attn_type="swin",
                attn_splits_list=[2, 8],
                corr_radius_list=[-1, 4],
                prop_radius_list=[-1, 1],
                num_reg_refine=6,
                task="flow",
            )["flow_preds"][-1]

    model = Flow(network).eval()
    a = torch.rand(1, 1, 320, 576).repeat(1, 3, 1, 1) * 255
    b = torch.cat((a[:, :, :, :4], a[:, :, :, :-4]), dim=3)
    with torch.inference_mode():
        expected = model(a, b).numpy()
        torch.onnx.export(
            model,
            (a, b),
            destination / "gmflow-raw.onnx",
            input_names=["image0", "image1"],
            output_names=["flow"],
            opset_version=16,
            dynamo=False,
        )
    graph, checked = onnxsim.simplify(str(destination / "gmflow-raw.onnx"))
    if not checked:
        raise ValueError("ONNX simplification check failed")
    onnx.checker.check_model(graph)
    onnx_path = destination / "gmflow.onnx"
    onnx.save(graph, onnx_path)
    session = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    actual = session.run(None, {"image0": a.numpy(), "image1": b.numpy()})[0]
    error = np.abs(actual - expected)
    if (
        actual.shape != (1, 2, 320, 576)
        or not np.isfinite(actual).all()
        or np.quantile(error, 0.99) > 0.05
    ):
        raise ValueError("Export failed numerical parity check")
    manifest = {
        "source_commit": commit,
        "weights_url": WEIGHTS_URL,
        "weights_sha256": WEIGHTS_SHA256,
        "onnx_sha256": hashlib.sha256(onnx_path.read_bytes()).hexdigest(),
        "torch": torch.__version__,
        "onnx": onnx.__version__,
        "onnxsim": onnxsim.__version__,
        "onnxruntime": ort.__version__,
        "shape": [1, 3, 320, 576],
        "nodes": len(graph.graph.node),
        "ort_vs_torch_p99_abs": float(np.quantile(error, 0.99)),
        "ort_vs_torch_max_abs": float(error.max()),
    }
    (destination / "manifest.json").write_text(
        json.dumps(manifest, indent=2), encoding="utf-8"
    )
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
