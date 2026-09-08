// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::gpu::{BufferDescription, BufferSource, Buffers, wgpu::WgpuWrapper};
use crate::stabilization::{
    FrameTransform, PixelType, RGBAf, Stabilization, distortion_models::DistortionModel,
};

#[test]
#[ignore = "Requires a WGPU device; run explicitly on graphics validation hosts"]
fn residual_warp_matches_cpu_on_wgpu_with_lens_strength_rects_and_inversion() {
    check_renderer(Backend::Wgpu);
}

#[cfg(feature = "use-opencl")]
#[test]
#[ignore = "Requires an OpenCL device; run explicitly on graphics validation hosts"]
fn residual_warp_matches_cpu_on_opencl_with_lens_strength_rects_and_inversion() {
    check_renderer(Backend::OpenCl);
}

#[derive(Clone, Copy)]
enum Backend {
    Wgpu,
    #[cfg(feature = "use-opencl")]
    OpenCl,
}

fn check_renderer(backend: Backend) {
    let devices = match backend {
        Backend::Wgpu => WgpuWrapper::list_devices(),
        #[cfg(feature = "use-opencl")]
        Backend::OpenCl => crate::gpu::opencl::OclWrapper::list_devices(),
    };
    assert!(!devices.is_empty(), "No graphics device available");
    println!("Devices: {devices:?}");
    let selected = std::env::var("GYROFLOW_TEST_GPU")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert!(selected < devices.len(), "Invalid GYROFLOW_TEST_GPU index");
    if matches!(backend, Backend::Wgpu) {
        WgpuWrapper::set_device(selected).unwrap();
    }
    println!("Testing {}", devices[selected]);
    for (w, h, ow, oh, strength, inverted, inset) in [
        (128, 80, 128, 80, 1.0, false, 0),
        (128, 80, 96, 64, 0.4, false, 3),
        (80, 128, 64, 96, 0.0, true, 2),
    ] {
        let mut p = ComputeParams::default();
        p.width = w;
        p.height = h;
        p.output_width = ow;
        p.output_height = oh;
        p.scaled_fps = 30.0;
        p.fov_scale = 0.9;
        p.lens_correction_amount = strength;
        p.distortion_model = DistortionModel::from_name("opencv_standard");
        p.lens.calib_dimension = crate::lens_profile::Dimensions { w, h };
        p.lens.fisheye_params.camera_matrix = vec![
            [100.0, 0.0, w as f64 / 2.0],
            [0.0, 100.0, h as f64 / 2.0],
            [0.0, 0.0, 1.0],
        ];
        p.lens.fisheye_params.distortion_coeffs = vec![-0.12, 0.02, 0.0, 0.0, 0.0];
        p.optical_corrections = Arc::new(BTreeMap::from([(
            0,
            Grid(std::array::from_fn(|i| {
                [
                    0.008 + (i / COLS) as f32 * 0.0005,
                    -0.007 + (i % COLS) as f32 * 0.0002,
                ]
            })),
        )]));
        let mut transform = FrameTransform::at_timestamp(&p, 0.0, 0);
        let k = &mut transform.kernel_params;
        k.width = w as i32;
        k.height = h as i32;
        k.output_width = ow as i32;
        k.output_height = oh as i32;
        k.stride = (w * 16) as i32;
        k.output_stride = (ow * 16) as i32;
        k.interpolation = 2;
        k.bytes_per_pixel = 16;
        k.pix_element_count = 4;
        k.max_pixel_value = 255.0;
        k.pixel_value_limit = 255.0;
        if inverted {
            k.flags |= 128;
        }
        if inset > 0 {
            k.flags |= 64;
        }
        k.source_rect = [0, 0, w as i32, h as i32];
        k.output_rect = [inset, inset, ow as i32 - inset * 2, oh as i32 - inset * 2];
        k.safe_area_rect = [0.0, 0.0, ow as f32, oh as f32];
        let mut input: Vec<f32> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [x as f32, y as f32, (x + y) as f32 / 2.0, 1.0]))
            .collect();
        let mut cpu = vec![0.0_f32; ow * oh * 4];
        let mut gpu = cpu.clone();
        let mut buffers = Buffers {
            input: BufferDescription {
                size: (w, h, w * 16),
                data: BufferSource::Cpu {
                    buffer: bytemuck::cast_slice_mut(&mut input),
                },
                ..Default::default()
            },
            output: BufferDescription {
                size: (ow, oh, ow * 16),
                data: BufferSource::Cpu {
                    buffer: bytemuck::cast_slice_mut(&mut cpu),
                },
                ..Default::default()
            },
        };
        // WGPU's texture path uses fragment centers (x + 0.5, y + 0.5);
        // the CPU API uses integer coordinates. Compare the same positions.
        let mut cpu_params = transform.kernel_params;
        if matches!(backend, Backend::Wgpu) {
            cpu_params.translation2d[0] += 0.5 * ow as f32 / cpu_params.output_rect[2] as f32;
            cpu_params.translation2d[1] += 0.5 * oh as f32 / cpu_params.output_rect[3] as f32;
        }
        assert!(Stabilization::undistort_image_cpu::<2, RGBAf>(
            &mut buffers,
            &cpu_params,
            &p.distortion_model,
            None,
            &transform.matrices,
            &[],
            &transform.mesh_data
        ));
        buffers.output.data = BufferSource::Cpu {
            buffer: bytemuck::cast_slice_mut(&mut gpu),
        };
        match backend {
            Backend::Wgpu => {
                let renderer = WgpuWrapper::new(
                    &transform.kernel_params,
                    RGBAf::wgpu_format().unwrap(),
                    p.distortion_model.clone(),
                    None,
                    &buffers,
                    16,
                )
                .unwrap();
                assert!(renderer.undistort_image(&mut buffers, &transform, &[0; 16]));
            }
            #[cfg(feature = "use-opencl")]
            Backend::OpenCl => {
                use crate::gpu::opencl::OclWrapper;
                OclWrapper::set_device(selected, &buffers).unwrap();
                let renderer = OclWrapper::new(
                    &transform.kernel_params,
                    RGBAf::ocl_names(),
                    p.distortion_model.clone(),
                    None,
                    &buffers,
                    16,
                )
                .unwrap();
                renderer
                    .undistort_image(&mut buffers, &transform, &[0; 16])
                    .unwrap();
            }
        }
        for (x, y) in [(20, 20), (ow / 2, oh / 2)] {
            let i = (y * ow + x) * 4;
            println!(
                "({x}, {y}): CPU {:?}, GPU {:?}",
                &cpu[i..i + 4],
                &gpu[i..i + 4]
            );
        }
        let max_error = (10..oh - 10)
            .flat_map(|y| {
                (10..ow - 10).flat_map(move |x| (0..4).map(move |c| (y * ow + x) * 4 + c))
            })
            .map(|i| {
                assert!(gpu[i].is_finite());
                (gpu[i] - cpu[i]).abs()
            })
            .fold(0.0_f32, f32::max);
        println!("{w}x{h}->{ow}x{oh}, lens={strength}, inverted={inverted}, max error={max_error}");
        assert!(
            max_error <= 0.04,
            "CPU/GPU coordinate mismatch: {max_error}"
        );
        if let Some(directory) = std::env::var_os("GYROFLOW_TEST_DUMP_DIR") {
            let directory = std::path::PathBuf::from(directory).join(format!("{w}x{h}-{ow}x{oh}"));
            std::fs::create_dir_all(&directory).unwrap();
            for (name, bytes) in [
                ("params.bin", bytemuck::bytes_of(&cpu_params)),
                ("input.bin", bytemuck::cast_slice(&input)),
                ("expected.bin", bytemuck::cast_slice(&cpu)),
                ("matrices.bin", bytemuck::cast_slice(&transform.matrices)),
                ("mesh.bin", bytemuck::cast_slice(&transform.mesh_data)),
            ] {
                std::fs::write(directory.join(name), bytes).unwrap();
            }
        }
    }
}
