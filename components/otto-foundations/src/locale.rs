//! Which locales the environment asks for messages in, read the way gettext
//! reads it. otto-kit and otto-agents each turn these into catalogue names.

/// The locales the environment asks messages to be in, most preferred first,
/// as written (`pt_BR.UTF-8`, `de`).
///
/// gettext's order: `LANGUAGE`, a colon-separated list, comes first, then the
/// locale messages are in, the first of `LC_ALL`, `LC_MESSAGES` and `LANG`
/// that is set. `LANGUAGE` is ignored when that locale is `C` or `POSIX`,
/// which ask for untranslated text. An empty variable counts as unset.
/// Unlike gettext, `LANGUAGE` still applies when none of the three is set:
/// that is how a worker started with a cleared environment is told its
/// language.
///
/// `var` looks a variable up; [`env_message_locales`] reads the process's
/// environment.
pub fn message_locales(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let get = |name: &str| var(name).filter(|value| !value.is_empty());
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(&get);
    let mut out = Vec::new();
    if !locale.as_deref().is_some_and(is_untranslated) {
        if let Some(list) = get("LANGUAGE") {
            out.extend(
                list.split(':')
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    out.extend(locale);
    out
}

/// [`message_locales`] of the process's environment.
pub fn env_message_locales() -> Vec<String> {
    message_locales(|name| std::env::var(name).ok())
}

/// `C` and `POSIX`, with any encoding or modifier (`C.UTF-8`).
fn is_untranslated(locale: &str) -> bool {
    let bare = locale.split(['.', '@']).next().unwrap_or(locale);
    bare == "C" || bare == "POSIX"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locales(vars: &[(&str, &str)]) -> Vec<String> {
        message_locales(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    #[test]
    fn language_comes_before_the_locale() {
        assert_eq!(
            locales(&[("LANG", "de_DE.UTF-8"), ("LANGUAGE", "fr:it")]),
            ["fr", "it", "de_DE.UTF-8"]
        );
    }

    #[test]
    fn the_first_locale_variable_set_is_the_one() {
        assert_eq!(
            locales(&[("LC_ALL", "pt_BR"), ("LC_MESSAGES", "es"), ("LANG", "de")]),
            ["pt_BR"]
        );
        assert_eq!(
            locales(&[("LC_ALL", ""), ("LC_MESSAGES", "es"), ("LANG", "de")]),
            ["es"]
        );
    }

    #[test]
    fn a_c_locale_ignores_language() {
        assert_eq!(locales(&[("LC_ALL", "C"), ("LANGUAGE", "fr")]), ["C"]);
        assert_eq!(
            locales(&[("LANG", "C.UTF-8"), ("LANGUAGE", "fr")]),
            ["C.UTF-8"]
        );
        assert_eq!(locales(&[("LANG", "POSIX"), ("LANGUAGE", "fr")]), ["POSIX"]);
    }

    #[test]
    fn language_alone_still_applies() {
        assert_eq!(locales(&[("LANGUAGE", "fr::it")]), ["fr", "it"]);
        assert!(locales(&[]).is_empty());
    }
}
