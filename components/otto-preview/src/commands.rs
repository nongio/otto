//! Studio's commands, in one list: the menus in the top bar, the keys and the
//! keyboard shortcuts sheet all read it, so a key the menu shows is a key
//! that works, and the sheet lists every one.

use otto_kit::app_menu::{Menu, MenuEntry};
use otto_kit::Modifiers;
use smithay_client_toolkit::seat::keyboard::Keysym;

use crate::chrome::Tool;
use crate::viewer::Viewer;

/// Something the window does, from a menu or a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Close,
    Undo,
    Redo,
    Copy,
    SelectAll,
    Pen,
    ShowMarks,
    DeleteMark,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    PreviousPage,
    NextPage,
    Sidebar,
    Chat,
    Shortcuts,
}

const ALL: [Command; 16] = [
    Command::Close,
    Command::Undo,
    Command::Redo,
    Command::Copy,
    Command::SelectAll,
    Command::Pen,
    Command::ShowMarks,
    Command::DeleteMark,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ZoomFit,
    Command::PreviousPage,
    Command::NextPage,
    Command::Sidebar,
    Command::Chat,
    Command::Shortcuts,
];

impl Command {
    /// What the menu item says when it is picked.
    pub fn id(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Copy => "copy",
            Self::SelectAll => "select_all",
            Self::Pen => "pen",
            Self::ShowMarks => "show_marks",
            Self::DeleteMark => "delete_mark",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::ZoomFit => "zoom_fit",
            Self::PreviousPage => "previous_page",
            Self::NextPage => "next_page",
            Self::Sidebar => "sidebar",
            Self::Chat => "chat",
            Self::Shortcuts => "shortcuts",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ALL.into_iter().find(|command| command.id() == id)
    }

    pub fn label(self) -> String {
        match self {
            Self::Close => otto_kit::t_owned!("studio-close"),
            Self::Undo => otto_kit::t_owned!("studio-undo"),
            Self::Redo => otto_kit::t_owned!("studio-redo"),
            Self::Copy => otto_kit::t_owned!("studio-copy"),
            Self::SelectAll => otto_kit::t_owned!("studio-select-all"),
            Self::Pen => otto_kit::t_owned!("studio-pen"),
            Self::ShowMarks => otto_kit::t_owned!("studio-show-marks"),
            Self::DeleteMark => otto_kit::t_owned!("studio-delete-mark"),
            Self::ZoomIn => otto_kit::t_owned!("studio-zoom-in"),
            Self::ZoomOut => otto_kit::t_owned!("studio-zoom-out"),
            Self::ZoomFit => otto_kit::t_owned!("studio-zoom-fit"),
            Self::PreviousPage => otto_kit::t_owned!("studio-previous-page"),
            Self::NextPage => otto_kit::t_owned!("studio-next-page"),
            Self::Sidebar => otto_kit::t_owned!("studio-sidebar"),
            Self::Chat => otto_kit::t_owned!("studio-chat"),
            Self::Shortcuts => otto_kit::t_owned!("studio-shortcuts"),
        }
    }

    /// Its key, as the menu and the sheet write it. Cmd works wherever Ctrl
    /// does: Otto hands it to apps as either.
    pub fn shortcut(self) -> Option<&'static str> {
        Some(match self {
            Self::Close => "Ctrl+W",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Y",
            Self::Copy => "Ctrl+C",
            Self::SelectAll => "Ctrl+A",
            Self::Pen => "Ctrl+Shift+A",
            Self::ShowMarks => "Ctrl+Shift+H",
            Self::DeleteMark => "Backspace",
            Self::ZoomIn => "Ctrl++",
            Self::ZoomOut => "Ctrl+-",
            Self::ZoomFit => "Ctrl+0",
            Self::PreviousPage => "PageUp",
            Self::NextPage => "PageDown",
            Self::Sidebar => return None,
            Self::Chat => "Ctrl+K",
            Self::Shortcuts => "Ctrl+/",
        })
    }

    /// The toolbar button that does the same, run through it so a menu and
    /// a click cannot differ.
    pub fn tool(self) -> Option<Tool> {
        Some(match self {
            Self::Undo => Tool::Undo,
            Self::Redo => Tool::Redo,
            Self::Pen => Tool::Mark,
            Self::ShowMarks => Tool::ShowMarks,
            Self::ZoomIn => Tool::ZoomIn,
            Self::ZoomOut => Tool::ZoomOut,
            Self::ZoomFit => Tool::ZoomFit,
            Self::PreviousPage => Tool::PreviousPage,
            Self::NextPage => Tool::NextPage,
            Self::Sidebar => Tool::Sidebar,
            Self::Chat => Tool::Chat,
            _ => return None,
        })
    }

    /// Whether it can run on `viewer` now.
    fn enabled(self, viewer: &Viewer) -> bool {
        match self {
            Self::Copy => viewer.has_selection(),
            Self::DeleteMark => !viewer.marks.pending().is_empty(),
            Self::Close | Self::SelectAll | Self::Shortcuts => true,
            command => command.tool().is_none_or(|tool| viewer.tool_enabled(tool)),
        }
    }

    /// Whether it is on, for the ones that are toggles.
    fn checked(self, viewer: &Viewer) -> Option<bool> {
        match self {
            Self::Pen => Some(viewer.marking),
            Self::ShowMarks => Some(!viewer.marks_hidden),
            Self::Sidebar => Some(viewer.sidebar_open()),
            Self::Chat => Some(viewer.chat_open),
            _ => None,
        }
    }

    fn entry(self, viewer: &Viewer) -> MenuEntry {
        let mut entry = MenuEntry::item(self.id(), self.label()).enabled(self.enabled(viewer));
        if let Some(shortcut) = self.shortcut() {
            entry = entry.with_shortcut(shortcut);
        }
        if let Some(checked) = self.checked(viewer) {
            entry = entry.checked(checked);
        }
        entry
    }
}

/// The menus as they read for `viewer` now.
pub fn menus(viewer: &Viewer) -> Vec<Menu> {
    sections()
        .into_iter()
        .map(|(title, groups)| {
            let mut items = Vec::new();
            for group in groups {
                if !items.is_empty() {
                    items.push(MenuEntry::Separator);
                }
                items.extend(group.into_iter().map(|command| command.entry(viewer)));
            }
            Menu::new(title, items)
        })
        .collect()
}

/// The menus' titles and their commands, in groups split by separators.
fn sections() -> Vec<(String, Vec<Vec<Command>>)> {
    use Command::*;
    vec![
        (otto_kit::t_owned!("studio-menu-file"), vec![vec![Close]]),
        (
            otto_kit::t_owned!("studio-menu-edit"),
            vec![vec![Undo, Redo], vec![Copy, SelectAll]],
        ),
        (
            otto_kit::t_owned!("studio-menu-marks"),
            vec![vec![Pen, ShowMarks], vec![DeleteMark]],
        ),
        (
            otto_kit::t_owned!("studio-menu-view"),
            vec![
                vec![ZoomIn, ZoomOut, ZoomFit],
                vec![PreviousPage, NextPage],
                vec![Sidebar, Chat],
            ],
        ),
        (
            otto_kit::t_owned!("studio-menu-help"),
            vec![vec![Shortcuts]],
        ),
    ]
}

/// The command a key runs from anywhere in the window, chat or not; the
/// document's own keys (scrolling, zoom, copy) stay with the viewer.
pub fn global_key(keysym: Keysym, modifiers: Modifiers) -> Option<Command> {
    let ctrl = modifiers.ctrl || modifiers.logo;
    if !ctrl || modifiers.alt {
        return None;
    }
    match (keysym, modifiers.shift) {
        (Keysym::k | Keysym::K, false) => Some(Command::Chat),
        (Keysym::a | Keysym::A, true) => Some(Command::Pen),
        (Keysym::h | Keysym::H, true) => Some(Command::ShowMarks),
        (Keysym::slash | Keysym::question, _) => Some(Command::Shortcuts),
        _ => None,
    }
}

/// The command a key runs with the document focused: the file's versions.
pub fn document_key(keysym: Keysym, modifiers: Modifiers) -> Option<Command> {
    let ctrl = modifiers.ctrl || modifiers.logo;
    if !ctrl || modifiers.alt {
        return None;
    }
    match (keysym, modifiers.shift) {
        (Keysym::z | Keysym::Z, false) => Some(Command::Undo),
        (Keysym::z | Keysym::Z, true) | (Keysym::y | Keysym::Y, false) => Some(Command::Redo),
        _ => None,
    }
}

/// What the keyboard shortcuts sheet lists: every menu's commands with a
/// key, then the keys that are no menu's.
pub fn sheet() -> Vec<(String, Vec<(String, String)>)> {
    let mut sheet: Vec<(String, Vec<(String, String)>)> = sections()
        .into_iter()
        .map(|(title, groups)| {
            let rows = groups
                .into_iter()
                .flatten()
                .filter_map(|command| Some((command.label(), command.shortcut()?.to_owned())))
                .collect();
            (title, rows)
        })
        .filter(|(_, rows): &(String, Vec<_>)| !rows.is_empty())
        .collect();
    // Redo has a second key the menu has no room for.
    let redo = Command::Redo.label();
    for (_, rows) in &mut sheet {
        for (label, keys) in rows.iter_mut() {
            if *label == redo {
                keys.push_str(" · Ctrl+Shift+Z");
            }
        }
    }
    sheet.push((
        otto_kit::t_owned!("studio-sheet-document"),
        vec![
            (
                otto_kit::t_owned!("studio-sheet-scroll"),
                "↑ ↓ ← →".to_owned(),
            ),
            (otto_kit::t_owned!("studio-sheet-page"), "Space".to_owned()),
            (
                otto_kit::t_owned!("studio-sheet-ends"),
                "Home End".to_owned(),
            ),
            (
                otto_kit::t_owned!("studio-sheet-zoom-pointer"),
                "Ctrl+Scroll".to_owned(),
            ),
            (otto_kit::t_owned!("studio-sheet-stop"), "Esc".to_owned()),
        ],
    ));
    sheet.push((
        otto_kit::t_owned!("studio-sheet-chat"),
        vec![
            (otto_kit::t_owned!("studio-sheet-send"), "Enter".to_owned()),
            (
                otto_kit::t_owned!("studio-sheet-dictate"),
                "Ctrl+D".to_owned(),
            ),
            (
                otto_kit::t_owned!("studio-sheet-stop-agent"),
                "Ctrl+C".to_owned(),
            ),
            (
                otto_kit::t_owned!("studio-sheet-to-document"),
                "Esc".to_owned(),
            ),
        ],
    ));
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool, logo: bool) -> Modifiers {
        Modifiers {
            ctrl,
            shift,
            logo,
            ..Modifiers::default()
        }
    }

    #[test]
    fn every_command_reads_back_from_its_id() {
        for command in ALL {
            assert_eq!(Command::from_id(command.id()), Some(command));
        }
    }

    #[test]
    fn undo_and_redo_answer_ctrl_and_cmd() {
        for (ctrl, logo) in [(true, false), (false, true)] {
            assert_eq!(
                document_key(Keysym::z, mods(ctrl, false, logo)),
                Some(Command::Undo)
            );
            assert_eq!(
                document_key(Keysym::Z, mods(ctrl, true, logo)),
                Some(Command::Redo)
            );
            assert_eq!(
                document_key(Keysym::y, mods(ctrl, false, logo)),
                Some(Command::Redo)
            );
        }
        assert_eq!(document_key(Keysym::z, mods(false, false, false)), None);
    }

    #[test]
    fn the_pen_the_marks_the_chat_and_the_sheet_have_keys() {
        let ctrl_shift = mods(true, true, false);
        assert_eq!(global_key(Keysym::A, ctrl_shift), Some(Command::Pen));
        assert_eq!(global_key(Keysym::H, ctrl_shift), Some(Command::ShowMarks));
        assert_eq!(
            global_key(Keysym::k, mods(false, false, true)),
            Some(Command::Chat)
        );
        assert_eq!(
            global_key(Keysym::slash, mods(true, false, false)),
            Some(Command::Shortcuts)
        );
        // Ctrl+A without Shift is the document's select all.
        assert_eq!(global_key(Keysym::a, mods(true, false, false)), None);
    }

    #[test]
    fn every_key_a_menu_shows_reads_as_a_shortcut() {
        for command in ALL {
            if let Some(shortcut) = command.shortcut() {
                assert!(
                    otto_kit::app_menu::Shortcut::parse(shortcut).is_some(),
                    "{shortcut}"
                );
            }
        }
    }
}
