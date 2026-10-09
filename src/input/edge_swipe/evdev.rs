//! Touchpad slot state read straight from the kernel.
//!
//! libinput opens every evdev node through the session (logind / libseat
//! owns them, Otto cannot open them itself), so [`TrackingInterface`] wraps
//! that session interface and keeps a `dup()` of each descriptor it hands
//! out in a shared [`EvdevFds`] registry. The duplicate shares its open file
//! description with libinput, which is why it is never `read()`: that would
//! steal libinput's events. Only query ioctls are used on it, which report
//! the kernel's current device state without touching the event queue:
//!
//! - `EVIOCGABS` for the range and resolution of the multitouch axes.
//! - `EVIOCGMTSLOTS` for each slot's tracking id and x position.
//!
//! When the session is paused the descriptors are revoked, the ioctls fail,
//! and sampling reports nothing until libinput reopens the device.

// Rust guideline compliant 2026-02-21

use std::{
    collections::HashMap,
    os::fd::{AsRawFd, OwnedFd, RawFd},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use smithay::reexports::input::LibinputInterface;

use super::{EdgeZone, Touch};

/// `ABS_MT_SLOT` from `linux/input-event-codes.h`.
const ABS_MT_SLOT: u32 = 0x2f;
/// `ABS_MT_POSITION_X` from `linux/input-event-codes.h`.
const ABS_MT_POSITION_X: u32 = 0x35;
/// `ABS_MT_TRACKING_ID` from `linux/input-event-codes.h`.
const ABS_MT_TRACKING_ID: u32 = 0x39;

/// Most slots read per sample. Touchpads report 2 to 10; the rest are
/// ignored.
const MAX_SLOTS: usize = 16;

/// `_IOC_READ` from `asm-generic/ioctl.h`.
const IOC_READ: u32 = 2;

/// Builds an `_IOC(_IOC_READ, 'E', nr, size)` request number.
const fn evioc_read(nr: u32, size: usize) -> u32 {
    // Layout from asm-generic/ioctl.h: dir:2 | size:14 | type:8 | nr:8.
    #[expect(clippy::cast_possible_truncation, reason = "ioctl sizes fit 14 bits")]
    let size = size as u32;
    (IOC_READ << 30) | (size << 16) | ((b'E' as u32) << 8) | nr
}

/// `struct input_absinfo` from `linux/input.h`.
#[repr(C)]
#[derive(Default)]
struct InputAbsinfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// `EVIOCGMTSLOTS` payload: the axis code, then one value per slot.
#[repr(C)]
struct MtSlots {
    code: u32,
    values: [i32; MAX_SLOTS],
}

/// Reads the `EVIOCGABS` info of `axis`.
fn abs_info(fd: RawFd, axis: u32) -> Option<InputAbsinfo> {
    let mut info = InputAbsinfo::default();
    let request = evioc_read(0x40 + axis, size_of::<InputAbsinfo>());
    // SAFETY: EVIOCGABS writes exactly one `input_absinfo` (whose size is
    // encoded in the request) into the pointed-to buffer, which is a live,
    // properly aligned `#[repr(C)]` mirror of that struct. The ioctl only
    // queries state and never reads from the event queue.
    let ret = unsafe { libc::ioctl(fd, request as _, &raw mut info) };
    (ret >= 0).then_some(info)
}

/// Reads the `EVIOCGMTSLOTS` values of `code` into `out`.
fn mt_slots(fd: RawFd, code: u32, out: &mut MtSlots) -> bool {
    out.code = code;
    let request = evioc_read(0x0a, size_of::<MtSlots>());
    // SAFETY: EVIOCGMTSLOTS fills at most `size` bytes (encoded in the
    // request) after the leading code, and `out` is a live `#[repr(C)]`
    // buffer of exactly that size. It only queries state.
    let ret = unsafe { libc::ioctl(fd, request as _, std::ptr::from_mut(out)) };
    ret >= 0
}

/// One evdev node libinput has open.
#[derive(Debug)]
struct OpenDevice {
    /// The descriptor number libinput received, to match `close_restricted`.
    libinput_fd: RawFd,
    /// Our duplicate. Only ever used for query ioctls.
    dup: OwnedFd,
    /// Multitouch x axis and slot count, read on first use.
    axes: Option<(EdgeZone, usize)>,
}

/// Descriptors of the evdev nodes libinput has open, by device node path.
///
/// Cloning shares the registry.
#[derive(Clone, Debug, Default)]
pub struct EvdevFds {
    devices: Arc<Mutex<HashMap<PathBuf, OpenDevice>>>,
}

impl EvdevFds {
    fn with<R>(&self, f: impl FnOnce(&mut HashMap<PathBuf, OpenDevice>) -> R) -> R {
        let mut devices = self.devices.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut devices)
    }

    fn insert(&self, path: &Path, libinput_fd: &OwnedFd) {
        // Only evdev nodes carry slot state.
        if !path.starts_with("/dev/input") {
            return;
        }
        match libinput_fd.try_clone() {
            Ok(dup) => self.with(|devices| {
                devices.insert(
                    path.to_path_buf(),
                    OpenDevice {
                        libinput_fd: libinput_fd.as_raw_fd(),
                        dup,
                        axes: None,
                    },
                );
            }),
            Err(err) => tracing::debug!(?path, %err, "edge swipe: cannot dup evdev fd"),
        }
    }

    fn remove(&self, libinput_fd: RawFd) {
        self.with(|devices| devices.retain(|_, d| d.libinput_fd != libinput_fd));
    }

    /// Reads the touches on the pad at `path` into `touches`.
    ///
    /// Returns the pad's x axis, or `None` when the node is not open, has no
    /// multitouch axes, or the kernel refused the query (session paused).
    pub fn sample(&self, path: &Path, touches: &mut Vec<Touch>) -> Option<EdgeZone> {
        touches.clear();
        self.with(|devices| {
            let device = devices.get_mut(path)?;
            let fd = device.dup.as_raw_fd();
            let (zone, slots) = match device.axes {
                Some(axes) => axes,
                None => {
                    let x = abs_info(fd, ABS_MT_POSITION_X)?;
                    let slot = abs_info(fd, ABS_MT_SLOT)?;
                    let zone = EdgeZone {
                        min: x.minimum,
                        max: x.maximum,
                        resolution: x.resolution,
                    };
                    let slots = usize::try_from(slot.maximum.saturating_add(1))
                        .unwrap_or(0)
                        .min(MAX_SLOTS);
                    if zone.max <= zone.min || slots == 0 {
                        return None;
                    }
                    device.axes = Some((zone, slots));
                    (zone, slots)
                }
            };

            let mut ids = MtSlots {
                code: 0,
                values: [0; MAX_SLOTS],
            };
            let mut xs = MtSlots {
                code: 0,
                values: [0; MAX_SLOTS],
            };
            if !mt_slots(fd, ABS_MT_TRACKING_ID, &mut ids)
                || !mt_slots(fd, ABS_MT_POSITION_X, &mut xs)
            {
                return None;
            }
            touches.extend(
                ids.values[..slots]
                    .iter()
                    .zip(&xs.values[..slots])
                    .filter(|(id, _)| **id >= 0)
                    .map(|(&tracking_id, &x)| Touch { tracking_id, x }),
            );
            Some(zone)
        })
    }
}

/// A libinput interface that records the evdev descriptors it opens.
///
/// Opening and closing go through `inner`; every `/dev/input` node opened
/// also gets a duplicate in the shared [`EvdevFds`].
#[derive(Debug)]
pub struct TrackingInterface<I> {
    inner: I,
    fds: EvdevFds,
}

impl<I> TrackingInterface<I> {
    /// Wraps `inner`, recording its descriptors in `fds`.
    pub fn new(inner: I, fds: EvdevFds) -> Self {
        Self { inner, fds }
    }
}

impl<I: LibinputInterface> LibinputInterface for TrackingInterface<I> {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let fd = self.inner.open_restricted(path, flags)?;
        self.fds.insert(path, &fd);
        Ok(fd)
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        self.fds.remove(fd.as_raw_fd());
        self.inner.close_restricted(fd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        // EVIOCGABS(ABS_MT_POSITION_X) and EVIOCGMTSLOTS(sizeof) as the
        // kernel macros expand them.
        assert_eq!(evioc_read(0x40 + ABS_MT_POSITION_X, 24), 0x8018_4575);
        assert_eq!(evioc_read(0x0a, 4 + 4 * MAX_SLOTS), 0x8044_450a);
        assert_eq!(size_of::<InputAbsinfo>(), 24);
        assert_eq!(size_of::<MtSlots>(), 4 + 4 * MAX_SLOTS);
    }
}
