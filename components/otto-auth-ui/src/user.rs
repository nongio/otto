//! Who the panel is authenticating.
//!
//! greetd exposes no user database, so the display name and avatar are looked
//! up the same way a desktop does: `/etc/passwd` for the real name, then the
//! conventional avatar locations. Nothing here needs privileges — a greeter
//! running as its own unprivileged user can read all of it.

use std::path::{Path, PathBuf};

/// Where desktops keep avatars, in the order they are preferred. The
/// AccountsService copy is the one GNOME/KDE write, so it is the most likely
/// to exist and the most likely to be current.
const AVATAR_PATHS: [&str; 1] = ["/var/lib/AccountsService/icons"];

/// Avatar filenames looked for inside the user's home directory.
const HOME_AVATARS: [&str; 3] = [".face", ".face.icon", ".local/share/avatar"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// Login name.
    pub name: String,
    /// Name to show. The real name from `/etc/passwd` when there is one, the
    /// login name otherwise.
    pub display_name: String,
    pub avatar: Option<PathBuf>,
}

impl User {
    /// Look `name` up in the password database.
    ///
    /// A name with no entry still produces a `User` — the greeter shows what
    /// was typed, and letting the authentication attempt fail is what tells
    /// the person they got it wrong, not the panel refusing to render.
    pub fn lookup(name: &str) -> Self {
        let entry = passwd_entry(name);
        let home = entry.as_ref().map(|entry| PathBuf::from(&entry.home));

        Self {
            name: name.to_string(),
            display_name: entry
                .as_ref()
                .and_then(|entry| entry.real_name())
                .unwrap_or_else(|| name.to_string()),
            avatar: find_avatar(name, home.as_deref()),
        }
    }

    /// The user this process is running as — the subject of a lock screen,
    /// where there is nothing to type a name into.
    pub fn current() -> Option<Self> {
        let name = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .ok()?;
        (!name.is_empty()).then(|| Self::lookup(&name))
    }

    /// Up to two initials, for when there is no avatar to draw.
    pub fn initials(&self) -> String {
        let mut initials: String = self
            .display_name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .flat_map(|c| c.to_uppercase())
            .collect();

        if initials.is_empty() {
            initials = self
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
        }
        initials
    }
}

struct PasswdEntry {
    gecos: String,
    home: String,
}

impl PasswdEntry {
    /// The GECOS field is comma-separated; only the first part is the name.
    /// It is often empty for system accounts, which should fall back rather
    /// than render as a blank line.
    fn real_name(&self) -> Option<String> {
        let name = self.gecos.split(',').next()?.trim();
        (!name.is_empty()).then(|| name.to_string())
    }
}

fn passwd_entry(name: &str) -> Option<PasswdEntry> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;

    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == name).then_some(())?;
        let mut fields = fields.skip(3); // password, uid, gid
        Some(PasswdEntry {
            gecos: fields.next()?.to_string(),
            home: fields.next()?.to_string(),
        })
    })
}

fn find_avatar(name: &str, home: Option<&Path>) -> Option<PathBuf> {
    let system = AVATAR_PATHS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file());
    if system.is_some() {
        return system;
    }

    // A greeter cannot usually read into another user's home directory; that
    // is fine, `is_file` simply fails and the initials are drawn instead.
    let home = home?;
    HOME_AVATARS
        .iter()
        .map(|file| home.join(file))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_come_from_the_display_name() {
        let user = User {
            name: "riccardo".into(),
            display_name: "Riccardo Canalicchio".into(),
            avatar: None,
        };
        assert_eq!(user.initials(), "RC");
    }

    #[test]
    fn initials_fall_back_to_the_login_name() {
        let user = User {
            name: "greeter".into(),
            display_name: "".into(),
            avatar: None,
        };
        assert_eq!(user.initials(), "G");
    }

    /// A single-word name must not produce an empty string, and non-ASCII
    /// names must uppercase properly rather than being dropped.
    #[test]
    fn initials_handle_one_word_and_non_ascii() {
        let one = User {
            name: "ada".into(),
            display_name: "ada".into(),
            avatar: None,
        };
        assert_eq!(one.initials(), "A");

        let accented = User {
            name: "orn".into(),
            display_name: "örn þór".into(),
            avatar: None,
        };
        assert_eq!(accented.initials(), "ÖÞ");
    }

    #[test]
    fn unknown_users_still_render() {
        let user = User::lookup("definitely-not-a-real-account");
        assert_eq!(user.display_name, "definitely-not-a-real-account");
        assert!(user.avatar.is_none());
    }

    /// Fields after the home directory (the shell) must not be mistaken for it,
    /// and a `gecos` with extra comma fields must yield only the name.
    #[test]
    fn passwd_parsing_picks_the_right_fields() {
        let entry = PasswdEntry {
            gecos: "Riccardo Canalicchio,,,".into(),
            home: "/home/riccardo".into(),
        };
        assert_eq!(entry.real_name().as_deref(), Some("Riccardo Canalicchio"));

        let system = PasswdEntry {
            gecos: "".into(),
            home: "/".into(),
        };
        assert_eq!(system.real_name(), None);
    }
}
