// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    println!("cargo:rerun-if-env-changed=GYROFLOW_GMFLOW_ONNX");
    let model = std::env::var("GYROFLOW_GMFLOW_ONNX")
        .expect("GMFlow requires GYROFLOW_GMFLOW_ONNX; see _scripts/gmflow/README.md");
    let path = std::path::Path::new(&model);
    assert!(
        path.is_absolute(),
        "GYROFLOW_GMFLOW_ONNX must be an absolute path"
    );
    assert_eq!(
        path.file_name().and_then(|x| x.to_str()),
        Some("gmflow.onnx")
    );
    println!("cargo:rerun-if-changed={model}");
    burn_onnx::ModelGen::new()
        .input(&model)
        .out_dir("gmflow")
        .run_from_script();
}
