//! `otto-canvas`: the agent sessions, in Otto's side canvas.
//!
//! The side canvas is the column that slides in from the right edge of the
//! screen (a two-finger swipe in from the edge of the touchpad, or the
//! `CanvasToggle` shortcut). Otto owns the column; this program is a client
//! that places one item in it through `otto-canvas-v1`: the Agents panel,
//! which lists the agent sessions otto-agents has, as the launcher's agents
//! mode (`otto-launcher --agents`) lists them.
//!
//! The panel has a heading, a prompt field for a new request, and the rows.
//! It asks for the keyboard as the canvas is shown, so typing goes straight
//! into the prompt; on a compositor that cannot give it that, a click does.
//! Enter with something typed asks it; with nothing typed (or Right) it opens
//! the highlighted session. The arrows, Tab, Ctrl+N/P and the page keys walk
//! the rows, as in the launcher's list. Escape clears the prompt, and with
//! nothing in it slides the canvas away.
//!
//! Opening a session runs `otto-launcher --session <URI>`, asking runs
//! `otto-launcher --ask --send -- <request>`, and either sends the canvas
//! away: the launcher's card is where it carries on.
//!
//! The list is fetched when the canvas comes on screen and kept up to date
//! while it stays there, as the service announces changes. While the canvas
//! is hidden there is no connection and nothing to do.
//!
//! The panel is a rounded pane of the desktop's frosted material, as wide as
//! the compositor makes it. It tells the compositor how tall its rows would
//! make it, grows with them up to the share of the column it is given, and
//! scrolls past that. A compositor that gives no share (before version 5 of
//! the protocol) gets a panel no taller than [`MAX_HEIGHT`].

// Rust guideline compliant 2026-02-21

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use otto_agents_kit::item::Item;
use otto_agents_kit::keys::{self, FieldEdit};
use otto_agents_kit::rows::{
    divider_color, field_style, paint_item_rows, row_font, row_highlight_color, row_highlight_rect,
    row_subtitle_color, RowIcons, HIGHLIGHT_RADIUS, ROW_H,
};
use otto_agents_kit::sessions::{FeedStatus, SessionFeed};
use otto_kit::components::scroll::ScrollView;
use otto_kit::components::text_input::{TextInput, CARET_BLINK_PERIOD};
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode};
use otto_kit::skia::{Color4f, Contains, FontStyle, Point};
use otto_kit::surfaces::{
    apply_hairline_border, CanvasItemEvent, CanvasItemSurface, CanvasKeyboardInteractivity,
};
use otto_kit::CursorShape;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::wl_keyboard;

/// Corner radius of the panel, in logical points, where the desktop rounds
/// its corners. The same radius the desktop's other panels use.
const PANEL_CORNER: f32 = 16.0;

/// Tallest the panel grows, in logical points, when the compositor does not
/// share the column's height out; past it the rows scroll.
///
/// Such a compositor never says how tall the column is, so this is fixed:
/// eight rows under the heading and the field, which leaves room below for
/// other items on any screen the canvas is likely to be on.
const MAX_HEIGHT: i32 = 480;

/// Height of the heading's band, in logical points.
const HEADING_H: f32 = 44.0;

/// Where the heading's text and the field's text start, in logical points:
/// just inside the rows' highlight, so both line up with the list rather
/// than the panel.
const TEXT_X: f32 = 20.0;

/// Size of the heading's text, in points.
const HEADING_TEXT: f32 = 13.0;

/// Height of the prompt field, in logical points.
const FIELD_H: f32 = 40.0;

/// Size of the field's text, in points: the launcher's field face, smaller,
/// since the column is narrower than the launcher's card.
const FIELD_TEXT: f32 = 16.0;

/// Space between the divider under the field and the first row, in logical
/// points.
const LIST_PAD: f32 = 4.0;

/// Where the rows start, in logical points: under the heading, the field and
/// the hairline between the field and the rows.
const LIST_TOP: f32 = HEADING_H + FIELD_H + 1.0 + LIST_PAD;

/// Space under the last row, in logical points: the launcher's list padding.
const BOTTOM_PAD: f32 = 8.0;

/// The name the panel gives itself when it connects to the agent service.
const CLIENT_NAME: &str = "otto-canvas";

/// The program that opens a session or a new request: the launcher, in ask
/// mode.
const LAUNCHER: &str = "otto-launcher";

/// The left mouse button, as evdev numbers it.
const BTN_LEFT: u32 = 0x110;

/// How often the loop wakes while the scrollbar is up but nothing moves, so
/// it can fade out. A frame's worth: the fade is drawn on frame callbacks.
const FADE_TICK: Duration = Duration::from_millis(16);

/// What the panel shows under the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    /// Nothing yet: the first list is on its way.
    Waiting,
    /// The rows.
    Rows,
    /// There are no sessions.
    Empty,
    /// The agent service is not running.
    Unreachable,
}

/// The application: the Agents item, and the list it shows.
struct Sessions {
    item: Option<CanvasItemSurface>,
    /// What the item's handler heard, for the next update to act on. The
    /// handler runs during dispatch, where the application is not to hand.
    events: Rc<RefCell<Vec<CanvasItemEvent>>>,
    /// The connection to the agent service, while the canvas is on screen.
    feed: Option<SessionFeed>,
    /// Every session, as rows. Each row's origin index is its session's
    /// place in `resources`.
    rows: Vec<Item>,
    /// Every session's URI, in the order of the last listing.
    resources: Vec<String>,
    /// Whether the service is there and has listed the sessions.
    status: FeedStatus,
    /// The prompt: a new request, asked on Enter.
    input: TextInput,
    /// The highlighted session, by URI, so it stays on the same session as
    /// the list reorders under it.
    selected: Option<String>,
    /// The row a press landed on; a release on the same row picks it.
    pressed: Option<usize>,
    /// A press landed in the field, and a drag selects text in it.
    field_pressed: bool,
    /// Where the pointer is, in the item's coordinates, while it is over it.
    pointer: Option<(f32, f32)>,
    /// The cursor last asked for, so it is asked for only when it changes.
    cursor: CursorShape,
    /// When the caret's blink was last advanced.
    last_tick: Instant,
    icons: RowIcons,
    scroll: ScrollView,
    /// Whether the item needs drawing again.
    dirty: bool,
}

impl Sessions {
    fn new() -> Self {
        let mut input = TextInput::new("", canvas_field_style(dark()));
        input.state.placeholder = otto_kit::t!("launcher-search-ask").to_string();
        Self {
            item: None,
            events: Rc::new(RefCell::new(Vec::new())),
            feed: None,
            rows: Vec::new(),
            resources: Vec::new(),
            status: FeedStatus::Connecting,
            input,
            selected: None,
            pressed: None,
            field_pressed: false,
            pointer: None,
            cursor: CursorShape::Default,
            last_tick: Instant::now(),
            icons: RowIcons::default(),
            scroll: ScrollView::new(Rect::from_wh(0.0, 0.0)),
            dirty: false,
        }
    }

    /// What goes under the field.
    fn content(&self) -> Content {
        match self.status {
            FeedStatus::Connecting if self.resources.is_empty() => Content::Waiting,
            FeedStatus::Unreachable => Content::Unreachable,
            _ if self.rows.is_empty() => Content::Empty,
            _ => Content::Rows,
        }
    }

    /// Height of what sits under the field, in points: the rows, or one
    /// row's worth for a message.
    fn content_height(&self) -> f32 {
        match self.content() {
            Content::Rows => self.rows.len() as f32 * ROW_H,
            _ => ROW_H,
        }
    }

    /// Height the panel would be with every row showing, in whole points.
    fn natural_height(&self) -> i32 {
        (LIST_TOP + self.content_height() + BOTTOM_PAD).ceil() as i32
    }

    /// Height the panel is drawn at, in whole points: all of it when it
    /// fits its share of the column, and the share otherwise.
    fn panel_height(&self) -> i32 {
        let natural = self.natural_height();
        match self.item.as_ref() {
            Some(item) => item.fit_height(natural, MAX_HEIGHT),
            None => natural.min(MAX_HEIGHT),
        }
    }

    /// Where the rows are shown, in the item's coordinates, for an item
    /// `width` wide.
    fn viewport(&self, width: f32) -> Rect {
        let height = self.panel_height() as f32 - LIST_TOP - BOTTOM_PAD;
        Rect::from_xywh(0.0, LIST_TOP, width, height.max(0.0))
    }

    /// The item's width, in points.
    fn width(&self) -> f32 {
        self.item
            .as_ref()
            .map_or(0.0, |item| item.dimensions().0 as f32)
    }

    /// Where the prompt field is, in the item's coordinates.
    fn field_rect(&self) -> Rect {
        Rect::from_xywh(0.0, HEADING_H, self.width(), FIELD_H)
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        self.rows.iter().position(|row| {
            self.resources.get(row.origin.index).map(String::as_str) == Some(selected)
        })
    }

    /// The session URI of row `index`.
    fn resource_at(&self, index: usize) -> Option<&str> {
        let row = self.rows.get(index)?;
        self.resources.get(row.origin.index).map(String::as_str)
    }

    /// The row under `(x, y)`, in the item's coordinates.
    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.content() != Content::Rows
            || !self.scroll.state.viewport().contains(Point::new(x, y))
        {
            return None;
        }
        let (_, content_y) = self.scroll.viewport_to_content(x, y);
        let index = (content_y / ROW_H).floor();
        (index >= 0.0 && (index as usize) < self.rows.len()).then_some(index as usize)
    }

    fn select(&mut self, index: Option<usize>) {
        let selected = index.and_then(|index| self.resource_at(index).map(str::to_string));
        if selected != self.selected {
            self.selected = selected;
            self.dirty = true;
        }
    }

    /// Keep a row highlighted whenever there are rows, as the launcher does,
    /// so Enter always has something to open: the highlighted session if it
    /// is still listed, the first row otherwise.
    fn keep_selection(&mut self) {
        if self.selected_index().is_none() {
            self.select((!self.rows.is_empty()).then_some(0));
        }
    }

    /// Scroll just far enough to show row `index` whole.
    fn reveal(&mut self, index: usize) {
        let top = index as f32 * ROW_H;
        let bottom = top + ROW_H;
        let height = self.scroll.state.viewport_length();
        let offset = self.scroll.offset();
        if top < offset {
            self.scroll.scroll_to(top);
        } else if bottom > offset + height {
            self.scroll.scroll_to(bottom - height);
        }
    }

    /// Open the session in row `index` in the launcher, and send the canvas
    /// away: the launcher's card is where it carries on.
    fn open(&mut self, index: usize) {
        let Some(resource) = self.resource_at(index).map(str::to_string) else {
            return;
        };
        match launch(&["--session", &resource]) {
            Ok(()) => self.dismiss(),
            Err(err) => {
                tracing::warn!(%err, session = %resource, "could not open the session in the launcher");
            }
        }
    }

    /// Send `request` to the agent in the launcher, which opens on it
    /// running, or open an empty request when there is nothing to send; and
    /// send the canvas away.
    fn ask(&mut self, request: &str) {
        let request = request.trim();
        // After `--`, the request is words even if it looks like an option.
        let args: &[&str] = if request.is_empty() {
            &["--ask"]
        } else {
            &["--ask", "--send", "--", request]
        };
        match launch(args) {
            Ok(()) => self.dismiss(),
            Err(err) => tracing::warn!(%err, "could not start a request in the launcher"),
        }
    }

    fn dismiss(&self) {
        if let Some(item) = self.item.as_ref() {
            item.dismiss();
        }
    }

    /// Act on what the compositor said about the item.
    fn take_item_events(&mut self) {
        let events: Vec<_> = self.events.borrow_mut().drain(..).collect();
        for event in events {
            match event {
                CanvasItemEvent::Configure { .. } | CanvasItemEvent::MaxHeight { .. } => {
                    self.dirty = true;
                }
                CanvasItemEvent::Shown => {
                    // A fresh connection lists the sessions afresh. What was
                    // shown last time stays up until the new list arrives,
                    // rather than flashing empty.
                    if self.feed.is_none() {
                        self.feed = Some(SessionFeed::start(CLIENT_NAME));
                    }
                }
                CanvasItemEvent::Hidden => {
                    // Off screen, the list costs nothing: no connection, no
                    // scroll left running, no hover to come back to. What
                    // was typed goes too; the canvas opens on an empty prompt.
                    self.feed = None;
                    self.pressed = None;
                    self.field_pressed = false;
                    self.pointer = None;
                    self.scroll.stop();
                    self.scroll.on_pointer_leave();
                    if !self.input.value().is_empty() {
                        self.input.set_value("");
                        self.dirty = true;
                    }
                }
            }
        }
    }

    /// Take in what the feed has, if anything.
    fn take_feed(&mut self) {
        let Some(feed) = self.feed.as_mut() else {
            return;
        };
        if !feed.pump() {
            return;
        }
        let status = feed.status();
        if status == FeedStatus::Connecting {
            return;
        }
        self.status = status;
        self.rows = feed.items(0, "");
        self.resources = feed
            .sessions()
            .iter()
            .map(|session| session.resource.clone())
            .collect();
        self.keep_selection();
        self.dirty = true;
    }

    /// Size the item for its content, and draw it.
    fn draw(&mut self) {
        let Some(item) = self.item.as_ref() else {
            return;
        };
        // A compositor that shares the column's height says how much of it
        // is the panel's right after the first configure.
        if !item.is_configured() || (item.shares_height() && item.max_height().is_none()) {
            return;
        }
        // The compositor answers a new content height with a new share,
        // which draws again.
        item.set_content_height(self.natural_height());
        let height = self.panel_height();
        let (width, current) = item.dimensions();
        if height != current {
            item.set_height(height);
        }
        let width = width as f32;
        let content = self.content();
        self.scroll.set_viewport(self.viewport(width));
        self.scroll.set_content_length(match content {
            Content::Rows => self.content_height(),
            _ => 0.0,
        });
        self.input.set_size(width, FIELD_H);

        let dark = dark();
        let theme = AppContext::current_theme();
        let rows: Vec<&Item> = self.rows.iter().collect();
        let selected = self.selected_index();
        let scroll = &self.scroll;
        let icons = &self.icons;
        let input = &self.input;
        item.draw(|canvas| {
            canvas.clear(Color::TRANSPARENT);
            draw_heading(canvas, dark);

            canvas.save();
            canvas.translate((0.0, HEADING_H));
            input.render_at(canvas, width, FIELD_H);
            canvas.restore();

            let mut line = Paint::new(Color4f::from(divider_color(dark)), None);
            line.set_anti_alias(false);
            canvas.draw_rect(Rect::from_xywh(0.0, HEADING_H + FIELD_H, width, 1.0), &line);

            let message = match content {
                Content::Rows => None,
                Content::Waiting => Some(""),
                Content::Empty => Some(otto_kit::t!("launcher-agents-none")),
                Content::Unreachable => Some(otto_kit::t!("launcher-ask-unreachable")),
            };
            match message {
                Some(message) => {
                    draw_message(
                        canvas,
                        message,
                        Rect::from_xywh(0.0, LIST_TOP, width, ROW_H),
                        dark,
                    );
                }
                None => scroll.render(canvas, &theme, |canvas, band| {
                    if let Some(index) = selected {
                        let mut wash = Paint::new(Color4f::from(row_highlight_color(dark)), None);
                        wash.set_anti_alias(true);
                        canvas.draw_round_rect(
                            row_highlight_rect(index, width),
                            HIGHLIGHT_RADIUS,
                            HIGHLIGHT_RADIUS,
                            &wash,
                        );
                    }
                    paint_item_rows(canvas, band, &rows, &[""], width, dark, icons);
                }),
            }
        });
        self.dirty = false;
    }

    fn apply_material(&self) {
        let Some(style) = self
            .item
            .as_ref()
            .and_then(CanvasItemSurface::surface_style)
        else {
            return;
        };
        // The material goes on the compositor's layer, which blurs what is
        // behind the canvas and tints it; the buffer stays clear so the result
        // shows through.
        let colour = Color4f::from(AppContext::current_theme().material_popup);
        style.set_background_color(
            f64::from(colour.r),
            f64::from(colour.g),
            f64::from(colour.b),
            f64::from(colour.a),
        );
        style.set_blend_mode(BlendMode::BackgroundBlur);
        style.set_corner_radius(f64::from(otto_kit::corners::radius(PANEL_CORNER)));
        style.set_masks_to_bounds(ClipMode::Enabled);
        apply_hairline_border(&style);
    }

    /// Ask for the cursor that fits what is under the pointer: a text cursor
    /// over the field.
    fn update_cursor(&mut self, x: f32, y: f32) {
        let cursor = if self.field_rect().contains(Point::new(x, y)) {
            CursorShape::Text
        } else {
            CursorShape::Default
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            AppContext::set_cursor_shape(cursor);
        }
    }

    fn on_pointer(&mut self, event: &PointerEvent) {
        let (x, y) = (event.position.0 as f32, event.position.1 as f32);
        let point = Point::new(x, y);
        match event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                if matches!(event.kind, PointerEventKind::Enter { .. }) {
                    // A new surface under the pointer: its cursor is unknown.
                    self.cursor = CursorShape::Default;
                }
                self.pointer = Some((x, y));
                self.update_cursor(x, y);
                if self.field_pressed {
                    self.input.on_pointer_drag(x);
                    self.dirty = true;
                    return;
                }
                let moved = self.scroll.on_pointer_drag(x, y);
                let hovered = self.scroll.on_pointer_move(x, y);
                self.dirty |= moved || hovered;
                // The highlight follows the pointer, as it does in the
                // launcher, unless the scrollbar is being dragged.
                if !moved {
                    if let Some(index) = self.row_at(x, y) {
                        self.select(Some(index));
                    }
                }
            }
            PointerEventKind::Leave { .. } => {
                self.pointer = None;
                self.pressed = None;
                self.scroll.on_pointer_leave();
                self.dirty = true;
            }
            PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                self.pointer = Some((x, y));
                self.pressed = None;
                if self.field_rect().contains(point) {
                    // The press gave the item the keyboard; the caret goes
                    // where it landed.
                    self.input.state.set_focused(true);
                    let shift = AppContext::current_modifiers().shift;
                    self.input.on_pointer_down(x, 1, shift);
                    self.field_pressed = true;
                } else if !self.scroll.on_pointer_down(x, y) {
                    self.pressed = self.row_at(x, y);
                }
                self.dirty = true;
            }
            PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                self.scroll.on_pointer_up();
                if self.field_pressed {
                    self.field_pressed = false;
                    self.input.on_pointer_up();
                    return;
                }
                let released = self.row_at(x, y);
                if let Some(index) = self.pressed.take().filter(|&index| Some(index) == released) {
                    self.open(index);
                }
            }
            PointerEventKind::Axis { vertical, .. } => {
                if self.content() != Content::Rows {
                    return;
                }
                if vertical.stop {
                    self.scroll.on_wheel_end();
                } else if vertical.discrete != 0 {
                    self.scroll.on_wheel_discrete(vertical.absolute as f32);
                } else {
                    self.scroll.on_wheel(vertical.absolute as f32);
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// A key, with the launcher's agents-mode meanings.
    fn on_key(&mut self, event: &KeyEvent, serial: u32) {
        // A key means the item has the keyboard, whatever the last update
        // saw: the field takes it.
        self.input.state.set_focused(true);
        let control = keys::control_char(event);
        let modifiers = AppContext::current_modifiers();
        let shift = modifiers.shift;

        // Ctrl+L or Cmd+L, which from the launcher's list starts a new
        // request, asks what is typed, or opens an empty one.
        let ask_key = control == Some('l')
            || (modifiers.logo
                && !modifiers.ctrl
                && !modifiers.alt
                && matches!(event.keysym, Keysym::l | Keysym::L));
        if ask_key {
            let request = self.input.value().to_string();
            self.ask(&request);
            return;
        }

        match event.keysym {
            // Escape takes back what was typed first, and only then sends
            // the canvas away.
            Keysym::Escape => {
                if self.input.value().is_empty() {
                    self.dismiss();
                } else {
                    self.input.set_value("");
                    self.dirty = true;
                }
                return;
            }
            // Enter asks what is typed. With nothing typed it opens the
            // highlighted session, as in the launcher's list.
            Keysym::Return | Keysym::KP_Enter => {
                let request = self.input.value().trim().to_string();
                if !request.is_empty() {
                    self.ask(&request);
                } else if let Some(index) = self.selected_index() {
                    self.open(index);
                }
                return;
            }
            // Right with nothing typed opens the highlighted session, as it
            // does in the launcher's list.
            Keysym::Right if self.input.value().is_empty() => {
                if let Some(index) = self.selected_index() {
                    self.open(index);
                }
                return;
            }
            _ => {}
        }

        if let Some(delta) = keys::list_step(event.keysym, control, shift) {
            if !self.rows.is_empty() {
                let current = self.selected_index().unwrap_or(0);
                let next = if self.selected_index().is_some() {
                    keys::wrap(current, delta, self.rows.len())
                } else {
                    0
                };
                self.select(Some(next));
                self.reveal(next);
                self.dirty = true;
            }
            return;
        }

        match keys::edit_field(&mut self.input, event, control, shift, serial) {
            FieldEdit::Changed | FieldEdit::Moved => self.dirty = true,
            // Enter and Escape are answered above; the field only reports
            // them.
            FieldEdit::Commit | FieldEdit::Cancel | FieldEdit::None => {}
        }
    }

    /// Show the caret while the item has the keyboard, blinking.
    fn tick_caret(&mut self) {
        let focused = self
            .item
            .as_ref()
            .is_some_and(CanvasItemSurface::has_keyboard);
        if self.input.state.focused() != focused {
            self.input.state.set_focused(focused);
            self.dirty = true;
        }
        let now = Instant::now();
        let was_visible = self.input.caret_visible();
        self.input
            .tick(now.duration_since(self.last_tick).as_secs_f32());
        self.last_tick = now;
        if self.input.caret_visible() != was_visible {
            self.dirty = true;
        }
    }
}

impl App for Sessions {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let item = CanvasItemSurface::new(self.panel_height())?;
        // Said before the first draw, so the first share fits it.
        item.set_content_height(self.natural_height());
        let events = Rc::clone(&self.events);
        item.on_event(move |_, event| events.borrow_mut().push(event));
        // Typing goes straight into the field when the canvas opens. A
        // compositor without the request gives the keyboard on a click.
        if !item.set_keyboard_interactivity(CanvasKeyboardInteractivity::OnShow) {
            tracing::info!("the compositor gives canvas items the keyboard only on a click");
        }
        self.item = Some(item);
        self.apply_material();
        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        self.take_item_events();
        self.take_feed();

        let shown = self.item.as_ref().is_some_and(CanvasItemSurface::is_shown);
        if shown {
            self.tick_caret();
        }
        let in_flight = self
            .item
            .as_ref()
            .is_some_and(CanvasItemSurface::frame_in_flight);
        // A glide, a bounce or the scrollbar's fade moves on once per frame
        // the compositor has shown, so it runs at the display's pace.
        if shown && self.scroll.is_animating() && !in_flight && self.scroll.tick() {
            self.dirty = true;
            // The rows glide under a still pointer: the highlight goes with
            // whichever is under it now.
            if let Some((x, y)) = self.pointer {
                if let Some(index) = self.row_at(x, y) {
                    self.select(Some(index));
                }
            }
        }
        if self.dirty {
            self.draw();
        }
    }

    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        self.apply_material();
        self.input.style = canvas_field_style(dark());
        self.dirty = true;
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        for event in events {
            self.on_pointer(event);
        }
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        if state == wl_keyboard::KeyState::Pressed {
            self.on_key(event, serial);
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        let item = self.item.as_ref()?;
        if !item.is_shown() {
            return None;
        }
        if self.scroll.is_animating() {
            return Some(FADE_TICK);
        }
        // Half a blink period: the slowest the loop may sleep and still turn
        // the caret on and off on time.
        item.has_keyboard()
            .then(|| Duration::from_secs_f32(CARET_BLINK_PERIOD / 2.0))
    }

    fn poll_fds(&self) -> Vec<std::os::fd::RawFd> {
        self.feed.iter().map(SessionFeed::poll_fd).collect()
    }
}

/// Whether the desktop is in its dark scheme.
fn dark() -> bool {
    matches!(
        otto_kit::color_scheme::current_color_scheme(),
        ColorScheme::Dark
    )
}

/// The launcher's field look, at the column's size.
fn canvas_field_style(dark: bool) -> otto_kit::components::text_input::TextInputStyle {
    let mut style = field_style(dark);
    style.text_style.size = FIELD_TEXT;
    style.horizontal_padding = TEXT_X;
    style
}

/// The panel's heading, in the band above the field.
fn draw_heading(canvas: &Canvas, dark: bool) {
    let font = get_font_with_fallback(styles::BODY.family, FontStyle::bold(), HEADING_TEXT);
    let mut paint = Paint::new(Color4f::from(row_subtitle_color(dark)), None);
    paint.set_anti_alias(true);
    canvas.draw_str(
        otto_kit::t!("canvas-sessions-heading"),
        (TEXT_X, HEADING_H / 2.0 + HEADING_TEXT * 0.35),
        &font,
        &paint,
    );
}

/// A line of text centred in `rect`, standing in for the rows, the way the
/// launcher says its list is empty.
fn draw_message(canvas: &Canvas, message: &str, rect: Rect, dark: bool) {
    if message.is_empty() {
        return;
    }
    let font = row_font(15.0);
    let mut paint = Paint::new(Color4f::from(row_subtitle_color(dark)), None);
    paint.set_anti_alias(true);
    let width = font.measure_str(message, Some(&paint)).0;
    canvas.draw_str(
        message,
        (
            rect.left + (rect.width() - width) / 2.0,
            rect.center_y() + 5.0,
        ),
        &font,
        &paint,
    );
}

/// Start `otto-launcher` with `args`. It runs in a process group of its own,
/// so it outlives this program, and is reaped on a thread.
fn launch(args: &[&str]) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut child = Command::new(LAUNCHER)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    otto_kit::logging::init("info");
    otto_kit::i18n::init_from_desktop();

    AppRunner::new(Sessions::new()).run()
}
