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

use std::io::Write;
use std::path::Path;

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

/// The Wi-Fi radios' block, read from `/sys/class/rfkill`. `None` where
/// there is no Wi-Fi radio rfkill knows of, or no sysfs to ask.
pub fn wifi_block() -> Option<Block> {
    wifi_block_in(Path::new(SYSFS))
}

/// [`wifi_block`] under `root`, for a test.
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

/// Lift the soft block on every Wi-Fi radio. A hard block stays: the
/// kernel lets no software lift that.
pub fn unblock_wifi() -> std::io::Result<()> {
    unblock_wifi_at(Path::new(DEVICE))
}

/// [`unblock_wifi`] through the device at `device`, for a test.
fn unblock_wifi_at(device: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(device)?
        .write_all(&change_all_event(TYPE_WLAN, false))
}

/// A message each time a Wi-Fi radio is blocked or unblocked: the airplane
/// key, a physical switch, `rfkill`. NetworkManager does not say when a soft
/// block comes from elsewhere, so this is how the bar sees it.
///
/// The kernel answers a read of `/dev/rfkill` with one event per change, and
/// with the radios' current state when it is first opened. The reads block,
/// so they run on a thread of their own, which ends when the stream is
/// dropped and the next event comes. Where there is no `/dev/rfkill` the
/// stream never yields.
pub fn changes() -> futures_util::stream::BoxStream<'static, ()> {
    use futures_util::StreamExt;
    let Ok(mut device) = std::fs::File::open(DEVICE) else {
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
        assert_eq!(wifi_block_in(&root), None, "no radio, no answer");
        radio(&root, "rfkill0", "bluetooth", 1, 1);
        assert_eq!(wifi_block_in(&root), None, "bluetooth is not Wi-Fi");
        radio(&root, "rfkill1", "wlan", 0, 0);
        assert_eq!(wifi_block_in(&root), Some(Block::default()));
        radio(&root, "rfkill2", "wlan", 1, 0);
        let block = wifi_block_in(&root).unwrap();
        assert!(block.soft && !block.hard);
        radio(&root, "rfkill1", "wlan", 0, 1);
        let block = wifi_block_in(&root).unwrap();
        assert!(block.soft && block.hard && block.any());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unblocking_writes_one_change_all_event() {
        let root = scratch("event");
        let device = root.join("rfkill");
        std::fs::write(&device, b"").unwrap();
        unblock_wifi_at(&device).unwrap();
        assert_eq!(std::fs::read(&device).unwrap(), [0, 0, 0, 0, 1, 3, 0, 0]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
