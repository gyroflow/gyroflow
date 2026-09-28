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

    // Generate burn model code from ONNX when AI optical flow is enabled (issue #45).
    // The .onnx is expected at resources/gmflow/ — shipped pre-built or downloaded out-of-band.
    // TODO(#45): download from a hosted release asset once the URL is finalised.
    #[cfg(feature = "use-burn")]
    {
        let onnx_path = format!(
            "{project_dir}/../../resources/gmflow/gmflow-scale2-regrefine6-320x576-opset16-sim.onnx"
        );
        if !std::path::Path::new(&onnx_path).exists() {
            panic!(
                "AI optical flow is enabled (feature use-burn) but gmflow ONNX not found at {onnx_path}. \
                 Export it from AdrianEddy/unimatch + onnx-simplifier and place it there."
            );
        }
        println!("cargo:rerun-if-changed={onnx_path}");
        burn_onnx::ModelGen::new()
            .input(&onnx_path)
            .out_dir("model/")
            .run_from_script();
    }
}
