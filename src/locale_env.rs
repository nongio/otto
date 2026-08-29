//! The locale Otto hands to the applications it starts.
//!
//! Otto's own language comes from the `locales` setting, not from the
//! environment — a user who picks Italian in Settings gets Italian chrome
//! whatever `LANG` the session was started with. Applications cannot read that
//! setting: they read the environment, the way every toolkit does. Without
//! this the two disagree, and an Italian desktop launches English apps.
//!
//! Two variables, for two different reasons:
//!
//! * `LANGUAGE` is gettext's own, a priority list of languages, and it is
//!   honoured whether or not the named locales have been generated on this
//!   machine. It is always set.
//! * `LANG` and `LC_MESSAGES` are the C library's, and naming a locale that
//!   was never generated is worse than saying nothing: `setlocale` fails and
//!   the application falls back to C, losing the translation this was meant to
//!   supply. They are set only to a locale that actually exists here.
//!
//! `LC_ALL` is deliberately left alone. It overrides everything, so a user or
//! a session script that set it meant to, and Otto is not the right authority
//! to overrule it.

/// The assignments to publish, in `KEY=value` form.
///
/// `available` is the set of locales the C library has, as `locale -a` prints
/// them (`it_IT.utf8`); `lc_all_set` says whether the environment already
/// carries an `LC_ALL` that would make `LANG` moot.
pub fn assignments(locales: &[String], available: &[String], lc_all_set: bool) -> Vec<String> {
    let mut out = Vec::new();

    let language = language_list(locales);
    if !language.is_empty() {
        out.push(format!("LANGUAGE={language}"));
    }

    if !lc_all_set {
        if let Some(locale) = first_generated(locales, available) {
            out.push(format!("LANG={locale}"));
            out.push(format!("LC_MESSAGES={locale}"));
        }
    }

    out
}

/// `LANGUAGE`'s colon-separated priority list.
///
/// Each configured tag contributes its POSIX form and its bare language, so
/// `pt_BR` also matches a catalogue that only ships `pt`. English is not
/// appended: gettext already falls back to the untranslated source string,
/// which is English, and a literal `en` in the list would defeat a translation
/// that happens to be listed after it.
fn language_list(locales: &[String]) -> String {
    let mut tags: Vec<String> = Vec::new();
    for locale in locales {
        let posix = posix_form(locale);
        let language = posix.split('_').next().unwrap_or(&posix).to_string();
        for candidate in [posix, language] {
            if !candidate.is_empty() && !tags.contains(&candidate) {
                tags.push(candidate);
            }
        }
    }
    tags.join(":")
}

/// The first configured locale the C library can actually be set to, named the
/// way `setlocale` wants it (`it_IT.UTF-8`).
fn first_generated(locales: &[String], available: &[String]) -> Option<String> {
    let generated: Vec<String> = available.iter().map(|l| normalise(l)).collect();
    for locale in locales {
        let posix = posix_form(locale);
        if posix.is_empty() {
            continue;
        }
        // A tag without a region cannot name a C locale on its own: `it` is
        // not a locale, `it_IT` is. Take whichever generated locale is for
        // that language, so a user who typed just `it` still gets Italian.
        let candidates: Vec<String> = if posix.contains('_') {
            vec![posix.clone()]
        } else {
            available
                .iter()
                .filter(|l| l.starts_with(&format!("{posix}_")))
                .map(|l| l.split('.').next().unwrap_or(l).to_string())
                .collect()
        };
        for candidate in candidates {
            let wanted = normalise(&format!("{candidate}.UTF-8"));
            if generated.contains(&wanted) {
                return Some(format!("{candidate}.UTF-8"));
            }
        }
    }
    None
}

/// A BCP 47 tag as POSIX writes it: `pt-BR` is `pt_BR`, and any encoding or
/// modifier already on the string is dropped.
fn posix_form(locale: &str) -> String {
    locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .replace('-', "_")
}

/// `locale -a` prints `it_IT.utf8`, `setlocale` is given `it_IT.UTF-8`, and
/// both mean the same thing. Compare them with the punctuation and case taken
/// out rather than guessing which spelling a machine uses.
fn normalise(locale: &str) -> String {
    locale
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The locales this machine has generated, as `locale -a` reports them.
///
/// A machine where the command is missing or fails is treated as having none,
/// which costs the `LANG` export and keeps `LANGUAGE` — the conservative half.
pub fn generated_locales() -> Vec<String> {
    let Ok(output) = std::process::Command::new("locale").arg("-a").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// What [`export`] published, for the session export to repeat.
static PUBLISHED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Publish the configured language into this process's environment, so every
/// application Otto spawns inherits it.
pub fn export() {
    PUBLISHED.get_or_init(|| {
        let lc_all = std::env::var("LC_ALL").is_ok_and(|v| !v.is_empty());
        let assignments = assignments(&crate::configured_locales(), &generated_locales(), lc_all);
        for assignment in &assignments {
            if let Some((key, value)) = assignment.split_once('=') {
                std::env::set_var(key, value);
            }
        }
        assignments
    });
}

/// The assignments [`export`] made.
///
/// Bus-activated services are not Otto's children and inherit nothing from it,
/// so the session's activation environments have to be told separately — the
/// same way `WAYLAND_DISPLAY` is.
pub fn published() -> &'static [String] {
    PUBLISHED.get().map(Vec::as_slice).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> Vec<String> {
        ["it_IT", "it_IT.utf8", "en_US", "en_US.utf8", "pt_BR.utf8"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn a_generated_locale_is_named_the_way_setlocale_wants_it() {
        let out = assignments(&["it_IT".into()], &available(), false);
        assert!(out.contains(&"LANG=it_IT.UTF-8".to_string()));
        assert!(out.contains(&"LC_MESSAGES=it_IT.UTF-8".to_string()));
    }

    /// The whole point of `LANGUAGE`: a translation still applies on a machine
    /// that never generated the matching C locale.
    #[test]
    fn a_locale_that_is_not_generated_sets_language_but_not_lang() {
        let out = assignments(&["de_DE".into()], &available(), false);
        assert_eq!(out, ["LANGUAGE=de_DE:de"]);
    }

    #[test]
    fn a_bare_language_finds_the_generated_locale_for_it() {
        let out = assignments(&["it".into()], &available(), false);
        assert!(out.contains(&"LANG=it_IT.UTF-8".to_string()));
        assert!(out.contains(&"LANGUAGE=it".to_string()));
    }

    #[test]
    fn a_bcp47_tag_is_rewritten_the_posix_way() {
        let out = assignments(&["pt-BR".into()], &available(), false);
        assert!(out.contains(&"LANG=pt_BR.UTF-8".to_string()));
        assert!(out.contains(&"LANGUAGE=pt_BR:pt".to_string()));
    }

    /// Every configured language is offered to gettext, in order, so a second
    /// choice covers what the first has not translated.
    #[test]
    fn language_keeps_the_whole_preference_list() {
        let out = assignments(&["it_IT".into(), "fr".into()], &available(), false);
        assert!(out.contains(&"LANGUAGE=it_IT:it:fr".to_string()));
    }

    #[test]
    fn an_explicit_lc_all_is_left_to_win() {
        let out = assignments(&["it_IT".into()], &available(), true);
        assert_eq!(out, ["LANGUAGE=it_IT:it"]);
    }

    #[test]
    fn no_configured_locale_publishes_nothing() {
        assert!(assignments(&[], &available(), false).is_empty());
    }
}
