//! Localisation.
//!
//! Otto's user-facing strings live in Fluent catalogues under
//! `resources/locales/<locale>.ftl`, one file per locale, keyed by stable
//! identifiers rather than by their English text. `en-GB.ftl` is the source of
//! truth and the only file guaranteed to carry every key.
//!
//! # Using it
//!
//! ```ignore
//! use otto_kit::t;
//!
//! let label = t!("dock-keep-in-dock");
//! let status = t!("files-item-count", count = 3);
//! ```
//!
//! [`t!`] returns a `&'static str` for the plain case, which is what lets it
//! drop into the `&'static str` fields that Otto's menus and settings rows
//! already use. That works because a catalogue is immutable once loaded and
//! lives for the life of the process, so a formatted string can be interned
//! and handed out by reference forever. The interner is only consulted for
//! messages with arguments — a plain lookup borrows straight out of the
//! catalogue.
//!
//! # Fallback
//!
//! Locales resolve through a chain, most specific first, always ending at
//! `en-GB`. A user asking for `en-US` gets `[en-US, en-GB]`, so `en-US.ftl`
//! only needs the keys that actually differ — spellings and date formats —
//! and everything else falls through. The same holds for `pt-BR`, and for
//! the Chinese tags that all resolve to `zh-CN`.
//!
//! A key missing from every bundle in the chain is a bug, not a runtime
//! error: the key itself is returned so the gap is visible in the interface
//! rather than crashing the desktop.

pub mod portal;

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

use fluent_bundle::{bundle::FluentBundle, FluentArgs, FluentResource};

/// Re-exported so [`t!`] and [`t_owned!`] expand without the calling crate
/// having to depend on `fluent-bundle` itself — a macro should not leak the
/// dependencies of the crate that defines it.
pub use fluent_bundle::FluentValue;
use unic_langid::LanguageIdentifier;

/// The locale every chain ends at, and the only catalogue that carries every
/// key. Chosen because Otto's strings are authored in British English.
pub const SOURCE_LOCALE: &str = "en-GB";

/// Locales with a catalogue compiled into the binary.
///
/// Baked in with `include_str!` rather than read from disk: the compositor
/// draws its first frame before any filesystem the user controls is
/// necessarily mounted, and a desktop that cannot find its own strings is not
/// a recoverable state. It also keeps the catalogues in step with the code
/// that references them — a stale `.ftl` on disk cannot drift out of sync
/// with the keys the binary asks for.
const CATALOGUES: &[(&str, &str)] = &[
    (
        "en-GB",
        include_str!("../../../../resources/locales/en-GB.ftl"),
    ),
    (
        "en-US",
        include_str!("../../../../resources/locales/en-US.ftl"),
    ),
    ("de", include_str!("../../../../resources/locales/de.ftl")),
    ("es", include_str!("../../../../resources/locales/es.ftl")),
    ("fr", include_str!("../../../../resources/locales/fr.ftl")),
    ("it", include_str!("../../../../resources/locales/it.ftl")),
    ("pl", include_str!("../../../../resources/locales/pl.ftl")),
    (
        "pt-BR",
        include_str!("../../../../resources/locales/pt-BR.ftl"),
    ),
    ("ru", include_str!("../../../../resources/locales/ru.ftl")),
    ("uk", include_str!("../../../../resources/locales/uk.ftl")),
    (
        "zh-CN",
        include_str!("../../../../resources/locales/zh-CN.ftl"),
    ),
];

/// A bundle is `FluentBundle<FluentResource, IntlLangMemoizer>` — the concrete
/// memoizer matters only because the default alias is not `Send`.
type Bundle = FluentBundle<FluentResource, intl_memoizer::concurrent::IntlLangMemoizer>;

/// The loaded chain, most specific locale first.
static CHAIN: OnceLock<Vec<Bundle>> = OnceLock::new();

/// Formatted strings handed out as `&'static str`.
///
/// Only messages that take arguments land here; a plain lookup borrows out of
/// the catalogue and never allocates. Bounded in practice by the number of
/// distinct argument combinations the interface actually renders — item
/// counts, display names — which is small and repeats.
static INTERNED: OnceLock<RwLock<HashSet<&'static str>>> = OnceLock::new();

/// Resolve the locale chain and load the catalogues.
///
/// `requested` is the user's preferred locales, most preferred first, as they
/// come from `config.locales` or the `LC_*`/`LANG` environment. Unknown tags
/// are skipped. Calling this more than once is a no-op — the first call wins,
/// so the compositor sets the locale before any component reads a string.
pub fn init(requested: &[String]) {
    CHAIN.get_or_init(|| build_chain(requested));
}

/// Initialise from the environment, for components started outside the
/// compositor's settings plumbing.
pub fn init_from_env() {
    init(&env_locales());
}

/// Initialise from the compositor's "Preferred languages" setting, falling
/// back to the environment when it cannot be reached.
///
/// This is what Otto's own components should call: it makes them agree with
/// the compositor rather than with `LANG`, which is what the user actually
/// changed when they edited the setting. See [`portal`].
pub fn init_from_desktop() {
    init(&portal::locales_blocking());
}

fn build_chain(requested: &[String]) -> Vec<Bundle> {
    let mut wanted: Vec<String> = Vec::new();
    for tag in requested {
        for candidate in expand(tag) {
            if !wanted.contains(&candidate) {
                wanted.push(candidate);
            }
        }
    }
    // Every chain ends at the source locale, which is the only catalogue
    // guaranteed to be complete.
    if !wanted.iter().any(|l| l == SOURCE_LOCALE) {
        wanted.push(SOURCE_LOCALE.to_string());
    }

    wanted
        .iter()
        .filter_map(|tag| {
            let source = CATALOGUES
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(tag))
                .map(|(_, src)| *src)?;
            let langid: LanguageIdentifier = tag.parse().ok()?;
            let resource = match FluentResource::try_new(source.to_string()) {
                Ok(res) => res,
                // A catalogue that does not parse is a build-time mistake. Skip
                // it rather than take the desktop down; the chain falls through
                // to the next locale.
                Err((res, errors)) => {
                    tracing::error!("locale {tag}: {} parse error(s): {errors:?}", errors.len());
                    res
                }
            };
            let mut bundle = Bundle::new_concurrent(vec![langid]);
            // Fluent wraps placeables in U+2068/U+2069 to isolate their
            // direction. Correct for bidirectional text, but it renders as
            // stray boxes in Skia, and Otto ships no RTL locale yet.
            bundle.set_use_isolating(false);
            if bundle.add_resource(resource).is_err() {
                tracing::error!("locale {tag}: duplicate message identifiers");
            }
            Some(bundle)
        })
        .collect()
}

/// Expand one requested tag into the candidates worth trying, most specific
/// first: `pt_BR.UTF-8` becomes `["pt-BR", "pt"]`.
fn expand(tag: &str) -> Vec<String> {
    // Accept POSIX spellings (`en_GB.UTF-8@euro`) as well as BCP 47.
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
        // A bare `zh` is ambiguous between the scripts. Simplified is far
        // the more common on Linux, which names it by its territory rather
        // than its script, so every Chinese tag — `zh-SG` and `zh-Hans`
        // included — falls back to `zh-CN`. Anything else falls back to its
        // language subtag.
        if lang.eq_ignore_ascii_case("zh") {
            if !out.iter().any(|c| c.eq_ignore_ascii_case("zh-CN")) {
                out.push("zh-CN".to_string());
            }
        } else if !out.iter().any(|c| c == lang) {
            out.push(lang.to_string());
        }
    } else if cleaned.eq_ignore_ascii_case("zh") {
        out.push("zh-CN".to_string());
    } else if cleaned.eq_ignore_ascii_case("en") {
        // Bare `en` means American English by convention nearly everywhere it
        // appears, even though Otto authors in British English.
        out.push("en-US".to_string());
    }
    out
}

/// Preferred locales from the environment, most preferred first.
pub fn env_locales() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        let Ok(value) = std::env::var(var) else {
            continue;
        };
        // LANGUAGE is a colon-separated priority list; the others are single.
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

/// The locale actually in use — the first one in the resolved chain that had a
/// catalogue, as a BCP 47 tag.
pub fn current_locale() -> String {
    chain()
        .first()
        .and_then(|b| b.locales.first().map(|l| l.to_string()))
        .unwrap_or_else(|| SOURCE_LOCALE.to_string())
}

/// A BCP 47 tag written the way POSIX writes it: `pt-BR` is `pt_BR`. Any
/// encoding or modifier already on the string is dropped.
///
/// The subtag after the language is usually a territory, which POSIX spells
/// the same way, so the conversion is mostly punctuation. A script subtag is
/// not: POSIX has no place for one, and `zh_Hans` names nothing on any
/// machine. Those are mapped to the territory conventionally used for the
/// script instead, which is what both the C library and `chrono` have a
/// locale for. Only Chinese needs this today — the other scripts Otto could
/// meet (`sr-Latn`) are POSIX modifiers rather than territories, and are left
/// alone until there is a catalogue that wants one.
pub fn posix_form(tag: &str) -> String {
    let bare = tag.split(['.', '@']).next().unwrap_or(tag);
    if let Some((lang, script)) = bare.split_once(['-', '_']) {
        if let Some(territory) = script_territory(lang, script) {
            return format!("{lang}_{territory}");
        }
    }
    bare.replace('-', "_")
}

/// The territory that stands in for a script subtag, if this is one.
fn script_territory(lang: &str, script: &str) -> Option<&'static str> {
    match (
        lang.to_ascii_lowercase().as_str(),
        script.to_ascii_lowercase().as_str(),
    ) {
        ("zh", "hans") => Some("CN"),
        ("zh", "hant") => Some("TW"),
        _ => None,
    }
}

/// The current locale as a POSIX name, for libraries that want one.
///
/// `chrono` localises month and weekday names only against a POSIX-ish locale
/// (`ru_RU`, `pt_BR`), so a language subtag on its own is not enough. Regions
/// here are the conventional default for each language Otto ships; a tag that
/// already carries its own region keeps it.
pub fn posix_locale() -> String {
    let tag = posix_form(&current_locale());
    if tag.contains('_') {
        return tag;
    }
    let region = match tag.as_str() {
        "de" => "DE",
        "es" => "ES",
        "fr" => "FR",
        "it" => "IT",
        "pl" => "PL",
        "ru" => "RU",
        "uk" => "UA",
        "ja" => "JP",
        _ => return tag,
    };
    format!("{tag}_{region}")
}

fn chain() -> &'static [Bundle] {
    // A component that never called `init` still has to render. Fall back to
    // the environment rather than to an empty chain.
    CHAIN.get_or_init(|| build_chain(&env_locales()))
}

/// Look a message up, formatting `args` into it.
///
/// Returns the key itself when no bundle in the chain carries it, so a missing
/// string shows up as `dock-keep-in-dock` in the interface — visible, but not
/// fatal.
pub fn lookup(key: &str, args: Option<&FluentArgs>) -> Cow<'static, str> {
    for bundle in chain() {
        let Some(message) = bundle.get_message(key) else {
            continue;
        };
        let Some(pattern) = message.value() else {
            continue;
        };
        let mut errors = Vec::new();
        let formatted = bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            tracing::warn!("localisation error for {key}: {errors:?}");
        }
        return match formatted {
            // No placeables: the pattern is a literal borrowed straight out of
            // the catalogue, which lives as long as the process.
            Cow::Borrowed(s) => Cow::Borrowed(unsafe_extend(s)),
            Cow::Owned(s) => Cow::Owned(s),
        };
    }
    tracing::warn!("missing localisation key: {key}");
    Cow::Owned(key.to_string())
}

/// Look a message up, falling back to `fallback` when no bundle carries it.
///
/// For strings that already live in the code as English and are keyed by
/// something derived rather than written by hand — the settings schema keys
/// its labels off each setting's identifier. The English in the source stays
/// both the thing translators translate and the thing shown when they have
/// not yet, so a new setting is never worse than untranslated.
pub fn lookup_or(key: &str, fallback: &str) -> String {
    for bundle in chain() {
        let Some(message) = bundle.get_message(key) else {
            continue;
        };
        let Some(pattern) = message.value() else {
            continue;
        };
        let mut errors = Vec::new();
        let formatted = bundle.format_pattern(pattern, None, &mut errors);
        if !errors.is_empty() {
            tracing::warn!("localisation error for {key}: {errors:?}");
        }
        return formatted.into_owned();
    }
    fallback.to_string()
}

/// Look up a message and return it as a `&'static str`.
///
/// Formatted results are interned so callers that need a `&'static str` — the
/// menu and settings-row types — can have one.
pub fn lookup_static(key: &str, args: Option<&FluentArgs>) -> &'static str {
    match lookup(key, args) {
        Cow::Borrowed(s) => s,
        Cow::Owned(s) => intern(s),
    }
}

/// Promote a catalogue borrow to `'static`.
///
/// Sound because the bundles live in a `OnceLock` that is never cleared and
/// never handed out mutably after `init`, so every string inside one outlives
/// any caller.
fn unsafe_extend(s: &str) -> &'static str {
    unsafe { std::mem::transmute::<&str, &'static str>(s) }
}

fn intern(s: String) -> &'static str {
    let set = INTERNED.get_or_init(|| RwLock::new(HashSet::new()));
    if let Some(existing) = set.read().unwrap().get(s.as_str()) {
        return existing;
    }
    let mut guard = set.write().unwrap();
    if let Some(existing) = guard.get(s.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(s.into_boxed_str());
    guard.insert(leaked);
    leaked
}

/// Build [`FluentArgs`] from the `key = value` pairs a [`t!`] call supplies.
#[doc(hidden)]
pub fn args_from<'a>(pairs: Vec<(&'a str, FluentValue<'a>)>) -> FluentArgs<'a> {
    let mut args = FluentArgs::new();
    for (name, value) in pairs {
        args.set(name, value);
    }
    args
}

/// Every key the loaded source catalogue carries.
///
/// Used by the parity tests; not something the interface needs.
pub fn source_keys() -> HashSet<String> {
    let source = CATALOGUES
        .iter()
        .find(|(name, _)| *name == SOURCE_LOCALE)
        .map(|(_, src)| *src)
        .unwrap_or_default();
    keys_in(source)
}

/// Every message identifier defined in a `.ftl` source.
pub fn keys_in(source: &str) -> HashSet<String> {
    source
        .lines()
        .filter_map(|line| {
            // A message starts at column zero; continuations and attributes are
            // indented, and comments start with `#`.
            if line.starts_with(char::is_whitespace) || line.starts_with('#') {
                return None;
            }
            let (key, _) = line.split_once('=')?;
            let key = key.trim();
            (!key.is_empty() && !key.starts_with('-')).then(|| key.to_string())
        })
        .collect()
}

/// The catalogues compiled into this binary, as `(locale, source)`.
pub fn catalogues() -> &'static [(&'static str, &'static str)] {
    CATALOGUES
}

/// Look up a localised string by key.
///
/// ```ignore
/// t!("dock-quit")                       // -> &'static str
/// t!("files-item-count", count = 3)     // -> &'static str
/// t!("files-copying", name = path)      // values may be &str or numbers
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::lookup_static($key, None)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let args = $crate::i18n::args_from(vec![
            $((stringify!($name), $crate::i18n::FluentValue::from($value))),+
        ]);
        $crate::i18n::lookup_static($key, Some(&args))
    }};
}

/// Like [`t!`], but returns an owned `String` without interning.
///
/// Use this for strings built from unbounded input — a file name, a window
/// title — where interning would grow without limit.
#[macro_export]
macro_rules! t_owned {
    ($key:expr) => {
        $crate::i18n::lookup($key, None).into_owned()
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let args = $crate::i18n::args_from(vec![
            $((stringify!($name), $crate::i18n::FluentValue::from($value))),+
        ]);
        $crate::i18n::lookup($key, Some(&args)).into_owned()
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn expands_posix_tags() {
        assert_eq!(expand("pt_BR.UTF-8"), vec!["pt-BR", "pt"]);
        assert_eq!(expand("de_DE@euro"), vec!["de-DE", "de"]);
        assert_eq!(expand("fr"), vec!["fr"]);
        assert!(expand("C").is_empty());
        assert!(expand("POSIX").is_empty());
    }

    #[test]
    fn bare_en_means_american() {
        assert_eq!(expand("en"), vec!["en", "en-US"]);
    }

    #[test]
    fn bare_chinese_resolves_to_simplified() {
        assert_eq!(expand("zh"), vec!["zh", "zh-CN"]);
        assert_eq!(expand("zh_CN.UTF-8"), vec!["zh-CN"]);
        // Simplified outside the mainland, and the script spelling, both land
        // on the catalogue rather than falling through to English.
        assert_eq!(expand("zh_SG"), vec!["zh-SG", "zh-CN"]);
        assert_eq!(expand("zh-Hans"), vec!["zh-Hans", "zh-CN"]);
    }

    #[test]
    fn a_territory_survives_the_posix_rewrite() {
        assert_eq!(posix_form("pt-BR"), "pt_BR");
        assert_eq!(posix_form("de_DE@euro"), "de_DE");
        assert_eq!(posix_form("it_IT.UTF-8"), "it_IT");
        assert_eq!(posix_form("it"), "it");
    }

    /// `zh_Hans` names no locale on any machine — neither `chrono` nor the C
    /// library has one — so the script has to become the territory it stands
    /// for, whichever spelling the tag arrives in.
    #[test]
    fn a_script_becomes_the_territory_it_stands_for() {
        assert_eq!(posix_form("zh-Hans"), "zh_CN");
        assert_eq!(posix_form("zh_Hans"), "zh_CN");
        assert_eq!(posix_form("zh-hans.UTF-8"), "zh_CN");
        assert_eq!(posix_form("zh-Hant"), "zh_TW");
        // A territory that merely looks like one is still a territory.
        assert_eq!(posix_form("zh-CN"), "zh_CN");
    }

    #[test]
    fn source_catalogue_parses() {
        let source = CATALOGUES
            .iter()
            .find(|(name, _)| *name == SOURCE_LOCALE)
            .map(|(_, src)| *src)
            .expect("en-GB catalogue is compiled in");
        assert!(
            FluentResource::try_new(source.to_string()).is_ok(),
            "en-GB.ftl does not parse"
        );
    }

    /// Every translated locale carries every key `en-GB` does.
    ///
    /// `en-US` is exempt: it is a sparse overlay that deliberately carries only
    /// the keys whose spelling or date format differs.
    #[test]
    fn translated_locales_are_complete() {
        let source = source_keys();
        let mut gaps = Vec::new();
        for (locale, catalogue) in CATALOGUES {
            if *locale == SOURCE_LOCALE || *locale == "en-US" {
                continue;
            }
            let present = keys_in(catalogue);
            let mut missing: Vec<_> = source.difference(&present).cloned().collect();
            if !missing.is_empty() {
                missing.sort();
                gaps.push(format!("{locale}: {}", missing.join(", ")));
            }
        }
        assert!(
            gaps.is_empty(),
            "keys missing from catalogues:\n{}",
            gaps.join("\n")
        );
    }

    /// No locale invents a key the source does not have — usually a typo that
    /// would silently never be read.
    #[test]
    fn locales_invent_no_keys() {
        let source = source_keys();
        let mut extra = Vec::new();
        for (locale, catalogue) in CATALOGUES {
            if *locale == SOURCE_LOCALE {
                continue;
            }
            let mut unknown: Vec<_> = keys_in(catalogue).difference(&source).cloned().collect();
            if !unknown.is_empty() {
                unknown.sort();
                extra.push(format!("{locale}: {}", unknown.join(", ")));
            }
        }
        assert!(
            extra.is_empty(),
            "keys not in en-GB.ftl:\n{}",
            extra.join("\n")
        );
    }

    /// Every catalogue parses and loads cleanly.
    #[test]
    fn all_catalogues_parse() {
        for (locale, catalogue) in CATALOGUES {
            let result = FluentResource::try_new(catalogue.to_string());
            assert!(result.is_ok(), "{locale}.ftl does not parse");
        }
    }

    /// A locale must not silently drop a variable the English string uses: a
    /// count that never reaches the text reads as a missing number.
    #[test]
    fn placeables_survive_translation() {
        fn placeables(source: &str) -> HashMap<String, HashSet<String>> {
            let mut out: HashMap<String, HashSet<String>> = HashMap::new();
            let mut current: Option<String> = None;
            for line in source.lines() {
                if !line.starts_with(char::is_whitespace) && !line.starts_with('#') {
                    if let Some((key, _)) = line.split_once('=') {
                        let key = key.trim();
                        if !key.is_empty() && !key.starts_with('-') {
                            current = Some(key.to_string());
                        }
                    }
                }
                let Some(key) = current.as_ref() else {
                    continue;
                };
                let mut rest = line;
                while let Some(at) = rest.find("{ $") {
                    rest = &rest[at + 3..];
                    let end = rest.find([' ', '}']).unwrap_or(rest.len());
                    out.entry(key.clone())
                        .or_default()
                        .insert(rest[..end].to_string());
                }
            }
            out
        }

        let source = placeables(
            CATALOGUES
                .iter()
                .find(|(name, _)| *name == SOURCE_LOCALE)
                .map(|(_, src)| *src)
                .unwrap(),
        );

        let mut problems = Vec::new();
        for (locale, catalogue) in CATALOGUES {
            if *locale == SOURCE_LOCALE {
                continue;
            }
            let theirs = placeables(catalogue);
            for (key, wanted) in &source {
                let Some(got) = theirs.get(key) else { continue };
                let missing: Vec<_> = wanted.difference(got).cloned().collect();
                if !missing.is_empty() {
                    problems.push(format!("{locale}/{key}: dropped ${}", missing.join(", $")));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
