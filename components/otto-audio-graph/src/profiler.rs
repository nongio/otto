//! Dropouts (xruns) per driver, read from PipeWire's profiler.
//!
//! The server's profiler module reports, for every cycle of every driver,
//! how long the cycle took and how many times the driver has missed its
//! deadline. A connection that asks for it with
//! [`Connection::watch_dropouts`](crate::Connection::watch_dropouts) binds
//! the profiler and reports a driver's count as [`Event::Dropouts`] when it
//! changes; nothing is reported for the cycles that go well.
//!
//! pipewire-rs has no profiler proxy, so this binds it through the
//! registry's own method table and listens with the C event struct, as
//! `module.rs` does for modules.

// Rust guideline compliant 2026-02-21

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::ptr::NonNull;
use std::rc::Rc;

use pipewire as pw;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::Value;

use crate::model::Event;

/// What the listener keeps: the hook PipeWire points at, where to report,
/// and each driver's last count. Boxed, so the hook never moves.
struct Listener {
    hook: pw::spa::sys::spa_hook,
    emit: Rc<dyn Fn(Event)>,
    counts: RefCell<HashMap<u32, u32>>,
}

/// The events the profiler listener takes: only `profile`.
static EVENTS: pw::sys::pw_profiler_events = pw::sys::pw_profiler_events {
    version: pw::sys::PW_VERSION_PROFILER_EVENTS,
    profile: Some(on_profile),
};

/// A profile arrived: the cycles since the last one, one object each.
///
/// # Safety
///
/// `data` is the `Listener` registered with the hook, which lives until the
/// hook is removed in `Profiler::drop`, and `pod` a valid pod for the call.
unsafe extern "C" fn on_profile(data: *mut c_void, pod: *const pw::spa::sys::spa_pod) {
    if data.is_null() || pod.is_null() {
        return;
    }
    // SAFETY: see above; PipeWire calls this on the connection's thread,
    // the only one that touches the listener, and a pod is its 8-byte
    // header followed by `size` bytes of body.
    let (listener, bytes) = unsafe {
        let size = (*pod).size as usize + std::mem::size_of::<pw::spa::sys::spa_pod>();
        (
            &*data.cast::<Listener>(),
            std::slice::from_raw_parts(pod.cast::<u8>(), size),
        )
    };
    let Ok((_, value)) = PodDeserializer::deserialize_any_from(bytes) else {
        return;
    };
    for (driver, name, count) in driver_counts(&value) {
        let changed = listener.counts.borrow_mut().insert(driver, count) != Some(count);
        if changed {
            (listener.emit)(Event::Dropouts {
                driver,
                name,
                count,
            });
        }
    }
}

/// Each driver's id, name and dropout count in a profile: a struct of
/// per-cycle objects, or one object.
fn driver_counts(value: &Value) -> Vec<(u32, String, u32)> {
    let objects: Vec<&Value> = match value {
        Value::Struct(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut found: Vec<(u32, String, u32)> = Vec::new();
    for object in objects {
        let Value::Object(object) = object else {
            continue;
        };
        let field = |key: u32| {
            object
                .properties
                .iter()
                .find(|p| p.key == key)
                .and_then(|p| match &p.value {
                    Value::Struct(fields) => Some(fields),
                    _ => None,
                })
        };
        let Some(block) = field(pw::spa::sys::SPA_PROFILER_driverBlock) else {
            continue;
        };
        let (Some(Value::Int(id)), Some(Value::String(name))) = (block.first(), block.get(1))
        else {
            continue;
        };
        // The block's count, from PipeWire 0.3.68; before that, the info's.
        let count = match block.get(8) {
            Some(Value::Int(count)) => Some(*count),
            _ => field(pw::spa::sys::SPA_PROFILER_info).and_then(|info| match info.get(4) {
                Some(Value::Int(count)) => Some(*count),
                _ => None,
            }),
        };
        let (Ok(id), Some(Ok(count))) = (u32::try_from(*id), count.map(u32::try_from)) else {
            continue;
        };
        found.retain(|(d, _, _)| *d != id);
        found.push((id, name.clone(), count));
    }
    found
}

/// The profiler, bound on this connection; dropping it unbinds it.
pub(crate) struct Profiler {
    proxy: NonNull<pw::sys::pw_proxy>,
    listener: Box<Listener>,
}

impl Profiler {
    /// Bind the profiler global `id` and report dropouts to `emit`.
    pub(crate) fn bind(
        registry: &pw::registry::RegistryRc,
        id: u32,
        emit: &Rc<dyn Fn(Event)>,
    ) -> Option<Self> {
        let type_ = CString::new("PipeWire:Interface:Profiler").ok()?;
        // SAFETY: the registry pointer is live for the call, its interface's
        // method table is `pw_registry_methods`, and `bind` returns a new
        // proxy or null.
        let proxy = unsafe {
            let iface = registry.as_raw_ptr().cast::<pw::spa::sys::spa_interface>();
            let funcs = (*iface).cb.funcs.cast::<pw::sys::pw_registry_methods>();
            let bind = (*funcs).bind?;
            bind(
                (*iface).cb.data,
                id,
                type_.as_ptr(),
                pw::sys::PW_VERSION_PROFILER,
                0,
            )
        };
        let proxy = NonNull::new(proxy.cast::<pw::sys::pw_proxy>())?;
        let mut listener = Box::new(Listener {
            // SAFETY: a zeroed hook is how PipeWire expects a new one.
            hook: unsafe { std::mem::zeroed() },
            emit: emit.clone(),
            counts: RefCell::new(HashMap::new()),
        });
        let data: *mut Listener = &mut *listener;
        // SAFETY: the proxy is the profiler just bound, the hook and the
        // listener it hands back are boxed and outlive the hook (removed in
        // `drop` before the box goes), and the events are static.
        unsafe {
            pw::sys::pw_proxy_add_object_listener(
                proxy.as_ptr(),
                std::ptr::addr_of_mut!((*data).hook),
                std::ptr::addr_of!(EVENTS).cast(),
                data.cast(),
            );
        }
        Some(Self { proxy, listener })
    }
}

impl Drop for Profiler {
    fn drop(&mut self) {
        // SAFETY: the hook was added in `bind` and is removed once; the
        // proxy was made there and is destroyed once, after the hook.
        unsafe {
            pw::spa::sys::spa_hook_remove(std::ptr::addr_of_mut!(self.listener.hook));
            pw::sys::pw_proxy_destroy(self.proxy.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pw::spa::pod::{Object, Property};
    use pw::spa::utils::Fraction;

    fn cycle(id: i32, name: &str, xruns: i32) -> Value {
        Value::Object(Object {
            type_: pw::spa::sys::SPA_TYPE_OBJECT_Profiler,
            id: 0,
            properties: vec![
                Property::new(
                    pw::spa::sys::SPA_PROFILER_info,
                    Value::Struct(vec![
                        Value::Long(1),
                        Value::Float(0.1),
                        Value::Float(0.1),
                        Value::Float(0.1),
                        Value::Int(99),
                    ]),
                ),
                Property::new(
                    pw::spa::sys::SPA_PROFILER_driverBlock,
                    Value::Struct(vec![
                        Value::Int(id),
                        Value::String(name.into()),
                        Value::Long(0),
                        Value::Long(0),
                        Value::Long(0),
                        Value::Long(0),
                        Value::Int(0),
                        Value::Fraction(Fraction {
                            num: 1024,
                            denom: 48_000,
                        }),
                        Value::Int(xruns),
                    ]),
                ),
            ],
        })
    }

    #[test]
    fn each_driver_s_last_count_is_read() {
        let profile = Value::Struct(vec![
            cycle(40, "alsa_output.speakers", 2),
            cycle(41, "bluez_output.AA", 0),
            cycle(40, "alsa_output.speakers", 3),
        ]);
        let mut counts = driver_counts(&profile);
        counts.sort();
        assert_eq!(
            counts,
            vec![
                (40, "alsa_output.speakers".to_string(), 3),
                (41, "bluez_output.AA".to_string(), 0),
            ]
        );
    }

    #[test]
    fn an_older_block_falls_back_to_the_info_count() {
        let Value::Object(mut object) = cycle(40, "speakers", 0) else {
            unreachable!()
        };
        if let Value::Struct(fields) = &mut object.properties[1].value {
            fields.truncate(8);
        }
        assert_eq!(
            driver_counts(&Value::Object(object)),
            vec![(40, "speakers".to_string(), 99)]
        );
    }
}
