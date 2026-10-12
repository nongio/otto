//! Peek's settings: the `[peek]` section of `~/.config/otto/files.toml`.
//!
//! Files and Preview both read it, since both embed the same Peek session and
//! must agree on whether text in pictures is recognised, and by what. The
//! rest of the file is Files' own (the sidebar, the desk) and stays there.

use serde::Deserialize;

/// The `[peek]` section: text recognition in pictures.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PeekConfig {
    /// Whether text in pictures is recognised for selection and Find.
    #[serde(default = "yes")]
    pub recognise_text: bool,
    /// The recogniser to run instead of tesseract: a command line that
    /// reads a PNG on stdin and writes hOCR to stdout, with `{languages}`
    /// where the languages go. Empty means the default.
    #[serde(default)]
    pub recogniser: String,
}

// A missing section means on, the same as a missing key.
impl Default for PeekConfig {
    fn default() -> Self {
        Self {
            recognise_text: true,
            recogniser: String::new(),
        }
    }
}

fn yes() -> bool {
    true
}

/// Read the `[peek]` section of `~/.config/otto/files.toml`, or the
/// defaults when there is none.
pub fn load() -> PeekConfig {
    #[derive(Deserialize, Default)]
    struct Only {
        #[serde(default)]
        peek: PeekConfig,
    }
    let Some(path) = otto_kit::xdg::otto_config_file("files.toml") else {
        return PeekConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return PeekConfig::default();
    };
    toml::from_str::<Only>(&text)
        .map(|file| file.peek)
        .unwrap_or_default()
}
