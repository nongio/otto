mod activity;
mod dbus_service;
mod dialog;
mod notifications;
mod renderer;
mod state;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_kit::protocols::otto_surface_style_v1::{ClipMode, ContentsGravity};
use otto_kit::surfaces::{LayerShellSurface, SubsurfaceSurface};
use otto_kit::{App, AppContext, AppRunner};
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::wl_keyboard::KeyState;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
};

use crate::activity::Activity;
use crate::dbus_service::{DialogService, IslandService, DBUS_NAME};
use crate::dialog::{DialogHit, DialogId, DialogResponse, DialogView};
use crate::renderer::{
    animate_to, apply_island_style, draw_centered, set_size_and_position, COMPACT_H, MINI_H, MINI_W,
};
use crate::state::{IslandState, SharedState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const LAYER_W: u32 = 800;
const LAYER_H: u32 = 400; // Tall enough for pill + MAX_VISIBLE_CARDS cards.
const BAR_HEIGHT: f32 = 36.0;
const GAP: f32 = 6.0;
/// Top edge of a dialog panel, dropped down just below the island bar.
const DIALOG_TOP: f32 = BAR_HEIGHT + 14.0;
/// Seconds of inactivity before the focused island shrinks to Mini.
const FOCUS_TIMEOUT_SECS: f64 = 4.0;
/// Seconds a destroyed surface is kept alive so its exit animation can play.
const DESTROY_DELAY_SECS: f64 = 0.8;

// ---------------------------------------------------------------------------
// Island — one notification group or music activity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IslandMode {
    Mini,
    Compact,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IslandKind {
    Notification,
}

/// Signature of what a pill buffer currently shows — redraw only on change.
#[derive(Clone, PartialEq)]
struct PillContent {
    mode: IslandMode,
    icon: String,
    title: String,
    count: usize,
    w: f32,
    h: f32,
}

/// Signature of what a card buffer currently shows.
#[derive(Clone, PartialEq)]
struct CardContent {
    title: String,
    body: String,
    icon: String,
    time_label: String,
}

/// An island represents one group (notification app_id or music).
/// It owns a pill/circle subsurface and optionally card subsurfaces.
struct Island {
    /// The group key (app_id for notifications, "org.otto.music" for music).
    app_id: String,
    kind: IslandKind,
    /// The icon for this group (resolved once, used consistently in all modes).
    icon: String,
    /// The pill/circle subsurface.
    surface: SubsurfaceSurface,
    /// Lazily-created card subsurfaces (only when Expanded, notifications only).
    cards: Vec<CardSurface>,
    /// Current mode.
    mode: IslandMode,
    /// When this group first appeared.
    created_at: std::time::Instant,
    /// Last known notification count (for pulse detection).
    last_count: usize,
    /// Last seen activity ID (to detect new notifications even when count doesn't change).
    last_activity_id: u64,
    /// When set, the island temporarily shows as Compact until this instant.
    peek_until: Option<std::time::Instant>,
    /// Last layout target (w, h, x, y) — skip animation when unchanged.
    last_layout: (f32, f32, f32, f32),
    /// Last drawn pill content — skip redraw when unchanged.
    last_content: Option<PillContent>,
}

struct CardSurface {
    surface: SubsurfaceSurface,
    activity_id: u64,
    /// Last drawn card content — skip redraw when unchanged.
    last_content: Option<CardContent>,
}

/// A presented Access-style dialog panel (one subsurface, drawn as a whole).
struct DialogPanel {
    id: DialogId,
    surface: SubsurfaceSurface,
    view: DialogView,
    /// Per choice-group selected option index.
    selected: Vec<usize>,
    /// Panel top-left in layer coordinates (for hit testing).
    origin: (f32, f32),
    /// Cached content height (for layer sizing / input region).
    layout_h: f32,
    /// Whether the entrance animation has played.
    entered: bool,
}

// ---------------------------------------------------------------------------
// IslandApp
// ---------------------------------------------------------------------------

struct IslandApp {
    state: SharedState,
    layer_surface: Option<LayerShellSurface>,
    islands: Vec<Island>,
    surfaces_ready: bool,
    /// Which island (by app_id) is currently focused (Compact/Expanded).
    focused_app: Option<String>,
    /// Which island (by app_id) the pointer is currently hovering over.
    hovered_app: Option<String>,
    /// Surfaces pending destruction (kept alive for animations, destroyed next cycle).
    pending_destroy: Vec<(SubsurfaceSurface, std::time::Instant)>,
    /// Last time the user interacted (pointer event). Used for focus timeout.
    last_interaction: std::time::Instant,
    /// Last applied layer size — skip set_size/commit when unchanged.
    last_layer_size: Option<(u32, u32)>,
    /// Last applied input region rects — skip region set/commit when unchanged.
    last_input_region: Option<Vec<(i32, i32, i32, i32)>>,
    /// The currently-presented Access-style dialog, if any.
    dialog: Option<DialogPanel>,
}

impl IslandApp {
    fn new(state: SharedState) -> Self {
        Self {
            state,
            layer_surface: None,
            islands: Vec::new(),
            surfaces_ready: false,
            focused_app: None,
            hovered_app: None,
            pending_destroy: Vec::new(),
            last_interaction: std::time::Instant::now(),
            last_layer_size: None,
            last_input_region: None,
            dialog: None,
        }
    }

    /// Compute the current effective layer width based on island layout.
    fn layer_width(&self) -> f32 {
        let total_w: f32 = self
            .islands
            .iter()
            .map(|i| i.last_layout.0.max(MINI_H))
            .sum::<f32>()
            + (self.islands.len().saturating_sub(1)) as f32 * GAP;
        (total_w + 40.0).max(LAYER_W as f32)
    }

    /// Get the parent wl_surface for creating subsurfaces.
    fn wl_surface(&self) -> Option<wayland_client::protocol::wl_surface::WlSurface> {
        self.layer_surface
            .as_ref()
            .map(|l| l.base_surface().wl_surface().clone())
    }

    /// Create a new subsurface for an island pill.
    /// Starts at Mini pill size so it doesn't flash as a big black rect.
    fn create_pill_subsurface(&self) -> Option<SubsurfaceSurface> {
        let wl = self.wl_surface()?;
        let surface =
            SubsurfaceSurface::new(&wl, 0, 0, renderer::SLOT_BUF_W, renderer::SLOT_BUF_H).ok()?;
        apply_island_style(&surface, MINI_H as f64 / 2.0, ContentsGravity::Center);
        // Center coordinates (anchor point is 0.5, 0.5).
        let cx = self.layer_width() / 2.0;
        let cy = BAR_HEIGHT / 2.0;
        set_size_and_position(&surface, MINI_W, MINI_H, cx, cy);
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });
        Some(surface)
    }

    /// Queue a surface for destruction after animations have time to play.
    fn defer_destroy(&mut self, surface: SubsurfaceSurface) {
        self.pending_destroy
            .push((surface, std::time::Instant::now()));
    }

    /// Destroy surfaces whose animations have had time to complete.
    fn flush_pending_destroy(&mut self) {
        let cutoff = std::time::Instant::now() - Duration::from_secs_f64(DESTROY_DELAY_SECS);
        self.pending_destroy.retain_mut(|(surface, queued_at)| {
            if *queued_at <= cutoff {
                surface.destroy();
                false
            } else {
                true
            }
        });
    }

    // -----------------------------------------------------------------------
    // Sync: reconcile islands with the current state
    // -----------------------------------------------------------------------

    fn sync(&mut self) {
        let state = self.state.lock().unwrap();
        let grouped = state.grouped_activities();
        drop(state);

        // Build the set of app_ids that should exist as islands.
        let mut desired: Vec<(String, IslandKind, String)> = Vec::new(); // (app_id, kind, icon)
        for (activity, _count) in &grouped {
            let kind = IslandKind::Notification;
            if !desired.iter().any(|(id, _, _)| id == &activity.app_id) {
                desired.push((activity.app_id.clone(), kind, activity.icon.clone()));
            }
        }

        // Remove islands whose app_id is no longer present.
        let mut removed_island = false;
        let mut i = 0;
        while i < self.islands.len() {
            if desired
                .iter()
                .any(|(app_id, _, _)| app_id == &self.islands[i].app_id)
            {
                i += 1;
            } else {
                let mut island = self.islands.remove(i);
                tracing::info!(app_id = %island.app_id, "island removed");
                let cx = self.layer_width() / 2.0;
                let cy = BAR_HEIGHT / 2.0;
                let h = COMPACT_H;
                renderer::animate_to_with_opacity(
                    &island.surface,
                    0.0,
                    h,
                    cx,
                    cy,
                    h as f64 / 2.0,
                    Some(0.0),
                    0.3,
                );
                for card in &island.cards {
                    renderer::animate_dismiss(&card.surface, 1.2);
                }
                // Defer destruction of all surfaces.
                for card in island.cards.drain(..) {
                    self.defer_destroy(card.surface);
                }
                self.defer_destroy(island.surface);
                removed_island = true;
            }
        }

        // Add islands for new app_ids.
        for (app_id, kind, icon) in &desired {
            if !self.islands.iter().any(|i| i.app_id == *app_id) {
                if let Some(surface) = self.create_pill_subsurface() {
                    tracing::info!(%app_id, ?kind, %icon, "island created");
                    self.islands.push(Island {
                        app_id: app_id.clone(),
                        kind: *kind,
                        icon: icon.clone(),
                        surface,
                        cards: Vec::new(),
                        mode: IslandMode::Mini,
                        created_at: std::time::Instant::now(),
                        last_count: 0,
                        last_activity_id: 0,
                        peek_until: None,
                        last_layout: (0.0, 0.0, 0.0, 0.0),
                        last_content: None,
                    });
                    // Auto-focus only if no island is currently Expanded.
                    let any_expanded = self.islands.iter().any(|i| i.mode == IslandMode::Expanded);
                    if !any_expanded {
                        self.focused_app = Some(app_id.clone());
                    }
                    self.last_interaction = std::time::Instant::now();
                }
            }
        }

        // Sort islands by creation time (oldest left).
        self.islands.sort_by_key(|i| i.created_at);

        // If focused app no longer exists, clear focus.
        if let Some(ref focused) = self.focused_app {
            if !self.islands.iter().any(|i| i.app_id == *focused) {
                self.focused_app = None;
            }
        }

        // Assign modes: focused gets Compact/Expanded, peeking stays Compact, rest → Mini.
        // Expanded islands are preserved — they coexist with Compact (peeking) islands.
        for island in &mut self.islands {
            if island.mode == IslandMode::Expanded {
                // Expanded stays Expanded — only user interaction (click/focus loss) closes it.
            } else if Some(&island.app_id) == self.focused_app.as_ref() {
                if island.mode == IslandMode::Mini {
                    island.mode = IslandMode::Compact;
                    tracing::debug!(app_id = %island.app_id, "Mini → Compact (focused)");
                }
            } else if island.peek_until.is_some() {
                // Peeking — stay Compact until peek expires.
            } else {
                // Non-focused, non-peeking, non-expanded → Mini.
                if island.mode != IslandMode::Mini {
                    tracing::debug!(app_id = %island.app_id, from = ?island.mode, "→ Mini");
                }
                island.mode = IslandMode::Mini;
            }
        }

        self.layout(&grouped, removed_island);
    }

    /// Close the card stack — animate out but keep surfaces alive for reuse.
    fn close_cards_for(island: &mut Island) {
        tracing::info!(app_id = %island.app_id, cards = island.cards.len(), "stack closed");
        // Slide up to pill center y, keep current x. Fade out.
        let pill_cy = BAR_HEIGHT / 2.0;
        let pill_cx = island.last_layout.2; // cx from last layout
        for card in &island.cards {
            renderer::animate_position_opacity(
                &card.surface,
                renderer::CARD_W,
                renderer::CARD_H,
                pill_cx,
                pill_cy,
                0.0,
                0.0,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Layout: position all islands and their cards
    // -----------------------------------------------------------------------

    fn layout(&mut self, grouped: &[(Activity, usize)], reposition_delay: bool) {
        if self.islands.is_empty() {
            let size_changed = self.update_layer_size();
            self.update_input_region(size_changed);
            return;
        }

        // Compute element sizes for layout.
        let island_size = |island: &Island, mode: IslandMode| -> (f32, f32) {
            let entry = grouped.iter().find(|(a, _)| a.app_id == island.app_id);
            let count = entry.map(|(_, c)| *c).unwrap_or(1);
            let title = entry.map(|(a, _)| a.title.as_str()).unwrap_or("");
            match mode {
                IslandMode::Mini => (renderer::mini_width(count), MINI_H),
                IslandMode::Compact => {
                    let w = renderer::pill_width(&island.app_id, title, count);
                    (w, COMPACT_H)
                }
                IslandMode::Expanded => {
                    let w =
                        renderer::pill_width(&island.app_id, title, count).max(renderer::CARD_W);
                    (w, COMPACT_H)
                }
            }
        };

        // Compute total row width.
        let total_w: f32 = self
            .islands
            .iter()
            .map(|i| island_size(i, i.mode).0)
            .sum::<f32>()
            + (self.islands.len() - 1) as f32 * GAP;

        let mut x = ((self.layer_width() - total_w) / 2.0).max(0.0);

        // Collect positions for expanded islands, pulse targets, and layout targets.
        let mut expanded_layouts: Vec<(usize, f32, f32, f32, f32)> = Vec::new();
        let mut pulse_targets: Vec<(usize, f32, f32, f32, f32)> = Vec::new();
        let mut layout_targets: Vec<(usize, f32, f32, f32, f32)> = Vec::new(); // (idx, w, h, x, y)
        let mut content_updates: Vec<(usize, PillContent)> = Vec::new();

        for (idx, island) in self.islands.iter().enumerate() {
            let count = grouped
                .iter()
                .find(|(a, _)| a.app_id == island.app_id)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            let icon = island.icon.as_str();

            let (base_w, base_h) = island_size(island, island.mode);
            let is_hovered = self.hovered_app.as_ref() == Some(&island.app_id);
            let grow = if is_hovered
                && (island.mode == IslandMode::Mini || island.mode == IslandMode::Compact)
            {
                renderer::HOVER_GROW
            } else {
                0.0
            };
            let w = base_w + grow;
            let h = base_h + grow;
            // Center coordinates for anchor_point(0.5, 0.5).
            let cx = x + w / 2.0;
            let cy = BAR_HEIGHT / 2.0;

            // Detect new notification: count increased or representative activity changed.
            let current_activity_id = grouped
                .iter()
                .find(|(a, _)| a.app_id == island.app_id)
                .map(|(a, _)| a.id)
                .unwrap_or(0);
            let count_increased = count > island.last_count;
            let activity_changed =
                current_activity_id != island.last_activity_id && island.last_activity_id > 0;
            // Only pulse on new notifications (count went up), not on dismissals.
            let should_pulse = island.kind == IslandKind::Notification && count_increased;
            if island.kind == IslandKind::Notification {
                tracing::debug!(
                    app_id = %island.app_id,
                    mode = ?island.mode,
                    count,
                    last_count = island.last_count,
                    current_activity_id,
                    last_activity_id = island.last_activity_id,
                    count_increased,
                    activity_changed,
                    should_pulse,
                    "notification pulse check"
                );
            }

            match island.mode {
                IslandMode::Mini => {
                    let content = PillContent {
                        mode: island.mode,
                        icon: icon.to_string(),
                        title: String::new(),
                        count,
                        w,
                        h,
                    };
                    if island.last_content.as_ref() != Some(&content) {
                        draw_centered(&island.surface, w, h, |canvas| {
                            renderer::draw_mini(canvas, icon, count, w, h);
                        });
                        content_updates.push((idx, content));
                    }
                    if should_pulse {
                        pulse_targets.push((idx, w, h, cx, cy));
                    } else {
                        layout_targets.push((idx, w, h, cx, cy));
                    }
                }
                IslandMode::Compact | IslandMode::Expanded => {
                    let title = grouped
                        .iter()
                        .find(|(a, _)| a.app_id == island.app_id)
                        .map(|(a, _)| a.title.as_str())
                        .unwrap_or("");
                    let expanded = island.mode == IslandMode::Expanded;
                    let content = PillContent {
                        mode: island.mode,
                        icon: icon.to_string(),
                        title: title.to_string(),
                        count,
                        w,
                        h,
                    };
                    if island.last_content.as_ref() != Some(&content) {
                        draw_centered(&island.surface, w, h, |canvas| {
                            renderer::draw_pill(
                                canvas,
                                &island.app_id,
                                icon,
                                title,
                                count,
                                expanded,
                                w,
                                h,
                            );
                        });
                        content_updates.push((idx, content));
                    }
                    if should_pulse {
                        pulse_targets.push((idx, w, h, cx, cy));
                    } else {
                        layout_targets.push((idx, w, h, cx, cy));
                    }

                    if island.mode == IslandMode::Expanded {
                        // Store top-left x for card positioning.
                        expanded_layouts.push((idx, x, cx, cy, w));
                    }
                }
            }

            x += w + GAP;
        }

        for (idx, content) in content_updates {
            self.islands[idx].last_content = Some(content);
        }

        // Apply layout animations only when target changed.
        let layout_delay = if reposition_delay { 0.4 } else { 0.0 };
        for (idx, w, h, x, y) in layout_targets {
            let target = (w, h, x, y);
            if self.islands[idx].last_layout != target {
                let radius = h as f64 / 2.0;
                animate_to(&self.islands[idx].surface, w, h, x, y, radius, layout_delay);
                self.islands[idx].last_layout = target;
            }
        }

        // Apply pulse and peek as Compact for new notifications.
        for (idx, w, h, cx, cy) in pulse_targets {
            let current_mode = self.islands[idx].mode;
            // If already Compact or Expanded, don't downgrade — just refresh content.
            if current_mode == IslandMode::Expanded || current_mode == IslandMode::Compact {
                tracing::info!(
                    app_id = %self.islands[idx].app_id,
                    mode = ?current_mode,
                    "new notification while open — refresh only"
                );
            } else {
                tracing::info!(
                    app_id = %self.islands[idx].app_id,
                    from = ?current_mode,
                    "pulse → peek Compact for 3s"
                );
                renderer::animate_pulse(
                    &self.islands[idx].surface,
                    w,
                    h,
                    cx,
                    cy,
                    h as f64 / 2.0,
                    6.0,
                );
                self.islands[idx].last_layout = (w, h, cx, cy);
                self.islands[idx].peek_until =
                    Some(std::time::Instant::now() + Duration::from_secs(3));
                self.islands[idx].mode = IslandMode::Compact;
            }
            // Update tracking now so the next sync doesn't re-trigger.
            let app_id = &self.islands[idx].app_id;
            if let Some((a, c)) = grouped.iter().find(|(a, _)| &a.app_id == app_id) {
                self.islands[idx].last_count = *c;
                self.islands[idx].last_activity_id = a.id;
            }
            // Mark dirty so the next tick re-layouts at Compact size.
            // Safe from loops because last_count/last_activity_id are now current.
            let mut st = self.state.lock().unwrap();
            st.dirty = true;
        }
        for island in &mut self.islands {
            let entry = grouped.iter().find(|(a, _)| a.app_id == island.app_id);
            island.last_count = entry.map(|(_, c)| *c).unwrap_or(0);
            island.last_activity_id = entry.map(|(a, _)| a.id).unwrap_or(0);
        }

        // Now lay out cards for expanded islands (separate pass to avoid borrow conflict).
        // Collect (notifs, group_icon) per app_id.
        let state = self.state.lock().unwrap();
        let all_notifs: std::collections::HashMap<String, (Vec<Activity>, String)> = {
            let mut map = std::collections::HashMap::new();
            for (idx, _, _, _, _) in &expanded_layouts {
                let app_id = &self.islands[*idx].app_id;
                let notifs: Vec<Activity> = state
                    .notifications_for_app(app_id)
                    .into_iter()
                    .cloned()
                    .collect();
                // Group icon: from grouped_activities representative.
                let group_icon = grouped
                    .iter()
                    .find(|(a, _)| a.app_id == *app_id)
                    .map(|(a, _)| a.icon.clone())
                    .unwrap_or_default();
                map.insert(app_id.clone(), (notifs, group_icon));
            }
            map
        };
        drop(state);

        let mut dismissed_card_surfaces: Vec<SubsurfaceSurface> = Vec::new();
        // place_above on a new card only takes effect on a parent commit — force one.
        let mut card_created = false;

        // Capture wl_surface before mutable borrow of islands.
        let wl = self.wl_surface();

        for (idx, pill_left_x, _pill_cx, _pill_cy, pill_w) in expanded_layouts {
            let island = &mut self.islands[idx];
            let Some((notifs, group_icon)) = all_notifs.get(&island.app_id) else {
                continue;
            };
            let pill_h = COMPACT_H;

            let card_w = renderer::CARD_W;
            let card_h = renderer::CARD_H;
            let card_gap = renderer::CARD_GAP;
            // Center x for cards (centered under pill).
            let card_cx = pill_left_x + pill_w / 2.0;
            // Pill bottom edge in top-left coords.
            let pill_bottom = (BAR_HEIGHT - pill_h) / 2.0 + pill_h;
            let max_cards = 5;

            for (i, notif) in notifs.iter().take(max_cards).enumerate() {
                // Card center y.
                let card_top = pill_bottom + card_gap + (i as f32) * (card_h + card_gap);
                let card_cy = card_top + card_h / 2.0;
                // Start position: center of card at pill bottom.
                let start_cy = pill_bottom + card_h / 2.0;

                let content = CardContent {
                    title: notif.title.clone(),
                    body: notif.body.clone(),
                    icon: if notif.icon.is_empty() {
                        group_icon.clone()
                    } else {
                        notif.icon.clone()
                    },
                    time_label: renderer::elapsed_label(notif.created_at),
                };
                let existing = island.cards.iter().position(|c| c.activity_id == notif.id);
                let is_new = existing.is_none();
                let cidx = if let Some(ci) = existing {
                    ci
                } else {
                    let Some(ref wl) = wl else { continue };
                    let Ok(surface) = SubsurfaceSurface::new(
                        wl,
                        0,
                        0,
                        renderer::SLOT_BUF_W,
                        renderer::SLOT_BUF_H,
                    ) else {
                        continue;
                    };
                    renderer::apply_card_style(&surface);
                    // Wayland subsurface stacking is parent-relative, not screen-relative.
                    // For a top-anchored layer shell, "above" in the stack means further
                    // from the screen edge — i.e. visually behind the pill. So place_above
                    // makes cards render behind the title surface.
                    surface.place_above(island.surface.wl_surface());
                    // Pre-render content before making the surface visible.
                    draw_centered(&surface, card_w, card_h, |canvas| {
                        renderer::draw_card(canvas, notif, group_icon, card_w, card_h);
                    });
                    set_size_and_position(&surface, card_w, card_h, card_cx, start_cy);
                    island.cards.push(CardSurface {
                        surface,
                        activity_id: notif.id,
                        last_content: Some(content.clone()),
                    });
                    card_created = true;
                    island.cards.len() - 1
                };

                // Redraw only when the card's content actually changed.
                if island.cards[cidx].last_content.as_ref() != Some(&content) {
                    draw_centered(&island.cards[cidx].surface, card_w, card_h, |canvas| {
                        renderer::draw_card(canvas, notif, group_icon, card_w, card_h);
                    });
                    island.cards[cidx].last_content = Some(content);
                }

                if is_new {
                    // New card: start at pill bottom, invisible, slide down + fade in.
                    set_size_and_position(
                        &island.cards[cidx].surface,
                        card_w,
                        card_h,
                        card_cx,
                        start_cy,
                    );
                    if let Some(ss) = island.cards[cidx].surface.base_surface().surface_style() {
                        ss.set_opacity(0.0);
                    }
                    renderer::animate_position_opacity_slow(
                        &island.cards[cidx].surface,
                        card_w,
                        card_h,
                        card_cx,
                        card_cy,
                        1.0,
                        i as f64 * 0.05,
                    );
                } else {
                    // Existing card: animate to position + ensure visible.
                    renderer::animate_position_opacity_slow(
                        &island.cards[cidx].surface,
                        card_w,
                        card_h,
                        card_cx,
                        card_cy,
                        1.0,
                        i as f64 * 0.05,
                    );
                }
            }

            // Remove dismissed cards and reorder to match notification order.
            let notif_ids: Vec<u64> = notifs.iter().take(max_cards).map(|n| n.id).collect();
            let mut i = 0;
            while i < island.cards.len() {
                if notif_ids.contains(&island.cards[i].activity_id) {
                    i += 1;
                } else {
                    let card = island.cards.remove(i);
                    dismissed_card_surfaces.push(card.surface);
                }
            }
            // Sort cards to match layout order (same as notif_ids).
            island.cards.sort_by_key(|c| {
                notif_ids
                    .iter()
                    .position(|&id| id == c.activity_id)
                    .unwrap_or(usize::MAX)
            });
        }
        for s in dismissed_card_surfaces {
            self.defer_destroy(s);
        }

        let size_changed = self.update_layer_size();
        self.update_input_region(size_changed || card_created);
    }

    // -----------------------------------------------------------------------
    // Layer size & input region
    // -----------------------------------------------------------------------

    /// Returns true when the layer size changed (a wl_surface commit is needed
    /// to apply the pending zwlr set_size).
    fn update_layer_size(&mut self) -> bool {
        let Some(layer) = &self.layer_surface else {
            return false;
        };

        // Compute the minimum height needed for current layout.
        let mut max_h = BAR_HEIGHT;

        for island in &self.islands {
            if island.mode == IslandMode::Expanded {
                let card_count = island.cards.len().min(5) as f32;
                let pill_h = COMPACT_H;
                let pill_bottom = (BAR_HEIGHT - pill_h) / 2.0 + pill_h;
                let stack_h = pill_bottom
                    + renderer::CARD_GAP
                    + card_count * renderer::CARD_H
                    + (card_count - 1.0).max(0.0) * renderer::CARD_GAP;
                max_h = max_h.max(stack_h + 4.0);
            }
        }

        // A presented dialog panel drops below the bar and may extend the layer.
        if let Some(panel) = &self.dialog {
            max_h = max_h.max(DIALOG_TOP + panel.layout_h + 12.0);
        }

        // Compute the minimum width needed for all islands.
        let total_w: f32 = self
            .islands
            .iter()
            .map(|i| i.last_layout.0.max(MINI_H))
            .sum::<f32>()
            + (self.islands.len().saturating_sub(1)) as f32 * GAP;
        let mut needed_w = (total_w + 40.0).max(LAYER_W as f32); // padding + minimum
        if self.dialog.is_some() {
            needed_w = needed_w.max(dialog::DIALOG_W + 40.0);
        }

        let size = (needed_w.ceil() as u32, max_h.ceil() as u32);
        if self.last_layer_size == Some(size) {
            return false;
        }
        layer.set_size(size.0, size.1);
        self.last_layer_size = Some(size);
        true
    }

    fn update_input_region(&mut self, force_commit: bool) {
        let Some(layer) = &self.layer_surface else {
            return;
        };

        // Collect input rects when there are visible islands.
        // Empty region = zero input area (clicks pass through).
        let mut rects: Vec<(i32, i32, i32, i32)> = Vec::new();
        if !self.islands.is_empty() {
            // One rect per island, derived from last_layout (center coords).
            for island in &self.islands {
                let (w, _h, cx, _cy) = island.last_layout;
                let pill_h = match island.mode {
                    IslandMode::Mini => MINI_H,
                    IslandMode::Compact | IslandMode::Expanded => COMPACT_H,
                };
                let pill_w = match island.mode {
                    IslandMode::Expanded => w.max(renderer::CARD_W),
                    _ => w.max(MINI_H),
                };
                let x = cx - pill_w / 2.0;
                let y = (BAR_HEIGHT - pill_h) / 2.0;
                rects.push((
                    x.max(0.0) as i32,
                    y as i32,
                    pill_w.ceil() as i32,
                    pill_h.ceil() as i32,
                ));
            }

            // Card stack region — one rect per expanded island, positioned under its pill.
            for island in &self.islands {
                if island.mode != IslandMode::Expanded || island.cards.is_empty() {
                    continue;
                }
                let pill_w = island.last_layout.0;
                let pill_cx = island.last_layout.2;
                let pill_left = pill_cx - pill_w / 2.0;
                let pill_h = COMPACT_H;
                let pill_bottom = (BAR_HEIGHT - pill_h) / 2.0 + pill_h;
                let card_w = renderer::CARD_W;
                let card_h = renderer::CARD_H;
                let card_gap = renderer::CARD_GAP;
                let card_count = island.cards.len() as f32;
                let stack_top = pill_bottom + card_gap;
                let stack_h = card_count * card_h + (card_count - 1.0) * card_gap;
                let card_region_x = pill_left + (pill_w - card_w) / 2.0;
                rects.push((
                    card_region_x.max(0.0) as i32,
                    stack_top as i32,
                    card_w.ceil() as i32,
                    stack_h.ceil() as i32,
                ));
            }
        }

        // A dialog panel captures input over its own rect. A modal dialog
        // additionally captures the whole layer so clicks can't fall through to
        // windows behind while a decision is pending.
        if let Some(panel) = &self.dialog {
            if panel.view.modal {
                if let Some((lw, lh)) = self.last_layer_size {
                    rects.push((0, 0, lw as i32, lh as i32));
                }
            } else {
                let (ox, oy) = panel.origin;
                rects.push((
                    ox.max(0.0) as i32,
                    oy.max(0.0) as i32,
                    dialog::DIALOG_W.ceil() as i32,
                    panel.layout_h.ceil() as i32,
                ));
            }
        }

        let region_changed = self.last_input_region.as_ref() != Some(&rects);
        if !region_changed && !force_commit {
            return;
        }

        let wl_surface = layer.base_surface().wl_surface();
        if region_changed {
            let cs = AppContext::compositor_state();
            let Ok(region) = Region::new(cs) else { return };
            for &(x, y, w, h) in &rects {
                region.add(x, y, w, h);
            }
            wl_surface.set_input_region(Some(region.wl_region()));
            self.last_input_region = Some(rects);
        }
        wl_surface.commit();
    }

    // -----------------------------------------------------------------------
    // Hit testing
    // -----------------------------------------------------------------------

    /// Returns (app_id, Option<activity_id>) for what's at (px, py).
    /// activity_id is Some when a card is hit.
    fn hit_test(&self, px: f32, py: f32) -> Option<(String, Option<u64>)> {
        for island in &self.islands {
            let (w, _h, cx, _cy) = island.last_layout;
            let pill_h = match island.mode {
                IslandMode::Mini => MINI_H,
                IslandMode::Compact | IslandMode::Expanded => COMPACT_H,
            };
            let pill_w = match island.mode {
                IslandMode::Expanded => w.max(renderer::CARD_W),
                _ => w.max(MINI_H),
            };
            let x = cx - pill_w / 2.0;
            let y = (BAR_HEIGHT - pill_h) / 2.0;

            // Hit test cards first (they sit below the pill).
            if island.mode == IslandMode::Expanded {
                let card_w = renderer::CARD_W;
                let card_h = renderer::CARD_H;
                let card_gap = renderer::CARD_GAP;
                let card_x = x + (pill_w - card_w) / 2.0;

                for (i, card) in island.cards.iter().enumerate() {
                    let card_y = y + pill_h + card_gap + (i as f32) * (card_h + card_gap);
                    if px >= card_x
                        && px <= card_x + card_w
                        && py >= card_y
                        && py <= card_y + card_h
                    {
                        return Some((island.app_id.clone(), Some(card.activity_id)));
                    }
                }
            }

            // Hit test pill/circle.
            if px >= x && px <= x + pill_w && py >= y && py <= y + pill_h {
                return Some((island.app_id.clone(), None));
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // Click handling
    // -----------------------------------------------------------------------

    fn handle_click(&mut self, px: f32, py: f32) {
        // A dialog is modal — it consumes all clicks while present.
        if self.handle_dialog_click(px, py) {
            return;
        }
        let Some((app_id, card_id)) = self.hit_test(px, py) else {
            return;
        };

        if let Some(activity_id) = card_id {
            // Determine if the click is in the close zone (right 40px of card).
            let close_zone = 40.0_f32;
            let is_close = self
                .islands
                .iter()
                .find(|i| i.app_id == app_id)
                .map(|island| {
                    let pill_w = island.last_layout.0;
                    let pill_cx = island.last_layout.2;
                    let card_w = renderer::CARD_W;
                    let pill_x = pill_cx - pill_w / 2.0;
                    let card_x = pill_x + (pill_w - card_w) / 2.0;
                    px - card_x > card_w - close_zone
                })
                .unwrap_or(false);

            // Clicked a card — animate dismiss (scale up + fade out), then remove.
            if let Some(island) = self.islands.iter().find(|i| i.app_id == app_id) {
                if let Some(card) = island.cards.iter().find(|c| c.activity_id == activity_id) {
                    renderer::animate_dismiss(&card.surface, 1.2);
                }
            }

            let mut state = self.state.lock().unwrap();
            let notification_id = state
                .activities
                .iter()
                .find(|a| a.id == activity_id)
                .and_then(|a| a.notification_id);
            let default_action = state
                .activities
                .iter()
                .find(|a| a.id == activity_id)
                .and_then(|a| a.default_action.clone());
            if let Some(activity) = state.activities.iter().find(|a| a.id == activity_id) {
                tracing::info!(
                    activity_id,
                    %app_id,
                    close = is_close,
                    action = ?activity.default_action,
                    "card clicked"
                );
            }

            state.dismiss_activity(activity_id);
            drop(state);

            if !is_close {
                // Action click — focus the app and emit ActionInvoked.
                request_focus_app(app_id.clone());

                if let Some(nid) = notification_id {
                    let action_key = default_action.as_deref().unwrap_or("default").to_string();
                    emit_action_invoked(nid, action_key);
                }
            }
        } else {
            // Clicked a pill/circle.
            // Close any other expanded island first — only one can be expanded at a time.
            for island in self.islands.iter_mut().filter(|i| i.app_id != app_id) {
                if island.mode == IslandMode::Expanded {
                    Self::close_cards_for(island);
                    island.mode = IslandMode::Compact;
                    island.last_layout = (0.0, 0.0, 0.0, 0.0);
                }
            }
            let island = self.islands.iter_mut().find(|i| i.app_id == app_id);
            if let Some(island) = island {
                match island.mode {
                    IslandMode::Mini | IslandMode::Compact => {
                        tracing::info!(%app_id, from = ?island.mode, "click: → Expanded");
                        self.focused_app = Some(app_id.clone());
                        self.last_interaction = std::time::Instant::now();
                        island.mode = IslandMode::Expanded;
                        island.peek_until = None;
                    }
                    IslandMode::Expanded => {
                        tracing::info!(%app_id, "click: Expanded → Compact");
                        Self::close_cards_for(island);
                        island.mode = IslandMode::Compact;
                        island.last_layout = (0.0, 0.0, 0.0, 0.0);
                        // Keep focus so timeout governs Mini transition.
                        self.focused_app = Some(app_id.clone());
                        self.last_interaction = std::time::Instant::now();
                    }
                }
            }
            // Mark dirty so sync() runs.
            let mut state = self.state.lock().unwrap();
            state.dirty = true;
        }
    }

    // -----------------------------------------------------------------------
    // Access-style dialogs
    // -----------------------------------------------------------------------

    /// Reconcile the presented dialog panel with the front of the dialog queue.
    fn sync_dialog(&mut self) {
        let front = {
            let mut state = self.state.lock().unwrap();
            state.prune_withdrawn_dialogs();
            state.front_dialog_view()
        };

        match (front, self.dialog.as_ref().map(|p| p.id)) {
            // Same dialog already presented — just (re)render below.
            (Some(view), Some(cur)) if view.id == cur => {
                // Update view in case labels changed; keep selection/anim state.
                if let Some(panel) = self.dialog.as_mut() {
                    panel.view = view;
                }
            }
            // A different (or first) dialog — replace any existing panel.
            (Some(view), _) => {
                if let Some(old) = self.dialog.take() {
                    self.animate_dialog_out(old);
                }
                if let Some(panel) = self.create_dialog_panel(view) {
                    self.dialog = Some(panel);
                }
            }
            // Queue drained — dismiss the panel.
            (None, Some(_)) => {
                if let Some(old) = self.dialog.take() {
                    self.animate_dialog_out(old);
                }
            }
            (None, None) => {}
        }

        self.render_dialog();
        let size_changed = self.update_layer_size();
        self.update_input_region(size_changed);
    }

    fn create_dialog_panel(&self, view: DialogView) -> Option<DialogPanel> {
        let wl = self.wl_surface()?;
        let surface =
            SubsurfaceSurface::new(&wl, 0, 0, dialog::DIALOG_BUF_W, dialog::DIALOG_BUF_H).ok()?;
        dialog::apply_dialog_style(&surface);
        // Keep the panel above the island pills.
        for island in &self.islands {
            surface.place_above(island.surface.wl_surface());
        }
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });
        let selected: Vec<usize> = view.choices.iter().map(|g| g.default).collect();
        tracing::info!(id = view.id, app_id = %view.app_id, title = %view.title, "dialog shown");
        Some(DialogPanel {
            id: view.id,
            surface,
            view,
            selected,
            origin: (0.0, 0.0),
            layout_h: 0.0,
            entered: false,
        })
    }

    /// Draw and position the active dialog panel.
    fn render_dialog(&mut self) {
        let layer_w = self.layer_width();
        let Some(panel) = self.dialog.as_mut() else {
            return;
        };
        let layout = dialog::dialog_layout(&panel.view);
        let w = layout.width;
        let h = layout.height;

        let selected = panel.selected.clone();
        let view = panel.view.clone();
        panel.surface.draw(|canvas| {
            dialog::draw_dialog(canvas, &view, &selected, &layout);
        });

        let cx = layer_w / 2.0;
        let cy = DIALOG_TOP + h / 2.0;
        panel.origin = (cx - w / 2.0, DIALOG_TOP);
        panel.layout_h = h;

        if !panel.entered {
            // Start slightly above and transparent, then spring in.
            set_size_and_position(&panel.surface, w, h, cx, cy - 12.0);
            if let Some(ss) = panel.surface.base_surface().surface_style() {
                ss.set_opacity(0.0);
            }
            renderer::animate_to_with_opacity(
                &panel.surface,
                w,
                h,
                cx,
                cy,
                dialog::PANEL_RADIUS as f64,
                Some(1.0),
                0.0,
            );
            panel.entered = true;
        } else {
            set_size_and_position(&panel.surface, w, h, cx, cy);
        }
    }

    fn animate_dialog_out(&mut self, panel: DialogPanel) {
        renderer::animate_dismiss(&panel.surface, 0.96);
        self.defer_destroy(panel.surface);
    }

    /// Route a click to the active dialog. Returns true if a dialog consumed it.
    fn handle_dialog_click(&mut self, px: f32, py: f32) -> bool {
        let Some(panel) = self.dialog.as_ref() else {
            return false;
        };
        let (ox, oy) = panel.origin;
        let layout = dialog::dialog_layout(&panel.view);
        match dialog::hit_test(&layout, px - ox, py - oy) {
            Some(DialogHit::Option { group, option }) => {
                if let Some(panel) = self.dialog.as_mut() {
                    if let Some(sel) = panel.selected.get_mut(group) {
                        *sel = option;
                    }
                }
                self.render_dialog();
            }
            Some(DialogHit::Grant) => self.resolve_active_dialog(0),
            Some(DialogHit::Deny) => self.resolve_active_dialog(1),
            // Click landed on the panel background — swallow it (modal).
            None => {}
        }
        true
    }

    /// Deliver a decision for the active dialog and let the next tick dismiss it.
    fn resolve_active_dialog(&mut self, response: u32) {
        let Some(panel) = self.dialog.as_ref() else {
            return;
        };
        let results: Vec<(String, String)> = if response == 0 {
            panel
                .view
                .choices
                .iter()
                .enumerate()
                .filter_map(|(gi, g)| {
                    let idx = panel.selected.get(gi).copied().unwrap_or(g.default);
                    g.options.get(idx).map(|o| (g.id.clone(), o.id.clone()))
                })
                .collect()
        } else {
            Vec::new()
        };
        let id = panel.id;
        tracing::info!(id, response, "dialog resolved");
        let mut state = self.state.lock().unwrap();
        state.resolve_dialog(id, DialogResponse { response, results });
    }
}

// ---------------------------------------------------------------------------
// App trait implementation
// ---------------------------------------------------------------------------

impl App for IslandApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let layer_surface =
            LayerShellSurface::new(Layer::Overlay, "otto-islands", LAYER_W, LAYER_H)?;
        layer_surface.set_anchor(Anchor::Top);
        layer_surface.set_margin(2, 0, 0, 0);
        layer_surface.set_exclusive_zone(0);
        layer_surface.set_keyboard_interactivity(
            wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        );

        if let Some(style) = layer_surface.base_surface().surface_style() {
            style.set_masks_to_bounds(ClipMode::Enabled);
        }

        self.layer_surface = Some(layer_surface);
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, _w: i32, _h: i32, _serial: u32) {
        if !self.surfaces_ready {
            // Clear the parent surface.
            if let Some(layer) = &self.layer_surface {
                layer.draw(|canvas| {
                    canvas.clear(skia_safe::Color::TRANSPARENT);
                });
            }
            self.surfaces_ready = true;
            // Set empty input region so clicks pass through until islands appear.
            self.update_input_region(false);
        }
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        if !self.surfaces_ready {
            return;
        }

        // Destroy surfaces whose animations have completed.
        self.flush_pending_destroy();

        // Focus timeout: shrink Compact → Mini after inactivity.
        // Pause the timer while the pointer is hovering over any island.
        if self.hovered_app.is_some() {
            self.last_interaction = std::time::Instant::now();
        }
        let elapsed = self.last_interaction.elapsed().as_secs_f64();
        if elapsed >= FOCUS_TIMEOUT_SECS && self.focused_app.is_some() {
            let any_expanded = self.islands.iter().any(|i| i.mode == IslandMode::Expanded);
            if !any_expanded {
                tracing::info!(
                    focused = ?self.focused_app,
                    elapsed_secs = format!("{:.1}", elapsed),
                    "focus timeout → all Mini"
                );
                self.focused_app = None;
                let mut state = self.state.lock().unwrap();
                state.dirty = true;
                drop(state);
            }
        }

        let now = std::time::Instant::now();

        // Peek timeout: revert Compact peek back to Mini.
        for island in &mut self.islands {
            if let Some(until) = island.peek_until {
                if now >= until {
                    tracing::info!(app_id = %island.app_id, "peek expired → Mini");
                    island.peek_until = None;
                    island.mode = IslandMode::Mini;
                    island.last_layout = (0.0, 0.0, 0.0, 0.0);
                    // Snapshot current state so the next sync doesn't re-trigger peek.
                    let state = self.state.lock().unwrap();
                    let grouped = state.grouped_activities();
                    drop(state);
                    if let Some((a, c)) = grouped.iter().find(|(a, _)| a.app_id == island.app_id) {
                        island.last_count = *c;
                        island.last_activity_id = a.id;
                    }
                    let mut state = self.state.lock().unwrap();
                    state.dirty = true;
                    drop(state);
                }
            }
        }

        // Poll for withdrawn dialogs (caller aborted the request). This marks
        // state dirty when one is pruned so the panel is dismissed below.
        if self.dialog.is_some() {
            let mut state = self.state.lock().unwrap();
            state.prune_withdrawn_dialogs();
            drop(state);
        }

        let mut state = self.state.lock().unwrap();
        state.check_expired_refocus();

        let dirty = state.dirty;
        if dirty {
            state.dirty = false;
        }
        drop(state);

        if dirty {
            self.sync();
            self.sync_dialog();
        }
    }

    /// Wake only for the earliest pending deadline; block indefinitely when idle.
    /// D-Bus events wake the loop via `AppContext::request_wakeup()`, pointer and
    /// configure events via the Wayland fd — no periodic polling needed.
    fn idle_timeout(&self) -> Option<Duration> {
        let now = std::time::Instant::now();
        let mut deadlines: Vec<std::time::Instant> = Vec::new();

        for (_, queued_at) in &self.pending_destroy {
            deadlines.push(*queued_at + Duration::from_secs_f64(DESTROY_DELAY_SECS));
        }
        for island in &self.islands {
            if let Some(until) = island.peek_until {
                deadlines.push(until);
            }
        }
        // While a dialog is up, poll periodically to detect caller withdrawal
        // (the D-Bus method future being dropped closes the response channel).
        if self.dialog.is_some() {
            deadlines.push(now + Duration::from_millis(500));
        }
        // Focus timeout only counts down when it can actually fire (see on_update).
        if self.focused_app.is_some()
            && !self.islands.iter().any(|i| i.mode == IslandMode::Expanded)
        {
            deadlines.push(self.last_interaction + Duration::from_secs_f64(FOCUS_TIMEOUT_SECS));
        }
        if let Ok(state) = self.state.lock() {
            // Dirty work queued during this iteration (e.g. pulse relayout) —
            // re-enter on_update immediately.
            if state.dirty {
                return Some(Duration::ZERO);
            }
            for a in &state.activities {
                if a.timeout_ms > 0 && !a.expired {
                    deadlines.push(a.created_at + Duration::from_millis(a.timeout_ms as u64));
                }
            }
        }

        // +1ms so poll's millisecond truncation can't wake us just before the
        // deadline and spin.
        deadlines
            .into_iter()
            .min()
            .map(|d| d.saturating_duration_since(now) + Duration::from_millis(1))
    }

    fn on_keyboard_event(&mut self, _ctx: &AppContext, key: u32, state: KeyState, _serial: u32) {
        if state != KeyState::Pressed || self.dialog.is_none() {
            return;
        }
        // evdev keycodes: ESC = 1, ENTER = 28.
        match key {
            1 => self.resolve_active_dialog(1),
            28 => self.resolve_active_dialog(0),
            _ => {}
        }
    }

    fn on_keyboard_leave(
        &mut self,
        _ctx: &AppContext,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
    ) {
        // Close expanded stack on focus loss — animate cards out first.
        let mut changed = false;
        for island in &mut self.islands {
            if island.mode == IslandMode::Expanded {
                Self::close_cards_for(island);
                island.mode = IslandMode::Compact;
                island.last_layout = (0.0, 0.0, 0.0, 0.0);
                changed = true;
            }
        }
        // Restart the focus timeout from now.
        self.last_interaction = std::time::Instant::now();
        if changed {
            let mut state = self.state.lock().unwrap();
            state.dirty = true;
        }
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        self.last_interaction = std::time::Instant::now();
        for event in events {
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let (px, py) = event.position;
                    let hit = self.hit_test(px as f32, py as f32);
                    let new_hovered = hit.as_ref().map(|(app_id, _)| app_id.clone());
                    if new_hovered != self.hovered_app {
                        let old = &self.hovered_app;
                        // Relayout when a Mini or Compact island gains/loses hover (for grow effect).
                        let has_hover_grow = |app: &Option<String>| -> bool {
                            app.as_ref()
                                .and_then(|a| self.islands.iter().find(|i| i.app_id == *a))
                                .is_some_and(|i| {
                                    i.mode == IslandMode::Mini || i.mode == IslandMode::Compact
                                })
                        };
                        let needs_relayout = has_hover_grow(old) || has_hover_grow(&new_hovered);
                        self.hovered_app = new_hovered;
                        if needs_relayout {
                            let mut state = self.state.lock().unwrap();
                            state.dirty = true;
                        }
                    }
                    if hit.is_some() {
                        AppContext::set_cursor_shape(otto_kit::CursorShape::Pointer);
                    } else {
                        AppContext::set_cursor_shape(otto_kit::CursorShape::Default);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.hovered_app.is_some() {
                        self.hovered_app = None;
                        let mut state = self.state.lock().unwrap();
                        state.dirty = true;
                    }
                    AppContext::set_cursor_shape(otto_kit::CursorShape::Default);
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    let (px, py) = event.position;
                    self.handle_click(px as f32, py as f32);
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// D-Bus helpers
// ---------------------------------------------------------------------------

/// The D-Bus connection that owns the org.otto.Island bus name.
/// Signals must be emitted from this connection so receivers matching on
/// sender="org.otto.Island" can see them.
static ISLAND_DBUS_CONNECTION: std::sync::OnceLock<zbus::Connection> = std::sync::OnceLock::new();

/// Ask the compositor to focus the given app's window via D-Bus.
fn request_focus_app(app_id: String) {
    tokio::spawn(async move {
        let connection = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to connect to session bus for focus_app: {e}");
                return;
            }
        };
        let reply = connection
            .call_method(
                Some("org.otto.Compositor"),
                "/org/otto/Compositor",
                Some("org.otto.Compositor"),
                "FocusApp",
                &(app_id.as_str(),),
            )
            .await;
        if let Err(e) = reply {
            tracing::warn!(app_id, "focus_app D-Bus call failed: {e}");
        }
    });
}

/// Emit the org.freedesktop.Notifications ActionInvoked signal.
fn emit_action_invoked(notification_id: u32, action_key: String) {
    tokio::spawn(async move {
        let connection = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to connect to session bus for ActionInvoked: {e}");
                return;
            }
        };
        let result = connection
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "ActionInvoked",
                &(notification_id, action_key.as_str()),
            )
            .await;
        if let Err(e) = result {
            tracing::warn!(notification_id, "ActionInvoked signal failed: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let state: SharedState = Arc::new(Mutex::new(IslandState::new()));

    // Spawn org.otto.Island1 + org.otto.Dialog1 D-Bus services
    let dbus_state = state.clone();
    let dialog_state = state.clone();
    tokio::spawn(async move {
        let service = IslandService::new(dbus_state);
        let dialog_service = DialogService::new(dialog_state);

        let connection = match zbus::ConnectionBuilder::session()
            .expect("session bus")
            .name(DBUS_NAME)
            .expect("claim D-Bus name")
            .build()
            .await
        {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("Failed to build D-Bus connection: {e}");
                return;
            }
        };

        if let Err(e) = connection
            .object_server()
            .at(dbus_service::DBUS_PATH, service)
            .await
        {
            tracing::error!("Failed to register D-Bus object: {e}");
            return;
        }

        if let Err(e) = connection
            .object_server()
            .at(dbus_service::DIALOG_DBUS_PATH, dialog_service)
            .await
        {
            tracing::error!("Failed to register Dialog D-Bus object: {e}");
            return;
        }

        let _ = ISLAND_DBUS_CONNECTION.set(connection);
        tracing::info!("D-Bus service running on {DBUS_NAME}");
        std::future::pending::<()>().await;
    });

    // Spawn org.freedesktop.Notifications daemon
    let notif_state = state.clone();
    tokio::spawn(async move {
        let daemon = notifications::NotificationDaemon::new(notif_state);

        let connection = match zbus::ConnectionBuilder::session()
            .expect("session bus")
            .name(notifications::NOTIFICATIONS_DBUS_NAME)
            .expect("claim notifications name")
            .build()
            .await
        {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("Failed to build notifications D-Bus connection: {e}");
                return;
            }
        };

        if let Err(e) = connection
            .object_server()
            .at(notifications::NOTIFICATIONS_DBUS_PATH, daemon)
            .await
        {
            tracing::error!("Failed to register notifications object: {e}");
            return;
        }

        tracing::info!(
            "Notifications daemon running on {}",
            notifications::NOTIFICATIONS_DBUS_NAME
        );
        std::future::pending::<()>().await;
    });

    let app = IslandApp::new(state);
    AppRunner::new(app).run()?;

    Ok(())
}
