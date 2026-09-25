//! Sync files around Skia's queue submissions.
//!
//! skia-safe exposes no semaphores, so the renderer fences Skia's work from
//! the outside: Vulkan orders semaphore signal operations after every
//! command submitted earlier on the same queue, so an empty submit that
//! signals a `SYNC_FD`-exportable semaphore right after Skia's submit yields
//! a sync file that signals once the frame has rendered. Waits go the other
//! way: a client's sync file is imported as a temporary semaphore payload and
//! waited on by an empty submit ahead of Skia's.
//!
//! Semaphores and fences are pooled. A semaphore may only be destroyed or
//! reused once the batch that references it has executed, which the fence
//! submitted with that batch tells.

// Rust guideline compliant 2026-02-21

use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd, OwnedFd};

use smithay::{
    backend::{
        renderer::sync::{Fence, Interrupted},
        vulkan::device::Device,
    },
    reexports::ash::vk,
};

use super::SkiaVkError;

/// A frame's completion, as a sync file.
///
/// The file becomes readable once the GPU work submitted before it has
/// finished. It goes to KMS as the plane's `IN_FENCE_FD`, and to anything
/// else that takes a Smithay `SyncPoint`.
#[derive(Debug)]
pub struct SkiaVkSync(OwnedFd);

impl SkiaVkSync {
    /// Polls the sync file, blocking for at most `timeout_ms` (-1 blocks forever).
    fn poll(&self, timeout_ms: i32) -> std::io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            // SAFETY: `pfd` is a valid pollfd for the duration of the call.
            let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
            if ret >= 0 {
                return Ok(ret == 1);
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }

    /// Borrows the sync file.
    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Fence for SkiaVkSync {
    fn is_signaled(&self) -> bool {
        self.poll(0).unwrap_or(true)
    }

    fn wait(&self) -> Result<(), Interrupted> {
        self.poll(-1).map(|_| ()).map_err(|err| {
            tracing::warn!(?err, "waiting on a Vulkan sync file failed");
            Interrupted
        })
    }

    fn is_exportable(&self) -> bool {
        true
    }

    fn export(&self) -> Option<OwnedFd> {
        self.0.try_clone().ok()
    }
}

/// Which `SYNC_FD` semaphore operations the device supports.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SyncFdSupport {
    /// Semaphores can be exported as sync files.
    pub export: bool,
    /// Sync files can be imported into semaphores.
    pub import: bool,
}

/// A submitted semaphore and fence, and the frame serial the submit signals.
#[derive(Debug)]
struct Pending {
    semaphore: vk::Semaphore,
    fence: vk::Fence,
    serial: Option<u64>,
}

/// Pool of binary semaphores and fences for the empty submits.
///
/// Also counts frames: every signal is a serial, and a serial is complete
/// once its fence has signaled. Fences signal in submission order.
#[derive(Debug, Default)]
pub(crate) struct SyncPool {
    free: Vec<(vk::Semaphore, vk::Fence)>,
    pending: Vec<Pending>,
    submitted: u64,
    completed: u64,
}

impl SyncPool {
    /// Serial of the last frame signal submitted.
    pub fn submitted(&self) -> u64 {
        self.submitted
    }

    /// Serial of the last frame signal the GPU has reached.
    pub fn completed(&self) -> u64 {
        self.completed
    }

    /// Moves pairs whose batch has executed back to the free list.
    pub fn reclaim(&mut self, device: &Device) {
        let vk = device.vk();
        let mut i = 0;
        while i < self.pending.len() {
            let fence = self.pending[i].fence;
            // SAFETY: the fence belongs to `device`.
            if unsafe { vk.get_fence_status(fence) } == Ok(true) {
                // SAFETY: the fence is signaled, so no queue operation uses it.
                if unsafe { vk.reset_fences(&[fence]) }.is_ok() {
                    let done = self.pending.swap_remove(i);
                    if let Some(serial) = done.serial {
                        self.completed = self.completed.max(serial);
                    }
                    self.free.push((done.semaphore, done.fence));
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Hands out an unsignaled semaphore and fence, creating them if the pool is empty.
    fn acquire(
        &mut self,
        device: &Device,
        exportable: bool,
    ) -> Result<(vk::Semaphore, vk::Fence), SkiaVkError> {
        self.reclaim(device);
        if let Some(pair) = self.free.pop() {
            return Ok(pair);
        }
        let vk = device.vk();
        let mut export_info = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let mut sem_info = vk::SemaphoreCreateInfo::default();
        if exportable {
            sem_info = sem_info.push_next(&mut export_info);
        }
        // SAFETY: valid create info on a live device.
        let semaphore = unsafe { vk.create_semaphore(&sem_info, None) }?;
        // SAFETY: as above.
        let fence = match unsafe { vk.create_fence(&vk::FenceCreateInfo::default(), None) } {
            Ok(fence) => fence,
            Err(err) => {
                // SAFETY: the semaphore was never submitted.
                unsafe { vk.destroy_semaphore(semaphore, None) };
                return Err(err.into());
            }
        };
        Ok((semaphore, fence))
    }

    /// Submits an empty batch that signals once all earlier work on the
    /// queue is done, and returns that point as a sync file.
    ///
    /// Without `SYNC_FD` export, blocks until the queue is idle and returns
    /// `None`: everything submitted so far has then finished.
    ///
    /// # Errors
    ///
    /// Returns an error when the submit itself fails.
    pub fn signal(
        &mut self,
        device: &Device,
        support: SyncFdSupport,
    ) -> Result<Option<SkiaVkSync>, SkiaVkError> {
        let vk = device.vk();
        self.submitted += 1;
        let serial = self.submitted;
        if !support.export {
            // SAFETY: the queue belongs to `device` and is only used from this thread.
            unsafe { vk.queue_wait_idle(*device.queue()) }?;
            self.completed = serial;
            return Ok(None);
        }
        let (semaphore, fence) = self.acquire(device, true)?;
        let signal = [semaphore];
        let submit = vk::SubmitInfo::default().signal_semaphores(&signal);
        // SAFETY: the semaphore is unsignaled with no pending operation, the
        // fence is unsignaled, and the queue is only used from this thread.
        if let Err(err) = unsafe { vk.queue_submit(*device.queue(), &[submit], fence) } {
            self.free.push((semaphore, fence));
            return Err(err.into());
        }
        let fd_info = vk::SemaphoreGetFdInfoKHR::default()
            .semaphore(semaphore)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let exported = device
            .vk_khr_external_semaphore_fd()
            .ok_or(SkiaVkError::MissingExtension(
                "VK_KHR_external_semaphore_fd",
            ))
            // SAFETY: the semaphore has a pending signal operation, which a
            // SYNC_FD export requires.
            .and_then(|ext| unsafe { ext.get_semaphore_fd(&fd_info) }.map_err(Into::into));
        match exported {
            Ok(fd) => {
                self.pending.push(Pending {
                    semaphore,
                    fence,
                    serial: Some(serial),
                });
                // SAFETY: the export hands over ownership of a new fd.
                Ok(Some(SkiaVkSync(unsafe { OwnedFd::from_raw_fd(fd) })))
            }
            Err(err) => {
                tracing::warn!(?err, "exporting a sync file failed, waiting on the CPU");
                // SAFETY: the fence was submitted above; once it signals the
                // batch is done and the semaphore, still signaled, can go.
                unsafe {
                    let _ = vk.wait_for_fences(&[fence], true, u64::MAX);
                    vk.destroy_semaphore(semaphore, None);
                    vk.destroy_fence(fence, None);
                }
                self.completed = serial;
                Ok(None)
            }
        }
    }

    /// Makes later GPU work on the queue wait for `sync_file`.
    ///
    /// The file is imported as a temporary semaphore payload and waited on
    /// by an empty submit. Takes ownership of the fd.
    ///
    /// # Errors
    ///
    /// Returns an error when the import or the submit fails; the caller then
    /// waits on the CPU.
    pub fn wait(&mut self, device: &Device, sync_file: OwnedFd) -> Result<(), SkiaVkError> {
        let ext = device
            .vk_khr_external_semaphore_fd()
            .ok_or(SkiaVkError::MissingExtension(
                "VK_KHR_external_semaphore_fd",
            ))?;
        let (semaphore, fence) = self.acquire(device, true)?;
        let raw = sync_file.into_raw_fd();
        let import = vk::ImportSemaphoreFdInfoKHR::default()
            .semaphore(semaphore)
            .flags(vk::SemaphoreImportFlags::TEMPORARY)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
            .fd(raw);
        // SAFETY: the semaphore has no pending operation; on success Vulkan
        // owns `raw`.
        if let Err(err) = unsafe { ext.import_semaphore_fd(&import) } {
            // SAFETY: the import failed, so the fd is still ours to close.
            drop(unsafe { OwnedFd::from_raw_fd(raw) });
            self.free.push((semaphore, fence));
            return Err(err.into());
        }
        let wait = [semaphore];
        let stages = [vk::PipelineStageFlags::ALL_COMMANDS];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&wait)
            .wait_dst_stage_mask(&stages);
        // SAFETY: the semaphore carries the imported payload, the fence is
        // unsignaled, and the queue is only used from this thread.
        match unsafe { device.vk().queue_submit(*device.queue(), &[submit], fence) } {
            Ok(()) => {
                self.pending.push(Pending {
                    semaphore,
                    fence,
                    serial: None,
                });
                Ok(())
            }
            Err(err) => {
                // The temporary payload was never consumed; the semaphore may
                // still hold it, so it is not reused.
                // SAFETY: nothing was submitted with either object.
                unsafe {
                    device.vk().destroy_semaphore(semaphore, None);
                    device.vk().destroy_fence(fence, None);
                }
                Err(err.into())
            }
        }
    }

    /// Destroys every semaphore and fence.
    ///
    /// The caller has waited for the device to go idle.
    pub fn destroy(&mut self, device: &Device) {
        let vk = device.vk();
        let pending = self.pending.drain(..).map(|p| (p.semaphore, p.fence));
        for (semaphore, fence) in self.free.drain(..).chain(pending) {
            // SAFETY: the device is idle, so no batch references them.
            unsafe {
                vk.destroy_semaphore(semaphore, None);
                vk.destroy_fence(fence, None);
            }
        }
    }
}

/// Queries which `SYNC_FD` semaphore operations the physical device supports.
pub(crate) fn sync_fd_support(phd: &smithay::backend::vulkan::PhysicalDevice) -> SyncFdSupport {
    let info = vk::PhysicalDeviceExternalSemaphoreInfo::default()
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let mut props = vk::ExternalSemaphoreProperties::default();
    // SAFETY: the physical device belongs to this instance.
    unsafe {
        phd.instance()
            .handle()
            .get_physical_device_external_semaphore_properties(phd.handle(), &info, &mut props);
    }
    let features = props.external_semaphore_features;
    SyncFdSupport {
        export: features.contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE),
        import: features.contains(vk::ExternalSemaphoreFeatureFlags::IMPORTABLE),
    }
}

/// Attaches `sync_file` to `dmabuf` as a write fence.
///
/// Consumers that rely on implicit sync (a screencast reader, another GPU)
/// then wait for the GPU work the sync file stands for before reading.
///
/// # Errors
///
/// Returns the ioctl's error; kernels before 6.0 lack it.
pub(crate) fn attach_write_fence(
    dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    sync_file: std::os::fd::BorrowedFd<'_>,
) -> std::io::Result<()> {
    /// `struct dma_buf_import_sync_file` from `linux/dma-buf.h`.
    #[repr(C)]
    struct ImportSyncFile {
        flags: u32,
        fd: i32,
    }
    const DMA_BUF_SYNC_WRITE: u32 = 2;
    // _IOW('b', 3, struct dma_buf_import_sync_file)
    const DMA_BUF_IOCTL_IMPORT_SYNC_FILE: libc::c_ulong = 0x4008_6203;

    for handle in dmabuf.handles() {
        let mut arg = ImportSyncFile {
            flags: DMA_BUF_SYNC_WRITE,
            fd: sync_file.as_raw_fd(),
        };
        // SAFETY: `arg` matches the kernel's struct layout and outlives the call.
        let ret = unsafe {
            libc::ioctl(
                handle.as_raw_fd(),
                DMA_BUF_IOCTL_IMPORT_SYNC_FILE,
                &mut arg as *mut ImportSyncFile,
            )
        };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
