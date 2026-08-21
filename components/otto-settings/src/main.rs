//! Settings app for Otto — see `specs/settings-app.md`.
//!
//! Nothing is wired to the compositor yet: the values come from `model.rs`
//! and changing a control does nothing. What works is the window itself and
//! moving between panes, so the layout can be judged in place.
//!
//! ```sh
//! cargo run -p otto-settings                    # open the window
//! cargo run -p otto-settings -- --png out.png   # offscreen render instead
//! ```

mod discovery;
mod file_picker;
mod glyphs;
mod model;
mod panes;
mod preview;
mod settings_client;
mod view;
mod widgets;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use otto_kit::components::color_picker::{ColorPickerPopup, Swatch};
use otto_kit::components::dropdown::DropdownMenu;
use otto_kit::components::scroll::ScrollSurfaces;
use otto_kit::components::titlebar::{WindowControl, WindowControlsState};
use otto_kit::components::window::resize;
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1;
use otto_kit::CursorShape;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::seat::pointer::{AxisScroll, PointerEventKind};
use smithay_client_toolkit::shell::xdg::XdgSurface;
use view::{Settings, WINDOW_H, WINDOW_W};

struct SettingsApp {
    window: Option<Window>,
    /// Shared with the draw and pointer callbacks, which outlive this struct's
    /// borrow of itself.
    selected: Arc<Mutex<usize>>,
    /// The pane content's scroll position. Lives here rather than inside
    /// `Settings` because `Settings` is rebuilt fresh every frame, while the
    /// scroll offset — like `selected` — has to survive across frames.
    scroll: Arc<Mutex<ScrollView>>,
    /// The subsurfaces the pane is drawn into, so the compositor does the
    /// scrolling. `None` until the window exists to parent them to.
    surfaces: Rc<RefCell<Option<ScrollSurfaces>>>,
    /// Set when something other than the scroll changed what the pane should
    /// look like, so the next update repaints its band. See
    /// [`SettingsApp::sync_pane`].
    pane_dirty: Arc<Mutex<bool>>,
    /// Identifier of the slider currently being dragged, if any.
    dragging: Arc<Mutex<Option<String>>>,
    /// Switches whose knob is mid-flip, keyed by setting id. A switch is in
    /// here only while it is moving: the value itself changed on the press,
    /// this is the travel between the two ends. See [`toggle::Flip`].
    toggle_flips: Arc<Mutex<HashMap<&'static str, toggle::Flip>>>,
    /// One menu per pop-up button, built once at startup and reused.
    ///
    /// `ContextMenu::new` registers a pointer callback that can never be
    /// unregistered, and building one inside a pointer handler deadlocks on
    /// `AppContext`'s callback list — so these are constructed eagerly, before
    /// any event can arrive. See `otto_kit::components::dropdown::menu`.
    dropdowns: Rc<HashMap<&'static str, DropdownMenu>>,
    /// Identifier of the dropdown whose menu is up, so its field draws open.
    open_dropdown: Arc<Mutex<Option<&'static str>>>,
    /// One picker per colour well, built once at startup for the same reason
    /// the dropdown menus are.
    pickers: Rc<HashMap<&'static str, ColorPickerPopup>>,
    /// Identifier of the well whose picker is up.
    open_picker: Arc<Mutex<Option<&'static str>>>,
    /// The surface's current size, from the last configure.
    ///
    /// The draw closure has to be `Send`, and `Window` is not, so the size
    /// travels through here rather than being read off the window inside it.
    size: Arc<Mutex<(f32, f32)>>,
    /// Hover and press state of the traffic lights, shared between the window's
    /// pointer handler and its draw closure.
    controls: Arc<Mutex<WindowControlsState>>,
}

/// Every setting in the app that a colour well edits.
fn color_ids() -> Vec<&'static str> {
    let mut ids = Vec::new();
    for pane in model::panes() {
        for group in pane.groups {
            for row in group.rows {
                if let (Some(id), model::Control::Color(_)) = (row.id, &row.control) {
                    ids.push(id);
                }
            }
        }
    }
    ids
}

/// The presets a colour well offers.
///
/// `accent_color` is an enumeration on the compositor side, so its swatches
/// come from the served schema and picking anything else would be refused.
/// A free-form colour setting gets a general palette instead.
fn swatches_for(id: &str) -> Vec<Swatch> {
    if let Some(desc) = settings_client::describe(id) {
        if !desc.choices.is_empty() {
            return desc
                .choices
                .iter()
                .filter_map(|name| named_color(name).map(|c| Swatch::new(name.clone(), c)))
                .collect();
        }
    }

    [
        ("Blue", 0xFF0A84FF),
        ("Purple", 0xFFBF5AF2),
        ("Pink", 0xFFFF375F),
        ("Red", 0xFFFF453A),
        ("Orange", 0xFFFF9F0A),
        ("Yellow", 0xFFFFD60A),
        ("Green", 0xFF32D74B),
        ("Teal", 0xFF40C8E0),
        ("Graphite", 0xFF8E8E93),
    ]
    .into_iter()
    .map(|(name, argb)| Swatch::new(name, Color::from(argb)))
    .collect()
}

/// The compositor names its accent colours rather than taking hex, so a name
/// has to map to something drawable.
fn named_color(name: &str) -> Option<Color> {
    model::named_argb(name).map(Color::from)
}

/// Open a colour well's picker.
///
/// What a change sends depends on the setting: an enumerated one (accent
/// colour) takes the swatch's NAME, a free-form one takes hex. Sending hex to
/// an enumerated setting would be refused with `OutOfRange`.
fn open_picker_for(
    pickers: &HashMap<&'static str, ColorPickerPopup>,
    open_picker: &Arc<Mutex<Option<&'static str>>>,
    pane_dirty: &Arc<Mutex<bool>>,
    window: &Window,
    hit: view::ColorHit,
    serial: u32,
    dark: bool,
) {
    let Some(picker) = pickers.get(hit.id) else {
        return;
    };
    // Clicking the well again closes the picker, rather than doing nothing
    // because a popup is already up.
    if picker.is_open() {
        picker.close();
        *open_picker.lock().unwrap() = None;
        mark_pane_dirty(pane_dirty);
        window.request_frame();
        return;
    }
    let Some(parent) = window
        .surface()
        .map(|s| s.xdg_window().xdg_surface().clone())
    else {
        return;
    };

    let id = hit.id;
    let enumerated = settings_client::describe(id)
        .map(|d| !d.choices.is_empty())
        .unwrap_or(false);

    let changed_window = window.clone();
    let changed_dirty = pane_dirty.clone();
    let closed_open = open_picker.clone();
    let closed_dirty = pane_dirty.clone();
    let closed_window = window.clone();

    *open_picker.lock().unwrap() = Some(id);

    picker.open(
        &parent,
        hit.rect,
        serial,
        hit.current,
        if dark { Theme::dark() } else { Theme::light() },
        move |color| {
            let value = if enumerated {
                let Some(name) = swatch_name_for(id, color) else {
                    // Dragging in HSV cannot name a colour, and this setting
                    // only accepts names — ignore rather than be refused.
                    return;
                };
                settings_client::Value::Text(name)
            } else {
                settings_client::Value::Text(format!(
                    "#{:02X}{:02X}{:02X}",
                    color.r(),
                    color.g(),
                    color.b()
                ))
            };
            apply(id, value);
            mark_pane_dirty(&changed_dirty);
            changed_window.request_frame();
        },
        move || {
            *closed_open.lock().unwrap() = None;
            mark_pane_dirty(&closed_dirty);
            closed_window.request_frame();
        },
    );
}

/// The schema choice whose swatch is exactly `color`, for settings that take
/// a name rather than a value.
fn swatch_name_for(id: &str, color: Color) -> Option<String> {
    swatches_for(id)
        .into_iter()
        .find(|s| s.color == color)
        .map(|s| s.name)
}

/// Every setting in the app that a pop-up button edits.
///
/// Walked once at startup: the menus have to exist before the first pointer
/// event, and rebuilding them per frame is not an option.
fn select_ids() -> Vec<&'static str> {
    let mut ids = Vec::new();
    for pane in model::panes() {
        for group in pane.groups {
            for row in group.rows {
                if let (Some(id), model::Control::Select(_)) = (row.id, &row.control) {
                    ids.push(id);
                }
            }
        }
    }
    ids
}

/// The view model for the pane on screen right now.
///
/// Rebuilt on demand rather than cached: it reads current values out of the
/// settings store, so a `Set` that just landed is reflected immediately.
fn current_settings(
    selected: &Arc<Mutex<usize>>,
    size: &Arc<Mutex<(f32, f32)>>,
    toggle_flips: &Arc<Mutex<HashMap<&'static str, toggle::Flip>>>,
) -> Settings {
    let (w, h) = *size.lock().unwrap();
    let flips = toggle_flips
        .lock()
        .unwrap()
        .iter()
        .map(|(id, flip)| (*id, flip.fraction()))
        .collect();
    Settings::new(
        *selected.lock().unwrap(),
        current_color_scheme() == ColorScheme::Dark,
    )
    .with_size(w, h)
    .with_toggle_flips(flips)
}

/// The pane viewport for the surface's current size, clamped exactly the way
/// [`Settings::with_size`] clamps it so hit-testing and drawing agree on where
/// the pane is.
fn viewport_for(size: &Arc<Mutex<(f32, f32)>>) -> Rect {
    let (w, h) = *size.lock().unwrap();
    view::pane_viewport(w.max(view::MIN_W), h.max(view::MIN_H))
}

/// Ask for the pane's band to be repainted on the next update.
///
/// The pane's surfaces repaint only when a scroll runs out of painted content.
/// Anything else that changes what a row looks like — a value applied, a menu
/// opening, the window resizing — has to say so explicitly, or the band keeps
/// showing what it was painted with.
fn mark_pane_dirty(dirty: &Arc<Mutex<bool>>) {
    *dirty.lock().unwrap() = true;
}

/// Feed one axis event to the scroll view.
///
/// Shared by the window and the pane's surfaces: a wheel anywhere over the
/// window scrolls the pane, as it did when the window was a single surface.
fn handle_wheel(scroll: &Mutex<ScrollView>, vertical: &AxisScroll) {
    let mut scroll = scroll.lock().unwrap();
    // A notched wheel reports discrete steps and should move exactly one step
    // per click; a touchpad reports a continuous stream, which is what
    // momentum and rubber banding are for.
    if vertical.stop {
        // Fingers off the touchpad: whatever the gesture was carrying becomes
        // a fling, and anything pulled past an end springs back.
        scroll.on_wheel_end();
    } else if vertical.discrete != 0 {
        scroll.on_wheel_discrete(vertical.absolute as f32);
    } else {
        scroll.on_wheel(vertical.absolute as f32);
    }
}

/// Width of the strip `ScrollSurfaces` gives its scrollbar, in points.
///
/// Private there, so it is restated here: it is the x offset of the scrollbar
/// surface within the pane, and without it an event that landed on the
/// scrollbar cannot be put back into the pane's coordinates.
const THUMB_STRIP_W: f32 = 16.0;

/// Which of the pane's surfaces a pointer event landed on.
///
/// The pane is three subsurfaces (see `ScrollSurfaces`), and the pointer is
/// hit-tested against them rather than against the window, so an event over
/// the pane never reaches the window's own handler and arrives in that
/// surface's local coordinates instead of the window's. Each variant is one
/// way of putting those coordinates back into the pane's own space, which is
/// where every hit test below happens.
enum PaneTarget {
    /// The band holding the content. It is the surface that *moves* to
    /// scroll, so its origin sits `band_origin - offset` points down the
    /// pane — which is also why a coordinate translated through it stays
    /// still when the pointer does, however far the content has scrolled.
    Content,
    /// The scrollbar, which sits over the band and never moves: it is drawn
    /// where it belongs through its style node, while the subsurface the
    /// pointer is hit-tested against stays at the top of the gutter. So its
    /// local y *is* the pane's y, and events on it are handled exactly like
    /// events on the band — the scroll view's own hit tests decide what a
    /// press in the gutter means.
    Thumb,
}

/// The serial of a press, which the compositor requires to open a popup.
fn event_serial(kind: &PointerEventKind) -> u32 {
    match kind {
        PointerEventKind::Press { serial, .. } => *serial,
        _ => 0,
    }
}

/// Open a pop-up button's menu.
///
/// Six settings are `enum` in the served schema and carry their `choices`
/// with them. Everything else a pop-up button edits — fonts, cursor/icon/
/// sound themes, the lock and greeter commands — is `string`, because its
/// valid set depends on what is installed on this machine rather than
/// anything the compositor's schema can declare; those fall back to
/// [`discovery`]. A setting with neither — or a compositor that is not
/// serving a schema at all — gets no menu rather than an empty one.
fn open_menu(
    dropdowns: &HashMap<&'static str, DropdownMenu>,
    open_dropdown: &Arc<Mutex<Option<&'static str>>>,
    pane_dirty: &Arc<Mutex<bool>>,
    window: &Window,
    select: view::SelectHit,
    serial: u32,
) {
    let Some(menu) = dropdowns.get(select.id) else {
        return;
    };
    // Clicking the field again closes the menu, the way clicking a colour
    // well again closes its picker — without this the press is swallowed by
    // the menu already being up, and a pop-up button becomes the one control
    // in the pane that cannot undo its own click.
    if menu.is_open() {
        menu.close();
        *open_dropdown.lock().unwrap() = None;
        mark_pane_dirty(pane_dirty);
        window.request_frame();
        return;
    }
    let Some(desc) = settings_client::describe(select.id) else {
        return;
    };
    let Some(parent) = window
        .surface()
        .map(|s| s.xdg_window().xdg_surface().clone())
    else {
        return;
    };

    let choices: Vec<discovery::Choice> = if !desc.choices.is_empty() {
        desc.choices
            .iter()
            .map(|c| discovery::Choice {
                label: desc.display(c),
                value: c.clone(),
            })
            .collect()
    } else {
        match discovery::choices_for(select.id, &select.current) {
            Some(choices) => choices,
            None => return,
        }
    };
    if choices.is_empty() {
        return;
    }

    let selected = choices.iter().position(|c| c.value == select.current);
    let id = select.id;

    let chosen_open = open_dropdown.clone();
    let chosen_dirty = pane_dirty.clone();
    let chosen_window = window.clone();
    // The menu shows `label`s but applies `value`s — they differ only for
    // the synthetic "Automatic" entry a discovered dropdown may add.
    let labels: Vec<String> = choices.iter().map(|c| c.label.clone()).collect();
    let values: Vec<String> = choices.into_iter().map(|c| c.value).collect();

    let dismissed_open = open_dropdown.clone();
    let dismissed_dirty = pane_dirty.clone();
    let dismissed_window = window.clone();

    *open_dropdown.lock().unwrap() = Some(id);

    menu.open(
        &parent,
        select.rect,
        serial,
        &labels,
        selected,
        move |index| {
            if let Some(value) = values.get(index) {
                apply(id, settings_client::Value::Text(value.clone()));
            }
            *chosen_open.lock().unwrap() = None;
            mark_pane_dirty(&chosen_dirty);
            chosen_window.request_frame();
        },
        move || {
            *dismissed_open.lock().unwrap() = None;
            mark_pane_dirty(&dismissed_dirty);
            dismissed_window.request_frame();
        },
    );
}

/// Push one change to the compositor, reporting a refusal rather than letting
/// the UI show a value that was never accepted.
fn apply(id: &str, value: settings_client::Value) {
    match settings_client::set(id, value) {
        settings_client::SetOutcome::Applied => {}
        settings_client::SetOutcome::PendingRestart => {
            println!("{id}: saved, takes effect after a restart");
        }
        settings_client::SetOutcome::Failed(why) => {
            eprintln!("{id}: {why}");
        }
    }
}

/// Open the portal's file picker for `id`, on a thread of its own, and apply
/// whatever comes back.
///
/// The call blocks for as long as the dialog is up — minutes, if the user
/// browses — so it cannot happen on the main thread: drawing and input both
/// run there. The thread only ever touches the settings store (which is
/// `RwLock`-guarded) and then wakes the main loop, which is the same
/// arrangement `settings_client::spawn_change_listener` uses and is
/// documented as safe from any thread.
fn open_file_picker(id: &'static str) {
    let spawned = std::thread::Builder::new()
        .name("file-picker".into())
        .spawn(move || {
            // Wallpapers are the only file setting so far, so the filter is
            // written here. When a second one appears this belongs on the row.
            let filters: &[(&str, &[&str])] = &[
                (
                    "Images",
                    &["*.png", "*.jpg", "*.jpeg", "*.webp", "*.bmp", "*.gif"],
                ),
                ("All Files", &["*"]),
            ];

            match file_picker::open_file("Choose a Background Image", filters) {
                file_picker::Outcome::Chosen(paths) => {
                    if let Some(path) = paths.first() {
                        apply(
                            id,
                            settings_client::Value::Text(path.to_string_lossy().into_owned()),
                        );
                    }
                }
                file_picker::Outcome::Dismissed => {}
                file_picker::Outcome::Failed(why) => {
                    eprintln!("{id}: the file picker failed ({why})");
                }
            }
            // Either way the row may need redrawing, and the main thread is
            // asleep in `poll` until something says otherwise.
            AppContext::request_wakeup();
        });

    if let Err(err) = spawned {
        eprintln!("{id}: cannot start the file picker ({err})");
    }
}

impl SettingsApp {
    /// Bring the pane's surfaces in line with the model: where the viewport
    /// is, how tall the content is, and where the scroll has put it.
    ///
    /// `repaint` says the content itself changed — a value applied, a menu
    /// opened, the window resized — rather than merely scrolled. The surfaces
    /// repaint their band only when a scroll runs off its edge, which is the
    /// whole point of them and no help at all here, so anything else that
    /// changes how the pane looks has to invalidate it explicitly.
    fn sync_pane(&mut self, repaint: bool) {
        let settings = current_settings(&self.selected, &self.size, &self.toggle_flips)
            .with_open_dropdown(*self.open_dropdown.lock().unwrap())
            .with_open_picker(*self.open_picker.lock().unwrap());

        let mut scroll = self.scroll.lock().unwrap();
        // The scroll view drives surfaces that live inside the pane, so its
        // viewport is the pane's own, not the window-local one the all-in-one
        // render path uses.
        scroll.set_viewport(settings.local_viewport());
        // Re-measured every time rather than cached: it is cheap, and it keeps
        // the scroll range correct if the pane's content changes shape (a row
        // gaining a detail line, say) without the window resizing.
        scroll.set_content_length(settings.pane_content_height());

        let mut guard = self.surfaces.borrow_mut();
        let Some(surfaces) = guard.as_mut() else {
            return;
        };

        surfaces.set_viewport(settings.viewport());
        // The pane's ground lives in the clip surface, which is painted once
        // and then left alone, so a light/dark switch has to be pushed in or
        // it keeps the old scheme under freshly themed content.
        surfaces.set_background(view::pane_background(settings.dark));
        if repaint {
            surfaces.invalidate();
        }

        surfaces.sync(&scroll, &settings.theme, |canvas, band| {
            if std::env::var_os("OTTO_PANE_DEBUG").is_some() {
                eprintln!(
                    "[panedbg] band {:.0}..{:.0} offset={:.0} vp={:?} content_h={:.0} width={:.0}",
                    band.top,
                    band.bottom,
                    scroll.offset(),
                    settings.local_viewport(),
                    settings.pane_content_height(),
                    settings.width
                );
            }
            settings.render_content(canvas, band)
        });
    }
}

impl App for SettingsApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Settings", WINDOW_W as i32, WINDOW_H as i32)?;
        window.set_background(Color::TRANSPARENT);
        window.set_min_size(view::MIN_W as u32, view::MIN_H as u32);

        // The window paints its own rounded body, so ask the compositor to
        // clip and round the surface to match.
        // The sidebar is drawn as a translucent material, so ask the
        // compositor to blur what is behind the surface. Whether that request
        // was honoured is not reported back, so the view is told blur is on
        // only when the style object exists at all.
        // `OTTO_SETTINGS_NO_BLUR=1` paints the sidebar flat instead, which is
        // the only way to tell a working blur from a missing one when the
        // backdrop happens to be a flat colour.
        let want_blur = std::env::var("OTTO_SETTINGS_NO_BLUR").is_err();
        if window.surface_style().is_none() {
            eprintln!("settings: no surface style — sidebar cannot be a material");
        }
        let mut blurred = false;
        if let Some(style) = window.surface_style() {
            style.set_corner_radius(view::CORNER as f64);
            style.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
            // The pane scrolls in subsurfaces that sit over the window's own
            // buffer, so the window's rounded outline does not contain them:
            // without this the content runs square into the bottom-right
            // corner. Clipping the descendants to the window's style bounds
            // rounds them with it.
            style.set_clip_children(otto_surface_style_v1::ClipMode::Enabled);
            if want_blur {
                style.set_blend_mode(otto_surface_style_v1::BlendMode::BackgroundBlur);
                blurred = true;
            }
            eprintln!("settings: surface style present, blur requested = {want_blur}");
        }

        // Built here, at window setup, and never later: a `DropdownMenu`
        // constructed from inside a pointer handler deadlocks on
        // `AppContext`'s callback list.
        self.dropdowns = Rc::new(
            select_ids()
                .into_iter()
                .map(|id| (id, DropdownMenu::new()))
                .collect(),
        );
        self.pickers = Rc::new(
            color_ids()
                .into_iter()
                .map(|id| (id, ColorPickerPopup::new(swatches_for(id))))
                .collect(),
        );

        // The window surface paints the chrome and nothing else. The pane's
        // content lives in its own subsurfaces, which the compositor crops and
        // scrolls, so a frame of scrolling never reaches this closure.
        let selected = self.selected.clone();
        let size = self.size.clone();
        let controls = self.controls.clone();
        window.on_draw(move |canvas| {
            let index = *selected.lock().unwrap();
            let (w, h) = *size.lock().unwrap();
            Settings::new(index, current_color_scheme() == ColorScheme::Dark)
                .with_size(w, h)
                .with_blur(blurred)
                .with_controls(*controls.lock().unwrap())
                .render_chrome(canvas);
        });

        // The pane's surfaces, parented to the window. The background colour
        // they are given is the one the chrome paints under them, so the two
        // agree wherever an overscroll pulls the content away from an edge.
        let parent = window
            .surface()
            .map(|s| s.wl_surface().clone())
            .ok_or("window has no surface to hang the pane from")?;
        let surfaces = ScrollSurfaces::new(
            &parent,
            view::pane_viewport(WINDOW_W, WINDOW_H),
            AppContext::scale_factor() as f32,
            view::pane_background(current_color_scheme() == ColorScheme::Dark),
        )?;
        let content_id = surfaces.content_surface().id();
        let window_id = parent.id();
        *self.surfaces.borrow_mut() = Some(surfaces);

        // What the window surface itself is still pointed at: its resize
        // edges, its titlebar, and the sidebar. Clicking a sidebar row selects
        // that pane.
        let selected = self.selected.clone();
        let scroll = self.scroll.clone();
        let pane_dirty = self.pane_dirty.clone();
        let size_hit = self.size.clone();
        let controls_hit = self.controls.clone();
        let redraw = window.clone();
        window.on_pointer_event(move |events| {
            let mut needs_redraw = false;
            for event in events {
                let (x, y) = event.position;
                let (x, y) = (x as f32, y as f32);
                match &event.kind {
                    PointerEventKind::Press { serial, .. } => {
                        // The window draws its own decoration, so its edges
                        // are its own resize handles too.
                        let (win_w, win_h) = *size_hit.lock().unwrap();
                        if let Some(edge) = resize::edge_at(Rect::from_wh(win_w, win_h), x, y) {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                redraw.start_resize(&seat, *serial, edge);
                            }
                            continue;
                        }

                        // The titlebar is the app's own decoration, so moving
                        // and closing the window are the app's job too.
                        if let Some(hit) = view::titlebar_hit(x, y, size_hit.lock().unwrap().0) {
                            match hit {
                                view::TitlebarHit::Control(control) => {
                                    // Arming rather than acting: a control
                                    // fires on release, and only over the dot
                                    // the press landed on.
                                    controls_hit.lock().unwrap().on_press(Some(control));
                                    needs_redraw = true;
                                }
                                view::TitlebarHit::Drag => {
                                    if let Some(seat) = AppContext::seat_state().seats().next() {
                                        redraw.start_move(&seat, *serial);
                                    }
                                }
                            }
                            continue;
                        }

                        // Everything else the window still owns is the
                        // sidebar: the pane is hit-tested on its own surface,
                        // further down.
                        if let Some(index) = view::pane_at(x, y) {
                            let mut current = selected.lock().unwrap();
                            if *current != index {
                                *current = index;
                                // A different pane has an unrelated content
                                // height, so its old scroll position means
                                // nothing here.
                                scroll.lock().unwrap().state.set_offset(0.0);
                                mark_pane_dirty(&pane_dirty);
                                needs_redraw = true;
                            }
                        }
                    }
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        // Show which way an edge will move before it is
                        // grabbed; anywhere else is the ordinary pointer.
                        let (win_w, win_h) = *size_hit.lock().unwrap();
                        match resize::edge_at(Rect::from_wh(win_w, win_h), x, y) {
                            Some(edge) => AppContext::set_cursor_shape(edge.cursor()),
                            None => AppContext::set_cursor_shape(CursorShape::Default),
                        }

                        // The traffic lights reveal their glyphs while the
                        // pointer is over the group.
                        let control = view::titlebar_control_at(x, y, win_w);
                        if controls_hit.lock().unwrap().on_motion(control) {
                            needs_redraw = true;
                        }
                    }
                    // A wheel over the chrome still scrolls the pane, as it
                    // did when the window was a single surface.
                    PointerEventKind::Axis { vertical, .. } => handle_wheel(&scroll, vertical),
                    PointerEventKind::Release { .. } => {
                        let win_w = size_hit.lock().unwrap().0;
                        let control = view::titlebar_control_at(x, y, win_w);
                        let fired = {
                            let mut state = controls_hit.lock().unwrap();
                            if state.pressed().is_some() {
                                needs_redraw = true;
                            }
                            state.on_release(control)
                        };
                        match fired {
                            Some(WindowControl::Close) => std::process::exit(0),
                            Some(WindowControl::Minimize) => redraw.minimize(),
                            Some(WindowControl::Zoom) => redraw.toggle_maximized(),
                            None => {}
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        // The pointer is off the window, so nothing in the bar
                        // is hovered any more — without this the glyphs stay
                        // drawn on a window that was left behind.
                        if controls_hit.lock().unwrap().on_leave() {
                            needs_redraw = true;
                        }
                    }
                }
            }
            if needs_redraw {
                redraw.request_frame();
            }
        });

        // The pane is no longer part of the window surface, so its events
        // arrive here rather than in the handler above — and in the local
        // coordinates of whichever of its surfaces they landed on.
        let selected = self.selected.clone();
        let scroll = self.scroll.clone();
        let surfaces = self.surfaces.clone();
        let dragging = self.dragging.clone();
        let dropdowns = self.dropdowns.clone();
        let open_dropdown = self.open_dropdown.clone();
        let toggle_flips = self.toggle_flips.clone();
        let pickers = self.pickers.clone();
        let open_picker = self.open_picker.clone();
        let pane_dirty = self.pane_dirty.clone();
        let size_hit = self.size.clone();
        let redraw = window.clone();
        AppContext::register_pointer_callback(move |events| {
            for event in events {
                let surface = event.surface.id();
                let target = if surface == content_id {
                    PaneTarget::Content
                } else if surface == window_id {
                    // The handler above has this one.
                    continue;
                } else if open_dropdown.lock().unwrap().is_some()
                    || open_picker.lock().unwrap().is_some()
                {
                    // A popup of ours is up and handles its own events.
                    continue;
                } else {
                    // `ScrollSurfaces` does not hand out its scrollbar
                    // surface, so it is recognised by elimination: the only
                    // other surface of this app the pointer can be over.
                    PaneTarget::Thumb
                };

                let viewport = viewport_for(&size_hit);
                let (lx, ly) = event.position;
                let (lx, ly) = (lx as f32, ly as f32);
                // Into the pane's own coordinates, which is where the scroll
                // view's viewport, gutter and thumb live.
                let (px, py) = match target {
                    PaneTarget::Content => {
                        let band_origin = surfaces
                            .borrow()
                            .as_ref()
                            .map(|s| s.band_origin())
                            .unwrap_or(0.0);
                        let offset = scroll.lock().unwrap().offset();
                        (lx, ly + band_origin - offset)
                    }
                    PaneTarget::Thumb => (lx + viewport.width() - THUMB_STRIP_W, ly),
                };
                // And on into window coordinates, which is what the pane's
                // hit tests take and what the popups anchor against.
                let (x, y) = (px + viewport.left, py + viewport.top);

                match &event.kind {
                    PointerEventKind::Press { serial, .. } => {
                        // The pane reaches the window's right and bottom
                        // edges, so two of its resize handles are over it.
                        let (win_w, win_h) = *size_hit.lock().unwrap();
                        if let Some(edge) = resize::edge_at(Rect::from_wh(win_w, win_h), x, y) {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                redraw.start_resize(&seat, *serial, edge);
                            }
                            continue;
                        }

                        // A press anywhere in the pane catches an in-flight
                        // fling; on the scrollbar it also starts a drag, and
                        // then it is not a press on a control.
                        if scroll.lock().unwrap().on_pointer_down(px, py) {
                            continue;
                        }

                        // A control in the pane. The settings state is rebuilt
                        // here rather than cached because a successful Set
                        // changes what the next one would hit.
                        let offset = scroll.lock().unwrap().offset();
                        let settings = current_settings(&selected, &size_hit, &toggle_flips);

                        if let Some(color) = settings.color_hit(x, y, offset) {
                            open_picker_for(
                                &pickers,
                                &open_picker,
                                &pane_dirty,
                                &redraw,
                                color,
                                event_serial(&event.kind),
                                settings.dark,
                            );
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(select) = settings.select_hit(x, y, offset) {
                            open_menu(
                                &dropdowns,
                                &open_dropdown,
                                &pane_dirty,
                                &redraw,
                                select,
                                event_serial(&event.kind),
                            );
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(id) = settings.file_hit(x, y, offset) {
                            open_file_picker(id);
                        } else if let Some(hit) = settings.hit(x, y, offset) {
                            // A switch slides to its new position rather than
                            // jumping. Started from where the knob is right
                            // now, so a second click part-way through turns
                            // it back from there.
                            if let settings_client::Value::Bool(on) = hit.value {
                                let mut flips = toggle_flips.lock().unwrap();
                                let current = flips
                                    .get(hit.id)
                                    .map(|flip| flip.fraction())
                                    .unwrap_or_else(|| toggle::knob_fraction_for(!on));
                                flips.insert(hit.id, toggle::Flip::start(current, on));
                            }
                            apply(hit.id, hit.value);
                            *dragging.lock().unwrap() = hit.draggable.then(|| hit.id.to_string());
                            mark_pane_dirty(&pane_dirty);
                        }
                    }
                    PointerEventKind::Release { .. } => {
                        scroll.lock().unwrap().on_pointer_up();
                        *dragging.lock().unwrap() = None;
                    }
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        let (win_w, win_h) = *size_hit.lock().unwrap();
                        match resize::edge_at(Rect::from_wh(win_w, win_h), x, y) {
                            Some(edge) => AppContext::set_cursor_shape(edge.cursor()),
                            None => AppContext::set_cursor_shape(CursorShape::Default),
                        }

                        {
                            let mut scroll = scroll.lock().unwrap();
                            // Hovering the scrollbar keeps it up and widens
                            // it; dragging it moves the content.
                            scroll.on_pointer_move(px, py);
                            scroll.on_pointer_drag(px, py);
                        }
                        let held = dragging.lock().unwrap().clone();
                        if let Some(id) = held {
                            let settings = current_settings(&selected, &size_hit, &toggle_flips);
                            if let Some(value) = settings.drag_value(&id, x) {
                                apply(&id, value);
                                mark_pane_dirty(&pane_dirty);
                            }
                        }
                    }
                    PointerEventKind::Axis { vertical, .. } => handle_wheel(&scroll, vertical),
                    PointerEventKind::Leave { .. } => {
                        scroll.lock().unwrap().on_pointer_leave();
                    }
                }
            }
        });

        self.window = Some(window);
        Ok(())
    }

    /// The compositor decides the surface's size; the app follows it.
    fn on_configure(
        &mut self,
        _ctx: &AppContext,
        configure: smithay_client_toolkit::shell::xdg::window::WindowConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // A configure with no size is the compositor letting the client
        // choose, so keep what we have.
        let width = w
            .map(|v| v.get() as f32)
            .unwrap_or(self.size.lock().unwrap().0);
        let height = h
            .map(|v| v.get() as f32)
            .unwrap_or(self.size.lock().unwrap().1);

        // Configures arrive for far more than resizes — activation, tiling
        // state, a bare re-configure with the size we already have — and the
        // compositor sends them steadily while the window is up. Only a size
        // that actually moved changes what the pane looks like, so anything
        // else must not repaint: the band is the expensive thing here, and
        // repainting it on every configure is what the cropped scrolling path
        // exists to avoid.
        let mut size = self.size.lock().unwrap();
        if *size == (width, height) {
            return;
        }
        *size = (width, height);
        drop(size);

        // The pane's surfaces are placed against the window's size, so they
        // have to be told about the new one and repainted at it.
        mark_pane_dirty(&self.pane_dirty);
        window.request_frame();
    }

    /// Runs once per event loop iteration, on the main thread. This is where
    /// a `Changed` signal that landed on the background listener thread
    /// finally becomes a repaint: the listener only ever touches the
    /// (thread-safe) store and a dirty flag, because `Window` is not `Send`
    /// and cannot be handed to it.
    fn on_update(&mut self, _ctx: &AppContext) {
        if settings_client::take_dirty() {
            // Values, not chrome: only the pane has to be repainted.
            mark_pane_dirty(&self.pane_dirty);
        }

        // Scroll momentum, the overscroll bounce and the scrollbar's fade all
        // advance here rather than on input, since they keep running after
        // the gesture ends. `idle_timeout` keeps the loop turning while
        // there is something left to animate.
        let animating = {
            let mut scroll = self.scroll.lock().unwrap();
            let animating = scroll.is_animating();
            if animating {
                scroll.tick();
            }
            animating
        };

        // A flip that has landed is dropped rather than left at 1.0: the row
        // then draws from the value itself again, and nothing keeps asking
        // for frames. Dropping the last one still repaints once, so the
        // switch is never left a frame short of its end.
        let flipping = {
            let mut flips = self.toggle_flips.lock().unwrap();
            let before = flips.len();
            flips.retain(|_, flip| flip.is_running());
            !flips.is_empty() || flips.len() != before
        };

        let dirty = std::mem::replace(&mut *self.pane_dirty.lock().unwrap(), false);
        if animating || flipping || dirty {
            // A flip changes what a row looks like, not where the pane is
            // scrolled, so it has to repaint the band like any other value
            // change.
            self.sync_pane(dirty || flipping);
        }
    }

    /// The accent or the colour scheme changed. The pane repaints from the
    /// dirty flag, but the sidebar and titlebar live on the window's own
    /// surface, which only repaints when a frame is asked for — so ask.
    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        mark_pane_dirty(&self.pane_dirty);
        if let Some(window) = self.window.as_ref() {
            window.request_frame();
        }
    }

    /// A hand laid on the touchpad stops a gliding pane, the way a finger on a
    /// spinning wheel does. Nothing else in the pointer stream says so: a hold
    /// carries no motion and no button.
    fn on_pointer_hold_begin(&mut self, _ctx: &AppContext, _fingers: u32) {
        self.scroll.lock().unwrap().stop();
        mark_pane_dirty(&self.pane_dirty);
    }

    /// While the scroll view is animating the app needs a steady clock, not
    /// just the next input event.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        let animating = self.scroll.lock().unwrap().is_animating()
            || !self.toggle_flips.lock().unwrap().is_empty();
        animating.then(|| std::time::Duration::from_millis(8))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--png") {
        preview::render_to_png(args.get(1), args.get(2));
        return Ok(());
    }

    // Populate the settings store before the first frame, so panes draw live
    // values rather than placeholders and then flicker.
    settings_client::connect();
    if settings_client::is_online() {
        // Only worth watching once there is something to watch — an offline
        // store has no bus connection for a listener to subscribe on.
        settings_client::spawn_change_listener();
    } else {
        eprintln!("settings: showing placeholder values; changes will not be saved");
    }

    AppRunner::new(SettingsApp {
        window: None,
        selected: Arc::new(Mutex::new(0)),
        scroll: Arc::new(Mutex::new(ScrollView::new(view::pane_viewport_local(
            view::WINDOW_W,
            view::WINDOW_H,
        )))),
        surfaces: Rc::new(RefCell::new(None)),
        // The pane has never been painted, so the first update has to.
        pane_dirty: Arc::new(Mutex::new(true)),
        dragging: Arc::new(Mutex::new(None)),
        toggle_flips: Arc::new(Mutex::new(HashMap::new())),
        dropdowns: Rc::new(HashMap::new()),
        open_dropdown: Arc::new(Mutex::new(None)),
        pickers: Rc::new(HashMap::new()),
        open_picker: Arc::new(Mutex::new(None)),
        size: Arc::new(Mutex::new((view::WINDOW_W, view::WINDOW_H))),
        controls: Arc::new(Mutex::new(WindowControlsState::new())),
    })
    .run()?;
    Ok(())
}
