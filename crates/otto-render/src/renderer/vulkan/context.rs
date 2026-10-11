//! The Skia context on the renderer's Vulkan device.
//!
//! skia-safe's `BackendContext` cannot carry the device's enabled features,
//! so Skia would always assume none; without `samplerYcbcrConversion` it
//! refuses to sample YUV images. The context is therefore built through the
//! raw bindings, with `fDeviceFeatures2` filled in.

// Rust guideline compliant 2026-02-21

use std::{
    ffi::{c_char, c_void, CStr, CString},
    ptr,
    sync::OnceLock,
};

use layers::{sb, skia::gpu::DirectContext};
use smithay::{
    backend::vulkan::{device::Device, version::Version, PhysicalDevice},
    reexports::ash::{self, vk, vk::Handle},
};

use super::SkiaVkError;

/// Byte offset of `fDeviceFeatures2` in `skgpu::VulkanBackendContext`.
///
/// From `include/gpu/vk/VulkanBackendContext.h` of the Skia skia-bindings
/// ships: four handles, `fGraphicsQueueIndex` and `fMaxAPIVersion` (two
/// `uint32_t`), then the `fVkExtensions`, `fDeviceFeatures` and
/// `fDeviceFeatures2` pointers.
const DEVICE_FEATURES2_OFFSET: usize = 4 * 8 + 2 * 4 + 2 * 8;

// The bindings only know the struct's size; a different size means a Skia
// whose layout this offset was not taken from.
const _: () = assert!(std::mem::size_of::<sb::skgpu_VulkanBackendContext>() == 128);

/// The loader's entry points Skia resolves everything else through.
struct Loader {
    get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
}

static LOADER: OnceLock<Loader> = OnceLock::new();

/// Resolves a Vulkan entry point for Skia.
unsafe extern "C" fn get_proc(
    name: *const c_char,
    instance: sb::VkInstance,
    device: sb::VkDevice,
) -> *const c_void {
    let Some(loader) = LOADER.get() else {
        return ptr::null();
    };
    // SAFETY: Skia hands valid instance and device handles (or null) and a
    // NUL-terminated name.
    let f = unsafe {
        if device.is_null() {
            (loader.get_instance_proc_addr)(vk::Instance::from_raw(instance as u64), name)
        } else {
            (loader.get_device_proc_addr)(vk::Device::from_raw(device as u64), name)
        }
    };
    f.map_or(ptr::null(), |f| f as *const c_void)
}

/// Creates the Skia context on `device`'s queue.
///
/// `features` is the chain the device was created with; Skia reads it
/// while the context is made and keeps what it needs.
///
/// # Errors
///
/// Fails when the Vulkan loader cannot be loaded or Skia refuses the device.
pub(super) fn create(
    phd: &PhysicalDevice,
    device: &Device,
    extensions: &[&CStr],
    features: &vk::PhysicalDeviceFeatures2<'_>,
) -> Result<DirectContext, SkiaVkError> {
    if LOADER.get().is_none() {
        // SAFETY: loading the system Vulkan loader has no preconditions; it
        // is the library Smithay's instance already loaded.
        let entry =
            unsafe { ash::Entry::load() }.map_err(|e| SkiaVkError::Loader(e.to_string()))?;
        let _ = LOADER.set(Loader {
            get_instance_proc_addr: entry.static_fn().get_instance_proc_addr,
            get_device_proc_addr: phd.instance().handle().fp_v1_0().get_device_proc_addr,
        });
    }

    let names: Vec<CString> = extensions.iter().map(|ext| CString::from(*ext)).collect();
    let pointers: Vec<*const c_char> = names.iter().map(|name| name.as_ptr()).collect();
    // Skia asks for core entry points of the version it is told; the
    // instance is capped at 1.3, and entry points of a newer device version
    // are not handed out under it.
    let api_version = phd.api_version().min(Version::VERSION_1_3).to_raw();

    // SAFETY: the handles are live and outlive the backend context, which is
    // deleted right after the Skia context is made; the extension strings
    // are copied by Skia. The features pointer is written at the offset of
    // `fDeviceFeatures2` (checked against the struct size above) and only
    // read during `MakeVulkan`, while `features` is borrowed.
    unsafe {
        let backend = sb::C_VulkanBackendContext_new(
            phd.instance().handle().handle().as_raw() as _,
            phd.handle().as_raw() as _,
            device.vk().handle().as_raw() as _,
            device.queue().as_raw() as _,
            device.queue_family_idx(),
            Some(get_proc),
            ptr::null(),
            0,
            pointers.as_ptr(),
            pointers.len(),
        );
        if backend.is_null() {
            return Err(SkiaVkError::ContextCreation);
        }
        let native = backend.cast::<sb::skgpu_VulkanBackendContext>();
        sb::C_VulkanBackendContext_setMaxAPIVersion(native, api_version);
        backend
            .cast::<u8>()
            .add(DEVICE_FEATURES2_OFFSET)
            .cast::<*const vk::PhysicalDeviceFeatures2<'_>>()
            .write(features);
        let context = sb::C_GrDirectContexts_MakeVulkan(native, ptr::null());
        sb::C_VulkanBackendContext_delete(backend);
        // `DirectContext` is a `#[repr(transparent)]` handle over the
        // reference-counted pointer `MakeVulkan` returns an owned ref of.
        ptr::NonNull::new(context)
            .map(|context| std::mem::transmute::<ptr::NonNull<_>, DirectContext>(context))
            .ok_or(SkiaVkError::ContextCreation)
    }
}

/// Whether `phd` can sample YUV images through a sampler YCbCr conversion.
pub(super) fn supports_ycbcr(phd: &PhysicalDevice) -> bool {
    let mut vk11 = vk::PhysicalDeviceVulkan11Features::default();
    let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut vk11);
    // SAFETY: the physical device belongs to this instance, which is 1.1+.
    unsafe {
        phd.instance()
            .handle()
            .get_physical_device_features2(phd.handle(), &mut features);
    }
    vk11.sampler_ycbcr_conversion == vk::TRUE
}
