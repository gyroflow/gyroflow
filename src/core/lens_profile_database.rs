use walkdir::WalkDir;
use std::collections::HashMap;
use crate::calibration::lens_profile::LensProfile;
use itertools::Itertools;

#[derive(Default)]
pub struct LensProfileDatabase {
    map: HashMap<String, LensProfile>
}

impl LensProfileDatabase {
    pub fn load_all(&mut self) {
        let _time = std::time::Instant::now();

        WalkDir::new("./resources/camera_presets/").into_iter().for_each(|e| {
            if let Ok(entry) = e {
                let f_name = entry.path().to_string_lossy().replace('\\', "/");
                if f_name.ends_with(".json") {
                    let mut data = std::fs::read_to_string(&f_name).unwrap();
                    match LensProfile::from_json(&mut data) {
                        Ok(mut v) => {
                            v.filename = f_name.clone();
                            let key = if !v.identifier.is_empty() { 
                                v.identifier.clone()
                            } else {
                                f_name
                            };
                            self.map.insert(key, v);
                        },
                        Err(e) => {
                            log::error!("Error parsing lens profile: {}: {:?}", f_name, e);
                        }
                    }
                }
            }
        });
        
        ::log::info!("Loaded lens profiles in {:.3}ms", _time.elapsed().as_micros() as f64 / 1000.0);
    }

    pub fn get_all_names(&self) -> Vec<String> {
        let mut ret = Vec::with_capacity(self.map.len());
        for v in self.map.values() {
            if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                let strs = vec![&v.camera_brand, &v.camera_model, &v.lens_model, &v.camera_setting, &v.note].into_iter().filter(|x| !x.is_empty()).join(" ");

                ret.push(format!("{} {} {} {}x{}", strs, v.get_size_str(), v.get_aspect_ratio(), v.calib_dimension.w, v.calib_dimension.h));
            /* } else if !v.name.is_empty() {
                ret.push(v.name.clone());*/
             } else if !v.filename.is_empty() && v.filename.contains('/') {
                //let name = v.filename.clone();
                ret.push(self.cleanup_filename(v.filename.split('/').last().unwrap_or_default().to_string()));
            } else {
                log::debug!("Unknown: {:?}", v);
            }
        }
        ret.sort();
        ret
    }
    pub fn cleanup_filename(&self, name: String) -> String {
        name.replace(".json", "").replace("_", " ")
        .replace("4_3", "4:3").replace("4by3", "4:3").replace("16_9", "16:9").replace("16by9", "16:9")
        .replace("2_7K", "2.7k").replace("4K", "4k").replace("5K", "5k")
    }

    pub fn get_by_id(&self, id: &str) -> Option<&LensProfile> {
        self.map.get(id)
    }
}
