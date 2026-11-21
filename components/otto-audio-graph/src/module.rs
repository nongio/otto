//! PipeWire modules loaded into this program's own context.
//!
//! A loopback and an effect's filter chain are modules: loading one into the
//! connection's context makes its nodes this program's, so they go when it
//! does. This is the one place that calls into `libpipewire` directly.

// Rust guideline compliant 2026-02-21

use std::cell::Cell;
use std::ffi::{c_void, CString};
use std::ptr::NonNull;

use pipewire as pw;

/// Told when a module is destroyed, whoever destroys it: a filter chain or a
/// loopback destroys itself when its streams fail, and must not be destroyed
/// again.
///
/// Boxed, so the hook PipeWire keeps a pointer to never moves.
struct Watch {
    hook: pw::spa::sys::spa_hook,
    alive: Cell<bool>,
}

/// The events a [`Watch`] listens for: only `destroy`.
static EVENTS: pw::sys::pw_impl_module_events = pw::sys::pw_impl_module_events {
    version: pw::sys::PW_VERSION_IMPL_MODULE_EVENTS,
    destroy: Some(on_destroy),
    free: None,
    initialized: None,
    registered: None,
};

/// The module behind a [`Watch`] is being destroyed.
///
/// # Safety
///
/// `data` is the `Watch` registered with the hook, which lives until the
/// hook is removed, here or in `LoadedModule::drop`.
unsafe extern "C" fn on_destroy(data: *mut c_void) {
    let watch = data.cast::<Watch>();
    // SAFETY: see above; PipeWire calls this on the thread that owns the
    // context, the only one that touches the watch.
    unsafe {
        (*watch).alive.set(false);
        pw::spa::sys::spa_hook_remove(std::ptr::addr_of_mut!((*watch).hook));
    }
}

/// A module loaded into this program's context; dropping it unloads it.
///
/// Must be dropped before the context it was loaded into.
pub(crate) struct LoadedModule {
    module: NonNull<pw::sys::pw_impl_module>,
    watch: Box<Watch>,
}

impl LoadedModule {
    /// Whether the module still exists; `false` once it destroyed itself.
    pub(crate) fn is_alive(&self) -> bool {
        self.watch.alive.get()
    }
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        if !self.watch.alive.get() {
            return;
        }
        // SAFETY: the module is alive (its destroy event has not come), the
        // pointer came from `pw_context_load_module`, and the context it
        // lives in outlives it (see `State::clear` in `connection.rs`). The
        // hook is removed first, so the destroy event finds no watch.
        unsafe {
            pw::spa::sys::spa_hook_remove(std::ptr::addr_of_mut!(self.watch.hook));
            pw::sys::pw_impl_module_destroy(self.module.as_ptr());
        }
    }
}

/// Load module `name` with `args` (SPA JSON) into `context`. `None` when it
/// failed, which is logged.
pub(crate) fn load(
    context: &pw::context::ContextRc,
    name: &str,
    args: &str,
) -> Option<LoadedModule> {
    let (Ok(c_name), Ok(c_args)) = (CString::new(name), CString::new(args)) else {
        tracing::warn!(module = name, "module arguments hold a NUL byte");
        return None;
    };
    // SAFETY: the context is live for the call, both strings are valid and
    // NUL-terminated for its whole length, and a null properties pointer is
    // allowed. The module is destroyed before the context (see
    // `LoadedModule`).
    let module = unsafe {
        pw::sys::pw_context_load_module(
            context.as_raw_ptr(),
            c_name.as_ptr(),
            c_args.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    let Some(module) = NonNull::new(module) else {
        tracing::warn!(
            error = %std::io::Error::last_os_error(),
            module = name,
            "loading a module failed"
        );
        return None;
    };
    let mut watch = Box::new(Watch {
        // SAFETY: a hook is plain pointers and optional callbacks, all of
        // which may be zero; PipeWire fills it in when it is added.
        hook: unsafe { std::mem::zeroed() },
        alive: Cell::new(true),
    });
    let data: *mut Watch = std::ptr::addr_of_mut!(*watch);
    // SAFETY: the module is alive; the hook and its data are boxed and kept
    // in the `LoadedModule`, which removes the hook before freeing them, and
    // `EVENTS` is static.
    unsafe {
        pw::sys::pw_impl_module_add_listener(
            module.as_ptr(),
            std::ptr::addr_of_mut!((*data).hook).cast(),
            std::ptr::addr_of!(EVENTS),
            data.cast(),
        );
    }
    Some(LoadedModule { module, watch })
}

/// Quote `value` as a SPA JSON string.
pub(crate) fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
