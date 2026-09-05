//! Window layout: titlebar, sidebar, and the selected pane.

use otto_kit::components::color_picker::{self, WellInteraction};
use otto_kit::components::dropdown::{self, DropdownInteraction};
use otto_kit::components::text_input::TextInput;
use otto_kit::components::titlebar::{
    Titlebar, TitlebarGroup, WindowControls, WindowControlsState,
};
use otto_kit::controls_side::ControlsSide;
use otto_kit::prelude::*;
use skia_safe::{ClipOp, Contains, PathEffect, Point, RRect};

use crate::glyphs;
use crate::model::{self, Control, Pane, Row};
use crate::panes::keyboard;
use crate::settings_client::{self, Value};
use crate::widgets;
use std::collections::HashMap;

/// The size the window asks for on first map. After that the compositor is in
/// charge, and everything draws against [`Settings::width`]/`height` instead —
/// these two are a starting point, not a layout constant.
pub const WINDOW_W: f32 = 900.0;
pub const WINDOW_H: f32 = 640.0;
/// Below this the sidebar and a content column stop both fitting.
pub const MIN_W: f32 = 560.0;
pub const MIN_H: f32 = 360.0;
pub const CORNER: f32 = 12.0;

/// [`CORNER`], or square on a desktop configured without rounded corners.
pub fn corner() -> f32 {
    otto_kit::corners::radius(CORNER)
}
pub const TITLEBAR_H: f32 = 38.0;
pub const SIDEBAR_W: f32 = 214.0;
const CONTENT_PAD: f32 = 26.0;
const ROW_H: f32 = 42.0;
const ROW_H_DETAIL: f32 = 56.0;
const GROUP_GAP: f32 = 22.0;
/// Top padding of the pane content, in content-local points (origin at the
/// pane viewport's top-left, i.e. `(SIDEBAR_W, TITLEBAR_H)`).
const CONTENT_TOP_PAD: f32 = 16.0;
/// The displays arrangement canvas itself: the framed area the screens are
/// laid out in.
const ARRANGEMENT_CANVAS_H: f32 = 168.0;
/// Height the canvas adds ahead of the groups: the area plus the gap
/// `render_arrangement` leaves below it for its caption. Kept as one
/// constant, alongside that function, so [`Settings::pane_content_height`]
/// cannot drift from what it actually draws.
const ARRANGEMENT_HEIGHT: f32 = ARRANGEMENT_CANVAS_H + 30.0;
/// A file row's preview: how tall the thumbnail box is, and the space above
/// and below it. The width follows the image's own aspect, capped at
/// [`PREVIEW_W`] — a wallpaper is worth seeing in its own shape.
const PREVIEW_H: f32 = 108.0;
const PREVIEW_W: f32 = 192.0;
const PREVIEW_GAP: f32 = 10.0;
/// The button that clears a file setting, revealed on the preview's top-right
/// corner while the pointer is over the picture: its diameter, and how far it
/// is inset from that corner.
const PREVIEW_REMOVE_D: f32 = 22.0;
const PREVIEW_REMOVE_INSET: f32 = 6.0;

/// A theme row's preview: a light card with one slot per sample image. The
/// card is a fixed size whatever the theme carries, so choosing a different
/// theme never moves the rows under the pointer.
const SWATCH_ICON: f32 = 36.0;
const SWATCH_GAP: f32 = 16.0;
const SWATCH_PAD: f32 = 16.0;
const SWATCH_H: f32 = SWATCH_ICON + SWATCH_PAD * 2.0;
const SWATCH_W: f32 = SWATCH_PAD * 2.0
    + SWATCH_ICON * crate::theme_preview::SLOTS as f32
    + SWATCH_GAP * (crate::theme_preview::SLOTS as f32 - 1.0);

/// The scrollable pane viewport: everything right of the sidebar, below the
/// titlebar, in window-local coordinates. Where the pane's subsurfaces are
/// placed, and what the popup anchors are measured against.
pub fn pane_viewport(width: f32, height: f32) -> Rect {
    Rect::from_ltrb(SIDEBAR_W, TITLEBAR_H, width, height)
}

/// The same viewport in the pane's *own* coordinates, origin at its top-left.
///
/// This is the space the pane's subsurfaces live in, so it is also the space
/// the [`ScrollView`](otto_kit::components::scroll::ScrollView) driving them
/// has to be told about: `ScrollSurfaces` positions the scrollbar from the
/// thumb rect the view computes, relative to the pane, not to the window.
pub fn pane_viewport_local(width: f32, height: f32) -> Rect {
    let viewport = pane_viewport(width, height);
    Rect::from_wh(viewport.width(), viewport.height())
}

/// The flat ground the pane's content sits on. Forms want a high-contrast,
/// opaque backdrop rather than the sidebar's material.
pub fn pane_background(dark: bool) -> Color {
    if dark {
        Color::from_rgb(0x24, 0x26, 0x2B)
    } else {
        Color::from_rgb(0xFA, 0xFA, 0xFA)
    }
}

/// The sidebar's material: the colour the compositor tints its blur with.
///
/// It lives on the compositor's layer for the surface (see `apply_material` in
/// `main.rs`), not in the buffer — a ground painted into the buffer covers the
/// frost instead of colouring it.
pub fn sidebar_material(dark: bool) -> Color {
    if dark {
        Theme::dark_palette().material_sidebar
    } else {
        Theme::light_palette().material_sidebar
    }
}

/// The flat sidebar for a surface the compositor is *not* frosting.
pub fn sidebar_flat(dark: bool) -> Color {
    if dark {
        Color::from_rgb(0x1C, 0x1E, 0x22)
    } else {
        Color::from_rgb(0xEE, 0xEE, 0xF0)
    }
}

/// The titlebar band over the content area: the pane's ground, thinned just
/// enough that the compositor's blur shows through it. It stays much more
/// opaque than the sidebar — the band is a hairline away from a wall of form
/// text and cannot start competing with it.
pub fn titlebar_material(dark: bool) -> Color {
    if dark {
        Color::from_argb(0xE6, 0x24, 0x26, 0x2B)
    } else {
        Color::from_argb(0xE6, 0xFA, 0xFA, 0xFA)
    }
}

/// Sidebar row geometry. Drawing, hit-testing, the keyboard and what a screen
/// reader is told all go through this, so none of them can drift away from the
/// painted row.
pub fn sidebar_item_rect(index: usize) -> Rect {
    const FIRST_ITEM_Y: f32 = TITLEBAR_H + 10.0;
    const ITEM_H: f32 = 30.0;
    const ITEM_STEP: f32 = 32.0;
    Rect::from_xywh(
        8.0,
        FIRST_ITEM_Y + index as f32 * ITEM_STEP,
        SIDEBAR_W - 16.0,
        ITEM_H,
    )
}

/// The sidebar as a whole, which is what the keyboard lands on.
///
/// One stop for the list, not one per row: Tab enters the sidebar and the
/// arrows move within it, which is what the list role tells a screen reader to
/// expect and what every other toolkit does.
pub const SIDEBAR_FOCUS: FocusId = FocusId::from_raw(0x5EED_5EED);

/// A sidebar row's identity for assistive technologies.
///
/// The position is the identity: the sidebar is a fixed list, so a row means
/// the same thing across every rebuild. Not a keyboard stop of its own — see
/// [`SIDEBAR_FOCUS`] — but a node an assistive technology can still click.
pub fn sidebar_focus_id(index: usize) -> FocusId {
    FocusId::new(format!("pane-{index}"))
}

/// A pane control's identity for the keyboard and for assistive technologies.
///
/// The setting's own id, which is unique within a pane and stable across the
/// rebuild the view goes through every frame.
pub fn pane_focus_id(id: &str) -> FocusId {
    FocusId::new(format!("row-{id}"))
}

/// A pop-up row's field, in the same rect [`Settings::select_hit`] tests and
/// the menu is anchored to, given the row's rect in window coordinates.
///
/// `None` for any other kind of row. Shared with the hit test so a menu opened
/// from the keyboard drops out of the same button a click would have opened.
pub fn row_select_rect(row: &Row, rect: Rect) -> Option<Rect> {
    if !matches!(row.control, Control::Select(_)) {
        return None;
    }
    Some(select_rect(
        rect.right - 14.0,
        Settings::control_band(row, rect).center_y(),
    ))
}

/// One push button's identity for the keyboard and for assistive
/// technologies.
///
/// A row of buttons is several things to do, not one: an Add beside a Remove
/// needs both within reach, so each button is its own stop rather than the row
/// being a single stop that acts on the first.
pub fn button_focus_id(row: &str, button: &str) -> FocusId {
    FocusId::new(format!("row-{row}-button-{button}"))
}

/// A button row's button labels, in order. Empty for any other row.
fn row_buttons(row: &Row) -> &'static [&'static str] {
    match &row.control {
        Control::Button(labels) => labels,
        _ => &[],
    }
}

/// A button row's buttons, in the rects the pane draws and hit-tests them at,
/// given the row's rect in whatever space the caller holds it in.
///
/// Empty for any other kind of row. Shared with [`Settings::button_hit`]
/// through [`widgets::button_rects`], so a button can never be reachable
/// somewhere it is not drawn.
pub fn row_button_rects(row: &Row, rect: Rect) -> Vec<Rect> {
    match &row.control {
        Control::Button(labels) => {
            widgets::button_rects(rect.right - 14.0, rect.center_y(), labels)
        }
        _ => Vec::new(),
    }
}

/// The pane whose sidebar row contains `(x, y)`, if any.
pub fn pane_at(x: f32, y: f32) -> Option<usize> {
    (0..model::panes().len()).find(|&i| sidebar_item_rect(i).contains(Point::new(x, y)))
}

/// The pop-up button's rect within a row, given the row's trailing edge and
/// vertical centre. Drawing, hit-testing and popup anchoring all come through
/// here so a click can never land somewhere different from what was drawn.
fn select_rect(right: f32, cy: f32) -> Rect {
    Rect::from_xywh(
        right - widgets::SELECT_W,
        cy - dropdown::field::HEIGHT / 2.0,
        widgets::SELECT_W,
        dropdown::field::HEIGHT,
    )
}

/// The text field's rect within a row. Drawing, hit-testing and the live
/// editor's own geometry all come through here.
pub fn text_rect(right: f32, cy: f32) -> Rect {
    Rect::from_xywh(
        right - widgets::TEXT_W,
        cy - widgets::CONTROL_H / 2.0,
        widgets::TEXT_W,
        widgets::CONTROL_H,
    )
}

/// Width of the key combination field on a shortcut line. Fixed, not
/// measured: it is what the field's [`TextInput`] is sized to when an edit
/// starts, and a width that moved with the window would put the caret
/// somewhere other than where the click landed.
pub const SHORTCUT_KEYS_W: f32 = 168.0;
/// Widest the action pop-up gets before the keys field starts pushing it back.
const SHORTCUT_SELECT_W: f32 = 208.0;
const SHORTCUT_GAP: f32 = 8.0;

/// The three controls of a shortcut line — action pop-up, keys field, remove
/// button — laid out between the row's leading and trailing edges.
///
/// Drawing and hit-testing both come through here, so a press can never land
/// somewhere different from what was drawn.
fn shortcut_rects(left: f32, right: f32, cy: f32) -> (Rect, Rect, Rect) {
    let remove = Rect::from_xywh(
        right - widgets::LINE_BUTTON,
        cy - widgets::LINE_BUTTON / 2.0,
        widgets::LINE_BUTTON,
        widgets::LINE_BUTTON,
    );
    let keys = Rect::from_xywh(
        remove.left - SHORTCUT_GAP - SHORTCUT_KEYS_W,
        cy - widgets::CONTROL_H / 2.0,
        SHORTCUT_KEYS_W,
        widgets::CONTROL_H,
    );
    let action = Rect::from_ltrb(
        left,
        cy - dropdown::field::HEIGHT / 2.0,
        (left + SHORTCUT_SELECT_W).min(keys.left - SHORTCUT_GAP),
        cy + dropdown::field::HEIGHT / 2.0,
    );
    (action, keys, remove)
}

/// The "+" button on the trailing line of the shortcuts group.
fn add_shortcut_rect(left: f32, cy: f32) -> Rect {
    Rect::from_xywh(
        left,
        cy - widgets::LINE_BUTTON / 2.0,
        widgets::LINE_BUTTON,
        widgets::LINE_BUTTON,
    )
}

/// The colour well's rect within a row. Its width follows the hex text, so
/// the control is measured rather than fixed; drawing, hit-testing and popup
/// anchoring all come through here.
fn well_rect(right: f32, cy: f32, color: Color) -> Rect {
    let width = color_picker::well::measure(color);
    Rect::from_xywh(right - width, cy - 22.0 / 2.0, width, 22.0)
}

/// The framed canvas within the space the pane walk reserves for the
/// arrangement — everything but the caption gap below it.
fn arrangement_canvas(reserved: Rect) -> Rect {
    Rect::from_ltrb(
        reserved.left,
        reserved.top,
        reserved.right,
        reserved.top + ARRANGEMENT_CANVAS_H,
    )
}

/// Where each output's rectangle lands inside the arrangement canvas.
///
/// Drawing and hit-testing both come through here, so a click can never
/// select a different screen from the one under the pointer. The whole
/// desktop's bounding box is fitted into the canvas with a margin, which is
/// why this cannot be a per-output calculation: moving one screen rescales
/// all of them.
fn arrangement_screens(area: Rect) -> Vec<(model::Output, Rect)> {
    let outputs = model::outputs();
    if outputs.is_empty() {
        return Vec::new();
    }

    let min_x = outputs.iter().map(|o| o.x).fold(f32::MAX, f32::min);
    let min_y = outputs.iter().map(|o| o.y).fold(f32::MAX, f32::min);
    let max_x = outputs
        .iter()
        .map(|o| o.x + o.width)
        .fold(f32::MIN, f32::max);
    let max_y = outputs
        .iter()
        .map(|o| o.y + o.height)
        .fold(f32::MIN, f32::max);
    let margin = 22.0;
    let scale = ((area.width() - margin * 2.0) / (max_x - min_x))
        .min((area.height() - margin * 2.0) / (max_y - min_y));
    let ox = area.left + (area.width() - (max_x - min_x) * scale) / 2.0 - min_x * scale;
    let oy = area.top + (area.height() - (max_y - min_y) * scale) / 2.0 - min_y * scale;

    outputs
        .into_iter()
        .map(|output| {
            let rect = Rect::from_xywh(
                ox + output.x * scale,
                oy + output.y * scale,
                output.width * scale,
                output.height * scale,
            );
            (output, rect)
        })
        .collect()
}

/// The second line inside a screen's rectangle: what it is, and whether it is
/// on. Empty for an ordinary physical display that is running, which is the
/// case that needs no explaining.
fn screen_caption(output: &model::Output) -> &'static str {
    match (output.is_virtual(), output.enabled) {
        (true, true) => "Virtual",
        (true, false) => "Virtual · Off",
        (false, true) => "",
        (false, false) => "Off",
    }
}

/// Padding `Titlebar` is given, which is also where it places its leading
/// group — so the traffic lights end up at `(TITLEBAR_PAD, TITLEBAR_PAD)`.
const TITLEBAR_PAD: f32 = (TITLEBAR_H - 12.0) / 2.0;

/// The traffic lights as the desktop wants them: ordered close-outermost for
/// whichever end of the bar they sit at, but still at the origin — the
/// `Titlebar` places the group itself.
fn window_controls() -> WindowControls {
    WindowControls::new().with_reversed(otto_kit::controls_side::side() == ControlsSide::Right)
}

/// The traffic lights for hit-testing, in window-local coordinates.
///
/// The drawn group is positioned by `Titlebar` itself and so is built at the
/// origin; only the hit-test needs the absolute offset. Getting this wrong in
/// the other direction — handing the *positioned* group to `Titlebar` — makes
/// it apply the padding twice and the dots sit low.
fn window_controls_hit(width: f32) -> WindowControls {
    let controls = window_controls();
    let x = match otto_kit::controls_side::side() {
        ControlsSide::Left => TITLEBAR_PAD,
        ControlsSide::Right => width - TITLEBAR_PAD - controls.width(),
    };
    controls.at(x, TITLEBAR_PAD)
}

/// What a press in the titlebar means.
pub enum TitlebarHit {
    /// One of the traffic lights.
    Control(otto_kit::components::titlebar::WindowControl),
    /// Bare titlebar: the press starts a window move.
    Drag,
}

/// What a window-local point hits in the titlebar, if anything.
pub fn titlebar_hit(x: f32, y: f32, width: f32) -> Option<TitlebarHit> {
    if !(0.0..=TITLEBAR_H).contains(&y) || !(0.0..=width).contains(&x) {
        return None;
    }
    match window_controls_hit(width).control_at(x, y) {
        Some(control) => Some(TitlebarHit::Control(control)),
        None => Some(TitlebarHit::Drag),
    }
}

/// The traffic light under a window-local point, if any — the hover test,
/// which unlike [`titlebar_hit`] does not care about the draggable bar.
pub fn titlebar_control_at(
    x: f32,
    y: f32,
    width: f32,
) -> Option<otto_kit::components::titlebar::WindowControl> {
    match titlebar_hit(x, y, width) {
        Some(TitlebarHit::Control(control)) => Some(control),
        _ => None,
    }
}

/// A colour well the pointer landed on, with everything needed to open it.
pub struct ColorHit {
    pub id: &'static str,
    /// The well's rect in window-local coordinates, for anchoring the popup.
    pub rect: Rect,
    pub current: Color,
}

/// A pop-up button the pointer landed on, with everything needed to open it.
pub struct SelectHit {
    pub id: &'static str,
    /// The field's rect in window-local coordinates, for anchoring the menu.
    pub rect: Rect,
    pub current: String,
}

/// A text field the pointer landed on, with everything needed to start
/// editing it in place.
pub struct TextHit {
    /// The row's label. Not every text row is bound to a setting — the
    /// Displays pane's position fields are not — so the label is what
    /// identifies a field when there is no identifier to key it on. It is
    /// unique within a pane, which is as far as an editing session reaches.
    pub label: &'static str,
    /// The setting a commit should reach, where the row has one.
    pub id: Option<&'static str>,
    /// The value the field starts from — the last committed one.
    pub current: String,
    /// Where in the field's own box the click landed, so the caret can go
    /// under the pointer rather than to a default position.
    pub local_x: f32,
}

/// A push button the pointer landed on. Both halves are labels: the row's,
/// and the button's within it.
pub struct ButtonHit {
    pub row: &'static str,
    pub button: &'static str,
}

/// A button being held down, so it can be drawn pressed.
///
/// A button acts on *release*, not on press. That is what gives a pressed
/// state something to mean — and what lets a press be taken back by sliding
/// off the button before letting go.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pressed {
    /// A row's push button, by its row's label and its own.
    Button {
        row: &'static str,
        button: &'static str,
    },
    /// A file row's "Choose…", by the setting it edits.
    Choose(&'static str),
    /// The button on a file row's preview that clears the setting.
    RemoveFile(&'static str),
    /// A shortcut line's remove button.
    Remove(usize),
    /// The button that adds a shortcut line.
    Add,
}

/// What a press on the shortcuts group means.
///
/// Separate from every other hit test because a shortcut line is three
/// controls in one row, and none of them changes a setting: the list they edit
/// lives in [`keyboard`], not in the compositor.
pub enum ShortcutHit {
    /// The action pop-up, carrying what [`Settings::select_hit`]'s caller
    /// needs to open a menu over it.
    Action(SelectHit),
    /// The key combination field, with the offset of the press inside it so
    /// the caret lands under the pointer.
    Keys { index: usize, offset_x: f32 },
    /// The "−" button: delete this line.
    Remove(usize),
    /// The "+" button: append one.
    Add,
}

/// The pointer over a file row's preview.
pub struct PreviewHit {
    pub id: &'static str,
    /// Whether it is over the button that clears the setting, rather than
    /// just over the picture that reveals it.
    pub remove: bool,
}

/// A click that landed on a control bound to a setting.
pub struct Hit {
    pub id: &'static str,
    /// The value the click implies — a toggle's opposite, or the slider value
    /// at the pointer.
    pub value: Value,
    /// Whether continuing to move the pointer should keep changing the value.
    pub draggable: bool,
}

/// One group of a pane, placed by [`Settings::pane_layout`].
struct GroupLayout<'a> {
    title: Option<&'a str>,
    /// Top of the title's 24pt band, where the group has a title.
    title_y: Option<f32>,
    /// The grouped-list card behind the rows.
    card: Rect,
    rows: Vec<(&'a Row, Rect)>,
}

impl GroupLayout<'_> {
    /// Everything the group paints, title band included.
    fn bounds(&self) -> Rect {
        match self.title_y {
            Some(top) => Rect::from_ltrb(self.card.left, top, self.card.right, self.card.bottom),
            None => self.card,
        }
    }
}

/// A whole pane placed in content-local coordinates.
struct PaneLayout<'a> {
    /// The displays arrangement canvas, on the pane that has one.
    arrangement: Option<Rect>,
    groups: Vec<GroupLayout<'a>>,
    /// Where the walk ended — the pane's content height.
    height: f32,
}

/// The decoded thumbnail for `path`, or `None` if it is not a readable image.
///
/// Decoding a 2560x1600 wallpaper is far too expensive to do per frame, and
/// the pane repaints its band on every value change — so each path is decoded
/// once, scaled down to preview size once, and kept. The cache is keyed by
/// path and holds the failures too: a missing file must not be re-read from
/// disk sixty times a second either.
///
/// One entry per path chosen this session, which is bounded by how many times
/// somebody opens the file picker; nothing here needs eviction.
fn preview_image(path: &str) -> Option<skia_safe::Image> {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<String, Option<skia_safe::Image>>> =
            std::cell::RefCell::new(HashMap::new());
    }

    CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(path) {
            return cached.clone();
        }
        let decoded = decode_preview(path);
        cache.borrow_mut().insert(path.to_string(), decoded.clone());
        decoded
    })
}

/// Read `path` and decode it, scaled down to something a thumbnail needs.
fn decode_preview(path: &str) -> Option<skia_safe::Image> {
    let bytes = std::fs::read(path).ok()?;
    let full = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))?;

    // Scaled to twice the drawn size, so the thumbnail is still sharp on a
    // 2x output without carrying the whole wallpaper around in memory.
    let target = PREVIEW_W.max(PREVIEW_H) * 2.0;
    let scale = (target / full.width().max(full.height()) as f32).min(1.0);
    if scale >= 1.0 {
        return Some(full);
    }
    let (w, h) = (
        (full.width() as f32 * scale).round() as i32,
        (full.height() as f32 * scale).round() as i32,
    );
    let info = skia_safe::ImageInfo::new_n32_premul((w, h), None);
    let mut surface = skia_safe::surfaces::raster(&info, None, None)?;
    surface
        .canvas()
        .draw_image_rect(&full, None, Rect::from_iwh(w, h), &Paint::default());
    Some(surface.image_snapshot())
}

/// The box a file row's thumbnail occupies: at most [`PREVIEW_W`]x[`PREVIEW_H`],
/// keeping the image's own aspect inside it, centred on `cx` and starting at
/// `y`. A file that cannot be decoded gets the empty 16:9 frame that
/// [`Settings::render_preview`] draws in its place.
///
/// Drawing and hit-testing both come through here: the box's width follows the
/// image, so a rect measured any other way would put the remove button
/// somewhere the picture is not.
fn preview_box(path: &str, cx: f32, y: f32) -> Rect {
    let (w, h) = match preview_image(path) {
        Some(image) => {
            let (iw, ih) = (image.width() as f32, image.height() as f32);
            let scale = (PREVIEW_W / iw).min(PREVIEW_H / ih);
            (iw * scale, ih * scale)
        }
        None => (PREVIEW_H * 16.0 / 9.0, PREVIEW_H),
    };
    Rect::from_xywh(cx - w / 2.0, y, w, h)
}

/// Where the remove button sits on a thumbnail.
fn preview_remove_rect(box_rect: Rect) -> Rect {
    Rect::from_xywh(
        box_rect.right - PREVIEW_REMOVE_INSET - PREVIEW_REMOVE_D,
        box_rect.top + PREVIEW_REMOVE_INSET,
        PREVIEW_REMOVE_D,
        PREVIEW_REMOVE_D,
    )
}

/// Which theme a row previews, where it previews one at all.
///
/// Keyed by the setting identifier rather than the label: these two rows are
/// bound, and a label is translated.
fn theme_swatch_kind(row: &Row) -> Option<ThemeSwatch> {
    match row.id? {
        "icon_theme" => Some(ThemeSwatch::Icons),
        "cursor_theme" => Some(ThemeSwatch::Cursors),
        _ => None,
    }
}

/// The two kinds of theme a row can show samples of.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemeSwatch {
    Icons,
    Cursors,
}

/// Does `rect` fall inside the band of content being asked for?
///
/// Only the vertical extent is compared: a pane is a single column that
/// always spans the full content width, so a horizontal test would reject
/// nothing and would misjudge anything (a badge, a restart pill) that a
/// measured control pushes past the nominal right edge. The comparison is
/// inclusive so a row ending exactly on the band's top edge survives, and
/// with it the separator hairline it draws on that edge.
fn intersects_band(rect: Rect, band: Rect) -> bool {
    rect.bottom >= band.top && rect.top <= band.bottom
}

pub struct Settings {
    /// The surface's current size in logical points, from the last configure.
    pub width: f32,
    pub height: f32,
    pub panes: Vec<Pane>,
    pub selected: usize,
    pub theme: Theme,
    pub dark: bool,
    /// Identifier of the row whose dropdown is currently open, so its field
    /// draws in the open state while the menu is up.
    pub open_dropdown: Option<&'static str>,
    /// Same, for the row whose colour picker is open.
    pub open_picker: Option<&'static str>,
    /// Knob positions for switches that are mid-flip, keyed by setting id.
    /// A row not listed here draws its switch at rest — see
    /// [`Settings::with_toggle_flips`].
    pub toggle_flips: HashMap<&'static str, f32>,
    /// Whether the surface carries compositor background blur, so the sidebar
    /// can be painted as a translucent material rather than a flat fill.
    pub blurred: bool,
    /// Whether this is the focused window. A background one steps back: its
    /// traffic lights go grey, its title drops a shade, and the accent drains
    /// out of everything that follows it — see [`Self::with_active`].
    pub active: bool,
    /// The button under a held pointer, drawn pressed. See [`Pressed`].
    pub pressed: Option<Pressed>,
    /// The file row whose preview the pointer is over, so that preview can
    /// show the button that clears it. Hidden otherwise: a wallpaper is worth
    /// seeing whole, and a control parked on it permanently is one more thing
    /// covering the picture than the row needs.
    pub hovered_preview: Option<&'static str>,
    /// Pointer state of the traffic lights: the app draws its own decoration,
    /// so revealing the glyphs on hover is the app's job too.
    pub controls: WindowControlsState,
    /// What is being typed into right now, and the field doing the typing.
    ///
    /// One field serves both a settings row and a shortcut line's key
    /// combination — see `EditTarget` in `main.rs`. `None`, the usual case,
    /// draws every value as static text.
    pub editing: Option<(crate::EditTarget, TextInput)>,
}

impl Settings {
    pub fn new(selected: usize, dark: bool) -> Self {
        Self {
            width: WINDOW_W,
            height: WINDOW_H,
            panes: model::panes(),
            selected,
            theme: if dark { Theme::dark() } else { Theme::light() },
            dark,
            open_dropdown: None,
            open_picker: None,
            toggle_flips: HashMap::new(),
            blurred: false,
            active: true,
            pressed: None,
            hovered_preview: None,
            controls: WindowControlsState::new(),
            editing: None,
        }
    }

    /// The live editor for `target`, if that is what currently has the
    /// keyboard.
    fn editing_field(&self, target: crate::EditTarget) -> Option<&TextInput> {
        self.editing
            .as_ref()
            .filter(|(editing, _)| *editing == target)
            .map(|(_, input)| input)
    }

    /// Carry an in-progress edit into this frame.
    pub fn with_editing(mut self, editing: Option<(crate::EditTarget, TextInput)>) -> Self {
        self.editing = editing;
        self
    }

    /// The surface has compositor blur behind it.
    pub fn with_blur(mut self, blurred: bool) -> Self {
        self.blurred = blurred;
        self
    }

    /// Whether the window is the focused one.
    ///
    /// A window in the background mutes its accent, so the selected sidebar
    /// row, the switches that are on and every other accented control stop
    /// competing with the window the user is actually working in.
    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        if !active {
            self.theme.with_muted_accent();
        }
        self
    }

    pub fn with_pressed(mut self, pressed: Option<Pressed>) -> Self {
        self.pressed = pressed;
        self
    }

    /// Carry which file row's preview is hovered into this frame.
    pub fn with_hovered_preview(mut self, id: Option<&'static str>) -> Self {
        self.hovered_preview = id;
        self
    }

    /// Whether `pressed` is this row's push button.
    fn button_pressed(&self, row: &str, button: &str) -> bool {
        matches!(self.pressed, Some(Pressed::Button { row: r, button: b }) if r == row && b == button)
    }

    /// Draw against the surface's actual size rather than the size it was
    /// created at. Clamped so a very small surface still lays out.
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width.max(MIN_W);
        self.height = height.max(MIN_H);
        self
    }

    /// The scrollable pane viewport at this window's current size, in
    /// window-local coordinates.
    pub fn viewport(&self) -> Rect {
        pane_viewport(self.width, self.height)
    }

    /// The same viewport in the pane's own coordinates — see
    /// [`pane_viewport_local`].
    pub fn local_viewport(&self) -> Rect {
        pane_viewport_local(self.width, self.height)
    }

    /// Where each mid-flip switch's knob currently is, 0.0 off to 1.0 on.
    ///
    /// The value itself is already in the store by the time a flip runs — the
    /// change is applied on press, not when the animation lands — so this is
    /// purely how the switch is drawn on the way there.
    pub fn with_toggle_flips(mut self, flips: HashMap<&'static str, f32>) -> Self {
        self.toggle_flips = flips;
        self
    }

    /// Mark one row's dropdown as open.
    pub fn with_open_dropdown(mut self, id: Option<&'static str>) -> Self {
        self.open_dropdown = id;
        self
    }

    /// Carry the traffic lights' hover/press state into this frame.
    pub fn with_controls(mut self, controls: WindowControlsState) -> Self {
        self.controls = controls;
        self
    }

    /// Mark one row's colour picker as open.
    pub fn with_open_picker(mut self, id: Option<&'static str>) -> Self {
        self.open_picker = id;
        self
    }

    fn fill(&self, color: Color) -> Paint {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color);
        paint
    }

    /// Render with the pane content unscrolled — what `--png` previews and
    /// any caller without an interactive scroll position want.
    pub fn render(&self, canvas: &Canvas) {
        self.render_with_scroll(canvas, 0.0);
    }

    /// Render with the pane content's vertical scroll at `pane_scroll_offset`
    /// (content-local points, as tracked by an externally-owned
    /// [`ScrollView`](otto_kit::components::scroll::ScrollView)). The window
    /// chrome — titlebar, sidebar — never scrolls.
    pub fn render_with_scroll(&self, canvas: &Canvas, pane_scroll_offset: f32) {
        let mut scroll = ScrollState::new(self.viewport());
        scroll.set_content_length(self.pane_content_height());
        scroll.set_offset(pane_scroll_offset);
        // A still render has no gesture to fade the overlay bar in, so show
        // it outright — a screenshot of a scrolled pane should say so.
        scroll.set_scrollbar_opacity(1.0);
        self.render_with_scroll_state(canvas, &scroll);
    }

    /// Render from a live [`ScrollState`] — the interactive path, where the
    /// offset comes with an overscroll bounce and a fading scrollbar that
    /// [`ScrollView`](otto_kit::components::scroll::ScrollView) is animating.
    pub fn render_with_scroll_state(&self, canvas: &Canvas, scroll: &ScrollState) {
        canvas.save();
        canvas.clip_rrect(self.frame(), ClipOp::Intersect, true);

        self.render_ground(canvas);
        self.render_sidebar(canvas);

        let content_width = self.width - SIDEBAR_W;
        ScrollRenderer::draw(canvas, scroll, &self.theme, |canvas, content| {
            self.render_pane(canvas, content_width, content);
        });

        self.render_titlebar(canvas);
        self.render_divider(canvas, SIDEBAR_W);

        canvas.restore();
    }

    /// Everything the window surface itself paints: the two grounds, the
    /// sidebar, the titlebar and the divider — and no pane content.
    ///
    /// The pane scrolls in its own compositor-cropped subsurfaces (see
    /// `main.rs`), which sit over the whole viewport. Painting the content
    /// here as well would only be overdraw the window would have to repaint on
    /// every frame of a scroll, which is the cost the subsurfaces exist to
    /// remove.
    pub fn render_chrome(&self, canvas: &Canvas) {
        canvas.save();
        canvas.clip_rrect(self.frame(), ClipOp::Intersect, true);

        self.render_ground(canvas);
        self.render_sidebar(canvas);
        self.render_titlebar(canvas);
        // The pane surface starts exactly at `SIDEBAR_W`, so a hairline
        // centred there would lose its right half underneath it. Nudging it
        // half a point left keeps the whole stroke on the window's own side.
        self.render_divider(canvas, SIDEBAR_W - 0.5);

        canvas.restore();
    }

    /// The pane's content for one band of it, in content-local coordinates —
    /// what the pane's own surface paints. See [`Self::render_pane`] for the
    /// coordinate space and for what `band` licenses.
    pub fn render_content(&self, canvas: &Canvas, band: Rect) {
        self.render_pane(canvas, self.width - SIDEBAR_W, band);
    }

    /// What the window is called: the app, then the pane you are in.
    pub fn title(&self) -> String {
        otto_kit::t_owned!(
            "settings-window-title",
            pane = self.panes[self.selected].name
        )
    }

    /// The window's rounded outline, which everything is clipped to.
    fn frame(&self) -> RRect {
        let corner = corner();
        RRect::new_rect_xy(Rect::from_wh(self.width, self.height), corner, corner)
    }

    /// The two backdrops: the opaque content area and the sidebar's material.
    ///
    /// The content area is painted even though the pane's own surface covers
    /// it — it is what shows through wherever that surface does not reach, in
    /// particular the gap a rubber-band overscroll opens at either end.
    fn render_ground(&self, canvas: &Canvas) {
        // Only below the titlebar: the band itself is a translucent material
        // (see `render_titlebar`), so it must not have an opaque ground under
        // it.
        canvas.draw_rect(
            Rect::from_ltrb(SIDEBAR_W, TITLEBAR_H, self.width, self.height),
            &self.fill(pane_background(self.dark)),
        );

        // The sidebar is a material over whatever the compositor blurs behind
        // the surface — and the frost, tint included, is entirely the
        // compositor's (see `apply_material` in `main.rs`). Anything painted
        // here would sit *on top* of it, so when the surface carries
        // `BackgroundBlur` the buffer is left transparent and the layer shows
        // through. Without it there is nothing behind the surface to show, so
        // paint a flat sidebar instead.
        if !self.blurred {
            canvas.draw_rect(
                Rect::from_wh(SIDEBAR_W, self.height),
                &self.fill(sidebar_flat(self.dark)),
            );
        }
    }

    /// Sidebar/content divider, drawn last so it sits above both.
    fn render_divider(&self, canvas: &Canvas, x: f32) {
        let mut hairline = Paint::default();
        hairline.set_color(self.theme.fill_tertiary);
        hairline.set_stroke_width(1.0);
        canvas.draw_line(Point::new(x, 0.0), Point::new(x, self.height), &hairline);
    }

    /// Place the selected pane's content in content-local coordinates.
    ///
    /// Drawing, measuring and hit-testing all go through this one walk, so
    /// the painted geometry, the height the scroll view is told about and the
    /// clickable rects cannot drift apart.
    fn pane_layout(&self, content_width: f32) -> PaneLayout<'_> {
        let pane = &self.panes[self.selected];
        let x0 = CONTENT_PAD;
        let x1 = content_width - CONTENT_PAD;
        let mut y = CONTENT_TOP_PAD;

        let arrangement = (pane.name == "Displays").then(|| {
            let area = Rect::from_ltrb(x0, y, x1, y + ARRANGEMENT_HEIGHT);
            y += ARRANGEMENT_HEIGHT;
            area
        });

        let mut groups = Vec::with_capacity(pane.groups.len());
        for group in &pane.groups {
            let title_y = group.title.as_ref().map(|_| {
                let top = y;
                y += 24.0;
                top
            });

            let card_top = y;
            let rows: Vec<_> = group
                .rows
                .iter()
                .map(|row| {
                    let height = Self::row_height(row);
                    let rect = Rect::from_ltrb(x0, y, x1, y + height);
                    y += height;
                    (row, rect)
                })
                .collect();
            let card = Rect::from_ltrb(x0, card_top, x1, y);
            y += GROUP_GAP;

            groups.push(GroupLayout {
                title: group.title.as_deref(),
                title_y,
                card,
                rows,
            });
        }

        PaneLayout {
            arrangement,
            groups,
            height: y,
        }
    }

    /// Total height of the selected pane's content, in the same
    /// content-local coordinate space `render_pane` draws in.
    pub fn pane_content_height(&self) -> f32 {
        self.pane_layout(self.width - SIDEBAR_W).height
    }

    /// Every row of the current pane, with where it is in *window* coordinates
    /// at the given scroll position.
    ///
    /// The same layout the pane is drawn and hit-tested from, so what the
    /// keyboard reaches and what a screen reader is told cannot drift away
    /// from what is painted. Rows scrolled out of sight are included: the
    /// keyboard's job is to reach them, and the caller scrolls to what it
    /// focuses.
    ///
    /// Every row, bound to a setting or not. Filtering to bound rows here is
    /// what once made the Displays pane unreachable: almost all of it is
    /// unbound on purpose, and a row nobody serves is still a row the user
    /// can see and has to be able to operate. Callers decide what stops on
    /// what through [`Row::focusable`].
    pub fn pane_rows(&self, scroll_offset: f32) -> Vec<(&Row, Rect)> {
        let viewport = self.viewport();
        let content_width = self.width - SIDEBAR_W;
        self.row_rects(content_width)
            .into_iter()
            .map(|(row, rect)| {
                (
                    row,
                    rect.with_offset((viewport.left, viewport.top - scroll_offset)),
                )
            })
            .collect()
    }

    fn row_rects(&self, content_width: f32) -> Vec<(&Row, Rect)> {
        self.pane_layout(content_width)
            .groups
            .into_iter()
            .flat_map(|group| group.rows)
            .collect()
    }

    /// The setting a click lands on, given a window-local point and the pane's
    /// current scroll offset. Returns the row's identifier, the value the
    /// click implies, and whether the caller should keep tracking a drag.
    pub fn hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<Hit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let id = row.id?;
        let right = rect.right - 14.0;
        let cy = Self::control_band(row, rect).center_y();

        match &row.control {
            Control::Toggle(on) => {
                let toggle = Rect::from_xywh(
                    right - widgets::TOGGLE_W,
                    cy - widgets::TOGGLE_H / 2.0,
                    widgets::TOGGLE_W,
                    widgets::TOGGLE_H,
                );
                toggle.contains(local).then(|| Hit {
                    id,
                    value: Value::Bool(!on),
                    draggable: false,
                })
            }
            Control::Slider {
                min, max, readout, ..
            } => {
                let readout_w = widgets::CONTROL_TEXT.font().measure_str(readout, None).0;
                let track_x = right - readout_w - 12.0 - widgets::SLIDER_W;
                // Generous vertically: the track is 4pt tall, which is not a
                // realistic click target.
                let track = Rect::from_xywh(track_x, cy - 12.0, widgets::SLIDER_W, 24.0);
                track.contains(local).then(|| {
                    let t = ((local.x - track_x) / widgets::SLIDER_W).clamp(0.0, 1.0);
                    Hit {
                        id,
                        value: settings_client::number_for(id, min + t * (max - min)),
                        draggable: true,
                    }
                })
            }
            _ => None,
        }
    }

    /// The identifier of the file row whose "Choose…" button a click lands
    /// on. Separate from [`Self::hit`] for the same reason a dropdown is: it
    /// does not carry a new value, it starts something that will produce one.
    pub fn file_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<&'static str> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let id = row.id?;
        if !matches!(row.control, Control::File(_)) {
            return None;
        }
        widgets::choose_rect(rect.right - 14.0, Self::control_band(row, rect).center_y())
            .contains(local)
            .then_some(id)
    }

    /// The file row whose preview a point falls on, and whether it falls on
    /// that preview's remove button.
    ///
    /// Serves both the hover — which is what reveals the button — and the
    /// press that clears the setting, so the button can never be armed by a
    /// press on a row that is not showing it.
    pub fn preview_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<PreviewHit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let id = row.id?;
        let Control::File(path) = &row.control else {
            return None;
        };
        if path.is_empty() {
            return None;
        }

        let box_rect = preview_box(
            path,
            rect.center_x(),
            rect.top + Self::control_height(row) + PREVIEW_GAP,
        );
        box_rect.contains(local).then(|| PreviewHit {
            id,
            remove: preview_remove_rect(box_rect).contains(local),
        })
    }

    /// The text field a click lands on, with the field's rect in content-local
    /// coordinates and the offset in the value the click points at.
    ///
    /// Separate from [`Self::hit`] for the same reason a dropdown is: the
    /// click does not carry a new value, it takes the keyboard.
    pub fn text_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<TextHit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let Control::Text(current) = &row.control else {
            return None;
        };

        let field = text_rect(rect.right - 14.0, Self::control_band(row, rect).center_y());
        if !field.contains(local) {
            return None;
        }
        Some(TextHit {
            // Not every text row is bound: the Displays pane's position
            // fields have no identifier, so the label is what an editing
            // session is keyed on there.
            label: row.label,
            id: row.id,
            current: current.clone(),
            // Box-local, which is what `TextInput::on_pointer_down` wants.
            local_x: local.x - field.left,
        })
    }

    /// The pop-up button a click lands on, if any. Separate from [`Self::hit`]
    /// because a dropdown does not change a value on press — it opens a menu,
    /// and the value changes when something in that menu is chosen.
    pub fn select_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<SelectHit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let id = row.id?;
        let Control::Select(current) = &row.control else {
            return None;
        };

        let field = select_rect(rect.right - 14.0, Self::control_band(row, rect).center_y());
        if !dropdown::field::hit_test(field, local.x, local.y) {
            return None;
        }

        // Back into window-local coordinates for the popup's anchor rect: the
        // menu is positioned against the surface, not against scrolled content.
        Some(SelectHit {
            id,
            rect: Rect::from_xywh(
                field.left + viewport.left,
                field.top + viewport.top - scroll_offset,
                field.width(),
                field.height(),
            ),
            current: current.clone(),
        })
    }

    /// What a click on the shortcuts group lands on, if anything.
    ///
    /// Tried before [`Self::select_hit`] by the caller: a shortcut line's
    /// action *is* a pop-up button, but one whose choices and target are the
    /// pane's rather than the settings schema's.
    pub fn shortcut_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<ShortcutHit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let left = rect.left + 14.0;
        let right = rect.right - 14.0;
        let cy = rect.center_y();

        match &row.control {
            Control::Shortcut { index } => {
                let (action, keys, remove) = shortcut_rects(left, right, cy);
                if remove.contains(local) {
                    return Some(ShortcutHit::Remove(*index));
                }
                if keys.contains(local) {
                    return Some(ShortcutHit::Keys {
                        index: *index,
                        offset_x: local.x - keys.left,
                    });
                }
                if dropdown::field::hit_test(action, local.x, local.y) {
                    // Window-local for the popup's anchor, like `select_hit`:
                    // the menu is placed against the surface, not against
                    // content that has been scrolled under it.
                    return Some(ShortcutHit::Action(SelectHit {
                        id: keyboard::slot_id(*index)?,
                        rect: Rect::from_xywh(
                            action.left + viewport.left,
                            action.top + viewport.top - scroll_offset,
                            action.width(),
                            action.height(),
                        ),
                        current: keyboard::lines().get(*index)?.action.clone(),
                    }));
                }
                None
            }
            Control::AddShortcut => add_shortcut_rect(left, cy)
                .contains(local)
                .then_some(ShortcutHit::Add),
            _ => None,
        }
    }

    /// The colour well a click lands on, if any. Like a dropdown, a well does
    /// not change a value on press — it opens a picker.
    pub fn color_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<ColorHit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let id = row.id?;
        let Control::Color(argb) = &row.control else {
            return None;
        };

        let color = Color::from(*argb);
        let well = well_rect(
            rect.right - 14.0,
            Self::control_band(row, rect).center_y(),
            color,
        );
        if !color_picker::well::hit_test(well, local.x, local.y) {
            return None;
        }

        Some(ColorHit {
            id,
            rect: Rect::from_xywh(
                well.left + viewport.left,
                well.top + viewport.top - scroll_offset,
                well.width(),
                well.height(),
            ),
            current: color,
        })
    }

    /// The push button a click lands on, if any.
    pub fn button_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<ButtonHit> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        let Control::Button(labels) = &row.control else {
            return None;
        };

        widgets::button_rects(rect.right - 14.0, rect.center_y(), labels)
            .into_iter()
            .position(|button| button.contains(local))
            .map(|index| ButtonHit {
                row: row.label,
                button: labels[index],
            })
    }

    /// The label of the switch a click lands on, for a row that is *not* bound
    /// to a setting.
    ///
    /// [`Self::hit`] only reports bound rows: it exists to produce a value for
    /// the compositor, and a row with no identifier has nowhere to send one.
    /// The displays pane's switches are all unbound (see
    /// [`crate::panes::displays`]) and still have to do something, so they are
    /// hit-tested separately and routed by label.
    pub fn unbound_toggle_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<&'static str> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(_, rect)| rect.contains(local))?;
        if row.id.is_some() || !matches!(row.control, Control::Toggle(_)) {
            return None;
        }

        let toggle = Rect::from_xywh(
            rect.right - 14.0 - widgets::TOGGLE_W,
            rect.center_y() - widgets::TOGGLE_H / 2.0,
            widgets::TOGGLE_W,
            widgets::TOGGLE_H,
        );
        toggle.contains(local).then_some(row.label)
    }

    /// The index of the screen a click lands on in the arrangement canvas —
    /// an index into [`model::outputs`], which is the order the canvas draws
    /// them in.
    pub fn screen_hit(&self, x: f32, y: f32, scroll_offset: f32) -> Option<usize> {
        let viewport = self.viewport();
        if !viewport.contains(Point::new(x, y)) {
            return None;
        }

        let content_width = self.width - SIDEBAR_W;
        let local = Point::new(x - viewport.left, y - viewport.top + scroll_offset);

        let area = self.pane_layout(content_width).arrangement?;
        arrangement_screens(arrangement_canvas(area))
            .into_iter()
            .position(|(_, rect)| rect.contains(local))
    }

    /// The value a drag to `x` implies, for a slider already being dragged.
    pub fn drag_value(&self, id: &str, x: f32) -> Option<Value> {
        let content_width = self.width - SIDEBAR_W;
        let local_x = x - SIDEBAR_W;

        let (row, rect) = self
            .row_rects(content_width)
            .into_iter()
            .find(|(row, _)| row.id == Some(id))?;

        match &row.control {
            Control::Slider {
                min, max, readout, ..
            } => {
                let readout_w = widgets::CONTROL_TEXT.font().measure_str(readout, None).0;
                let track_x = rect.right - 14.0 - readout_w - 12.0 - widgets::SLIDER_W;
                let t = ((local_x - track_x) / widgets::SLIDER_W).clamp(0.0, 1.0);
                let raw = min + t * (max - min);
                Some(settings_client::number_for(
                    id,
                    settings_client::snap(id, raw),
                ))
            }
            _ => None,
        }
    }

    fn render_titlebar(&self, canvas: &Canvas) {
        // The band over the content is slightly translucent, so the frosted
        // backdrop carries across the whole top of the window instead of
        // stopping at the sidebar's edge. Without compositor blur it would be
        // a tint over the raw desktop, so paint it flat there.
        canvas.draw_rect(
            Rect::from_ltrb(SIDEBAR_W, 0.0, self.width, TITLEBAR_H),
            &self.fill(if self.blurred {
                titlebar_material(self.dark)
            } else {
                pane_background(self.dark)
            }),
        );

        // Traffic lights sit over the sidebar — or, on a desktop that puts its
        // controls at the trailing edge, over the far end of the pane. The
        // pane name titles the bar either way.
        let group = TitlebarGroup::new().add(
            self.controls.apply(
                window_controls()
                    .with_active(self.active)
                    .with_dark(self.dark),
            ),
        );
        let bar = Titlebar::new()
            .at(0.0, 0.0)
            .with_width(self.width)
            .with_height(TITLEBAR_H)
            .with_corner_radius(corner())
            .with_padding(TITLEBAR_PAD)
            .with_background(Color::TRANSPARENT);
        match otto_kit::controls_side::side() {
            ControlsSide::Left => bar.with_leading(group),
            ControlsSide::Right => bar.with_controls(group),
        }
        .render(canvas);

        // The app and the pane, the same string the toplevel carries — so the
        // bar reads the same as the window's entry in the switcher and the
        // dock rather than naming only half of where you are.
        widgets::text_centered_y(
            canvas,
            &self.title(),
            SIDEBAR_W + CONTENT_PAD,
            TITLEBAR_H / 2.0,
            styles::TITLE_3_EMPHASIZED,
            if self.active {
                self.theme.text_primary
            } else {
                self.theme.text_secondary
            },
        );

        // Hairline under the bar, separating it from the pane. Like the
        // vertical divider it is nudged half a point onto the window's own
        // side, since the pane's surface starts exactly at `TITLEBAR_H` and
        // would take the lower half of a centred stroke with it. It stops at
        // the sidebar, which runs the full height of the window.
        let mut hairline = Paint::default();
        hairline.set_color(self.theme.fill_tertiary);
        hairline.set_stroke_width(1.0);
        canvas.draw_line(
            Point::new(SIDEBAR_W, TITLEBAR_H - 0.5),
            Point::new(self.width, TITLEBAR_H - 0.5),
            &hairline,
        );
    }

    fn render_sidebar(&self, canvas: &Canvas) {
        // No search field: it was drawn but never searched anything, and a
        // control that does nothing is worse than no control. The list starts
        // at the top of the sidebar instead — see `sidebar_item_rect`.
        // Which row the keyboard is on, if this window has it at all.
        let focused =
            AppContext::keyboard_focus().and_then(|surface| AppContext::focused_control(&surface));

        for (i, pane) in self.panes.iter().enumerate() {
            let item = sidebar_item_rect(i);
            let selected = i == self.selected;
            if selected && focused == Some(SIDEBAR_FOCUS) {
                otto_kit::focus::draw_focus_ring(canvas, item, 7.0);
            }
            if selected {
                canvas.draw_rrect(
                    RRect::new_rect_xy(item, 7.0, 7.0),
                    &self.fill(self.theme.material_selection_focused),
                );
            }
            let tint = if selected {
                Color::WHITE
            } else {
                self.theme.text_primary
            };

            glyphs::draw(
                canvas,
                pane.icon,
                item.left + 17.0,
                item.center_y(),
                15.0,
                tint,
            );

            widgets::text_centered_y(
                canvas,
                pane.name,
                item.left + 33.0,
                item.center_y(),
                if selected {
                    styles::BODY_EMPHASIZED
                } else {
                    styles::BODY
                },
                tint,
            );
        }
    }

    /// Draws in content-local coordinates: `(0, 0)` is the pane viewport's
    /// top-left, i.e. `(SIDEBAR_W, TITLEBAR_H)`. The caller is responsible
    /// for the clip and scroll translation (see [`Self::render_with_scroll`]).
    ///
    /// `content` is the band of that space the caller wants painted. Anything
    /// lying entirely outside it is skipped — text a scrolled-away row would
    /// have shaped is the expensive part of a frame, and it is invisible. A
    /// caller that wants the whole pane passes a band tall enough to hold it.
    fn render_pane(&self, canvas: &Canvas, content_width: f32, content: Rect) {
        let layout = self.pane_layout(content_width);
        let x0 = CONTENT_PAD;
        let x1 = content_width - CONTENT_PAD;

        // Which row the keyboard is on, if any. Read here rather than passed
        // in, so every path that paints the pane rings it the same way.
        let focused =
            AppContext::keyboard_focus().and_then(|surface| AppContext::focused_control(&surface));

        if let Some(area) = layout.arrangement {
            if intersects_band(area, content) {
                self.render_arrangement(canvas, x0, x1, area.top);
            }
        }

        for group in &layout.groups {
            if std::env::var_os("OTTO_PANE_DEBUG").is_some() {
                eprintln!(
                    "[groupdbg] {:?} bounds {:?} band {:?} drawn={}",
                    group.title,
                    group.bounds(),
                    content,
                    intersects_band(group.bounds(), content)
                );
            }
            if !intersects_band(group.bounds(), content) {
                continue;
            }

            if let (Some(title), Some(title_y)) = (group.title, group.title_y) {
                widgets::text_centered_y(
                    canvas,
                    title,
                    x0 + 2.0,
                    title_y + 9.0,
                    styles::SUBHEADLINE_EMPHASIZED,
                    self.theme.text_secondary,
                );
            }

            // Grouped-list card behind the rows.
            let rrect = RRect::new_rect_xy(group.card, 9.0, 9.0);
            canvas.draw_rrect(
                rrect,
                &self.fill(if self.dark {
                    Color::from_argb(0x14, 0xFF, 0xFF, 0xFF)
                } else {
                    Color::WHITE
                }),
            );
            let mut border = Paint::default();
            border.set_anti_alias(true);
            border.set_style(skia_safe::PaintStyle::Stroke);
            border.set_stroke_width(1.0);
            border.set_color(self.theme.fill_tertiary);
            canvas.draw_rrect(rrect, &border);

            for (i, (row, rect)) in group.rows.iter().enumerate() {
                if !intersects_band(*rect, content) {
                    continue;
                }
                // Around the whole row rather than the control at its edge: the
                // row is what Tab moves between, and a ring around a switch
                // alone reads as though only the switch were selected.
                //
                // Keyed on the row's handle, not on its identifier: a row the
                // compositor serves nothing for is a keyboard stop like any
                // other, and drawing only the served ones is what made Tab
                // look as though it skipped half the Displays pane when it was
                // in fact stopping there invisibly.
                if focused == Some(pane_focus_id(row.handle())) {
                    otto_kit::focus::draw_focus_ring(canvas, rect.with_inset((3.0, 1.0)), 8.0);
                }
                // A row of push buttons is a stop per button, so the ring goes
                // around the button the keyboard is on rather than around the
                // whole row.
                for (button, bounds) in row_buttons(row).iter().zip(row_button_rects(row, *rect)) {
                    if focused == Some(button_focus_id(row.handle(), button)) {
                        otto_kit::focus::draw_focus_ring(
                            canvas,
                            bounds.with_outset((3.0, 3.0)),
                            7.0,
                        );
                    }
                }
                self.render_row(canvas, row, x0, x1, rect.top, rect.height());
                if i + 1 < group.rows.len() {
                    widgets::separator(canvas, x0 + 14.0, x1, rect.bottom, &self.theme);
                }
            }
        }
    }

    /// A file row's thumbnail, centred on `cx` and starting at `y`.
    ///
    /// Centred rather than aligned to the label: the picture is the row's
    /// subject, not an annotation hanging off its text, and the box's width
    /// changes with the image's aspect — pinned to the left it would shift
    /// sideways every time a differently-shaped wallpaper was chosen.
    ///
    /// The box is at most [`PREVIEW_W`]x[`PREVIEW_H`] and keeps the image's own
    /// aspect inside it, so a portrait wallpaper is not stretched into a
    /// letterbox. A file that cannot be decoded — gone, or not an image —
    /// draws the empty frame with a line saying so rather than nothing at all,
    /// which would read as a preview still loading.
    fn render_preview(&self, canvas: &Canvas, row: &Row, path: &str, cx: f32, y: f32) {
        let image = preview_image(path);
        let box_rect = preview_box(path, cx, y);
        let rrect = RRect::new_rect_xy(box_rect, 6.0, 6.0);

        canvas.save();
        canvas.clip_rrect(rrect, ClipOp::Intersect, true);
        match &image {
            Some(image) => {
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                canvas.draw_image_rect(image, None, box_rect, &paint);
            }
            None => {
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(self.theme.fill_quaternary);
                canvas.draw_rect(box_rect, &paint);
                widgets::text_centered_y(
                    canvas,
                    "Cannot be shown",
                    box_rect.left + 10.0,
                    box_rect.center_y(),
                    styles::SUBHEADLINE,
                    self.theme.text_tertiary,
                );
            }
        }
        canvas.restore();

        // A hairline keeps a pale image from bleeding into the card behind it.
        let mut border = Paint::default();
        border.set_anti_alias(true);
        border.set_style(skia_safe::PaintStyle::Stroke);
        border.set_stroke_width(1.0);
        border.set_color(self.theme.fill_secondary);
        canvas.draw_rrect(rrect, &border);

        // Clearing the setting is the picture's own affordance: the row has
        // one control and it chooses a file, so "no wallpaper" would otherwise
        // need a path typed by hand.
        if self.hovered_preview == row.id || self.removing_file(row.id) {
            widgets::preview_remove(
                canvas,
                preview_remove_rect(box_rect),
                self.removing_file(row.id),
            );
        }
    }

    /// Whether the remove button on this row's preview is being held.
    fn removing_file(&self, id: Option<&'static str>) -> bool {
        matches!(self.pressed, Some(Pressed::RemoveFile(held)) if Some(held) == id)
    }

    /// A card of sample images from `theme`: a few icons, or a few pointers.
    ///
    /// The ground is deliberately light rather than the pane's own: icon and
    /// cursor themes are drawn to sit on a desktop, and a dark card would hide
    /// exactly the dark themes somebody is trying to compare.
    fn render_theme_swatch(
        &self,
        canvas: &Canvas,
        kind: ThemeSwatch,
        theme: &str,
        cx: f32,
        y: f32,
    ) {
        // Sourced at twice the drawn size, the way every icon in this toolkit
        // is: a 36pt slot filled by a 36px raster is soft on a 2x output, and
        // an icon that only exists at 24px would otherwise be drawn smaller
        // than the one beside it.
        let px = (SWATCH_ICON * 2.0) as i32;
        let images = match kind {
            ThemeSwatch::Icons => crate::theme_preview::icon_theme_images(theme, px),
            ThemeSwatch::Cursors => crate::theme_preview::cursor_theme_images(theme, px),
        };

        let card = Rect::from_xywh(cx - SWATCH_W / 2.0, y, SWATCH_W, SWATCH_H);
        let rrect = RRect::new_rect_xy(card, 10.0, 10.0);

        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(Color::WHITE);
        canvas.draw_rrect(rrect, &fill);

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        for (slot, image) in images.iter().enumerate() {
            let Some(image) = image else { continue };
            let left = card.left + SWATCH_PAD + slot as f32 * (SWATCH_ICON + SWATCH_GAP);
            let box_rect = Rect::from_xywh(left, card.top + SWATCH_PAD, SWATCH_ICON, SWATCH_ICON);
            let (iw, ih) = (image.width() as f32, image.height() as f32);
            // Fitted to the slot in its own aspect, so a wide cursor and a
            // square icon both sit inside the same box.
            let scale = (SWATCH_ICON / iw).min(SWATCH_ICON / ih);
            let (w, h) = (iw * scale, ih * scale);
            let dst = Rect::from_xywh(
                box_rect.center_x() - w / 2.0,
                box_rect.center_y() - h / 2.0,
                w,
                h,
            );
            canvas.draw_image_rect(image, None, dst, &paint);
        }

        // The same hairline the wallpaper thumbnail carries, for the same
        // reason: a white card on a white group needs an edge.
        let mut border = Paint::default();
        border.set_anti_alias(true);
        border.set_style(skia_safe::PaintStyle::Stroke);
        border.set_stroke_width(1.0);
        border.set_color(self.theme.fill_secondary);
        canvas.draw_rrect(rrect, &border);
    }

    /// The band a row's own controls sit in — the whole row, less anything
    /// that hangs below them.
    ///
    /// Drawing and every hit test measure the control from here rather than
    /// from the row's own centre: a file row carrying a preview is twice as
    /// tall as the line its field is on, and centring in *that* would leave
    /// the field floating in the middle of the picture.
    pub(crate) fn control_band(row: &Row, rect: Rect) -> Rect {
        Rect::from_ltrb(
            rect.left,
            rect.top,
            rect.right,
            rect.top + Self::control_height(row),
        )
    }

    fn control_height(row: &Row) -> f32 {
        if row.detail.is_some() {
            ROW_H_DETAIL
        } else {
            ROW_H
        }
    }

    /// What a row's preview adds under its controls, gaps included. A file row
    /// with something chosen has one — an empty setting has nothing to show
    /// and should not reserve a hole for it — and so do the two theme rows,
    /// which always do: their card is the same size whether or not the theme
    /// answers, so the pane does not resize as the pop-up is walked.
    fn preview_height(row: &Row) -> f32 {
        match &row.control {
            Control::File(path) if !path.is_empty() => PREVIEW_H + PREVIEW_GAP * 2.0,
            Control::Select(_) if theme_swatch_kind(row).is_some() => SWATCH_H + PREVIEW_GAP * 2.0,
            _ => 0.0,
        }
    }

    fn row_height(row: &Row) -> f32 {
        Self::control_height(row) + Self::preview_height(row)
    }

    /// The gap kept between a row's label and the control it runs into.
    const LABEL_GAP: f32 = 16.0;
    /// Room a label keeps in a narrow window before a control that can shrink
    /// starts giving way instead. Enough for a word and an ellipsis.
    const LABEL_MIN: f32 = 96.0;

    /// Where a row's trailing control begins, given the row's trailing edge
    /// and vertical centre.
    ///
    /// Every arm measures the control the same way [`Self::render_row`] draws
    /// it, so the room a label is given is the room that is actually free.
    /// Rows whose controls start at the *leading* edge — the shortcut lines —
    /// carry no label of their own, so the trailing edge is the honest answer
    /// for them.
    fn control_left(row: &Row, label_x: f32, right: f32, cy: f32) -> f32 {
        match &row.control {
            Control::Toggle(_) => right - widgets::TOGGLE_W,
            Control::Slider { readout, .. } => {
                let readout_w = widgets::CONTROL_TEXT.font().measure_str(readout, None).0;
                right - readout_w - 12.0 - widgets::SLIDER_W
            }
            Control::Select(_) => select_rect(right, cy).left,
            Control::Color(argb) => well_rect(right, cy, Color::from(*argb)).left,
            Control::Text(_) => text_rect(right, cy).left,
            Control::Button(labels) => widgets::button_rects(right, cy, labels)
                .first()
                .map(|rect| rect.left)
                .unwrap_or(right),
            Control::File(_) => {
                widgets::file_field_rect(right, label_x + Self::LABEL_MIN + Self::LABEL_GAP, cy)
                    .left
            }
            Control::Value(value) => {
                if value.contains('+') {
                    right - widgets::key_combo_width(value)
                } else {
                    right - widgets::CONTROL_TEXT.font().measure_str(value, None).0
                }
            }
            Control::Shortcut { .. } | Control::AddShortcut => right,
        }
    }

    /// A row's text cropped to the room it has.
    ///
    /// [`otto_kit::typography::ellipsize`] always keeps the ellipsis, so a
    /// room too narrow to hold even that comes back as a lone "…" wider than
    /// the space it was given — a File row at [`MIN_W`] has no room at all.
    /// Nothing is the honest answer there: an ellipsis on its own names no
    /// setting, and drawing it would put the very overlap this crop exists to
    /// prevent back under the control.
    fn crop(text: &str, style: TextStyle, room: f32) -> String {
        let font = style.font();
        if font.measure_str(text, None).0 <= room {
            return text.to_string();
        }
        if room < font.measure_str("\u{2026}", None).0 {
            return String::new();
        }
        otto_kit::typography::ellipsize(&font, text, room)
    }

    /// How much width a row's label and detail line each have before they run
    /// into the control beside them.
    ///
    /// A translated label is as long as the language makes it and the room is
    /// fixed, so the text is cropped to what is free rather than drawn over
    /// the control. The restart pill trails the text, so its room comes out of
    /// the same budget — off the detail line when there is one, off the label
    /// when there is not.
    fn text_room(row: &Row, label_x: f32, right: f32, cy: f32) -> (f32, f32) {
        let room =
            (Self::control_left(row, label_x, right, cy) - Self::LABEL_GAP - label_x).max(0.0);
        let pill_room = widgets::restart_pill_width() + 10.0;
        let (label, detail) = match (row.restart_required, row.detail.is_some()) {
            (true, true) => (room, room - pill_room),
            (true, false) => (room - pill_room, room),
            (false, _) => (room, room),
        };
        (label.max(0.0), detail.max(0.0))
    }

    fn render_row(&self, canvas: &Canvas, row: &Row, x0: f32, x1: f32, y: f32, h: f32) {
        // The controls sit on the row's first line, not in the middle of it:
        // a row carrying a preview is much taller than the line its field is
        // on. See [`Self::control_band`].
        let _ = h;
        let cy = y + Self::control_height(row) / 2.0;
        let label_x = x0 + 14.0;
        let right = x1 - 14.0;

        let (label_room, detail_room) = Self::text_room(row, label_x, right, cy);
        let label = Self::crop(row.label, styles::BODY, label_room);

        match row.detail.as_deref() {
            Some(detail) => {
                widgets::text_centered_y(
                    canvas,
                    &label,
                    label_x,
                    cy - 9.0,
                    styles::BODY,
                    self.theme.text_primary,
                );
                let detail = Self::crop(detail, styles::SUBHEADLINE, detail_room);
                widgets::text_centered_y(
                    canvas,
                    &detail,
                    label_x,
                    cy + 9.0,
                    styles::SUBHEADLINE,
                    self.theme.text_secondary,
                );
            }
            None => widgets::text_centered_y(
                canvas,
                &label,
                label_x,
                cy,
                styles::BODY,
                self.theme.text_primary,
            ),
        }

        match &row.control {
            Control::Toggle(on) => {
                let fraction = row
                    .id
                    .and_then(|id| self.toggle_flips.get(id).copied())
                    .unwrap_or_else(|| toggle::knob_fraction_for(*on));
                widgets::toggle(canvas, right - widgets::TOGGLE_W, cy, fraction, &self.theme)
            }
            Control::Slider {
                value,
                min,
                max,
                readout,
            } => {
                let readout_w = widgets::CONTROL_TEXT.font().measure_str(readout, None).0;
                let x = right - readout_w - 12.0 - widgets::SLIDER_W;
                widgets::slider(canvas, x, cy, *value, *min, *max, readout, &self.theme);
            }
            Control::Select(value) => {
                let open = row.id.is_some() && row.id == self.open_dropdown;
                // The control holds the configuration token; the field shows
                // the schema's human name for it where there is one.
                let shown = match row.id {
                    Some(id) => settings_client::display_choice(id, value),
                    None => value.clone(),
                };
                dropdown::field::draw(
                    canvas,
                    select_rect(right, cy),
                    &shown,
                    if open {
                        DropdownInteraction::Open
                    } else {
                        DropdownInteraction::Normal
                    },
                    &self.theme,
                );
            }
            Control::Color(argb) => {
                let color = Color::from(*argb);
                let open = row.id.is_some() && row.id == self.open_picker;
                color_picker::well::draw(
                    canvas,
                    well_rect(right, cy, color),
                    color,
                    if open {
                        WellInteraction::Open
                    } else {
                        WellInteraction::Normal
                    },
                    &self.theme,
                );
            }
            Control::Shortcut { index } => self.render_shortcut(canvas, *index, label_x, right, cy),
            Control::AddShortcut => {
                let button = add_shortcut_rect(label_x, cy);
                widgets::line_button(
                    canvas,
                    button,
                    true,
                    self.pressed == Some(Pressed::Add),
                    &self.theme,
                );
                widgets::text_centered_y(
                    canvas,
                    otto_kit::t!("settings-add-shortcut"),
                    button.right + 10.0,
                    cy,
                    widgets::CONTROL_TEXT,
                    self.theme.text_secondary,
                );
            }
            Control::Text(value) => {
                let field = text_rect(right, cy);
                // While a row is being edited its field *is* the editor: the
                // value underneath is what the edit started from, and drawing
                // it as well would put stale text under a live caret.
                match self.editing_field(crate::EditTarget::for_row(row.id, row.label)) {
                    Some(input) => {
                        canvas.save();
                        canvas.translate((field.left, field.top));
                        input.render_at(canvas, field.width(), field.height());
                        canvas.restore();
                    }
                    None => widgets::text_field(canvas, field, value, &self.theme),
                }
            }
            Control::Button(labels) => {
                let held = labels
                    .iter()
                    .position(|button| self.button_pressed(row.label, button));
                widgets::buttons(canvas, right, cy, labels, held, &self.theme)
            }
            Control::File(value) => widgets::file_field(
                canvas,
                right,
                label_x + Self::LABEL_MIN + Self::LABEL_GAP,
                cy,
                value,
                matches!(self.pressed, Some(Pressed::Choose(id)) if Some(id) == row.id),
                &self.theme,
            ),
            Control::Value(value) => {
                // Shortcut rows read as key combinations; everything else is
                // plain secondary text.
                if value.contains('+') {
                    widgets::key_combo(canvas, right, cy, value, &self.theme)
                } else {
                    widgets::text_right(
                        canvas,
                        value,
                        right,
                        cy,
                        widgets::CONTROL_TEXT,
                        self.theme.text_secondary,
                    )
                }
            }
        }

        // A chosen file gets shown, not just named: a wallpaper is picked by
        // eye, and a path is the one thing about it that says nothing.
        if let Control::File(path) = &row.control {
            if !path.is_empty() {
                self.render_preview(
                    canvas,
                    row,
                    path,
                    (x0 + x1) / 2.0,
                    y + Self::control_height(row) + PREVIEW_GAP,
                );
            }
        }

        // A theme is chosen by eye too, and its name is the one thing about it
        // that shows nothing.
        if let (Some(kind), Control::Select(theme)) = (theme_swatch_kind(row), &row.control) {
            self.render_theme_swatch(
                canvas,
                kind,
                theme,
                (x0 + x1) / 2.0,
                y + Self::control_height(row) + PREVIEW_GAP,
            );
        }

        // The pill trails the text it belongs to — the detail line if there is
        // one, otherwise the label — so it can never sit on top of either.
        if row.restart_required {
            let detail = row
                .detail
                .as_deref()
                .map(|detail| Self::crop(detail, styles::SUBHEADLINE, detail_room));
            let (text, style, pill_cy) = match detail.as_deref() {
                Some(detail) => (detail, styles::SUBHEADLINE, cy + 9.0),
                None => (label.as_str(), styles::BODY, cy),
            };
            let x = label_x + style.font().measure_str(text, None).0 + 10.0;
            widgets::restart_pill(canvas, x, pill_cy);
        }
    }

    /// One shortcut line: the action it runs, the combination that triggers
    /// it, and the button that deletes it.
    fn render_shortcut(&self, canvas: &Canvas, index: usize, left: f32, right: f32, cy: f32) {
        let Some(line) = keyboard::lines().into_iter().nth(index) else {
            return;
        };
        let (action, keys, remove) = shortcut_rects(left, right, cy);

        dropdown::field::draw(
            canvas,
            action,
            &line.action,
            if keyboard::slot_id(index) == self.open_dropdown {
                DropdownInteraction::Open
            } else {
                DropdownInteraction::Normal
            },
            &self.theme,
        );

        // While this line is being typed the toolkit's field owns the box —
        // it is the only thing that can draw a caret and a selection.
        match self.editing_field(crate::EditTarget::ShortcutKeys(index)) {
            Some(input) => {
                canvas.save();
                canvas.translate((keys.left, keys.top));
                input.render_at(canvas, keys.width(), keys.height());
                canvas.restore();
            }
            None => widgets::field_box(canvas, keys, &line.keys, "Unassigned", &self.theme),
        }

        widgets::line_button(
            canvas,
            remove,
            false,
            self.pressed == Some(Pressed::Remove(index)),
            &self.theme,
        );
    }

    /// Displays arrangement canvas, drawn from `y` down. It occupies
    /// [`ARRANGEMENT_HEIGHT`], which is what the pane walk reserves for it.
    fn render_arrangement(&self, canvas: &Canvas, x0: f32, x1: f32, y: f32) {
        let area = arrangement_canvas(Rect::from_ltrb(x0, y, x1, y + ARRANGEMENT_HEIGHT));
        let rrect = RRect::new_rect_xy(area, 9.0, 9.0);
        canvas.draw_rrect(
            rrect,
            &self.fill(if self.dark {
                Color::from_argb(0x14, 0xFF, 0xFF, 0xFF)
            } else {
                Color::from_argb(0x08, 0x00, 0x00, 0x00)
            }),
        );

        // The desktop's bounds, marked out around whatever the screens
        // occupy. Taken from the drawn rects rather than recomputed from the
        // model, so it cannot end up framing a layout that is not there.
        let screens = arrangement_screens(area);
        if !screens.is_empty() {
            let margin = 6.0;
            widgets::dashed_rect(
                canvas,
                Rect::from_ltrb(
                    screens.iter().map(|(_, r)| r.left).fold(f32::MAX, f32::min) - margin,
                    screens.iter().map(|(_, r)| r.top).fold(f32::MAX, f32::min) - margin,
                    screens
                        .iter()
                        .map(|(_, r)| r.right)
                        .fold(f32::MIN, f32::max)
                        + margin,
                    screens
                        .iter()
                        .map(|(_, r)| r.bottom)
                        .fold(f32::MIN, f32::max)
                        + margin,
                ),
                self.theme.fill_tertiary,
            );
        }

        let selected = model::selected_output();
        for (index, (output, rect)) in screens.iter().enumerate() {
            let rrect = RRect::new_rect_xy(*rect, 4.0, 4.0);
            // A screen that is off is drawn on the same ground as one that is
            // on, only fainter: it still holds its place in the arrangement,
            // and dropping it out of the layout would move everything else.
            let ground = if self.dark {
                Color::from_rgb(0x33, 0x39, 0x45)
            } else {
                Color::from_rgb(0xD5, 0xDC, 0xE6)
            };
            canvas.draw_rrect(
                rrect,
                &self.fill(if output.enabled {
                    ground
                } else {
                    Color::from_argb(0x4D, ground.r(), ground.g(), ground.b())
                }),
            );

            // Selection is the accent ring; being primary is the bar strip;
            // being virtual is the dashed edge. They are different states and
            // a display can have any combination of them, so the ring wins the
            // stroke and the caption carries the rest.
            let mut border = Paint::default();
            border.set_anti_alias(true);
            border.set_style(skia_safe::PaintStyle::Stroke);
            border.set_stroke_width(if index == selected { 2.0 } else { 1.0 });
            border.set_color(if index == selected {
                self.theme.accent
            } else {
                self.theme.fill_primary
            });
            if output.is_virtual() && index != selected {
                border.set_path_effect(PathEffect::dash(&[4.0, 4.0], 0.0));
            }
            canvas.draw_rrect(rrect, &border);

            if output.primary {
                let strip = Rect::from_ltrb(rect.left, rect.top, rect.right, rect.top + 4.0);
                canvas.save();
                canvas.clip_rrect(rrect, ClipOp::Intersect, true);
                canvas.draw_rect(strip, &self.fill(self.theme.text_secondary));
                canvas.restore();
            }

            let caption = screen_caption(output);
            let style = styles::SUBHEADLINE;
            let width = style.font().measure_str(&output.name, None).0;
            widgets::text_centered_y(
                canvas,
                &output.name,
                rect.center_x() - width / 2.0,
                // Two lines of text are centred as a pair, so a screen with no
                // caption does not sit its name higher than its neighbour.
                if caption.is_empty() {
                    rect.center_y()
                } else {
                    rect.center_y() - 8.0
                },
                style,
                if output.enabled {
                    self.theme.text_primary
                } else {
                    self.theme.text_tertiary
                },
            );
            if !caption.is_empty() {
                let style = styles::CAPTION_2;
                let width = style.font().measure_str(caption, None).0;
                widgets::text_centered_y(
                    canvas,
                    caption,
                    rect.center_x() - width / 2.0,
                    rect.center_y() + 8.0,
                    style,
                    self.theme.text_tertiary,
                );
            }
        }

        widgets::text_centered_y(
            canvas,
            "Click a display to change its settings below",
            x0 + 2.0,
            area.bottom + 12.0,
            styles::SUBHEADLINE,
            self.theme.text_tertiary,
        );
    }
}

/// Rounded window with a drop shadow, drawn on a desktop backdrop.
pub fn render_on_desktop(canvas: &Canvas, settings: &Settings, x: f32, y: f32) {
    let frame = Rect::from_xywh(x, y, settings.width, settings.height);
    let rrect = RRect::new_rect_xy(frame, corner(), corner());

    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_color(Color::from_argb(0x59, 0, 0, 0));
    shadow.set_mask_filter(skia_safe::MaskFilter::blur(
        skia_safe::BlurStyle::Normal,
        18.0,
        false,
    ));
    canvas.save();
    canvas.translate((0.0, 10.0));
    canvas.draw_rrect(rrect, &shadow);
    canvas.restore();

    canvas.save();
    canvas.translate((x, y));
    settings.render(canvas);
    canvas.restore();

    // Hairline edge so the window reads as separate from the wallpaper.
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(Color::from_argb(0x2E, 0xFF, 0xFF, 0xFF));
    canvas.draw_rrect(rrect, &edge);
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::PictureRecorder;

    /// The pane with the most content, so there is something to scroll past.
    fn tallest_pane() -> Settings {
        (0..model::panes().len())
            .map(|i| Settings::new(i, false))
            .max_by(|a, b| a.pane_content_height().total_cmp(&b.pane_content_height()))
            .expect("at least one pane")
    }

    /// Draw ops `render_pane` emits for one band of content.
    fn ops_for_band(settings: &Settings, band: Rect) -> usize {
        let content_width = settings.width - SIDEBAR_W;
        let mut recorder = PictureRecorder::new();
        let canvas = recorder.begin_recording(
            Rect::from_wh(content_width, settings.pane_content_height()),
            false,
        );
        settings.render_pane(canvas, content_width, band);
        recorder
            .finish_recording_as_picture(None)
            .expect("recorded picture")
            .approximate_op_count()
    }

    /// The pixel at window-local `(x, y)` after `draw` has painted a window.
    fn pixel_at(settings: &Settings, draw: impl FnOnce(&Canvas), x: i32, y: i32) -> [u8; 4] {
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((settings.width as i32, settings.height as i32))
                .expect("raster surface");
        surface.canvas().clear(Color::TRANSPARENT);
        draw(surface.canvas());
        let info = skia_safe::ImageInfo::new(
            (1, 1),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut pixel = [0u8; 4];
        assert!(surface.read_pixels(&info, &mut pixel, 4, (x, y)));
        pixel
    }

    /// The label a row actually draws, cropped the way `render_row` crops it.
    fn drawn_label(row: &Row, content_width: f32, cy: f32) -> String {
        let label_x = CONTENT_PAD + 14.0;
        let right = content_width - CONTENT_PAD - 14.0;
        let (room, _) = Settings::text_room(row, label_x, right, cy);
        Settings::crop(row.label, styles::BODY, room)
    }

    /// The pane holding the file row, with `path` chosen in it.
    ///
    /// The path is deliberately one that cannot be decoded: the preview then
    /// draws its fixed 16:9 "cannot be shown" frame, so the test knows the
    /// thumbnail's size without shipping an image to read.
    fn settings_with_wallpaper(path: &str) -> (Settings, Rect) {
        let mut settings = (0..model::panes().len())
            .map(|i| Settings::new(i, false))
            .find(|s| {
                s.row_rects(s.width - SIDEBAR_W)
                    .iter()
                    .any(|(row, _)| row.id == Some("background_image"))
            })
            .expect("a pane carries the wallpaper row");

        for group in &mut settings.panes[settings.selected].groups {
            for row in &mut group.rows {
                if row.id == Some("background_image") {
                    row.control = Control::File(path.into());
                }
            }
        }

        let content_width = settings.width - SIDEBAR_W;
        let (row, rect) = settings
            .row_rects(content_width)
            .into_iter()
            .find(|(row, _)| row.id == Some("background_image"))
            .expect("the wallpaper row");
        let box_rect = preview_box(
            path,
            rect.center_x(),
            rect.top + Settings::control_height(row) + PREVIEW_GAP,
        );
        // Back into window coordinates, unscrolled, which is what the hit
        // tests take.
        let viewport = settings.viewport();
        (
            settings,
            box_rect.with_offset((viewport.left, viewport.top)),
        )
    }

    #[test]
    fn the_remove_button_is_hit_only_on_its_own_corner_of_the_preview() {
        let (settings, preview) = settings_with_wallpaper("/nowhere/not-an-image.png");
        let button = preview_remove_rect(preview);

        let hit = settings
            .preview_hit(button.center_x(), button.center_y(), 0.0)
            .expect("the button is on the preview");
        assert_eq!(hit.id, "background_image");
        assert!(hit.remove, "the corner does not clear the setting");

        // The rest of the picture hovers the preview without arming anything:
        // a click there must not throw the wallpaper away.
        let elsewhere = settings
            .preview_hit(button.left - 6.0, button.center_y(), 0.0)
            .expect("the picture is hovered");
        assert!(!elsewhere.remove, "the picture itself clears the setting");
    }

    #[test]
    fn a_row_with_no_file_chosen_has_nothing_to_hover() {
        // Nothing is drawn, so nothing may be hit: the rows below it are where
        // that space belongs.
        let (settings, preview) = settings_with_wallpaper("");
        let button = preview_remove_rect(preview);
        assert!(settings
            .preview_hit(button.center_x(), button.center_y(), 0.0)
            .is_none());
    }

    #[test]
    fn a_label_too_long_for_its_row_is_cropped_rather_than_drawn_over_the_control() {
        // A translation is as long as the language makes it. Before the crop,
        // "Colora le icone come il Dock" ran under the switch beside it.
        let row = Row::new(
            "A label far longer than any row in any pane could ever hope to show",
            Control::Toggle(true),
        );
        // At the window's narrowest, where the room is tightest.
        let content_width = MIN_W - SIDEBAR_W;
        let label = drawn_label(&row, content_width, 20.0);

        assert!(label.ends_with('\u{2026}'), "{label:?} is not elided");
        let label_x = CONTENT_PAD + 14.0;
        let right = content_width - CONTENT_PAD - 14.0;
        let drawn_right = label_x + styles::BODY.font().measure_str(&label, None).0;
        assert!(
            drawn_right <= Settings::control_left(&row, label_x, right, 20.0),
            "the label reaches the control"
        );
    }

    #[test]
    fn no_row_label_reaches_the_control_beside_it() {
        for pane in 0..model::panes().len() {
            // At the window's narrowest: a label that clears its control here
            // clears it at every width.
            let settings = Settings::new(pane, false).with_size(MIN_W, MIN_H);
            let content_width = settings.width - SIDEBAR_W;
            for (row, rect) in settings.row_rects(content_width) {
                let cy = rect.top + Settings::control_height(row) / 2.0;
                let label = drawn_label(row, content_width, cy);
                let drawn_right =
                    CONTENT_PAD + 14.0 + styles::BODY.font().measure_str(&label, None).0;
                let control = Settings::control_left(
                    row,
                    CONTENT_PAD + 14.0,
                    content_width - CONTENT_PAD - 14.0,
                    cy,
                );
                assert!(
                    drawn_right <= control,
                    "{:?} runs into its control ({drawn_right} > {control})",
                    row.label
                );
            }
        }
    }

    #[test]
    fn the_chrome_paints_the_ground_under_the_pane_but_none_of_its_content() {
        // The pane's own surface paints the content now, so anything the
        // window paints inside the viewport is overdraw it would have to
        // repaint on every frame of a scroll.
        let settings = Settings::new(0, false);
        let card = settings.pane_layout(settings.width - SIDEBAR_W).groups[0].card;
        let x = (SIDEBAR_W + card.left + 40.0) as i32;
        let y = (TITLEBAR_H + card.top + 20.0) as i32;

        let whole = pixel_at(&settings, |canvas| settings.render(canvas), x, y);
        let chrome = pixel_at(&settings, |canvas| settings.render_chrome(canvas), x, y);
        let ground = pane_background(false);

        assert_ne!(whole, chrome, "the all-in-one render paints a card here");
        assert_eq!(chrome, [ground.r(), ground.g(), ground.b(), 0xFF]);
    }

    #[test]
    fn the_chrome_still_paints_the_sidebar_and_the_titlebar() {
        let settings = Settings::new(0, false);
        let item = sidebar_item_rect(0);
        for (x, y) in [
            (item.center_x() as i32, item.center_y() as i32),
            (SIDEBAR_W as i32 + 60, (TITLEBAR_H / 2.0) as i32),
        ] {
            assert_eq!(
                pixel_at(&settings, |canvas| settings.render(canvas), x, y),
                pixel_at(&settings, |canvas| settings.render_chrome(canvas), x, y),
            );
        }
    }

    #[test]
    fn the_panes_own_viewport_is_the_windows_with_its_origin_at_zero() {
        // What the surfaces the pane lives in are placed and sized against.
        let settings = Settings::new(0, false);
        let window = settings.viewport();
        assert_eq!(
            settings.local_viewport(),
            Rect::from_wh(window.width(), window.height())
        );
    }

    #[test]
    fn the_content_pass_draws_the_band_it_is_given() {
        // The band closure hands `render_content` a rect in content space and
        // expects exactly what the all-in-one path would have drawn for it.
        let settings = tallest_pane();
        let viewport = settings.viewport();
        let band = Rect::from_xywh(0.0, 400.0, viewport.width(), viewport.height());

        let mut recorder = PictureRecorder::new();
        let canvas = recorder.begin_recording(
            Rect::from_wh(viewport.width(), settings.pane_content_height()),
            false,
        );
        settings.render_content(canvas, band);
        let ops = recorder
            .finish_recording_as_picture(None)
            .expect("recorded picture")
            .approximate_op_count();

        assert_eq!(ops, ops_for_band(&settings, band));
        assert!(ops > 0, "the band should hold something to draw");
    }

    fn rows_in_band(settings: &Settings, band: Rect) -> usize {
        settings
            .row_rects(settings.width - SIDEBAR_W)
            .into_iter()
            .filter(|(_, rect)| intersects_band(*rect, band))
            .count()
    }

    #[test]
    fn a_pane_taller_than_its_viewport_draws_only_the_visible_band() {
        let settings = tallest_pane();
        let viewport = settings.viewport();
        assert!(
            settings.pane_content_height() > viewport.height(),
            "the test needs a pane that overflows"
        );

        let whole = Rect::from_wh(viewport.width(), settings.pane_content_height());
        let visible = Rect::from_xywh(0.0, 0.0, viewport.width(), viewport.height());

        assert!(ops_for_band(&settings, visible) < ops_for_band(&settings, whole));
        assert!(rows_in_band(&settings, visible) < rows_in_band(&settings, whole));
    }

    #[test]
    fn scrolling_draws_a_different_set_of_rows_not_a_larger_one() {
        let settings = tallest_pane();
        let viewport = settings.viewport();
        let max_offset = settings.pane_content_height() - viewport.height();

        let top = Rect::from_xywh(0.0, 0.0, viewport.width(), viewport.height());
        let bottom = Rect::from_xywh(0.0, max_offset, viewport.width(), viewport.height());

        let whole = Rect::from_wh(viewport.width(), settings.pane_content_height());
        assert!(ops_for_band(&settings, bottom) < ops_for_band(&settings, whole));

        // The last row is out of the first band and in the last one, which is
        // what makes this a band and not a prefix.
        let rows = settings.row_rects(settings.width - SIDEBAR_W);
        let last = rows.last().expect("rows").1;
        assert!(!intersects_band(last, top));
        assert!(intersects_band(last, bottom));
    }

    #[test]
    fn hit_testing_still_reaches_a_row_that_is_only_visible_when_scrolled() {
        // The tallest pane is not necessarily one with a toggle in it, and a
        // toggle is what this test aims at — so pick the tallest pane that
        // overflows its viewport AND has one, rather than assuming the two
        // coincide (they stopped coinciding once a row was removed).
        let settings = (0..model::panes().len())
            .map(|i| Settings::new(i, false))
            .filter(|s| s.pane_content_height() > s.viewport().height())
            .filter(|s| {
                s.row_rects(s.width - SIDEBAR_W)
                    .iter()
                    .any(|(row, _)| matches!(row.control, Control::Toggle(_)))
            })
            .max_by(|a, b| a.pane_content_height().total_cmp(&b.pane_content_height()))
            .expect("a scrolling pane with a toggle in it");
        let viewport = settings.viewport();
        let offset = settings.pane_content_height() - viewport.height();

        let (_, rect) = settings
            .row_rects(settings.width - SIDEBAR_W)
            .into_iter()
            .rfind(|(row, _)| matches!(row.control, Control::Toggle(_)))
            .expect("a toggle somewhere in the pane");

        let x = viewport.left + rect.right - 14.0 - widgets::TOGGLE_W / 2.0;
        let y = viewport.top + rect.center_y() - offset;
        assert!(settings.hit(x, y, offset).is_some());
    }
}
