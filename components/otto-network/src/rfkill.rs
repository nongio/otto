//! The kernel's radio kill switches (rfkill) for Wi-Fi.
//!
//! A Wi-Fi radio can be off for three reasons, and NetworkManager tells
//! them apart only partly:
//!
//! - NetworkManager's own switch (`WirelessEnabled`).
//! - A **soft block**: the airplane-mode key, `rfkill block wifi`, another
//!   desktop's switch. Software can lift it.
//! - A **hard block**: a physical switch or a firmware setting. Only the
//!   person can lift it, by flipping the switch.
//!
//! Each radio's state is read from `/sys/class/rfkill`, which anyone may
//! read. A soft block is lifted by writing an event to `/dev/rfkill`, which
//! systemd's `uaccess` rule lets the active session write
//! (`70-uaccess.rules`). See the kernel's `Documentation/driver-api/rfkill.rst`.
//!
//! [`Radios`] names where those are, so a test can point them elsewhere.

// Rust guideline compliant 2026-02-21

use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the kernel lists the radios.
const SYSFS: &str = "/sys/class/rfkill";
/// The control device.
const DEVICE: &str = "/dev/rfkill";

/// `RFKILL_TYPE_ALL`, in an event about every radio.
const TYPE_ALL: u8 = 0;
/// `RFKILL_TYPE_WLAN`.
const TYPE_WLAN: u8 = 1;
/// `RFKILL_OP_CHANGE_ALL`: set every radio of a type.
const OP_CHANGE_ALL: u8 = 3;

/// How the Wi-Fi radios are blocked, all of them together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Block {
    /// A radio is soft-blocked: software can turn it back on.
    pub soft: bool,
    /// A radio is hard-blocked: only its switch can.
    pub hard: bool,
}

impl Block {
    /// Whether the radio is blocked at all.
    pub fn any(self) -> bool {
        self.soft || self.hard
    }
}

/// Where the Wi-Fi kill switches are read, followed and lifted.
///
/// [`Radios::system`] is the machine's own (`/sys/class/rfkill` and
/// `/dev/rfkill`), which [`crate::read`], [`crate::set_wifi_enabled`],
/// [`crate::watch`] and [`crate::follow`] use. Their `_with` variants take
/// another, so a test against a stand-in NetworkManager neither reads the
/// host's radios nor turns its Wi-Fi on.
///
/// # Examples
///
/// ```
/// use otto_network::rfkill::Radios;
///
/// // No radios: no block is read, none is lifted and none is followed.
/// assert_eq!(Radios::none().wifi_block(), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Radios {
    sysfs: Option<PathBuf>,
    device: Option<PathBuf>,
}

impl Default for Radios {
    fn default() -> Self {
        Self::system()
    }
}

impl Radios {
    /// The machine's radios: `/sys/class/rfkill` and `/dev/rfkill`.
    pub fn system() -> Self {
        Self::new(SYSFS, Some(PathBuf::from(DEVICE)))
    }

    /// Radios listed under `sysfs`, laid out as `/sys/class/rfkill` is, and
    /// controlled through `device`, or through nothing when it is `None`.
    pub fn new(sysfs: impl Into<PathBuf>, device: Option<PathBuf>) -> Self {
        Self {
            sysfs: Some(sysfs.into()),
            device,
        }
    }

    /// No radios: nothing is read, lifted or followed.
    pub fn none() -> Self {
        Self {
            sysfs: None,
            device: None,
        }
    }

    /// The Wi-Fi radios' block. `None` where there is no Wi-Fi radio rfkill
    /// knows of, or no sysfs to ask.
    pub fn wifi_block(&self) -> Option<Block> {
        wifi_block_in(self.sysfs.as_deref()?)
    }

    /// Lift the soft block on every Wi-Fi radio. A hard block stays: the
    /// kernel lets no software lift that.
    pub(crate) fn unblock_wifi(&self) -> std::io::Result<()> {
        let Some(device) = &self.device else {
            return Err(std::io::ErrorKind::NotFound.into());
        };
        std::fs::OpenOptions::new()
            .write(true)
            .open(device)?
            .write_all(&change_all_event(TYPE_WLAN, false))
    }

    /// A message each time a Wi-Fi radio is blocked or unblocked: the
    /// airplane key, a physical switch, `rfkill`. NetworkManager does not
    /// say when a soft block comes from elsewhere, so this is how the bar
    /// sees it.
    ///
    /// The kernel answers a read of the device with one event per change,
    /// and with the radios' current state when it is first opened. The reads
    /// block, so they run on a thread of their own, which ends when the
    /// stream is dropped and the next event comes. Where there is no device
    /// the stream never yields.
    pub(crate) fn changes(&self) -> futures_util::stream::BoxStream<'static, ()> {
        use futures_util::StreamExt;
        let Some(Ok(mut device)) = self.device.as_deref().map(std::fs::File::open) else {
            return futures_util::stream::pending().boxed();
        };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("rfkill-watch".into())
            .spawn(move || {
                use std::io::Read;
                // Room for a newer, longer event: only the first eight bytes
                // are read from it.
                let mut event = [0u8; 64];
                while let Ok(read) = device.read(&mut event) {
                    if read < 8 {
                        break;
                    }
                    if matches!(event[4], TYPE_WLAN | TYPE_ALL) && tx.send(()).is_err() {
                        break;
                    }
                }
            });
        if spawned.is_err() {
            return futures_util::stream::pending().boxed();
        }
        futures_util::stream::unfold(
            rx,
            |mut rx| async move { rx.recv().await.map(|()| ((), rx)) },
        )
        .boxed()
    }
}

/// The Wi-Fi radios' block, read from the radios listed under `root`.
fn wifi_block_in(root: &Path) -> Option<Block> {
    let mut found = None::<Block>;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let dir = entry.path();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        if read("type").trim() != "wlan" {
            continue;
        }
        let block = found.get_or_insert_with(Block::default);
        block.soft |= read("soft").trim() == "1";
        block.hard |= read("hard").trim() == "1";
    }
    found
}

/// `struct rfkill_event` (its first, eight-byte version, which every kernel
/// takes): `idx` (u32), `type`, `op`, `soft`, `hard`.
fn change_all_event(kind: u8, soft: bool) -> [u8; 8] {
    let mut event = [0u8; 8];
    event[4] = kind;
    event[5] = OP_CHANGE_ALL;
    event[6] = u8::from(soft);
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("otto-rfkill-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn radio(root: &Path, name: &str, kind: &str, soft: u8, hard: u8) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        std::fs::write(dir.join("soft"), format!("{soft}\n")).unwrap();
        std::fs::write(dir.join("hard"), format!("{hard}\n")).unwrap();
    }

    #[test]
    fn the_wifi_radios_block_together() {
        let root = scratch("block");
        let radios = Radios::new(&root, None);
        assert_eq!(radios.wifi_block(), None, "no radio, no answer");
        assert_eq!(Radios::none().wifi_block(), None, "no sysfs, no answer");
        radio(&root, "rfkill0", "bluetooth", 1, 1);
        assert_eq!(radios.wifi_block(), None, "bluetooth is not Wi-Fi");
        radio(&root, "rfkill1", "wlan", 0, 0);
        assert_eq!(radios.wifi_block(), Some(Block::default()));
        radio(&root, "rfkill2", "wlan", 1, 0);
        let block = radios.wifi_block().unwrap();
        assert!(block.soft && !block.hard);
        radio(&root, "rfkill1", "wlan", 0, 1);
        let block = radios.wifi_block().unwrap();
        assert!(block.soft && block.hard && block.any());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unblocking_writes_one_change_all_event() {
        let root = scratch("event");
        let device = root.join("rfkill");
        std::fs::write(&device, b"").unwrap();
        Radios::new(&root, Some(device.clone()))
            .unblock_wifi()
            .unwrap();
        assert_eq!(std::fs::read(&device).unwrap(), [0, 0, 0, 0, 1, 3, 0, 0]);
        assert!(
            Radios::none().unblock_wifi().is_err(),
            "no device, nothing written"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
