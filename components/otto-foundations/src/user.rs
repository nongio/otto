//! Who the session belongs to, as the environment says.

// Rust guideline compliant 2026-02-21

/// The login name: `$USER`, else `$LOGNAME`. `None` when neither holds one.
///
/// An empty value counts as unset, the way a shell spells it, so an empty
/// `USER` falls through to `LOGNAME` rather than naming nobody.
pub fn login_name() -> Option<String> {
    first_set(["USER", "LOGNAME"].map(std::env::var_os))
}

/// The first of `values` that is set and not empty.
fn first_set(values: impl IntoIterator<Item = Option<std::ffi::OsString>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .filter_map(|value| value.into_string().ok())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule, checked against the lookup's part so no environment is
    /// touched: tests share a process, and its variables with it.
    #[test]
    fn an_empty_or_unset_name_falls_through() {
        let name = |values: [Option<&str>; 2]| first_set(values.map(|v| v.map(Into::into)));
        assert_eq!(name([Some("ada"), Some("root")]).as_deref(), Some("ada"));
        assert_eq!(name([Some(""), Some("ada")]).as_deref(), Some("ada"));
        assert_eq!(name([None, Some("ada")]).as_deref(), Some("ada"));
        assert_eq!(name([Some(""), None]), None);
        assert_eq!(name([None, None]), None);
    }
}
