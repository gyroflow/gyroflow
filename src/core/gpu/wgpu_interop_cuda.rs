// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use wgpu::hal::api::Vulkan;
use ash::vk::{ self, ImageCreateInfo, BufferCreateInfo };
use std::collections::HashMap;
use parking_lot::RwLock;

use CUmemAllocationType::*;
use CUmemAllocationHandleType::*;
use CUmemLocationType::*;
use CUmemAllocationGranularity_flags::*;
use CUmemAccess_flags::*;

macro_rules! cuda {
    ($cfn:ident.$func:ident($($arg:tt)*)) => {
        let err = unsafe { ($cfn.$func)($($arg)*) };
        if err != CUresult::CUDA_SUCCESS {
            log::error!("Call to {} failed: {:?}", stringify!($func), err);
        }
    };
}

lazy_static::lazy_static! {
    static ref UUIDMAP: RwLock<HashMap<String, usize>> = RwLock::new(HashMap::new());
}

pub struct CudaSharedMemory {
    pub device_ptr: CUdeviceptr,
    pub external_memory: Option<CUexternalMemory>,
    pub shared_handle: isize,
    pub cuda_alloc_size: usize,
    pub vulkan_pitch_alignment: usize,
    pub additional_drop_func: Option<Box<dyn FnOnce()>>
}
impl Drop for CudaSharedMemory {
    fn drop(&mut self) {
        if let Ok(cuda) = CUDA.as_ref() {
            if let Some(ext) = self.external_memory {
                log::debug!("Freeing CUDA pointer: 0x{:08x}", self.device_ptr);
                cuda!(cuda.cuMemFree(self.device_ptr));
                cuda!(cuda.cuDestroyExternalMemory(ext));
            } else {
                log::debug!("Freeing CUDA address: 0x{:08x}", self.device_ptr);
                cuda!(cuda.cuMemUnmap(self.device_ptr, self.cuda_alloc_size));
                cuda!(cuda.cuMemAddressFree(self.device_ptr, self.cuda_alloc_size));
            }
        }
        if let Some(drop_fn) = self.additional_drop_func.take() {
            drop_fn();
        }
    }
}
unsafe impl Send for CudaSharedMemory { }

pub fn get_current_cuda_device() -> i32 {
    let mut dev = 0;
    if let Ok(cuda) = CUDA.as_ref() {
        unsafe { (cuda.cudaGetDevice)(&mut dev); }
    }
    dev
}
pub fn get_device_uuid(device: usize) -> String {
    let mut uuid = CUuuid::default();
    if let Ok(cuda) = CUDA.as_ref() {
        unsafe { (cuda.cuDeviceGetUuid)(&mut uuid, device as _); }
    }
    uuid.bytes.iter().map(|x| format!("{:02x}", x)).collect()
}

pub fn get_current_device_id_by_uuid(adapters: &Vec<wgpu::Adapter>) -> usize {
    let mut adapter_id = get_current_cuda_device() as usize;

    let uuid = get_device_uuid(adapter_id);
    log::debug!("Current device UUID: {uuid}, adapter_id: {adapter_id}");

    let mut found = false;
    {
        let uuidmap = UUIDMAP.read();
        if !uuidmap.is_empty() {
            if let Some(id) = uuidmap.get(&uuid) {
                adapter_id = *id;
                found = true;
            }
        }
    }

    if !found {
        for (i, adapter) in adapters.iter().enumerate() {
            if adapter.get_info().backend != wgpu::Backend::Vulkan {
                continue;
            }
            let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            }, None));
            if let Ok((device, _q)) = device {
                unsafe {
                    let _ = device.as_hal::<wgpu::hal::api::Vulkan, _, _>(|device| {
                        device.map(|device| {
                            let mut id_props = ash::vk::PhysicalDeviceIDProperties::default();
                            let mut device_properties = ash::vk::PhysicalDeviceProperties2::default().push_next(&mut id_props);
                            device.shared_instance().raw_instance().get_physical_device_properties2(device.raw_physical_device(), &mut device_properties);

                            let dev_uuid = id_props.device_uuid.iter().map(|x| format!("{:02x}", x)).collect::<String>();
                            UUIDMAP.write().insert(dev_uuid.clone(), i);
                            log::debug!("Device #{i} uuid: {dev_uuid}");
                            if dev_uuid == uuid {
                                adapter_id = i;
                            }
                        })
                    });
                }
            }
        }
    }
    adapter_id
}

fn align(a: usize, b: usize) -> usize { ((a + b - 1) / b) * b }

pub fn cuda_2d_copy_on_device(size: (usize, usize, usize), dst: CUdeviceptr, src: CUdeviceptr, dst_alignment: usize, src_alignment: usize) {
    let desc = CUDA_MEMCPY2D_st {
        Height: size.1,
        WidthInBytes: size.2,

        dstPitch: align(size.2, dst_alignment),
        dstDevice: dst,
        srcPitch: align(size.2, src_alignment),
        srcDevice: src,

        dstArray: std::ptr::null_mut(),
        dstHost: std::ptr::null_mut(),
        dstMemoryType: CUmemorytype::CU_MEMORYTYPE_DEVICE,
        dstXInBytes: 0,
        dstY: 0,
        srcArray: std::ptr::null_mut(),
        srcHost: std::ptr::null_mut(),
        srcMemoryType: CUmemorytype::CU_MEMORYTYPE_DEVICE,
        srcXInBytes: 0,
        srcY: 0,
    };
    if let Ok(cuda) = CUDA.as_ref() {
        // let mut base = 0; let mut size = 0; let mut size2 = 0;
        // cuda!(cuda.cuMemGetAddressRange_v2(&mut base, &mut size, src));
        // cuda!(cuda.cuMemGetAddressRange_v2(&mut base, &mut size2, dst));
        // log::debug!("cuMemcpy2D_v2 | 0x{src:08x} ({size} bytes) => 0x{dst:08x} ({size2} bytes) | Device {}", get_current_cuda_device());
        // cuda!(cuda.cuMemcpy(dst, src, size.2 * size.1));
        cuda!(cuda.cuMemcpy2D_v2(&desc as *const _));
    }
}

pub fn cuda_synchronize() {
    if let Ok(cuda) = CUDA.as_ref() {
        cuda!(cuda.cuCtxSynchronize());
        //unsafe { (cuda.cudaDeviceSynchronize)(); }
    }
}

pub fn import_external_d3d12_resource(ptr: *mut std::ffi::c_void, size: usize) -> Result<CudaSharedMemory, Box<dyn std::error::Error>> {
    let cuda = CUDA.as_ref()?;
    let desc = CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st {
        type_: CUexternalMemoryHandleType_enum::CU_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_RESOURCE,
        handle: CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 {
            win32: CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1__bindgen_ty_1 {
                handle: ptr,
                name: std::ptr::null_mut()
            }
        },
        size: size as u64,
        flags: 1, // CUDA_EXTERNAL_MEMORY_DEDICATED
        reserved: Default::default(),
    };

    let mut memory: CUexternalMemory = std::ptr::null_mut();
    cuda!(cuda.cuImportExternalMemory(&mut memory, &desc));

    let buffer_desc = CUDA_EXTERNAL_MEMORY_BUFFER_DESC_st {
        flags: 0,
        size: size as u64,
        offset: 0,
        reserved: Default::default(),
    };

    let mut dptr = 0;
    cuda!(cuda.cuExternalMemoryGetMappedBuffer(&mut dptr, memory, (&buffer_desc) as *const _));

    Ok(CudaSharedMemory {
        device_ptr: dptr,
        shared_handle: 0,
        external_memory: Some(memory),
        cuda_alloc_size: size,
        vulkan_pitch_alignment: wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize,
        additional_drop_func: None,
    })
}

pub fn allocate_shared_cuda_memory(size: usize) -> Result<CudaSharedMemory, Box<dyn std::error::Error>> {
    let share_type = if cfg!(target_os = "windows") {
        CU_MEM_HANDLE_TYPE_WIN32
    } else {
        CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR
    };

    let cuda = CUDA.as_ref()?;
    let mut dev = 0;
    unsafe { (cuda.cudaGetDevice)(&mut dev); }
    let location = CUmemLocation { type_: CU_MEM_LOCATION_TYPE_DEVICE, id: dev };

    let mut uuid = CUuuid::default();
    unsafe { (cuda.cuDeviceGetUuid)(&mut uuid, dev); }
    log::debug!("Device #{dev} UUID: {}", uuid.bytes.iter().map(|x| format!("{:02x}", x)).collect::<String>());

    let mut device_ptr: CUdeviceptr = 0u64;
    let mut shared_handle = 0isize;

    let mut cu_mem_handle: CUmemGenericAllocationHandle = 0;
    let mut prop = CUmemAllocationProp {
        type_: CU_MEM_ALLOCATION_TYPE_PINNED,
        requestedHandleTypes: share_type,
        location,
        win32HandleMetaData: std::ptr::null_mut(),
        allocFlags: CUmemAllocationProp_st__bindgen_ty_1::default()
    };

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Security::{ *, Authorization::* };
        use windows::Win32::Foundation::*;
        use windows::Wdk::Foundation::*;
        use windows::core::*;
        std::thread_local! {
            static OBJ_ATTRIBUTES: Option<OBJECT_ATTRIBUTES> = unsafe {
                const SDDL: &'static str = "D:P(OA;;GARCSDWDWOCCDCLCSWLODTWPRPCRFA;;;WD)\0";

                let mut sec_desc = PSECURITY_DESCRIPTOR::default();
                let result = ConvertStringSecurityDescriptorToSecurityDescriptorA(PCSTR::from_raw(SDDL.as_ptr()), SDDL_REVISION_1, &mut sec_desc, None);
                if result.is_ok() {
                    Some(OBJECT_ATTRIBUTES {
                        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
                        RootDirectory: HANDLE::default(),
                        ObjectName: std::ptr::null_mut(),
                        Attributes: 0,
                        SecurityDescriptor: sec_desc.0,
                        SecurityQualityOfService: std::ptr::null_mut(),
                    })
                } else {
                    log::error!("IPC failure: ConvertStringSecurityDescriptorToSecurityDescriptor failed! {:?}", GetLastError());
                    None
                }
            };
        }
        OBJ_ATTRIBUTES.with(|x| {
            if let Some(x) = x {
                prop.win32HandleMetaData = x as *const _ as *mut _;
            }
        });
    }

    let mut granularity = 0usize;
    cuda!(cuda.cuMemGetAllocationGranularity(&mut granularity, &prop, CU_MEM_ALLOC_GRANULARITY_MINIMUM));

    let asize = align(size, granularity);

    cuda!(cuda.cuMemAddressReserve(&mut device_ptr, asize, granularity, 0, 0));
    cuda!(cuda.cuMemCreate(&mut cu_mem_handle, asize, &prop, 0));
    cuda!(cuda.cuMemExportToShareableHandle((&mut shared_handle) as *mut isize as *mut _, cu_mem_handle, share_type, 0));
    log::debug!("Creating CUDA memory 0x{device_ptr:08x} (device {}) | size: {size}, aligned size: {asize}", dev);

    cuda!(cuda.cuMemMap(device_ptr, asize, 0, cu_mem_handle, 0));
    cuda!(cuda.cuMemRelease(cu_mem_handle));

    cuda!(cuda.cuMemSetAccess(device_ptr, asize, &CUmemAccessDesc_st { location, flags: CU_MEM_ACCESS_FLAGS_PROT_READWRITE }, 1));

    Ok(CudaSharedMemory {
        device_ptr,
        shared_handle,
        external_memory: None,
        cuda_alloc_size: asize,
        vulkan_pitch_alignment: 1,
        additional_drop_func: None,
    })
}

pub fn create_vk_image_backed_by_cuda_memory(device: &wgpu::Device, size: (usize, usize, usize), format: wgpu::TextureFormat) -> Result<(vk::Image, CudaSharedMemory), Box<dyn std::error::Error>> {
    let mut cuda_mem = allocate_shared_cuda_memory(size.2 * size.1)?;

    let handle_type = if cfg!(target_os = "windows") {
        vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
    } else {
        vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD
    };

    unsafe {
        let raw_image = device.as_hal::<Vulkan, _, _>(|device| {
            device.map(|device| {
                let raw_device = device.raw_device();

                #[cfg(target_os = "windows")]
                let mut import_memory_info = vk::ImportMemoryWin32HandleInfoKHR::default()
                    .handle_type(handle_type)
                    .handle(cuda_mem.shared_handle);

                #[cfg(target_os = "linux")]
                let mut import_memory_info = vk::ImportMemoryFdInfoKHR::default()
                    .handle_type(handle_type)
                    .fd(cuda_mem.shared_handle as std::ffi::c_int);

                let mut ext_create_info = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);

                let image_create_info = ImageCreateInfo::default()
                    .push_next(&mut ext_create_info)
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(super::wgpu_interop_vulkan::format_wgpu_to_vulkan(format))
                    .extent(vk::Extent3D { width: size.0 as u32, height: size.1 as u32, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::LINEAR)
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

                #[cfg(target_os = "windows")]
                let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(cuda_mem.shared_handle as *mut _));
                #[cfg(target_os = "linux")]
                libc::close(cuda_mem.shared_handle as i32);

                Ok::<ash::vk::Image, vk::Result>(raw_image)
            })
        }).unwrap().unwrap(); // TODO: unwrap

        Ok((raw_image, cuda_mem))
    }
}

pub fn create_vk_buffer_backed_by_cuda_memory(device: &wgpu::Device, size: (usize, usize, usize)) -> Result<(vk::Buffer, CudaSharedMemory), Box<dyn std::error::Error>> {
    let buffer_size = size.2 * size.1;
    let mut cuda_mem = allocate_shared_cuda_memory(buffer_size)?;

    let handle_type = if cfg!(target_os = "windows") {
        vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
    } else {
        vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD
    };

    unsafe {
        let raw_buffer = device.as_hal::<Vulkan, _, _>(|device| {
            device.map(|device| {
                let raw_device = device.raw_device();

                #[cfg(target_os = "windows")]
                let mut import_memory_info = vk::ImportMemoryWin32HandleInfoKHR::default()
                    .handle_type(handle_type)
                    .handle(cuda_mem.shared_handle);

                #[cfg(target_os = "linux")]
                let mut import_memory_info = vk::ImportMemoryFdInfoKHR::default()
                    .handle_type(handle_type)
                    .fd(cuda_mem.shared_handle as std::ffi::c_int);

                let mut ext_create_info = vk::ExternalMemoryBufferCreateInfo::default().handle_types(handle_type);

                let buffer_create_info = BufferCreateInfo::default()
                    .push_next(&mut ext_create_info)
                    .size(cuda_mem.cuda_alloc_size as vk::DeviceSize)
                    .usage(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);

                let raw_buffer = raw_device.create_buffer(&buffer_create_info, None)?;

                let memory_type_index = {
                    let mem_requirements = raw_device.get_buffer_memory_requirements(raw_buffer);
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

                raw_device.bind_buffer_memory(raw_buffer, allocated_memory, 0)?;

                let raw_device = raw_device.clone();
                cuda_mem.additional_drop_func = Some(Box::new(move || {
                    raw_device.free_memory(allocated_memory, None);
                }));

                #[cfg(target_os = "windows")]
                let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(cuda_mem.shared_handle as *mut _));
                #[cfg(target_os = "linux")]
                libc::close(cuda_mem.shared_handle as i32);

                Ok::<ash::vk::Buffer, vk::Result>(raw_buffer)
            })
        }).unwrap().unwrap(); // TODO: unwrap

        Ok((raw_buffer, cuda_mem))
    }
}


////////////////////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////////////////////
// https://github.dev/Rust-GPU/Rust-CUDA/blob/master/crates/cust_raw/src/cuda.rs

use ::std::os::raw::*;

#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum cudaError_enum {
    CUDA_SUCCESS = 0,
    CUDA_ERROR_INVALID_VALUE = 1,
    CUDA_ERROR_OUT_OF_MEMORY = 2,
    CUDA_ERROR_NOT_INITIALIZED = 3,
    CUDA_ERROR_DEINITIALIZED = 4,
    CUDA_ERROR_PROFILER_DISABLED = 5,
    CUDA_ERROR_PROFILER_NOT_INITIALIZED = 6,
    CUDA_ERROR_PROFILER_ALREADY_STARTED = 7,
    CUDA_ERROR_PROFILER_ALREADY_STOPPED = 8,
    CUDA_ERROR_STUB_LIBRARY = 34,
    CUDA_ERROR_NO_DEVICE = 100,
    CUDA_ERROR_INVALID_DEVICE = 101,
    CUDA_ERROR_DEVICE_NOT_LICENSED = 102,
    CUDA_ERROR_INVALID_IMAGE = 200,
    CUDA_ERROR_INVALID_CONTEXT = 201,
    CUDA_ERROR_CONTEXT_ALREADY_CURRENT = 202,
    CUDA_ERROR_MAP_FAILED = 205,
    CUDA_ERROR_UNMAP_FAILED = 206,
    CUDA_ERROR_ARRAY_IS_MAPPED = 207,
    CUDA_ERROR_ALREADY_MAPPED = 208,
    CUDA_ERROR_NO_BINARY_FOR_GPU = 209,
    CUDA_ERROR_ALREADY_ACQUIRED = 210,
    CUDA_ERROR_NOT_MAPPED = 211,
    CUDA_ERROR_NOT_MAPPED_AS_ARRAY = 212,
    CUDA_ERROR_NOT_MAPPED_AS_POINTER = 213,
    CUDA_ERROR_ECC_UNCORRECTABLE = 214,
    CUDA_ERROR_UNSUPPORTED_LIMIT = 215,
    CUDA_ERROR_CONTEXT_ALREADY_IN_USE = 216,
    CUDA_ERROR_PEER_ACCESS_UNSUPPORTED = 217,
    CUDA_ERROR_INVALID_PTX = 218,
    CUDA_ERROR_INVALID_GRAPHICS_CONTEXT = 219,
    CUDA_ERROR_NVLINK_UNCORRECTABLE = 220,
    CUDA_ERROR_JIT_COMPILER_NOT_FOUND = 221,
    CUDA_ERROR_UNSUPPORTED_PTX_VERSION = 222,
    CUDA_ERROR_JIT_COMPILATION_DISABLED = 223,
    CUDA_ERROR_UNSUPPORTED_EXEC_AFFINITY = 224,
    CUDA_ERROR_INVALID_SOURCE = 300,
    CUDA_ERROR_FILE_NOT_FOUND = 301,
    CUDA_ERROR_SHARED_OBJECT_SYMBOL_NOT_FOUND = 302,
    CUDA_ERROR_SHARED_OBJECT_INIT_FAILED = 303,
    CUDA_ERROR_OPERATING_SYSTEM = 304,
    CUDA_ERROR_INVALID_HANDLE = 400,
    CUDA_ERROR_ILLEGAL_STATE = 401,
    CUDA_ERROR_NOT_FOUND = 500,
    CUDA_ERROR_NOT_READY = 600,
    CUDA_ERROR_ILLEGAL_ADDRESS = 700,
    CUDA_ERROR_LAUNCH_OUT_OF_RESOURCES = 701,
    CUDA_ERROR_LAUNCH_TIMEOUT = 702,
    CUDA_ERROR_LAUNCH_INCOMPATIBLE_TEXTURING = 703,
    CUDA_ERROR_PEER_ACCESS_ALREADY_ENABLED = 704,
    CUDA_ERROR_PEER_ACCESS_NOT_ENABLED = 705,
    CUDA_ERROR_PRIMARY_CONTEXT_ACTIVE = 708,
    CUDA_ERROR_CONTEXT_IS_DESTROYED = 709,
    CUDA_ERROR_ASSERT = 710,
    CUDA_ERROR_TOO_MANY_PEERS = 711,
    CUDA_ERROR_HOST_MEMORY_ALREADY_REGISTERED = 712,
    CUDA_ERROR_HOST_MEMORY_NOT_REGISTERED = 713,
    CUDA_ERROR_HARDWARE_STACK_ERROR = 714,
    CUDA_ERROR_ILLEGAL_INSTRUCTION = 715,
    CUDA_ERROR_MISALIGNED_ADDRESS = 716,
    CUDA_ERROR_INVALID_ADDRESS_SPACE = 717,
    CUDA_ERROR_INVALID_PC = 718,
    CUDA_ERROR_LAUNCH_FAILED = 719,
    CUDA_ERROR_COOPERATIVE_LAUNCH_TOO_LARGE = 720,
    CUDA_ERROR_NOT_PERMITTED = 800,
    CUDA_ERROR_NOT_SUPPORTED = 801,
    CUDA_ERROR_SYSTEM_NOT_READY = 802,
    CUDA_ERROR_SYSTEM_DRIVER_MISMATCH = 803,
    CUDA_ERROR_COMPAT_NOT_SUPPORTED_ON_DEVICE = 804,
    CUDA_ERROR_MPS_CONNECTION_FAILED = 805,
    CUDA_ERROR_MPS_RPC_FAILURE = 806,
    CUDA_ERROR_MPS_SERVER_NOT_READY = 807,
    CUDA_ERROR_MPS_MAX_CLIENTS_REACHED = 808,
    CUDA_ERROR_MPS_MAX_CONNECTIONS_REACHED = 809,
    CUDA_ERROR_STREAM_CAPTURE_UNSUPPORTED = 900,
    CUDA_ERROR_STREAM_CAPTURE_INVALIDATED = 901,
    CUDA_ERROR_STREAM_CAPTURE_MERGE = 902,
    CUDA_ERROR_STREAM_CAPTURE_UNMATCHED = 903,
    CUDA_ERROR_STREAM_CAPTURE_UNJOINED = 904,
    CUDA_ERROR_STREAM_CAPTURE_ISOLATION = 905,
    CUDA_ERROR_STREAM_CAPTURE_IMPLICIT = 906,
    CUDA_ERROR_CAPTURED_EVENT = 907,
    CUDA_ERROR_STREAM_CAPTURE_WRONG_THREAD = 908,
    CUDA_ERROR_TIMEOUT = 909,
    CUDA_ERROR_GRAPH_EXEC_UPDATE_FAILURE = 910,
    CUDA_ERROR_EXTERNAL_DEVICE = 911,
    CUDA_ERROR_UNKNOWN = 999,
}
pub use self::cudaError_enum as CUresult;
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUmemLocationType {
    CU_MEM_LOCATION_TYPE_INVALID = 0,
    CU_MEM_LOCATION_TYPE_DEVICE = 1,
    CU_MEM_LOCATION_TYPE_MAX = 2147483647,
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUmemAllocationType {
    CU_MEM_ALLOCATION_TYPE_INVALID = 0,
    CU_MEM_ALLOCATION_TYPE_PINNED = 1,
    CU_MEM_ALLOCATION_TYPE_MAX = 2147483647,
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUmemorytype {
    CU_MEMORYTYPE_HOST = 1,
    CU_MEMORYTYPE_DEVICE = 2,
    CU_MEMORYTYPE_ARRAY = 3,
    CU_MEMORYTYPE_UNIFIED = 4,
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CUarray_st {
    _unused: [u8; 0],
}
pub type CUarray = *mut CUarray_st;
#[repr(C)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUDA_MEMCPY2D_st {
    pub srcXInBytes: usize,
    pub srcY: usize,
    pub srcMemoryType: CUmemorytype,
    pub srcHost: *const c_void,
    pub srcDevice: CUdeviceptr,
    pub srcArray: CUarray,
    pub srcPitch: usize,
    pub dstXInBytes: usize,
    pub dstY: usize,
    pub dstMemoryType: CUmemorytype,
    pub dstHost: *mut c_void,
    pub dstDevice: CUdeviceptr,
    pub dstArray: CUarray,
    pub dstPitch: usize,
    pub WidthInBytes: usize,
    pub Height: usize,
}
#[repr(C)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUmemLocation {
    pub type_: CUmemLocationType,
    pub id: c_int,
}
#[repr(C)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUmemAllocationProp {
    pub type_: CUmemAllocationType,
    pub requestedHandleTypes: CUmemAllocationHandleType,
    pub location: CUmemLocation,
    pub win32HandleMetaData: *mut c_void,
    pub allocFlags: CUmemAllocationProp_st__bindgen_ty_1,
}
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUmemAllocationProp_st__bindgen_ty_1 {
    pub compressionType: c_uchar,
    pub gpuDirectRDMACapable: c_uchar,
    pub usage: c_ushort,
    pub reserved: [c_uchar; 4usize],
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUmemAccess_flags {
    CU_MEM_ACCESS_FLAGS_PROT_NONE = 0,
    CU_MEM_ACCESS_FLAGS_PROT_READ = 1,
    CU_MEM_ACCESS_FLAGS_PROT_READWRITE = 3,
    CU_MEM_ACCESS_FLAGS_PROT_MAX = 2147483647,
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUmemAllocationGranularity_flags {
    CU_MEM_ALLOC_GRANULARITY_MINIMUM = 0,
    CU_MEM_ALLOC_GRANULARITY_RECOMMENDED = 1,
}
#[repr(C)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUmemAccessDesc_st {
    pub location: CUmemLocation,
    pub flags: CUmemAccess_flags,
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUmemAllocationHandleType {
    CU_MEM_HANDLE_TYPE_NONE = 0,
    CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR = 1,
    CU_MEM_HANDLE_TYPE_WIN32 = 2,
    CU_MEM_HANDLE_TYPE_WIN32_KMT = 4,
    CU_MEM_HANDLE_TYPE_MAX = 2147483647,
}
pub type CUmemGenericAllocationHandle_v1 = c_ulonglong;
pub type CUmemGenericAllocationHandle = CUmemGenericAllocationHandle_v1;
pub type CUdeviceptr = c_ulonglong;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CUextMemory_st {
    _unused: [u8; 0],
}
pub type CUexternalMemory = *mut CUextMemory_st;
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum CUexternalMemoryHandleType_enum {
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD = 1,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32 = 2,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_KMT = 3,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_HEAP = 4,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_RESOURCE = 5,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_D3D11_RESOURCE = 6,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_D3D11_RESOURCE_KMT = 7,
    CU_EXTERNAL_MEMORY_HANDLE_TYPE_NVSCIBUF = 8,
}
pub use self::CUexternalMemoryHandleType_enum as CUexternalMemoryHandleType;
#[repr(C)]
#[derive(Copy, Clone)]
pub struct CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st {
    pub type_: CUexternalMemoryHandleType,
    pub handle: CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1,
    pub size: ::std::os::raw::c_ulonglong,
    pub flags: ::std::os::raw::c_uint,
    pub reserved: [::std::os::raw::c_uint; 16usize],
}
#[repr(C)]
#[derive(Copy, Clone)]
pub union CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 {
    pub fd: ::std::os::raw::c_int,
    pub win32: CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1__bindgen_ty_1,
    pub nvSciBufObject: *const ::std::os::raw::c_void,
}
#[repr(C)]
#[derive(Debug, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1__bindgen_ty_1 {
    pub handle: *mut ::std::os::raw::c_void,
    pub name: *const ::std::os::raw::c_void,
}
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUDA_EXTERNAL_MEMORY_BUFFER_DESC_st {
    pub offset: ::std::os::raw::c_ulonglong,
    pub size: ::std::os::raw::c_ulonglong,
    pub flags: ::std::os::raw::c_uint,
    pub reserved: [::std::os::raw::c_uint; 16usize],
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct CUuuid {
    pub bytes: [::core::ffi::c_char; 16usize],
}

#[cfg(target_os = "windows")]
use libloading::os::windows as dl;
#[cfg(target_os = "linux")]
use libloading::os::unix as dl;

pub struct CudaFunctions {
    _cudart: dl::Library,
    _nvcuda: dl::Library,
    pub cudaDeviceSynchronize: dl::Symbol<unsafe extern "C" fn() -> i32>,
    pub cudaFree:              dl::Symbol<unsafe extern "C" fn(ptr: *mut c_void) -> i32>,
    pub cudaGetDevice:         dl::Symbol<unsafe extern "C" fn(device: *mut c_int) -> i32>,

    pub cuMemExportToShareableHandle:    dl::Symbol<unsafe extern "C" fn(shareableHandle: *mut c_void, handle: CUmemGenericAllocationHandle, handleType: CUmemAllocationHandleType, flags: c_ulonglong) -> CUresult>,
    pub cuMemGetAllocationGranularity:   dl::Symbol<unsafe extern "C" fn(granularity: *mut usize, prop: *const CUmemAllocationProp, option: CUmemAllocationGranularity_flags) -> CUresult>,
    pub cuMemCreate:                     dl::Symbol<unsafe extern "C" fn(handle: *mut CUmemGenericAllocationHandle, size: usize, prop: *const CUmemAllocationProp, flags: c_ulonglong) -> CUresult>,
    pub cuMemFree:                       dl::Symbol<unsafe extern "C" fn(ptr: CUdeviceptr) -> CUresult>,
    pub cuMemAddressReserve:             dl::Symbol<unsafe extern "C" fn(ptr: *mut CUdeviceptr, size: usize, alignment: usize, addr: CUdeviceptr, flags: c_ulonglong) -> CUresult>,
    pub cuMemMap:                        dl::Symbol<unsafe extern "C" fn(ptr: CUdeviceptr, size: usize, offset: usize, handle: CUmemGenericAllocationHandle, flags: c_ulonglong) -> CUresult>,
    pub cuMemSetAccess:                  dl::Symbol<unsafe extern "C" fn(ptr: CUdeviceptr, size: usize, desc: *const CUmemAccessDesc_st, count: usize) -> CUresult>,
    pub cuMemUnmap:                      dl::Symbol<unsafe extern "C" fn(ptr: CUdeviceptr, size: usize) -> CUresult>,
    pub cuMemRelease:                    dl::Symbol<unsafe extern "C" fn(handle: CUmemGenericAllocationHandle) -> CUresult>,
    pub cuMemcpy:                        dl::Symbol<unsafe extern "C" fn(dst: CUdeviceptr, src: CUdeviceptr, ByteCount: usize) -> CUresult>,
    pub cuMemcpy2D_v2:                   dl::Symbol<unsafe extern "C" fn(pCopy: *const CUDA_MEMCPY2D_st) -> CUresult>,
    pub cuMemAddressFree:                dl::Symbol<unsafe extern "C" fn(ptr: CUdeviceptr, size: usize) -> CUresult>,
    pub cuCtxSynchronize:                dl::Symbol<unsafe extern "C" fn() -> CUresult>,
    pub cuMemGetAddressRange_v2:         dl::Symbol<unsafe extern "C" fn(pbase: *mut CUdeviceptr, psize: *mut usize, dptr: CUdeviceptr) -> CUresult>,

    pub cuDeviceGetUuid:                 dl::Symbol<unsafe extern "C" fn(uuid: *mut CUuuid, dev: c_int) -> CUresult>,

    pub cuImportExternalMemory:          dl::Symbol<unsafe extern "C" fn(extMem_out: *mut CUexternalMemory, memHandleDesc: *const CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st) -> CUresult>,
    pub cuExternalMemoryGetMappedBuffer: dl::Symbol<unsafe extern "C" fn(devPtr: *mut CUdeviceptr, extMem: CUexternalMemory, bufferDesc: *const CUDA_EXTERNAL_MEMORY_BUFFER_DESC_st) -> CUresult>,
    pub cuDestroyExternalMemory:         dl::Symbol<unsafe extern "C" fn(extMem: CUexternalMemory) -> CUresult>,
}

impl CudaFunctions {
    pub unsafe fn new() -> Result<Self, libloading::Error> {
        let candidates = if cfg!(target_os = "windows") {
            vec![
                "cudart64_121.dll",
                "cudart64_120.dll",
                "cudart64_12.dll",
                "cudart64_110.dll",
                "cudart64_101.dll",
                "cudart64_91.dll",
                "cudart64_90.dll",
                "cudart64_80.dll",
                "cudart64_75.dll",
                "cudart64_65.dll",
            ]
        } else {
            vec![
                "libcudart.so",
                "/usr/local/cuda/lib64/libcudart.so",
                // "/usr/local/cuda-10.0/targets/amd64-linux/lib/libcudart.so",
            ]
        };
        let mut cudart = None;
        for filename in candidates {
            if let Ok(l) = dl::Library::new(filename) {
                cudart = Some(l);
                log::debug!("Loaded {}", &filename);
                break;
            }
        }
        if cudart.is_none() { return Err(libloading::Error::DlOpenUnknown); }
        let cudart = cudart.unwrap();

        let nvcuda = dl::Library::new(if cfg!(target_os = "windows") { "nvcuda.dll" } else { "libcuda.so.1" })?;

        Ok(Self {
            cudaDeviceSynchronize:           cudart.get(b"cudaDeviceSynchronize")?,
            cudaGetDevice:                   cudart.get(b"cudaGetDevice")?,
            cudaFree:                        cudart.get(b"cudaFree")?,

            cuMemExportToShareableHandle:    nvcuda.get(b"cuMemExportToShareableHandle")?,
            cuMemGetAllocationGranularity:   nvcuda.get(b"cuMemGetAllocationGranularity")?,
            cuMemCreate:                     nvcuda.get(b"cuMemCreate")?,
            cuMemFree:                       nvcuda.get(b"cuMemFree")?,
            cuMemAddressReserve:             nvcuda.get(b"cuMemAddressReserve")?,
            cuMemMap:                        nvcuda.get(b"cuMemMap")?,
            cuMemSetAccess:                  nvcuda.get(b"cuMemSetAccess")?,
            cuMemUnmap:                      nvcuda.get(b"cuMemUnmap")?,
            cuMemRelease:                    nvcuda.get(b"cuMemRelease")?,
            cuMemcpy:                        nvcuda.get(b"cuMemcpy")?,
            cuMemcpy2D_v2:                   nvcuda.get(b"cuMemcpy2D_v2")?,
            cuMemAddressFree:                nvcuda.get(b"cuMemAddressFree")?,
            cuCtxSynchronize:                nvcuda.get(b"cuCtxSynchronize")?,
            cuMemGetAddressRange_v2:         nvcuda.get(b"cuMemGetAddressRange_v2")?,

            cuDeviceGetUuid:                 nvcuda.get(b"cuDeviceGetUuid_v2").or_else(|_| nvcuda.get(b"cuDeviceGetUuid"))?,

            cuImportExternalMemory:          nvcuda.get(b"cuImportExternalMemory")?,
            cuExternalMemoryGetMappedBuffer: nvcuda.get(b"cuExternalMemoryGetMappedBuffer")?,
            cuDestroyExternalMemory:         nvcuda.get(b"cuDestroyExternalMemory")?,

            _cudart: cudart,
            _nvcuda: nvcuda,
        })
    }
}

lazy_static::lazy_static! {
    pub static ref CUDA: Result<CudaFunctions, libloading::Error> = unsafe { CudaFunctions::new() };
}
