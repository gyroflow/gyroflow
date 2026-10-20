// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

fn main() {
    // Download lens profiles if not already present
    let project_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let db_path = format!("{project_dir}/../../resources/camera_presets/profiles.cbor.gz");
    if !std::path::Path::new(&db_path).exists() {
        std::fs::create_dir_all(&format!("{project_dir}/../../resources/camera_presets")).unwrap();
        if let Ok(mut body) = ureq::get("https://github.com/gyroflow/lens_profiles/releases/latest/download/profiles.cbor.gz").call().map(|x| x.into_body().into_reader()) {
            match std::fs::File::create(&db_path) {
                Ok(mut file) => { std::io::copy(&mut body, &mut file).unwrap(); },
                Err(e) => { panic!("Failed to create {db_path}: {e:?}"); }
            }
        }
    }

    // Generate burn model code from ONNX when AI optical flow is enabled (issue #45). The ONNX is gmflow-scale2-regrefine6
    // exported with memory-light local correlations, so that no tensor needs a GPU buffer of 128 MiB or more (see
    // synchronization/optical_flow/gmflow.rs). Downloaded once into resources/gmflow/, pinned by SHA-256.
    #[cfg(feature = "use-burn")]
    {
        use sha2::Digest;
        use std::io::Read;
        const ONNX_URL: &str = "https://github.com/yasumorishima/gyroflow-models/releases/download/v1.1.0/gmflow-scale2-regrefine6-320x576-opset16-sim.onnx";
        const ONNX_SHA256: &str = "cc3f5d4efae6d52acd90cf51775737a9abd17d820acc87bad14b02f2fa8025b0";
        let onnx_dir = format!("{project_dir}/../../resources/gmflow");
        let onnx_path = format!("{onnx_dir}/gmflow-scale2-regrefine6-320x576-opset16-sim.onnx");
        let sha256 = |bytes: &[u8]| format!("{:x}", sha2::Sha256::digest(bytes));
        if std::fs::read(&onnx_path).map(|b| sha256(&b)).ok().as_deref() != Some(ONNX_SHA256) {
            std::fs::create_dir_all(&onnx_dir).unwrap_or_else(|e| panic!("Failed to create {onnx_dir}: {e:?}"));
            let mut bytes = Vec::new();
            ureq::get(ONNX_URL).call()
                .unwrap_or_else(|e| panic!("AI optical flow (feature use-burn): could not download the gmflow ONNX from {ONNX_URL}: {e}"))
                .into_body().into_reader().read_to_end(&mut bytes)
                .unwrap_or_else(|e| panic!("AI optical flow (feature use-burn): could not download the gmflow ONNX from {ONNX_URL}: {e}"));
            let got = sha256(&bytes);
            if got != ONNX_SHA256 {
                panic!("AI optical flow (feature use-burn): the gmflow ONNX from {ONNX_URL} has SHA-256 {got}, expected {ONNX_SHA256}");
            }
            let tmp = format!("{onnx_path}.tmp.{}", std::process::id());
            std::fs::write(&tmp, &bytes).unwrap_or_else(|e| panic!("Failed to write {tmp}: {e:?}"));
            std::fs::rename(&tmp, &onnx_path).unwrap_or_else(|e| panic!("Failed to rename {tmp}: {e:?}"));
        }
        println!("cargo:rerun-if-changed={onnx_path}");
        // Weights are downloaded at runtime (gmflow.rs), so only `from_bytes` is generated.
        burn_onnx::ModelGen::new()
            .input(&onnx_path)
            .out_dir("model/")
            .load_strategy(burn_onnx::LoadStrategy::Bytes)
            .run_from_script();
    }
}
