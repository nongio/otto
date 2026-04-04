mod activity;
mod dbus_service;
mod music;
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
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
};

use crate::activity::{Activity, ActivitySource, PresentationMode};
use crate::dbus_service::{IslandService, DBUS_NAME};
use crate::music::MusicMonitor;
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
/// Seconds of inactivity before the focused island shrinks to Mini.
const FOCUS_TIMEOUT_SECS: f64 = 4.0;

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
    Music,
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
    /// EQ overlay subsurface (child of pill, music islands only).
    eq_surface: Option<SubsurfaceSurface>,
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
}

struct CardSurface {
    surface: SubsurfaceSurface,
    activity_id: u64,
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
    /// Music monitor — tracks MPRIS playback and PipeWire audio levels.
    music_monitor: MusicMonitor,
    /// Currently pressed music control (for visual feedback).
    music_pressed: Option<(music::MusicAction, std::time::Instant)>,
    /// Last time the music EQ was redrawn (throttle to ~24fps).
    music_last_redraw: std::time::Instant,
    /// Last time the full music pill was redrawn (progress bar, ~1fps).
    music_last_full_redraw: std::time::Instant,
}

impl IslandApp {
    fn new(state: SharedState, music_monitor: MusicMonitor) -> Self {
        Self {
            state,
            layer_surface: None,
            islands: Vec::new(),
            surfaces_ready: false,
            focused_app: None,
            hovered_app: None,
            pending_destroy: Vec::new(),
            last_interaction: std::time::Instant::now(),
            music_monitor,
            music_pressed: None,
            music_last_redraw: std::time::Instant::now(),
            music_last_full_redraw: std::time::Instant::now(),
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

    /// Destroy surfaces whose animations have had time to complete (>0.8s).
    fn flush_pending_destroy(&mut self) {
        let cutoff = std::time::Instant::now() - Duration::from_secs_f64(0.8);
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
            let kind = if activity.app_id == "org.otto.music"
                && activity.source == ActivitySource::Internal
            {
                IslandKind::Music
            } else {
                IslandKind::Notification
            };
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
                    // Create EQ child subsurface for music islands.
                    // Buffer sized for the largest EQ mode (expanded ~210×22).
                    let eq_surface = if *kind == IslandKind::Music {
                        let eq = SubsurfaceSurface::new(surface.wl_surface(), 0, 0, 220, 32).ok();
                        if let Some(ref eq) = eq {
                            if let Some(ss) = eq.base_surface().surface_style() {
                                ss.set_contents_gravity(ContentsGravity::Center);
                                ss.set_anchor_point(0.5, 0.5);
                            }
                            eq.place_above(surface.wl_surface());
                        }
                        eq
                    } else {
                        None
                    };
                    self.islands.push(Island {
                        app_id: app_id.clone(),
                        kind: *kind,
                        icon: icon.clone(),
                        surface,
                        eq_surface,
                        cards: Vec::new(),
                        mode: IslandMode::Mini,
                        created_at: std::time::Instant::now(),
                        last_count: 0,
                        last_activity_id: 0,
                        peek_until: None,
                        last_layout: (0.0, 0.0, 0.0, 0.0),
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
        // Music islands stay Compact unless another island is Expanded.
        let any_expanded = self.islands.iter().any(|i| i.mode == IslandMode::Expanded);
        for island in &mut self.islands {
            if island.mode == IslandMode::Expanded {
                // Expanded stays Expanded — only user interaction (click/focus loss) closes it.
            } else if island.kind == IslandKind::Music {
                // Music stays Compact unless another island is Expanded.
                if any_expanded {
                    if island.mode != IslandMode::Mini {
                        island.mode = IslandMode::Mini;
                    }
                } else if island.mode == IslandMode::Mini {
                    island.mode = IslandMode::Compact;
                }
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
            self.update_layer_size();
            self.update_input_region();
            return;
        }

        // Compute element sizes for layout.
        let island_size = |island: &Island, mode: IslandMode| -> (f32, f32) {
            if island.kind == IslandKind::Music {
                // Use MusicActivityRenderer sizes via the ActivityRenderer trait.
                return match mode {
                    IslandMode::Mini => {
                        use crate::activity::ActivityRenderer;
                        let dummy = music::MusicActivityRenderer {
                            title: String::new(),
                            artist: String::new(),
                            album_art: None,
                            is_playing: false,
                            progress: 0.0,
                            duration_secs: 0.0,
                            accent: skia_safe::Color::TRANSPARENT,
                            levels: [0.0; 8],
                            pressed: None,
                        };
                        dummy.size(PresentationMode::Minimal)
                    }
                    IslandMode::Compact => {
                        use crate::activity::ActivityRenderer;
                        let dummy = music::MusicActivityRenderer {
                            title: String::new(),
                            artist: String::new(),
                            album_art: None,
                            is_playing: false,
                            progress: 0.0,
                            duration_secs: 0.0,
                            accent: skia_safe::Color::TRANSPARENT,
                            levels: [0.0; 8],
                            pressed: None,
                        };
                        dummy.size(PresentationMode::Compact)
                    }
                    IslandMode::Expanded => {
                        use crate::activity::ActivityRenderer;
                        let dummy = music::MusicActivityRenderer {
                            title: String::new(),
                            artist: String::new(),
                            album_art: None,
                            is_playing: false,
                            progress: 0.0,
                            duration_secs: 0.0,
                            accent: skia_safe::Color::TRANSPARENT,
                            levels: [0.0; 8],
                            pressed: None,
                        };
                        dummy.size(PresentationMode::Expanded)
                    }
                };
            }
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
            // Music expanded: top-aligned (grow downward from pill top edge).
            let cy = if island.kind == IslandKind::Music && island.mode == IslandMode::Expanded {
                let pill_top = (BAR_HEIGHT - COMPACT_H) / 2.0;
                pill_top + h / 2.0
            } else {
                BAR_HEIGHT / 2.0
            };

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

            if island.kind == IslandKind::Music {
                // Music islands use MusicActivityRenderer for all modes.
                let pmode = match island.mode {
                    IslandMode::Mini => PresentationMode::Minimal,
                    IslandMode::Compact => PresentationMode::Compact,
                    IslandMode::Expanded => PresentationMode::Expanded,
                };
                if let Some(mut mr) = self.music_monitor.renderer() {
                    // Apply pressed state for visual feedback.
                    if let Some((action, instant)) = &self.music_pressed {
                        if instant.elapsed().as_millis() < 300 {
                            mr.pressed = Some(*action);
                        }
                    }
                    draw_centered(&island.surface, w, h, |canvas| {
                        mr.draw_without_eq(canvas, pmode, w, h);
                    });
                    // Position and resize the EQ child subsurface.
                    // EQ is a child of the pill — position is relative to pill origin.
                    if let Some(eq_surf) = &island.eq_surface {
                        let (eq_w, eq_h, eq_ox, eq_oy) = mr.eq_layout(pmode, w, h);
                        let eq_cx = eq_ox + eq_w / 2.0;
                        let eq_cy = eq_oy + eq_h / 2.0;
                        renderer::set_size_and_position(eq_surf, eq_w, eq_h, eq_cx, eq_cy);
                    }
                    self.music_last_redraw = std::time::Instant::now();
                }
                layout_targets.push((idx, w, h, cx, cy));
            } else {
                match island.mode {
                    IslandMode::Mini => {
                        draw_centered(&island.surface, w, h, |canvas| {
                            renderer::draw_mini(canvas, icon, count, w, h);
                        });
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
            }

            x += w + GAP;
        }

        // Apply layout animations only when target changed.
        let layout_delay = if reposition_delay { 0.4 } else { 0.0 };
        for (idx, w, h, x, y) in layout_targets {
            let target = (w, h, x, y);
            if self.islands[idx].last_layout != target {
                let radius = if self.islands[idx].kind == IslandKind::Music
                    && self.islands[idx].mode == IslandMode::Expanded
                {
                    16.0
                } else {
                    h as f64 / 2.0
                };
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
                    });
                    island.cards.len() - 1
                };

                // Redraw content for existing cards (count/time may have changed).
                draw_centered(&island.cards[cidx].surface, card_w, card_h, |canvas| {
                    renderer::draw_card(canvas, notif, group_icon, card_w, card_h);
                });

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

        self.update_layer_size();
        self.update_input_region();
    }

    // -----------------------------------------------------------------------
    // Layer size & input region
    // -----------------------------------------------------------------------

    fn update_layer_size(&self) {
        let Some(layer) = &self.layer_surface else {
            return;
        };

        // Compute the minimum height needed for current layout.
        let mut max_h = BAR_HEIGHT;

        for island in &self.islands {
            if island.mode == IslandMode::Expanded {
                if island.kind == IslandKind::Music {
                    // Music expanded: top-aligned, full height from last_layout.
                    let pill_top = (BAR_HEIGHT - COMPACT_H) / 2.0;
                    let h = island.last_layout.1;
                    max_h = max_h.max(pill_top + h + 4.0);
                } else {
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
        }

        // Compute the minimum width needed for all islands.
        let total_w: f32 = self
            .islands
            .iter()
            .map(|i| i.last_layout.0.max(MINI_H))
            .sum::<f32>()
            + (self.islands.len().saturating_sub(1)) as f32 * GAP;
        let needed_w = (total_w + 40.0).max(LAYER_W as f32); // padding + minimum

        layer.set_size(needed_w.ceil() as u32, max_h.ceil() as u32);
    }

    fn update_input_region(&self) {
        let Some(layer) = &self.layer_surface else {
            return;
        };
        let cs = AppContext::compositor_state();
        let Ok(region) = Region::new(cs) else { return };

        // Add input rects when there are visible islands.
        // Empty region = zero input area (clicks pass through).
        if !self.islands.is_empty() {
            // One rect per island, derived from last_layout (center coords).
            for island in &self.islands {
                let (w, h, cx, cy) = island.last_layout;
                let (pill_w, pill_h) =
                    if island.kind == IslandKind::Music && island.mode == IslandMode::Expanded {
                        // Music expanded: full surface size, top-aligned.
                        (w.max(MINI_H), h)
                    } else {
                        let ph = match island.mode {
                            IslandMode::Mini => MINI_H,
                            IslandMode::Compact | IslandMode::Expanded => COMPACT_H,
                        };
                        let pw = match island.mode {
                            IslandMode::Expanded => w.max(renderer::CARD_W),
                            _ => w.max(MINI_H),
                        };
                        (pw, ph)
                    };
                let x = cx - pill_w / 2.0;
                let y = cy - pill_h / 2.0;
                region.add(
                    x.max(0.0) as i32,
                    y.max(0.0) as i32,
                    pill_w.ceil() as i32,
                    pill_h.ceil() as i32,
                );
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
                region.add(
                    card_region_x.max(0.0) as i32,
                    stack_top as i32,
                    card_w.ceil() as i32,
                    stack_h.ceil() as i32,
                );
            }
        }

        let wl_surface = layer.base_surface().wl_surface();
        wl_surface.set_input_region(Some(region.wl_region()));
        wl_surface.commit();
    }

    // -----------------------------------------------------------------------
    // Hit testing
    // -----------------------------------------------------------------------

    /// Returns (app_id, Option<activity_id>) for what's at (px, py).
    /// activity_id is Some when a card is hit.
    fn hit_test(&self, px: f32, py: f32) -> Option<(String, Option<u64>)> {
        for island in &self.islands {
            let (w, h, cx, cy) = island.last_layout;
            let (pill_w, pill_h) =
                if island.kind == IslandKind::Music && island.mode == IslandMode::Expanded {
                    (w.max(MINI_H), h)
                } else {
                    let ph = match island.mode {
                        IslandMode::Mini => MINI_H,
                        IslandMode::Compact | IslandMode::Expanded => COMPACT_H,
                    };
                    let pw = match island.mode {
                        IslandMode::Expanded => w.max(renderer::CARD_W),
                        _ => w.max(MINI_H),
                    };
                    (pw, ph)
                };
            let x = cx - pill_w / 2.0;
            let y = cy - pill_h / 2.0;

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
            // Check if this is a music island — handle music controls.
            let is_music = self
                .islands
                .iter()
                .find(|i| i.app_id == app_id)
                .is_some_and(|i| i.kind == IslandKind::Music);

            if is_music {
                let island = self.islands.iter().find(|i| i.app_id == app_id);
                if let Some(island) = island {
                    if island.mode == IslandMode::Expanded {
                        // Hit test music controls in expanded mode.
                        let (w, h, cx, cy) = island.last_layout;
                        let pill_x = cx - w / 2.0;
                        let pill_y = cy - h / 2.0;
                        let lx = px - pill_x;
                        let ly = py - pill_y;
                        if let Some(mr) = self.music_monitor.renderer() {
                            if let Some(action) = mr.hit_test_expanded(lx, ly, w, h) {
                                tracing::info!(?action, "music control hit");
                                self.music_pressed = Some((action, std::time::Instant::now()));
                                music::execute_action(action);
                                let mut state = self.state.lock().unwrap();
                                state.dirty = true;
                                return;
                            }
                        }
                    }

                    // For Mini → Compact, or Compact → Expanded transitions.
                    let island = self
                        .islands
                        .iter_mut()
                        .find(|i| i.app_id == app_id)
                        .unwrap();
                    match island.mode {
                        IslandMode::Mini => {
                            tracing::info!(%app_id, "music click: Mini → Compact");
                            // Close any expanded island so music can stay Compact.
                            for other in self.islands.iter_mut().filter(|i| i.app_id != app_id) {
                                if other.mode == IslandMode::Expanded {
                                    Self::close_cards_for(other);
                                    other.mode = IslandMode::Compact;
                                    other.last_layout = (0.0, 0.0, 0.0, 0.0);
                                }
                            }
                            let island = self
                                .islands
                                .iter_mut()
                                .find(|i| i.app_id == app_id)
                                .unwrap();
                            self.focused_app = Some(app_id.clone());
                            self.last_interaction = std::time::Instant::now();
                            island.mode = IslandMode::Compact;
                            island.peek_until = None;
                        }
                        IslandMode::Compact => {
                            tracing::info!(%app_id, "music click: Compact → Expanded");
                            self.focused_app = Some(app_id.clone());
                            self.last_interaction = std::time::Instant::now();
                            island.mode = IslandMode::Expanded;
                            island.peek_until = None;
                        }
                        IslandMode::Expanded => {
                            tracing::info!(%app_id, "music click: Expanded → Compact");
                            island.mode = IslandMode::Compact;
                            island.last_layout = (0.0, 0.0, 0.0, 0.0);
                            self.focused_app = Some(app_id.clone());
                            self.last_interaction = std::time::Instant::now();
                        }
                    }
                }
                let mut state = self.state.lock().unwrap();
                state.dirty = true;
                return;
            }

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
                layer.base_surface().on_frame(|| {});
            }
            self.surfaces_ready = true;
            // Set empty input region so clicks pass through until islands appear.
            self.update_input_region();
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

        // Sync music monitor — creates/updates/dismisses the music activity.
        self.music_monitor.sync_to_island(&self.state);

        // If music is playing, redraw music island surfaces (throttled to 1fps).
        let music_playing = self
            .music_monitor
            .playback
            .lock()
            .ok()
            .is_some_and(|info| info.is_playing);
        let eq_redraw_due = self.music_last_redraw.elapsed().as_millis() >= 42;
        if music_playing && eq_redraw_due {
            self.music_last_redraw = std::time::Instant::now();
            for island in &self.islands {
                if island.kind == IslandKind::Music {
                    if let Some(eq_surf) = &island.eq_surface {
                        let pmode = match island.mode {
                            IslandMode::Mini => PresentationMode::Minimal,
                            IslandMode::Compact => PresentationMode::Compact,
                            IslandMode::Expanded => PresentationMode::Expanded,
                        };
                        if let Some(mr) = self.music_monitor.renderer() {
                            let (w, h, _, _) = island.last_layout;
                            let (eq_w, eq_h, _, _) = mr.eq_layout(pmode, w, h);
                            // EQ buffer is 220×32 logical. Center the EQ content in it.
                            let buf_w = 220.0_f32;
                            let buf_h = 32.0_f32;
                            eq_surf.draw(|canvas| {
                                let tx = (buf_w - eq_w) / 2.0;
                                let ty = (buf_h - eq_h) / 2.0;
                                canvas.save();
                                canvas.translate((tx, ty));
                                mr.draw_eq_only(canvas, pmode, eq_w, eq_h);
                                canvas.restore();
                            });
                        }
                    }
                }
            }
        }

        // Full pill redraw every 1s when expanded (progress bar + time counter).
        if music_playing && self.music_last_full_redraw.elapsed().as_millis() >= 1000 {
            self.music_last_full_redraw = std::time::Instant::now();
            for island in &self.islands {
                if island.kind == IslandKind::Music && island.mode == IslandMode::Expanded {
                    if let Some(mut mr) = self.music_monitor.renderer() {
                        if let Some((action, instant)) = &self.music_pressed {
                            if instant.elapsed().as_millis() < 300 {
                                mr.pressed = Some(*action);
                            }
                        }
                        let (w, h, _, _) = island.last_layout;
                        if w > 0.0 && h > 0.0 {
                            draw_centered(&island.surface, w, h, |canvas| {
                                mr.draw_without_eq(canvas, PresentationMode::Expanded, w, h);
                            });
                        }
                    }
                }
            }
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
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        // Faster tick rate when music is playing for ~10fps equalizer animation.
        let music_playing = self
            .music_monitor
            .playback
            .lock()
            .ok()
            .is_some_and(|info| info.is_playing);
        if music_playing {
            Some(Duration::from_millis(42))
        } else {
            Some(Duration::from_millis(200))
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

    // Spawn org.otto.Island1 D-Bus service
    let dbus_state = state.clone();
    tokio::spawn(async move {
        let service = IslandService::new(dbus_state);

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

    let playback = music::start_playerctl_monitor();
    let audio_level = music::start_pipewire_level_monitor();
    let music_monitor = MusicMonitor::new(playback, audio_level);

    let app = IslandApp::new(state, music_monitor);
    AppRunner::new(app).run()?;

    Ok(())
}
