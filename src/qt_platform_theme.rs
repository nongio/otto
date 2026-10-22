//! The platform theme Qt applications take in an Otto session.
//!
//! Qt chooses its platform theme from `XDG_CURRENT_DESKTOP`: GNOME and the
//! other GTK desktops get `gtk3`, KDE gets `kde`, and any other name is
//! looked up as a theme plugin of that name. Under `otto` there is none, so
//! every Qt application fell back to Qt's generic theme: `Sans Serif 9`, no
//! colour scheme, Qt's own file dialogs. Qt's own `gtk3` theme reads what
//! Otto writes for GTK 3 (`gnome_interface`), the interface font and the
//! colour scheme, and follows the scheme live, so an Otto session names it,
//! through `QT_QPA_PLATFORMTHEME`, unless the user already named a theme of
//! their own (`kde` for KDE's menus in the top bar, say).
//!
//! Rasterisation is not the theme's business: Qt reads hinting and
//! antialiasing from fontconfig, as Otto does, except that at a device pixel
//! ratio other than 1 it draws every font unhinted and antialiased whatever
//! fontconfig says (`qfontconfigdatabase.cpp`, `isDprScaling`); nothing a
//! compositor publishes changes that.

pub const VAR: &str = "QT_QPA_PLATFORMTHEME";
const THEME: &str = "gtk3";

/// The theme Otto names, given what the environment already says: none
/// when the user named one.
fn choose(current: Option<&str>) -> Option<&'static str> {
    match current {
        Some(named) if !named.trim().is_empty() => None,
        _ => Some(THEME),
    }
}

/// Name the theme for everything Otto spawns, and return the assignment for
/// the session services that are not its children (bus-activated ones
/// inherit nothing from it), or nothing when the user's choice stands.
pub fn apply() -> Option<String> {
    let chosen = choose(std::env::var(VAR).ok().as_deref())?;
    std::env::set_var(VAR, chosen);
    Some(format!("{VAR}={chosen}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_s_theme_stands() {
        assert_eq!(choose(None), Some("gtk3"));
        assert_eq!(choose(Some("")), Some("gtk3"));
        assert_eq!(choose(Some("kde")), None);
        assert_eq!(choose(Some("qt5ct")), None);
    }
}
