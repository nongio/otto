//! Which app a process belongs to, read from the process itself rather than
//! from anything it says about itself.
//!
//! A Flatpak app is named by its sandbox's own record of it, which the app
//! cannot change. Any other program is named by the desktop entry that runs
//! its executable, or by the executable's file name when none does.

use std::path::Path;

/// The app `pid` belongs to, by desktop file id, Flatpak app id, or program
/// name, in that order of preference.
pub fn app_id_for_pid(pid: u32) -> Option<String> {
    if let Some(app) = flatpak_app(pid) {
        return Some(app);
    }
    let program = program(pid)?;
    Some(
        crate::desktop_entry::lookup_app_by_binary(&program)
            .and_then(|info| info.desktop_file_id)
            .unwrap_or(program),
    )
}

/// The file name of the executable `pid` runs.
pub fn program(pid: u32) -> Option<String> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    exe.file_name()?.to_str().map(str::to_string)
}

/// The app id of a Flatpak app's process.
pub fn flatpak_app(pid: u32) -> Option<String> {
    let info =
        std::fs::read_to_string(Path::new(&format!("/proc/{pid}/root/.flatpak-info"))).ok()?;
    flatpak_info_app(&info)
}

fn flatpak_info_app(info: &str) -> Option<String> {
    let mut in_application = false;
    for line in info.lines().map(str::trim) {
        if line.starts_with('[') {
            in_application = line == "[Application]";
        } else if in_application {
            if let Some(name) = line.strip_prefix("name=") {
                return Some(name.to_string()).filter(|name| !name.is_empty());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flatpak_is_named_by_its_sandbox() {
        let info =
            "[Application]\nname=com.google.Chrome\nruntime=runtime/x\n\n[Instance]\nname=other\n";
        assert_eq!(flatpak_info_app(info).as_deref(), Some("com.google.Chrome"));
        assert_eq!(flatpak_info_app("[Instance]\nname=x\n"), None);
    }

    #[test]
    fn this_process_is_named() {
        assert!(app_id_for_pid(std::process::id()).is_some());
    }
}
