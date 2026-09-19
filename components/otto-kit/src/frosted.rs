//! The tinted frosted materials.
//!
//! A surface that belongs to something — an agent's panel, its launcher, the
//! island it answers in — wears a material of the same family as the rest of
//! the desktop's chrome, only carrying a hue. The family is what says "these
//! belong together" without a badge or a label on any of them; the hue is what
//! says which family.
//!
//! Each material is a grey with the chroma turned up, half opaque. The grey is
//! what keeps it a material: the lightness is the scheme's, so a frosted red
//! panel sits at the same weight as the popup beside it and the same text
//! colours read on both. Only the hue changes between them, and between the
//! two schemes only the grey does.
//!
//! The light materials are far more saturated than the dark ones, which looks
//! wrong in a table and right on screen: how far a colour can travel from grey
//! is bounded by its own lightness — `1 - |2L - 1|` — and a near-white
//! material has almost no room, so it asks for nearly all the chroma there is
//! and still lands a pastel.
//!
//! They are also levelled by what the eye reads as lightness rather than by
//! where they sit between black and white. The two are not the same: a blue
//! and a yellow written at the same midpoint of their channels are not seen as
//! the same brightness, and a light orange built that way comes out looking
//! tanned next to a light green. The light materials are the colour of that
//! hue whose *relative luminance* is 0.86, so the row reads as one weight of
//! surface all the way across.
//!
//! The blur is the compositor's, and so is the tint: a surface hands its
//! material to `otto_surface_style_v1` as a background colour and paints its
//! content over the result. See [`crate::backdrop`].

use skia_safe::Color;

/// How opaque a frosted material is: half. Enough for the hue to be the
/// surface's own rather than the wallpaper's, little enough that the desktop
/// still moves behind it.
const ALPHA: u8 = 0x80;

/// What the light materials are levelled at: the relative luminance every one
/// of them carries, in the sense the contrast standards use.
///
/// High, because these are surfaces for dark text to sit on. Chroma is capped
/// at 0.85 of the most a hue could carry at this luminance — the yellows and
/// limes reach it long before the blues do, and left uncapped they turn to
/// neon while the rest stay pastel. The tables below are written out at those
/// values; this is what the test levels them against.
#[cfg(test)]
const LIGHT_LUMINANCE: f32 = 0.86;

/// The named frosted materials, twelve hues round the wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frosted {
    Red,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Teal,
    Cyan,
    Blue,
    Indigo,
    Violet,
    Magenta,
}

impl Frosted {
    /// Every material, in wheel order.
    pub const ALL: [Frosted; 12] = [
        Frosted::Red,
        Frosted::Orange,
        Frosted::Amber,
        Frosted::Yellow,
        Frosted::Lime,
        Frosted::Green,
        Frosted::Teal,
        Frosted::Cyan,
        Frosted::Blue,
        Frosted::Indigo,
        Frosted::Violet,
        Frosted::Magenta,
    ];

    /// The material called `name` in configuration: its variant in lower
    /// case, such as `"teal"`. `None` for anything else. The names are the
    /// ones `agents.toml` takes for an agent's `colour`, and otto-agents keeps
    /// the same list.
    pub fn from_name(name: &str) -> Option<Frosted> {
        Frosted::ALL
            .into_iter()
            .find(|frosted| frosted.name() == name)
    }

    /// The material's name in configuration, the inverse of [`Frosted::from_name`].
    pub const fn name(self) -> &'static str {
        match self {
            Frosted::Red => "red",
            Frosted::Orange => "orange",
            Frosted::Amber => "amber",
            Frosted::Yellow => "yellow",
            Frosted::Lime => "lime",
            Frosted::Green => "green",
            Frosted::Teal => "teal",
            Frosted::Cyan => "cyan",
            Frosted::Blue => "blue",
            Frosted::Indigo => "indigo",
            Frosted::Violet => "violet",
            Frosted::Magenta => "magenta",
        }
    }

    /// The material for `dark`'s scheme, as a frosted surface should paint it.
    ///
    /// Taken up to at least [`crate::frosting::UNFROSTED_MIN_ALPHA`] while the
    /// desktop's frosting is off: with nothing blurred behind it a half-opaque
    /// panel is not a lighter version of itself, it is whatever happens to be
    /// under it showing through.
    pub fn material(self, dark: bool) -> Color {
        crate::frosting::material(self.colour(dark))
    }

    /// The material as designed, frosting or no frosting.
    pub fn colour(self, dark: bool) -> Color {
        let (r, g, b) = if dark {
            self.dark_rgb()
        } else {
            self.light_rgb()
        };
        Color::from_argb(ALPHA, r, g, b)
    }

    /// The dark scheme: grey `#5C` with a third of the chroma it can carry.
    const fn dark_rgb(self) -> (u8, u8, u8) {
        match self {
            Frosted::Red => (0x78, 0x43, 0x40),
            Frosted::Orange => (0x78, 0x54, 0x40),
            Frosted::Amber => (0x78, 0x62, 0x40),
            Frosted::Yellow => (0x78, 0x6E, 0x40),
            Frosted::Lime => (0x66, 0x78, 0x40),
            Frosted::Green => (0x40, 0x78, 0x49),
            Frosted::Teal => (0x40, 0x78, 0x72),
            Frosted::Cyan => (0x40, 0x6F, 0x78),
            Frosted::Blue => (0x40, 0x5C, 0x78),
            Frosted::Indigo => (0x41, 0x40, 0x78),
            Frosted::Violet => (0x54, 0x40, 0x78),
            Frosted::Magenta => (0x78, 0x40, 0x4B),
        }
    }

    /// The light scheme: each hue at a relative luminance of 0.86, at 0.85 of
    /// the chroma it can carry there.
    const fn light_rgb(self) -> (u8, u8, u8) {
        match self {
            Frosted::Red => (0xFD, 0xEB, 0xE9),
            Frosted::Orange => (0xFC, 0xEC, 0xDE),
            Frosted::Amber => (0xFB, 0xEE, 0xCF),
            Frosted::Yellow => (0xF8, 0xF2, 0xA3),
            Frosted::Lime => (0xDF, 0xF8, 0xAC),
            Frosted::Green => (0xC9, 0xFB, 0xD5),
            Frosted::Teal => (0xC1, 0xFA, 0xF0),
            Frosted::Cyan => (0xD7, 0xF4, 0xFC),
            Frosted::Blue => (0xE5, 0xF0, 0xFD),
            Frosted::Indigo => (0xEE, 0xED, 0xFE),
            Frosted::Violet => (0xF2, 0xEC, 0xFD),
            Frosted::Magenta => (0xFD, 0xEA, 0xF0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance, as the contrast standards define it.
    fn luminance(c: Color) -> f32 {
        let channel = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    /// A dark material's lightness belongs to its scheme, not to its hue: the
    /// whole point is that a frosted panel sits at the same weight as the
    /// untinted one beside it, whichever hue it wears.
    #[test]
    fn the_dark_materials_share_a_lightness() {
        for frosted in Frosted::ALL {
            let c = frosted.colour(true);
            let (max, min) = (
                c.r().max(c.g()).max(c.b()) as i32,
                c.r().min(c.g()).min(c.b()) as i32,
            );
            let found = (max + min) / 2;
            assert!(
                (found - 0x5C).abs() <= 1,
                "{frosted:?} sits at {found:#04X}, not 0x5C",
            );
        }
    }

    /// The light materials are levelled by eye instead — see the module docs.
    /// Text is read on these, and a row of surfaces the eye sees as different
    /// brightnesses is a row of different materials.
    #[test]
    fn the_light_materials_share_a_luminance() {
        for frosted in Frosted::ALL {
            let found = luminance(frosted.colour(false));
            assert!(
                (found - LIGHT_LUMINANCE).abs() <= 0.01,
                "{frosted:?} reads at {found:.3}, not {LIGHT_LUMINANCE:.2}",
            );
        }
    }

    /// Twelve hues means twelve *different* hues — a duplicate would leave two
    /// families indistinguishable.
    #[test]
    fn the_materials_are_all_different() {
        for dark in [true, false] {
            let mut seen = Vec::new();
            for frosted in Frosted::ALL {
                let colour = frosted.colour(dark);
                assert!(!seen.contains(&colour), "{frosted:?} repeats a material");
                seen.push(colour);
            }
        }
    }

    /// A name round-trips, and nothing else names a material.
    #[test]
    fn names_round_trip() {
        for frosted in Frosted::ALL {
            assert_eq!(Frosted::from_name(frosted.name()), Some(frosted));
        }
        assert_eq!(Frosted::from_name("Teal"), None, "names are lower case");
        assert_eq!(Frosted::from_name("grey"), None);
    }

    /// otto-agents validates an agent's `colour` against its own copy of the
    /// names, since it cannot depend on this crate: the two lists must agree,
    /// or a colour the config accepts would arrive here as no material.
    #[test]
    fn the_names_are_the_ones_agents_accepts() {
        let source = include_str!("../../otto-agents/src/config.rs");
        let start = source
            .find("pub const NAMES: [&str; 12] = [")
            .expect("otto-agents' Colour::NAMES");
        let list = &source[start..source[start..].find("];").expect("the list ends") + start];
        let names: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
        let ours: Vec<&str> = Frosted::ALL.into_iter().map(Frosted::name).collect();
        assert_eq!(names, ours);
    }

    /// Every material carries chroma. A grey one would take the compositor's
    /// neutral blur path and come back the colour of the wallpaper.
    #[test]
    fn every_material_carries_chroma() {
        for dark in [true, false] {
            for frosted in Frosted::ALL {
                let c = frosted.colour(dark);
                let chroma = c.r().max(c.g()).max(c.b()) - c.r().min(c.g()).min(c.b());
                assert!(chroma > 0x10, "{frosted:?} is too close to grey");
            }
        }
    }
}
