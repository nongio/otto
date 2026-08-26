mod activity;
mod dbus_service;
mod dialog;
mod dock_badges;
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
use crate::dock_badges::DockBadges;
use crate::renderer::{
    animate_to, apply_island_style, draw_content, set_size_and_position, COMPACT_H, MINI_H,
};
use crate::state::{IslandState, SharedState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const LAYER_W: u32 = 800;
const LAYER_H: u32 = 400; // Tall enough for an open notification.
const BAR_HEIGHT: f32 = 36.0;
const GAP: f32 = 6.0;
/// Top edge of a dialog panel, dropped down just below the island bar.
const DIALOG_TOP: f32 = BAR_HEIGHT + 14.0;
/// Seconds of inactivity before the focused island shrinks to Mini.
const FOCUS_TIMEOUT_SECS: f64 = 4.0;
/// Seconds a newly-arrived notification stays open, long enough to be read
/// before it settles back into its app's stack.
const ARRIVAL_READ_SECS: u64 = 6;
/// Seconds a destroyed surface is kept alive so its exit animation can play.
const DESTROY_DELAY_SECS: f64 = 0.8;
/// Per-island delay added as the push travels outward from whichever island
/// grew, so the row ripples instead of moving in lockstep.
const CASCADE_STAGGER_SECS: f64 = 0.035;
/// Width of the card's right-hand Close zone, used to hit-test a close click.
const CARD_CLOSE_ZONE: f32 = 40.0;

// ---------------------------------------------------------------------------
// Island — one notification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IslandMode {
    Mini,
    Compact,
    Expanded,
}

/// Signature of what an island buffer currently shows — redraw only on change.
#[derive(Clone, PartialEq)]
struct IslandContent {
    mode: IslandMode,
    icon: String,
    title: String,
    body: String,
    time_label: String,
    actions: Vec<crate::activity::NotificationAction>,
    w: f32,
    h: f32,
}

/// An island is one notification. There is no separate group header and no
/// separate card: the island *is* the notification, and expanding it grows
/// this same bubble into the full title/body/actions layout. Notifications
/// from the same app are grouped visually by overlapping their islands into
/// a peek stack, not by collapsing them into one representative.
struct Island {
    /// The notification this island represents.
    activity_id: u64,
    /// Grouping key — same-app islands overlap into one stack.
    app_id: String,
    icon: String,
    surface: SubsurfaceSurface,
    mode: IslandMode,
    /// When this notification arrived (newest sits at the front of its stack).
    created_at: std::time::Instant,
    /// When set, this is a fresh arrival announcing itself until this instant:
    /// it opens Expanded so it can be read, then settles into its stack.
    peek_until: Option<std::time::Instant>,
    /// Whether the user opened this island themselves. Only a user-opened
    /// island stays Expanded indefinitely; one that opened on arrival closes
    /// again when its window runs out.
    opened_by_user: bool,
    /// Last layout target (w, h, x, y) — skip animation when unchanged.
    last_layout: (f32, f32, f32, f32),
    /// Last drawn content — skip redraw when unchanged.
    last_content: Option<IslandContent>,
    /// Inline actions, cached for hit-testing without locking state.
    actions: Vec<crate::activity::NotificationAction>,
    /// Body text, cached alongside the actions: the action row sits under the
    /// wrapped body, so hit-testing has to know how many lines it took.
    body: String,
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
    /// Which island (by notification id) is currently focused (Compact/Expanded).
    focused_island: Option<u64>,
    /// Which island the pointer is over — it grows to the peek size.
    hovered_island: Option<u64>,
    /// Which group (by app_id) the pointer is over — that stack fans out.
    hovered_app: Option<String>,
    /// Last applied front-to-back order, so subsurfaces are only restacked
    /// when the order actually changes.
    last_stack_order: Option<Vec<u64>>,
    /// Width the last layout pass centered the island row within.
    last_content_width: f32,
    /// The island the last push radiated from — kept so the row cascades back
    /// in the same order when that island shrinks again.
    last_active_island: Option<u64>,
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
    /// Unread-notification counts published onto the dock icons.
    dock_badges: DockBadges,
}

impl IslandApp {
    fn new(state: SharedState) -> Self {
        Self {
            state,
            layer_surface: None,
            islands: Vec::new(),
            surfaces_ready: false,
            focused_island: None,
            hovered_island: None,
            hovered_app: None,
            last_stack_order: None,
            last_content_width: LAYER_W as f32,
            last_active_island: None,
            pending_destroy: Vec::new(),
            last_interaction: std::time::Instant::now(),
            last_layer_size: None,
            last_input_region: None,
            dialog: None,
            dock_badges: DockBadges::new(),
        }
    }

    /// The layer width the last layout pass centered its row within.
    fn layer_width(&self) -> f32 {
        self.last_content_width.max(LAYER_W as f32)
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
        apply_island_style(&surface, MINI_H as f64 / 2.0, ContentsGravity::TopLeft);
        // Center coordinates (anchor point is 0.5, 0.5).
        let cx = self.layer_width() / 2.0;
        let cy = BAR_HEIGHT / 2.0;
        set_size_and_position(&surface, MINI_H, MINI_H, cx, cy);
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
        let notifications: Vec<Activity> = state.activities.clone();
        drop(state);

        // The dock shows what is still waiting to be read, whether or not the
        // island for it is still on screen.
        self.dock_badges.sync(&notifications);

        // Remove islands whose notification is gone.
        let mut removed_island = false;
        let mut i = 0;
        while i < self.islands.len() {
            if notifications
                .iter()
                .any(|a| a.id == self.islands[i].activity_id)
            {
                i += 1;
            } else {
                let island = self.islands.remove(i);
                tracing::info!(app_id = %island.app_id, id = island.activity_id, "island removed");
                renderer::animate_dismiss(&island.surface, 1.2);
                self.defer_destroy(island.surface);
                removed_island = true;
            }
        }

        // Add an island per new notification.
        for activity in &notifications {
            if self.islands.iter().any(|i| i.activity_id == activity.id) {
                continue;
            }
            let Some(surface) = self.create_pill_subsurface() else {
                continue;
            };
            tracing::info!(app_id = %activity.app_id, id = activity.id, "island created");
            self.islands.push(Island {
                activity_id: activity.id,
                app_id: activity.app_id.clone(),
                icon: activity.icon.clone(),
                surface,
                mode: IslandMode::Mini,
                created_at: activity.created_at,
                // A new notification announces itself Expanded for a few
                // seconds, then settles back into its app's stack.
                peek_until: Some(
                    std::time::Instant::now() + Duration::from_secs(ARRIVAL_READ_SECS),
                ),
                last_layout: (0.0, 0.0, 0.0, 0.0),
                last_content: None,
                actions: activity.actions.clone(),
                body: activity.body.clone(),
                opened_by_user: false,
            });
            self.last_interaction = std::time::Instant::now();
        }

        // Refresh cached per-notification data.
        for island in &mut self.islands {
            if let Some(a) = notifications.iter().find(|a| a.id == island.activity_id) {
                island.actions = a.actions.clone();
                island.icon = a.icon.clone();
                island.body = a.body.clone();
            }
        }

        // If the focused island is gone, clear focus.
        if let Some(focused) = self.focused_island {
            if !self.islands.iter().any(|i| i.activity_id == focused) {
                self.focused_island = None;
            }
        }

        // The newest still-announcing arrival, which opens Expanded so it can
        // be read on sight.
        let arriving = self
            .islands
            .iter()
            .filter(|i| i.peek_until.is_some())
            .max_by_key(|i| i.created_at)
            .map(|i| i.activity_id);

        // An island the user opened themselves is never taken over: a new
        // arrival must not yank away whatever is being read right now, so it
        // announces itself Compact instead.
        let user_expanded = self
            .islands
            .iter()
            .any(|i| i.mode == IslandMode::Expanded && i.opened_by_user);
        let expand_id = if user_expanded { None } else { arriving };

        // Exactly one island may be Compact at a time. The pointer wins it,
        // then whatever was last clicked, then the arrival when it could not
        // open — so a burst of notifications doesn't blow the row up into a
        // wall of pills.
        let compact_id = self.hovered_island.or(self.focused_island).or(arriving);

        for island in &mut self.islands {
            let id = Some(island.activity_id);
            if island.mode == IslandMode::Expanded && island.opened_by_user {
                // User-opened: only user interaction (click / focus loss) closes it.
            } else if id == expand_id {
                island.mode = IslandMode::Expanded;
            } else if id == compact_id {
                island.mode = IslandMode::Compact;
            } else {
                island.mode = IslandMode::Mini;
            }
        }

        self.layout(&notifications, removed_island);
    }

    // -----------------------------------------------------------------------
    // Layout: position all islands and their cards
    // -----------------------------------------------------------------------

    /// Islands grouped by app, front-to-back within each group.
    ///
    /// Groups are ordered by their oldest notification so a group keeps its
    /// place in the row as notifications come and go. Within a group it is
    /// always newest-first: focusing or hovering an island grows it where it
    /// stands rather than pulling it to the front, so the deck never
    /// reshuffles under the pointer.
    fn group_order(&self) -> Vec<Vec<usize>> {
        // `self.islands` is already in arrival order, so nothing is sorted
        // here: groups appear in the order their first notification did, and
        // members in the order they arrived. Reversing a group's members just
        // expresses the same row front-to-back, since the newest bubble is the
        // rightmost one and sits on top.
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (idx, island) in self.islands.iter().enumerate() {
            match groups.iter_mut().find(|(app, _)| *app == island.app_id) {
                Some((_, members)) => members.push(idx),
                None => groups.push((island.app_id.clone(), vec![idx])),
            }
        }
        groups
            .into_iter()
            .map(|(_, mut members)| {
                members.reverse();
                members
            })
            .collect()
    }

    fn layout(&mut self, notifications: &[Activity], reposition_delay: bool) {
        if self.islands.is_empty() {
            let size_changed = self.update_layer_size();
            self.update_input_region(size_changed);
            return;
        }

        let title_of = |island: &Island| -> String {
            notifications
                .iter()
                .find(|a| a.id == island.activity_id)
                .map(|a| a.title.clone())
                .unwrap_or_default()
        };

        let groups = self.group_order();

        // Measure every island, then lay each group out as an overlapped stack.
        // (idx, offset within group, w, h)
        let mut placements: Vec<Vec<(usize, f32, f32, f32)>> = Vec::new();
        let mut group_widths: Vec<f32> = Vec::new();

        for members in &groups {
            let app_id = &self.islands[members[0]].app_id;
            let step = if self.hovered_app.as_deref() == Some(app_id.as_str()) {
                renderer::FAN_STEP
            } else {
                renderer::PEEK_STEP
            };

            // Measure every member first, then place them: the front of the
            // stack sits at the right-hand end of the group and the older
            // bubbles peek out to its left, behind it.
            let mut sizes: Vec<(usize, f32, f32)> = Vec::new();
            for &idx in members {
                let island = &self.islands[idx];
                let (w, h) = match island.mode {
                    IslandMode::Mini => (MINI_H, MINI_H),
                    IslandMode::Compact => (renderer::pill_width(&title_of(island)), COMPACT_H),
                    IslandMode::Expanded => {
                        let h = notifications
                            .iter()
                            .find(|a| a.id == island.activity_id)
                            .map(renderer::card_height)
                            .unwrap_or(renderer::CARD_H);
                        (renderer::CARD_W, h)
                    }
                };
                sizes.push((idx, w, h));
            }

            // Place every member in arrival order, left to right. Each bubble
            // covers a fixed slice of the one before it, so the step comes
            // from the bubble's own width: growing to compact — or opening
            // fully — pushes the newer ones along instead of expanding
            // underneath them, and nothing ever changes places.
            let overlap = (MINI_H - step).max(0.0);
            let n = sizes.len();
            let mut placed: Vec<(usize, f32, f32, f32)> = Vec::new();
            let mut group_w = 0.0_f32;
            let mut x = 0.0_f32;

            for (k, &(idx, w, h)) in sizes.iter().rev().enumerate() {
                let expanded = self.islands[idx].mode == IslandMode::Expanded;
                // An open notification is a panel, not a bubble in the deck:
                // it stands clear of its neighbours instead of overlapping.
                if expanded {
                    x += GAP;
                }
                placed.push((idx, x, w, h));
                group_w = group_w.max(x + w);

                if expanded {
                    x += w + GAP;
                } else if n - 1 - k < renderer::MAX_STACK {
                    // Anything deeper than MAX_STACK from the front piles up
                    // in place, so a huge group can't stretch the row.
                    x += (w - overlap).max(0.0);
                }
            }

            group_widths.push(group_w);
            placements.push(placed);
        }

        let total_w: f32 =
            group_widths.iter().sum::<f32>() + (group_widths.len().saturating_sub(1)) as f32 * GAP;

        // The row stays centred on its current width, so an island growing
        // spreads in both directions: its neighbours to the left are pushed
        // left and those to the right are pushed right, half the growth each
        // way. The layer itself keeps a fixed generous width so the surface
        // origin doesn't shift underneath and cancel the effect.
        self.last_content_width = (total_w + 40.0).max(LAYER_W as f32);
        let mut group_x = ((self.last_content_width - total_w) / 2.0).max(0.0);

        let mut targets: Vec<(usize, f32, f32, f32, f32)> = Vec::new();
        for (gi, placed) in placements.iter().enumerate() {
            for &(idx, offset, w, h) in placed {
                let x = group_x + offset;
                let cx = x + w / 2.0;
                let cy = match self.islands[idx].mode {
                    // Expanded grows downward from where the pill's top edge is.
                    IslandMode::Expanded => (BAR_HEIGHT - COMPACT_H) / 2.0 + h / 2.0,
                    _ => BAR_HEIGHT / 2.0,
                };
                targets.push((idx, w, h, cx, cy));
            }
            group_x += group_widths[gi] + GAP;
        }

        // The push travels along the row rather than hitting every bubble at
        // once: whichever island is currently grown is the source, and each
        // island further from it starts a little later. When nothing is grown
        // the source is whatever was grown last, so the row cascades back too.
        let mut row: Vec<usize> = targets.iter().map(|(idx, ..)| *idx).collect();
        row.sort_by(|&a, &b| {
            let (ax, bx) = (self.islands[a].last_layout.2, self.islands[b].last_layout.2);
            ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
        });
        let active = self
            .islands
            .iter()
            .find(|i| i.mode == IslandMode::Expanded)
            .or_else(|| self.islands.iter().find(|i| i.mode == IslandMode::Compact))
            .map(|i| i.activity_id)
            .or(self.last_active_island);
        self.last_active_island = active;
        let source_pos = active
            .and_then(|id| row.iter().position(|&i| self.islands[i].activity_id == id))
            .unwrap_or(0);

        // Redraw buffers whose content changed, then animate to the new layout.
        let layout_delay = if reposition_delay { 0.4 } else { 0.0 };
        for (idx, w, h, cx, cy) in targets {
            let cascade = row
                .iter()
                .position(|&i| i == idx)
                .map(|p| p.abs_diff(source_pos) as f64 * CASCADE_STAGGER_SECS)
                .unwrap_or(0.0);
            let activity = notifications
                .iter()
                .find(|a| a.id == self.islands[idx].activity_id)
                .cloned();
            let Some(activity) = activity else { continue };

            let mode = self.islands[idx].mode;
            let content = IslandContent {
                mode,
                icon: activity.icon.clone(),
                title: activity.title.clone(),
                body: activity.body.clone(),
                time_label: renderer::elapsed_label(activity.created_at),
                actions: activity.actions.clone(),
                w,
                h,
            };
            if self.islands[idx].last_content.as_ref() != Some(&content) {
                let surface = &mut self.islands[idx].surface;
                match mode {
                    IslandMode::Mini => draw_content(surface, w, h, |canvas| {
                        renderer::draw_mini(canvas, &activity.icon, w, h);
                    }),
                    IslandMode::Compact => draw_content(surface, w, h, |canvas| {
                        renderer::draw_pill(canvas, &activity.icon, &activity.title, w, h);
                    }),
                    IslandMode::Expanded => draw_content(surface, w, h, |canvas| {
                        renderer::draw_card(canvas, &activity, w, h);
                    }),
                }
                self.islands[idx].last_content = Some(content);
            }

            let radius = match mode {
                IslandMode::Expanded => renderer::CARD_RADIUS as f64,
                _ => h as f64 / 2.0,
            };
            let target = (w, h, cx, cy);
            if self.islands[idx].last_layout == (0.0, 0.0, 0.0, 0.0) {
                // First layout for a brand-new island: land it in place, then
                // pop it open.
                set_size_and_position(&self.islands[idx].surface, w, h, cx, cy);
                renderer::animate_enter_pop(&self.islands[idx].surface, radius);
                self.islands[idx].last_layout = target;
            } else if self.islands[idx].last_layout != target {
                animate_to(
                    &self.islands[idx].surface,
                    w,
                    h,
                    cx,
                    cy,
                    radius,
                    layout_delay + cascade,
                );
                self.islands[idx].last_layout = target;
            }
        }

        // Z-order only — this never moves anything, it just decides what
        // draws on top: an open notification, then the deck front-to-back.
        let stacking: Vec<Vec<usize>> = groups
            .iter()
            .map(|members| {
                let mut m = members.clone();
                m.sort_by_key(|&i| self.islands[i].mode != IslandMode::Expanded);
                m
            })
            .collect();
        let restacked = self.restack(&stacking);
        let size_changed = self.update_layer_size();
        self.update_input_region(size_changed || restacked);
    }

    /// Keep the front of each stack visually on top. Wayland subsurface
    /// stacking is parent-relative: for this top-anchored layer surface
    /// `place_above` pushes a surface *behind*, so walking each group
    /// front-to-back and placing each one above its predecessor puts the
    /// front island on top. Returns true when the order actually changed.
    fn restack(&mut self, groups: &[Vec<usize>]) -> bool {
        let order: Vec<u64> = groups
            .iter()
            .flat_map(|m| m.iter().map(|&i| self.islands[i].activity_id))
            .collect();
        if self.last_stack_order.as_ref() == Some(&order) {
            return false;
        }
        for members in groups {
            for pair in members.windows(2) {
                let (front, behind) = (pair[0], pair[1]);
                let front_surface = self.islands[front].surface.wl_surface().clone();
                self.islands[behind].surface.place_above(&front_surface);
            }
        }
        self.last_stack_order = Some(order);
        true
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

        // Tall enough for the deepest expanded island, plus any dialog panel.
        let mut max_h = BAR_HEIGHT;
        for island in &self.islands {
            let (_, h, _, cy) = island.last_layout;
            max_h = max_h.max(cy + h / 2.0 + 4.0);
        }
        if let Some(panel) = &self.dialog {
            max_h = max_h.max(DIALOG_TOP + panel.layout_h + 12.0);
        }

        let mut needed_w = self.layer_width();
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
            // One rect per island, straight from its layout box. Overlapping
            // rects in a wl_region are fine — the union is what matters.
            for island in &self.islands {
                let (w, h, cx, cy) = island.last_layout;
                rects.push((
                    (cx - w / 2.0).max(0.0) as i32,
                    (cy - h / 2.0).max(0.0) as i32,
                    w.ceil() as i32,
                    h.ceil() as i32,
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

    /// What is at (px, py): the island hit, and which of its inline action
    /// buttons if the hit landed on one. Islands are tested front-to-back
    /// within each group so the top of an overlapped stack wins.
    fn hit_test(&self, px: f32, py: f32) -> Option<(u64, Option<String>)> {
        for members in self.group_order() {
            for idx in members {
                let island = &self.islands[idx];
                let (w, h, cx, cy) = island.last_layout;
                let (x, y) = (cx - w / 2.0, cy - h / 2.0);
                if px < x || px > x + w || py < y || py > y + h {
                    continue;
                }

                // Only an expanded island draws action buttons.
                let action_id = if island.mode == IslandMode::Expanded && !island.actions.is_empty()
                {
                    let (local_x, local_y) = (px - x, py - y);
                    renderer::card_action_rects(&island.body, &island.actions, w)
                        .into_iter()
                        .find(|(bx, by, bw, bh, _, _)| {
                            local_x >= *bx
                                && local_x <= *bx + *bw
                                && local_y >= *by
                                && local_y <= *by + *bh
                        })
                        .map(|(_, _, _, _, id, _)| id)
                } else {
                    None
                };
                return Some((island.activity_id, action_id));
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
        let Some((activity_id, action_id)) = self.hit_test(px, py) else {
            return;
        };
        let Some(idx) = self
            .islands
            .iter()
            .position(|i| i.activity_id == activity_id)
        else {
            return;
        };

        self.last_interaction = std::time::Instant::now();

        // Mini → Compact → Expanded is just growth; nothing is dismissed until
        // the notification is actually open.
        if self.islands[idx].mode != IslandMode::Expanded {
            // Only one island is open at a time.
            for island in &mut self.islands {
                if island.mode == IslandMode::Expanded {
                    island.mode = IslandMode::Compact;
                    island.opened_by_user = false;
                }
            }
            let island = &mut self.islands[idx];
            island.peek_until = None;
            island.mode = match island.mode {
                IslandMode::Mini => IslandMode::Compact,
                _ => IslandMode::Expanded,
            };
            island.opened_by_user = island.mode == IslandMode::Expanded;
            tracing::info!(app_id = %island.app_id, mode = ?island.mode, "click: island opened");
            self.focused_island = Some(activity_id);
            let mut state = self.state.lock().unwrap();
            state.dirty = true;
            return;
        }

        // The island is expanded: decide between close, an inline action, and
        // the body (default action). An action button always wins over the
        // close zone — the buttons sit in the body row, clear of it.
        let (w, _h, cx, _cy) = self.islands[idx].last_layout;
        let is_close = action_id.is_none() && px - (cx - w / 2.0) > w - CARD_CLOSE_ZONE;

        let mut state = self.state.lock().unwrap();
        let activity = state.activities.iter().find(|a| a.id == activity_id);
        let notification_id = activity.and_then(|a| a.notification_id);
        let default_action = activity.and_then(|a| a.default_action.clone());
        let app_id = self.islands[idx].app_id.clone();
        tracing::info!(activity_id, %app_id, close = is_close, ?action_id, "expanded island clicked");

        state.dismiss_activity(activity_id);
        drop(state);

        renderer::animate_dismiss(&self.islands[idx].surface, 1.2);

        if is_close {
            // Reason 2: dismissed by the user via the Close affordance.
            if let Some(nid) = notification_id {
                emit_notification_closed(nid, 2);
            }
        } else {
            // Action click — focus the app and emit ActionInvoked. A hit on a
            // specific inline action button reports that action's id; otherwise
            // (a click on the body) it's the default action.
            request_focus_app(app_id);
            if let Some(nid) = notification_id {
                let action_key = action_id
                    .or(default_action)
                    .unwrap_or_else(|| "default".to_string());
                emit_action_invoked(nid, action_key);
                // The notification is gone from the island either way, so the
                // sender has to hear about it — an app that tracks its own
                // notifications would otherwise think this one is still up.
                // Reason 2: dismissed by the user, by acting on it.
                emit_notification_closed(nid, 2);
            }
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
        // Start fully transparent. The panel is created before anything knows
        // its layout, so until `render_dialog` has positioned it and started
        // the entrance it must not be able to show at its default geometry —
        // otherwise it flashes for a frame in the wrong place.
        if let Some(ss) = surface.base_surface().surface_style() {
            ss.set_opacity(0.0);
        }
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

        let cx = layer_w / 2.0;
        let cy = DIALOG_TOP + h / 2.0;
        panel.origin = (cx - w / 2.0, DIALOG_TOP);
        panel.layout_h = h;

        // Geometry before content: `draw` commits the buffer, so a panel drawn
        // while still at its default position would be presented there for a
        // frame before the entrance moved it.
        set_size_and_position(&panel.surface, w, h, cx, cy);

        let selected = panel.selected.clone();
        let view = panel.view.clone();
        draw_content(&mut panel.surface, w, h, |canvas| {
            dialog::draw_dialog(canvas, &view, &selected, &layout);
        });

        if !panel.entered {
            // Pop open from a small rounded shape — the transform animates,
            // the content does not. Opacity was pinned to 0 at creation, so
            // this is the first frame the panel can be seen at all.
            renderer::animate_enter_pop(&panel.surface, dialog::PANEL_RADIUS as f64);
            panel.entered = true;
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
        // Redraw on *every* configure, not just the first. The parent surface is
        // fully transparent, but its buffer is what bounds the input region: a
        // region rect outside the attached buffer is clipped away by the
        // compositor. Without a redraw after a grow, the layer keeps its initial
        // buffer and clicks below the old height are never delivered — a tall
        // dialog's lower rows and its buttons go dead.
        if let Some(layer) = &self.layer_surface {
            layer.draw(|canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
            });
        }
        let first = !self.surfaces_ready;
        self.surfaces_ready = true;
        // On the first configure this sets an empty region so clicks pass
        // through until islands appear; later ones re-commit the region now
        // that the buffer is large enough to carry it.
        self.update_input_region(!first);
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
        if elapsed >= FOCUS_TIMEOUT_SECS && self.focused_island.is_some() {
            let any_expanded = self.islands.iter().any(|i| i.mode == IslandMode::Expanded);
            if !any_expanded {
                tracing::info!(
                    focused = ?self.focused_island,
                    elapsed_secs = format!("{:.1}", elapsed),
                    "focus timeout → all Mini"
                );
                self.focused_island = None;
                let mut state = self.state.lock().unwrap();
                state.dirty = true;
                drop(state);
            }
        }

        let now = std::time::Instant::now();

        // Arrival timeout: a newly-arrived notification stops announcing
        // itself and settles back into its app's stack. The window is held
        // open while the pointer is on that island, so it never collapses out
        // from under someone reading it.
        let mut dirty_after_peek = false;
        for island in &mut self.islands {
            if let Some(until) = island.peek_until {
                if self.hovered_island == Some(island.activity_id) {
                    island.peek_until = Some(now + Duration::from_secs(ARRIVAL_READ_SECS));
                } else if now >= until {
                    tracing::info!(app_id = %island.app_id, "arrival window expired → Mini");
                    island.peek_until = None;
                    dirty_after_peek = true;
                }
            }
        }

        if dirty_after_peek {
            let mut state = self.state.lock().unwrap();
            state.dirty = true;
            drop(state);
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
        if self.focused_island.is_some()
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
        // Collapse an open notification on focus loss.
        let mut changed = false;
        for island in &mut self.islands {
            if island.mode == IslandMode::Expanded {
                island.mode = IslandMode::Compact;
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
                    let new_island = hit.as_ref().map(|(id, _)| *id);
                    let new_app = new_island.and_then(|id| {
                        self.islands
                            .iter()
                            .find(|i| i.activity_id == id)
                            .map(|i| i.app_id.clone())
                    });
                    // The hovered island grows; its whole group fans out.
                    if new_island != self.hovered_island || new_app != self.hovered_app {
                        self.hovered_island = new_island;
                        self.hovered_app = new_app;
                        let mut state = self.state.lock().unwrap();
                        state.dirty = true;
                    }
                    if hit.is_some() {
                        AppContext::set_cursor_shape(otto_kit::CursorShape::Pointer);
                    } else {
                        AppContext::set_cursor_shape(otto_kit::CursorShape::Default);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.hovered_app.is_some() || self.hovered_island.is_some() {
                        self.hovered_app = None;
                        self.hovered_island = None;
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

/// The D-Bus connection that owns the org.freedesktop.Notifications bus name.
/// Signals must be emitted from this connection, not an anonymous one, so
/// receivers matching on sender="org.freedesktop.Notifications" can see them.
static NOTIFICATIONS_DBUS_CONNECTION: std::sync::OnceLock<zbus::Connection> =
    std::sync::OnceLock::new();

/// Emit the org.freedesktop.Notifications ActionInvoked signal.
fn emit_action_invoked(notification_id: u32, action_key: String) {
    let Some(connection) = NOTIFICATIONS_DBUS_CONNECTION.get().cloned() else {
        tracing::warn!(
            notification_id,
            "ActionInvoked: notifications D-Bus connection not ready"
        );
        return;
    };
    tokio::spawn(async move {
        let Ok(ctxt) =
            zbus::SignalContext::new(&connection, notifications::NOTIFICATIONS_DBUS_PATH)
        else {
            tracing::warn!(
                notification_id,
                "ActionInvoked: failed to build signal context"
            );
            return;
        };
        if let Err(e) =
            notifications::NotificationDaemon::action_invoked(&ctxt, notification_id, &action_key)
                .await
        {
            tracing::warn!(notification_id, "ActionInvoked signal failed: {e}");
        }
    });
}

/// Emit the org.freedesktop.Notifications NotificationClosed signal.
/// Reasons per spec: 1 = expired, 2 = dismissed by the user, 3 = closed via
/// CloseNotification, 4 = undefined.
fn emit_notification_closed(notification_id: u32, reason: u32) {
    let Some(connection) = NOTIFICATIONS_DBUS_CONNECTION.get().cloned() else {
        tracing::warn!(
            notification_id,
            "NotificationClosed: notifications D-Bus connection not ready"
        );
        return;
    };
    tokio::spawn(async move {
        let Ok(ctxt) =
            zbus::SignalContext::new(&connection, notifications::NOTIFICATIONS_DBUS_PATH)
        else {
            tracing::warn!(
                notification_id,
                "NotificationClosed: failed to build signal context"
            );
            return;
        };
        if let Err(e) =
            notifications::NotificationDaemon::notification_closed(&ctxt, notification_id, reason)
                .await
        {
            tracing::warn!(notification_id, "NotificationClosed signal failed: {e}");
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

        let _ = NOTIFICATIONS_DBUS_CONNECTION.set(connection);
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
