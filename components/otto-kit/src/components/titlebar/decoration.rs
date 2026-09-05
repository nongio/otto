use skia_safe::{Canvas, Color, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

use super::{
    SharingIndicator, Titlebar, TitlebarGroup, TitlebarMaterial, WindowControl, WindowControls,
};
use crate::controls_side::ControlsSide;

/// How much of a decoration a window gets.
///
/// A floating window gets the full bar; a tile gets less, and how much less is
/// the desktop's `[tiling] decoration` setting — see
/// [`crate::tile_decoration`]. The variant is what both the compositor and an
/// otto-kit client switch on, so a server-decorated tile and a client-drawn
/// one end up the same height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecorationVariant {
    /// The full bar: title, all three controls, rounded top corners, shadow.
    #[default]
    Floating,
    /// A tile under `decoration = "minimal"`: one text line high, the title,
    /// the close control alone, square corners, no shadow.
    Minimal,
    /// A tile under `decoration = "none"`: no bar at all. The compositor marks
    /// the focused tile with a hairline border instead.
    Hidden,
}

impl DecorationVariant {
    /// The variant a tile takes under `decoration`.
    pub fn tiled(decoration: crate::tile_decoration::TileDecoration) -> Self {
        match decoration {
            crate::tile_decoration::TileDecoration::Minimal => DecorationVariant::Minimal,
            crate::tile_decoration::TileDecoration::None => DecorationVariant::Hidden,
        }
    }

    /// Whether a window wearing this variant is one of the tree's tiles: the
    /// cases that square their corners and drop their shadow, alongside
    /// maximized.
    pub fn is_tiled(self) -> bool {
        !matches!(self, DecorationVariant::Floating)
    }
}

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
    /// How much of a decoration this window gets. A tile gets less; see
    /// [`DecorationVariant`].
    pub variant: DecorationVariant,
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
            variant: DecorationVariant::Floating,
        }
    }
}

impl WindowDecoration {
    /// Height of the titlebar strip, in logical points
    pub const DEFAULT_HEIGHT: f32 = 34.0;
    /// Height of a tile's minimal bar: one line of the title type, which is
    /// 13pt on a 1.5 line, rounded up — a little over half the floating bar.
    /// [`Self::minimal_height_matches_the_title_line`] holds it to that.
    pub const MINIMAL_HEIGHT: f32 = 20.0;
    /// Diameter of one traffic-light dot
    pub const CONTROL_SIZE: f32 = 13.0;
    /// The same dot on a minimal bar, where 13pt would leave no room to
    /// centre it.
    pub const MINIMAL_CONTROL_SIZE: f32 = 11.0;
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

    /// The bar height a variant asks for, in logical points. The one place
    /// the compositor and a client both read, so a tile's client area is
    /// configured with exactly what the bar leaves.
    pub fn height_for(variant: DecorationVariant) -> f32 {
        match variant {
            DecorationVariant::Floating => Self::DEFAULT_HEIGHT,
            DecorationVariant::Minimal => Self::MINIMAL_HEIGHT,
            DecorationVariant::Hidden => 0.0,
        }
    }

    /// Wear `variant`, taking its height and — for a tile — its square
    /// corners with it. A tile abuts its neighbours on every side it touches,
    /// so it squares off exactly as a maximized window does.
    pub fn with_variant(mut self, variant: DecorationVariant) -> Self {
        self.variant = variant;
        self.titlebar_height = Self::height_for(variant);
        if variant.is_tiled() {
            self.corner_radius = 0.0;
        }
        self
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
        let controls = WindowControls::new();
        // A minimal bar keeps the close control alone: it is the one thing a
        // tile still needs a target for, and there is no room for three.
        let controls = if self.variant == DecorationVariant::Minimal {
            controls.close_only()
        } else {
            controls
        };
        controls
            .with_size(self.control_size())
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

    /// Diameter of a dot on this bar.
    fn control_size(&self) -> f32 {
        match self.variant {
            DecorationVariant::Minimal => Self::MINIMAL_CONTROL_SIZE,
            _ => Self::CONTROL_SIZE,
        }
    }

    /// Dots are vertically centered, so the padding follows the bar height.
    fn padding(&self) -> f32 {
        let floor = if self.variant == DecorationVariant::Minimal {
            // A 20pt bar has 4.5pt above and below an 11pt dot; the 4pt floor
            // the full bar needs would push it off centre.
            0.0
        } else {
            4.0
        };
        ((self.titlebar_height - self.control_size()) / 2.0).max(floor)
    }

    /// Which control is under a window-local point, if any.
    pub fn control_at(&self, x: f32, y: f32) -> Option<WindowControl> {
        if !self.show_controls || self.variant == DecorationVariant::Hidden {
            return None;
        }
        self.controls()
            .at(self.controls_x(), self.padding())
            .control_at(x, y)
    }

    /// Whether a window-local point is in the draggable part of the titlebar
    /// (the bar, minus the controls).
    pub fn is_drag_area(&self, x: f32, y: f32) -> bool {
        if !self.hits_titlebar(x, y) {
            return false;
        }
        self.control_at(x, y).is_none()
    }

    /// Whether a window-local point is anywhere in the titlebar strip.
    pub fn hits_titlebar(&self, x: f32, y: f32) -> bool {
        // A window with no bar has no strip to hit: a zero-height one would
        // still answer for the single row at y == 0.
        if self.variant == DecorationVariant::Hidden || self.titlebar_height <= 0.0 {
            return false;
        }
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
        // `none` draws nothing at all — the compositor marks the focused tile
        // with a hairline border instead.
        if self.variant == DecorationVariant::Hidden {
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimal bar is one line of the title type. Written as a constant
    /// because the compositor's geometry needs it before any font is loaded;
    /// this is what keeps that constant honest against the type scale.
    #[test]
    fn minimal_height_matches_the_title_line() {
        let line = (WindowDecoration::DEFAULT_TITLE_STYLE.size * 1.5).ceil();
        assert_eq!(WindowDecoration::MINIMAL_HEIGHT, line);
        // "Roughly half the floating bar", and never taller than it.
        assert!(WindowDecoration::MINIMAL_HEIGHT < WindowDecoration::DEFAULT_HEIGHT);
        assert!(WindowDecoration::MINIMAL_HEIGHT > WindowDecoration::DEFAULT_HEIGHT / 2.0 - 4.0);
    }

    #[test]
    fn a_variant_carries_its_height_and_its_corners() {
        let floating = WindowDecoration::new("t", 400.0).with_corner_radius(12.0);
        assert_eq!(floating.titlebar_height, WindowDecoration::DEFAULT_HEIGHT);

        let minimal = floating.clone().with_variant(DecorationVariant::Minimal);
        assert_eq!(minimal.titlebar_height, WindowDecoration::MINIMAL_HEIGHT);
        assert_eq!(minimal.corner_radius, 0.0);

        let hidden = floating.with_variant(DecorationVariant::Hidden);
        assert_eq!(hidden.titlebar_height, 0.0);
        assert_eq!(hidden.corner_radius, 0.0);
    }

    /// Hit testing follows the variant, or a drag lands on a bar that is not
    /// drawn and a click misses the one that is.
    #[test]
    fn hit_testing_follows_the_variant() {
        let floating = WindowDecoration::new("t", 400.0);
        assert!(floating.hits_titlebar(200.0, 30.0));

        let minimal = floating.clone().with_variant(DecorationVariant::Minimal);
        assert!(minimal.hits_titlebar(200.0, 10.0));
        // Below the compact bar is the client's own content.
        assert!(!minimal.hits_titlebar(200.0, 30.0));

        let hidden = floating.with_variant(DecorationVariant::Hidden);
        assert!(!hidden.hits_titlebar(200.0, 0.0));
        assert!(hidden.control_at(0.0, 0.0).is_none());
    }

    /// A minimal bar keeps close and nothing else, at either end.
    #[test]
    fn a_minimal_bar_keeps_only_close() {
        for side in [ControlsSide::Left, ControlsSide::Right] {
            let minimal = WindowDecoration::new("t", 400.0)
                .with_controls_side(side)
                .with_variant(DecorationVariant::Minimal);
            let hits: Vec<WindowControl> = (0..400)
                .filter_map(|x| minimal.control_at(x as f32, minimal.titlebar_height / 2.0))
                .collect();
            assert!(
                hits.iter().all(|c| *c == WindowControl::Close),
                "{side:?} bar offered {hits:?}"
            );
            assert!(!hits.is_empty(), "{side:?} bar offered no close control");
        }
    }
}
