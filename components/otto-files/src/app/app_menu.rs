//! Files' menus in the top bar: its commands, a menu per group, as the
//! palette lists them for where the window is now.
//!
//! The View menu leads with the views as checks, the one on ticked. A
//! command whose argument is one of a few choices (Sort By, Go to Place) is a
//! submenu of them; one that needs something typed opens the palette in its
//! field, as the context menu does.

use otto_kit::app_menu::{Menu, MenuEntry};

use super::*;

/// Between a command's id and the choice it is given, in a menu item's id.
const CHOICE: char = '\u{1f}';

/// The views, with the keys that switch to them.
const VIEWS: [(&str, &str); 4] = [
    (command::id::VIEW_LIST, "Ctrl+1"),
    (command::id::VIEW_GRID, "Ctrl+2"),
    (command::id::VIEW_COLUMNS, "Ctrl+3"),
    (command::id::VIEW_PHOTOS, "Ctrl+4"),
];

impl Browser {
    /// The menus as they read here and now.
    pub(super) fn app_menus(&self) -> Vec<Menu> {
        let situation = self.situation();
        let commands = self.commands.commands(&situation);
        command::Group::ALL
            .into_iter()
            .map(|group| {
                let mut items = Vec::new();
                if group == command::Group::View {
                    for choice in command::view_choices() {
                        let key = VIEWS
                            .iter()
                            .find(|(id, _)| *id == choice.value)
                            .map(|(_, key)| *key);
                        let mut entry = MenuEntry::item(choice.value.clone(), choice.title)
                            .checked(situation.view == choice.value);
                        if let Some(key) = key {
                            entry = entry.with_shortcut(key);
                        }
                        items.push(entry);
                    }
                    items.push(MenuEntry::Separator);
                }
                for command in commands.iter().filter(|command| command.group == group) {
                    // The views are the checks above.
                    let is_view = VIEWS.iter().any(|(id, _)| *id == command.id)
                        || command.id == command::id::CHANGE_VIEW;
                    if is_view {
                        continue;
                    }
                    items.push(entry(command, &situation));
                }
                if items.last() == Some(&MenuEntry::Separator) {
                    items.pop();
                }
                Menu::new(group.label(), items)
            })
            .filter(|menu| !menu.items.is_empty())
            .collect()
    }

    /// Run what was picked from a menu.
    pub(super) fn run_app_menu_pick(&mut self, id: &str, serial: u32) {
        let result = match id.split_once(CHOICE) {
            Some((id, value)) => self
                .run_request(&command::Request::new(id, Some(value.to_owned())), serial)
                .map(Some),
            None if id == command::id::QUICK_LOOK => Ok(Some(Followup::Peek)),
            None => {
                self.run_menu_command(id, serial);
                Ok(None)
            }
        };
        match result {
            Ok(Some(Followup::Peek)) => self.palette_peek = true,
            Ok(_) => {}
            Err(error) => self.status = Some(error),
        }
        self.dirty = true;
    }
}

/// `command` as a menu item: a submenu of its choices when it takes one, the
/// one in force checked.
fn entry(command: &command::Command, situation: &command::Situation) -> MenuEntry {
    if let Some(choices) = command.arg.as_ref().and_then(command::ArgSpec::choices) {
        let current = (command.id == command::id::SORT_BY).then_some(situation.sort.as_str());
        let items = choices
            .iter()
            .map(|choice| {
                let item = MenuEntry::item(
                    format!("{}{CHOICE}{}", command.id, choice.value),
                    choice.title.clone(),
                );
                match current {
                    Some(current) => item.checked(current == choice.value),
                    None => item,
                }
            })
            .collect();
        return MenuEntry::Submenu(Menu::new(command.title.clone(), items));
    }
    let label = if command.arg.is_some() {
        format!("{}…", command.title)
    } else {
        command.title.clone()
    };
    let mut item = MenuEntry::item(command.id.clone(), label);
    if let Some(shortcut) = &command.shortcut {
        item = item.with_shortcut(shortcut);
    }
    item
}

#[cfg(test)]
mod tests {
    use super::super::typeahead_tests::browser_over;
    use super::*;

    fn titles(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item.id.clone()),
                MenuEntry::Submenu(menu) => Some(format!("[{}]", menu.title)),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn the_menus_are_the_commands_by_group_with_the_view_checked() {
        let (browser, _dir) = browser_over(&["a.txt", "b"]);
        let menus = browser.app_menus();
        let names: Vec<_> = menus.iter().map(|menu| menu.title.clone()).collect();
        assert_eq!(
            names,
            command::Group::ALL
                .into_iter()
                .map(command::Group::label)
                .collect::<Vec<_>>()
        );
        let view = menus.last().unwrap();
        let checked: Vec<_> = view
            .items
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) if item.checked == Some(true) => Some(item.id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(checked, [browser.situation().view.as_str()]);
        // Every view is there once, current or not.
        let view_items = titles(view);
        for (id, _) in VIEWS {
            assert_eq!(view_items.iter().filter(|item| *item == id).count(), 1);
        }
    }

    #[test]
    fn a_choice_is_a_submenu_and_its_pick_runs_with_it() {
        let (mut browser, _dir) = browser_over(&["a.txt", "b"]);
        let menus = browser.app_menus();
        let sort = menus
            .iter()
            .flat_map(|menu| &menu.items)
            .find_map(|entry| {
                match entry {
                MenuEntry::Submenu(menu)
                    if menu.items.iter().any(|item| {
                        matches!(item, MenuEntry::Item(item) if item.id.starts_with("sort_by"))
                    }) =>
                {
                    Some(menu.clone())
                }
                _ => None,
            }
            })
            .expect("sort by is a submenu");
        let by_size = sort
            .items
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.id.ends_with("size") => Some(item.id.clone()),
                _ => None,
            })
            .unwrap();
        browser.run_app_menu_pick(&by_size, 0);
        assert_eq!(browser.situation().sort, "size");
    }
}
