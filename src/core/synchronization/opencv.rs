use nalgebra::{Vector2, Rotation3};
use std::ffi::c_void;
use opencv::core::{Mat, Size, Point2f, TermCriteria, CV_8UC1};
use opencv::prelude::MatTraitConst;

// use opencv::prelude::{PlatformInfoTraitConst, DeviceTraitConst, UMatTraitConst};
// use opencv::core::{UMat, UMatUsageFlags, AccessFlag::ACCESS_READ};

#[derive(Default, Clone)]
pub struct ItemOpenCV {
    features: Mat,
    img_bytes: Vec<u8>,
    size: (i32, i32),
}
unsafe impl Send for ItemOpenCV { }
unsafe impl Sync for ItemOpenCV { }

impl ItemOpenCV {
    pub fn detect_features(_frame: usize, img: image::GrayImage) -> Self {
        let (w, h) = (img.width() as i32, img.height() as i32);
        let mut bytes = img.into_raw();
        let inp = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();
        
        // opencv::imgcodecs::imwrite("D:/test.jpg", &inp, &opencv::types::VectorOfi32::new());
        
        let mut pts = Mat::default();

        //let inp = inp.get_umat(ACCESS_READ, UMatUsageFlags::USAGE_DEFAULT).unwrap();
        //let mut pts = UMat::new(UMatUsageFlags::USAGE_DEFAULT);

        let _ = opencv::imgproc::good_features_to_track(&inp, &mut pts, 1000, 0.01, 10.0, &Mat::default(), 3, false, 0.04);

        //let pts = pts.get_mat(ACCESS_READ).unwrap().clone();
        Self {
            features: pts,
            size: (w, h),
            img_bytes: bytes
        }
    }
    
    pub fn get_features_count(&self) -> usize {
        self.features.rows() as usize
    }
    pub fn get_feature_at_index(&self, i: usize) -> (f32, f32) {
        let pt = self.features.at::<Point2f>(i as i32).unwrap();
        (pt.x, pt.y)
    }
    
    pub fn estimate_pose(&mut self, next: &mut Self, focal: Vector2<f64>, principal: Vector2<f64>) -> Option<Rotation3<f64>> {    
        let (w, h) = self.size;

        let a1_img = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, self.img_bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();
        let a2_img = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, next.img_bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();
        
        let a1_pts = &self.features;
        //let a2_pts = a2.features;
        
        let mut a2_pts = Mat::default();
        let mut status = Mat::default();
        let mut err = Mat::default();

        let _ = opencv::video::calc_optical_flow_pyr_lk(&a1_img, &a2_img, &a1_pts, &mut a2_pts, &mut status, &mut err, Size::new(21, 21), 3, TermCriteria::new(3/*count+eps*/,30,0.01).unwrap(), 0, 1e-4);

        let mut pts1: Vec<Point2f> = Vec::new();
        let mut pts2: Vec<Point2f> = Vec::new();
        for i in 0..status.rows() {
            if *status.at::<u8>(i).unwrap() == 1u8 {
                let pt1 = a1_pts.at::<Point2f>(i).unwrap();
                let pt2 = a2_pts.at::<Point2f>(i).unwrap();
                if pt1.x >= 0.0 && pt1.x < w as f32 && pt1.y >= 0.0 && pt1.y < h as f32 {
                    if pt2.x >= 0.0 && pt2.x < w as f32 && pt2.y >= 0.0 && pt2.y < h as f32 {
                        pts1.push(*pt1);
                        pts2.push(*pt2);
                    }
                }
            }
        }
        let a1_pts = Mat::from_slice(&pts1).unwrap();
        let a2_pts = Mat::from_slice(&pts2).unwrap();
        
        let scaled_k = Mat::from_slice_2d(&[
            [focal.x, 0.0, principal.x],
            [0.0, focal.y, principal.y],
            [0.0, 0.0, 1.0]
        ]).unwrap();

        // let e = opencv::calib3d::find_essential_mat(&a1_pts, &a2_pts, &scaled_k, &Mat::default(), &scaled_k, &Mat::default(), opencv::calib3d::RANSAC, 0.999, 0.1, &mut Mat::default()).unwrap();
        let e = opencv::calib3d::find_essential_mat(&a1_pts, &a2_pts, &scaled_k, opencv::calib3d::RANSAC, 0.999, 0.1, 1000, &mut Mat::default()).unwrap();
    
        let mut r1 = Mat::default();
        let mut r2 = Mat::default();
        let mut t = Mat::default();
        let _ = opencv::calib3d::decompose_essential_mat(&e, &mut r1, &mut r2, &mut t);
        
        let r1 = cv_to_rot2(r1);
        let r2 = cv_to_rot2(r2);
    
        Some(if r1.angle() < r2.angle() {
            r1
        } else {
            r2
        })
    }
}

pub fn init() -> Result<(), opencv::Error> {
    /*let opencl_have = opencv::core::have_opencl().unwrap();
    if opencl_have {
        opencv::core::set_use_opencl(true)?;
        let mut platforms = opencv::types::VectorOfPlatformInfo::new();
        opencv::core::get_platfoms_info(&mut platforms)?;
        for (platf_num, platform) in platforms.into_iter().enumerate() {
            println!("Platform #{}: {}", platf_num, platform.name()?);
            for dev_num in 0..platform.device_number()? {
                let mut dev = opencv::core::Device::default();
                platform.get_device(&mut dev, dev_num)?;
                println!("  OpenCL device #{}: {}", dev_num, dev.name()?);
                println!("    vendor:  {}", dev.vendor_name()?);
                println!("    version: {}", dev.version()?);
            }
        }
    }
    let opencl_use = opencv::core::use_opencl()?;
    println!(
        "OpenCL is {} and {}",
        if opencl_have { "available" } else { "not available" },
        if opencl_use { "enabled" } else { "disabled" },
    );*/
    Ok(())
}

fn cv_to_rot2(r1: Mat) -> Rotation3<f64> {
    Rotation3::from_matrix_unchecked(nalgebra::Matrix3::new(
        *r1.at_2d::<f64>(0, 0).unwrap(), *r1.at_2d::<f64>(0, 1).unwrap(), *r1.at_2d::<f64>(0, 2).unwrap(),
        *r1.at_2d::<f64>(1, 0).unwrap(), *r1.at_2d::<f64>(1, 1).unwrap(), *r1.at_2d::<f64>(1, 2).unwrap(),
        *r1.at_2d::<f64>(2, 0).unwrap(), *r1.at_2d::<f64>(2, 1).unwrap(), *r1.at_2d::<f64>(2, 2).unwrap()
    ))
}
