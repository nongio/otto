use super::typeahead_tests::browser_over;
use super::*;

fn open_and_type(browser: &mut Browser, text: &str) {
    browser.open_palette();
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    for ch in text.chars() {
        let palette = browser.palette.as_mut().expect("palette is open");
        palette.on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
    }
}

fn press(browser: &mut Browser, key: palette::Key) -> palette::Outcome {
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    let outcome = browser
        .palette
        .as_mut()
        .expect("palette is open")
        .on_key(key, mods);
    if outcome == palette::Outcome::Changed {
        browser.refresh_palette_completions();
    }
    outcome
}

/// Running a command through the palette is running it: the same rename
/// the in-place field does, on the same undo stack.
#[test]
fn a_command_run_from_the_palette_does_what_the_menu_does() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    browser.select(0, 0);
    open_and_type(&mut browser, "rename");
    assert_eq!(
        press(&mut browser, palette::Key::Tab),
        palette::Outcome::Changed
    );
    // The field starts on the name the file already has.
    assert_eq!(browser.palette.as_ref().unwrap().input().value(), "a.txt");
    browser
        .palette
        .as_mut()
        .unwrap()
        .input_mut()
        .set_value("b.txt");
    let palette::Outcome::Run(request) = press(&mut browser, palette::Key::Enter) else {
        panic!("Return should run the rename");
    };
    browser.run_request(&request, 0).expect("the rename works");
    assert!(dir.0.join("b.txt").exists());
    assert!(!dir.0.join("a.txt").exists());
    assert_eq!(browser.undo.len(), 1);
}

/// A command that cannot run is not on the list at all, so nothing has to
/// be greyed out and the arrow keys have nothing to skip.
#[test]
fn the_palette_offers_only_what_can_run_now() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.open_palette();
    let offered: Vec<&str> = browser
        .palette
        .as_ref()
        .unwrap()
        .commands()
        .iter()
        .map(|command| command.id.as_str())
        .collect();
    assert!(!offered.contains(&command::id::UNDO));
    assert!(!offered.contains(&command::id::GO_BACK));
    assert!(offered.contains(&command::id::NEW_FOLDER));
}

/// The whole point of the path argument: type a fragment, Tab, arrive.
#[test]
fn go_to_path_navigates_where_the_argument_says() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    std::fs::create_dir(dir.0.join("inner")).unwrap();
    open_and_type(&mut browser, "go to path");
    press(&mut browser, palette::Key::Tab);
    browser
        .palette
        .as_mut()
        .unwrap()
        .input_mut()
        .set_value("inner");
    let palette::Outcome::Run(request) = press(&mut browser, palette::Key::Enter) else {
        panic!("Return should go there");
    };
    browser
        .run_request(&request, 0)
        .expect("the folder is there");
    assert_eq!(browser.current_path(), dir.0.join("inner"));
}

/// A path that is not there is a refusal, not a navigation — and the
/// palette stays open around it.
#[test]
fn a_path_that_is_not_there_is_refused() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    open_and_type(&mut browser, "go to path");
    press(&mut browser, palette::Key::Tab);
    browser
        .palette
        .as_mut()
        .unwrap()
        .input_mut()
        .set_value("nowhere");
    let palette::Outcome::Run(request) = press(&mut browser, palette::Key::Enter) else {
        panic!("Return should try");
    };
    let error = browser
        .run_request(&request, 0)
        .expect_err("no such folder");
    browser.palette.as_mut().unwrap().set_error(error);
    assert!(browser.palette.as_ref().unwrap().error().is_some());
    assert_eq!(browser.current_path(), dir.0);
}

/// The host completes a path against the disk, since the palette may not.
#[test]
fn the_host_completes_a_path_against_the_disk() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    std::fs::create_dir(dir.0.join("shared")).unwrap();
    std::fs::create_dir(dir.0.join("shrunk")).unwrap();
    open_and_type(&mut browser, "go to path");
    press(&mut browser, palette::Key::Tab);
    browser
        .palette
        .as_mut()
        .unwrap()
        .input_mut()
        .set_value("sh");
    browser.refresh_palette_completions();
    assert_eq!(browser.palette.as_ref().unwrap().completions().len(), 2);
    // Tab extends as far as both answers agree.
    press(&mut browser, palette::Key::Tab);
    assert_eq!(browser.palette.as_ref().unwrap().input().value(), "sh");
    press(&mut browser, palette::Key::Down);
    press(&mut browser, palette::Key::Tab);
    assert_eq!(browser.palette.as_ref().unwrap().input().value(), "shared/");
}

/// A view is a choice, so it is answered with an arrow key rather than by
/// typing the name of one.
#[test]
fn change_view_is_answered_from_a_list() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.set_mode(ViewMode::List);
    open_and_type(&mut browser, "change view");
    press(&mut browser, palette::Key::Tab);
    // Opens on the view that is already current.
    let palette::Outcome::Run(request) = press(&mut browser, palette::Key::Enter) else {
        panic!("Return should take the highlighted view");
    };
    assert_eq!(request.arg.as_deref(), Some("list"));
    browser.run_request(&request, 0).unwrap();
    assert_eq!(browser.mode, ViewMode::List);
}

/// Named folders come through the palette; the toolbar's unnamed one still
/// lands in rename.
#[test]
fn new_folder_takes_the_name_it_was_given() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    browser
        .new_folder_named("reports")
        .expect("the name is free");
    assert!(dir.0.join("reports").is_dir());
    // A second one with the same name is an error, not a "reports 2".
    assert!(browser.new_folder_named("reports").is_err());
}

/// A pattern selects what is on screen, and nothing else.
#[test]
fn the_path_bar_counts_the_selection() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.png", "c.txt"]);
    browser.clear_selection();
    assert_eq!(browser.path_bar_note(), None);
    browser.select_matching("*.png").unwrap();
    assert_eq!(
        browser.path_bar_note().as_deref(),
        Some(otto_kit::t_owned!("files-status-selected", count = 2, total = 3).as_str())
    );
}

/// Typing a pattern says what it is picking as it goes, and says plainly
/// when it picks nothing — in the palette, not as an error.
#[test]
fn a_previewed_pattern_reports_its_count_in_the_palette() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.png", "c.txt"]);
    browser.clear_selection();
    browser.open_palette();
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    for ch in "select m".chars() {
        browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
    }
    browser
        .palette
        .as_mut()
        .unwrap()
        .on_key(palette::Key::Tab, mods);
    assert!(browser.palette.as_ref().unwrap().prompt().is_some());
    for ch in "*.png".chars() {
        let outcome = browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
        browser.settle_palette(outcome, 0);
    }
    assert_eq!(
        browser.palette_message().as_deref(),
        Some(otto_kit::t_owned!("files-status-selected", count = 2, total = 3).as_str())
    );
    for ch in "x".chars() {
        let outcome = browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
        browser.settle_palette(outcome, 0);
    }
    assert_eq!(
        browser.palette_message().as_deref(),
        Some(otto_kit::t_owned!("files-nothing-matches", pattern = "*.pngx").as_str())
    );
    assert!(browser.palette.as_ref().unwrap().error().is_none());
}

/// The whole rename, through the palette: the command is offered for the
/// selection, the dry run appears line by line as the pattern is typed
/// with its summary under it, a taken name is called out and refused, and
/// Return renames everything in one undo step.
#[test]
fn renaming_a_selection_shows_its_dry_run_and_then_does_it() {
    let (mut browser, dir) =
        browser_over(&["IMG_001.jpg", "IMG_002.jpg", "Holiday 2.jpg", "notes.txt"]);
    browser.clear_selection();
    browser.select_matching("IMG*").unwrap();
    browser.open_palette();
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    let type_in = |browser: &mut Browser, text: &str| {
        for ch in text.chars() {
            let outcome = browser
                .palette
                .as_mut()
                .unwrap()
                .on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
            browser.settle_palette(outcome, 0);
        }
    };
    type_in(&mut browser, "rename 2");
    let outcome = browser
        .palette
        .as_mut()
        .unwrap()
        .on_key(palette::Key::Tab, mods);
    browser.settle_palette(outcome, 0);
    assert!(browser.palette.as_ref().unwrap().prompt().is_some());

    // The initial `{name}` is selected whole; typing replaces it.
    type_in(&mut browser, "Holiday {n}");
    let rows = browser.palette_rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "Holiday 1.jpg");
    assert_eq!(rows[0].subtitle.as_deref(), Some("IMG_001.jpg"));
    assert_eq!(
        rows[0].kind,
        view::PaletteRowKind::Preview {
            conflict: false,
            excluded: false
        }
    );
    assert_eq!(
        rows[1].kind,
        view::PaletteRowKind::Preview {
            conflict: true,
            excluded: false
        },
        "Holiday 2.jpg is already in the folder"
    );
    assert_eq!(
        browser.palette_message().as_deref(),
        Some(otto_kit::t_owned!("files-rename-conflicts", count = 1).as_str())
    );
    // Return refuses while a name is taken, and nothing has moved.
    let outcome = browser
        .palette
        .as_mut()
        .unwrap()
        .on_key(palette::Key::Enter, mods);
    browser.settle_palette(outcome, 0);
    assert!(browser.palette.is_some());
    assert!(dir.0.join("IMG_001.jpg").exists());

    // A pattern that clears: everything renamed, one undo step.
    for _ in 0..3 {
        let outcome = browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Backspace), mods);
        browser.settle_palette(outcome, 0);
    }
    type_in(&mut browser, "{n@5}");
    assert_eq!(
        browser.palette_message().as_deref(),
        Some(otto_kit::t_owned!("files-rename-preview", count = 2, total = 2).as_str())
    );
    let outcome = browser
        .palette
        .as_mut()
        .unwrap()
        .on_key(palette::Key::Enter, mods);
    browser.settle_palette(outcome, 0);
    assert!(browser.palette.is_none(), "a clean run closes the palette");
    assert!(dir.0.join("Holiday 5.jpg").exists());
    assert!(dir.0.join("Holiday 6.jpg").exists());
    assert!(!dir.0.join("IMG_001.jpg").exists());
    let undos = browser.undo.len();
    browser.undo_last();
    assert_eq!(browser.undo.len(), undos - 1);
    assert!(dir.0.join("IMG_001.jpg").exists());
    assert!(dir.0.join("IMG_002.jpg").exists());
}

/// Recent reached from the palette — by name or as the place — takes a
/// search down with it, the way the sidebar already did.
#[test]
fn going_to_recent_from_the_palette_closes_the_search() {
    for id in [command::id::RECENT, command::id::GO_TO_PLACE] {
        let (mut browser, _dir) = browser_over(&["a.txt"]);
        browser.searching = true;
        view::set_search_band(true);
        let arg = (id == command::id::GO_TO_PLACE).then(|| crate::recent::SENTINEL.to_string());
        browser
            .run_request(&command::Request::new(id, arg), 0)
            .unwrap();
        assert!(browser.recent, "{id}: in Recent");
        assert!(!browser.searching, "{id}: the search came down");
        assert!(browser.search.is_none(), "{id}: no search field either");
        assert!(!browser.back.is_empty(), "{id}: the search is behind Back");
    }
}

/// The right-click menu lists what the providers offer, and picking one
/// that takes an argument lands in the palette's field with the dry run
/// already showing.
#[test]
fn the_context_menu_offers_provider_commands_through_the_palette() {
    let (mut browser, _dir) = browser_over(&["a.jpg", "b.jpg"]);
    browser.clear_selection();
    browser.select_matching("*.jpg").unwrap();
    let items = browser.context_menu_items(-1.0, -1.0);
    let labels: Vec<String> = items
        .iter()
        .filter_map(|item| match &item.kind {
            otto_kit::prelude::MenuItemKind::Action { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect();
    let rename = otto_kit::t_owned!("files-rename-many", count = 2);
    assert!(
        labels.iter().any(|l| l == &format!("{rename}…")),
        "{labels:?}"
    );

    browser.run_menu_command(rename::RENAME_MANY, 0);
    let palette = browser.palette.as_ref().expect("the palette opened");
    assert!(palette.prompt().is_some(), "in the argument field");
    assert_eq!(palette.input().value(), "{name}");
    assert_eq!(browser.palette_rows().len(), 2, "the dry run is showing");
}

/// Down into the dry run and Space leaves a file out; the dry run is
/// made again without it, and so is the run. A click on a line does the
/// same, and a space typed in the field is still a space.
#[test]
fn a_dry_run_line_can_be_toggled_out_of_the_run_and_back() {
    let (mut browser, dir) = browser_over(&["a.jpg", "b.jpg", "c.jpg"]);
    browser.clear_selection();
    browser.select_matching("*.jpg").unwrap();
    browser.open_palette();
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    let press = |browser: &mut Browser, key: palette::Key| {
        let outcome = browser.palette.as_mut().unwrap().on_key(key, mods);
        browser.settle_palette(outcome, 0);
    };
    let type_in = |browser: &mut Browser, text: &str| {
        for ch in text.chars() {
            press(browser, palette::Key::Edit(TextInputKey::Char(ch)));
        }
    };
    type_in(&mut browser, "rename 3");
    press(&mut browser, palette::Key::Tab);
    type_in(&mut browser, "Pic {n}");
    let titles = |browser: &Browser| -> Vec<(String, String, bool)> {
        browser
            .palette_rows()
            .into_iter()
            .map(|row| {
                let excluded = matches!(
                    row.kind,
                    view::PaletteRowKind::Preview { excluded: true, .. }
                );
                (row.subtitle.unwrap_or_default(), row.title, excluded)
            })
            .collect()
    };
    assert_eq!(
        titles(&browser),
        vec![
            ("a.jpg".into(), "Pic 1.jpg".into(), false),
            ("b.jpg".into(), "Pic 2.jpg".into(), false),
            ("c.jpg".into(), "Pic 3.jpg".into(), false),
        ]
    );

    // Down onto the first line, down again, Space: b is out and c takes
    // its number.
    press(&mut browser, palette::Key::Down);
    press(&mut browser, palette::Key::Down);
    assert_eq!(browser.palette.as_ref().unwrap().highlighted(), Some(1));
    press(&mut browser, palette::Key::Edit(TextInputKey::Char(' ')));
    assert_eq!(
        titles(&browser),
        vec![
            ("a.jpg".into(), "Pic 1.jpg".into(), false),
            ("b.jpg".into(), String::new(), true),
            ("c.jpg".into(), "Pic 2.jpg".into(), false),
        ]
    );
    assert!(
        browser.palette_rows()[1].highlighted,
        "the highlight stays put"
    );
    assert_eq!(
        browser.palette_message().as_deref(),
        Some(otto_kit::t_owned!("files-rename-preview", count = 2, total = 2).as_str())
    );

    // Space again brings it back; a click on the third line takes that
    // one out instead.
    press(&mut browser, palette::Key::Edit(TextInputKey::Char(' ')));
    assert!(!titles(&browser)[1].2);
    assert!(browser.palette.as_mut().unwrap().toggle_row(2));
    browser.preview_palette_argument();
    assert_eq!(titles(&browser)[2], ("c.jpg".into(), String::new(), true));

    // Typing goes back to the field, where a space is a space.
    press(&mut browser, palette::Key::Edit(TextInputKey::Char('s')));
    press(&mut browser, palette::Key::Edit(TextInputKey::Char(' ')));
    assert_eq!(
        browser.palette.as_ref().unwrap().input().value(),
        "Pic {n}s "
    );
    assert!(titles(&browser)[2].2, "what was toggled out stays out");

    // Return renames the two that are in, and c is untouched.
    for _ in 0..2 {
        press(&mut browser, palette::Key::Edit(TextInputKey::Backspace));
    }
    press(&mut browser, palette::Key::Enter);
    assert!(browser.palette.is_none());
    assert!(dir.0.join("Pic 1.jpg").exists());
    assert!(dir.0.join("Pic 2.jpg").exists());
    assert!(dir.0.join("c.jpg").exists());
    assert!(!dir.0.join("Pic 3.jpg").exists());
}

#[test]
fn select_matching_picks_the_files_the_pattern_names() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.png", "c.txt", "D.PNG"]);
    browser.select_matching("*.png").expect("three match");
    let selected: Vec<String> = browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    // A lowercase pattern also takes the shouty extension.
    assert_eq!(selected, vec!["a.png", "b.png", "D.PNG"]);
}

/// A pattern carrying case means that case.
#[test]
fn a_pattern_with_case_in_it_is_matched_with_case() {
    let (mut browser, _dir) = browser_over(&["a.png", "D.PNG"]);
    browser.select_matching("*.PNG").expect("one matches");
    let selected: Vec<String> = browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(selected, vec!["D.PNG"]);
}

/// A pattern matching nothing is a refusal, and leaves the selection be.
#[test]
fn a_pattern_matching_nothing_is_refused() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.select(0, 0);
    assert!(browser.select_matching("*.png").is_err());
    assert_eq!(browser.selected_entries().len(), 1);
}

/// Moving the selection is the move a cut and paste would make.
#[test]
fn move_to_folder_moves_the_selection() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    std::fs::create_dir(dir.0.join("inbox")).unwrap();
    browser.select(0, 0);
    browser
        .move_selection_to("inbox")
        .expect("the folder is there");
    assert!(dir.0.join("inbox/a.txt").is_file());
    assert!(!dir.0.join("a.txt").exists());
    assert_eq!(browser.undo.len(), 1);
}

/// A folder cannot be moved inside itself: the subtree would go somewhere
/// it can no longer be reached from.
#[test]
fn a_folder_is_not_moved_into_itself() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    std::fs::create_dir_all(dir.0.join("outer/inner")).unwrap();
    browser.reload_all();
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let outer = browser
        .visible(0)
        .iter()
        .position(|entry| entry.name == "outer")
        .expect("outer is listed");
    browser.select(0, outer);
    assert!(browser.move_selection_to("outer/inner").is_err());
    assert!(dir.0.join("outer/inner").is_dir());
}

/// A previewed pattern selects as it is typed.
#[test]
fn select_matching_shows_its_answer_while_it_is_typed() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.png", "c.txt"]);
    open_and_type(&mut browser, "select matching");
    press(&mut browser, palette::Key::Tab);

    for ch in "*.png".chars() {
        browser.palette.as_mut().unwrap().on_key(
            palette::Key::Edit(TextInputKey::Char(ch)),
            KeyMods {
                shift: false,
                ctrl: false,
            },
        );
        browser.preview_palette_argument();
    }
    let selected: Vec<String> = browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(selected, vec!["a.png", "b.png"]);
}

/// Deleting back through the pattern widens the answer again, rather than
/// leaving the narrower one standing.
#[test]
fn deleting_through_a_pattern_gives_the_selection_back() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.txt"]);
    open_and_type(&mut browser, "select matching");
    press(&mut browser, palette::Key::Tab);
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    for ch in "*.png".chars() {
        browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
        browser.preview_palette_argument();
    }
    assert_eq!(browser.selected_entries().len(), 1);

    // All the way back to an empty pattern: nothing is selected again.
    for _ in 0..5 {
        browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Backspace), mods);
        browser.preview_palette_argument();
    }
    assert_eq!(browser.selected_entries().len(), 0);
}

/// And abandoning the palette puts back whatever was selected before it
/// opened — the preview was shown, not done.
#[test]
fn abandoning_a_preview_puts_the_selection_back() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.png", "c.txt"]);
    browser.select(0, 2);
    let before: Vec<String> = browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.name)
        .collect();

    open_and_type(&mut browser, "select matching");
    press(&mut browser, palette::Key::Tab);
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    for ch in "*.png".chars() {
        browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Edit(TextInputKey::Char(ch)), mods);
        browser.preview_palette_argument();
    }
    assert_eq!(browser.selected_entries().len(), 2);

    browser.close_palette();
    let after: Vec<String> = browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(after, before);
}

/// Running it keeps what it selected.
#[test]
fn running_a_preview_keeps_its_answer() {
    let (mut browser, _dir) = browser_over(&["a.png", "b.png", "c.txt"]);
    open_and_type(&mut browser, "select matching");
    press(&mut browser, palette::Key::Tab);
    browser
        .palette
        .as_mut()
        .unwrap()
        .input_mut()
        .set_value("*.png");
    let palette::Outcome::Run(request) = press(&mut browser, palette::Key::Enter) else {
        panic!("Return should run it");
    };
    browser.run_request(&request, 0).expect("two match");
    browser.finish_palette();
    assert_eq!(browser.selected_entries().len(), 2);
}

/// A free-text argument has no completions, and that is not a failure to
/// find anything: the panel must not say "no command matches" about a
/// pattern being typed.
#[test]
fn a_text_argument_is_not_reported_as_matching_nothing() {
    let (mut browser, _dir) = browser_over(&["a.png"]);
    open_and_type(&mut browser, "select matching");
    assert!(browser.palette_message().is_none());
    press(&mut browser, palette::Key::Tab);
    assert!(browser.palette.as_ref().unwrap().prompt().is_some());
    assert!(
        browser.palette_message().is_none(),
        "an argument with no completions says nothing"
    );

    // A query that really matches no command still says so.
    browser.close_palette();
    open_and_type(&mut browser, "zzzz");
    assert!(browser.palette_message().is_some());
}

/// The selection goes into a folder made for it, and one undo takes both
/// halves back — the files first, the folder after.
#[test]
fn new_folder_with_selection_gathers_the_files() {
    let (mut browser, dir) = browser_over(&["a.txt", "b.txt", "c.txt"]);
    browser.select(0, 0);
    let second = browser.visible(0)[1].selection_key();
    browser.columns[0].selection.insert(second);
    browser
        .new_folder_with_selection("reports")
        .expect("the name is free");

    assert!(dir.0.join("reports/a.txt").is_file());
    assert!(dir.0.join("reports/b.txt").is_file());
    assert!(dir.0.join("c.txt").is_file());
    assert!(!dir.0.join("a.txt").exists());
    assert_eq!(browser.undo.len(), 1);

    browser.undo_last();
    assert!(dir.0.join("a.txt").is_file());
    assert!(dir.0.join("b.txt").is_file());
    assert!(!dir.0.join("reports").exists(), "the empty folder goes too");
}

/// With nothing selected there is nothing to gather, and no folder is left
/// behind by the attempt.
#[test]
fn new_folder_with_selection_needs_a_selection() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    browser.clear_selection();
    assert!(browser.new_folder_with_selection("reports").is_err());
    assert!(!dir.0.join("reports").exists());
}

/// The top band takes hold of the card; a row is picked, not grabbed.
#[test]
fn the_top_band_drags_and_the_rows_pick() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    let card = browser.palette_card();

    browser.palette_press(card.center_x(), card.top + 4.0, 0);
    assert!(
        browser.palette_drag.is_some(),
        "the field band is the handle"
    );
    browser.palette_drag = None;

    // The first row that asks for an argument: pressing it enters
    // argument mode, the same as Return on it would.
    let rows = browser.palette_layout();
    let view_rows = Browser::palette_view_rows(&rows);
    let target = rows
        .iter()
        .position(|row| row.badge.is_some())
        .expect("a command with an argument");
    let rect = view::palette_row_rect(1200.0, &view_rows, target);
    drop(view_rows);
    browser.palette_press(rect.center_x(), rect.center_y(), 0);
    assert!(browser.palette_drag.is_none(), "a row is not a handle");
    assert!(browser.palette.as_ref().unwrap().prompt().is_some());

    browser.palette_drag = Some((10.0, 10.0));
    browser.drag_palette_to(400.0, 300.0);
    let moved = browser.palette_card();
    assert_eq!(moved.left, 390.0);
    assert_eq!(moved.top, 290.0);
}

/// The row under the pointer is the highlighted one, so Return always does
/// what the pointer is resting on.
#[test]
fn hovering_a_row_highlights_it() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    let rows = browser.palette_layout();
    let view_rows = Browser::palette_view_rows(&rows);
    let items: Vec<usize> = (0..rows.len())
        .filter(|&i| rows[i].kind == view::PaletteRowKind::Item)
        .collect();
    assert!(items.len() >= 2);
    let second = view::palette_row_rect(1200.0, &view_rows, items[1]);
    drop(view_rows);
    browser.palette_hover(second.center_x(), second.center_y());
    assert_eq!(
        browser.palette.as_ref().unwrap().highlighted(),
        Some(items[1])
    );
    // A heading is read, not hovered.
    if let Some(heading) = rows
        .iter()
        .position(|r| r.kind == view::PaletteRowKind::Heading)
    {
        let rows2 = browser.palette_layout();
        let view_rows = Browser::palette_view_rows(&rows2);
        let rect = view::palette_row_rect(1200.0, &view_rows, heading);
        drop(view_rows);
        browser.palette_hover(rect.center_x(), rect.center_y());
        assert_eq!(
            browser.palette.as_ref().unwrap().highlighted(),
            Some(items[1])
        );
    }
}

/// The list is a scroll view: a wheel over it moves the rows, and the
/// arrows keep the highlight in view by the least that shows it.
#[test]
fn the_list_scrolls_under_the_wheel_and_the_arrows() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    let rows = browser.palette_layout();
    assert!(
        rows.len() > view::PALETTE_MAX_ROWS,
        "the resting list should be longer than the viewport"
    );
    let content = browser.palette_scroll.state.content_length();
    let viewport = browser.palette_scroll.state.viewport();
    assert!(content > viewport.height());

    let card = browser.palette_card();
    browser.palette_wheel(card.center_x(), card.center_y(), 3.0, false, true);
    assert!(
        browser.palette_scroll.offset() > 0.0,
        "a notch scrolls the list"
    );
    browser.palette_scroll.scroll_to(0.0);

    // Arrow down past the bottom of the viewport: the list follows.
    let mods = KeyMods {
        shift: false,
        ctrl: false,
    };
    for _ in 0..rows.len() {
        let outcome = browser
            .palette
            .as_mut()
            .unwrap()
            .on_key(palette::Key::Down, mods);
        browser.settle_palette(outcome, 0);
    }
    let offset = browser.palette_scroll.offset();
    assert!(offset > 0.0, "the highlight at the end pulls the list down");
    assert!(offset <= content - viewport.height() + 0.5);
}

/// A press anywhere else lets the palette go, and the click is spent on
/// that: the selection underneath is exactly what it was.
#[test]
fn a_press_off_the_palette_closes_it_and_nothing_else() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt"]);
    browser.size = (1200.0, 800.0);
    let selection = browser.columns[0].selection.clone();
    browser.open_palette();
    browser.palette_press(20.0, 700.0, 0);
    assert!(browser.palette.is_none());
    assert!(browser.palette_drag.is_none());
    assert_eq!(browser.columns[0].selection, selection);
}

/// And cannot be dragged off the window, where it would hold the keyboard
/// from somewhere the user cannot see.
#[test]
fn the_palette_is_held_to_the_window_until_the_display_is_known() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    browser.palette_drag = Some((10.0, 10.0));

    browser.drag_palette_to(-500.0, -500.0);
    let card = browser.palette_card();
    assert!(card.left >= 0.0 && card.top >= 0.0);

    browser.drag_palette_to(5000.0, 5000.0);
    let card = browser.palette_card();
    assert!(card.left <= 1200.0 - card.width() + 0.5);
    assert!(card.top <= 800.0 - view::PALETTE_FIELD_H + 0.5);
}

/// On its own surface the card may leave the window — that is what the
/// surface is for — but not the display. The window here sits 200 points
/// in from the display's left edge, so a card dragged to -100 is off the
/// window and still on screen.
#[test]
fn the_palette_leaves_the_window_but_not_the_display() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    // In window points, so the display starts left of and above the
    // window's own origin — which is what negative coordinates mean here.
    browser.palette_display = Some(Rect::from_ltrb(-200.0, -100.0, 1720.0, 980.0));
    browser.palette_drag = Some((10.0, 10.0));

    browser.drag_palette_to(-100.0, -50.0);
    let card = browser.palette_card();
    assert!(card.left < 0.0, "the card should hang off the window");
    assert!(card.top < 0.0, "the card should rise above the window");

    // And no further than the display.
    browser.drag_palette_to(-5000.0, -5000.0);
    let card = browser.palette_card();
    assert!(card.left >= -200.0 - 0.5);
    assert!(card.top >= -100.0 - 0.5);

    browser.drag_palette_to(5000.0, 5000.0);
    let card = browser.palette_card();
    assert!(card.left <= 1720.0 - card.width() + 0.5);
    assert!(card.top <= 980.0 - view::PALETTE_FIELD_H + 0.5);
}

/// The answer is relative to the window, so it only means anything for as
/// long as the window stays put — one palette session.
#[test]
fn a_fresh_palette_asks_where_the_display_is_again() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    browser.palette_display = Some(Rect::from_ltrb(-200.0, -100.0, 1720.0, 980.0));
    browser.close_palette();

    browser.open_palette();
    assert_eq!(browser.palette_display, None);
}

/// The next open finds the card where it was last left — and brought back
/// inside the window if that has shrunk in the meantime.
#[test]
fn the_palette_reopens_where_it_was_left() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    browser.palette_drag = Some((10.0, 10.0));
    browser.drag_palette_to(400.0, 300.0);
    browser.palette_dropped();
    let left_at = browser.palette_card();
    browser.close_palette();

    browser.open_palette();
    assert_eq!(browser.palette_card(), left_at);
    browser.close_palette();

    // A narrower window: still on screen.
    browser.size = (500.0, 800.0);
    browser.open_palette();
    let card = browser.palette_card();
    assert!(card.right <= 500.0 + 0.5, "clamped into the smaller window");
    assert!(card.left >= 0.0);
}

#[test]
fn a_palette_never_dropped_opens_where_it_belongs() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.size = (1200.0, 800.0);
    browser.open_palette();
    let resting = browser.palette_card();
    browser.palette_drag = Some((10.0, 10.0));
    browser.drag_palette_to(400.0, 300.0);
    // Closed mid-drag, never let go of: nothing to remember.
    browser.palette_drag = None;
    browser.palette_memory = None;
    browser.close_palette();

    browser.open_palette();
    assert_eq!(browser.palette_card().left, resting.left);
    assert_eq!(browser.palette_card().top, resting.top);
}

/// Opening the palette changes nothing underneath it, and closing it
/// leaves the browser exactly as it was.
#[test]
fn the_palette_leaves_the_browser_alone() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt"]);
    browser.select(0, 1);
    let before = (browser.active, browser.columns[0].cursor, browser.mode);
    open_and_type(&mut browser, "trash");
    press(&mut browser, palette::Key::Down);
    assert_eq!(
        press(&mut browser, palette::Key::Escape),
        palette::Outcome::Close
    );
    browser.close_palette();
    assert_eq!(
        (browser.active, browser.columns[0].cursor, browser.mode),
        before
    );
}
