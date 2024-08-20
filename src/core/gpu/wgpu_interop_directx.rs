// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use wgpu::TextureFormat;
use wgpu::hal::api::Vulkan;
use wgpu::hal::api::Dx12;
use windows::Win32::Graphics::{ Dxgi::*, Dxgi::Common::*, Direct3D11::*, Direct3D12 };
use windows::Win32::Foundation::{ CloseHandle, HANDLE, E_NOINTERFACE, E_FAIL, GENERIC_ALL };
use windows::Win32::System::Threading::{ CreateEventA, WaitForSingleObject };
use windows::core::Interface;
use ash::vk::{ self, ImageCreateInfo };

pub struct DirectX11Fence {
    fence: ID3D11Fence,
    event: HANDLE,
    fence_value: std::sync::atomic::AtomicU64
}
unsafe impl Send for DirectX11Fence {}
impl DirectX11Fence {
    pub fn new(device: &ID3D11Device) -> windows::core::Result<Self> {
        unsafe {
            let device = device.cast::<ID3D11Device5>()?;
            let mut fence: Option<ID3D11Fence> = None;

            device.CreateFence(0, D3D11_FENCE_FLAG_NONE, &mut fence)?;
            let fence = fence.ok_or(windows::core::Error::new(E_FAIL, "Failed to create fence"))?;

            let event = CreateEventA(None, false, false, windows::core::PCSTR::null())?;

            Ok(Self {
                fence,
                event,
                fence_value: Default::default()
            })
        }
    }
    pub fn synchronize(&self, context: &ID3D11DeviceContext) -> windows::core::Result<()> {
        let context = context.cast::<ID3D11DeviceContext4>()?;
        let v = self.fence_value.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        unsafe {
            context.Signal(&self.fence, v)?;
            self.fence.SetEventOnCompletion(v, self.event)?;
            WaitForSingleObject(self.event, 5000);
        }
        Ok(())
    }
}
impl Drop for DirectX11Fence {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.event); }
    }
}

pub struct DirectX11SharedTexture {
    intermediate_texture: ID3D11Texture2D,
    fence: DirectX11Fence,
}
impl DirectX11SharedTexture {
    pub fn synchronized_copy_from(&self, context: &ID3D11DeviceContext, tex: &ID3D11Texture2D) -> windows::core::Result<()> { self.synchronized_copy(context, tex, true) }
    pub fn synchronized_copy_to  (&self, context: &ID3D11DeviceContext, tex: &ID3D11Texture2D) -> windows::core::Result<()> { self.synchronized_copy(context, tex, false) }

    fn synchronized_copy(&self, context: &ID3D11DeviceContext, texture: &ID3D11Texture2D, from: bool) -> windows::core::Result<()> {
        unsafe {
            if let Ok(mutex) = self.intermediate_texture.cast::<IDXGIKeyedMutex>() {
                mutex.AcquireSync(0, 500)?;
                if from {
                    context.CopyResource(&self.intermediate_texture, texture);
                } else {
                    context.CopyResource(texture, &self.intermediate_texture);
                }
                self.fence.synchronize(context)?;
                mutex.ReleaseSync(0)?;
                Ok(())
            } else {
                Err(windows::core::Error::new(E_NOINTERFACE, "Failed to query IDXGIKeyedMutex"))
            }
        }
    }
}

pub fn get_shared_texture_d3d11(device: &ID3D11Device, texture: &ID3D11Texture2D) -> Result<(HANDLE, Option<DirectX11SharedTexture>), Box<dyn std::error::Error>> {
    unsafe {
        // Try to open or create shared handle if possible
        if let Ok(dxgi_resource) = texture.cast::<IDXGIResource1>() {
            if let Ok(handle) = dxgi_resource.CreateSharedHandle(None, DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0, None) {
                if !handle.is_invalid() {
                    return Ok((handle, None));
                }
            }
        }

        // No shared handle and not possible to create one.
        // We need to create a new texture and use texture copy from our original one.
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        desc.MiscFlags |= D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 as u32 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32;

        let mut new_texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut new_texture))?;
        if let Some(new_texture) = new_texture {
            let dxgi_resource: IDXGIResource1 = new_texture.cast::<IDXGIResource1>()?;
            let handle = dxgi_resource.CreateSharedHandle(None, DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0, None)?;

            Ok((handle, Some(DirectX11SharedTexture {
                intermediate_texture: new_texture,
                fence: DirectX11Fence::new(device)?
            })))
        } else {
            Err("Call to CreateTexture2D failed".into())
        }
    }
}

pub fn create_vk_image_from_d3d11_texture(device: &wgpu::Device, d3d11_device: &ID3D11Device, texture: &ID3D11Texture2D) -> Result<(vk::Image, Option<DirectX11SharedTexture>), Box<dyn std::error::Error>> {
    unsafe {
        let (handle, shared_texture) = get_shared_texture_d3d11(d3d11_device, texture)?;

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);

        let raw_image = device.as_hal::<Vulkan, _, _>(|device| {
            device.map(|device| {
                let raw_device = device.raw_device();
                let handle_type = vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE; // D3D12_RESOURCE_KHR

                let mut import_memory_info = vk::ImportMemoryWin32HandleInfoKHR::default()
                    .handle_type(handle_type)
                    .handle(handle.0 as isize);

                let allocate_info = vk::MemoryAllocateInfo::default()
                    .push_next(&mut import_memory_info)
                    .memory_type_index(0);

                let allocated_memory = raw_device.allocate_memory(&allocate_info, None)?;

                let mut ext_create_info = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);

                let image_create_info = ImageCreateInfo::default()
                    .push_next(&mut ext_create_info)
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(super::wgpu_interop_vulkan::format_wgpu_to_vulkan(format_dxgi_to_wgpu(desc.Format)))
                    .extent(vk::Extent3D { width: desc.Width, height: desc.Height, depth: desc.ArraySize })
                    .mip_levels(desc.MipLevels)
                    .array_layers(desc.ArraySize)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);

                let raw_image = raw_device.create_image(&image_create_info, None)?;

                raw_device.bind_image_memory(raw_image, allocated_memory, 0)?;

                let _ = CloseHandle(handle);

                Ok::<ash::vk::Image, vk::Result>(raw_image)
            })
        }).unwrap().unwrap()?; // TODO: unwrap

        Ok((raw_image, shared_texture))
    }
}

pub fn create_dx12_resource_from_d3d11_texture(device: &wgpu::Device, d3d11_device: &ID3D11Device, texture: &ID3D11Texture2D) -> Result<(Direct3D12::ID3D12Resource, Option<DirectX11SharedTexture>), Box<dyn std::error::Error>> {
    unsafe {
        let (handle, shared_texture) = get_shared_texture_d3d11(d3d11_device, texture)?;

        let raw_image = device.as_hal::<Dx12, _, _>(|hdevice| {
            hdevice.map(|hdevice| {
                let raw_device = hdevice.raw_device();

                let mut resource = None::<Direct3D12::ID3D12Resource>;
                match raw_device.OpenSharedHandle(handle, &mut resource) {
                    Ok(_) => Ok(resource.unwrap()),
                    Err(e) => Err(e)
                }
            })
        }).unwrap().unwrap()?; // TODO: unwrap

        Ok((raw_image, shared_texture))
    }
}

pub fn create_texture_from_dx12_resource(device: &wgpu::Device, resource: Direct3D12::ID3D12Resource, desc: &wgpu::TextureDescriptor) -> wgpu::Texture {
    unsafe {
        let texture = <Dx12 as wgpu::hal::Api>::Device::texture_from_raw(resource, desc.format, desc.dimension, desc.size, 1, 1);

        device.create_texture_from_hal::<Dx12>(texture, &desc)
    }
}

/*pub fn create_native_shared_texture_dx12(device: &wgpu::Device, desc: &wgpu::TextureDescriptor) -> Result<(::d3d12::Resource, usize, usize), String> {
    unsafe {
        device.as_hal::<Dx12, _, _>(|hdevice| {
            hdevice.map(|hdevice| {
                let raw_device = hdevice.raw_device();

                let mut resource = None::<Direct3D12::ID3D12Resource>;

                { // Texture
                    let raw_desc = Direct3D12::D3D12_RESOURCE_DESC {
                        Dimension: Direct3D12::D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                        Alignment: 0,
                        Width: desc.size.width as u64,
                        Height: desc.size.height,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        Format: format_wgpu_to_dxgi(desc.format).0,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: desc.sample_count,
                            Quality: 0,
                        },
                        Layout: Direct3D12::D3D12_TEXTURE_LAYOUT_UNKNOWN,
                        Flags: Direct3D12::D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                    };
                    let heap_properties = Direct3D12::D3D12_HEAP_PROPERTIES {
                        Type: Direct3D12::D3D12_HEAP_TYPE_CUSTOM,
                        CPUPageProperty: Direct3D12::D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE,
                        MemoryPoolPreference: Direct3D12::D3D12_MEMORY_POOL_L0,
                        CreationNodeMask: 0,
                        VisibleNodeMask: 0,
                    };

                    raw_device.CreateCommittedResource(
                        &heap_properties,
                        Direct3D12::D3D12_HEAP_FLAG_SHARED,
                        &raw_desc,
                        Direct3D12::D3D12_RESOURCE_STATE_COMMON,
                        None, // clear value
                        &mut resource,
                    ).map_err(|e| format!("{e:?}"))?;
                }

                let resource = resource.unwrap();

                let actual_desc = resource.GetDesc();
                let ai = raw_device.GetResourceAllocationInfo(0, &[actual_desc]);
                let actual_size = ai.SizeInBytes as usize;

                match raw_device.CreateSharedHandle(&resource, None, GENERIC_ALL.0, windows::core::PCWSTR::null()) {
                    Ok(handle) => Ok::<(Direct3D12::ID3D12Resource, HANDLE, usize), String>((resource, handle, actual_size)),
                    Err(e) => Err(e.to_string())
                }
            })
        }).unwrap() // TODO: unwrap
    }
}*/

pub fn create_native_shared_buffer_dx12(device: &wgpu::Device, size: usize) -> Result<(Direct3D12::ID3D12Resource, HANDLE, usize), String> {
    unsafe {
        device.as_hal::<Dx12, _, _>(|hdevice| {
            hdevice.map(|hdevice| {
                let raw_device = hdevice.raw_device();

                let mut resource = None::<Direct3D12::ID3D12Resource>;

                {
                    let raw_desc = Direct3D12::D3D12_RESOURCE_DESC {
                        Dimension: Direct3D12::D3D12_RESOURCE_DIMENSION_BUFFER,
                        Alignment: 0,
                        Width: size as u64,
                        Height: 1,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        Format: DXGI_FORMAT_UNKNOWN,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Layout: Direct3D12::D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                        Flags: Direct3D12::D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                    };
                    let heap_properties = Direct3D12::D3D12_HEAP_PROPERTIES {
                        Type: Direct3D12::D3D12_HEAP_TYPE_CUSTOM,
                        CPUPageProperty: Direct3D12::D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE,
                        MemoryPoolPreference: Direct3D12::D3D12_MEMORY_POOL_L0,
                        CreationNodeMask: 0,
                        VisibleNodeMask: 0,
                    };

                    raw_device.CreateCommittedResource(
                        &heap_properties,
                        Direct3D12::D3D12_HEAP_FLAG_SHARED,
                        &raw_desc,
                        Direct3D12::D3D12_RESOURCE_STATE_COMMON,
                        None, // clear value
                        &mut resource,
                    ).map_err(|e| format!("{e:?}"))?;
                }

                let resource = resource.unwrap();

                let actual_desc = resource.GetDesc();
                let ai = raw_device.GetResourceAllocationInfo(0, &[actual_desc]);
                let actual_size = ai.SizeInBytes as usize;

                match raw_device.CreateSharedHandle(&resource, None, GENERIC_ALL.0, windows::core::PCWSTR::null()) {
                    Ok(handle) => Ok::<(Direct3D12::ID3D12Resource, HANDLE, usize), String>((resource, handle, actual_size)),
                    Err(e) => Err(e.to_string())
                }
            })
        }).unwrap().unwrap() // TODO: unwrap
    }
}

pub fn format_dxgi_to_wgpu(format: DXGI_FORMAT) -> TextureFormat {
    match format {
        DXGI_FORMAT_R8_UNORM => TextureFormat::R8Unorm,
        DXGI_FORMAT_R8_SNORM => TextureFormat::R8Snorm,
        DXGI_FORMAT_R8_UINT => TextureFormat::R8Uint,
        DXGI_FORMAT_R8_SINT => TextureFormat::R8Sint,
        DXGI_FORMAT_R16_UINT => TextureFormat::R16Uint,
        DXGI_FORMAT_R16_SINT => TextureFormat::R16Sint,
        DXGI_FORMAT_R16_UNORM => TextureFormat::R16Unorm,
        DXGI_FORMAT_R16_SNORM => TextureFormat::R16Snorm,
        DXGI_FORMAT_R16_FLOAT => TextureFormat::R16Float,
        DXGI_FORMAT_R8G8_UNORM => TextureFormat::Rg8Unorm,
        DXGI_FORMAT_R8G8_SNORM => TextureFormat::Rg8Snorm,
        DXGI_FORMAT_R8G8_UINT => TextureFormat::Rg8Uint,
        DXGI_FORMAT_R8G8_SINT => TextureFormat::Rg8Sint,
        DXGI_FORMAT_R16G16_UNORM => TextureFormat::Rg16Unorm,
        DXGI_FORMAT_R16G16_SNORM => TextureFormat::Rg16Snorm,
        DXGI_FORMAT_R32_UINT => TextureFormat::R32Uint,
        DXGI_FORMAT_R32_SINT => TextureFormat::R32Sint,
        DXGI_FORMAT_R32_FLOAT => TextureFormat::R32Float,
        DXGI_FORMAT_R16G16_UINT => TextureFormat::Rg16Uint,
        DXGI_FORMAT_R16G16_SINT => TextureFormat::Rg16Sint,
        DXGI_FORMAT_R16G16_FLOAT => TextureFormat::Rg16Float,
        DXGI_FORMAT_R8G8B8A8_TYPELESS => TextureFormat::Rgba8Unorm,
        DXGI_FORMAT_R8G8B8A8_UNORM => TextureFormat::Rgba8Unorm,
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => TextureFormat::Rgba8UnormSrgb,
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => TextureFormat::Bgra8UnormSrgb,
        DXGI_FORMAT_R8G8B8A8_SNORM => TextureFormat::Rgba8Snorm,
        DXGI_FORMAT_B8G8R8A8_UNORM => TextureFormat::Bgra8Unorm,
        DXGI_FORMAT_R8G8B8A8_UINT => TextureFormat::Rgba8Uint,
        DXGI_FORMAT_R8G8B8A8_SINT => TextureFormat::Rgba8Sint,
        DXGI_FORMAT_R10G10B10A2_UNORM => TextureFormat::Rgb10a2Unorm,
        DXGI_FORMAT_R10G10B10A2_UINT => TextureFormat::Rgb10a2Uint,
        DXGI_FORMAT_R11G11B10_FLOAT => TextureFormat::Rg11b10UFloat,
        DXGI_FORMAT_R32G32_UINT => TextureFormat::Rg32Uint,
        DXGI_FORMAT_R32G32_SINT => TextureFormat::Rg32Sint,
        DXGI_FORMAT_R32G32_FLOAT => TextureFormat::Rg32Float,
        DXGI_FORMAT_R16G16B16A16_UINT => TextureFormat::Rgba16Uint,
        DXGI_FORMAT_R16G16B16A16_SINT => TextureFormat::Rgba16Sint,
        DXGI_FORMAT_R16G16B16A16_UNORM => TextureFormat::Rgba16Unorm,
        DXGI_FORMAT_R16G16B16A16_SNORM => TextureFormat::Rgba16Snorm,
        DXGI_FORMAT_R16G16B16A16_FLOAT => TextureFormat::Rgba16Float,
        DXGI_FORMAT_R32G32B32A32_UINT => TextureFormat::Rgba32Uint,
        DXGI_FORMAT_R32G32B32A32_SINT => TextureFormat::Rgba32Sint,
        DXGI_FORMAT_R32G32B32A32_FLOAT => TextureFormat::Rgba32Float,
        DXGI_FORMAT_D32_FLOAT => TextureFormat::Depth32Float,
        DXGI_FORMAT_D32_FLOAT_S8X24_UINT => TextureFormat::Depth32FloatStencil8,
        DXGI_FORMAT_R9G9B9E5_SHAREDEXP => TextureFormat::Rgb9e5Ufloat,
        DXGI_FORMAT_BC1_UNORM => TextureFormat::Bc1RgbaUnorm,
        DXGI_FORMAT_BC1_UNORM_SRGB => TextureFormat::Bc1RgbaUnormSrgb,
        DXGI_FORMAT_BC2_UNORM => TextureFormat::Bc2RgbaUnorm,
        DXGI_FORMAT_BC2_UNORM_SRGB => TextureFormat::Bc2RgbaUnormSrgb,
        DXGI_FORMAT_BC3_UNORM => TextureFormat::Bc3RgbaUnorm,
        DXGI_FORMAT_BC3_UNORM_SRGB => TextureFormat::Bc3RgbaUnormSrgb,
        DXGI_FORMAT_BC4_UNORM => TextureFormat::Bc4RUnorm,
        DXGI_FORMAT_BC4_SNORM => TextureFormat::Bc4RSnorm,
        DXGI_FORMAT_BC5_UNORM => TextureFormat::Bc5RgUnorm,
        DXGI_FORMAT_BC5_SNORM => TextureFormat::Bc5RgSnorm,
        DXGI_FORMAT_BC6H_UF16 => TextureFormat::Bc6hRgbUfloat,
        DXGI_FORMAT_BC6H_SF16 => TextureFormat::Bc6hRgbFloat,
        DXGI_FORMAT_BC7_UNORM => TextureFormat::Bc7RgbaUnorm,
        DXGI_FORMAT_BC7_UNORM_SRGB => TextureFormat::Bc7RgbaUnormSrgb,
        _ => panic!("Unsupported texture format: {:?}", format),
    }
}

pub fn format_wgpu_to_dxgi(format: TextureFormat) -> DXGI_FORMAT {
    match format {
        TextureFormat::R8Unorm => DXGI_FORMAT_R8_UNORM,
        TextureFormat::R8Snorm => DXGI_FORMAT_R8_SNORM,
        TextureFormat::R8Uint => DXGI_FORMAT_R8_UINT,
        TextureFormat::R8Sint => DXGI_FORMAT_R8_SINT,
        TextureFormat::R16Uint => DXGI_FORMAT_R16_UINT,
        TextureFormat::R16Sint => DXGI_FORMAT_R16_SINT,
        TextureFormat::R16Unorm => DXGI_FORMAT_R16_UNORM,
        TextureFormat::R16Snorm => DXGI_FORMAT_R16_SNORM,
        TextureFormat::R16Float => DXGI_FORMAT_R16_FLOAT,
        TextureFormat::Rg8Unorm => DXGI_FORMAT_R8G8_UNORM,
        TextureFormat::Rg8Snorm => DXGI_FORMAT_R8G8_SNORM,
        TextureFormat::Rg8Uint => DXGI_FORMAT_R8G8_UINT,
        TextureFormat::Rg8Sint => DXGI_FORMAT_R8G8_SINT,
        TextureFormat::Rg16Unorm => DXGI_FORMAT_R16G16_UNORM,
        TextureFormat::Rg16Snorm => DXGI_FORMAT_R16G16_SNORM,
        TextureFormat::R32Uint => DXGI_FORMAT_R32_UINT,
        TextureFormat::R32Sint => DXGI_FORMAT_R32_SINT,
        TextureFormat::R32Float => DXGI_FORMAT_R32_FLOAT,
        TextureFormat::Rg16Uint => DXGI_FORMAT_R16G16_UINT,
        TextureFormat::Rg16Sint => DXGI_FORMAT_R16G16_SINT,
        TextureFormat::Rg16Float => DXGI_FORMAT_R16G16_FLOAT,
        TextureFormat::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
        TextureFormat::Rgba8UnormSrgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        TextureFormat::Bgra8UnormSrgb => DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        TextureFormat::Rgba8Snorm => DXGI_FORMAT_R8G8B8A8_SNORM,
        TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
        TextureFormat::Rgba8Uint => DXGI_FORMAT_R8G8B8A8_UINT,
        TextureFormat::Rgba8Sint => DXGI_FORMAT_R8G8B8A8_SINT,
        TextureFormat::Rgb10a2Unorm => DXGI_FORMAT_R10G10B10A2_UNORM,
        TextureFormat::Rg11b10UFloat => DXGI_FORMAT_R11G11B10_FLOAT,
        TextureFormat::Rg32Uint => DXGI_FORMAT_R32G32_UINT,
        TextureFormat::Rg32Sint => DXGI_FORMAT_R32G32_SINT,
        TextureFormat::Rg32Float => DXGI_FORMAT_R32G32_FLOAT,
        TextureFormat::Rgba16Uint => DXGI_FORMAT_R16G16B16A16_UINT,
        TextureFormat::Rgba16Sint => DXGI_FORMAT_R16G16B16A16_SINT,
        TextureFormat::Rgba16Unorm => DXGI_FORMAT_R16G16B16A16_UNORM,
        TextureFormat::Rgba16Snorm => DXGI_FORMAT_R16G16B16A16_SNORM,
        TextureFormat::Rgba16Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
        TextureFormat::Rgba32Uint => DXGI_FORMAT_R32G32B32A32_UINT,
        TextureFormat::Rgba32Sint => DXGI_FORMAT_R32G32B32A32_SINT,
        TextureFormat::Rgba32Float => DXGI_FORMAT_R32G32B32A32_FLOAT,
        TextureFormat::Depth32Float => DXGI_FORMAT_D32_FLOAT,
        TextureFormat::Depth32FloatStencil8 => DXGI_FORMAT_D32_FLOAT_S8X24_UINT,
        TextureFormat::Rgb9e5Ufloat => DXGI_FORMAT_R9G9B9E5_SHAREDEXP,
        TextureFormat::Bc1RgbaUnorm => DXGI_FORMAT_BC1_UNORM,
        TextureFormat::Bc1RgbaUnormSrgb => DXGI_FORMAT_BC1_UNORM_SRGB,
        TextureFormat::Bc2RgbaUnorm => DXGI_FORMAT_BC2_UNORM,
        TextureFormat::Bc2RgbaUnormSrgb => DXGI_FORMAT_BC2_UNORM_SRGB,
        TextureFormat::Bc3RgbaUnorm => DXGI_FORMAT_BC3_UNORM,
        TextureFormat::Bc3RgbaUnormSrgb => DXGI_FORMAT_BC3_UNORM_SRGB,
        TextureFormat::Bc4RUnorm => DXGI_FORMAT_BC4_UNORM,
        TextureFormat::Bc4RSnorm => DXGI_FORMAT_BC4_SNORM,
        TextureFormat::Bc5RgUnorm => DXGI_FORMAT_BC5_UNORM,
        TextureFormat::Bc5RgSnorm => DXGI_FORMAT_BC5_SNORM,
        TextureFormat::Bc6hRgbUfloat => DXGI_FORMAT_BC6H_UF16,
        TextureFormat::Bc6hRgbFloat => DXGI_FORMAT_BC6H_SF16,
        TextureFormat::Bc7RgbaUnorm => DXGI_FORMAT_BC7_UNORM,
        TextureFormat::Bc7RgbaUnormSrgb => DXGI_FORMAT_BC7_UNORM_SRGB,
        _ => panic!("Unsupported texture format: {:?}", format),
    }
}
