//! otto-launcher — type to find something, Enter to have it.
//!
//! A fullscreen overlay that takes the keyboard, filters a list of [`Item`]s
//! from one or more [`Source`]s, and activates the one that is selected. Apps
//! and open windows are the two sources it ships with; a source is a trait, so
//! files, clipboard history or a calculator are additions rather than rewrites.
//!
//! Run it and it stays up until something is picked, Escape is pressed, or the
//! click lands outside the card. It is meant to be bound to a key and started
//! fresh each time — there is no daemon, and nothing to keep warm: the desktop
//! entry scan is the only work at startup, and it is milliseconds.

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use otto_kit::accessibility::{A11yTree, Action, ActionRequest, Role};
use otto_kit::components::text_input::{
    KeyMods, TextInput, TextInputKey, TextInputResponse, CARET_BLINK_PERIOD,
};
use otto_kit::focus::FocusId;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::protocols::otto_timing_function_v1::Preset;
use otto_kit::surfaces::{LayerShellSurface, SubsurfaceSurface};
use otto_kit::{App, AppContext, AppRunner, ObjectId};
use skia_safe::Rect;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_client::Proxy;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

use otto_launcher::apps::Apps;
use otto_launcher::calc::Calculator;
use otto_launcher::source::{rank, Item, Origin, Source};
use otto_launcher::view::{
    field_style, Palette, CARD_W, FIELD_H, MAX_CARD_H, MAX_ROWS, RADIUS, ROW_H,
};
use otto_launcher::windows;

/// How long the scene is kept painting after a change, so the selection's
/// slide and the card's resize are seen through to the end.
const SETTLE: Duration = Duration::from_millis(220);

/// A frame the compositor never answered must not freeze the launcher.
const FRAME_TIMEOUT: Duration = Duration::from_millis(500);

/// How the card arrives and leaves.
///
/// It scales and fades rather than sliding: the launcher appears where it is
/// going to stay. The two are deliberately out of step. The fade is quick — the
/// card should be *there* almost at once, because it is about to be typed into
/// — while the scale takes its time and springs past full size before settling,
/// which is what gives the arrival some life. Leaving is quicker than arriving,
/// and does not bounce: on the way out there is nothing to settle into.
const FADE_IN: Duration = Duration::from_millis(90);
const SCALE_IN: Duration = Duration::from_millis(340);
const FADE_OUT: Duration = Duration::from_millis(90);
const SCALE_OUT: Duration = Duration::from_millis(110);

/// How much the scale overshoots on the way in. Enough to notice, not enough
/// to wobble.
const BOUNCE: f64 = 0.35;

/// When the launcher may stop — the longest thing the exit is waiting on.
const CLOSE: Duration = Duration::from_millis(120);

/// How small the card is before it arrives, and again once it has gone. Near
/// enough to full size that it reads as a swell rather than a zoom.
const OPEN_SCALE: f64 = 0.96;
const CLOSE_SCALE: f64 = 0.96;

struct Launcher {
    /// The fullscreen surface. It carries the scrim, takes the keyboard, and
    /// catches the click that lands outside the card.
    surface: Option<LayerShellSurface>,
    /// The card. A surface of its own so the compositor can frost what is
    /// behind *it* rather than behind the whole screen.
    card: Option<SubsurfaceSurface>,
    palette: Option<Palette>,
    /// Last size handed to the card's style, so an unchanged frame does not
    /// re-send it.
    card_size: (f32, f32),

    sources: Vec<Box<dyn Source>>,
    labels: Vec<&'static str>,
    /// Everything the sources have, which is what a query is ranked against.
    items: Vec<Item>,
    /// What is shown before anything is typed — the last few applications
    /// launched, and every window in the switcher.
    resting: Vec<Item>,
    /// The rows as displayed: answers derived from the query, then either the
    /// resting list or the ranked matches.
    rows: Vec<Item>,

    input: TextInput,
    /// Index into `matches` of the highlighted row.
    selected: usize,
    /// First row on screen — the list scrolls past [`MAX_ROWS`].
    offset: usize,

    shift: bool,
    sized: bool,
    /// Set once the launcher has been interacted with, after which losing the
    /// keyboard means "gone" rather than "not arrived yet".
    engaged: bool,

    /// Set once the card has something on it and the entrance has been
    /// started, so it is started exactly once.
    opened: bool,
    /// When the closing animation will have finished, and the launcher can
    /// stop. Input is ignored from the moment this is set.
    closing_at: Option<Instant>,

    dirty: bool,
    painted_at: Option<Instant>,
    settle_until: Option<Instant>,
    last_tick: Instant,
}

/// The query field's identity for assistive technologies.
const FIELD: FocusId = FocusId::from_raw(0xF1E1_D000);
/// The results list's.
const RESULTS: FocusId = FocusId::from_raw(0xF1E1_D001);

/// One result row's, by its position in the list.
fn row_focus(index: usize) -> FocusId {
    FocusId::new(format!("row-{index}"))
}

/// Which sources a run offers. Everything, by default; the single-kind scopes
/// exist so a binding can mean "switch window" specifically, the way rofi's
/// `-show window` does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Everything,
    Apps,
    Windows,
}

impl Scope {
    /// What the field says when it is empty. It names the mode, because the
    /// two bindings mean different things and the field is the only place that
    /// says which one is up.
    fn placeholder(self) -> &'static str {
        match self {
            Scope::Everything => otto_kit::t!("launcher-search-everything"),
            Scope::Apps => otto_kit::t!("launcher-search-apps"),
            Scope::Windows => otto_kit::t!("launcher-search-windows"),
        }
    }
}

/// When the process started, for the startup timings. A launcher is judged on
/// how long it takes to appear, so the stages that make up that time are
/// measurable without a profiler.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn since_start() -> u128 {
    STARTED.get_or_init(Instant::now).elapsed().as_millis()
}

impl Launcher {
    /// `query` is what the field starts with — the command line's optional
    /// argument, for a binding that opens the launcher already narrowed.
    fn new(query: &str, scope: Scope) -> Self {
        let mut sources: Vec<Box<dyn Source>> = Vec::new();
        let mut labels: Vec<&'static str> = Vec::new();

        if scope != Scope::Windows {
            let apps = Apps::load(sources.len());
            tracing::debug!(ms = since_start(), "desktop entries scanned");
            labels.push(apps.label());
            sources.push(Box::new(apps));

            let calculator = Calculator::new(sources.len());
            labels.push(calculator.label());
            sources.push(Box::new(calculator));
        }

        if scope != Scope::Apps {
            let started = Instant::now();
            let connected = windows::Windows::connect(sources.len(), scope == Scope::Windows);
            tracing::debug!(ms = started.elapsed().as_millis(), "toplevels listed");
            match connected {
                Some(windows) => {
                    labels.push(windows.label());
                    sources.push(Box::new(windows));
                }
                None => tracing::info!("no foreign-toplevel protocol; windows will not be listed"),
            }
        }

        let mut input = TextInput::editing(query, field_style(dark()));
        input.state.placeholder = scope.placeholder().to_string();
        // Without a box to be laid out in, the field scrolls the text it is
        // given out of its own (zero-width) clip and draws nothing.
        input.set_size(CARD_W, FIELD_H);

        Self {
            surface: None,
            card: None,
            palette: None,
            card_size: (0.0, 0.0),
            sources,
            labels,
            items: Vec::new(),
            resting: Vec::new(),
            rows: Vec::new(),
            input,
            selected: 0,
            offset: 0,
            shift: false,
            sized: false,
            engaged: false,
            opened: false,
            closing_at: None,
            dirty: true,
            painted_at: None,
            settle_until: None,
            last_tick: Instant::now(),
        }
    }

    /// Ask every source for its items again, keeping the selection on the same
    /// item where that is still possible.
    fn reload(&mut self) {
        let previous = self.selected_origin();
        self.items = self
            .sources
            .iter_mut()
            .flat_map(|source| source.items())
            .collect();
        self.resting = self
            .sources
            .iter_mut()
            .flat_map(|source| source.resting())
            .collect();
        self.refilter();

        if let Some(origin) = previous {
            if let Some(position) =
                (0..self.row_count()).find(|row| self.row(*row).is_some_and(|i| i.origin == origin))
            {
                self.selected = position;
                self.scroll_to_selection();
            }
        }
    }

    fn refilter(&mut self) {
        let query = self.input.value().to_string();

        let mut rows: Vec<Item> = self
            .sources
            .iter_mut()
            .filter_map(|source| source.answer(&query))
            .collect();

        if query.trim().is_empty() {
            // Nothing typed: the sources' own idea of what is worth showing,
            // not everything they have.
            rows.extend(self.resting.iter().cloned());
        } else {
            rows.extend(
                rank(&self.items, &query)
                    .into_iter()
                    .filter_map(|matched| self.items.get(matched.index).cloned()),
            );
        }

        self.rows = rows;
        self.selected = 0;
        self.offset = 0;
        self.dirty = true;
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row(&self, index: usize) -> Option<&Item> {
        self.rows.get(index)
    }

    fn selected_origin(&self) -> Option<Origin> {
        Some(self.row(self.selected)?.origin)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.row_count() == 0 {
            return;
        }
        let count = self.row_count() as isize;
        // Wrapping, because a list that stops at the end makes someone check
        // where the end was.
        self.selected = (self.selected as isize + delta).rem_euclid(count) as usize;
        self.scroll_to_selection();
        self.dirty = true;
    }

    fn scroll_to_selection(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + MAX_ROWS {
            self.offset = self.selected + 1 - MAX_ROWS;
        }
    }

    /// Carry out the selection and leave. A source that refuses says why and
    /// the launcher stays up, because the alternative is vanishing without
    /// having done anything.
    fn activate(&mut self) {
        let Some(origin) = self.selected_origin() else {
            return;
        };
        let Some(source) = self.sources.get_mut(origin.source) else {
            return;
        };
        match source.activate(origin.index) {
            Ok(()) => self.close(),
            Err(err) => {
                tracing::error!(%err, "could not activate the selection");
                self.dirty = true;
            }
        }
    }

    // === Painting ===

    fn push(&mut self) {
        if !self.sized {
            return;
        }
        // Destructured so the rows and the palette borrow different fields:
        // the references handed to the palette point into `rows`, and they have
        // to outlive the call that draws them.
        let Self {
            rows,
            palette,
            input,
            labels,
            offset,
            selected,
            ..
        } = self;
        let Some(palette) = palette.as_mut() else {
            return;
        };
        let shown: Vec<&Item> = rows.iter().collect();
        // "No results" answers a question. Nothing has been asked yet when the
        // query is empty, and the launcher has nothing to report.
        let empty_message = (!input.value().trim().is_empty()).then_some("No results");
        palette.update(input, &shown, labels, *offset, *selected, empty_message);

        let size = palette.card_size();
        self.settle_until = Some(Instant::now() + SETTLE);
        if size != self.card_size {
            self.card_size = size;
            self.resize_card(size);
        }
    }

    /// Tell the compositor how much of the card's buffer is card. The frost,
    /// the rounding and the shadow follow this rectangle, so a list that grew
    /// or shrank has to say so or the material keeps the old shape.
    fn resize_card(&self, (width, height): (f32, f32)) {
        let (Some(card), Some(palette)) = (self.card.as_ref(), self.palette.as_ref()) else {
            return;
        };
        let Some(style) = card.base_surface().surface_style() else {
            return;
        };
        // Surface-style geometry is in physical pixels.
        let scale = AppContext::fractional_scale();
        let (x, y) = palette.card_origin();
        // The anchor is the top centre, and position is measured from it.
        style.set_position((x + width / 2.0) as f64 * scale, y as f64 * scale);
        style.set_size(width as f64 * scale, height as f64 * scale);
        // And the subsurface itself follows, in logical points. The style moves
        // where the card is *drawn*; this is where the compositor looks for it
        // when the pointer is over it, and pointer positions arrive relative to
        // it. Left behind at the parent's origin, the card would be hovered
        // three rows away from the cursor.
        card.set_position(x as i32, y as i32);
    }

    /// Let the card in, once there is something drawn on it.
    ///
    /// Not before: the entrance animates a surface, and a surface with no
    /// buffer yet would fade in as an empty rectangle and then fill.
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

    /// Start closing, and stop the launcher once the card has gone.
    ///
    /// Whatever was chosen has already happened by this point — the
    /// application is starting, the window is being focused — so the animation
    /// costs nothing but the launcher's own last hundred milliseconds.
    fn close(&mut self) {
        if self.closing_at.is_some() {
            return;
        }
        self.closing_at = Some(Instant::now() + CLOSE);

        let Some(style) = self
            .card
            .as_ref()
            .and_then(|card| card.base_surface().surface_style())
        else {
            AppContext::request_exit();
            return;
        };
        animate(FADE_OUT, Curve::Preset(Preset::EaseInQuad), || {
            style.set_opacity(0.0);
        });
        animate(SCALE_OUT, Curve::Preset(Preset::EaseInQuad), || {
            style.set_scale(CLOSE_SCALE, CLOSE_SCALE);
        });
        // The transaction is a request like any other, and the launcher is
        // about to stop doing anything else.
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
        tracing::trace!(selected = self.selected, rows = self.rows.len(), "painting");
        // otto-kit hands over a canvas with the buffer scale already applied,
        // so both scenes are drawn in logical points.
        // The parent surface draws nothing: it is there to take the keyboard
        // and to catch the click that lands beside the card.
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });

        if let Some(card) = self.card.as_ref() {
            let base = card.base_surface();
            card.draw(|canvas| {
                // Transparent, not the card's colour: what shows through is
                // the frosted material the compositor put underneath.
                canvas.clear(skia_safe::Color::TRANSPARENT);
                base.render_layer_node(canvas);
            });
        }
    }
}

/// Run `changes` inside an animated transaction of `duration`.
///
/// Every animated property the closure sets joins that one transaction, so
/// properties that should move together do. Properties that should *not* move
/// together — the card's fade and its scale — go in transactions of their own.
fn animate(duration: Duration, curve: Curve, changes: impl FnOnce()) {
    let Some(manager) = AppContext::surface_style_manager() else {
        changes();
        return;
    };
    let qh = AppContext::queue_handle();

    let timing = manager.create_timing_function(qh, ());
    match curve {
        Curve::Preset(preset) => timing.set_preset(preset),
        // The spring is tuned to settle inside the transaction's duration, so
        // the bounce is a shape rather than a length.
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
    /// Overshoots and settles back. The argument is how far.
    Spring(f64),
}

/// How solid the card runs, whatever the theme's popup material says.
///
/// There is a ceiling worth staying under: past roughly this the frost stops
/// reading as frost and the card may as well be opaque, which throws away the
/// blur the compositor is doing anyway.
const CARD_MIN_ALPHA: u8 = 0xD8;

fn at_least_opaque(colour: skia_safe::Color, min_alpha: u8) -> skia_safe::Color {
    skia_safe::Color::from_argb(
        colour.a().max(min_alpha),
        colour.r(),
        colour.g(),
        colour.b(),
    )
}

/// The card's frost colour, which is the one part of the material that follows
/// the colour scheme. Split out of `apply_card_material` so a theme that lands
/// after the card exists can be applied without also rewinding the entrance
/// state that function sets.
fn apply_card_colour(card: &SubsurfaceSurface) {
    let Some(style) = card.base_surface().surface_style() else {
        tracing::warn!("no otto-surface-style; the card will not be frosted");
        return;
    };
    // `material_popup` is the token the bar's menus use — the launcher is the
    // same kind of thing, floating over whatever happens to be behind it. It is
    // taken up to at least `CARD_MIN_ALPHA` on top of that: the card is large
    // and full of small text, and a busy desktop showing through it costs more
    // legibility than the frost gives back.
    let colour = skia_safe::Color4f::from(at_least_opaque(
        AppContext::current_theme().material_popup,
        CARD_MIN_ALPHA,
    ));
    style.set_background_color(
        colour.r as f64,
        colour.g as f64,
        colour.b as f64,
        colour.a as f64,
    );
}

/// Ask the compositor for the card's material: the frost, and the shape it is
/// cut to.
///
/// None of this can be drawn client-side. A blur needs the pixels behind the
/// surface, and the only process that has them is the compositor — so the card
/// declares what it wants to look like and paints its text on top of the
/// result. `material_medium` is the same token the bar's menus use, so the
/// launcher reads as the same kind of surface as the rest of the desktop.
fn apply_card_material(card: &SubsurfaceSurface) {
    apply_card_colour(card);
    let Some(style) = card.base_surface().surface_style() else {
        return;
    };
    style.set_blend_mode(BlendMode::BackgroundBlur);
    style.set_corner_radius(RADIUS as f64 * AppContext::fractional_scale());
    style.set_masks_to_bounds(ClipMode::Enabled);
    style.set_shadow(0.32, 32.0, 0.0, 12.0, 0.0, 0.0, 0.0);
    // The buffer is the card at its tallest; a shorter card shows the top of
    // it, so the field stays where it is as rows come and go.
    style.set_contents_gravity(ContentsGravity::TopLeft);

    // Transforms are taken about the top centre. Centre horizontally so the
    // card swells outwards evenly; top vertically so the field stays put both
    // while the card arrives and later when the list grows under it.
    style.set_anchor_point(0.5, 0.0);

    // Where the card starts: just short of full size, and invisible. The
    // entrance animates out of this once there is something on it to see.
    style.set_scale(OPEN_SCALE, OPEN_SCALE);
    style.set_opacity(0.0);
}

/// Whether to draw dark. The colour scheme comes from the desktop portal, the
/// same source otto-kit's other components read.
fn dark() -> bool {
    matches!(
        otto_kit::color_scheme::current_color_scheme(),
        otto_kit::theme::ColorScheme::Dark
    )
}

impl App for Launcher {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(ms = since_start(), "wayland connected");
        // The engine has to exist before the surface does: a surface builds its
        // own root layer node, and that node is what the scene hangs off.
        AppContext::enable_layer_engine(1920.0, 1080.0);
        tracing::debug!(ms = since_start(), "layer engine ready");

        // Anchored to all four edges with a zero size, so the compositor gives
        // us the whole output — the scrim needs it, and so does "click outside
        // the card to dismiss".
        let surface = LayerShellSurface::with_anchor(
            Layer::Overlay,
            "otto-launcher",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            Some(0),
        )?;
        // Visible to assistive technologies from the moment it exists: the
        // launcher is modal and takes every key, so a screen reader that
        // cannot read it cannot tell the user what has taken over the session.
        AppContext::enable_accessibility(&surface.wl_surface().id());

        // Exclusive: the launcher is modal while it is up, and every keystroke
        // belongs to it — including the ones the focused window would want.
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        tracing::debug!(ms = since_start(), "layer surface created");

        // The card is a child surface, sized once to the tallest it can be.
        // Shrinking it is the compositor clipping the same buffer, which costs
        // nothing and keeps the text from being re-laid out as rows come and
        // go.
        let card = SubsurfaceSurface::new(
            surface.base_surface().wl_surface(),
            0,
            0,
            CARD_W as i32,
            MAX_CARD_H as i32,
        )?;
        apply_card_material(&card);

        tracing::debug!(ms = since_start(), "card surface created");
        let Some(engine) = AppContext::layers_renderer(|renderer| renderer.engine().clone()) else {
            return Err("the layers engine is unavailable".into());
        };
        let palette = Palette::new(engine, card.base_surface().layer_node(), dark());

        self.palette = Some(palette);
        self.card = Some(card);
        self.surface = Some(surface);
        tracing::debug!(ms = since_start(), "surfaces created");
        self.reload();
        tracing::debug!(ms = since_start(), "sources loaded");
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        tracing::debug!(width, height, ms = since_start(), "configured");
        if let Some(palette) = self.palette.as_mut() {
            palette.set_size(width as f32, height as f32);
        }
        self.sized = true;
        self.dirty = true;
    }

    /// The portal answers the colour scheme asynchronously, so the launcher is
    /// usually already up — and drawn in the default light — by the time the
    /// answer arrives. Recolour everything that was built from it.
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
        _serial: u32,
    ) {
        // Shift is tracked from the key itself: the runner reports keys, not
        // modifier state, and selection with Shift+arrows needs to know.
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

        // Ctrl combinations arrive as control characters rather than as a
        // modifier flag, which is enough to recognise them by.
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
                self.activate();
                return;
            }
            (Keysym::Down, _) | (_, Some('n')) => {
                self.move_selection(1);
                return;
            }
            (Keysym::Up, _) | (_, Some('p')) => {
                self.move_selection(-1);
                return;
            }
            (Keysym::Tab, _) => {
                self.move_selection(if self.shift { -1 } else { 1 });
                return;
            }
            (Keysym::ISO_Left_Tab, _) => {
                self.move_selection(-1);
                return;
            }
            (Keysym::Page_Down, _) => {
                self.move_selection(MAX_ROWS as isize);
                return;
            }
            (Keysym::Page_Up, _) => {
                self.move_selection(-(MAX_ROWS as isize));
                return;
            }
            // Clear the query without reaching for backspace — the fastest way
            // to start a different search.
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
            TextInputResponse::Commit => self.activate(),
            TextInputResponse::Cancel => self.close(),
            _ => {}
        }
    }

    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {
        // Something else has taken the keyboard. A modal that has lost its
        // input is only in the way — but not before it has ever had it, which
        // is what `engaged` guards against at startup.
        if self.engaged {
            self.close();
        }
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        if self.closing_at.is_some() {
            return;
        }
        let Some(palette) = self.palette.as_ref() else {
            return;
        };
        let card_surface = self.card.as_ref().map(|card| card.wl_surface().clone());
        for event in events {
            // Positions are relative to the surface the pointer is over, and
            // the card is a surface of its own: an event on it is already in
            // card coordinates, an event on the parent is not on the card at
            // all.
            let on_card = card_surface
                .as_ref()
                .is_some_and(|surface| *surface == event.surface);
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Motion { .. } if on_card => {
                    if let Some(row) = palette.row_at(x, y) {
                        let target = self.offset + row;
                        if target < self.row_count() && target != self.selected {
                            self.selected = target;
                            self.dirty = true;
                        }
                    }
                }
                PointerEventKind::Press { .. } => {
                    self.engaged = true;
                    // The card's buffer stays at its full height even when the
                    // list is short, so the surface catches presses under a
                    // card that is not there. Those are beside it, like a press
                    // on the parent — and beside the card is the other way of
                    // saying Escape.
                    let beside = !on_card || y > palette.card_size().1;
                    if beside {
                        self.close();
                        return;
                    }
                }
                PointerEventKind::Release { .. } if on_card => {
                    if let Some(row) = palette.row_at(x, y) {
                        let target = self.offset + row;
                        if target < self.row_count() {
                            self.selected = target;
                            self.activate();
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// What a screen reader reads: the field, and the results under it.
    ///
    /// The launcher already moves its own selection with the arrows, so there
    /// is no traversal ring here — the highlighted row *is* the focus, and
    /// saying so is what makes a screen reader read each result as the user
    /// arrows through them.
    fn accessibility(&mut self, _ctx: &AppContext, _surface: &ObjectId) -> Option<A11yTree> {
        let palette = self.palette.as_ref()?;
        let (card_x, card_y) = palette.card_origin();
        let (card_w, card_h) = palette.card_size();

        let title = self.input.state.placeholder.clone();
        let mut tree = A11yTree::new(title.clone());

        let field = Rect::from_xywh(card_x, card_y, card_w, FIELD_H);
        tree.control(FIELD, field, Role::SearchInput, true, |node| {
            node.set_label(title.clone());
            node.set_value(self.input.value().to_owned());
            node.add_action(Action::SetValue);
        });

        // Only the rows on screen: the list scrolls, and a row that has been
        // scrolled past is not something to point at.
        let first = self.offset;
        let last = (self.offset + MAX_ROWS).min(self.row_count());
        let list = Rect::from_xywh(card_x, card_y + FIELD_H, card_w, card_h - FIELD_H);

        let rows: Vec<(usize, String, Option<String>)> = (first..last)
            .filter_map(|index| {
                let item = self.row(index)?;
                Some((index, item.title.clone(), item.subtitle.clone()))
            })
            .collect();
        let selected = self.selected;

        tree.region(RESULTS, list, Role::ListBox, "Results", |tree| {
            for (index, title, subtitle) in rows {
                let on_screen = index - first;
                let bounds = Rect::from_xywh(
                    card_x,
                    card_y + FIELD_H + on_screen as f32 * ROW_H,
                    card_w,
                    ROW_H,
                );
                tree.control(
                    row_focus(index),
                    bounds,
                    Role::ListBoxOption,
                    true,
                    |node| {
                        node.set_label(title);
                        if let Some(subtitle) = subtitle {
                            node.set_description(subtitle);
                        }
                        node.set_selected(index == selected);
                        node.add_action(Action::Click);
                    },
                );
            }
        });

        if self.row_count() > 0 {
            tree.set_focus(row_focus(selected));
        }

        Some(tree)
    }

    /// A screen reader picked a result: run it, exactly as Enter would.
    fn on_accessibility_action(
        &mut self,
        _ctx: &AppContext,
        _surface: &ObjectId,
        request: &ActionRequest,
    ) {
        if !matches!(request.action, Action::Click) {
            return;
        }
        let target = (0..self.row_count()).find(|index| {
            otto_kit::accessibility::node_id(row_focus(*index)) == request.target_node
        });
        let Some(index) = target else { return };

        self.selected = index;
        self.activate();
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        // The card has gone; nothing is left to do but stop.
        if let Some(at) = self.closing_at {
            if Instant::now() >= at {
                AppContext::request_exit();
            }
            return;
        }

        let mut changed = false;
        for source in self.sources.iter_mut() {
            source.pump();
            changed |= source.changed();
        }
        if changed {
            self.reload();
        }

        // The caret blinks on its own clock; the field asks to be redrawn when
        // the phase flips.
        let now = Instant::now();
        let was_visible = self.input.caret_visible();
        self.input
            .tick(now.duration_since(self.last_tick).as_secs_f32());
        self.last_tick = now;
        if self.input.caret_visible() != was_visible {
            self.dirty = true;
        }

        if self.dirty {
            self.dirty = false;
            self.push();
            self.paint();
            // The first frame is on the card, so it has something to arrive
            // with.
            self.open();
            tracing::debug!(ms = since_start(), "first frame");
            return;
        }

        if self.frame_in_flight() {
            return;
        }
        // Keep painting while a transition is still running.
        if self.settle_until.is_some_and(|until| now < until) {
            self.paint();
        } else {
            self.settle_until = None;
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        if self.closing_at.is_some() {
            return Some(Duration::from_millis(8));
        }
        Some(if self.settle_until.is_some() {
            Duration::from_millis(8)
        } else {
            // Half a blink period: the slowest the loop may sleep and still
            // turn the caret on and off on time.
            Duration::from_secs_f32(CARET_BLINK_PERIOD / 2.0)
        })
    }

    fn poll_fds(&self) -> Vec<RawFd> {
        self.sources.iter().filter_map(|s| s.poll_fd()).collect()
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

    // Before the first string is looked up, and before the window is drawn: a
    // launcher is judged on how fast it appears, and this is one round trip
    // that has to finish first either way.
    otto_kit::i18n::init_from_desktop();

    // `--apps` / `--windows` narrow what is offered; anything else on the
    // command line is the initial query, so a binding can open the launcher
    // already filtered.
    // Apps unless asked otherwise: the two bindings are "launch something" and
    // "switch to a window", and a mode that quietly does both is neither.
    let mut scope = Scope::Apps;
    let mut words: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--apps" | "-a" => scope = Scope::Apps,
            "--windows" | "-w" => scope = Scope::Windows,
            "--all" => scope = Scope::Everything,
            "--help" | "-h" => {
                println!("usage: otto-launcher [--apps|--windows|--all] [query]");
                return Ok(());
            }
            _ => words.push(arg),
        }
    }
    AppRunner::new(Launcher::new(&words.join(" "), scope)).run()?;
    Ok(())
}
