//! Deferred destruction of GPU memory that Skia may still read.
//!
//! Skia wraps images it does not own, so it cannot keep them alive. A
//! texture's Skia image escapes the renderer (lay-rs keeps it to draw the
//! window, pictures record it), and a frame's draws reach the GPU after the
//! texture that fed them may already be dropped. Memory therefore goes
//! through here: it is destroyed once nothing outside holds the Skia handle
//! that reads it, and the GPU has finished the last frame submitted while
//! the handle was still reachable.

// Rust guideline compliant 2026-02-21

use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use layers::skia::{self, ConditionallySend};

/// The Skia handle through which retired memory can still be read.
pub(crate) enum RetiredHandle {
    /// A texture's image.
    Image(skia::Image),
    /// A render target's surface.
    Surface(skia::Surface),
}

impl RetiredHandle {
    /// Nothing but this handle refers to the Skia object any more.
    fn unreferenced(&self) -> bool {
        match self {
            Self::Image(image) => image.can_send(),
            Self::Surface(surface) => surface.can_send(),
        }
    }
}

/// Memory waiting to be destroyed.
pub(crate) struct Retired {
    handle: RetiredHandle,
    _memory: Box<dyn Any>,
    /// Serial of the first frame submitted after the handle became
    /// unreferenced; once the GPU completes it, the memory can go.
    ready_after: Option<u64>,
}

/// Where dropped textures and targets hand their memory to the renderer.
pub(crate) type RetireQueue = Arc<Mutex<Vec<Retired>>>;

/// Hands `memory`, read through `handle`, to the renderer for destruction.
pub(crate) fn retire(queue: &RetireQueue, handle: RetiredHandle, memory: Box<dyn Any>) {
    let retired = Retired {
        handle,
        _memory: memory,
        ready_after: None,
    };
    queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(retired);
}

// SAFETY: Skia objects and Vulkan images are only touched on the render
// thread; the queue is shared with textures Smithay requires to be `Send`,
// which never leave that thread in Otto.
unsafe impl Send for Retired {}

/// Retired memory the renderer is waiting to destroy.
#[derive(Default)]
pub(crate) struct Graveyard {
    queue: RetireQueue,
    entries: Vec<Retired>,
}

impl Graveyard {
    /// The queue textures and targets retire into.
    pub fn queue(&self) -> RetireQueue {
        self.queue.clone()
    }

    /// Destroys what is safe to destroy.
    ///
    /// `submitted` is the serial of the last frame submitted to the GPU,
    /// `completed` the last one the GPU finished. Call after a submit, so
    /// every draw recorded so far belongs to `submitted` or earlier.
    pub fn reap(&mut self, submitted: u64, completed: u64) {
        self.entries
            .append(&mut self.queue.lock().unwrap_or_else(|e| e.into_inner()));
        for entry in &mut self.entries {
            if entry.ready_after.is_none() && entry.handle.unreferenced() {
                entry.ready_after = Some(submitted);
            }
        }
        self.entries
            .retain(|entry| entry.ready_after.is_none_or(|ready| ready > completed));
    }

    /// Destroys everything. The GPU must be idle.
    pub fn clear(&mut self) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.entries.clear();
    }
}
