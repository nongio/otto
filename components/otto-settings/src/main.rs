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

use otto_kit::accessibility::{node_id as a11y_node, A11yTree, Action, ActionRequest, Role};
use otto_kit::components::color_picker::{ColorPickerPopup, Swatch};
use otto_kit::components::dropdown::DropdownMenu;
use otto_kit::components::scroll::ScrollSurfaces;
use otto_kit::components::text_input::{KeyMods, TextInput, TextInputKey, TextInputResponse};
use otto_kit::components::titlebar::{WindowControl, WindowControlsState};
use otto_kit::components::window::resize;
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1;
use otto_kit::CursorShape;
use panes::{displays, keyboard};
use smithay_client_toolkit::reexports::client::protocol::{wl_keyboard, wl_surface};
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::seat::pointer::{AxisScroll, PointerEventKind};
use smithay_client_toolkit::shell::xdg::XdgSurface;
use view::{Settings, ShortcutHit, WINDOW_H, WINDOW_W};

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
    /// What the keyboard was on last pass, so a focus that *moved* can be
    /// scrolled to without fighting the user's own scrolling afterwards.
    last_focus: Option<FocusId>,
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
    /// The text row that currently has the keyboard, if any. `None` means no
    /// field is being edited and key presses are nobody's.
    editing: Arc<Mutex<Option<Editing>>>,
    /// The button being held down, if any. A button acts on release, so the
    /// press has to be remembered in between — and drawn, which is the whole
    /// point of remembering it.
    pressed: Arc<Mutex<Option<view::Pressed>>>,
    /// Modifier state, kept from `on_modifiers` so a key press can be read
    /// with the modifiers that were down when it arrived.
    modifiers: Arc<Mutex<Mods>>,
    /// Whether the compositor is frosting the surface *right now*.
    ///
    /// Not the same as having asked for a frost: the window drops the blur
    /// while it is unfocused, and the chrome has to follow — the materials are
    /// translucent, and with nothing frosted behind them they would leave the
    /// desktop showing sharp through the sidebar. Shared with the draw
    /// closure, which is why it is an atomic rather than a plain flag.
    frosted: Arc<std::sync::atomic::AtomicBool>,
}

/// A text field with the keyboard: what it edits, and the live editor.
///
/// The value being typed lives here, not in the model — the model holds the
/// last *committed* value, which is what a cancelled edit falls back to.
struct Editing {
    target: EditTarget,
    input: TextInput,
}

/// What a field being typed into writes back to when it is committed.
///
/// One editor serves both because everything else about them is identical —
/// the caret, the blink, the key translation, the commit-on-click-elsewhere.
/// Only the destination differs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    /// A row bound to `org.otto.Settings`, written back through `apply`.
    Setting(&'static str),
    /// The key combination on one shortcut line, written back to the
    /// process-local store in `panes::keyboard` — shortcuts are not in the
    /// served schema, so there is nowhere else for them to go.
    ShortcutKeys(usize),
    /// A row the compositor does not serve, keyed by its label: the Displays
    /// pane's position fields, which write back to `panes::displays`'s own
    /// store. Labels are unique within a pane, which is as far as an editing
    /// session ever reaches.
    Unbound(&'static str),
}

impl EditTarget {
    /// Which target a settings row edits: its identifier where it has one, and
    /// its label where it does not.
    fn for_row(id: Option<&'static str>, label: &'static str) -> Self {
        match id {
            Some(id) => Self::Setting(id),
            None => Self::Unbound(label),
        }
    }
}

/// The modifiers a text field cares about.
#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    ctrl: bool,
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
    // The shortcut lines' action pop-ups. There is one slot per line the pane
    // can hold rather than one per line it holds *now*, because a menu cannot
    // be built later: the list is editable, and a line added at runtime has to
    // find its menu already made.
    ids.extend_from_slice(keyboard::slot_ids());
    // The Displays pane's resolution and refresh pop-ups. They already come
    // back from the walk above — both rows carry an `id` — but only while the
    // pane has a display to show, and a session that gains its first output
    // later would find no menu made. Adding them unconditionally costs two
    // entries; `HashMap` collapses the duplicates.
    ids.extend_from_slice(displays::slot_ids());
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
    editing: &Arc<Mutex<Option<Editing>>>,
    pressed: &Arc<Mutex<Option<view::Pressed>>>,
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
    .with_pressed(*pressed.lock().unwrap())
    .with_editing(
        editing
            .lock()
            .unwrap()
            .as_ref()
            .map(|edit| (edit.target, edit.input.clone())),
    )
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
    let Some(parent) = window
        .surface()
        .map(|s| s.xdg_window().xdg_surface().clone())
    else {
        return;
    };

    // A shortcut line's action is the one pop-up whose choices are not a
    // setting's: they are the builtin actions listed in `panes::keyboard`,
    // and what is picked goes back to that list rather than onto the bus.
    // A display's mode pop-ups are answered by the pane from the compositor's
    // own probe rather than from the settings schema, which serves no
    // per-output mode. Checked before the schema so a future setting of the
    // same name could not quietly take the row over.
    let display_slot = displays::menu_choices(select.id);
    let slot = keyboard::slot_index(select.id);
    let choices: Vec<discovery::Choice> = if let Some(values) = display_slot {
        values
            .into_iter()
            .map(|value| discovery::Choice {
                label: value.clone(),
                value,
            })
            .collect()
    } else if slot.is_some() {
        keyboard::actions()
            .iter()
            .map(|action| discovery::Choice {
                label: (*action).to_string(),
                value: (*action).to_string(),
            })
            .collect()
    } else {
        let Some(desc) = settings_client::describe(select.id) else {
            return;
        };
        if !desc.choices.is_empty() {
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
                if displays::menu_choices(id).is_some() {
                    displays::choose(id, value);
                } else {
                    match slot {
                        Some(line) => keyboard::set_action(line, value.clone()),
                        None => apply(id, settings_client::Value::Text(value.clone())),
                    }
                }
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

/// Whether the pointer is still on the button it went down on.
///
/// Re-run against the *current* pane rather than remembered as a rectangle:
/// the pane can have been rebuilt between the press and the release — a
/// shortcut line removed, a display selected — and a stale rectangle would
/// fire whatever moved into its place.
fn released_on(settings: &Settings, held: view::Pressed, x: f32, y: f32, offset: f32) -> bool {
    match held {
        view::Pressed::Button { row, button } => settings
            .button_hit(x, y, offset)
            .is_some_and(|hit| hit.row == row && hit.button == button),
        view::Pressed::Choose(id) => settings.file_hit(x, y, offset) == Some(id),
        view::Pressed::Remove(index) => matches!(
            settings.shortcut_hit(x, y, offset),
            Some(view::ShortcutHit::Remove(hit)) if hit == index
        ),
        view::Pressed::Add => matches!(
            settings.shortcut_hit(x, y, offset),
            Some(view::ShortcutHit::Add)
        ),
    }
}

/// Do what a button does, once it has been both pressed and released on.
fn activate(held: view::Pressed) {
    match held {
        // Push buttons belong to the pane that drew them, and a row label is
        // unique within one, so both are offered the press and only the owner
        // acts.
        view::Pressed::Button { row, button } => {
            panes::displays::press(row, button);
            panes::general::press(row, button);
        }
        view::Pressed::Choose(id) => open_file_picker(id),
        view::Pressed::Remove(index) => keyboard::remove(index),
        view::Pressed::Add => keyboard::add(),
    }
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

            match file_picker::open_file(otto_kit::t!("settings-choose-background-image"), filters)
            {
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

/// What the window is called: the app, then the pane you are in.
///
/// The dock, the app switcher and the window list all read the toplevel's
/// title, and "Settings" alone says nothing about where in the app you were.
/// Describes one pane row for an assistive technology, as the control it is.
///
/// A row is announced by the widget it holds, not as a generic item: a switch
/// has to say it is a switch and whether it is on, a slider has to carry the
/// range its value means anything against. The label is the row's own, with the
/// detail line as the description — which is where the schema's help text ends
/// up, so a screen reader reads what the pane shows in small print.
fn describe_row(tree: &mut A11yTree, id: &'static str, row: &model::Row, bounds: Rect) {
    let focus = view::pane_focus_id(id);
    let label = row.label;
    let detail = row.detail.clone();

    let describe = |node: &mut otto_kit::accessibility::Node| {
        if let Some(detail) = &detail {
            node.set_description(detail.to_string());
        }
    };

    match &row.control {
        model::Control::Toggle(on) => {
            tree.toggle(focus, bounds, label, *on, true);
        }
        model::Control::Slider {
            value, min, max, ..
        } => {
            tree.slider(
                focus,
                bounds,
                label,
                f64::from(*value),
                f64::from(*min)..=f64::from(*max),
                f64::from((max - min) / 20.0),
                true,
            );
        }
        model::Control::Select(current) => {
            tree.combo_box(focus, bounds, label, current.clone(), false, true);
        }
        model::Control::Text(text) => {
            tree.control(focus, bounds, Role::TextInput, true, |node| {
                node.set_label(label);
                node.set_value(text.clone());
                describe(node);
            });
        }
        model::Control::File(path) => {
            tree.control(focus, bounds, Role::Button, true, |node| {
                node.set_label(label);
                // The path is the row's value; an empty one is not "unnamed",
                // it is a file nobody has chosen yet.
                node.set_value(if path.is_empty() {
                    otto_kit::t_owned!("settings-no-file-chosen")
                } else {
                    path.clone()
                });
                node.add_action(Action::Click);
                describe(node);
            });
        }
        model::Control::Color(rgb) => {
            tree.control(focus, bounds, Role::ColorWell, true, |node| {
                node.set_label(label);
                node.set_value(format!("#{:06X}", rgb & 0x00FF_FFFF));
                node.add_action(Action::Click);
                describe(node);
            });
        }
        model::Control::Button(labels) => {
            tree.control(focus, bounds, Role::Button, true, |node| {
                node.set_label(match labels.first() {
                    Some(button) => format!("{label}: {button}"),
                    None => label.to_owned(),
                });
                node.add_action(Action::Click);
                describe(node);
            });
        }
        model::Control::Value(value) => {
            tree.control(focus, bounds, Role::Label, false, |node| {
                node.set_label(label);
                node.set_value(value.clone());
                describe(node);
            });
        }
        // A shortcut line is three controls in a row and the add line is a
        // button for a list this does not describe yet; announcing either as
        // one thing would be a lie about what it is.
        model::Control::Shortcut { .. } | model::Control::AddShortcut => {}
    }
}

/// Selects a sidebar pane from something other than a pointer click — the
/// keyboard, or an assistive technology. Returns whether anything changed.
///
/// The pointer path does the same thing inline, against the same state: a
/// different pane means a new title and a scroll position that no longer means
/// anything.
fn select_pane(app: &SettingsApp, index: usize) -> bool {
    let mut current = app.selected.lock().unwrap();
    if *current == index {
        return false;
    }
    *current = index;
    drop(current);

    if let Some(window) = app.window.as_ref() {
        window.set_title(&window_title(index));
    }
    app.scroll.lock().unwrap().state.set_offset(0.0);
    mark_pane_dirty(&app.pane_dirty);
    if let Some(window) = app.window.as_ref() {
        window.request_frame();
    }
    true
}

fn window_title(selected: usize) -> String {
    match model::panes().get(selected) {
        Some(pane) => otto_kit::t_owned!("settings-window-title", pane = pane.name),
        None => "Otto Settings".to_string(),
    }
}

/// The looks of a settings row's text field while it is being edited, matched
/// to what [`crate::widgets::text_field`] draws at rest so focusing a field
/// does not make it jump.
fn text_input_style(dark: bool) -> TextInputStyle {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut style = TextInputStyle::with_theme(theme.clone());
    // The same size the row draws at rest: focusing a field must not resize
    // the text you were about to edit.
    style.text_style = widgets::CONTROL_TEXT;
    style.horizontal_padding = 9.0;
    style.corner_radius = 6.0;
    // A focused field goes to paper — the page's own ground rather than the
    // faint fill it sits in at rest — so the one field taking the keyboard is
    // obvious among a column of identical-looking ones.
    style.background = if dark {
        Color::from_rgb(0x1A, 0x1C, 0x20)
    } else {
        Color::WHITE
    };
    style
}

/// Send what a field currently holds and stop editing.
///
/// Returns whether there was anything to commit, so a caller can skip a
/// repaint it does not need.
fn commit_edit(editing: &Arc<Mutex<Option<Editing>>>) -> bool {
    let Some(edit) = editing.lock().unwrap().take() else {
        return false;
    };
    match edit.target {
        EditTarget::Setting(id) => apply(id, settings_client::text_for(id, edit.input.value())),
        // Trimmed because the compositor's trigger parser splits on `+` and
        // trims each part, so surrounding space is noise either way — better
        // not to store it than to store something that only parses by luck.
        EditTarget::ShortcutKeys(index) => {
            keyboard::set_keys(index, edit.input.value().trim().to_string())
        }
        EditTarget::Unbound(label) => panes::displays::commit_text(label, edit.input.value()),
    }
    true
}

/// Start typing into a field, with the caret where the press landed.
///
/// `width` is the field's own, which differs between a settings row and a
/// shortcut line — the editor needs it to scroll the caret into view.
fn start_edit(
    editing: &Arc<Mutex<Option<Editing>>>,
    target: EditTarget,
    value: String,
    offset_x: f32,
    width: f32,
    dark: bool,
) {
    let mut input =
        TextInput::new(value, text_input_style(dark)).with_size(width, widgets::CONTROL_H);
    input.state.set_focused(true);
    input.on_pointer_down(offset_x, 1, false);
    *editing.lock().unwrap() = Some(Editing { target, input });
}

/// Stop editing without sending anything — Escape, and a keyboard focus lost
/// to another window.
fn cancel_edit(editing: &Arc<Mutex<Option<Editing>>>) -> bool {
    editing.lock().unwrap().take().is_some()
}

/// How often the app wakes itself while something is moving: a scroll fling,
/// a switch mid-flip, or a caret blinking.
const IDLE_TICK: std::time::Duration = std::time::Duration::from_millis(8);

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
        let settings = current_settings(
            &self.selected,
            &self.size,
            &self.toggle_flips,
            &self.editing,
            &self.pressed,
        )
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

/// Put the sidebar's material colour on the compositor's layer for the
/// window's surface.
///
/// The frost is the compositor's, and so is its tint: the blur samples what is
/// behind the surface and this colour is what tints the result. It has to be
/// re-applied whenever the colour scheme changes, since the layer keeps the
/// colour it was last given.
fn apply_material(window: &Window) {
    window.set_material(view::sidebar_material(
        current_color_scheme() == ColorScheme::Dark,
    ));
}

impl SettingsApp {
    /// Declares what Tab moves between, in the order it moves.
    ///
    /// Rebuilt every pass, from the same geometry the sidebar is drawn with, so
    /// a row can never be reachable somewhere it is not painted.
    fn declare_focusables(&self) {
        let Some(surface) = self.window.as_ref().and_then(Window::surface_id) else {
            return;
        };
        let panes = model::panes().len();
        let settings = current_settings(
            &self.selected,
            &self.size,
            &self.toggle_flips,
            &self.editing,
            &self.pressed,
        );
        let offset = self.scroll.lock().unwrap().state.offset();
        let rows: Vec<(FocusId, Rect)> = settings
            .pane_rows(offset)
            .into_iter()
            .filter_map(|(row, rect)| Some((view::pane_focus_id(row.id?), rect)))
            .collect();

        // The sidebar is one stop, not eight: a list is entered with Tab and
        // walked with the arrows, which is what every toolkit does and what the
        // list role tells a screen reader to expect. The pane's controls are
        // separate controls, so each is its own stop.
        let selected = *self.selected.lock().unwrap();
        let sidebar = view::sidebar_item_rect(selected.min(panes.saturating_sub(1)));

        AppContext::with_focus_ring(&surface, |ring| {
            ring.begin();
            ring.add(view::SIDEBAR_FOCUS, sidebar, true);
            for (id, rect) in &rows {
                ring.add(*id, *rect, true);
            }
            ring.end();
        });
    }

    /// Moves the sidebar's selection, when the sidebar is what the keyboard is
    /// on. Returns whether it took the key.
    fn move_sidebar(&self, delta: isize) -> bool {
        let surface = self.window.as_ref().and_then(Window::surface_id);
        let focused = surface.and_then(|s| AppContext::focused_control(&s));
        if focused != Some(view::SIDEBAR_FOCUS) {
            return false;
        }

        let panes = model::panes().len() as isize;
        let current = *self.selected.lock().unwrap() as isize;
        // Stops at the ends rather than wrapping: a sidebar is a place in a
        // list, and arrowing off the bottom onto the top loses that place.
        let moved = (current + delta).clamp(0, panes - 1) as usize;
        select_pane(self, moved);
        true
    }

    /// Brings a control the keyboard has just moved to into view.
    ///
    /// Only on the pass the focus actually moves: scrolling to it on every pass
    /// would drag the pane back the moment the user scrolled away from a
    /// focused control.
    fn scroll_focus_into_view(&mut self) {
        let Some(surface) = self.window.as_ref().and_then(Window::surface_id) else {
            return;
        };
        let focused = AppContext::focused_control(&surface);
        if focused == self.last_focus {
            return;
        }
        self.last_focus = focused;

        // The ring may have moved onto — or off — a pane row, which is drawn
        // into the scroll subsurfaces rather than the window's own buffer. The
        // window repaints itself on the frame the run loop asked for; the band
        // has to be invalidated by hand.
        mark_pane_dirty(&self.pane_dirty);

        let Some(focused) = focused else { return };

        let settings = current_settings(
            &self.selected,
            &self.size,
            &self.toggle_flips,
            &self.editing,
            &self.pressed,
        );
        let (width, height) = *self.size.lock().unwrap();
        let viewport = view::pane_viewport(width, height);

        let mut scroll = self.scroll.lock().unwrap();
        let offset = scroll.state.offset();
        let Some((_, bounds)) = settings
            .pane_rows(offset)
            .into_iter()
            .find(|(row, _)| row.id.is_some_and(|id| view::pane_focus_id(id) == focused))
        else {
            return;
        };

        // A margin, so a row does not sit flush against the edge it came in
        // from and read as though it were cut off.
        const MARGIN: f32 = 12.0;
        let moved = if bounds.top - MARGIN < viewport.top {
            offset - (viewport.top - bounds.top + MARGIN)
        } else if bounds.bottom + MARGIN > viewport.bottom {
            offset + (bounds.bottom - viewport.bottom + MARGIN)
        } else {
            return;
        };

        scroll.state.set_offset(moved.max(0.0));
        drop(scroll);
        mark_pane_dirty(&self.pane_dirty);
    }

    /// The pane control the keyboard is on, if it is on one.
    fn focused_row(&self) -> Option<(&'static str, model::Control)> {
        let surface = self.window.as_ref().and_then(Window::surface_id)?;
        let focused = AppContext::focused_control(&surface)?;

        let settings = current_settings(
            &self.selected,
            &self.size,
            &self.toggle_flips,
            &self.editing,
            &self.pressed,
        );
        let offset = self.scroll.lock().unwrap().state.offset();
        settings.pane_rows(offset).into_iter().find_map(|(row, _)| {
            let id = row.id?;
            (view::pane_focus_id(id) == focused).then(|| (id, row.control.clone()))
        })
    }

    /// Acts on the focused control the way a click on it would.
    ///
    /// `step` is how far a value control should move: zero for "activate it",
    /// which is all a switch or a button understands.
    fn activate_focused(&self, step: f32) -> bool {
        let Some((id, control)) = self.focused_row() else {
            return false;
        };

        match control {
            model::Control::Toggle(on) if step == 0.0 => {
                // The knob slides rather than jumping, exactly as it does for a
                // click — the keyboard is not a second way for a switch to
                // behave.
                let mut flips = self.toggle_flips.lock().unwrap();
                let current = flips
                    .get(id)
                    .map(|flip| flip.fraction())
                    .unwrap_or_else(|| toggle::knob_fraction_for(on));
                flips.insert(id, toggle::Flip::start(current, !on));
                drop(flips);
                apply(id, settings_client::Value::Bool(!on));
            }
            model::Control::Slider {
                value, min, max, ..
            } if step != 0.0 => {
                // A twentieth of the range per press: fine enough to land on a
                // value, coarse enough to cross the track without holding the
                // key down.
                let moved = (value + step * (max - min) / 20.0).clamp(min, max);
                if moved == value {
                    return false;
                }
                apply(id, settings_client::number_for(id, moved));
            }
            model::Control::Button(labels) if step == 0.0 => {
                // The first button is the row's own action; a row with several
                // needs the pointer until each one is reachable in its turn.
                let Some(label) = labels.first() else {
                    return false;
                };
                activate(view::Pressed::Button {
                    row: id,
                    button: label,
                });
            }
            model::Control::File(_) if step == 0.0 => open_file_picker(id),
            _ => return false,
        }

        mark_pane_dirty(&self.pane_dirty);
        if let Some(window) = self.window.as_ref() {
            window.request_frame();
        }
        true
    }

    /// Whether the keyboard is on the sidebar.
    fn sidebar_focused(&self) -> bool {
        let surface = self.window.as_ref().and_then(Window::surface_id);
        surface.and_then(|s| AppContext::focused_control(&s)) == Some(view::SIDEBAR_FOCUS)
    }
}

impl App for SettingsApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new(
            &window_title(*self.selected.lock().unwrap()),
            WINDOW_W as i32,
            WINDOW_H as i32,
        )?;
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
        if let Some(style) = window.surface_style() {
            style.set_corner_radius(view::CORNER as f64);
            style.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
            // The pane scrolls in subsurfaces that sit over the window's own
            // buffer, so the window's rounded outline does not contain them:
            // without this the content runs square into the bottom-right
            // corner. Clipping the descendants to the window's style bounds
            // rounds them with it.
            style.set_clip_children(otto_surface_style_v1::ClipMode::Enabled);
            eprintln!("settings: surface style present, blur requested = {want_blur}");
        }
        // The material's colour goes on the compositor's layer, not into the
        // buffer: `BackgroundBlur` blurs what is behind the layer and tints
        // the result with this colour, and the window fades that tint between
        // its translucent and opaque forms as focus comes and goes. A ground
        // painted into the buffer — even a translucent one — would sit on top
        // of all of it, which is why `render_ground` leaves the sidebar to
        // the compositor whenever there is a style to carry it.
        // Asked of the *window* rather than of the style directly: the window
        // re-applies the blend mode on every configure, and the first style
        // request goes out before the surface is mapped — set once on the
        // style alone it reaches a surface the compositor has no window for
        // yet, and the frost never arrives. The window also drops the blur
        // while it is unfocused, so no full-window gaussian runs for a window
        // nobody is looking at. Same path `otto-files` takes.
        apply_material(&window);
        window.set_background_blur(want_blur);

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
        let frosted = self.frosted.clone();
        window.on_draw(move |canvas| {
            let index = *selected.lock().unwrap();
            let (w, h) = *size.lock().unwrap();
            Settings::new(index, current_color_scheme() == ColorScheme::Dark)
                .with_size(w, h)
                .with_blur(frosted.load(std::sync::atomic::Ordering::Relaxed))
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
                                    // Dragging moves the window; a double
                                    // click zooms it.
                                    if let Some(seat) = AppContext::seat_state().seats().next() {
                                        redraw.titlebar_press(&seat, *serial, x, y);
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
                                redraw.set_title(&window_title(index));
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
        let editing_hit = self.editing.clone();
        let pressed_hit = self.pressed.clone();
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
                        let settings = current_settings(
                            &selected,
                            &size_hit,
                            &toggle_flips,
                            &editing_hit,
                            &pressed_hit,
                        );

                        // A press anywhere but on the field being edited is
                        // an answer to it, not an abandonment: commit before
                        // the press does whatever else it does.
                        let same_field = settings
                            .text_hit(x, y, offset)
                            .map(|hit| EditTarget::for_row(hit.id, hit.label))
                            == editing_hit.lock().unwrap().as_ref().map(|edit| edit.target);
                        if !same_field && commit_edit(&editing_hit) {
                            mark_pane_dirty(&pane_dirty);
                        }

                        // A shortcut line is three controls in one row, none of
                        // them a setting, so it is asked first — its action
                        // pop-up would otherwise fall through to `select_hit`,
                        // which looks the identifier up in the schema and finds
                        // nothing.
                        let shortcut = settings.shortcut_hit(x, y, offset);
                        // Pressing anywhere but inside the field being typed
                        // into accepts what is in it, the way clicking away
                        // from a rename does.
                        let stays_open = match (&shortcut, editing_hit.lock().unwrap().as_ref()) {
                            (Some(ShortcutHit::Keys { index, .. }), Some(edit)) => {
                                edit.target == EditTarget::ShortcutKeys(*index)
                            }
                            _ => false,
                        };
                        if !stays_open {
                            commit_edit(&editing_hit);
                        }

                        if let Some(hit) = shortcut {
                            match hit {
                                ShortcutHit::Action(select) => open_menu(
                                    &dropdowns,
                                    &open_dropdown,
                                    &pane_dirty,
                                    &redraw,
                                    select,
                                    event_serial(&event.kind),
                                ),
                                ShortcutHit::Keys { index, offset_x } => {
                                    // A second press in the field already open
                                    // moves the caret. Starting over would
                                    // reload the line and throw away whatever
                                    // has been typed but not committed.
                                    let moved = {
                                        let mut editing = editing_hit.lock().unwrap();
                                        match editing.as_mut().filter(|edit| {
                                            edit.target == EditTarget::ShortcutKeys(index)
                                        }) {
                                            Some(edit) => {
                                                edit.input.on_pointer_down(offset_x, 1, false);
                                                true
                                            }
                                            None => false,
                                        }
                                    };
                                    if !moved {
                                        if let Some(keys) = keyboard::keys(index) {
                                            start_edit(
                                                &editing_hit,
                                                EditTarget::ShortcutKeys(index),
                                                keys,
                                                offset_x,
                                                view::SHORTCUT_KEYS_W,
                                                settings.dark,
                                            );
                                        }
                                    }
                                }
                                // Both act on release; the press only lights
                                // the button up. See `view::Pressed`.
                                ShortcutHit::Remove(index) => {
                                    *pressed_hit.lock().unwrap() =
                                        Some(view::Pressed::Remove(index));
                                }
                                ShortcutHit::Add => {
                                    *pressed_hit.lock().unwrap() = Some(view::Pressed::Add);
                                }
                            }
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(color) = settings.color_hit(x, y, offset) {
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
                            *pressed_hit.lock().unwrap() = Some(view::Pressed::Choose(id));
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(index) = settings.screen_hit(x, y, offset) {
                            // The arrangement is a picker: the rows under it
                            // are the settings of whichever screen is chosen
                            // there, so selecting one rebuilds the pane.
                            model::select_output(index);
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(button) = settings.button_hit(x, y, offset) {
                            *pressed_hit.lock().unwrap() = Some(view::Pressed::Button {
                                row: button.row,
                                button: button.button,
                            });
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(label) = settings.unbound_toggle_hit(x, y, offset) {
                            // A switch the compositor does not serve. It has no
                            // identifier to `apply`, and no flip animation
                            // either — the pane it belongs to owns the value.
                            panes::displays::toggle(label);
                            mark_pane_dirty(&pane_dirty);
                        } else if let Some(text) = settings.text_hit(x, y, offset) {
                            // Moving between fields commits the one being
                            // left: a click elsewhere is an answer, not an
                            // abandonment.
                            let target = EditTarget::for_row(text.id, text.label);
                            let already = {
                                let current = editing_hit.lock().unwrap();
                                current.as_ref().map(|edit| edit.target) == Some(target)
                            };
                            if already {
                                // A second press in the field already open
                                // moves the caret. Starting over would reload
                                // the row and throw away what has been typed
                                // but not committed.
                                if let Some(edit) = editing_hit.lock().unwrap().as_mut() {
                                    edit.input.on_pointer_down(text.local_x, 1, false);
                                }
                            } else {
                                start_edit(
                                    &editing_hit,
                                    target,
                                    text.current.clone(),
                                    text.local_x,
                                    widgets::TEXT_W,
                                    settings.dark,
                                );
                            }
                            mark_pane_dirty(&pane_dirty);
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

                        // A button acts here, not on the press — and only if
                        // the pointer is still on the one it went down on. A
                        // press slid off before letting go is taken back,
                        // which is what every other button on the desktop
                        // does and the only reason a pressed state is worth
                        // drawing.
                        // Bound before the body: a lock guard in an `if let`
                        // scrutinee lives as long as the body does, and
                        // `current_settings` takes the same lock.
                        let held = pressed_hit.lock().unwrap().take();
                        if let Some(held) = held {
                            mark_pane_dirty(&pane_dirty);
                            let settings = current_settings(
                                &selected,
                                &size_hit,
                                &toggle_flips,
                                &editing_hit,
                                &pressed_hit,
                            );
                            let offset = scroll.lock().unwrap().offset();
                            if released_on(&settings, held, x, y, offset) {
                                activate(held);
                            }
                            redraw.request_frame();
                        }
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
                        // Sliding off a held button un-presses it, and back
                        // on presses it again, so the highlight always says
                        // what letting go now would do.
                        let held_button = *pressed_hit.lock().unwrap();
                        if let Some(held) = held_button {
                            let settings = current_settings(
                                &selected,
                                &size_hit,
                                &toggle_flips,
                                &editing_hit,
                                &pressed_hit,
                            );
                            let offset = scroll.lock().unwrap().offset();
                            if !released_on(&settings, held, x, y, offset) {
                                *pressed_hit.lock().unwrap() = None;
                                mark_pane_dirty(&pane_dirty);
                                redraw.request_frame();
                            }
                        }

                        let held = dragging.lock().unwrap().clone();
                        if let Some(id) = held {
                            let settings = current_settings(
                                &selected,
                                &size_hit,
                                &toggle_flips,
                                &editing_hit,
                                &pressed_hit,
                            );
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

        // Publish the window to assistive technologies. Nothing is built until
        // one attaches — see `App::accessibility`.
        if let Some(surface) = self.window.as_ref().and_then(Window::surface_id) {
            AppContext::enable_accessibility(&surface);
        }
        self.declare_focusables();

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

        // Activation arrives on a configure, and the window turns its blur on
        // and off with it — so this is where the chrome learns whether there
        // is a frost behind its materials. A configure that changes nothing
        // else still has to repaint the sidebar, which is why this runs before
        // the size early-out below.
        let frosted = window.background_blur() && window.is_activated();
        if self
            .frosted
            .swap(frosted, std::sync::atomic::Ordering::Relaxed)
            != frosted
        {
            window.request_frame();
        }
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
        self.declare_focusables();
        self.scroll_focus_into_view();

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

        // The caret blinks on the app's clock, so the field being edited gets
        // its phase advanced here and its band repainted with it.
        let blinking = {
            let mut editing = self.editing.lock().unwrap();
            match editing.as_mut() {
                Some(edit) => {
                    let was = edit.input.caret_visible();
                    edit.input.tick(IDLE_TICK.as_secs_f32());
                    was != edit.input.caret_visible()
                }
                None => false,
            }
        };

        let dirty = std::mem::replace(&mut *self.pane_dirty.lock().unwrap(), false);
        if animating || flipping || dirty || blinking {
            // A flip changes what a row looks like, not where the pane is
            // scrolled, so it has to repaint the band like any other value
            // change.
            self.sync_pane(dirty || flipping || blinking);
        }
    }

    /// Modifier state, saved for the key press it belongs to.
    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        *self.modifiers.lock().unwrap() = Mods {
            shift: modifiers.shift,
            ctrl: modifiers.ctrl,
        };
    }

    /// Keys go to the text field that has the keyboard, if there is one.
    /// Nothing else in the app takes typed input yet.
    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        use smithay_client_toolkit::seat::keyboard::Keysym;

        if state != wl_keyboard::KeyState::Pressed {
            return;
        }

        // Nothing is being typed into: the key belongs to whatever the keyboard
        // focus is on — a sidebar row, or a control in the pane it selected.
        if self.editing.lock().unwrap().is_none() {
            match event.keysym {
                // The sidebar's selection follows the arrows straight away,
                // so moving through it shows each pane rather than needing a
                // press to confirm. Enter has nothing left to do there.
                Keysym::Return | Keysym::KP_Enter | Keysym::space => {
                    if !self.sidebar_focused() {
                        self.activate_focused(0.0);
                    }
                }
                Keysym::Down => {
                    if !self.move_sidebar(1) {
                        self.activate_focused(-1.0);
                    }
                }
                Keysym::Up => {
                    if !self.move_sidebar(-1) {
                        self.activate_focused(1.0);
                    }
                }
                // A slider holds a range, so the arrows walk it.
                Keysym::Left => {
                    self.activate_focused(-1.0);
                }
                Keysym::Right => {
                    self.activate_focused(1.0);
                }
                Keysym::Home => {
                    self.move_sidebar(-(model::panes().len() as isize));
                }
                Keysym::End => {
                    self.move_sidebar(model::panes().len() as isize);
                }
                _ => {}
            }
            return;
        }
        let Mods { shift, ctrl } = *self.modifiers.lock().unwrap();

        let key = match event.keysym {
            Keysym::Return | Keysym::KP_Enter => Some(TextInputKey::Enter),
            Keysym::Escape => Some(TextInputKey::Escape),
            Keysym::Left => Some(TextInputKey::Left),
            Keysym::Right => Some(TextInputKey::Right),
            Keysym::Home => Some(TextInputKey::Home),
            Keysym::End => Some(TextInputKey::End),
            Keysym::BackSpace => Some(TextInputKey::Backspace),
            Keysym::Delete => Some(TextInputKey::Delete),
            Keysym::a if ctrl => Some(TextInputKey::SelectAll),
            // Whatever the keymap produced, as a whole: an input method can
            // commit more than one character at a time, and taking only the
            // first would silently drop the rest. A modifier pressed on its
            // own produces nothing here — `on_modifiers` already recorded
            // what it changed.
            _ => {
                let text: String = event
                    .utf8
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                (!text.is_empty()).then_some(TextInputKey::Text(text))
            }
        };
        let Some(key) = key else { return };

        let response = self
            .editing
            .lock()
            .unwrap()
            .as_mut()
            .map(|edit| edit.input.on_key(key, KeyMods { shift, ctrl }));
        match response {
            Some(TextInputResponse::Commit) => {
                commit_edit(&self.editing);
            }
            Some(TextInputResponse::Cancel) => {
                cancel_edit(&self.editing);
            }
            Some(TextInputResponse::Ignored) | None => return,
            Some(_) => {}
        }
        mark_pane_dirty(&self.pane_dirty);
        if let Some(window) = self.window.as_ref() {
            window.request_frame();
        }
    }

    /// The keyboard went elsewhere. An edit in flight has no way left to be
    /// answered, so it is dropped rather than left blinking on a window that
    /// no longer has focus.
    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {
        if cancel_edit(&self.editing) {
            mark_pane_dirty(&self.pane_dirty);
            if let Some(window) = self.window.as_ref() {
                window.request_frame();
            }
        }
    }

    /// What a screen reader reads: the window, its sidebar, and which pane is
    /// showing.
    ///
    /// The pane's own controls are not described yet — the sidebar is what can
    /// be reached from the keyboard, and describing a control that cannot be
    /// operated would be worse than saying nothing about it.
    fn accessibility(&mut self, _ctx: &AppContext, _surface: &ObjectId) -> Option<A11yTree> {
        let selected = *self.selected.lock().unwrap();
        let panes = model::panes();

        let mut tree = A11yTree::new(window_title(selected));
        tree.region(
            FocusId::new("sidebar"),
            Rect::from_xywh(0.0, 0.0, view::SIDEBAR_W, WINDOW_H),
            Role::List,
            otto_kit::t!("a11y-categories"),
            |tree| {
                for (index, pane) in panes.iter().enumerate() {
                    tree.list_row(
                        view::sidebar_focus_id(index),
                        view::sidebar_item_rect(index),
                        pane.name,
                        index == selected,
                    );
                }
            },
        );

        // The pane the sidebar selected, with everything in it that can be
        // reached. Built from the same rows the pane is drawn from, at the
        // scroll position it is drawn at.
        let pane_settings = current_settings(
            &self.selected,
            &self.size,
            &self.toggle_flips,
            &self.editing,
            &self.pressed,
        );
        let offset = self.scroll.lock().unwrap().state.offset();
        let (width, height) = *self.size.lock().unwrap();
        let rows = pane_settings.pane_rows(offset);

        tree.region(
            FocusId::new("pane"),
            Rect::from_ltrb(view::SIDEBAR_W, view::TITLEBAR_H, width, height),
            Role::Group,
            panes
                .get(selected)
                .map(|pane| pane.name)
                .unwrap_or(otto_kit::t!("a11y-settings")),
            |tree| {
                for (row, bounds) in rows {
                    let Some(id) = row.id else { continue };
                    describe_row(tree, id, row, bounds);
                }
            },
        );

        // The sidebar is one keyboard stop but eight nodes: say which row is
        // current, or a screen reader is told only that the window has focus.
        if self.sidebar_focused() {
            tree.set_focus(view::sidebar_focus_id(selected));
        }

        Some(tree)
    }

    /// A screen reader asked for a sidebar row: the same thing a click on it
    /// does.
    fn on_accessibility_action(
        &mut self,
        _ctx: &AppContext,
        _surface: &ObjectId,
        request: &ActionRequest,
    ) {
        let index = (0..model::panes().len())
            .find(|index| a11y_node(view::sidebar_focus_id(*index)) == request.target_node);
        if let Some(index) = index {
            if matches!(request.action, Action::Click) {
                select_pane(self, index);
            }
            return;
        }

        // A control in the pane. The focus moves to it first, so acting on it
        // goes through exactly the path a keyboard press would — there is one
        // way for a control to be operated, not two.
        let Some(surface) = self.window.as_ref().and_then(Window::surface_id) else {
            return;
        };
        let settings = current_settings(
            &self.selected,
            &self.size,
            &self.toggle_flips,
            &self.editing,
            &self.pressed,
        );
        let offset = self.scroll.lock().unwrap().state.offset();
        let target = settings.pane_rows(offset).into_iter().find_map(|(row, _)| {
            let id = row.id?;
            let focus = view::pane_focus_id(id);
            (a11y_node(focus) == request.target_node).then_some(focus)
        });
        let Some(focus) = target else { return };

        AppContext::focus_control(&surface, Some(focus));

        match request.action {
            Action::Click | Action::Focus => {
                if matches!(request.action, Action::Click) {
                    self.activate_focused(0.0);
                }
            }
            Action::Increment => {
                self.activate_focused(1.0);
            }
            Action::Decrement => {
                self.activate_focused(-1.0);
            }
            _ => {}
        }
    }

    /// The accent or the colour scheme changed. The pane repaints from the
    /// dirty flag, but the sidebar and titlebar live on the window's own
    /// surface, which only repaints when a frame is asked for — so ask.
    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        mark_pane_dirty(&self.pane_dirty);
        if let Some(window) = self.window.as_ref() {
            // The frost's tint is on the compositor's layer, which keeps the
            // colour it was last given — hand it the new scheme's material.
            apply_material(window);
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
        // A blinking caret needs one too: nothing else wakes the loop while
        // the user is looking at a field they have stopped typing into.
        let animating = self.scroll.lock().unwrap().is_animating()
            || !self.toggle_flips.lock().unwrap().is_empty()
            // A blinking caret needs the same steady clock, and for the same
            // reason: nothing else is going to ask for the next frame.
            || self.editing.lock().unwrap().is_some();
        animating.then_some(IDLE_TICK)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Before anything reads a string, the preview path included: every row
    // label and group heading is looked up as its pane is constructed, and a
    // preview rendered in English would not be a preview of this desktop.
    // Asks the compositor rather than reading LANG, so this app agrees with
    // the setting it is itself showing.
    otto_kit::i18n::init_from_desktop();

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
        last_focus: None,
        toggle_flips: Arc::new(Mutex::new(HashMap::new())),
        dropdowns: Rc::new(HashMap::new()),
        open_dropdown: Arc::new(Mutex::new(None)),
        pickers: Rc::new(HashMap::new()),
        open_picker: Arc::new(Mutex::new(None)),
        size: Arc::new(Mutex::new((view::WINDOW_W, view::WINDOW_H))),
        controls: Arc::new(Mutex::new(WindowControlsState::new())),
        editing: Arc::new(Mutex::new(None)),
        pressed: Arc::new(Mutex::new(None)),
        modifiers: Arc::new(Mutex::new(Mods::default())),
        frosted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    })
    .run()?;
    Ok(())
}
