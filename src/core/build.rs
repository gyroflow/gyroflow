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

    // The AI optical flow (gmflow, `use-burn`) is for desktop GPUs: the mobile builds leave it out (android.just, ios.just)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if std::env::var_os("CARGO_FEATURE_USE_BURN").is_some() && (target_os == "android" || target_os == "ios") {
        println!("cargo:warning=gyroflow-core: `use-burn` (GMFlow) is meant for desktops, build {target_os} with `--no-default-features --features opencv`");
    }

    gpu_packages();
}

/// The packages gmflow's kernels compile through (burn, cubecl, wgpu and naga) as the build resolved them, for the marker
/// of its check of the cooperative matrices (`coop_marker` in synchronization/optical_flow/gmflow.rs): their versions,
/// git revisions and checksums, out of every lock file that can be the one in effect. That's this package's own when
/// it's built alone, the workspace's when it's a dependency (the app's, at the root), so all of them up the directories
/// from here and from the target directory, as the `built` crate looks for it. Empty without any: the check then runs at
/// every start
fn gpu_packages() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("gpu_packages.txt");
    let mut lists = Vec::new();
    if std::env::var_os("CARGO_FEATURE_USE_BURN").is_some() {
        let starts = [std::env::var("CARGO_MANIFEST_DIR").unwrap(), std::env::var("OUT_DIR").unwrap()];
        let locks: std::collections::BTreeSet<std::path::PathBuf> = starts.iter()
            .flat_map(|dir| std::path::Path::new(dir).ancestors().map(|d| d.join("Cargo.lock")).collect::<Vec<_>>())
            .filter(|lock| lock.is_file())
            .collect();
        for lock in locks {
            println!("cargo:rerun-if-changed={}", lock.display());
            if let Ok(text) = std::fs::read_to_string(&lock) {
                lists.push(text.split("[[package]]").filter_map(|package| {
                    let field = |key: &str| package.lines().find_map(|l| l.strip_prefix(key)?.trim_start().strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_owned());
                    let name = field("name")?;
                    ["burn", "cubecl", "wgpu", "naga"].iter().any(|p| name.starts_with(p))
                        .then(|| format!("{name} {} {} {}", field("version").unwrap_or_default(), field("source").unwrap_or_default(), field("checksum").unwrap_or_default()))
                }).collect::<Vec<_>>().join("\n"));
            }
        }
        // The rest as without the lock files (which turn cargo's own tracking of the package's files off): the profiles
        // looked for again while they're missing
        println!("cargo:rerun-if-changed={}/../../resources/camera_presets/profiles.cbor.gz", std::env::var("CARGO_MANIFEST_DIR").unwrap());
        println!("cargo:rerun-if-changed=build.rs");
    }
    let text = lists.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n\n");
    if std::fs::read_to_string(&out).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&out, text).unwrap();
    }
}
