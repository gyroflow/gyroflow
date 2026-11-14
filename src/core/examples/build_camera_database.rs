// SPDX-License-Identifier: GPL-3.0-or-later
// Tool to build resources/camera_database.json from profiles.cbor.gz and camera domain knowledge

use std::collections::HashMap;
use std::io::Read;
use gyroflow_core::camera_database::{CameraDatabase, CameraDatabaseRaw, CameraInfo, LensInfo, MountInfo};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Building camera_database.json...");

    let profiles_path = "../../resources/camera_presets/profiles.cbor.gz";
    let data = std::fs::read(profiles_path)?;
    let mut gz = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
    let mut decompressed = Vec::new();
    gz.read_to_end(&mut decompressed)?;

    let array: Vec<(String, serde_json::Value)> = ciborium::from_reader(std::io::Cursor::new(decompressed))?;
    println!("Loaded {} entries from profiles.cbor.gz", array.len());

    let mut raw = CameraDatabaseRaw {
        version: 1,
        ..Default::default()
    };

    // 1. Define standard mounts
    raw.mounts = vec![
        MountInfo {
            id: "sony_e".into(),
            name: "Sony E".into(),
            flange_distance: Some(18.0),
            aliases: vec!["Sony FE".into(), "E-mount".into(), "FE-mount".into(), "Sony E-mount".into()],
        },
        MountInfo {
            id: "canon_rf".into(),
            name: "Canon RF".into(),
            flange_distance: Some(20.0),
            aliases: vec!["RF-mount".into(), "Canon RF-mount".into()],
        },
        MountInfo {
            id: "canon_ef".into(),
            name: "Canon EF".into(),
            flange_distance: Some(44.0),
            aliases: vec!["EF-mount".into(), "Canon EF-S".into(), "EF".into()],
        },
        MountInfo {
            id: "canon_ef_m".into(),
            name: "Canon EF-M".into(),
            flange_distance: Some(18.0),
            aliases: vec!["EF-M".into(), "EOS-M".into()],
        },
        MountInfo {
            id: "nikon_z".into(),
            name: "Nikon Z".into(),
            flange_distance: Some(16.0),
            aliases: vec!["Z-mount".into(), "Nikon Z-mount".into()],
        },
        MountInfo {
            id: "nikon_f".into(),
            name: "Nikon F".into(),
            flange_distance: Some(46.5),
            aliases: vec!["F-mount".into(), "Nikon F-mount".into()],
        },
        MountInfo {
            id: "micro_four_thirds".into(),
            name: "Micro Four Thirds".into(),
            flange_distance: Some(19.25),
            aliases: vec!["MFT".into(), "M4/3".into(), "Micro 4/3".into(), "m43".into(), "Four Thirds".into()],
        },
        MountInfo {
            id: "fujifilm_x".into(),
            name: "Fujifilm X".into(),
            flange_distance: Some(17.7),
            aliases: vec!["Fuji X".into(), "X-mount".into(), "FX".into()],
        },
        MountInfo {
            id: "fujifilm_g".into(),
            name: "Fujifilm G".into(),
            flange_distance: Some(26.7),
            aliases: vec!["G-mount".into(), "GFX".into()],
        },
        MountInfo {
            id: "leica_l".into(),
            name: "Leica L".into(),
            flange_distance: Some(20.0),
            aliases: vec!["L-mount".into(), "L Mount".into()],
        },
        MountInfo {
            id: "leica_m".into(),
            name: "Leica M".into(),
            flange_distance: Some(27.8),
            aliases: vec!["M-mount".into(), "M".into()],
        },
        MountInfo {
            id: "arri_pl".into(),
            name: "ARRI PL".into(),
            flange_distance: Some(52.0),
            aliases: vec!["PL".into(), "PL-mount".into()],
        },
        MountInfo {
            id: "dji_dl".into(),
            name: "DJI DL".into(),
            flange_distance: Some(16.8),
            aliases: vec!["DL-mount".into(), "DL".into()],
        },
        MountInfo {
            id: "m12".into(),
            name: "M12".into(),
            flange_distance: None,
            aliases: vec!["M12x0.5".into(), "S-mount".into()],
        },
        MountInfo {
            id: "fixed_lens".into(),
            name: "Fixed Lens".into(),
            flange_distance: None,
            aliases: vec!["Fixed".into(), "Integrated".into(), "Built-in".into()],
        },
    ];

    // 2. Define canonical brand aliases
    let brand_aliases_tuples = [
        ("sony", "Sony"), ("sony corporation", "Sony"),
        ("canon", "Canon"),
        ("nikon", "Nikon"),
        ("panasonic", "Panasonic"), ("lumix", "Panasonic"),
        ("fujifilm", "Fujifilm"), ("fuji", "Fujifilm"),
        ("blackmagic", "Blackmagic"), ("blackmagic design", "Blackmagic"), ("bmpcc", "Blackmagic"),
        ("red", "RED"), ("red digital cinema", "RED"),
        ("gopro", "GoPro"),
        ("dji", "DJI"),
        ("insta360", "Insta360"),
        ("runcam", "RunCam"),
        ("caddx", "Caddx"),
        ("walksnail", "Walksnail"),
        ("hawkeye", "Hawkeye"),
        ("sigma", "Sigma"),
        ("leica", "Leica"),
        ("olympus", "Olympus"), ("om system", "Olympus"),
        ("z cam", "Z CAM"), ("zcam", "Z CAM"),
        ("apple", "Apple"),
        ("samsung", "Samsung"),
        ("xiaomi", "Xiaomi"),
        ("kinefinity", "Kinefinity"),
        ("tamron", "Tamron"),
        ("samyang", "Samyang"), ("rokinon", "Samyang"),
        ("zeiss", "Zeiss"), ("carl zeiss", "Zeiss"),
        ("laowa", "Laowa"), ("venus optics", "Laowa"),
        ("sirui", "Sirui"),
        ("ttartisan", "TTArtisan"),
        ("7artisans", "7Artisans"),
        ("meike", "Meike"),
        ("voigtlander", "Voigtländer"), ("voigtländer", "Voigtländer"),
        ("dzo", "DZOFILM"), ("dzofilm", "DZOFILM"),
    ];
    for (k, v) in brand_aliases_tuples {
        raw.brand_aliases.insert(k.into(), v.into());
    }

    // Helper closure to normalize brand
    let norm_brand = |b: &str| -> String {
        let trimmed = b.trim();
        if trimmed.is_empty() { return String::new(); }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(canonical) = raw.brand_aliases.get(&lower) {
            return canonical.clone();
        }
        let mut chars = trimmed.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };

    // Helper closure to infer mount and crop factor from camera brand and model
    let infer_camera_info = |brand: &str, model: &str| -> (Option<String>, Option<String>, Option<f64>, Option<f64>, Option<f64>) {
        let m_lower = model.to_ascii_lowercase();
        match brand {
            "Sony" => {
                if m_lower.contains("rx100") || m_lower.contains("rx0") || m_lower.contains("zv-1") {
                    (Some("Fixed Lens".into()), Some("1-inch".into()), Some(2.7), Some(13.2), Some(8.8))
                } else if m_lower.starts_with("a6") || m_lower.starts_with("a5") || m_lower.contains("zv-e10") || m_lower.contains("fx30") || m_lower.contains("nex") {
                    (Some("Sony E".into()), Some("APS-C".into()), Some(1.5), Some(23.6), Some(15.6))
                } else {
                    // a7, a9, a1, fx3, fx6, fx9, zv-e1
                    (Some("Sony E".into()), Some("Full Frame".into()), Some(1.0), Some(35.6), Some(23.8))
                }
            },
            "Canon" => {
                if m_lower.starts_with('r') || m_lower.contains("c70") || m_lower.contains("c200") {
                    if m_lower.contains("r7") || m_lower.contains("r10") || m_lower.contains("r50") || m_lower.contains("r100") {
                        (Some("Canon RF".into()), Some("APS-C".into()), Some(1.6), Some(22.3), Some(14.9))
                    } else {
                        (Some("Canon RF".into()), Some("Full Frame".into()), Some(1.0), Some(36.0), Some(24.0))
                    }
                } else if m_lower.contains("eos m") || m_lower.contains("m50") || m_lower.contains("m6") || m_lower.contains("m5") {
                    (Some("Canon EF-M".into()), Some("APS-C".into()), Some(1.6), Some(22.3), Some(14.9))
                } else if m_lower.contains("7d") || m_lower.contains("80d") || m_lower.contains("90d") || m_lower.contains("rebel") {
                    (Some("Canon EF".into()), Some("APS-C".into()), Some(1.6), Some(22.3), Some(14.9))
                } else if m_lower.contains("g7x") || m_lower.contains("g9x") {
                    (Some("Fixed Lens".into()), Some("1-inch".into()), Some(2.7), Some(13.2), Some(8.8))
                } else {
                    (Some("Canon EF".into()), Some("Full Frame".into()), Some(1.0), Some(36.0), Some(24.0))
                }
            },
            "Nikon" => {
                if m_lower.starts_with('z') {
                    if m_lower.contains("z50") || m_lower.contains("zfc") || m_lower.contains("z30") {
                        (Some("Nikon Z".into()), Some("APS-C".into()), Some(1.5), Some(23.5), Some(15.7))
                    } else {
                        (Some("Nikon Z".into()), Some("Full Frame".into()), Some(1.0), Some(35.9), Some(23.9))
                    }
                } else if m_lower.contains("d500") || m_lower.contains("d7500") || m_lower.contains("d7200") || m_lower.contains("d5600") {
                    (Some("Nikon F".into()), Some("APS-C".into()), Some(1.5), Some(23.5), Some(15.7))
                } else {
                    (Some("Nikon F".into()), Some("Full Frame".into()), Some(1.0), Some(35.9), Some(23.9))
                }
            },
            "Panasonic" => {
                if m_lower.starts_with('s') || m_lower.contains("bs1h") {
                    (Some("Leica L".into()), Some("Full Frame".into()), Some(1.0), Some(35.6), Some(23.8))
                } else if m_lower.contains("vx980") || m_lower.contains("hc-") {
                    (Some("Fixed Lens".into()), Some("1/2.3-inch".into()), Some(5.6), Some(6.17), Some(4.55))
                } else {
                    (Some("Micro Four Thirds".into()), Some("Micro Four Thirds".into()), Some(2.0), Some(17.3), Some(13.0))
                }
            },
            "Fujifilm" => {
                if m_lower.starts_with("gfx") {
                    (Some("Fujifilm G".into()), Some("Medium Format".into()), Some(0.79), Some(43.8), Some(32.9))
                } else if m_lower.contains("x100") {
                    (Some("Fixed Lens".into()), Some("APS-C".into()), Some(1.5), Some(23.5), Some(15.6))
                } else {
                    (Some("Fujifilm X".into()), Some("APS-C".into()), Some(1.5), Some(23.5), Some(15.6))
                }
            },
            "Blackmagic" => {
                if m_lower.contains("pocket 6k") || m_lower.contains("6k pro") || m_lower.contains("6k g2") || m_lower.contains("ursa") {
                    (Some("Canon EF".into()), Some("Super 35".into()), Some(1.56), Some(23.10), Some(12.99))
                } else if m_lower.contains("cinema camera 6k") && !m_lower.contains("pocket") {
                    (Some("Leica L".into()), Some("Full Frame".into()), Some(1.0), Some(36.0), Some(24.0))
                } else {
                    (Some("Micro Four Thirds".into()), Some("Micro Four Thirds".into()), Some(1.9), Some(18.96), Some(10.0))
                }
            },
            "RED" => {
                if m_lower.contains("komodo") {
                    (Some("Canon RF".into()), Some("Super 35".into()), Some(1.33), Some(27.03), Some(14.26))
                } else if m_lower.contains("raptor") {
                    (Some("Canon RF".into()), Some("Full Frame".into()), Some(1.0), Some(40.96), Some(21.60))
                } else {
                    (Some("Canon EF".into()), Some("Super 35".into()), Some(1.35), Some(29.90), Some(15.77))
                }
            },
            "Z CAM" => {
                if m_lower.contains("e2-s6") {
                    (Some("Canon EF".into()), Some("Super 35".into()), Some(1.5), Some(23.4), Some(15.6))
                } else if m_lower.contains("e2-f") {
                    (Some("Canon EF".into()), Some("Full Frame".into()), Some(1.0), Some(35.97), Some(23.98))
                } else {
                    (Some("Micro Four Thirds".into()), Some("Micro Four Thirds".into()), Some(2.0), Some(17.3), Some(13.0))
                }
            },
            "GoPro" => {
                (Some("Fixed Lens".into()), Some("Action 1/1.9-inch".into()), Some(5.0), Some(6.4), Some(5.6))
            },
            "DJI" => {
                if m_lower.contains("ronin 4d") || m_lower.contains("inspire") {
                    (Some("DJI DL".into()), Some("Full Frame".into()), Some(1.0), Some(36.0), Some(24.0))
                } else {
                    (Some("Fixed Lens".into()), Some("Action".into()), Some(4.0), Some(9.6), Some(7.2))
                }
            },
            "Insta360" => {
                if m_lower.contains("1-inch") {
                    (Some("Fixed Lens".into()), Some("1-inch".into()), Some(2.7), Some(13.2), Some(8.8))
                } else {
                    (Some("Fixed Lens".into()), Some("Action".into()), Some(5.0), Some(6.4), Some(4.8))
                }
            },
            "RunCam" | "Caddx" | "Walksnail" | "Hawkeye" => {
                (Some("M12".into()), Some("FPV Micro".into()), Some(5.6), Some(6.0), Some(4.5))
            },
            "Sigma" => {
                (Some("Leica L".into()), Some("Full Frame".into()), Some(1.0), Some(35.9), Some(23.9))
            },
            "Leica" => {
                if m_lower.starts_with('m') {
                    (Some("Leica M".into()), Some("Full Frame".into()), Some(1.0), Some(35.8), Some(23.9))
                } else {
                    (Some("Leica L".into()), Some("Full Frame".into()), Some(1.0), Some(35.8), Some(23.9))
                }
            },
            "Olympus" => {
                (Some("Micro Four Thirds".into()), Some("Micro Four Thirds".into()), Some(2.0), Some(17.3), Some(13.0))
            },
            _ => (None, None, None, None, None),
        }
    };

    // Helper closure to infer lens mount and focal length from lens model
    let infer_lens_info = |_brand: &str, lens_model: &str, cam_mount: Option<&str>| -> (Option<String>, Option<f64>, Option<f64>) {
        let l_lower = lens_model.to_ascii_lowercase();

        // Extract focal length if present (e.g. "24-70mm" -> min: 24, max: 70; "50mm" -> min: 50, max: 50)
        let mut min_f = None;
        let mut max_f = None;
        if let Some(pos) = l_lower.find("mm") {
            let prefix = &l_lower[..pos];
            if let Some(num_str) = prefix.split_whitespace().last() {
                if let Some(dash) = num_str.find('-') {
                    let f1 = num_str[..dash].parse::<f64>().ok();
                    let f2 = num_str[dash+1..].parse::<f64>().ok();
                    min_f = f1;
                    max_f = f2;
                } else if let Ok(f) = num_str.parse::<f64>() {
                    min_f = Some(f);
                    max_f = Some(f);
                }
            }
        }

        let mount = if l_lower.contains("e-mount") || l_lower.contains("fe ") || l_lower.starts_with("fe ") || l_lower.contains("sel") {
            Some("Sony E".into())
        } else if l_lower.contains("rf") || l_lower.contains("rf-mount") {
            Some("Canon RF".into())
        } else if l_lower.contains("ef-s") || l_lower.contains("ef ") || l_lower.starts_with("ef ") {
            Some("Canon EF".into())
        } else if l_lower.contains("z-mount") || l_lower.contains("nikkor z") {
            Some("Nikon Z".into())
        } else if l_lower.contains("f-mount") || l_lower.contains("nikkor f") || l_lower.contains("af-s") {
            Some("Nikon F".into())
        } else if l_lower.contains("m4/3") || l_lower.contains("mft") || l_lower.contains("micro four thirds") || l_lower.contains("lumix g") || l_lower.contains("m.zuiko") {
            Some("Micro Four Thirds".into())
        } else if l_lower.contains("xf ") || l_lower.contains("xc ") || l_lower.contains("fuji x") {
            Some("Fujifilm X".into())
        } else if l_lower.contains("gf ") || l_lower.contains("gfx") {
            Some("Fujifilm G".into())
        } else if l_lower.contains("l-mount") || l_lower.contains("lumix s") {
            Some("Leica L".into())
        } else if l_lower.contains("m-mount") {
            Some("Leica M".into())
        } else if l_lower.contains("pl-mount") || l_lower.contains("arri pl") {
            Some("ARRI PL".into())
        } else if l_lower.contains("m12") {
            Some("M12".into())
        } else {
            cam_mount.map(|s| s.to_string())
        };

        (mount, min_f, max_f)
    };

    // Extract all unique cameras and lenses from profiles.cbor.gz
    let mut camera_set: HashMap<(String, String), CameraInfo> = HashMap::new();
    let mut lens_set: HashMap<(String, String), LensInfo> = HashMap::new();

    for (fname, val) in &array {
        if fname.starts_with("__") { continue; }
        let raw_brand = val.get("camera_brand").and_then(|v| v.as_str()).unwrap_or("").trim();
        let raw_model = val.get("camera_model").and_then(|v| v.as_str()).unwrap_or("").trim();
        let raw_lens = val.get("lens_model").and_then(|v| v.as_str()).unwrap_or("").trim();

        if raw_brand.is_empty() || raw_model.is_empty() { continue; }

        let brand = norm_brand(raw_brand);
        let model = raw_model.to_string();

        let (mount, sensor_type, crop_factor, sensor_w, sensor_h) = infer_camera_info(&brand, &model);

        let key = (brand.clone(), model.clone());
        let cam_entry = camera_set.entry(key).or_insert_with(|| CameraInfo {
            brand: brand.clone(),
            model: model.clone(),
            mount: mount.clone(),
            sensor_type,
            crop_factor,
            sensor_width: sensor_w,
            sensor_height: sensor_h,
            aliases: Vec::new(),
        });

        // Add alias if original brand/model differed
        if raw_model != model && !cam_entry.aliases.contains(&raw_model.to_string()) {
            cam_entry.aliases.push(raw_model.to_string());
        }

        // Process lens
        if !raw_lens.is_empty() {
            let (l_mount, min_f, max_f) = infer_lens_info(&brand, raw_lens, mount.as_deref());
            let l_key = (brand.clone(), raw_lens.to_string());
            lens_set.entry(l_key).or_insert_with(|| LensInfo {
                brand: brand.clone(),
                model: raw_lens.to_string(),
                mount: l_mount,
                sensor_coverage: None,
                crop_factor,
                min_focal: min_f,
                max_focal: max_f,
                aspect_ratio: Some("3:2".into()),
                aliases: Vec::new(),
            });
        }
    }

    // Populate common camera aliases (e.g., ILCE-7M3 -> a7 III, BMPCC4K -> Pocket Cinema Camera 4K)
    let known_model_aliases = [
        ("Sony", "ILCE-7M3", "a7 III"), ("Sony", "a73", "a7 III"), ("Sony", "a7m3", "a7 III"),
        ("Sony", "ILCE-7M4", "a7 IV"), ("Sony", "a74", "a7 IV"), ("Sony", "a7m4", "a7 IV"),
        ("Sony", "ILCE-7SM3", "a7S III"), ("Sony", "a7s3", "a7S III"), ("Sony", "a7siii", "a7S III"),
        ("Sony", "ILCE-7RM4", "a7R IV"), ("Sony", "a7r4", "a7R IV"), ("Sony", "a7riv", "a7R IV"),
        ("Sony", "ILCE-7RM5", "a7R V"), ("Sony", "a7r5", "a7R V"), ("Sony", "a7rv", "a7R V"),
        ("Sony", "ILCE-1", "a1"), ("Sony", "ILCE-9", "a9"), ("Sony", "ILCE-9M2", "a9 II"),
        ("Sony", "ILME-FX3", "FX3"), ("Sony", "ILME-FX30", "FX30"), ("Sony", "ILME-FX6", "FX6"),
        ("Blackmagic", "bmpcc4k", "Pocket Cinema Camera 4K"), ("Blackmagic", "pocket 4k", "Pocket Cinema Camera 4K"),
        ("Blackmagic", "bmpcc6k", "Pocket Cinema Camera 6K"), ("Blackmagic", "pocket 6k", "Pocket Cinema Camera 6K"),
        ("Blackmagic", "bmpcc6k pro", "Pocket Cinema Camera 6K Pro"),
        ("GoPro", "hero5", "HERO5 Black"), ("GoPro", "gopro5", "HERO5 Black"),
        ("GoPro", "hero6", "HERO6 Black"), ("GoPro", "gopro6", "HERO6 Black"),
        ("GoPro", "hero7", "HERO7 Black"), ("GoPro", "gopro7", "HERO7 Black"),
        ("GoPro", "hero8", "HERO8 Black"), ("GoPro", "gopro8", "HERO8 Black"),
        ("GoPro", "hero9", "HERO9 Black"), ("GoPro", "gopro9", "HERO9 Black"),
        ("GoPro", "hero10", "HERO10 Black"), ("GoPro", "gopro10", "HERO10 Black"),
        ("GoPro", "hero11", "HERO11 Black"), ("GoPro", "gopro11", "HERO11 Black"),
        ("GoPro", "hero12", "HERO12 Black"), ("GoPro", "gopro12", "HERO12 Black"),
        ("GoPro", "hero13", "HERO13 Black"), ("GoPro", "gopro13", "HERO13 Black"),
        ("Panasonic", "gh5", "GH5"), ("Panasonic", "gh6", "GH6"), ("Panasonic", "s5", "S5"), ("Panasonic", "s5ii", "S5 II"),
        ("Fujifilm", "xt3", "X-T3"), ("Fujifilm", "xt4", "X-T4"), ("Fujifilm", "xt5", "X-T5"),
        ("RED", "komodo", "KOMODO 6K"), ("RED", "komodo-x", "KOMODO-X 6K S35"), ("RED", "raptor", "V-RAPTOR 8K VV"),
    ];

    for (b, alias, canon) in known_model_aliases {
        let b_str = b.to_string();
        let canon_str = canon.to_string();
        let key = format!("{}:{}", b.to_ascii_lowercase(), alias.to_ascii_lowercase());
        raw.model_aliases.insert(key, canon_str.clone());

        let (mount, sensor_type, crop_factor, sensor_w, sensor_h) = infer_camera_info(&b_str, &canon_str);
        let cam_entry = camera_set.entry((b_str.clone(), canon_str.clone())).or_insert_with(|| CameraInfo {
            brand: b_str.clone(),
            model: canon_str.clone(),
            mount,
            sensor_type,
            crop_factor,
            sensor_width: sensor_w,
            sensor_height: sensor_h,
            aliases: Vec::new(),
        });
        if !cam_entry.aliases.contains(&alias.to_string()) {
            cam_entry.aliases.push(alias.to_string());
        }
    }

    raw.cameras = camera_set.into_values().collect();
    raw.lenses = lens_set.into_values().collect();

    // Sort cleanly
    raw.cameras.sort_by(|a, b| {
        let b_cmp = a.brand.cmp(&b.brand);
        if b_cmp != std::cmp::Ordering::Equal { b_cmp } else { a.model.cmp(&b.model) }
    });
    raw.lenses.sort_by(|a, b| {
        let b_cmp = a.brand.cmp(&b.brand);
        if b_cmp != std::cmp::Ordering::Equal { b_cmp } else { a.model.cmp(&b.model) }
    });

    println!("Total structured cameras: {}", raw.cameras.len());
    println!("Total structured lenses: {}", raw.lenses.len());
    println!("Total structured mounts: {}", raw.mounts.len());
    println!("Total brand aliases: {}", raw.brand_aliases.len());

    let json_output = serde_json::to_string_pretty(&raw)?;
    let out_path = "../../resources/camera_database.json";
    std::fs::write(out_path, &json_output)?;
    println!("Successfully written {} bytes to {}", json_output.len(), out_path);

    // Verify it loads with CameraDatabase::from_json
    let db = CameraDatabase::from_json(&json_output)?;
    println!("Verification passed! CameraDatabase parsed successfully with {} brands and {} mounts.",
        db.get_brands().len(), db.mounts.len()
    );

    Ok(())
}
