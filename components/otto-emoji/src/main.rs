//! otto-emoji — find an emoji, press Enter, and it is typed where the cursor
//! was.
//!
//! A card over the desktop, in the launcher's material: a query field, a
//! strip of category tabs, a grid, and a footer naming what is under the
//! cursor with the skin tone to use. Type to search by name, or browse the
//! palette by category. Picking one closes the card and types the emoji
//! into whichever window had the keyboard, through a virtual keyboard — see
//! [`otto_emoji::typing`]. `--copy` puts it on the clipboard instead.
//!
//! Started fresh on a key binding and gone as soon as something is picked:
//! there is no daemon. The emoji table is baked into the binary and the
//! images are drawn as they scroll into view.

use std::time::{Duration, Instant};

use otto_kit::accessibility::{A11yTree, Action, ActionRequest, Role};
use otto_kit::clipboard;
use otto_kit::components::scroll::ScrollView;
use otto_kit::components::text_input::{
    KeyMods, TextInput, TextInputKey, TextInputResponse, CARET_BLINK_PERIOD,
};
use otto_kit::focus::FocusId;
use otto_kit::protocols::otto_surface_style_v1::{BeakEdge, BlendMode, ClipMode, ContentsGravity};
use otto_kit::protocols::otto_timing_function_v1::Preset;
use otto_kit::surfaces::{LayerShellSurface, SubsurfaceSurface};
use otto_kit::{App, AppContext, AppRunner, ObjectId};
use skia_safe::Rect;
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_client::Proxy;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

use otto_emoji::data::{Table, Tone, GROUPS};
use otto_emoji::pan::Pan;
use otto_emoji::view::{
    field_style, tab_at, tone_at, Beak, Cell, Layout, Palette, Pane, Placement, BEAK, BEAK_REACH,
    CARD_H, CARD_W, CELL, COLUMNS, FIELD_H, FOOTER_Y, GRID_H, GRID_ROWS, GRID_Y, RADIUS, TABS_H,
};
use otto_emoji::{rank, recents, typing};

/// How long the scene is kept painting after a change, so the selection's
/// slide and the marker's are seen through to the end.
const SETTLE: Duration = Duration::from_millis(220);

/// A frame the compositor never answered must not freeze the picker.
const FRAME_TIMEOUT: Duration = Duration::from_millis(500);

/// How the card arrives and leaves — the launcher's entrance, which is the
/// same kind of card.
const FADE_IN: Duration = Duration::from_millis(90);
const SCALE_IN: Duration = Duration::from_millis(340);
const FADE_OUT: Duration = Duration::from_millis(90);
const SCALE_OUT: Duration = Duration::from_millis(110);
const BOUNCE: f64 = 0.35;
const CLOSE: Duration = Duration::from_millis(120);
const OPEN_SCALE: f64 = 0.96;
const CLOSE_SCALE: f64 = 0.96;

/// The query field's identity for assistive technologies.
const FIELD: FocusId = FocusId::from_raw(0xE3_0000);
/// The grid's.
const GRID: FocusId = FocusId::from_raw(0xE3_0001);

fn cell_focus(index: usize) -> FocusId {
    FocusId::new(format!("cell-{index}"))
}

/// What to do with the pick.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Deliver {
    /// Decide per pick: typed keys where they work, a paste where they do
    /// not. The default.
    Auto,
    /// Always type it into the focused window once the card is gone.
    Type,
    /// Put it on the clipboard and stop there — no paste. The picker stays
    /// around until it is pasted or someone copies something else, because a
    /// clipboard offer lives as long as the client that made it.
    Copy,
}

/// The delivery route a pick actually takes, once `Deliver::Auto` has decided.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    Type,
    Paste,
    Copy,
}

/// Why the picker is closing, which is what happens after the card is gone.
#[derive(Clone)]
enum Outcome {
    Cancelled,
    Picked(String),
}

struct Picker {
    surface: Option<LayerShellSurface>,
    card: Option<SubsurfaceSurface>,
    palette: Option<Palette>,
    input_region: Option<(i32, i32, i32, i32)>,
    /// The placement and scale the card's geometry was last built from. The
    /// scale is part of it because the compositor reports an output's integer
    /// scale on the first configure and its fractional scale a moment later:
    /// geometry worked out once is worked out with the wrong number, and has
    /// to be redone when the right one arrives.
    card_geometry: Option<(Placement, f64)>,
    /// Kept alive for as long as the picker is up: dropping it stops the
    /// compositor reporting where the text cursor is.
    text_cursor: Option<otto_kit::protocols::otto_text_cursor_v1::OttoTextCursorV1>,

    table: Table,
    /// The recent picks, as typed, most recent first.
    recent: Vec<String>,
    tone: Tone,
    deliver: Deliver,

    /// What the grid shows: the palette by section, or the matches for the
    /// query.
    cells: Vec<Cell>,
    layout: Layout,
    /// The swipe across the category panes, and one vertical scroll per pane.
    /// A pane fills the card, so sideways is paging — it settles onto a whole
    /// pane the way the workspace swipe does — while inside a pane it is
    /// ordinary scrolling with a fling and a rubber band.
    pan: Pan,
    panes: Vec<ScrollView>,
    /// Which way the current two-finger gesture is going: `Some(true)` a pan
    /// across the panes, `Some(false)` a scroll inside one. Chosen by the
    /// gesture's first delta and kept until the fingers lift.
    gesture_horizontal: Option<bool>,
    /// When the last axis event arrived, so a vertical gesture whose end is
    /// never announced cannot leave a scroll view running for ever.
    last_axis: Option<Instant>,
    /// Index into `cells` of the highlighted one.
    selected: Option<usize>,
    /// Whether the query is showing matches rather than the palette.
    searching: bool,

    input: TextInput,
    shift: bool,
    sized: bool,
    engaged: bool,
    opened: bool,
    closing_at: Option<Instant>,
    outcome: Option<Outcome>,
    /// The serial of the keystroke or click that made the pick, which a
    /// clipboard claim has to carry.
    pick_serial: u32,
    /// Set once the surface is gone and the pick has been delivered.
    delivered: bool,
    /// What had the keyboard when the picker started — decides whether the
    /// pick is typed or pasted. Asked before the picker's own surface exists.
    target: otto_emoji::target::Kind,
    /// Set once the picker is only staying alive to serve the clipboard —
    /// after a copy, or after a paste has put the previous clipboard back.
    /// A Wayland selection lives only as long as the client offering it, so
    /// the picker must outlast the card until something else claims the
    /// clipboard, at which point it has nothing left to do.
    holding_clipboard: bool,

    dirty: bool,
    painted_at: Option<Instant>,
    settle_until: Option<Instant>,
    last_tick: Instant,
}

static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn since_start() -> u128 {
    STARTED.get_or_init(Instant::now).elapsed().as_millis()
}

impl Picker {
    fn new(query: &str, deliver: Deliver) -> Self {
        let table = Table::load();
        tracing::debug!(
            ms = since_start(),
            emoji = table.emoji.len(),
            "table loaded"
        );
        let recent = recents::recent();
        let tone = recents::tone();

        let mut input = TextInput::editing(query, field_style(dark()));
        input.state.placeholder = otto_kit::t!("emoji-search").to_string();
        input.set_size(CARD_W, FIELD_H);

        Self {
            surface: None,
            card: None,
            palette: None,
            input_region: None,
            card_geometry: None,
            text_cursor: None,
            table,
            recent,
            tone,
            deliver,
            cells: Vec::new(),
            layout: Layout::default(),
            pan: Pan::new(CARD_W),
            panes: Vec::new(),
            gesture_horizontal: None,
            last_axis: None,
            selected: None,
            searching: false,
            input,
            shift: false,
            sized: false,
            engaged: false,
            opened: false,
            closing_at: None,
            outcome: None,
            pick_serial: 0,
            delivered: false,
            target: otto_emoji::target::focused_kind(),
            holding_clipboard: false,
            dirty: true,
            painted_at: None,
            settle_until: None,
            last_tick: Instant::now(),
        }
    }

    /// The emoji table, less whatever the font cannot draw.
    fn prune_undrawable(&mut self) {
        let Some(palette) = self.palette.as_ref() else {
            return;
        };
        let before = self.table.emoji.len();
        self.table
            .retain_drawable(|emoji| palette.can_draw(emoji.first_codepoint()));
        let dropped = before - self.table.emoji.len();
        if dropped > 0 {
            tracing::info!(dropped, "emoji the font cannot draw were left out");
        }
        self.recent.retain(|text| self.table.find(text).is_some());
    }

    // === The list ===

    /// Rebuild the cells and lines from the query, the tone and the recent
    /// list, keeping the selection on the same cell where it still exists.
    fn refilter(&mut self) {
        let query = self.input.value().trim().to_string();
        let previous = self
            .selected
            .and_then(|index| self.cells.get(index))
            .cloned();

        let mut cells = Vec::new();
        let mut panes: Vec<Pane> = Vec::new();
        if query.is_empty() {
            self.searching = false;
            // The recent picks are their own pane, first, exactly as they were
            // typed — a tone picked last week stays that tone.
            let recent: Vec<Cell> = self
                .recent
                .iter()
                .filter_map(|text| {
                    Some(Cell {
                        emoji: self.table.find(text)?,
                        text: text.clone(),
                    })
                })
                .collect();
            if !recent.is_empty() {
                panes.push(Pane {
                    tab: 0,
                    first: 0,
                    count: recent.len(),
                });
                cells.extend(recent);
            }

            for (group_index, _) in GROUPS.iter().enumerate() {
                let first = cells.len();
                for index in self.table.in_group(group_index) {
                    let emoji = &self.table.emoji[index];
                    cells.push(Cell {
                        emoji: index,
                        text: emoji.with_tone(self.tone).to_string(),
                    });
                }
                panes.push(Pane {
                    tab: group_index + 1,
                    first,
                    count: cells.len() - first,
                });
            }
        } else {
            self.searching = true;
            for index in rank(&self.table.emoji, &query) {
                let emoji = &self.table.emoji[index];
                cells.push(Cell {
                    emoji: index,
                    text: emoji.with_tone(self.tone).to_string(),
                });
            }
            // Matches are one pane: the ranking is the order, and there is
            // nothing to pan between.
            panes.push(Pane {
                tab: 0,
                first: 0,
                count: cells.len(),
            });
        }

        self.layout = Layout::new(panes);
        self.cells = cells;

        // One vertical scroll per pane, each holding its own pane's height.
        self.panes.resize_with(self.layout.pane_count(), || {
            ScrollView::new(Rect::from_xywh(0.0, 0.0, CARD_W, GRID_H))
        });
        self.panes.truncate(self.layout.pane_count());
        for (index, pane) in self.panes.iter_mut().enumerate() {
            pane.set_content_length(self.layout.pane_height(index));
        }
        self.pan.set_extent(CARD_W, self.layout.max_pan());

        self.selected = match previous {
            Some(cell) if self.searching => {
                // A search moves the list under the selection; the first
                // match is what Enter should mean.
                let _ = cell;
                (!self.cells.is_empty()).then_some(0)
            }
            Some(cell) => self.cells.iter().position(|c| c.emoji == cell.emoji),
            None => (!self.cells.is_empty()).then_some(0),
        };
        if self.searching {
            self.pan.jump_to_pane(0);
            if let Some(pane) = self.panes.first_mut() {
                pane.scroll_to(0.0);
            }
            self.selected = (!self.cells.is_empty()).then_some(0);
        } else if self.selected.is_none() && !self.cells.is_empty() {
            let pane = self.current_pane();
            self.selected = self.layout.cell_in(pane, 0, 0);
        }
        self.dirty = true;
    }

    /// Where the palette opens: the recent picks when there are enough of
    /// them to be worth a pane of their own, otherwise the first category. A
    /// pane fills the card, and opening onto two or three recent emoji in an
    /// otherwise empty grid reads as a broken palette rather than a useful
    /// shortcut.
    fn open_on_default_pane(&mut self) {
        let start = self
            .layout
            .panes
            .iter()
            .position(|pane| pane.tab != 0 || pane.count >= COLUMNS)
            .unwrap_or(0);
        self.pan.jump_to_pane(start);
        self.select(self.layout.cell_in(start, 0, 0));
    }

    /// The pane the viewport is resting on.
    fn current_pane(&self) -> usize {
        self.pan
            .pane()
            .min(self.layout.pane_count().saturating_sub(1))
    }

    /// How far each pane is scrolled, for the renderer.
    fn pane_scrolls(&self) -> Vec<f32> {
        self.panes.iter().map(ScrollView::offset).collect()
    }

    fn pane_scroll(&self, pane: usize) -> f32 {
        self.panes.get(pane).map(ScrollView::offset).unwrap_or(0.0)
    }

    /// Bring the selected cell into view: pan to its pane, and scroll that
    /// pane so the cell is on screen.
    fn scroll_to_selection(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let (Some((pane, _, _)), Some(rect)) =
            (self.layout.place(selected), self.layout.cell_rect(selected))
        else {
            return;
        };
        self.pan.jump_to_pane(pane);
        let Some(view) = self.panes.get_mut(pane) else {
            return;
        };
        let scroll = view.offset();
        if rect.top < scroll {
            view.scroll_to(rect.top);
        } else if rect.bottom > scroll + GRID_H {
            view.scroll_to(rect.bottom - GRID_H);
        }
    }

    fn select(&mut self, index: Option<usize>) {
        if index != self.selected {
            self.selected = index;
            self.dirty = true;
        }
    }

    /// Left or right along the row, stepping into the neighbouring pane at
    /// each end — the cells run through the panes in order, so this is a walk
    /// through the whole palette.
    fn move_horizontal(&mut self, delta: isize) {
        if self.cells.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.cells.len() as isize - 1) as usize;
        self.select(Some(next));
        self.scroll_to_selection();
    }

    /// Up or down a row inside the selection's own pane, keeping the column.
    fn move_vertical(&mut self, rows: isize) {
        if self.cells.is_empty() {
            return;
        }
        let Some((pane, row, column)) = self.selected.and_then(|index| self.layout.place(index))
        else {
            let pane = self.current_pane();
            self.select(self.layout.cell_in(pane, 0, 0));
            return;
        };
        let target = row as isize + rows;
        let last = self.layout.panes[pane].rows() as isize - 1;
        let landed = self
            .layout
            .cell_in(pane, target.clamp(0, last.max(0)) as usize, column);
        if landed.is_some() {
            self.select(landed);
            self.scroll_to_selection();
        }
    }

    /// Pan to a tab's pane, and select its first cell.
    fn go_to_tab(&mut self, tab: usize) {
        let Some(pane) = self.layout.pane_for_tab(tab) else {
            return;
        };
        self.pan.glide_to_pane(pane, Instant::now());
        self.select(self.layout.cell_in(pane, 0, 0));
        self.dirty = true;
    }

    /// The tab the marker sits under: the pane the pan is resting on.
    fn active_tab(&self) -> Option<usize> {
        if self.searching || self.cells.is_empty() {
            return None;
        }
        self.layout.pane_tab(self.current_pane())
    }

    /// The next or previous pane.
    fn next_tab(&mut self, delta: isize) {
        if self.searching || self.layout.pane_count() == 0 {
            return;
        }
        let count = self.layout.pane_count() as isize;
        let next = (self.current_pane() as isize + delta).rem_euclid(count) as usize;
        if let Some(tab) = self.layout.pane_tab(next) {
            self.go_to_tab(tab);
        }
    }

    fn set_tone(&mut self, tone: Tone) {
        if tone == self.tone {
            return;
        }
        self.tone = tone;
        recents::remember_tone(tone);
        // Re-toning rebuilds every cell; keep the view where it was.
        let pane = self.current_pane();
        let scrolls = self.pane_scrolls();
        self.refilter();
        self.pan.jump_to_pane(pane);
        for (view, offset) in self.panes.iter_mut().zip(scrolls) {
            view.scroll_to(offset);
        }
    }

    /// The name shown in the footer: the selected emoji's.
    fn selected_name(&self) -> String {
        self.selected
            .and_then(|index| self.cells.get(index))
            .and_then(|cell| self.table.emoji.get(cell.emoji))
            .map(|emoji| emoji.name.to_string())
            .unwrap_or_default()
    }

    // === Picking ===

    fn pick(&mut self, serial: u32) {
        let Some(cell) = self.selected.and_then(|index| self.cells.get(index)) else {
            return;
        };
        let text = cell.text.clone();
        recents::remember(&text);
        self.pick_serial = serial;
        self.outcome = Some(Outcome::Picked(text));
        self.close();
    }

    /// The card has gone. Deliver the pick, and go.
    fn finish(&mut self) {
        if self.delivered {
            return;
        }
        self.delivered = true;

        let Some(Outcome::Picked(text)) = self.outcome.clone() else {
            AppContext::request_exit();
            return;
        };
        let route = self.route(&text);

        // The clipboard has to be claimed here, before the surface goes: a
        // selection is only granted to a client the compositor still considers
        // focused with a live input serial, and the teardown below hands the
        // keyboard back.
        let claimed = match route {
            Route::Type => false,
            Route::Paste | Route::Copy => clipboard::set_text(&text, self.pick_serial),
        };

        // The surfaces go, and the compositor is made to say it has seen them:
        // the keyboard returns to whoever had it only once the picker has let
        // go, the keys or the paste have to arrive after that, and the flush
        // is also what registers the selection claimed above.
        if let Some(mut card) = self.card.take() {
            card.destroy();
        }
        if let Some(surface) = self.surface.take() {
            surface.destroy();
        }
        AppContext::flush();
        let connection = AppContext::connection();
        let mut queue = connection.new_event_queue::<()>();
        if let Err(err) = queue.roundtrip(&mut ()) {
            tracing::warn!(%err, "could not sync with the compositor before delivering");
        }

        match route {
            Route::Type => self.type_keys(&text),
            Route::Copy => {
                if claimed {
                    self.holding_clipboard = true;
                } else {
                    tracing::warn!("could not claim the clipboard");
                    AppContext::request_exit();
                }
            }
            Route::Paste => {
                if !claimed {
                    tracing::warn!("could not claim the clipboard; typing instead");
                    self.type_keys(&text);
                    return;
                }
                // The emoji is on the clipboard and the window is told to
                // paste it. It is left there afterwards, the way every Wayland
                // picker leaves it: putting the old clipboard back would mean
                // reclaiming the selection from a process that no longer has
                // focus, which the compositor refuses. The picker lingers to
                // serve the emoji until something else is copied.
                if let Err(err) = typing::press_paste(false) {
                    tracing::warn!(%err, "could not press paste; the emoji is on the clipboard");
                }
                tracing::debug!(%text, "pasted");
                self.holding_clipboard = true;
            }
        }
    }

    /// Which way the pick is delivered.
    fn route(&self, text: &str) -> Route {
        match self.deliver {
            Deliver::Type => Route::Type,
            Deliver::Copy => Route::Copy,
            Deliver::Auto => {
                // Keys reach everything that decodes a keysym properly, which
                // is everything but Chromium; and Chromium is fine with a
                // keysym inside the BMP. So keys unless the text is outside the
                // BMP and the window is not a terminal — the one kind of window
                // where the paste chord is the wrong thing to press.
                let terminal = self.target == otto_emoji::target::Kind::Terminal;
                if typing::fits_in_bmp(text) || terminal {
                    Route::Type
                } else {
                    Route::Paste
                }
            }
        }
    }

    /// Type `text` as keys, and go.
    /// Type `text` as keys, and go.
    fn type_keys(&mut self, text: &str) {
        let started = Instant::now();
        match typing::type_text(text) {
            Ok(()) => tracing::debug!(ms = started.elapsed().as_millis(), %text, "typed"),
            Err(err) => {
                tracing::warn!(%err, "could not type the emoji; copying instead");
                self.copy(text);
                return;
            }
        }
        AppContext::request_exit();
    }

    /// Put `text` on the clipboard. The picker then lingers, invisible,
    /// until the selection is taken from it: a clipboard offer dies with the
    /// client that made it.
    fn copy(&mut self, text: &str) {
        if clipboard::set_text(text, self.pick_serial) {
            tracing::debug!(%text, "copied; staying until the clipboard moves on");
        } else {
            tracing::warn!("could not claim the clipboard");
            AppContext::request_exit();
        }
    }

    // === Painting ===

    fn push(&mut self) {
        if !self.sized {
            return;
        }
        let name = self.selected_name();
        let active = self.active_tab();
        let tone = self.tone;
        let pan = self.pan.offset();
        let scrolls = self.pane_scrolls();
        let empty_message = (self.searching && self.cells.is_empty())
            .then(|| otto_kit::t!("emoji-no-results").to_string());
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        palette.update_field(&self.input);
        palette.update_tabs(active);
        palette.update_grid(
            &self.cells,
            &self.layout,
            pan,
            &scrolls,
            self.selected,
            empty_message.as_deref(),
        );
        palette.update_footer(&name, tone);
        self.settle_until = Some(Instant::now() + SETTLE);
        self.place_card();
        self.update_input_region();
    }

    /// Tell the compositor which part of the picker is worth pointing at:
    /// the card. The parent covers the output and would otherwise take every
    /// click on it — see the launcher, which learnt this the hard way.
    fn update_input_region(&mut self) {
        let (Some(surface), Some(card), Some(palette)) = (
            self.surface.as_ref(),
            self.card.as_ref(),
            self.palette.as_ref(),
        ) else {
            return;
        };
        if !self.sized {
            return;
        }
        let rect = palette.input_rect();
        if self.input_region == Some(rect) {
            return;
        }
        let compositor = AppContext::compositor_state();
        let (Ok(on_parent), Ok(on_card)) = (Region::new(compositor), Region::new(compositor))
        else {
            tracing::warn!("no wl_region; the picker will take input over the whole output");
            return;
        };
        let (x, y, width, height) = rect;
        on_parent.add(x, y, width, height);
        // On the card's own surface the body starts below the beak's strip,
        // so the region does too — the beak is something to look at, not
        // something to click.
        let body_y = palette.placement().body_offset().1 as i32;
        on_card.add(0, body_y, width, height);
        let parent = surface.wl_surface();
        parent.set_input_region(Some(on_parent.wl_region()));
        card.wl_surface()
            .set_input_region(Some(on_card.wl_region()));
        parent.commit();
        card.wl_surface().commit();
        self.input_region = Some(rect);
    }

    /// Put the card where the placement says, at the scale the output is
    /// actually being drawn at.
    ///
    /// Kept apart from the input region because the two go stale for different
    /// reasons: the region changes when the card moves, and the geometry
    /// changes when the card moves *or* when the scale does. The compositor
    /// reports an output's integer scale on the first configure and its
    /// fractional scale a moment later, so geometry worked out once and never
    /// revisited is worked out at the wrong scale — which leaves the
    /// compositor's material a fifth wider than the picker's own drawing.
    fn place_card(&mut self) {
        let (Some(card), Some(palette)) = (self.card.as_ref(), self.palette.as_ref()) else {
            return;
        };
        if !self.sized {
            return;
        }
        let placement = palette.placement();
        let scale = AppContext::fractional_scale();
        if self.card_geometry == Some((placement, scale)) {
            return;
        }
        self.card_geometry = Some((placement, scale));

        // The surface is the whole balloon, so it starts above the body when
        // the beak is on the top edge.
        let (sx, sy) = placement.surface_origin();
        let (sw, sh) = placement.surface_size();
        card.set_position(sx as i32, sy as i32);
        let Some(style) = card.base_surface().surface_style() else {
            return;
        };
        style.set_position((sx + sw / 2.0) as f64 * scale, sy as f64 * scale);
        style.set_size(sw as f64 * scale, sh as f64 * scale);
        // The outline itself: a balloon when there is a caret to point at, and
        // a plain rounded rectangle when there is not.
        let (edge, offset) = match (placement.beak, placement.beak_offset()) {
            (Some((_, Beak::Top)), Some(offset)) => (BeakEdge::Top, offset),
            (Some((_, Beak::Bottom)), Some(offset)) => (BeakEdge::Bottom, offset),
            _ => (BeakEdge::None, 0.0),
        };
        style.set_beak(
            edge,
            offset as f64 * scale,
            BEAK as f64 * scale,
            BEAK_REACH as f64 * scale,
        );
    }

    fn open(&mut self) {
        if self.opened {
            return;
        }
        let Some(style) = self
            .card
            .as_ref()
            .and_then(|card| card.base_surface().surface_style())
        else {
            return;
        };
        self.opened = true;
        animate(FADE_IN, Curve::Preset(Preset::EaseOutQuad), || {
            style.set_opacity(1.0);
        });
        animate(SCALE_IN, Curve::Spring(BOUNCE), || {
            style.set_scale(1.0, 1.0);
        });
    }

    fn close(&mut self) {
        if self.closing_at.is_some() {
            return;
        }
        if self.outcome.is_none() {
            self.outcome = Some(Outcome::Cancelled);
        }
        self.closing_at = Some(Instant::now() + CLOSE);
        let Some(style) = self
            .card
            .as_ref()
            .and_then(|card| card.base_surface().surface_style())
        else {
            return;
        };
        animate(FADE_OUT, Curve::Preset(Preset::EaseInQuad), || {
            style.set_opacity(0.0);
        });
        animate(SCALE_OUT, Curve::Preset(Preset::EaseInQuad), || {
            style.set_scale(CLOSE_SCALE, CLOSE_SCALE);
        });
        AppContext::flush();
    }

    fn frame_in_flight(&self) -> bool {
        self.painted_at
            .is_some_and(|at| at.elapsed() < FRAME_TIMEOUT)
            && self
                .surface
                .as_ref()
                .is_some_and(|surface| surface.base_surface().frame_in_flight())
    }

    fn paint(&mut self) {
        let (Some(surface), true) = (self.surface.as_ref(), self.sized) else {
            return;
        };
        self.painted_at = Some(Instant::now());
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });
        if let Some(card) = self.card.as_ref() {
            let base = card.base_surface();
            card.draw(|canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
                base.render_layer_node(canvas);
            });
        }
    }

    /// A pointer position on the card, to what it is over.
    fn hit(&self, x: f32, y: f32) -> Hit {
        // Pointer positions arrive relative to the balloon's surface, which
        // starts at the beak; the card's own layout starts at the body.
        let y = y - self
            .palette
            .as_ref()
            .map(|palette| palette.placement().body_offset().1)
            .unwrap_or(0.0);
        if !(0.0..=CARD_W).contains(&x) || !(0.0..=CARD_H).contains(&y) {
            return Hit::Outside;
        }
        let tabs_top = FIELD_H + 1.0;
        if (tabs_top..tabs_top + TABS_H).contains(&y) {
            return tab_at(x).map(Hit::Tab).unwrap_or(Hit::Nothing);
        }
        if (GRID_Y..GRID_Y + GRID_H).contains(&y) {
            let pan = self.pan.offset();
            let pane = self.layout.pane_at(x, pan).unwrap_or(0);
            return self
                .layout
                .cell_at(x, y - GRID_Y, pan, self.pane_scroll(pane))
                .map(Hit::Cell)
                .unwrap_or(Hit::Nothing);
        }
        if y >= FOOTER_Y {
            return tone_at(x).map(Hit::Tone).unwrap_or(Hit::Nothing);
        }
        Hit::Nothing
    }
}

enum Hit {
    Outside,
    Nothing,
    Tab(usize),
    Cell(usize),
    Tone(Tone),
}

fn animate(duration: Duration, curve: Curve, changes: impl FnOnce()) {
    let Some(manager) = AppContext::surface_style_manager() else {
        changes();
        return;
    };
    let qh = AppContext::queue_handle();
    let timing = manager.create_timing_function(qh, ());
    match curve {
        Curve::Preset(preset) => timing.set_preset(preset),
        Curve::Spring(bounce) => timing.set_spring(bounce, 0.0),
    }
    let transaction = manager.begin_transaction(qh, ());
    transaction.set_duration(duration.as_secs_f64());
    transaction.set_timing_function(&timing);
    changes();
    transaction.commit();
}

enum Curve {
    Preset(Preset),
    Spring(f64),
}

/// How solid the card runs, whatever the theme's popup material says — the
/// launcher's floor, for the same reason: small things on a busy desktop.
const CARD_MIN_ALPHA: u8 = 0xD8;

fn at_least_opaque(colour: skia_safe::Color, min_alpha: u8) -> skia_safe::Color {
    skia_safe::Color::from_argb(
        colour.a().max(min_alpha),
        colour.r(),
        colour.g(),
        colour.b(),
    )
}

fn apply_card_colour(card: &SubsurfaceSurface) {
    let Some(style) = card.base_surface().surface_style() else {
        tracing::warn!("no otto-surface-style; the card will not be frosted");
        return;
    };
    let colour = skia_safe::Color4f::from(otto_kit::frosting::material(at_least_opaque(
        AppContext::current_theme().material_popup,
        CARD_MIN_ALPHA,
    )));
    style.set_background_color(
        colour.r as f64,
        colour.g as f64,
        colour.b as f64,
        colour.a as f64,
    );
}

fn apply_card_material(card: &SubsurfaceSurface) {
    apply_card_colour(card);
    let Some(style) = card.base_surface().surface_style() else {
        return;
    };
    style.set_blend_mode(if otto_kit::frosting::enabled() {
        BlendMode::BackgroundBlur
    } else {
        BlendMode::Normal
    });
    // In points, not pixels: the compositor scales the radius itself, and
    // pre-scaling it here as well is what makes a card's corners come out
    // roughly `screen_scale` times rounder than asked — round enough, at this
    // size, to swallow the beak that has to sit clear of them.
    style.set_corner_radius(otto_kit::corners::radius(RADIUS) as f64);
    style.set_masks_to_bounds(ClipMode::Enabled);
    style.set_shadow(0.32, 32.0, 0.0, 12.0, 0.0, 0.0, 0.0);
    style.set_contents_gravity(ContentsGravity::TopLeft);
    style.set_anchor_point(0.5, 0.0);
    style.set_scale(OPEN_SCALE, OPEN_SCALE);
    style.set_opacity(0.0);
}

fn dark() -> bool {
    matches!(
        otto_kit::color_scheme::current_color_scheme(),
        otto_kit::theme::ColorScheme::Dark
    )
}

impl App for Picker {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(ms = since_start(), "wayland connected");
        // Asked for before anything is drawn: the answer decides where the
        // card goes, and the compositor replies at once, so it is there by the
        // time the first configure arrives.
        self.text_cursor = AppContext::seat_state()
            .seats()
            .next()
            .and_then(|seat| AppContext::watch_text_cursor(&seat));
        AppContext::enable_layer_engine(1920.0, 1080.0);

        let surface = LayerShellSurface::with_anchor(
            Layer::Overlay,
            "otto-emoji",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            Some(0),
        )?;
        AppContext::enable_accessibility(&surface.wl_surface().id());
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

        // Tall enough for the balloon at its tallest: the body plus a beak.
        // The surface is allocated once and the compositor is told how much of
        // it to treat as the card, so a beak appearing or moving costs no
        // reallocation.
        let card = SubsurfaceSurface::new(
            surface.base_surface().wl_surface(),
            0,
            0,
            CARD_W as i32,
            (CARD_H + BEAK_REACH) as i32,
        )?;
        apply_card_material(&card);

        let Some(engine) = AppContext::layers_renderer(|renderer| renderer.engine().clone()) else {
            return Err("the layers engine is unavailable".into());
        };
        let palette = Palette::new(
            engine,
            card.base_surface().layer_node(),
            dark(),
            AppContext::fractional_scale() as f32,
        );
        self.palette = Some(palette);
        self.card = Some(card);
        self.surface = Some(surface);
        tracing::debug!(ms = since_start(), "surfaces created");

        self.prune_undrawable();
        self.refilter();
        self.open_on_default_pane();
        tracing::debug!(
            ms = since_start(),
            cells = self.cells.len(),
            "palette built"
        );
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        tracing::debug!(width, height, ms = since_start(), "configured");
        if let Some(palette) = self.palette.as_mut() {
            palette.set_size(width as f32, height as f32);
            palette.set_caret(
                AppContext::text_cursor()
                    .map(|(x, y, w, h)| (x as f32, y as f32, w as f32, h as f32)),
            );
        }
        self.sized = true;
        self.dirty = true;
        self.place_card();
        self.update_input_region();
    }

    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        let dark = dark();
        self.input.style = field_style(dark);
        if let Some(palette) = self.palette.as_mut() {
            palette.set_dark(dark);
        }
        if let Some(card) = self.card.as_ref() {
            apply_card_colour(card);
        }
        self.dirty = true;
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        match event.keysym {
            Keysym::Shift_L | Keysym::Shift_R => {
                self.shift = state == wl_keyboard::KeyState::Pressed;
                return;
            }
            _ => {}
        }
        if state != wl_keyboard::KeyState::Pressed || self.closing_at.is_some() {
            return;
        }
        self.engaged = true;

        let control = event
            .utf8
            .as_deref()
            .and_then(|text| text.chars().next())
            .filter(|c| (*c as u32) < 0x20 && *c != '\r' && *c != '\n' && *c != '\t')
            .map(|c| char::from(c as u8 + 0x60));

        match (event.keysym, control) {
            (Keysym::Escape, _) => {
                self.close();
                return;
            }
            (Keysym::Return | Keysym::KP_Enter, _) => {
                self.pick(serial);
                return;
            }
            (Keysym::Down, _) | (_, Some('n')) => {
                self.move_vertical(1);
                return;
            }
            (Keysym::Up, _) | (_, Some('p')) => {
                self.move_vertical(-1);
                return;
            }
            (Keysym::Page_Down, _) => {
                self.move_vertical(GRID_ROWS as isize);
                return;
            }
            (Keysym::Page_Up, _) => {
                self.move_vertical(-(GRID_ROWS as isize));
                return;
            }
            (Keysym::Tab, _) => {
                self.next_tab(if self.shift { -1 } else { 1 });
                return;
            }
            (Keysym::ISO_Left_Tab, _) => {
                self.next_tab(-1);
                return;
            }
            // Left and right walk the grid when the field has nothing to
            // move through; with a query typed they edit it, as expected.
            (Keysym::Left, _) if self.input.value().is_empty() => {
                self.move_horizontal(-1);
                return;
            }
            (Keysym::Right, _) if self.input.value().is_empty() => {
                self.move_horizontal(1);
                return;
            }
            (Keysym::Home, _) if self.input.value().is_empty() => {
                let pane = self.current_pane();
                if let Some(view) = self.panes.get_mut(pane) {
                    view.scroll_to(0.0);
                }
                self.select(self.layout.cell_in(pane, 0, 0));
                self.dirty = true;
                return;
            }
            (Keysym::End, _) if self.input.value().is_empty() => {
                let pane = self.current_pane();
                let last = self
                    .layout
                    .panes
                    .get(pane)
                    .and_then(|p| p.count.checked_sub(1).map(|offset| p.first + offset));
                self.select(last);
                self.scroll_to_selection();
                self.dirty = true;
                return;
            }
            (_, Some('u')) => {
                self.input.set_value("");
                self.refilter();
                return;
            }
            (_, Some('a')) => {
                self.input
                    .on_key(TextInputKey::SelectAll, KeyMods::default());
                self.dirty = true;
                return;
            }
            (_, Some('c')) | (_, Some('x')) => {
                let cut = control == Some('x');
                let key = if cut {
                    TextInputKey::Cut
                } else {
                    TextInputKey::Copy
                };
                if let TextInputResponse::Clipboard(text) =
                    self.input.on_key(key, KeyMods::default())
                {
                    clipboard::set_text(&text, serial);
                }
                if cut {
                    self.refilter();
                } else {
                    self.dirty = true;
                }
                return;
            }
            (_, Some('v')) => {
                if let Some(text) = clipboard::text() {
                    self.input
                        .on_key(TextInputKey::Paste(text), KeyMods::default());
                    self.refilter();
                }
                return;
            }
            (_, Some('w')) => {
                self.input.on_key(
                    TextInputKey::Backspace,
                    KeyMods {
                        shift: false,
                        ctrl: true,
                    },
                );
                self.refilter();
                return;
            }
            _ => {}
        }

        let mods = KeyMods {
            shift: self.shift,
            ctrl: false,
        };
        let key = match event.keysym {
            Keysym::Left => TextInputKey::Left,
            Keysym::Right => TextInputKey::Right,
            Keysym::Home => TextInputKey::Home,
            Keysym::End => TextInputKey::End,
            Keysym::BackSpace => TextInputKey::Backspace,
            Keysym::Delete => TextInputKey::Delete,
            _ => {
                let text: String = event
                    .utf8
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if text.is_empty() {
                    return;
                }
                TextInputKey::Text(text)
            }
        };

        match self.input.on_key(key, mods) {
            TextInputResponse::Changed => self.refilter(),
            TextInputResponse::Moved => self.dirty = true,
            TextInputResponse::Commit => self.pick(serial),
            TextInputResponse::Cancel => self.close(),
            _ => {}
        }
    }

    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {
        // Something else has taken the keyboard — a click beside the card,
        // which the input region lets through. A modal without its input is
        // only in the way.
        if self.engaged {
            self.close();
        }
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        if self.closing_at.is_some() {
            return;
        }
        let card_surface = self.card.as_ref().map(|card| card.wl_surface().clone());
        for event in events {
            let on_card = card_surface
                .as_ref()
                .is_some_and(|surface| *surface == event.surface);
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Motion { .. } if on_card => {
                    if let Hit::Cell(cell) = self.hit(x, y) {
                        self.select(Some(cell));
                    }
                }
                PointerEventKind::Press { .. } => {
                    self.engaged = true;
                    if !on_card || matches!(self.hit(x, y), Hit::Outside) {
                        self.close();
                        return;
                    }
                }
                PointerEventKind::Release { serial, .. } if on_card => match self.hit(x, y) {
                    Hit::Cell(cell) => {
                        self.select(Some(cell));
                        self.pick(serial);
                        return;
                    }
                    Hit::Tab(tab) => self.go_to_tab(tab),
                    Hit::Tone(tone) => self.set_tone(tone),
                    Hit::Outside | Hit::Nothing => {}
                },
                PointerEventKind::Axis {
                    vertical,
                    horizontal,
                    ..
                } if on_card => {
                    let now = Instant::now();
                    self.last_axis = Some(now);
                    let (dy, dx) = (vertical.absolute as f32, horizontal.absolute as f32);
                    let stop = vertical.stop || horizontal.stop;
                    let discrete = vertical.discrete != 0 || horizontal.discrete != 0;
                    let pane = self
                        .layout
                        .pane_at(x, self.pan.offset())
                        .unwrap_or_else(|| self.current_pane());

                    if stop {
                        // Fingers lifted: the swipe settles onto a pane, the
                        // scroll under it turns into a fling, and the next
                        // delta picks its axis afresh.
                        self.pan.release(now);
                        for view in self.panes.iter_mut() {
                            view.on_wheel_end();
                        }
                        self.gesture_horizontal = None;
                        self.dirty = true;
                        continue;
                    }

                    // A gesture belongs to one axis, chosen by its first delta
                    // and kept until it lifts: a swipe across the panes, or a
                    // scroll inside the one under the pointer.
                    let horizontal_gesture =
                        *self.gesture_horizontal.get_or_insert(dx.abs() > dy.abs());

                    if horizontal_gesture {
                        // Searching is a single pane, with nothing to swipe to.
                        if !self.searching {
                            if discrete {
                                // A tilted wheel is a step, not a drag: one
                                // notch, one pane.
                                let step = if dx > 0.0 { 1 } else { -1 };
                                self.next_tab(step);
                            } else {
                                self.pan.drag(dx * otto_emoji::pan::pan_speed(), now);
                            }
                            self.dirty = true;
                        }
                    } else {
                        let Some(view) = self.panes.get_mut(pane) else {
                            continue;
                        };
                        if discrete {
                            // A notch is a fixed step: no fling, no band.
                            view.on_wheel_discrete(dy);
                        } else if dy != 0.0 {
                            view.on_wheel(dy);
                        } else {
                            continue;
                        }
                        if let Hit::Cell(cell) = self.hit(x, y) {
                            self.select(Some(cell));
                        }
                        self.dirty = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn accessibility(&mut self, _ctx: &AppContext, _surface: &ObjectId) -> Option<A11yTree> {
        let palette = self.palette.as_ref()?;
        let (card_x, card_y) = palette.card_origin();

        let title = self.input.state.placeholder.clone();
        let mut tree = A11yTree::new(title.clone());
        let field = Rect::from_xywh(card_x, card_y, CARD_W, FIELD_H);
        tree.control(FIELD, field, Role::SearchInput, true, |node| {
            node.set_label(title.clone());
            node.set_value(self.input.value().to_owned());
            node.add_action(Action::SetValue);
        });

        // Only the cells on screen: the panes pan, and each one scrolls.
        let grid = Rect::from_xywh(card_x, card_y + GRID_Y, CARD_W, GRID_H);
        let pan = self.pan.offset();
        let shown: Vec<(usize, Rect, String)> = (0..self.cells.len())
            .filter_map(|index| {
                let (pane, _, _) = self.layout.place(index)?;
                let rect = self.layout.cell_rect(index)?;
                let left = Layout::pane_origin(pane) - pan + rect.left;
                let top = rect.top - self.pane_scroll(pane);
                if left + CELL < 0.0 || left > CARD_W || top + CELL < 0.0 || top > GRID_H {
                    return None;
                }
                let name = self
                    .table
                    .emoji
                    .get(self.cells[index].emoji)?
                    .name
                    .to_string();
                Some((
                    index,
                    Rect::from_xywh(card_x + left, card_y + GRID_Y + top, CELL, CELL),
                    name,
                ))
            })
            .collect();
        let selected = self.selected;
        tree.region(
            GRID,
            grid,
            Role::ListBox,
            otto_kit::t!("a11y-results"),
            |tree| {
                for (index, bounds, name) in shown {
                    tree.control(
                        cell_focus(index),
                        bounds,
                        Role::ListBoxOption,
                        true,
                        |node| {
                            node.set_label(name);
                            node.set_selected(Some(index) == selected);
                            node.add_action(Action::Click);
                        },
                    );
                }
            },
        );
        if let Some(index) = selected {
            tree.set_focus(cell_focus(index));
        }
        Some(tree)
    }

    fn on_accessibility_action(
        &mut self,
        _ctx: &AppContext,
        _surface: &ObjectId,
        request: &ActionRequest,
    ) {
        if !matches!(request.action, Action::Click) {
            return;
        }
        let target = (0..self.cells.len()).find(|index| {
            otto_kit::accessibility::node_id(cell_focus(*index)) == request.target_node
        });
        let Some(index) = target else { return };
        self.select(Some(index));
        self.pick(AppContext::last_input_serial());
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        if let Some(at) = self.closing_at {
            let now = Instant::now();
            if now >= at {
                self.finish();
                // Once the picker is only holding the clipboard, it retires as
                // soon as something else claims the selection — that is the
                // whole of what it was staying alive for.
                if self.holding_clipboard && !clipboard::owns_selection() {
                    AppContext::request_exit();
                }
            }
            return;
        }

        if !self.engaged && AppContext::keyboard_focus().is_some() {
            self.engaged = true;
        }

        let now = Instant::now();
        let was_visible = self.input.caret_visible();
        self.input
            .tick(now.duration_since(self.last_tick).as_secs_f32());
        self.last_tick = now;
        if self.input.caret_visible() != was_visible {
            self.dirty = true;
        }

        // Advance the swipe's settle and the flings inside the panes. Both
        // end by themselves — the swipe finishes a gesture nobody ended, and
        // a stalled vertical gesture is released here — so neither can leave
        // the picker painting for ever.
        if self.pan.tick(now) {
            self.dirty = true;
        }
        if self
            .last_axis
            .is_some_and(|at| now.duration_since(at) >= otto_emoji::pan::GESTURE_TIMEOUT)
        {
            self.last_axis = None;
            self.gesture_horizontal = None;
            for view in self.panes.iter_mut() {
                view.on_wheel_end();
            }
        }
        for view in self.panes.iter_mut() {
            if view.is_animating() && view.tick() {
                self.dirty = true;
            }
        }

        // Never paint while the compositor still owes a frame. Input arrives
        // far faster than frames do — a touchpad alone is hundreds of events
        // a second — and painting on each one queues commits the compositor
        // cannot keep up with, until the connection blocks and the picker
        // hangs holding the keyboard. Waiting coalesces a burst into one
        // frame, which is all that could have been shown anyway.
        if self.frame_in_flight() {
            return;
        }
        if self.dirty {
            self.dirty = false;
            self.push();
            self.paint();
            self.open();
            return;
        }
        if self.settle_until.is_some_and(|until| now < until) {
            self.paint();
        } else {
            self.settle_until = None;
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        if self.closing_at.is_some() {
            return Some(if self.holding_clipboard {
                // Just serving the clipboard; the loss that frees us arrives
                // as an event, so this is only a floor.
                Duration::from_secs(2)
            } else if self.delivered {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(8)
            });
        }
        // A running swipe, fling or bounce needs a frame's worth of ticks.
        if self.pan.is_busy()
            || self.last_axis.is_some()
            || self.panes.iter().any(ScrollView::is_animating)
        {
            return Some(Duration::from_millis(8));
        }
        Some(if self.settle_until.is_some() {
            Duration::from_millis(8)
        } else {
            Duration::from_secs_f32(CARET_BLINK_PERIOD / 2.0)
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    STARTED.get_or_init(Instant::now);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    otto_kit::i18n::init_from_desktop();

    let mut deliver = Deliver::Auto;
    let mut words: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--copy" | "-c" => deliver = Deliver::Copy,
            "--type" | "-t" => deliver = Deliver::Type,
            "--auto" => deliver = Deliver::Auto,
            "--help" | "-h" => {
                println!("usage: otto-emoji [--auto|--type|--copy] [query]");
                println!("  --auto  type where keys work, paste where they don't (default)");
                println!("  --type  always type the pick into the focused window");
                println!("  --copy  put the pick on the clipboard and stop there");
                return Ok(());
            }
            _ => words.push(arg),
        }
    }
    AppRunner::new(Picker::new(&words.join(" "), deliver)).run()?;
    Ok(())
}
