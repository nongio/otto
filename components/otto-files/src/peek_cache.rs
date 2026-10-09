//! Previews Peek has already decoded, kept so that opening a file again —
//! closing Peek and pressing space on the same photo, stepping back to the
//! previous file — does not run the sandboxed worker a second time.
//!
//! Every decode is a process, and for the formats a program on the system has
//! to convert — a video's poster frame, a phone's HEIC — a slow one: a few
//! hundred milliseconds the panel would otherwise spend showing the icon.
//!
//! Kept in memory, for as long as Files runs, and keyed on everything that
//! decides what the worker would make: the file, its modification time and
//! length, and the request itself. A file rewritten under us, or a panel of a
//! different size, is decoded afresh rather than served stale or blurred.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use otto_kit::preview::{Pixels, Preview};
use otto_peek::decode::Request;

/// How many bytes of decoded previews to keep. A panel-sized picture is a
/// handful of megabytes, so this is a few dozen of them.
pub const BUDGET_BYTES: usize = 192 << 20;

/// The largest preview worth keeping. An animation can come back as a strip
/// of frames a large share of the budget on its own; keeping it would push
/// out everything else to save one decode.
pub const MAX_ENTRY_BYTES: usize = 64 << 20;

/// What decides the worker's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
    /// The request as it would go to the worker. Its `Debug` form, because
    /// every field of it shapes the answer and it has no `Eq` of its own.
    request: String,
}

struct Entry {
    key: Key,
    preview: Preview,
    bytes: usize,
}

/// Most recently used last.
#[derive(Default)]
pub struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    entries: VecDeque::new(),
    bytes: 0,
});

/// `path` decoded for `request`: from the cache when it is there, from the
/// worker otherwise.
pub fn decode(path: &Path, request: &Request) -> Preview {
    let decode = || otto_peek::decode_path(path, request);
    let Some(key) = key_for(path, request) else {
        // Gone, or unreadable: the worker says so better than a cache could.
        return decode();
    };
    if let Some(hit) = lock().get(&key) {
        return hit;
    }
    // Decoded outside the lock: it takes a process and up to seconds, and
    // the thumbnails and the next preview should not queue behind it.
    let preview = decode();
    lock().put(key, &preview);
    preview
}

fn lock() -> std::sync::MutexGuard<'static, Cache> {
    // A thread that panicked while holding it left a cache, not a broken
    // invariant worth refusing previews over.
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn key_for(path: &Path, request: &Request) -> Option<Key> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Key {
        path: path.to_path_buf(),
        modified: metadata.modified().ok(),
        len: metadata.len(),
        request: format!("{request:?}"),
    })
}

impl Cache {
    fn get(&mut self, key: &Key) -> Option<Preview> {
        let at = self.entries.iter().position(|entry| entry.key == *key)?;
        let entry = self.entries.remove(at)?;
        let preview = entry.preview.clone();
        self.entries.push_back(entry);
        Some(preview)
    }

    fn put(&mut self, key: Key, preview: &Preview) {
        // An unavailable file is often a passing state — a deadline missed
        // while the machine was busy, a file still being written — and is
        // worth asking about again.
        if matches!(preview, Preview::Unavailable { .. }) {
            return;
        }
        let bytes = size_of(preview);
        if bytes > MAX_ENTRY_BYTES {
            return;
        }
        // The same file at another modification time is a stale copy now.
        self.remove_where(|entry| entry.key.path == key.path && entry.key.modified != key.modified);
        self.remove_where(|entry| entry.key == key);
        self.entries.push_back(Entry {
            key,
            preview: preview.clone(),
            bytes,
        });
        self.bytes += bytes;
        while self.bytes > BUDGET_BYTES {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= oldest.bytes;
        }
    }

    fn remove_where(&mut self, stale: impl Fn(&Entry) -> bool) {
        let bytes = &mut self.bytes;
        self.entries.retain(|entry| {
            let gone = stale(entry);
            if gone {
                *bytes -= entry.bytes;
            }
            !gone
        });
    }
}

/// Roughly what a preview holds, in bytes: its pixels, which are nearly all
/// of it, and a flat allowance for everything else.
fn size_of(preview: &Preview) -> usize {
    const SMALL: usize = 4 << 10;
    let pixels = |pixels: &Pixels| pixels.data.len();
    SMALL
        + match preview {
            Preview::Pixels { pixels: p, .. } => pixels(p),
            Preview::Pages { pages, words } => {
                pages
                    .iter()
                    .filter_map(|page| page.pixels.as_ref())
                    .map(pixels)
                    .sum::<usize>()
                    + words.len() * 64
            }
            Preview::Card { hero, .. } => hero.as_ref().map_or(0, pixels),
            Preview::Text { lines, .. } => lines.iter().map(String::len).sum(),
            Preview::Document { blocks, .. } => blocks.len() * 256,
            Preview::Rows { rows, .. } => rows.len() * 256,
            Preview::Unavailable { .. } => 0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, modified: u64, request: &str) -> Key {
        Key {
            path: PathBuf::from(name),
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(modified)),
            len: 1,
            request: request.into(),
        }
    }

    fn picture(bytes: usize) -> Preview {
        Preview::Pixels {
            pixels: Pixels {
                width: 1,
                height: 1,
                intrinsic_width: 1,
                intrinsic_height: 1,
                data: vec![0; bytes],
                frame_delays: Vec::new(),
                words: Vec::new(),
            },
            pages: 1,
            page: 1,
        }
    }

    #[test]
    fn serves_what_it_was_given_for_the_same_request_only() {
        let mut cache = Cache::default();
        cache.put(key("a.heic", 1, "panel"), &picture(16));
        assert!(cache.get(&key("a.heic", 1, "panel")).is_some());
        assert!(cache.get(&key("a.heic", 1, "zoomed")).is_none());
        assert!(cache.get(&key("b.heic", 1, "panel")).is_none());
    }

    #[test]
    fn a_rewritten_file_drops_its_old_previews() {
        let mut cache = Cache::default();
        cache.put(key("a.jpg", 1, "panel"), &picture(16));
        cache.put(key("a.jpg", 1, "zoomed"), &picture(16));
        cache.put(key("a.jpg", 2, "panel"), &picture(16));
        assert!(cache.get(&key("a.jpg", 1, "zoomed")).is_none());
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, size_of(&picture(16)));
    }

    #[test]
    fn keeps_no_failures_and_nothing_too_large() {
        let mut cache = Cache::default();
        let unavailable = Preview::Unavailable {
            reason: "timed out".into(),
            icon: Vec::new(),
        };
        cache.put(key("a.mp4", 1, "panel"), &unavailable);
        cache.put(key("b.gif", 1, "panel"), &picture(MAX_ENTRY_BYTES + 1));
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn drops_the_least_recently_used_past_the_budget() {
        let mut cache = Cache::default();
        // Three fit; a fourth does not.
        let large = BUDGET_BYTES / 4 + 1;
        cache.put(key("a", 1, "p"), &picture(large));
        cache.put(key("b", 1, "p"), &picture(large));
        cache.put(key("c", 1, "p"), &picture(large));
        // Used, so it outlives `b`.
        assert!(cache.get(&key("a", 1, "p")).is_some());
        cache.put(key("d", 1, "p"), &picture(large));
        assert!(cache.bytes <= BUDGET_BYTES);
        assert!(cache.get(&key("b", 1, "p")).is_none());
        for kept in ["a", "c", "d"] {
            assert!(cache.get(&key(kept, 1, "p")).is_some());
        }
    }
}
