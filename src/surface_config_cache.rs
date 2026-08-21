//! Idempotence keys for [`crate::workspaces::utils::configure_surface_layer`].
//!
//! Mirroring a Wayland surface into its lay-rs layer is a write-only
//! operation: every `set_position` / `set_size` / `set_draw_content` schedules
//! a change with `NEEDS_LAYOUT` or `NEEDS_PAINT`, and lay-rs applies it without
//! comparing the value first. Re-configuring an unchanged layer therefore
//! *manufactures* damage.
//!
//! That matters because the surface sync runs per WINDOW, not per surface: a
//! commit on any surface of a window re-configures the window's whole surface
//! tree and every popup hanging off it. A client repainting its content at
//! frame rate was dirtying its own tooltip at frame rate, and popup damage
//! drives the cross-plane backdrop rebuild (see `udev::backdrop`) — a
//! full-screen downscale + blur + a re-render of every blur-bearing plane.
//!
//! So each surface's configuration is reduced to one hash and remembered here.
//! An unchanged key means the layer already holds exactly this state and the
//! whole configure is skipped, leaving the node clean. The key includes the
//! surface's `CommitCounter` (via `WindowViewSurface`'s `Hash`), so real
//! content changes still fall through and repaint.
//!
//! Keyed by surface id and evicted with the surface's texture.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use smithay::reexports::wayland_server::backend::ObjectId;

static CONFIG_KEYS: OnceLock<Mutex<HashMap<ObjectId, u64>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<ObjectId, u64>> {
    CONFIG_KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record `key` for `id` and report whether it differs from the stored one.
/// `true` means the caller must (re)configure the layer.
///
/// Fails open: if the lock can't be taken, report a change rather than risk
/// skipping a real update.
pub fn record_if_changed(id: &ObjectId, key: u64) -> bool {
    let Ok(mut map) = store().try_lock() else {
        return true;
    };
    record_in(&mut map, id, key)
}

/// The gate itself, over any key type so it can be tested without a Wayland
/// client (`ObjectId` needs one). Returns whether the caller must reconfigure.
fn record_in<K>(map: &mut HashMap<K, u64>, id: &K, key: u64) -> bool
where
    K: std::hash::Hash + Eq + Clone,
{
    match map.get(id) {
        Some(prev) if *prev == key => false,
        _ => {
            map.insert(id.clone(), key);
            true
        }
    }
}

/// Drop the remembered key so the next configure runs in full. Use whenever
/// the layer behind a surface is replaced or its state is changed outside
/// `configure_surface_layer`.
pub fn invalidate(id: &ObjectId) {
    if let Ok(mut map) = store().try_lock() {
        map.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sight_of_a_surface_always_configures() {
        let mut map: HashMap<&str, u64> = HashMap::new();
        assert!(record_in(&mut map, &"a", 1));
    }

    #[test]
    fn an_unchanged_configuration_is_skipped() {
        // The whole point: a client repainting its window re-runs the sync for
        // every OTHER surface it owns (subsurfaces, popups) with byte-identical
        // values. Those must not reach the scene.
        let mut map: HashMap<&str, u64> = HashMap::new();
        assert!(record_in(&mut map, &"tooltip", 7));
        for _ in 0..100 {
            assert!(
                !record_in(&mut map, &"tooltip", 7),
                "an unchanged surface must never be reconfigured"
            );
        }
    }

    #[test]
    fn a_changed_configuration_configures_once_then_settles() {
        // Real content changes must fall through — the key carries the
        // surface's CommitCounter, so this is what a client commit looks like.
        let mut map: HashMap<&str, u64> = HashMap::new();
        record_in(&mut map, &"win", 1);
        assert!(record_in(&mut map, &"win", 2), "a new commit reconfigures");
        assert!(!record_in(&mut map, &"win", 2), "and then settles again");
    }

    #[test]
    fn surfaces_do_not_share_state() {
        let mut map: HashMap<&str, u64> = HashMap::new();
        assert!(record_in(&mut map, &"a", 1));
        assert!(record_in(&mut map, &"b", 1), "b has never been configured");
        assert!(!record_in(&mut map, &"a", 1));
    }

    #[test]
    fn invalidation_forces_a_full_reconfigure() {
        // Used where a layer's draw content is replaced behind the gate's back
        // (scanout promotion blanks it). Without this the demotion re-import
        // would match the stale key and leave the window blank.
        let mut map: HashMap<&str, u64> = HashMap::new();
        record_in(&mut map, &"promoted", 5);
        assert!(!record_in(&mut map, &"promoted", 5));
        map.remove("promoted");
        assert!(
            record_in(&mut map, &"promoted", 5),
            "invalidated: reconfigure"
        );
    }
}
