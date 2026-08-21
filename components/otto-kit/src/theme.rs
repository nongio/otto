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

/// Application color theme based on Otto's design system
#[derive(Debug, Clone)]
pub struct Theme {
    // Accent colors
    /// The user's accent, from `org.freedesktop.appearance accent-color`.
    /// Blue until the portal answers — and blue is also Otto's default, so a
    /// missing portal looks like the default rather than like a failure.
    pub accent: Color,
    pub accent_gray: Color,

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

    // Shadow
    pub shadow: Color,
}

impl Theme {
    /// Light theme, with the user's accent folded in.
    pub fn light() -> Self {
        Self::light_palette().with_system_accent()
    }

    /// Dark theme, with the user's accent folded in.
    pub fn dark() -> Self {
        Self::dark_palette().with_system_accent()
    }

    /// The light palette exactly as designed, with Otto's own blue as the
    /// accent. Callers wanting the user's choice want `light`.
    pub fn light_palette() -> Self {
        Self {
            accent: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93),

            fill_primary: Color::from_argb(0x35, 0x00, 0x00, 0x00),
            fill_secondary: Color::from_argb(0x14, 0x00, 0x00, 0x00),
            fill_tertiary: Color::from_argb(0x0D, 0x00, 0x00, 0x00),
            fill_quaternary: Color::from_argb(0x08, 0x00, 0x00, 0x00),

            text_primary: Color::from_argb(0xD9, 0x00, 0x00, 0x00),
            text_secondary: Color::from_argb(0x80, 0x00, 0x00, 0x00),
            text_tertiary: Color::from_argb(0x40, 0x00, 0x00, 0x00),

            material_titlebar: Color::from_argb(0xCC, 0xEA, 0xEA, 0xEA),
            // Sidebars sit over compositor blur, but they are a window's own
            // ground and carry small text: at 0x8C the backdrop read straight
            // through and the wallpaper competed with the rows. Keep enough
            // tint that the blur reads as frost, not as a see-through hole.
            material_sidebar: Color::from_argb(0xDE, 0xF2, 0xF2, 0xF2),
            material_medium: Color::from_argb(0x7A, 0xF6, 0xF6, 0xF6),
            material_popup: Color::from_argb(0xD8, 0xF6, 0xF6, 0xF6),
            material_highlight: Color::from_argb(0x9E, 0xF7, 0xF7, 0xF7),
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF),

            shadow: Color::from_argb(0x66, 0x1B, 0x1B, 0x1B),
        }
    }

    /// The dark palette exactly as designed. See `light_palette`.
    pub fn dark_palette() -> Self {
        Self {
            accent: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93),

            // Semi-transparent whites for layering on dark backgrounds
            fill_primary: Color::from_argb(0x40, 0xFF, 0xFF, 0xFF),
            fill_secondary: Color::from_argb(0x1A, 0xFF, 0xFF, 0xFF),
            fill_tertiary: Color::from_argb(0x0F, 0xFF, 0xFF, 0xFF),
            fill_quaternary: Color::from_argb(0x08, 0xFF, 0xFF, 0xFF),

            text_primary: Color::from_argb(0xF2, 0xFF, 0xFF, 0xFF),
            text_secondary: Color::from_argb(0x80, 0xFF, 0xFF, 0xFF),
            text_tertiary: Color::from_argb(0x40, 0xFF, 0xFF, 0xFF),

            // Dark translucent surfaces
            material_titlebar: Color::from_argb(0xBF, 0x28, 0x28, 0x28),
            material_sidebar: Color::from_argb(0xF0, 0x1E, 0x1E, 0x1E),
            material_medium: Color::from_argb(0x83, 0x28, 0x28, 0x28),
            material_popup: Color::from_argb(0xD8, 0x28, 0x28, 0x28),
            material_highlight: Color::from_argb(0xA2, 0x69, 0x67, 0x67),
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF),

            shadow: Color::from_argb(0x99, 0x00, 0x00, 0x00),
        }
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
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}
