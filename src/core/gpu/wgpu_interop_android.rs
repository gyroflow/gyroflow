// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use wgpu::hal::api::Vulkan;
use ash::vk::{ self, ImageCreateInfo, BufferCreateInfo };


pub fn create_vk_image_from_android_hw_buffer(hw_buffer: *mut std::ffi::c_void, device: &wgpu::Device, size: (usize, usize, usize), format: wgpu::TextureFormat) -> Result<(vk::Image, CudaSharedMemory), Box<dyn std::error::Error>> {
    let handle_type =  vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID;

    unsafe {
        let hdevice = device.as_hal::<Vulkan>();
        let raw_image = {
            hdevice.map(|device| {
                let raw_device = device.raw_device();

                let mut import_memory_info = vk::ImportAndroidHardwareBufferInfoANDROID::default()
                    .buffer(hw_buffer as _);

                let image_create_info = ImageCreateInfo::default()
                    .push_next(
                        &mut vk::ExternalMemoryImageCreateInfo::default().handle_types(
                            vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID
                        )
                    )
                    .push_next(
                        &mut vk::ExternalFormatANDROID::default()
                            .external_format(self.input_format_properties.external_format),
                    )
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(super::wgpu_interop_vulkan::format_wgpu_to_vulkan(format))
                    .extent(vk::Extent3D { width: size.0 as u32, height: size.1 as u32, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    //.tiling(vk::ImageTiling::LINEAR)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);

                let raw_image = raw_device.create_image(&image_create_info, None)?;

                let layout = raw_device.get_image_subresource_layout(raw_image, vk::ImageSubresource::default());
                cuda_mem.vulkan_pitch_alignment = layout.row_pitch as usize;

                let memory_type_index = {
                    let mem_requirements = raw_device.get_image_memory_requirements(raw_image);
                    let memory_properties = device.shared_instance().raw_instance().get_physical_device_memory_properties(device.raw_physical_device());
                    let mut memory_type_index = 0;
                    for i in 0..memory_properties.memory_type_count as usize {
                        if (mem_requirements.memory_type_bits & (1 << i)) == 0 {
                            continue;
                        }
                        let properties = memory_properties.memory_types[i].property_flags;
                        if properties.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL) {
                            memory_type_index = i;
                            break;
                        }
                    }
                    memory_type_index as u32
                };

                let allocate_info = vk::MemoryAllocateInfo::default()
                    .allocation_size(cuda_mem.cuda_alloc_size as u64)
                    .push_next(&mut import_memory_info)
                    .memory_type_index(memory_type_index);

                let allocated_memory = raw_device.allocate_memory(&allocate_info, None)?;

                raw_device.bind_image_memory(raw_image, allocated_memory, 0)?;

                Ok::<ash::vk::Image, vk::Result>(raw_image)
            })
        }?;

        Ok((raw_image, cuda_mem))
    }
}
