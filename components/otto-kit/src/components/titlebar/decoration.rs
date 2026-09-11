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
    /// A tile under `decoration = "normal"`: the floating bar's height, type
    /// and all three controls, but square corners and no shadow — a tile
    /// abuts its neighbours whatever it wears on top.
    Normal,
    /// A tile under `decoration = "minimal"`: one text line high, the title,
    /// the controls at a smaller size, corners rounded at a fraction of the
    /// floating radius — see [`WindowDecoration::MINIMAL_CORNER_RADIUS`] —
    /// and no shadow.
    Minimal,
    /// A tile under `decoration = "none"`: no bar at all. The compositor marks
    /// the focused tile with a hairline border instead.
    Hidden,
}

impl DecorationVariant {
    /// The variant a tile takes under `decoration`.
    pub fn tiled(decoration: crate::tile_decoration::TileDecoration) -> Self {
        match decoration {
            crate::tile_decoration::TileDecoration::Normal => DecorationVariant::Normal,
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
    /// Height of a tile's minimal bar: one line of the body type, which is
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
    /// Title type: 14pt semibold, between the 13pt body the bar started at
    /// and the 15pt Settings once set its own bar in — the one size every
    /// floating bar on the desktop now shares.
    pub const DEFAULT_TITLE_STYLE: TextStyle = styles::TITLEBAR;
    /// Title type on the compact bar: one step down the scale, at 11pt. The
    /// floating bar's 13pt is set against 34 points of height; on a 20pt bar
    /// it crowds the strip and reads as the loudest thing on a tile that is
    /// mostly its client's. The weight stays semibold, so the title still
    /// carries at the smaller size.
    pub const MINIMAL_TITLE_STYLE: TextStyle = styles::SUBHEADLINE_EMPHASIZED;
    /// How far in from the leading edge the compact bar's control sits.
    ///
    /// Centring the dot vertically leaves it 4.5pt from the edge, which is
    /// inside the arc of a rounded window corner — the dot and the corner
    /// visibly clash. This is the floating bar's own inset, so the lights
    /// also keep the same column whether a window floats or tiles.
    pub const MINIMAL_CONTROL_INSET: f32 = 10.0;
    /// Corner radius of a tile's frame under `decoration = "minimal"`, in
    /// logical points. The floating frame's 12pt is drawn against a 34pt bar;
    /// on a bar 20pt high it swallows most of the strip, so a minimal tile
    /// keeps a softened corner at half that. The other tiled variants square
    /// off entirely, since they abut their neighbours with the full bar or
    /// none — see [`Self::corner_radius_for`].
    pub const MINIMAL_CORNER_RADIUS: f32 = 6.0;

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
            DecorationVariant::Floating | DecorationVariant::Normal => Self::DEFAULT_HEIGHT,
            DecorationVariant::Minimal => Self::MINIMAL_HEIGHT,
            DecorationVariant::Hidden => 0.0,
        }
    }

    /// The frame corner radius a variant wears, in logical points, given the
    /// `floating` radius the window has while it floats. The one place the
    /// compositor and a client both read, so a tile's bar, its frame and a
    /// server-decorated neighbour all round the same corner.
    ///
    /// A minimal tile keeps [`Self::MINIMAL_CORNER_RADIUS`]; the normal and
    /// hidden variants square off, as a maximized window does. Respects the
    /// desktop's rounded-corners setting like [`crate::corners::radius`].
    pub fn corner_radius_for(variant: DecorationVariant, floating: f32) -> f32 {
        match variant {
            DecorationVariant::Floating => crate::corners::radius(floating),
            DecorationVariant::Minimal => crate::corners::radius(Self::MINIMAL_CORNER_RADIUS),
            DecorationVariant::Normal | DecorationVariant::Hidden => 0.0,
        }
    }

    /// Wear `variant`, taking its height and its corners with it. A tile
    /// abuts its neighbours on every side it touches, so it squares off as a
    /// maximized window does — except the minimal one, which keeps a small
    /// radius; see [`Self::corner_radius_for`].
    pub fn with_variant(mut self, variant: DecorationVariant) -> Self {
        self.variant = variant;
        self.titlebar_height = Self::height_for(variant);
        self.corner_radius = Self::corner_radius_for(variant, self.corner_radius);
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
        // Every variant offers the same group — close, minimize, and the zoom
        // dot when the desktop shows it. The minimal bar draws them smaller
        // rather than fewer: a tile is still minimized and zoomed from its
        // bar, and 11pt dots with the usual gap fit a 20pt strip.
        WindowControls::new()
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
        let padding = self.horizontal_padding();
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
        Self::control_size_for(self.variant)
    }

    /// Diameter of a dot on a bar wearing `variant`. Public so an application
    /// that draws its own bar sizes its dots as the compositor does beside it.
    pub fn control_size_for(variant: DecorationVariant) -> f32 {
        match variant {
            DecorationVariant::Minimal => Self::MINIMAL_CONTROL_SIZE,
            _ => Self::CONTROL_SIZE,
        }
    }

    /// The type a title is set in on a bar wearing `variant`, for an
    /// application drawing its own bar — the same answer
    /// [`Self::resolved_title_style`] gives the shared component.
    pub fn title_style_for(variant: DecorationVariant) -> TextStyle {
        match variant {
            DecorationVariant::Minimal => Self::MINIMAL_TITLE_STYLE,
            _ => Self::DEFAULT_TITLE_STYLE,
        }
    }

    /// How far in from either end the bar's groups sit.
    ///
    /// The full bar is inset by the same amount all round, which is what
    /// centres its dots. The compact bar cannot be: 4.5pt is all the height
    /// it has to spare above an 11pt dot, and that same distance along the
    /// leading edge puts the dot inside a rounded corner's arc. So the two
    /// come apart here, and the compact bar keeps the floating bar's inset
    /// horizontally while centring vertically on its own.
    fn horizontal_padding(&self) -> f32 {
        match self.variant {
            DecorationVariant::Minimal => Self::MINIMAL_CONTROL_INSET,
            _ => self.padding(),
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

    /// The type the title is set in on this bar.
    ///
    /// The compact bar has its own size — see [`Self::MINIMAL_TITLE_STYLE`] —
    /// and takes it whatever [`Self::title_style`] holds, so the compositor's
    /// struct literal and a client's builder land on the same bar.
    fn resolved_title_style(&self) -> TextStyle {
        match self.variant {
            DecorationVariant::Minimal => Self::MINIMAL_TITLE_STYLE,
            _ => self.title_style,
        }
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
            .with_horizontal_padding(self.horizontal_padding())
            .with_material(self.material())
            .with_title(
                Label::new(&self.title)
                    .with_style(self.resolved_title_style())
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

    /// The minimal bar is one line of the body type. Written as a constant
    /// because the compositor's geometry needs it before any font is loaded;
    /// this is what keeps that constant honest against the type scale.
    #[test]
    fn minimal_height_matches_the_title_line() {
        let line = (styles::BODY_EMPHASIZED.size * 1.5).ceil();
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
        assert_eq!(
            minimal.corner_radius,
            WindowDecoration::MINIMAL_CORNER_RADIUS
        );

        let hidden = floating.with_variant(DecorationVariant::Hidden);
        assert_eq!(hidden.titlebar_height, 0.0);
        assert_eq!(hidden.corner_radius, 0.0);
    }

    /// A minimal tile's corner is rounded, but at a fraction of the floating
    /// frame's: the bar is a fraction of the height, and the same radius
    /// would swallow it. Every other tile squares off.
    #[test]
    fn a_minimal_tile_keeps_a_smaller_corner() {
        let floating = 12.0;
        let minimal = WindowDecoration::corner_radius_for(DecorationVariant::Minimal, floating);
        assert!(minimal > 0.0);
        assert!(
            minimal <= floating / 2.0,
            "{minimal} against {floating} floating"
        );
        assert_eq!(
            WindowDecoration::corner_radius_for(DecorationVariant::Floating, floating),
            floating
        );
        assert_eq!(
            WindowDecoration::corner_radius_for(DecorationVariant::Normal, floating),
            0.0
        );
        assert_eq!(
            WindowDecoration::corner_radius_for(DecorationVariant::Hidden, floating),
            0.0
        );
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

    /// The compact bar sets its title smaller than the floating bar's, and
    /// says so whatever the caller put in the field — the compositor builds
    /// this struct as a literal and never touches `title_style`.
    #[test]
    fn the_compact_bar_sets_its_title_smaller() {
        let floating = WindowDecoration::new("t", 400.0);
        assert_eq!(
            floating.resolved_title_style().size,
            WindowDecoration::DEFAULT_TITLE_STYLE.size
        );

        let minimal = floating.clone().with_variant(DecorationVariant::Minimal);
        assert!(
            minimal.resolved_title_style().size < floating.resolved_title_style().size,
            "compact title {} is not smaller than the floating {}",
            minimal.resolved_title_style().size,
            floating.resolved_title_style().size
        );
        // Even against a caller that asked for the big one.
        let insistent = minimal.with_title_style(WindowDecoration::DEFAULT_TITLE_STYLE);
        assert_eq!(
            insistent.resolved_title_style().size,
            WindowDecoration::MINIMAL_TITLE_STYLE.size
        );
    }

    /// The compact bar's control clears a rounded corner, and keeps the
    /// column the floating bar's lights sit in.
    #[test]
    fn the_compact_control_is_inset_clear_of_the_corner() {
        let floating = WindowDecoration::new("t", 400.0);
        let minimal = floating.clone().with_variant(DecorationVariant::Minimal);

        // Vertically it still centres on its own short bar.
        assert!(minimal.padding() < floating.padding());
        // Horizontally it does not: that is the inset a corner arc needs.
        assert_eq!(
            minimal.horizontal_padding(),
            WindowDecoration::MINIMAL_CONTROL_INSET
        );
        assert!(minimal.horizontal_padding() > minimal.padding());

        // Nothing of the dot is drawn inside the inset, at either end.
        let leftmost = (0..400)
            .find(|x| {
                minimal
                    .control_at(*x as f32, minimal.titlebar_height / 2.0)
                    .is_some()
            })
            .expect("compact bar drew no control");
        assert!(
            leftmost as f32 >= WindowDecoration::MINIMAL_CONTROL_INSET,
            "control starts at {leftmost}, inside the {} inset",
            WindowDecoration::MINIMAL_CONTROL_INSET
        );
    }

    /// `decoration = "normal"` is the floating bar on a window that is still
    /// a tile: same height and controls, squared corners, and it counts as
    /// tiled everywhere that matters.
    #[test]
    fn the_normal_variant_is_the_full_bar_on_a_tile() {
        let normal = WindowDecoration::new("t", 400.0)
            .with_corner_radius(12.0)
            .with_variant(DecorationVariant::Normal);

        assert_eq!(normal.titlebar_height, WindowDecoration::DEFAULT_HEIGHT);
        assert_eq!(normal.corner_radius, 0.0);
        assert!(normal.variant.is_tiled());
        assert_eq!(
            normal.resolved_title_style().size,
            WindowDecoration::DEFAULT_TITLE_STYLE.size
        );

        // The full group. Which dots that is depends on the desktop —
        // `show_maximize_button` can drop the zoom — so this asks for the
        // ones no setting takes away.
        let offered: Vec<WindowControl> = (0..400)
            .filter_map(|x| normal.control_at(x as f32, normal.titlebar_height / 2.0))
            .collect();
        for control in [WindowControl::Close, WindowControl::Minimize] {
            assert!(offered.contains(&control), "normal bar has no {control:?}");
        }
    }

    /// A minimal bar keeps the whole group, smaller, at either end: a tile
    /// is still minimized from its bar.
    #[test]
    fn a_minimal_bar_keeps_the_full_group() {
        for side in [ControlsSide::Left, ControlsSide::Right] {
            let minimal = WindowDecoration::new("t", 400.0)
                .with_controls_side(side)
                .with_variant(DecorationVariant::Minimal);
            let hits: Vec<WindowControl> = (0..400)
                .filter_map(|x| minimal.control_at(x as f32, minimal.titlebar_height / 2.0))
                .collect();
            for control in [WindowControl::Close, WindowControl::Minimize] {
                assert!(hits.contains(&control), "{side:?} bar has no {control:?}");
            }
            // Smaller than the floating bar's, and still inside the strip.
            assert!(minimal.control_size() < WindowDecoration::CONTROL_SIZE);
            assert!(minimal.control_size() < minimal.titlebar_height);
        }
    }
}
