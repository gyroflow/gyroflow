// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

/// The basic idea here is to find chessboard every 10 frames and save all points to a map.
/// Then we pick a random 10 frames from that map and calculate the calibration.
/// Repeat that 1000 times, with new random set of frames each time and return the set which resulted in the lowest RMS

#[cfg(feature = "use-opencv")]
use opencv::{
    core::{ Mat, Size, Point2f, Vector, Point3d, Point3f, TermCriteria, TermCriteria_Type, CV_8UC1 },
    prelude::{ MatTraitConst, MatTraitConstManual },
    calib3d::{ CALIB_CB_MARKER, Fisheye_CALIB_RECOMPUTE_EXTRINSIC, Fisheye_CALIB_FIX_SKEW }
};

// The calibrator itself needs OpenCV, to find the chessboards (and for its `OpenCV` method); the module builds without
// it for `solver`, which is pure Rust
#[cfg(feature = "opencv")]
use {
    rand::prelude::IteratorRandom,
    std::{ ffi::c_void, collections::BTreeSet },
    std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst },
    rayon::iter::{ ParallelIterator, IntoParallelIterator },
    crate::stabilization::distortion_models::{ DistortionModel, OpenCVFisheye, OpenCVStandard, GenericPolynomial },
};
use std::collections::{ BTreeMap, HashSet };
use std::sync::Arc;
use nalgebra::Matrix3;
use parking_lot::RwLock;

pub mod drawing;
pub mod solver;

/// Which solver `LensCalibrator::calibrate` fits the lens with, from the boards `feed_frame` found
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationMethod {
    /// OpenCV's own calibration of the model (`cv::fisheye::calibrate`, `cv::calibrateCamera`) on `iterations` random
    /// sets of `max_images` frames, the one with the lowest error kept. Its fisheye can't take a board past 90° from
    /// the lens axis, and on a lens that reaches that far it often fails outright
    #[default]
    OpenCV,
    /// [`solver`] on every frame at once, with the model's own projection, so it calibrates lenses wider than 180°
    Gyroflow,
}
impl From<i32> for CalibrationMethod {
    fn from(v: i32) -> Self { if v == 1 { Self::Gyroflow } else { Self::OpenCV } }
}

/// The lens model `LensCalibrator::calibrate` fits
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationModel {
    #[default]
    OpenCVFisheye,
    OpenCVStandard,
    /// OpenCV has no such model: always fitted by [`solver`], whichever the method
    GenericPolynomial,
}
impl CalibrationModel {
    /// The profile's `distortion_model`. `None` for `opencv_fisheye`: it's the default, and the profiles calibrated
    /// before there was a choice leave it out
    pub fn distortion_model(&self) -> Option<&'static str> {
        match self {
            Self::OpenCVFisheye     => None,
            Self::OpenCVStandard    => Some("opencv_standard"),
            Self::GenericPolynomial => Some("generic_polynomial"),
        }
    }
}
impl From<i32> for CalibrationModel {
    fn from(v: i32) -> Self { match v { 1 => Self::OpenCVStandard, 2 => Self::GenericPolynomial, _ => Self::OpenCVFisheye } }
}

#[derive(Clone, Default, Debug)]
pub struct Detected {
    pub points: Vec<(f32, f32)>,
    pub frame: i32,
    pub timestamp_us: i64,
    pub avg_sharpness: f64,
    pub is_forced: bool
}
#[derive(Default, Clone)]
pub struct LensCalibrator {
    pub rows: usize,
    pub columns: usize,

    pub width: usize,
    pub height: usize,

    pub max_images: usize,
    pub iterations: usize,
    pub max_sharpness: f64,
    pub rms: f64,

    pub objp: Vec<(f64, f64)>,

    pub k: Matrix3<f64>,
    /// The model's distortion coefficients, as many as the method fitted
    pub d: Vec<f64>,
    /// The model `k` and `d` were fitted with. `model` is the choice for the next fit, and differs from this one
    /// from when it's changed until a fit with it succeeds
    pub fitted_model: CalibrationModel,

    pub sum_sharpness: Arc<RwLock<f64>>,

    pub forced_frames: HashSet<i32>,

    pub no_marker: bool,

    pub digital_lens: Option<String>,
    pub digital_lens_params: Option<Vec<f64>>,
    pub asymmetrical: bool,

    pub method: CalibrationMethod,
    pub model: CalibrationModel,

    pub all_matches: Arc<RwLock<BTreeMap<i32, Detected>>>, // frame, Detected
    pub image_points: Arc<RwLock<BTreeMap<i32, Detected>>>, // frame, Detected
    pub used_points: BTreeMap<i32, Detected>, // frame, Detected

    /// Counts the fits `calibrate_shared` started, so one that a later one overtook isn't stored over it
    #[cfg(feature = "opencv")]
    fit_generation: u64,
}

#[cfg(feature = "opencv")]
impl LensCalibrator {
    pub fn new() -> Self {
        #[cfg(feature = "opencv")]
        ::log::info!("OpenCV: {}", opencv::core::get_version_string().unwrap_or_default());

        let mut ret = Self {
            columns: 14,
            rows: 8,

            max_images: 10,
            iterations: 1000,

            max_sharpness: 5.0,
            sum_sharpness: Arc::new(RwLock::new(0.0)),

            width: 0,
            height: 0,

            ..Default::default()
        };

        for y in 0..ret.rows {
            for x in 0..ret.columns {
                ret.objp.push((x as f64, y as f64));
            }
        }

        ret
    }

    pub fn clear(&mut self) {
        self.all_matches.write().clear();
        self.image_points.write().clear();
        self.used_points.clear();
    }

    pub fn feed_frame<F>(&mut self, timestamp_us: i64, frame: i32, size: (u32, u32), org_size: (u32, u32), stride: usize, pt_scale: f32, pixels: &[u8], cancel_flag: Arc<AtomicBool>, total: usize, processed_imgs: Arc<AtomicUsize>, progress: F)
    where F: Fn((usize, usize, usize, f64, f64)) + Send + Sync + Clone + 'static {

        self.width = org_size.0 as usize;
        self.height = org_size.1 as usize;
        let grid_size = Size::new(self.columns as i32, self.rows as i32);
        let max_sharpness = self.max_sharpness;

        let mut pixels = pixels.to_vec();
        let img_points = self.image_points.clone();
        let all_matches = self.all_matches.clone();
        let is_forced = self.forced_frames.contains(&frame);
        let sum_sharpness = self.sum_sharpness.clone();

        let digital_lens = self.digital_lens.as_ref().map(|x| DistortionModel::from_name(&x));
        let digital_lens_params_opt = self.digital_lens_params.clone();
        let no_marker = self.no_marker;

        if let Some(detected) = all_matches.read().get(&frame) {
            if detected.avg_sharpness < max_sharpness {
                img_points.write().insert(frame, detected.clone());
                *sum_sharpness.write() += detected.avg_sharpness;
            }
            progress((processed_imgs.fetch_add(1, SeqCst) + 1, total, img_points.read().len(), 0.0, detected.avg_sharpness));
            return;
        }

        crate::run_threaded(move || {
            let avg_sharpness = (|| -> Result<f64, opencv::Error> {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(0.0);
                }

                // Apply contrast and brightness
                let contrast = 2.0;
                let brightness = -50.0;
                for px in pixels.iter_mut() {
                    *px = (*px as f64 * contrast + brightness).min(255.0) as u8;
                }

                let inp1 = unsafe { Mat::new_size_with_data_unsafe(Size::new(size.0 as i32, size.1 as i32), CV_8UC1, pixels.as_ptr() as *mut c_void, stride as usize)? };
                let mut inp = unsafe { Mat::new_size_with_data_unsafe(Size::new(size.0 as i32, size.1 as i32), CV_8UC1, pixels.as_ptr() as *mut c_void, stride as usize)? };

                let _ = opencv::imgproc::equalize_hist(&inp1, &mut inp);

                let mut corners = Mat::default();

                let mut flags = CALIB_CB_MARKER;
                if no_marker {
                    flags = 0;
                }

                if opencv::calib3d::find_chessboard_corners_sb(&inp, grid_size, &mut corners, flags)? {
                    if corners.rows() > 0 {
                        let sharpness = opencv::calib3d::estimate_chessboard_sharpness(&inp, grid_size, &corners, 0.8, false, &mut Mat::default()).unwrap_or_default();
                        let avg_sharpness = *sharpness.get(0).unwrap_or(&100.0);
                        let mut points = Vec::with_capacity(corners.rows() as usize);

                        let mut digital_lens_params = [0f32; 16];
                        if let Some(p) = digital_lens_params_opt {
                            for (i, v) in p.iter().take(16).enumerate() {
                                digital_lens_params[i] = *v as f32;
                            }
                        }
                        // TODO more params
                        let kernel_params = crate::stabilization::KernelParams {
                            width : size.0 as i32,
                            height: size.1 as i32,
                            output_width: size.0 as i32,
                            output_height: size.1 as i32,
                            digital_lens_params,
                            ..Default::default()
                        };

                        for (_pos, mut pt) in corners.iter::<Point2f>()? {
                            if let Some(digital) = &digital_lens {
                                if let Some(mut pt2) = digital.undistort_point((pt.x, pt.y), &kernel_params) {
                                    // TODO
                                    // Move from center to the left, because we trim the right part making it 4:3
                                    //pt2.0 -= 0.125; // (16-4) / (9-3) / 16

                                    pt = Point2f::new(pt2.0, pt2.1);
                                }
                            }
                            points.push((pt.x * pt_scale, pt.y * pt_scale));
                        }
                        log::debug!("avg sharpness: {:.5}, max: {:.5}", avg_sharpness, max_sharpness);
                        if avg_sharpness < max_sharpness || is_forced {
                            img_points.write().insert(frame, Detected { points: points.clone(), timestamp_us, frame, avg_sharpness, is_forced });
                            *sum_sharpness.write() += avg_sharpness;
                        }
                        all_matches.write().insert(frame, Detected { points, timestamp_us, avg_sharpness, frame, is_forced });
                        return Ok(avg_sharpness);
                    }
                }
                Err(opencv::Error::new(0, "Chessboard not found".to_string()))
            })();
            progress((processed_imgs.fetch_add(1, SeqCst) + 1, total, img_points.read().len(), 0.0, avg_sharpness.unwrap_or(0.0)));
        });
    }

    pub fn calibrate(&mut self, only_used: bool) -> Result<(), opencv::Error> {
        if self.method == CalibrationMethod::Gyroflow || self.model == CalibrationModel::GenericPolynomial {
            return self.calibrate_with_solver(only_used);
        }
        let model = self.model;
        let calib_criteria = TermCriteria::new(TermCriteria_Type::EPS as i32 | TermCriteria_Type::COUNT as i32, 30, 1e-6)?;

        let found_frames: BTreeSet<i32> = if only_used {
            self.used_points.keys().copied().collect()
        } else {
            self.image_points.read().keys().copied().collect()
        };

        let find_min = |a: (f64, Matrix3::<f64>, Vec<f64>, Vec<i32>), b: (f64, Matrix3::<f64>, Vec<f64>, Vec<i32>)| -> (f64, Matrix3::<f64>, Vec<f64>, Vec<i32>) { if a.0 < b.0 { a } else { b } };

        let image_points = self.image_points.read().clone();
        let mut width = self.width as i32;
        if let Some(digital) = self.digital_lens.as_ref().map(|x| DistortionModel::from_name(&x)) {
            // TODO
            //width = (width as f32 / 1.33333333).round() as i32;
        }
        let size = Size::new(width, self.height as i32);
        let objp = self.objp.clone();
        let max_images = self.max_images;
        let forced_frames = self.forced_frames.clone();

        let mut iterations = self.iterations;
        if found_frames.len() <= max_images || max_images == 0 || only_used {
            iterations = 1;
        }
        let result = (0..iterations).into_par_iter().map(|_| {
            let candidate_frames: BTreeSet<i32> = if iterations > 1 {
                // Dive the entire range to `max_images` even slices
                // Then pick a random frame from each slice
                let mut choosen = BTreeSet::new();
                if let Some(max) = found_frames.iter().max() {
                    if let Some(min) = found_frames.iter().min() {
                        let step = ((*max - *min) as f64 / max_images as f64).floor() as i32;
                        let mut val = *min;
                        for _ in 0..max_images {
                            let range = found_frames.range(val..val + step);
                            if let Some(el) = range.choose(&mut rand::rng()) {
                                choosen.insert(*el);
                            }
                            val += step;
                        }
                    }
                }
                choosen
            } else if only_used {
                // Calculate only using used frames
                found_frames.clone()
            } else {
                // Pick `max_images` random frames from the entire range
                found_frames.iter().copied()
                    .sample(&mut rand::rng(), max_images).into_iter()
                    .collect()
            };

            let final_frames: Vec<i32> = candidate_frames.iter().chain(forced_frames.iter()).filter_map(|k| Some(image_points.get(k)?.frame)).collect();

            if final_frames.len() == 1 {
                return (999.0000, Matrix3::<f64>::default(), Vec::new(), final_frames);
            }

            let imgpoints = Vector::<Vector<Point2f>>::from_iter(
                final_frames.iter().filter_map(|k| Some(Vector::from_iter(
                    image_points.get(k)?.points.iter().map(|(x, y)| Point2f::new(*x as f32, *y as f32))
                ))
            ));

            let mut k  = Mat::default(); let mut d  = Mat::default();
            let mut rv = Mat::default(); let mut tv = Mat::default();
            // let mut nop = Mat::default();

            // match opencv::calib3d::calibrate_camera_ro(&objpoints, &imgpoints, size, 13, &mut k, &mut d, &mut rv, &mut tv, &mut nop, Fisheye_CALIB_RECOMPUTE_EXTRINSIC | Fisheye_CALIB_FIX_SKEW, calib_criteria) {
            let result = if model == CalibrationModel::OpenCVStandard {
                // k1, k2, p1, p2, k3: the coefficients `opencv_standard` reads first. It wants single-precision board points
                let objpoints = Vector::<Vector<Point3f>>::from_iter(
                    (0..imgpoints.len()).into_iter().map(|_| Vector::<Point3f>::from_iter(
                        objp.iter().map(|(x, y)| Point3f::new(*x as f32, *y as f32, 0.0))
                    ))
                );
                opencv::calib3d::calibrate_camera(&objpoints, &imgpoints, size, &mut k, &mut d, &mut rv, &mut tv, 0, calib_criteria)
            } else {
                let objpoints = Vector::<Vector<Point3d>>::from_iter(
                    (0..imgpoints.len()).into_iter().map(|_| Vector::<Point3d>::from_iter(
                        objp.iter().map(|(x, y)| Point3d::new(*x, *y, 0.0))
                    ))
                );
                opencv::calib3d::calibrate(&objpoints, &imgpoints, size, &mut k, &mut d, &mut rv, &mut tv, Fisheye_CALIB_RECOMPUTE_EXTRINSIC | Fisheye_CALIB_FIX_SKEW, calib_criteria)
            };
            match result {
                Ok(rms) => {
                    if let Ok(k) = cv_to_mat3(k) {
                        if let Ok(d) = cv_to_vec(d) {
                            return (rms, k, d, final_frames);
                        }
                    }
                },
                Err(e) => {
                    log::warn!("Failed to calibrate! {:?}", e);
                }
            }
            (999.0000, Matrix3::<f64>::default(), Vec::new(), Vec::new())
        }).reduce_with(find_min);

        if let Some((rms, k, d, used_frames)) = result {
            self.k = k;
            self.d = d;
            self.fitted_model = model;
            self.rms = rms;
            self.used_points = used_frames.into_iter().filter_map(|f| Some((f, image_points.get(&f)?.clone()))).collect();

            Ok(())
        } else {
            Err(opencv::Error::new(0, "Unable to calibrate camera".to_string()))
        }
    }

    /// [`CalibrationMethod::Gyroflow`], and any method for `generic_polynomial`: every frame at once rather than random
    /// sets of them (so `max_images` and `iterations` don't apply), the frames that fit far worse than the rest left
    /// out of `used_points`
    fn calibrate_with_solver(&mut self, only_used: bool) -> Result<(), opencv::Error> {
        let image_points = self.image_points.read().clone();
        let candidates: BTreeSet<i32> = if only_used {
            self.used_points.keys().copied().collect()
        } else {
            image_points.keys().copied().collect()
        };
        let frames: Vec<i32> = candidates.into_iter().chain(self.forced_frames.iter().copied())
            .collect::<BTreeSet<i32>>().into_iter()
            .filter(|f| image_points.contains_key(f))
            .collect();
        let views: Vec<Vec<(f64, f64)>> = frames.iter().map(|f| image_points[f].points.iter().map(|p| (p.0 as f64, p.1 as f64)).collect()).collect();
        let protected: Vec<usize> = (0..frames.len()).filter(|&i| self.forced_frames.contains(&frames[i])).collect();

        let started = std::time::Instant::now();
        let (size, board) = ((self.width, self.height), &self.objp);
        let fit = match self.model {
            CalibrationModel::OpenCVFisheye     => solver::calibrate(&OpenCVFisheye::default(),     board, &views, size, &protected),
            CalibrationModel::OpenCVStandard    => solver::calibrate(&OpenCVStandard::default(),    board, &views, size, &protected),
            CalibrationModel::GenericPolynomial => solver::calibrate(&GenericPolynomial::default(), board, &views, size, &protected),
        };
        let Some(c) = fit else {
            return Err(opencv::Error::new(0, "Unable to calibrate camera".to_string()));
        };
        log::info!("Calibrated {:?} in {:.2}s on {} of {} frames: rms {:.4} px, f = ({:.2}, {:.2}), c = ({:.2}, {:.2}), k = {:?}, boards up to {:.1}° from the axis",
            self.model, started.elapsed().as_secs_f64(), c.kept.len(), frames.len(), c.rms, c.f.0, c.f.1, c.c.0, c.c.1, &c.k[..c.fitted], c.max_corner_angle.to_degrees());
        for &i in protected.iter().filter(|i| !c.kept.contains(i)) {
            log::warn!("Frame {} was added by hand, but its pattern couldn't be fitted with the others, so it's left out", frames[i]);
        }

        self.k = Matrix3::new(c.f.0, 0.0, c.c.0, 0.0, c.f.1, c.c.1, 0.0, 0.0, 1.0);
        self.d = c.k[..c.fitted].to_vec();
        self.fitted_model = self.model;
        self.rms = c.rms;
        self.used_points = c.kept.iter().map(|&i| (frames[i], image_points[&frames[i]].clone())).collect();
        Ok(())
    }

    /// `calibrate` on the calibrator `shared` holds, without holding its lock during the fit, which can take seconds:
    /// a frame added meanwhile, or the preview drawing the boards, only waits for the copy the fit starts from and for
    /// the store. The fit runs on a pool of its own (`fit_pool`), so no job of the app's pool ends up waiting inside it
    /// either. It's stored unless a later fit started meanwhile or the calibrator was replaced. A failed one leaves the
    /// previous fit in place, or clears it with `clear_on_failure`, so an export has nothing to write.
    /// Returns the calibrator with the fit; `None` when there's no calibrator or a later fit overtook this one
    pub fn calibrate_shared(shared: &RwLock<Option<Self>>, only_used: bool, clear_on_failure: bool) -> Option<Result<Self, opencv::Error>> {
        let (mut cal, generation) = {
            let mut lock = shared.write();
            let current = lock.as_mut()?;
            current.fit_generation += 1;
            (current.clone(), current.fit_generation)
        };
        // From a thread outside both pools: a worker of the app's pool waiting for the fit would run other jobs of
        // that pool meanwhile, on top of it
        let result = std::thread::scope(|s| s.spawn(|| fit_pool().install(|| cal.calibrate(only_used))).join())
            .unwrap_or_else(|_| Err(opencv::Error::new(0, "The calibration failed unexpectedly".to_string())));
        if result.is_err() && clear_on_failure {
            cal.clear_fit();
        }

        let mut lock = shared.write();
        let current = lock.as_mut()?;
        if current.fit_generation != generation || !Arc::ptr_eq(&current.image_points, &cal.image_points) {
            return None;
        }
        if result.is_ok() || clear_on_failure {
            current.k            = cal.k;
            current.d            = cal.d.clone();
            current.fitted_model = cal.fitted_model;
            current.rms          = cal.rms;
            current.used_points  = cal.used_points.clone();
        }
        Some(result.map(|()| cal))
    }

    /// Forgets the fit, as if there had never been one
    fn clear_fit(&mut self) {
        self.k = Matrix3::default();
        self.d.clear();
        self.rms = 0.0;
        self.used_points.clear();
    }
}

/// The pool every `calibrate_shared` fit runs its parallel work on (the solver's, OpenCV's random sets), away from
/// the app's pool, whose jobs may be waiting for the calibrator
#[cfg(feature = "opencv")]
fn fit_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().thread_name(|i| format!("Calibration {i}")).build().expect("Failed to start the calibration threads"))
}

#[cfg(feature = "use-opencv")]
fn cv_to_mat3(r1: Mat) -> Result<Matrix3<f64>, opencv::Error> {
    if r1.typ() != opencv::core::CV_64FC1 {
        return Err(opencv::Error::new(0, "Invalid matrix type".to_string()));
    }
    Ok(Matrix3::new(
        *r1.at_2d::<f64>(0, 0)?, *r1.at_2d::<f64>(0, 1)?, *r1.at_2d::<f64>(0, 2)?,
        *r1.at_2d::<f64>(1, 0)?, *r1.at_2d::<f64>(1, 1)?, *r1.at_2d::<f64>(1, 2)?,
        *r1.at_2d::<f64>(2, 0)?, *r1.at_2d::<f64>(2, 1)?, *r1.at_2d::<f64>(2, 2)?
    ))
}

#[cfg(feature = "use-opencv")]
fn cv_to_vec(v: Mat) -> Result<Vec<f64>, opencv::Error> {
    if v.typ() != opencv::core::CV_64FC1 {
        return Err(opencv::Error::new(0, "Invalid matrix type".to_string()));
    }
    (0..v.total() as i32).map(|i| v.at::<f64>(i).copied()).collect()
}

// https://github.com/Tangram-Vision/Tangram-Vision-Blog/blob/main/2021.05.28_CalibrationFromScratch/src/main.rs