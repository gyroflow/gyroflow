use opencv::core::{ Mat, Size, Point2f, Vector, Point3d, TermCriteria, CV_8UC1 };
use opencv::prelude::MatTraitConst;
use opencv::calib3d::CALIB_CB_ADAPTIVE_THRESH;
use opencv::calib3d::CALIB_CB_NORMALIZE_IMAGE;
use opencv::calib3d::CALIB_CB_FAST_CHECK;
use opencv::calib3d::Fisheye_CALIB_RECOMPUTE_EXTRINSIC;
use opencv::calib3d::Fisheye_CALIB_FIX_SKEW;
use rand::seq::SliceRandom;
use std::ffi::c_void;
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst };
use std::sync::Arc;
use nalgebra::{ Matrix3, Vector4 };
use opencv::core::TermCriteria_Type;
use std::collections::{BTreeMap, HashSet};
use parking_lot::RwLock;
use rayon::iter::{ ParallelIterator, IntoParallelIterator };

/// The basic idea here is to find chessboard every 10 frames and save all points to a map. 
/// Then we pick a random 10 frames from that map and calculate the calibration.
/// Repeat that step 1000 times, with new random set of frames each time and return the set which resulted in the lowest RMS

#[derive(Clone, Default, Debug)]
pub struct Detected {
    pub points: Vec<(f32, f32)>,
    pub frame: i32,
    pub timestamp_us: i64,
    pub avg_sharpness: f64,
    pub is_forced: bool
}
#[derive(Default)]
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
    pub d: Vector4<f64>,

    pub forced_frames: HashSet<i32>,

    pub all_matches: Arc<RwLock<BTreeMap<i32, Detected>>>, // frame, Detected
    pub image_points: Arc<RwLock<BTreeMap<i32, Detected>>>, // frame, Detected
    pub used_points: BTreeMap<i32, Detected>
}

impl LensCalibrator {
    pub fn new() -> Self {
        let mut ret = Self {
            columns: 14,
            rows: 8,

            max_images: 10,
            iterations: 1000,

            max_sharpness: 5.0,

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

    pub fn feed_frame<F>(&mut self, timestamp_us: i64, frame: i32, width: u32, height: u32, stride: usize, pixels: &[u8], cancel_flag: Arc<AtomicBool>, total: usize, processed_imgs: Arc<AtomicUsize>, progress: F) -> Result<(), opencv::Error>
    where F: Fn((usize, usize, usize, f64)) + Send + Sync + Clone + 'static {
        let subpix_criteria = TermCriteria::new(TermCriteria_Type::EPS as i32 | TermCriteria_Type::COUNT as i32, 30, 0.001)?;

        self.width = width as usize;
        self.height = height as usize;
        let grid_size = Size::new(self.columns as i32, self.rows as i32);
        let max_sharpness = self.max_sharpness;

        let pixels = pixels.to_vec();
        let img_points = self.image_points.clone();
        let all_matches = self.all_matches.clone();
        let is_forced = self.forced_frames.contains(&frame);

        if let Some(detected) = all_matches.read().get(&frame) {
            if detected.avg_sharpness < max_sharpness {
                img_points.write().insert(frame, detected.clone());
            }
            progress((processed_imgs.fetch_add(1, SeqCst) + 1, total, img_points.read().len(), 0.0));
            return Ok(());
        }

        crate::run_threaded(move || {
            let _ = (|| -> Result<(), opencv::Error> {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(());
                }

                let inp = unsafe { Mat::new_size_with_data(Size::new(width as i32, height as i32), CV_8UC1, pixels.as_ptr() as *mut c_void, stride as usize)? };
            
                let mut corners = Mat::default();

                if opencv::calib3d::find_chessboard_corners(&inp, grid_size, &mut corners, CALIB_CB_ADAPTIVE_THRESH | CALIB_CB_NORMALIZE_IMAGE | CALIB_CB_FAST_CHECK)? {
                    if opencv::calib3d::find_chessboard_corners(&inp, grid_size, &mut corners, CALIB_CB_ADAPTIVE_THRESH | CALIB_CB_NORMALIZE_IMAGE)? {
                        
                        opencv::imgproc::corner_sub_pix(&inp, &mut corners, Size::new(11, 11), Size::new(-1, -1), subpix_criteria)?;

                        if corners.rows() > 0 {
                            let sharpness = opencv::calib3d::estimate_chessboard_sharpness(&inp, grid_size, &corners, 0.8, false, &mut Mat::default()).unwrap_or_default();
                            let avg_sharpness = *sharpness.get(0).unwrap_or(&100.0);
                            let mut points = Vec::with_capacity(corners.rows() as usize);
                            for (_pos, pt) in corners.iter::<Point2f>()? {
                                points.push((pt.x, pt.y));
                            }
                            log::debug!("avg sharpness: {:.5}, max: {:.5}", avg_sharpness, max_sharpness);
                            if avg_sharpness < max_sharpness || is_forced { 
                                img_points.write().insert(frame, Detected { points: points.clone(), timestamp_us, frame, avg_sharpness, is_forced });
                            }
                            all_matches.write().insert(frame, Detected { points, timestamp_us, avg_sharpness, frame, is_forced });
                            return Ok(());
                        }
                    }
                }
                Err(opencv::Error::new(0, "Chessboard not found".to_string()))
            })();
            progress((processed_imgs.fetch_add(1, SeqCst) + 1, total, img_points.read().len(), 0.0));
        });
        Ok(())
    }

    pub fn calibrate(&mut self, only_used: bool) -> Result<(), opencv::Error> {
        let calib_criteria = TermCriteria::new(TermCriteria_Type::EPS as i32 | TermCriteria_Type::COUNT as i32, 30, 1e-6)?;

        let found_frames: Vec<i32> = if only_used {
            self.used_points.keys().copied().collect()
        } else {
            self.image_points.read().keys().copied().collect()
        };
        
        let find_min = |a: (f64, Matrix3::<f64>, Vector4::<f64>, Vec<i32>), b: (f64, Matrix3::<f64>, Vector4::<f64>, Vec<i32>)| -> (f64, Matrix3::<f64>, Vector4::<f64>, Vec<i32>) { if a.0 < b.0 { a } else { b } };

        let image_points = self.image_points.read().clone();
        let size = Size::new(self.width as i32, self.height as i32);
        let objp = self.objp.clone();
        let max_images = if only_used { found_frames.len() } else { self.max_images };
        let forced_frames = self.forced_frames.clone();

        let mut iterations = self.iterations;
        if found_frames.len() <= max_images || max_images == 0 || only_used {
            iterations = 1;
        }
        let result = (0..iterations).into_par_iter().map(|_| {
            let candidate_frames: HashSet<i32> = (&found_frames)
                .choose_multiple(&mut rand::thread_rng(), max_images)
                .copied()
                .chain(forced_frames.iter().copied())
                .collect();
  
            println!("forced_frames: {:?}", &forced_frames);
            println!("candidates: {:?}", &candidate_frames);

            let imgpoints = Vector::<Vector<Point2f>>::from_iter(
                candidate_frames.iter().filter_map(|k| Some(Vector::from_iter(
                    image_points.get(k)?.points.iter().map(|(x, y)| Point2f::new(*x as f32, *y as f32))
                ))
            ));
            let objpoints = Vector::<Vector<Point3d>>::from_iter(
                (0..imgpoints.len()).into_iter().map(|_| Vector::<Point3d>::from_iter(
                    objp.iter().map(|(x, y)| Point3d::new(*x, *y, 0.0))
                ))
            );
        
            let mut k  = Mat::default(); let mut d  = Mat::default();
            let mut rv = Mat::default(); let mut tv = Mat::default();
        
            if let Ok(rms) = opencv::calib3d::calibrate(&objpoints, &imgpoints, size, &mut k, &mut d, &mut rv, &mut tv, Fisheye_CALIB_RECOMPUTE_EXTRINSIC | Fisheye_CALIB_FIX_SKEW, calib_criteria) {
                if let Ok(k) = cv_to_mat3(k) {
                    if let Ok(d) = cv_to_vec4(d) {
                        return (rms, k, d, candidate_frames.into_iter().collect::<Vec<_>>());
                    }
                }
            }
            (999.0000, Matrix3::<f64>::default(), Vector4::<f64>::default(), Vec::new())
        }).reduce_with(find_min);

        if let Some((rms, k, d, used_frames)) = result {
            // TODO add this to undistortion
            //if center_camera {
            //    k[(0, 2)] = self.width as f64 / 2.0;
            //    k[(1, 2)] = self.height as f64 / 2.0;
            //}

            self.k = k;
            self.d = d;
            self.rms = rms;
            self.used_points = used_frames.into_iter().filter_map(|f| Some((f, image_points.get(&f)?.clone()))).collect();

            Ok(())
        } else {
            Err(opencv::Error::new(0, "Unable to calibrate camera".to_string()))
        }
    }

    pub fn draw_chessboard_corners(&self, w: u32, _h: u32, s: usize, pixels: &mut [u8], pattern_size: (usize, usize), corners: &[(f32, f32)], found: bool) {
        let r = 4.0;
        let ratio = w as f32 / self.width as f32;
        if !found {
            let color = (0, 0, 255);

            for x in corners {
                let pt = ((x.0 * ratio).round(), (x.1 * ratio).round());
                line(s, pixels, (pt.0 - r, pt.1 - r), (pt.0 + r, pt.1 + r), color);
                line(s, pixels, (pt.0 - r, pt.1 + r), (pt.0 + r, pt.1 - r), color);
                circle(s, pixels, pt, r + 1.0, color);
            }
        } else {
            let line_colors = [
                (0, 0, 255),
                (0, 128, 255),
                (0, 200, 200),
                (0, 255, 0),
                (200, 200, 0),
                (255, 0, 0),
                (255, 0, 255)
            ];

            let mut prev_pt = (0.0, 0.0);
            let mut i = 0;
            for y in 0..pattern_size.1 {
                let color = line_colors[y % line_colors.len()];

                for _x in 0..pattern_size.0 {
                    let pt = corners[i];
                    let pt = ((pt.0 * ratio).round(), (pt.1 * ratio).round());

                    if i != 0 {
                        line(s, pixels, prev_pt, pt, color);
                    }

                    line(s, pixels, (pt.0 - r, pt.1 - r), (pt.0 + r, pt.1 + r), color);
                    line(s, pixels, (pt.0 - r, pt.1 + r), (pt.0 + r, pt.1 - r), color);
                    circle(s, pixels, pt, r + 1.0, color);
                    prev_pt = pt;
                    i += 1;
                }
            }
        }
    }
}

fn line(s: usize, pixels: &mut [u8], p1: (f32, f32), p2: (f32, f32), color: (u8, u8, u8)) {
    let line = line_drawing::Bresenham::new((p1.0 as isize, p1.1 as isize), (p2.0 as isize, p2.1 as isize)); 
    for point in line {
        let pos = (point.1 * s as isize + point.0 * 4) as usize;
        if pixels.len() > pos + 2 { 
            pixels[pos + 0] = color.0; // R
            pixels[pos + 1] = color.1; // G
            pixels[pos + 2] = color.2; // B
        }
    }
}
fn circle(s: usize, pixels: &mut [u8], center: (f32, f32), radius: f32, color: (u8, u8, u8)) {
    let line = line_drawing::BresenhamCircle::new(center.0 as isize, center.1 as isize, radius as isize); 
    for point in line {
        let pos = (point.1 * s as isize + point.0 * 4) as usize;
        if pixels.len() > pos + 2 { 
            pixels[pos + 0] = color.0; // R
            pixels[pos + 1] = color.1; // G
            pixels[pos + 2] = color.2; // B
        }
    }
}

fn cv_to_mat3(r1: Mat) -> Result<Matrix3<f64>, opencv::Error> {
    if r1.typ() != opencv::core::CV_64FC1 {
        return Err(opencv::Error::new(0, "Invalid matrix type".into()));
    }
    Ok(Matrix3::new(
        *r1.at_2d::<f64>(0, 0)?, *r1.at_2d::<f64>(0, 1)?, *r1.at_2d::<f64>(0, 2)?,
        *r1.at_2d::<f64>(1, 0)?, *r1.at_2d::<f64>(1, 1)?, *r1.at_2d::<f64>(1, 2)?,
        *r1.at_2d::<f64>(2, 0)?, *r1.at_2d::<f64>(2, 1)?, *r1.at_2d::<f64>(2, 2)?
    ))
}

fn cv_to_vec4(v: Mat) -> Result<Vector4<f64>, opencv::Error> {
    if v.typ() != opencv::core::CV_64FC1 {
        return Err(opencv::Error::new(0, "Invalid matrix type".into()));
    }
    Ok(Vector4::new(
        *v.at::<f64>(0)?,
        *v.at::<f64>(1)?,
        *v.at::<f64>(2)?,
        *v.at::<f64>(3)?
    ))
}

// https://github.com/Tangram-Vision/Tangram-Vision-Blog/blob/main/2021.05.28_CalibrationFromScratch/src/main.rs