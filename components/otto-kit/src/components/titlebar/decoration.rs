use skia_safe::{Canvas, Color, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

use super::{
    SharingIndicator, Titlebar, TitlebarGroup, TitlebarMaterial, WindowControl, WindowControls,
};
use crate::controls_side::ControlsSide;

/// Everything needed to draw one window's decoration, and nothing about who is
/// drawing it.
///
/// This is the single description of the Otto window decoration: the
/// compositor renders it for server-side decorated windows, and otto-kit
/// clients render the same struct into their own surface. Both go through
/// [`WindowDecoration::draw`], so the two can never drift apart.
///
/// Coordinates are in logical points, with the origin at the window's top-left
/// (the decoration's own origin, not the client area's).
#[derive(Debug, Clone)]
pub struct WindowDecoration {
    pub title: String,
    /// Width of the window (and so of the titlebar)
    pub width: f32,
    /// Height of the titlebar strip
    pub titlebar_height: f32,
    /// Corner radius of the window frame; the bar rounds its top two corners
    /// to match. Maximized/tiled windows pass 0.
    pub corner_radius: f32,
    /// Focused window: colored controls, stronger material
    pub active: bool,
    pub dark: bool,
    /// Controls are shown at all (fullscreen/tiled cases may drop them)
    pub show_controls: bool,
    /// Pointer is over the control group, so the glyphs are revealed
    pub controls_hovered: bool,
    /// Control being held down
    pub pressed: Option<WindowControl>,
    /// Controls the window doesn't support (e.g. a non-resizable window has no
    /// zoom), drawn gray even when focused
    pub disabled: Vec<WindowControl>,
    /// The window's contents are being screencast: a badge appears at the
    /// trailing end of the bar, the way macOS marks a shared window.
    pub sharing: bool,
    /// Local backdrop blur sigma. Leave at 0 when the surface already carries
    /// `background_blur` — the compositor blurs behind it and blurring again
    /// here would double up.
    pub backdrop_blur: f32,
    /// Whether anything is blurred behind the bar — either the compositor
    /// blurring under the surface, or [`Self::backdrop_blur`] doing it here.
    /// With nothing behind it the material is filled in instead of left
    /// translucent; see [`TitlebarMaterial::opaque`].
    pub blurred: bool,
    /// Leave the material's tint to whoever owns the layer this is drawn on:
    /// only the sheen and the bevel hairlines are painted, over whatever is
    /// already there.
    ///
    /// The compositor sets this for a server-side titlebar. The tint is the
    /// one thing about the bar that changes when focus comes and goes, and on
    /// its own layer it can be *faded* between the frosted and the opaque form
    /// — see `view_window_decoration` — where painting it here would mean
    /// repainting the whole bar on every frame of that fade. The colour to use
    /// comes from [`Self::material_tint`].
    pub tint_on_layer: bool,
    /// Type style of the title
    pub title_style: TextStyle,
    /// Which end of the bar the traffic lights sit at. Defaults to what the
    /// desktop is configured for, so a client that says nothing follows it.
    pub controls_side: ControlsSide,
}

impl Default for WindowDecoration {
    fn default() -> Self {
        Self {
            title: String::new(),
            width: 0.0,
            titlebar_height: Self::DEFAULT_HEIGHT,
            corner_radius: crate::corners::radius(12.0),
            active: true,
            dark: false,
            show_controls: true,
            controls_hovered: false,
            pressed: None,
            disabled: Vec::new(),
            sharing: false,
            backdrop_blur: 0.0,
            blurred: true,
            tint_on_layer: false,
            title_style: Self::DEFAULT_TITLE_STYLE,
            controls_side: crate::controls_side::side(),
        }
    }
}

impl WindowDecoration {
    /// Height of the titlebar strip, in logical points
    pub const DEFAULT_HEIGHT: f32 = 34.0;
    /// Diameter of one traffic-light dot
    pub const CONTROL_SIZE: f32 = 13.0;
    /// Gap between dots
    pub const CONTROL_SPACING: f32 = 8.0;
    /// Title type: 13pt semibold, one step up from the secondary-label size
    /// the bar started at.
    pub const DEFAULT_TITLE_STYLE: TextStyle = styles::BODY_EMPHASIZED;

    pub fn new(title: impl Into<String>, width: f32) -> Self {
        Self {
            title: title.into(),
            width,
            ..Default::default()
        }
    }

    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn with_dark(mut self, dark: bool) -> Self {
        self.dark = dark;
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.titlebar_height = height;
        self
    }

    /// Round the window frame's corners to `radius` — or leave them square,
    /// on a desktop configured without rounded corners.
    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = crate::corners::radius(radius);
        self
    }

    pub fn with_sharing(mut self, sharing: bool) -> Self {
        self.sharing = sharing;
        self
    }

    pub fn with_backdrop_blur(mut self, sigma: f32) -> Self {
        self.backdrop_blur = sigma;
        self
    }

    pub fn with_blurred(mut self, blurred: bool) -> Self {
        self.blurred = blurred;
        self
    }

    pub fn with_tint_on_layer(mut self, on_layer: bool) -> Self {
        self.tint_on_layer = on_layer;
        self
    }

    /// Put the traffic lights at one end of the bar or the other.
    pub fn with_controls_side(mut self, side: ControlsSide) -> Self {
        self.controls_side = side;
        self
    }

    pub fn with_title_style(mut self, style: TextStyle) -> Self {
        self.title_style = style;
        self
    }

    /// Vertical inset the client content sits at
    pub fn content_offset(&self) -> f32 {
        self.titlebar_height
    }

    /// The control group's appearance. Drawn at the origin — `Titlebar` places
    /// the leading group itself — and offset by [`Self::padding`] for hit
    /// testing, which is exactly where `Titlebar` puts it.
    fn controls(&self) -> WindowControls {
        WindowControls::new()
            .with_size(Self::CONTROL_SIZE)
            .with_spacing(Self::CONTROL_SPACING)
            .with_active(self.active)
            .with_hovered(self.controls_hovered)
            .with_pressed(self.pressed)
            .with_disabled(self.disabled.clone())
            .with_dark(self.dark)
            .with_reversed(self.controls_side == ControlsSide::Right)
    }

    /// Where the control group's left edge sits, in window-local coordinates.
    fn controls_x(&self) -> f32 {
        let padding = self.padding();
        match self.controls_side {
            ControlsSide::Left => padding,
            ControlsSide::Right => self.width - self.controls().width() - padding,
        }
    }

    /// The screencast badge, sized off the bar so it stays proportionate on a
    /// compact or a tall titlebar.
    fn sharing_indicator(&self) -> SharingIndicator {
        SharingIndicator::new()
            .with_height((self.titlebar_height * 0.53).clamp(14.0, 20.0))
            .with_active(self.active)
            .with_dark(self.dark)
    }

    /// Dots are vertically centered, so the padding follows the bar height.
    fn padding(&self) -> f32 {
        ((self.titlebar_height - Self::CONTROL_SIZE) / 2.0).max(4.0)
    }

    /// Which control is under a window-local point, if any.
    pub fn control_at(&self, x: f32, y: f32) -> Option<WindowControl> {
        if !self.show_controls {
            return None;
        }
        self.controls()
            .at(self.controls_x(), self.padding())
            .control_at(x, y)
    }

    /// Whether a window-local point is in the draggable part of the titlebar
    /// (the bar, minus the controls).
    pub fn is_drag_area(&self, x: f32, y: f32) -> bool {
        if y < 0.0 || y > self.titlebar_height || x < 0.0 || x > self.width {
            return false;
        }
        self.control_at(x, y).is_none()
    }

    /// Whether a window-local point is anywhere in the titlebar strip.
    pub fn hits_titlebar(&self, x: f32, y: f32) -> bool {
        y >= 0.0 && y <= self.titlebar_height && x >= 0.0 && x <= self.width
    }

    /// The material for this bar's colour scheme and focus, before anything
    /// is decided about how — or whether — its tint gets painted.
    fn base_material(&self) -> TitlebarMaterial {
        let base = match (self.dark, self.active) {
            (false, true) => TitlebarMaterial::light_active(),
            (false, false) => TitlebarMaterial::light_inactive(),
            (true, true) => TitlebarMaterial::dark_active(),
            (true, false) => TitlebarMaterial::dark_inactive(),
        };
        base.with_backdrop_blur(self.backdrop_blur)
    }

    /// The bar's tint, for a caller painting it itself — see
    /// [`Self::tint_on_layer`].
    ///
    /// `frosted` picks which end of the fade is wanted: the material's own
    /// translucency, or that same colour filled in to full opacity for a bar
    /// with nothing blurred behind it.
    pub fn material_tint(&self, frosted: bool) -> Color {
        let material = self.base_material();
        if frosted {
            material.tint
        } else {
            material.opaque().tint
        }
    }

    fn material(&self) -> TitlebarMaterial {
        let base = self.base_material();
        if self.tint_on_layer {
            return base.with_tint(Color::TRANSPARENT);
        }
        if self.blurred {
            base
        } else {
            base.opaque()
        }
    }

    fn title_color(&self) -> Color {
        let theme = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        if self.active {
            theme.text_primary
        } else {
            theme.text_tertiary
        }
    }

    /// Draw the decoration with the window's top-left at the canvas origin.
    ///
    /// Only the titlebar strip is painted — the window's own background,
    /// shadow and corner clipping belong to whoever owns the frame (the
    /// compositor's shadow layer, or the client's surface).
    pub fn draw(&self, canvas: &Canvas) {
        let mut titlebar = Titlebar::new()
            .at(0.0, 0.0)
            .with_width(self.width)
            .with_height(self.titlebar_height)
            .with_corner_radius(self.corner_radius)
            .with_padding(self.padding())
            .with_material(self.material())
            .with_title(
                Label::new(&self.title)
                    .with_style(self.title_style)
                    .with_color(self.title_color()),
            );

        // `Titlebar` places its leading group at the left edge and its trailing
        // one at the right, so which group the lights go in *is* the side they
        // land on. The sharing badge always takes the other end: it never
        // collides with them, and the group reserves its width, which keeps a
        // long title clear of both.
        let lights = self
            .show_controls
            .then(|| TitlebarGroup::new().add(self.controls()));
        let badge = self
            .sharing
            .then(|| TitlebarGroup::new().add(self.sharing_indicator()));
        let (leading, trailing) = match self.controls_side {
            ControlsSide::Left => (lights, badge),
            ControlsSide::Right => (badge, lights),
        };
        if let Some(leading) = leading {
            titlebar = titlebar.with_leading(leading);
        }
        if let Some(trailing) = trailing {
            titlebar = titlebar.with_controls(trailing);
        }

        titlebar.render(canvas);
    }

    /// Bounds of the titlebar strip, for damage tracking.
    pub fn bounds(&self) -> Rect {
        Rect::from_wh(self.width, self.titlebar_height)
    }
}
