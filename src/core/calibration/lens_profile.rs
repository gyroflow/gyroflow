use serde::{ Serialize, Deserialize };
use super::LensCalibrator;

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
pub struct Dimensions { pub w: usize, pub h: usize }

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
#[allow(non_snake_case)]
pub struct CameraParams { pub RMS_error: f64, pub camera_matrix: Vec<Vec<f64>>, pub distortion_coeffs: Vec<f64> }

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct LensProfile {
    pub name: String,
    pub note: String,
    pub calibrated_by: String,
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_model: String,
    pub camera_setting: String,

    pub calib_dimension: Dimensions,
    pub orig_dimension: Dimensions,

    pub output_dimension: Option<Dimensions>,

    pub frame_readout_time: Option<f64>,

    pub input_horizontal_stretch: f64,
    pub num_images: usize,

    pub fps: f64,

    pub use_opencv_fisheye: bool,
    pub fisheye_params: CameraParams,

    pub use_opencv_standard: bool,
    pub calib_params: CameraParams,

    pub identifier: String,
    
    pub calibrator_version: String,
    pub date: String,

    #[serde(skip)]
    pub filename: String,
}

impl LensProfile {
    pub fn from_json(json: &mut str) -> Result<Self, simd_json::Error> {
        simd_json::from_str(json)
    }

    pub fn set_from_calibrator(&mut self, cal: &LensCalibrator) {
        self.input_horizontal_stretch = 1.0;
        self.use_opencv_fisheye = true;
        self.calib_dimension = Dimensions { w: cal.width, h: cal.height };
        self.orig_dimension  = Dimensions { w: cal.width, h: cal.height };
        self.num_images = cal.used_points.len();

        self.fisheye_params = CameraParams {
            RMS_error: cal.rms,
            camera_matrix: cal.k.row_iter().map(|x| x.iter().copied().collect::<Vec<f64>>()).collect(),
            distortion_coeffs: cal.d.as_slice().to_vec()
        };
    }

    pub fn get_json(&mut self) -> Result<String, serde_json::error::Error> {
        self.calibrator_version = env!("CARGO_PKG_VERSION").to_string();
        self.date = chrono::Local::today().naive_local().to_string();
        self.name = self.get_name();

        Ok(serde_json::to_string_pretty(&self)?)
    }

    pub fn get_name(&self) -> String {
        format!("{}_{}_{}_{}_{}x{}-{:.2}fps", self.camera_brand, self.camera_model, self.lens_model, self.camera_setting, self.calib_dimension.w, self.calib_dimension.h, self.fps)
    }

    pub fn get_aspect_ratio(&self) -> String {
        let ratios = [(1.0/1.0, "1:1"), (3.0/2.0, "3:2"), (2.0/3.0, "2:3"), (4.0/3.0, "4:3"), (3.0/4.0, "3:4"), (16.0/9.0, "16:9"), (9.0/16.0, "9:16")];
        let ratio = self.calib_dimension.w as f64 / self.calib_dimension.h as f64;
        let mut diffs = ratios.into_iter().map(|x| ((x.0 - ratio).abs(), x.1)).collect::<Vec<_>>();
        diffs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (lowest_diff, ratio_str) = *diffs.first().unwrap();
        if lowest_diff < 0.05 {
            return ratio_str.to_string();
        }

        let gcd = num::integer::gcd(self.calib_dimension.w, self.calib_dimension.h);

        let ratio1 = self.calib_dimension.w / gcd;
        let ratio2 = self.calib_dimension.h / gcd;
        
        if ratio1 >= 20 || ratio2 >= 20 {
            format!("{:.2}:1", ratio)
        } else {
            format!("{}:{}", ratio1, ratio2)
        }
    }
    pub fn get_size_str(&self) -> &'static str {
             if self.calib_dimension.w >= 8000 { "8k" }
        else if self.calib_dimension.w >= 6000 { "6k" }
        else if self.calib_dimension.w >= 5000 { "5k" }
        else if self.calib_dimension.w >= 4000 { "C4k" }
        else if self.calib_dimension.w >= 3840 { "4k" }
        else if self.calib_dimension.w >= 2700 { "2.7k" }
        else if self.calib_dimension.w >= 2500 { "2.5k" }
        else if self.calib_dimension.w >= 2000 { "2k" }
        else if self.calib_dimension.w >= 1920 { "1080p" }
        else if self.calib_dimension.w >= 1280 { "720p" }
        else if self.calib_dimension.w >= 640  { "480p" }
        else { "" }
    }

    pub fn save_to_file(&mut self, path: &str) -> std::io::Result<String> {
        let json = self.get_json()?;

        std::fs::write(path, &json)?;

        Ok(json)
    }
}
