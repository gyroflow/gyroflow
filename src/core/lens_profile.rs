
use std::collections::HashMap;

use simd_json::ValueAccess;
use walkdir::WalkDir;

#[derive(Default, Clone)]
pub struct LensProfile {
    pub camera: String,
    pub lens: String,
    pub camera_setting: String,
    pub calibrated_by: String,
    
    pub calib_dimension: (f64, f64),
    pub camera_matrix: Vec<f64>,
    pub distortion_coeffs: Vec<f64>
}

impl LensProfile {
    pub fn load_from_file(&mut self, path: &str) -> Option<()> {
        let mut data = std::fs::read(path).ok()?;
        let v = simd_json::to_borrowed_value(&mut data).ok()?;

        self.load_from_json_value(&v)?; // TODO unwrap
        Some(())
    }

    pub fn load_from_json_value(&mut self, v: &simd_json::borrowed::Value) -> Option<()> {
        self.camera         = if v.contains_key("camera_brand")   { format!("{} {}", v["camera_brand"].as_str()?, v["camera_model"].as_str()?) } else { String::new() };
        self.lens           = if v.contains_key("lens_model")     { v["lens_model"]    .as_str()?.to_string() } else { String::new() };
        self.camera_setting = if v.contains_key("camera_setting") { v["camera_setting"].as_str()?.to_string() } else { String::new() };
        self.calibrated_by  = if v.contains_key("calibrated_by")  { v["calibrated_by"] .as_str()?.to_string() } else { String::new() };

        let params = &v["fisheye_params"];
        let dim = &v["calib_dimension"];

        self.calib_dimension = (dim["w"].as_i64()? as f64, dim["h"].as_i64()? as f64);

        self.distortion_coeffs = params["distortion_coeffs"].as_array()?.iter().filter_map(|x| x.as_f64()).collect();
            
        self.camera_matrix = params["camera_matrix"].as_array()?.iter()
                .filter_map(|x| x.as_array())
                .flat_map(|x| x.iter())
                .filter_map(|x| x.as_f64())
                .collect();
        
        Some(())
    }

    pub fn get_profiles_list() -> std::io::Result<Vec<String>> {
        Ok(
            WalkDir::new("./resources/camera_presets/").into_iter().filter_map(|e| {
                if let Ok(entry) = e {
                    let f_name = entry.path().to_string_lossy().replace('\\', "/");
                    if f_name.ends_with(".json") {
                        return Some(f_name);
                    }
                }
                None
            }).collect()
        )
    }
    
    pub fn get_info(&self) -> HashMap<String, String> {
        let mut ret = HashMap::new();
        ret.insert("camera"         .into(), self.camera.clone());
        ret.insert("lens"           .into(), self.lens.clone());
        ret.insert("camera_setting" .into(), self.camera_setting.clone());
        ret.insert("calibrated_by"  .into(), self.calibrated_by.clone());
        ret.insert("calib_dimension".into(), format!("{}x{}", self.calib_dimension.0, self.calib_dimension.1));
        ret.insert("coefficients"   .into(), self.distortion_coeffs.iter().map(f64::to_string).collect::<Vec<String>>().join(";"));
        ret.insert("matrix"         .into(), self.camera_matrix.iter().map(f64::to_string).collect::<Vec<String>>().join(";"));
        ret
    }
}
