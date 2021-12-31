use serde::{ Serialize, Deserialize };
use super::LensCalibrator;

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Dimensions { w: usize, h: usize }

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[allow(non_snake_case)]
struct CameraParams { RMS_error: f64, camera_matrix: Vec<Vec<f64>>, distortion_coeffs: Vec<f64> }

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct LensProfile {
    name: String,
    note: String,
    calibrated_by: String,
    camera_brand: String,
    camera_model: String,
    lens_model: String,
    camera_setting: String,

    calib_dimension: Dimensions,
    orig_dimension: Dimensions,

    output_dimension: Option<Dimensions>,

    frame_readout_time: Option<f64>,

    input_horizontal_stretch: f64,
    num_images: usize,

    fps: f64,

    use_opencv_fisheye: bool,
    fisheye_params: CameraParams,

    use_opencv_standard: bool,
    calib_params: CameraParams,

    identifier: String,
    
    calibrator_version: String,
    date: String,
}

impl LensProfile {
    pub fn from_json(json: &str) -> Result<Self, serde_json::error::Error> {
        serde_json::from_str(json)
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

    pub fn save_to_file(&mut self, path: &str) -> std::io::Result<String> {
        let json = self.get_json()?;

        std::fs::write(path, &json)?;

        Ok(json)
    }
}
