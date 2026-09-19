//! Localisation of the words this service composes itself.
//!
//! The dialog that asks whether an agent may use a tool is rendered by
//! otto-islands, but its sentence — who wants to do what, and where — is put
//! together here, so it is this service that has to say it in the user's
//! language. The catalogues are the same `resources/locales/*.ftl` files the
//! rest of the desktop reads, keyed under `agents-`; only what this crate
//! needs of otto-kit's loader is repeated, so the service does not pull a GUI
//! toolkit in for a handful of strings.
//!
//! The locale comes from the environment (`LC_ALL`, `LC_MESSAGES`, `LANG`,
//! `LANGUAGE`), resolved most specific first and always ending at `en-GB`,
//! the source catalogue and the only one guaranteed to carry every key.

use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

/// The locale every chain ends at.
const SOURCE_LOCALE: &str = "en-GB";

/// The catalogues, baked in so the service cannot drift from the strings its
/// binary asks for.
const CATALOGUES: &[(&str, &str)] = &[
    (
        "en-GB",
        include_str!("../../../resources/locales/en-GB.ftl"),
    ),
    (
        "en-US",
        include_str!("../../../resources/locales/en-US.ftl"),
    ),
    ("de", include_str!("../../../resources/locales/de.ftl")),
    ("es", include_str!("../../../resources/locales/es.ftl")),
    ("fr", include_str!("../../../resources/locales/fr.ftl")),
    ("it", include_str!("../../../resources/locales/it.ftl")),
    ("ja", include_str!("../../../resources/locales/ja.ftl")),
    ("pl", include_str!("../../../resources/locales/pl.ftl")),
    (
        "pt-BR",
        include_str!("../../../resources/locales/pt-BR.ftl"),
    ),
    ("ru", include_str!("../../../resources/locales/ru.ftl")),
    ("uk", include_str!("../../../resources/locales/uk.ftl")),
    (
        "zh-CN",
        include_str!("../../../resources/locales/zh-CN.ftl"),
    ),
];

static CHAIN: OnceLock<Vec<FluentBundle<FluentResource>>> = OnceLock::new();

/// The message `key` in the user's language, with `args` formatted in. A key
/// no catalogue carries comes back as itself, so a gap is visible rather
/// than fatal.
pub fn t(key: &str, args: Option<&FluentArgs>) -> String {
    for bundle in chain() {
        let Some(pattern) = bundle.get_message(key).and_then(|message| message.value()) else {
            continue;
        };
        let mut errors = Vec::new();
        let formatted = bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            tracing::warn!("localisation error for {key}: {errors:?}");
        }
        return formatted.into_owned();
    }
    tracing::warn!("missing localisation key: {key}");
    key.to_owned()
}

fn chain() -> &'static [FluentBundle<FluentResource>] {
    CHAIN.get_or_init(|| build_chain(&requested_locales()))
}

/// Pins the chain to the source locale, so the English wording a test asserts
/// is what it gets wherever it runs. Integration tests call this before the
/// first string is composed; the unit tests get it from [`requested_locales`].
/// The first initialisation of the chain wins, here as anywhere.
pub fn pin_source_locale() {
    let _ = CHAIN.set(build_chain(&[]));
}

/// The locales the chain is built from. A test build asks for none of them:
/// the tests assert the source catalogue's words, and the environment they
/// run in is not theirs to choose.
fn requested_locales() -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    env_locales()
}

fn build_chain(requested: &[String]) -> Vec<FluentBundle<FluentResource>> {
    let mut wanted: Vec<String> = requested.to_vec();
    if !wanted.iter().any(|tag| tag == SOURCE_LOCALE) {
        wanted.push(SOURCE_LOCALE.to_owned());
    }
    wanted
        .iter()
        .filter_map(|tag| {
            let (_, source) = CATALOGUES
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(tag))?;
            let langid: LanguageIdentifier = tag.parse().ok()?;
            let resource = match FluentResource::try_new((*source).to_owned()) {
                Ok(resource) => resource,
                Err((resource, errors)) => {
                    tracing::error!("locale {tag}: {} parse error(s): {errors:?}", errors.len());
                    resource
                }
            };
            let mut bundle = FluentBundle::new_concurrent(vec![langid]);
            // The isolation marks around placeables render as stray boxes in
            // Otto's renderer, and no RTL locale ships yet.
            bundle.set_use_isolating(false);
            if bundle.add_resource(resource).is_err() {
                tracing::error!("locale {tag}: duplicate message identifiers");
            }
            Some(bundle)
        })
        .collect()
}

/// The locales the environment asks for, most preferred first, each expanded
/// to the catalogues worth trying: `pt_BR.UTF-8` becomes `pt-BR`, then `pt`.
fn env_locales() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        let Ok(value) = std::env::var(var) else {
            continue;
        };
        // LANGUAGE is a colon-separated list; the others name one locale.
        for tag in value.split(':') {
            for candidate in expand(tag) {
                if !out.contains(&candidate) {
                    out.push(candidate);
                }
            }
        }
    }
    out
}

/// One tag's candidates, most specific first, in the spelling the catalogues
/// use. Mirrors otto-kit: every Chinese tag falls back to `zh-CN`, a bare
/// `en` to `en-US`.
fn expand(tag: &str) -> Vec<String> {
    let cleaned = tag
        .split(['.', '@'])
        .next()
        .unwrap_or(tag)
        .replace('_', "-");
    if cleaned.is_empty() || cleaned == "C" || cleaned == "POSIX" {
        return Vec::new();
    }
    let mut out = vec![cleaned.clone()];
    if let Some((lang, _)) = cleaned.split_once('-') {
        if lang.eq_ignore_ascii_case("zh") {
            if !out.iter().any(|c| c.eq_ignore_ascii_case("zh-CN")) {
                out.push("zh-CN".to_owned());
            }
        } else if !out.iter().any(|c| c == lang) {
            out.push(lang.to_owned());
        }
    } else if cleaned.eq_ignore_ascii_case("zh") {
        out.push("zh-CN".to_owned());
    } else if cleaned.eq_ignore_ascii_case("en") {
        out.push("en-US".to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_expand_to_the_catalogues_worth_trying() {
        assert_eq!(expand("pt_BR.UTF-8"), ["pt-BR", "pt"]);
        assert_eq!(expand("zh_TW"), ["zh-TW", "zh-CN"]);
        assert_eq!(expand("en"), ["en", "en-US"]);
        assert!(expand("C").is_empty());
    }

    #[test]
    fn the_chain_always_ends_at_the_source_locale() {
        let chain = build_chain(&["de".to_owned()]);
        let locales: Vec<String> = chain
            .iter()
            .map(|bundle| bundle.locales[0].to_string())
            .collect();
        assert_eq!(locales, ["de", "en-GB"]);
        assert!(build_chain(&["xx".to_owned()]).len() == 1);
    }

    #[test]
    fn a_missing_key_comes_back_as_itself() {
        assert_eq!(t("agents-no-such-key", None), "agents-no-such-key");
    }
}
