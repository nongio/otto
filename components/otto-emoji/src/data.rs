//! The emoji table, baked in from `data/emoji.txt`.
//!
//! The file is Unicode's `emoji-test.txt` slimmed down by `data/regen.py`:
//! fully-qualified sequences only, in the CLDR order the file recommends for
//! palettes, with each emoji's skin-tone variants folded onto it. A picker
//! shows an emoji once and lets the tone be a setting; listing the five tones
//! beside every hand and face would bury the rest.

use std::fmt;

/// The categories, in palette order. The table's `group` is an index into
/// this. Unicode's "Component" group (the tone swatches and hair styles) is
/// not here: those are modifiers, and they are applied rather than typed.
pub const GROUPS: [Group; 9] = [
    Group::new("Smileys & Emotion", "emoji-group-smileys", "1F600"),
    Group::new("People & Body", "emoji-group-people", "1F44B"),
    Group::new("Animals & Nature", "emoji-group-nature", "1F43B"),
    Group::new("Food & Drink", "emoji-group-food", "1F354"),
    Group::new("Travel & Places", "emoji-group-travel", "1F697"),
    Group::new("Activities", "emoji-group-activities", "26BD"),
    Group::new("Objects", "emoji-group-objects", "1F4A1"),
    Group::new("Symbols", "emoji-group-symbols", "1F523"),
    Group::new("Flags", "emoji-group-flags", "1F3C1"),
];

/// One category of the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// The name as `emoji-test.txt` spells it, which is how the data file
    /// refers to it.
    pub source_name: &'static str,
    /// The localisation key for the name shown to the user.
    pub key: &'static str,
    /// Codepoints of the emoji that stands for the group on its tab.
    icon: &'static str,
}

impl Group {
    const fn new(source_name: &'static str, key: &'static str, icon: &'static str) -> Self {
        Self {
            source_name,
            key,
            icon,
        }
    }

    /// The emoji drawn on the group's tab.
    pub fn icon(&self) -> String {
        decode(self.icon)
    }
}

/// A skin tone, as the modifier codepoint order has them. `None` is the
/// unmodified yellow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    None,
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

impl Tone {
    pub const ALL: [Tone; 6] = [
        Tone::None,
        Tone::Light,
        Tone::MediumLight,
        Tone::Medium,
        Tone::MediumDark,
        Tone::Dark,
    ];

    /// Index into an emoji's `tones`, or `None` for the base.
    fn variant(self) -> Option<usize> {
        match self {
            Tone::None => None,
            Tone::Light => Some(0),
            Tone::MediumLight => Some(1),
            Tone::Medium => Some(2),
            Tone::MediumDark => Some(3),
            Tone::Dark => Some(4),
        }
    }

    /// The tone's own modifier character, which is how the setting is saved.
    pub fn modifier(self) -> Option<char> {
        self.variant()
            .and_then(|index| char::from_u32(0x1F3FB + index as u32))
    }

    pub fn from_modifier(text: &str) -> Tone {
        match text.trim().chars().next().map(|c| c as u32) {
            Some(0x1F3FB) => Tone::Light,
            Some(0x1F3FC) => Tone::MediumLight,
            Some(0x1F3FD) => Tone::Medium,
            Some(0x1F3FE) => Tone::MediumDark,
            Some(0x1F3FF) => Tone::Dark,
            _ => Tone::None,
        }
    }

    /// A flat colour that stands for the tone in a swatch. The base is the
    /// yellow the emoji fonts agree on.
    pub fn swatch(self) -> (u8, u8, u8) {
        match self {
            Tone::None => (0xFF, 0xC8, 0x3D),
            Tone::Light => (0xF7, 0xDE, 0xCE),
            Tone::MediumLight => (0xF3, 0xD2, 0xA2),
            Tone::Medium => (0xD5, 0xAB, 0x88),
            Tone::MediumDark => (0xAF, 0x7E, 0x57),
            Tone::Dark => (0x7C, 0x53, 0x3E),
        }
    }
}

/// One emoji, with its tone variants.
#[derive(Debug, Clone)]
pub struct Emoji {
    /// Index into [`GROUPS`].
    pub group: usize,
    /// Unicode's finer grouping — `face-smiling`, `hand-fingers-open` — which
    /// is searchable, because it often names what the name does not.
    pub subgroup: &'static str,
    /// The character sequence, fully qualified.
    pub text: String,
    /// Unicode's short name, lower case: `grinning face`.
    pub name: &'static str,
    /// The five tone variants, light to dark, for emoji that have them.
    tones: Option<[String; 5]>,
}

impl Emoji {
    /// The text for `tone`, or the base when the emoji has no tones.
    pub fn with_tone(&self, tone: Tone) -> &str {
        match (tone.variant(), self.tones.as_ref()) {
            (Some(index), Some(tones)) => tones[index].as_str(),
            _ => self.text.as_str(),
        }
    }

    pub fn has_tones(&self) -> bool {
        self.tones.is_some()
    }

    /// The first codepoint, which is what a font is asked about to know
    /// whether it can draw the emoji at all.
    pub fn first_codepoint(&self) -> u32 {
        self.text.chars().next().map(|c| c as u32).unwrap_or(0)
    }
}

impl fmt::Display for Emoji {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// `1F44B-1F3FB` to the characters it names.
fn decode(codepoints: &str) -> String {
    codepoints
        .split('-')
        .filter_map(|hex| u32::from_str_radix(hex, 16).ok())
        .filter_map(char::from_u32)
        .collect()
}

const DATA: &str = include_str!("../data/emoji.txt");

/// Every emoji, in palette order.
pub struct Table {
    pub emoji: Vec<Emoji>,
}

impl Table {
    /// Parse the baked-in data. Milliseconds: the file is a couple of
    /// thousand short lines.
    pub fn load() -> Self {
        Self::parse(DATA)
    }

    fn parse(text: &'static str) -> Self {
        let emoji = text
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let group_name = fields.next()?;
                let subgroup = fields.next()?;
                let codepoints = fields.next()?;
                let name = fields.next()?;
                let tones = fields.next().unwrap_or("");
                let group = GROUPS
                    .iter()
                    .position(|group| group.source_name == group_name)?;
                let tones: Option<[String; 5]> = {
                    let variants: Vec<String> = tones.split(' ').map(decode).collect();
                    (variants.len() == 5 && variants.iter().all(|v| !v.is_empty()))
                        .then(|| variants.try_into().ok())
                        .flatten()
                };
                Some(Emoji {
                    group,
                    subgroup,
                    text: decode(codepoints),
                    name,
                    tones,
                })
            })
            .collect();
        Self { emoji }
    }

    /// Drop every emoji `can_draw` says no to, so a font older than the data
    /// leaves gaps in the palette rather than boxes.
    pub fn retain_drawable(&mut self, mut can_draw: impl FnMut(&Emoji) -> bool) {
        self.emoji.retain(|emoji| can_draw(emoji));
    }

    /// Indices of the emoji in `group`, in order.
    pub fn in_group(&self, group: usize) -> impl Iterator<Item = usize> + '_ {
        self.emoji
            .iter()
            .enumerate()
            .filter(move |(_, emoji)| emoji.group == group)
            .map(|(index, _)| index)
    }

    /// The emoji whose text — base or any tone — is `text`.
    pub fn find(&self, text: &str) -> Option<usize> {
        self.emoji.iter().position(|emoji| {
            emoji.text == text
                || emoji
                    .tones
                    .as_ref()
                    .is_some_and(|tones| tones.iter().any(|tone| tone == text))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_covers_every_group_in_order() {
        let table = Table::load();
        assert!(table.emoji.len() > 1800, "{} emoji", table.emoji.len());
        let first_of_each: Vec<usize> = GROUPS
            .iter()
            .enumerate()
            .map(|(group, _)| table.in_group(group).next().expect("a non-empty group"))
            .collect();
        assert!(first_of_each.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(table.emoji[0].name, "grinning face");
        assert_eq!(table.emoji[0].text, "😀");
    }

    #[test]
    fn tones_apply_to_what_has_them() {
        let table = Table::load();
        let wave = table.find("👋").expect("waving hand");
        let wave = &table.emoji[wave];
        assert!(wave.has_tones());
        assert_eq!(wave.with_tone(Tone::None), "👋");
        assert_eq!(wave.with_tone(Tone::Medium), "👋🏽");
        assert_eq!(wave.with_tone(Tone::MediumLight), "👋🏼");

        let heart = &table.emoji[table.find("❤️").expect("red heart")];
        assert!(!heart.has_tones());
        assert_eq!(heart.with_tone(Tone::Dark), "❤️");
    }

    #[test]
    fn a_toned_variant_finds_its_base() {
        let table = Table::load();
        assert_eq!(table.find("👋🏿"), table.find("👋"));
    }

    #[test]
    fn tones_round_trip_through_their_modifier() {
        for tone in Tone::ALL {
            let saved = tone.modifier().map(String::from).unwrap_or_default();
            assert_eq!(Tone::from_modifier(&saved), tone);
        }
    }
}
