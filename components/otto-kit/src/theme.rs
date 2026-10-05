use crate::frosted::Frosted;
use skia_safe::Color;

/// System color scheme preference, matching XDG `org.freedesktop.appearance color-scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    NoPreference,
    Dark,
    Light,
}

impl ColorScheme {
    /// Construct from the XDG portal integer value.
    pub fn from_portal_value(v: u32) -> Self {
        match v {
            1 => Self::Dark,
            2 => Self::Light,
            _ => Self::NoPreference,
        }
    }
}

/// Every accent colour a user can choose, in the order they are offered.
///
/// [`Theme::named_accent`] resolves each of them, so a name offered here
/// always has a colour.
pub const ACCENT_NAMES: &[&str] = &[
    "blue", "purple", "pink", "red", "orange", "yellow", "green", "mint", "teal", "cyan", "indigo",
    "brown", "gray",
];

/// Application color theme based on Otto's design system
#[derive(Debug, Clone)]
pub struct Theme {
    // Accent colors
    /// The user's accent, from `org.freedesktop.appearance accent-color`.
    /// Blue until the portal answers — and blue is also Otto's default, so a
    /// missing portal looks like the default rather than like a failure.
    pub accent: Color,
    pub accent_gray: Color,
    /// The palette's red — for what stands apart from the accent rather than
    /// with it: a count that must not read as another selection, a delete that
    /// must not read as an ordinary button.
    pub accent_red: Color,
    /// The palette's yellow — for what is waiting on someone: a session
    /// blocked on an answer, a thing that needs a look but is not an error.
    pub accent_yellow: Color,
    /// The rest of the named accents — the colours a user can pick as their
    /// accent, by the names in [`ACCENT_NAMES`]. See [`Theme::named_accent`].
    pub accent_blue: Color,
    pub accent_orange: Color,
    pub accent_green: Color,
    pub accent_mint: Color,
    pub accent_teal: Color,
    pub accent_cyan: Color,
    pub accent_indigo: Color,
    pub accent_purple: Color,
    pub accent_pink: Color,
    pub accent_brown: Color,

    // Fill colors (backgrounds)
    pub fill_primary: Color,
    pub fill_secondary: Color,
    pub fill_tertiary: Color,
    pub fill_quaternary: Color,

    // Text colors
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,

    // Material colors (surfaces)
    pub material_titlebar: Color,
    pub material_sidebar: Color,
    pub material_medium: Color,
    /// Menus and popups. More opaque than `material_medium`: they float over
    /// arbitrary content and their text has to stay readable against it.
    pub material_popup: Color,
    pub material_highlight: Color,
    pub material_selection_focused: Color,
    /// The compositor's lighter frost: the app switcher's panel, the
    /// volume/brightness OSD.
    pub material_thin: Color,
    /// The compositor's heaviest frost: workspace previews in the selector.
    pub material_ultrathick: Color,
    /// A dock label balloon, floating over whatever the desktop shows.
    pub material_tooltip: Color,

    // Shadow
    pub shadow: Color,

    /// The hairline around a floating surface. See [`Theme::hairline`].
    pub hairline: Color,
}

impl Theme {
    /// The hairline that separates a floating surface — a menu, a window, a
    /// panel — from whatever it happens to sit over.
    ///
    /// One token for all of them, so the frame around a menu and the frame
    /// around the window it opened from are the same line — and the same one
    /// the compositor's own chrome draws, which is why it is a token of its
    /// own rather than `fill_secondary`: the two schemes need different
    /// alphas to land at the same weight — 10% black carries about as far on
    /// a bright backdrop as 8% white does on a dark one — and a window edged
    /// differently from the dock beside it reads as two lines, not one.
    ///
    /// Deliberately faint either way: the edge is there to close the shape,
    /// not to outline it, and anything stronger rings the surface once it
    /// lands on busy content.
    pub fn hairline(&self) -> Color {
        self.hairline
    }

    /// Width of that hairline.
    ///
    /// One logical point, which is what a hairline is meant to be. A
    /// surface-style border is scaled by the compositor like the corner
    /// radius, so this is the same line whatever the output scale — asking
    /// for one device pixel instead left HiDPI windows edged half as thickly
    /// as the dock beside them. Client-side draws (a panel painting its own
    /// body) are in the same units as the rest of their canvas.
    pub const HAIRLINE_WIDTH: f32 = 1.0;

    /// Light theme, with the user's accent folded in.
    pub fn light() -> Self {
        Self::light_palette()
            .with_system_accent()
            .with_system_backdrop(false)
    }

    /// Dark theme, with the user's accent folded in.
    pub fn dark() -> Self {
        Self::dark_palette()
            .with_system_accent()
            .with_system_backdrop(true)
    }

    /// Fill the materials in where nothing blurs behind them.
    ///
    /// A translucent material over bare pixels is not a lighter version of
    /// itself: it is whatever sits behind the surface showing through, and the
    /// text on top loses its contrast to it. See [`crate::backdrop`].
    fn with_system_backdrop(mut self, dark: bool) -> Self {
        if !crate::backdrop::blur_available() {
            self.with_solid_materials(dark);
        }
        self
    }

    /// Replace the translucent materials with opaque ones.
    ///
    /// Not the same hues made opaque: the light materials are near-white, and
    /// filled in they would be indistinguishable from the content they frame.
    /// Each lands a shade off the content ground instead, so a sidebar still
    /// reads as a sidebar and a menu still reads as lifted off the window.
    /// Highlights and selections are tints laid over these, and stay as they
    /// are.
    pub fn with_solid_materials(&mut self, dark: bool) -> &mut Self {
        if dark {
            self.material_titlebar = Color::from_rgb(0x2A, 0x2A, 0x2C);
            self.material_sidebar = Color::from_rgb(0x25, 0x25, 0x27);
            self.material_medium = Color::from_rgb(0x2A, 0x2A, 0x2C);
            self.material_popup = Color::from_rgb(0x2E, 0x2E, 0x30);
        } else {
            self.material_titlebar = Color::from_rgb(0xE8, 0xE8, 0xEB);
            self.material_sidebar = Color::from_rgb(0xEC, 0xEC, 0xEF);
            self.material_medium = Color::from_rgb(0xF0, 0xF0, 0xF3);
            self.material_popup = Color::from_rgb(0xF8, 0xF8, 0xF9);
        }
        self
    }

    /// The material for a card on a subsurface of the window it floats over —
    /// a command palette, a preview panel.
    ///
    /// `material_popup` where the compositor can frost what is under the card,
    /// and its solid form everywhere else: see
    /// [`crate::backdrop::blurs_behind_subsurfaces`].
    pub fn card_material(&self) -> Color {
        if crate::backdrop::blurs_behind_subsurfaces() {
            return self.material_popup;
        }
        let popup = self.material_popup;
        let dark = popup.r() < 0x80;
        let mut theme = if dark {
            Self::dark_palette()
        } else {
            Self::light_palette()
        };
        theme.with_solid_materials(dark).material_popup
    }

    /// The light palette exactly as designed, with Otto's own blue as the
    /// accent. Callers wanting the user's choice want `light`.
    pub fn light_palette() -> Self {
        Self {
            accent: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93),
            accent_red: Color::from_argb(0xFF, 0xFF, 0x3B, 0x30),
            accent_yellow: Color::from_argb(0xFF, 0xFF, 0xCC, 0x00),
            accent_blue: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_orange: Color::from_argb(0xFF, 0xFF, 0x95, 0x00),
            accent_green: Color::from_argb(0xFF, 0x28, 0xCD, 0x41),
            accent_mint: Color::from_argb(0xFF, 0x00, 0xC7, 0xBE),
            accent_teal: Color::from_argb(0xFF, 0x59, 0xAD, 0xC4),
            accent_cyan: Color::from_argb(0xFF, 0x55, 0xBE, 0xF0),
            accent_indigo: Color::from_argb(0xFF, 0x58, 0x56, 0xD6),
            accent_purple: Color::from_argb(0xFF, 0xAF, 0x52, 0xDE),
            accent_pink: Color::from_argb(0xFF, 0xFF, 0x2D, 0x55),
            accent_brown: Color::from_argb(0xFF, 0xA2, 0x84, 0x5E),

            fill_primary: Color::from_argb(0x35, 0x00, 0x00, 0x00),
            fill_secondary: Color::from_argb(0x14, 0x00, 0x00, 0x00),
            fill_tertiary: Color::from_argb(0x0D, 0x00, 0x00, 0x00),
            fill_quaternary: Color::from_argb(0x08, 0x00, 0x00, 0x00),

            text_primary: Color::from_argb(0xD9, 0x00, 0x00, 0x00),
            text_secondary: Color::from_argb(0x80, 0x00, 0x00, 0x00),
            text_tertiary: Color::from_argb(0x40, 0x00, 0x00, 0x00),

            material_titlebar: Color::from_argb(0xE4, 0xEA, 0xEA, 0xEA),
            // Sidebars sit over compositor blur, but they are a window's own
            // ground and carry small text: at 0x8C the backdrop read straight
            // through and the wallpaper competed with the rows. Keep enough
            // tint that the blur reads as frost, not as a see-through hole.
            material_sidebar: Color::from_argb(0xDE, 0xF2, 0xF2, 0xF2),
            material_medium: Color::from_argb(0x7A, 0xF6, 0xF6, 0xF6),
            material_popup: Color::from_argb(0xD8, 0xF6, 0xF6, 0xF6),
            material_highlight: Color::from_argb(0x9E, 0xF7, 0xF7, 0xF7),
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF),
            material_thin: Color::from_argb(0x5B, 0xF6, 0xF6, 0xF6),
            material_ultrathick: Color::from_argb(0xD6, 0xF6, 0xF6, 0xF6),
            // A dock label is dark text on this over whatever the desktop
            // shows: the same near-white frost as the other light materials,
            // a shade more opaque for a balloon this small.
            material_tooltip: Color::from_argb(0xE6, 0xF6, 0xF6, 0xF6),

            shadow: Color::from_argb(0x66, 0x1B, 0x1B, 0x1B),

            hairline: Color::from_argb(0x1A, 0x00, 0x00, 0x00),
        }
    }

    /// The dark palette exactly as designed. See `light_palette`.
    pub fn dark_palette() -> Self {
        Self {
            accent: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93),
            accent_red: Color::from_argb(0xFF, 0xFF, 0x45, 0x3A),
            accent_yellow: Color::from_argb(0xFF, 0xFF, 0xD6, 0x0A),
            accent_blue: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_orange: Color::from_argb(0xFF, 0xFF, 0x9F, 0x0A),
            accent_green: Color::from_argb(0xFF, 0x32, 0xD7, 0x4B),
            accent_mint: Color::from_argb(0xFF, 0x66, 0xD4, 0xCF),
            accent_teal: Color::from_argb(0xFF, 0x6A, 0xC4, 0xDC),
            accent_cyan: Color::from_argb(0xFF, 0x5A, 0xC8, 0xF5),
            accent_indigo: Color::from_argb(0xFF, 0x5E, 0x5C, 0xE6),
            accent_purple: Color::from_argb(0xFF, 0xBF, 0x5A, 0xF2),
            accent_pink: Color::from_argb(0xFF, 0xFF, 0x37, 0x5F),
            accent_brown: Color::from_argb(0xFF, 0xAC, 0x8E, 0x68),

            // Semi-transparent whites for layering on dark backgrounds
            fill_primary: Color::from_argb(0x40, 0xFF, 0xFF, 0xFF),
            fill_secondary: Color::from_argb(0x1A, 0xFF, 0xFF, 0xFF),
            fill_tertiary: Color::from_argb(0x0F, 0xFF, 0xFF, 0xFF),
            fill_quaternary: Color::from_argb(0x08, 0xFF, 0xFF, 0xFF),

            text_primary: Color::from_argb(0xF2, 0xFF, 0xFF, 0xFF),
            text_secondary: Color::from_argb(0x80, 0xFF, 0xFF, 0xFF),
            text_tertiary: Color::from_argb(0x40, 0xFF, 0xFF, 0xFF),

            // Dark translucent surfaces
            material_titlebar: Color::from_argb(0xE6, 0x28, 0x28, 0x28),
            material_sidebar: Color::from_argb(0xF0, 0x1E, 0x1E, 0x1E),
            material_medium: Color::from_argb(0x83, 0x28, 0x28, 0x28),
            material_popup: Color::from_argb(0xD8, 0x28, 0x28, 0x28),
            material_highlight: Color::from_argb(0xA2, 0x69, 0x67, 0x67),
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF),
            material_thin: Color::from_argb(0x55, 0x38, 0x38, 0x38),
            material_ultrathick: Color::from_argb(0xBE, 0x47, 0x47, 0x47),
            material_tooltip: Color::from_argb(0xC1, 0x4D, 0x4C, 0x4C),

            shadow: Color::from_argb(0x99, 0x00, 0x00, 0x00),

            hairline: Color::from_argb(0x14, 0xFF, 0xFF, 0xFF),
        }
    }

    /// The palette's colour for an accent name from [`ACCENT_NAMES`].
    ///
    /// Always the palette's own value: `blue` is `accent_blue`, not `accent`,
    /// which is whatever the user picked.
    pub fn named_accent(&self, name: &str) -> Option<Color> {
        Some(match name {
            "red" => self.accent_red,
            "orange" => self.accent_orange,
            "yellow" => self.accent_yellow,
            "green" => self.accent_green,
            "mint" => self.accent_mint,
            "teal" => self.accent_teal,
            "cyan" => self.accent_cyan,
            "blue" => self.accent_blue,
            "indigo" => self.accent_indigo,
            "purple" => self.accent_purple,
            "pink" => self.accent_pink,
            "gray" => self.accent_gray,
            "brown" => self.accent_brown,
            _ => return None,
        })
    }

    /// Whether this is a dark palette.
    ///
    /// Read off the popup material rather than stored: a `Theme` is passed
    /// around and re-tinted by value, and a flag beside the colours is one
    /// more thing that can disagree with them.
    pub fn is_dark(&self) -> bool {
        self.material_popup.r() < 0x80
    }

    /// One of the tinted frosted materials, in this theme's scheme. See
    /// [`crate::frosted`].
    pub fn frosted(&self, frosted: Frosted) -> Color {
        frosted.material(self.is_dark())
    }

    /// The frosted red material.
    pub fn frosted_red(&self) -> Color {
        self.frosted(Frosted::Red)
    }

    /// The frosted orange material.
    pub fn frosted_orange(&self) -> Color {
        self.frosted(Frosted::Orange)
    }

    /// The frosted amber material.
    pub fn frosted_amber(&self) -> Color {
        self.frosted(Frosted::Amber)
    }

    /// The frosted yellow material.
    pub fn frosted_yellow(&self) -> Color {
        self.frosted(Frosted::Yellow)
    }

    /// The frosted lime material.
    pub fn frosted_lime(&self) -> Color {
        self.frosted(Frosted::Lime)
    }

    /// The frosted green material.
    pub fn frosted_green(&self) -> Color {
        self.frosted(Frosted::Green)
    }

    /// The frosted teal material.
    pub fn frosted_teal(&self) -> Color {
        self.frosted(Frosted::Teal)
    }

    /// The frosted cyan material.
    pub fn frosted_cyan(&self) -> Color {
        self.frosted(Frosted::Cyan)
    }

    /// The frosted blue material.
    pub fn frosted_blue(&self) -> Color {
        self.frosted(Frosted::Blue)
    }

    /// The frosted indigo material.
    pub fn frosted_indigo(&self) -> Color {
        self.frosted(Frosted::Indigo)
    }

    /// The frosted violet material.
    pub fn frosted_violet(&self) -> Color {
        self.frosted(Frosted::Violet)
    }

    /// The frosted magenta material.
    pub fn frosted_magenta(&self) -> Color {
        self.frosted(Frosted::Magenta)
    }

    /// Return the appropriate theme for the given color scheme.
    /// Falls back to light for `NoPreference`.
    pub fn for_scheme(scheme: ColorScheme) -> Self {
        match scheme {
            ColorScheme::Dark => Self::dark(),
            _ => Self::light(),
        }
    }

    /// Apply the accent the portal reported, if it reported one.
    ///
    /// This lives in `light`/`dark` rather than only in `for_scheme` because
    /// plenty of call sites pick a palette directly from a `dark` flag they
    /// already have; folding it in higher up left those windows blue while
    /// everything around them followed the user.
    fn with_system_accent(mut self) -> Self {
        if let Some(accent) = crate::accent::current_accent() {
            self.with_accent(accent);
        }
        self
    }

    /// Re-tint everything that follows the accent.
    ///
    /// The focused-selection material is the accent at its own alpha, not a
    /// colour of its own — leaving it blue is what made a re-tinted list row
    /// clash with the toggle right beside it.
    pub fn with_accent(&mut self, accent: Color) -> &mut Self {
        let alpha = self.material_selection_focused.a();
        self.accent = accent;
        self.material_selection_focused =
            Color::from_argb(alpha, accent.r(), accent.g(), accent.b());
        self
    }

    /// Drain most of the colour out of everything that follows the accent.
    ///
    /// For a window that is not the focused one. A background window still has
    /// to show what is selected in it, but saying so in the user's accent puts
    /// every open window in the same voice and leaves the eye nothing to pick
    /// the front one out by. macOS answers this by turning the selection grey;
    /// this stops just short of grey, so the window still reads as part of the
    /// user's desktop while the focused one owns the colour.
    pub fn with_muted_accent(&mut self) -> &mut Self {
        self.accent = mute(self.accent);
        self.material_selection_focused = mute(self.material_selection_focused);
        self
    }
}

/// How far a muted accent travels towards its own grey. Not all the way: a
/// trace of the hue is what keeps a background window looking like the same
/// desktop rather than like a screenshot of a different one.
const MUTE: f32 = 0.85;

/// Mix a colour towards its own luminance, keeping its alpha — the alpha is
/// what makes a selection material a material, and greying is a hue change,
/// not a transparency one.
fn mute(color: Color) -> Color {
    let luma = 0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32;
    let mix = |channel: u8| (channel as f32 * (1.0 - MUTE) + luma * MUTE).clamp(0.0, 255.0) as u8;
    Color::from_argb(color.a(), mix(color.r()), mix(color.g()), mix(color.b()))
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_offered_accent_name_resolves() {
        for theme in [Theme::light_palette(), Theme::dark_palette()] {
            for name in ACCENT_NAMES {
                assert!(theme.named_accent(name).is_some(), "`{name}` has no colour");
            }
        }
    }

    #[test]
    fn solid_materials_are_opaque_and_off_the_content_ground() {
        for (dark, mut theme, ground) in [
            (false, Theme::light_palette(), Color::WHITE),
            (
                true,
                Theme::dark_palette(),
                Color::from_rgb(0x1C, 0x1C, 0x1E),
            ),
        ] {
            theme.with_solid_materials(dark);
            for material in [
                theme.material_titlebar,
                theme.material_sidebar,
                theme.material_medium,
                theme.material_popup,
            ] {
                assert_eq!(material.a(), 0xFF);
                assert_ne!(material, ground);
            }
        }
    }
}
