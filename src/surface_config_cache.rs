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
//! So each surface's configuration is reduced to two hashes and remembered
//! here: what it *shows* (buffer, crop, size) and where it *sits*. An unchanged
//! pair means the layer already holds exactly this state and the whole
//! configure is skipped, leaving the node clean. The content key includes the
//! surface's `CommitCounter`, so real content changes still fall through and
//! repaint.
//!
//! The two are kept apart because a surface that only moved — a subsurface
//! repositioned without a new buffer, which is how a scrolled band moves — has
//! nothing new to paint. Re-installing its draw content would repaint the
//! layer and report the damage of the buffer it *last* received as though it
//! had just arrived: the whole band, every frame of a scroll.
//!
//! Keyed by surface id and evicted with the surface's texture.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use smithay::reexports::wayland_server::backend::ObjectId;

/// What a surface's layer needs, given what it held before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconfigure {
    /// The layer already holds exactly this state.
    Nothing,
    /// Only where the surface sits changed: move it, paint nothing.
    Placement,
    /// What the surface shows changed, or the layer has never been configured.
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Keys {
    content: u64,
    placement: u64,
}

static CONFIG_KEYS: OnceLock<Mutex<HashMap<ObjectId, Keys>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<ObjectId, Keys>> {
    CONFIG_KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record the keys for `id` and report what the layer needs.
///
/// Fails open: if the lock can't be taken, report a full change rather than
/// risk skipping a real update.
pub fn record(id: &ObjectId, content: u64, placement: u64) -> Reconfigure {
    let Ok(mut map) = store().try_lock() else {
        return Reconfigure::Full;
    };
    record_in(&mut map, id, Keys { content, placement })
}

/// The gate itself, over any key type so it can be tested without a Wayland
/// client (`ObjectId` needs one).
fn record_in<K>(map: &mut HashMap<K, Keys>, id: &K, keys: Keys) -> Reconfigure
where
    K: std::hash::Hash + Eq + Clone,
{
    let previous = map.insert(id.clone(), keys);
    match previous {
        Some(prev) if prev == keys => Reconfigure::Nothing,
        Some(prev) if prev.content == keys.content => Reconfigure::Placement,
        _ => Reconfigure::Full,
    }
}

/// Drop the remembered keys so the next configure runs in full. Use whenever
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

    fn keys(content: u64, placement: u64) -> Keys {
        Keys { content, placement }
    }

    #[test]
    fn the_first_sight_of_a_surface_always_configures() {
        let mut map: HashMap<&str, Keys> = HashMap::new();
        assert_eq!(record_in(&mut map, &"a", keys(1, 1)), Reconfigure::Full);
    }

    #[test]
    fn an_unchanged_configuration_is_skipped() {
        // The whole point: a client repainting its window re-runs the sync for
        // every OTHER surface it owns (subsurfaces, popups) with byte-identical
        // values. Those must not reach the scene.
        let mut map: HashMap<&str, Keys> = HashMap::new();
        record_in(&mut map, &"tooltip", keys(7, 3));
        for _ in 0..100 {
            assert_eq!(
                record_in(&mut map, &"tooltip", keys(7, 3)),
                Reconfigure::Nothing,
                "an unchanged surface must never be reconfigured"
            );
        }
    }

    #[test]
    fn a_changed_configuration_configures_once_then_settles() {
        // Real content changes must fall through — the key carries the
        // surface's CommitCounter, so this is what a client commit looks like.
        let mut map: HashMap<&str, Keys> = HashMap::new();
        record_in(&mut map, &"win", keys(1, 0));
        assert_eq!(
            record_in(&mut map, &"win", keys(2, 0)),
            Reconfigure::Full,
            "a new commit reconfigures"
        );
        assert_eq!(
            record_in(&mut map, &"win", keys(2, 0)),
            Reconfigure::Nothing,
            "and then settles again"
        );
    }

    #[test]
    fn a_move_alone_is_only_a_placement() {
        // A scrolled band: repositioned every frame, never repainted. Treating
        // the move as new content would repaint the layer and re-report the
        // damage of the buffer it already had.
        let mut map: HashMap<&str, Keys> = HashMap::new();
        record_in(&mut map, &"band", keys(4, 10));
        assert_eq!(
            record_in(&mut map, &"band", keys(4, 11)),
            Reconfigure::Placement
        );
        assert_eq!(
            record_in(&mut map, &"band", keys(5, 12)),
            Reconfigure::Full,
            "a move that lands with a new buffer is still a full configure"
        );
    }

    #[test]
    fn surfaces_do_not_share_state() {
        let mut map: HashMap<&str, Keys> = HashMap::new();
        record_in(&mut map, &"a", keys(1, 1));
        assert_eq!(
            record_in(&mut map, &"b", keys(1, 1)),
            Reconfigure::Full,
            "b has never been configured"
        );
        assert_eq!(record_in(&mut map, &"a", keys(1, 1)), Reconfigure::Nothing);
    }

    #[test]
    fn invalidation_forces_a_full_reconfigure() {
        // Used where a layer's draw content is replaced behind the gate's back
        // (scanout promotion blanks it). Without this the demotion re-import
        // would match the stale key and leave the window blank.
        let mut map: HashMap<&str, Keys> = HashMap::new();
        record_in(&mut map, &"promoted", keys(5, 5));
        assert_eq!(
            record_in(&mut map, &"promoted", keys(5, 5)),
            Reconfigure::Nothing
        );
        map.remove("promoted");
        assert_eq!(
            record_in(&mut map, &"promoted", keys(5, 5)),
            Reconfigure::Full,
            "invalidated: reconfigure"
        );
    }
}
