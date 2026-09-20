use super::*;

/// A browser sitting on a real directory, so leaving a search has
/// somewhere truthful to go back to.
fn browser_at(dir: &std::path::Path) -> Browser {
    Browser::new(dir.to_path_buf())
}

/// Type a query with the scope pill set to Everywhere, which hands the
/// search to a worker and comes back with the pane still filling.
fn type_query(browser: &mut Browser, query: &str) {
    browser.toggle_search();
    browser.search_scope = model::SearchScope::Everywhere;
    if let Some(input) = browser.search.as_mut() {
        input.set_value(query.to_string());
    }
    browser.run_search();
}

fn named(names: &[&str]) -> Vec<Entry> {
    names
        .iter()
        .map(|name| Entry {
            name: (*name).to_string(),
            path: PathBuf::from("/tmp").join(name),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            kind: otto_kit::filetype::Kind::Text,
            size: Some(0),
            modified: None,
            origin: None,
        })
        .collect()
}

/// This-folder scope searches the folder, not the rows that happen to be
/// on screen in it. Filtering the visible listing would be instant and
/// wrong: nearly everything "in this folder" is in a subfolder of it, and
/// a scope that quietly meant "the names you can already see" would be the
/// one search people trust least.
#[test]
fn this_folder_scope_searches_the_folder_rather_than_the_rows_on_screen() {
    let mut browser = browser_at(&std::env::temp_dir());
    browser.columns[0].snapshot.entries = named(&["ledger.txt", "ledger-2026.txt", "photo.png"]);

    browser.toggle_search();
    assert_eq!(
        browser.search_scope,
        model::SearchScope::Folder,
        "this folder is the default"
    );
    if let Some(input) = browser.search.as_mut() {
        input.set_value("ledger".to_string());
    }
    browser.run_search();

    assert!(browser.searching, "the window is showing results");
    assert!(
        browser.columns[0].loading(),
        "which are being looked up rather than filtered out of the pane"
    );
    assert!(
        browser.columns[0].snapshot.entries.is_empty(),
        "and the rows that were on screen are not passed off as the answer"
    );
}

/// Leaving the query for the results — by clicking one or by arrowing
/// into one — takes the keyboard along, so the keys that act on a file
/// reach the file. What it must *not* do is close the search: the results
/// are the whole reason you went down there.
#[test]
fn leaving_the_query_moves_the_keyboard_without_closing_the_search() {
    let mut browser = browser_at(&std::env::temp_dir());
    type_query(&mut browser, "ledger");

    assert!(browser.blur_search(), "the keyboard moved");
    let input = browser.search.as_ref().expect("the strip is still open");
    assert!(!input.state.focused(), "and the query no longer takes keys");
    assert_eq!(input.value(), "ledger", "but it still says what you typed");
    assert!(browser.searching, "and the results are still on screen");

    // Already in the listing: nothing moved, so nothing needs redrawing.
    assert!(!browser.blur_search());
}

/// Ctrl+F on a strip that has given the keyboard back puts the caret in
/// the query rather than closing it: the results are still up, and the
/// only reason to press it again is to go on typing.
#[test]
fn ctrl_f_takes_the_keyboard_back_before_it_closes_anything() {
    let mut browser = browser_at(&std::env::temp_dir());
    browser.toggle_search();
    assert!(browser.search.is_some());

    // Arrowed down into the results: the strip stays, the keyboard goes.
    browser.blur_search();

    browser.toggle_search();
    let input = browser.search.as_ref().expect("the strip is still open");
    assert!(input.state.focused(), "and it has the caret back");

    // Only now does it close.
    browser.toggle_search();
    assert!(browser.search.is_none());
}

/// The caret is only asked to blink while a field actually holds the
/// keyboard — otherwise the window would wake 125 times a second to draw
/// a caret nobody can see.
#[test]
fn the_caret_blinks_only_while_a_field_has_the_keyboard() {
    use otto_kit::components::text_input::CARET_BLINK_PERIOD;

    let mut browser = browser_at(&std::env::temp_dir());
    assert!(!browser.has_focused_input(), "nothing is being edited");
    assert!(
        !browser.tick_caret(CARET_BLINK_PERIOD),
        "and so nothing changed phase"
    );

    browser.toggle_search();
    assert!(browser.has_focused_input(), "the filter strip has it");

    // Half a period turns the caret over exactly once; the rest of that
    // half does not turn it over again.
    assert!(browser.tick_caret(CARET_BLINK_PERIOD * 0.6));
    assert!(!browser.tick_caret(CARET_BLINK_PERIOD * 0.1));

    browser.clear_search();
    assert!(!browser.has_focused_input(), "and gives it back on Escape");
}

/// Switching the pill re-runs the same query against the other haystack
/// rather than dropping it.
#[test]
fn changing_scope_keeps_the_query() {
    let mut browser = browser_at(&std::env::temp_dir());
    browser.columns[0].snapshot.entries = named(&["nothing-matching.txt"]);
    type_query(&mut browser, "pdf");
    assert!(
        browser.columns[0].loading(),
        "Everywhere handed the query to a worker"
    );

    browser.set_search_scope(model::SearchScope::Folder);
    assert_eq!(
        browser.query().as_deref(),
        Some("pdf"),
        "the query survived"
    );
    assert!(
        browser.columns[0].snapshot.entries.is_empty(),
        "this folder has no pdf in it, so the results should be empty"
    );
}

#[test]
fn ctrl_f_opens_the_field_and_a_second_press_closes_it() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    assert!(browser.search.is_none(), "the field starts closed");
    browser.toggle_search();
    assert!(browser.search.is_some(), "Ctrl+F opens it");
    browser.toggle_search();
    assert!(browser.search.is_none(), "a second press closes it");
}

/// Neither scope can answer on the keystroke that asked: both go to the
/// desktop's index, on a worker. What they must not do is invent something
/// to show in the meantime.
#[test]
fn an_everywhere_search_leaves_the_pane_filling_rather_than_guessing() {
    let mut browser = browser_at(&std::env::temp_dir());
    type_query(&mut browser, "invoice");

    assert!(browser.searching, "the window is showing results");
    assert!(browser.columns[0].loading(), "and they are still arriving");
    assert!(
        browser.columns[0].snapshot.entries.is_empty(),
        "with nothing fabricated to fill the gap"
    );
    assert!(
        browser.columns[0].awaiting_first_listing(),
        "so the pane says it is working rather than showing an empty result"
    );
}

/// An abandoned search is not a navigation: Escape puts back the folder
/// Ctrl+F was pressed in, not wherever the results came from.
#[test]
fn clearing_the_search_returns_to_where_it_started() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    let origin = browser.current_path();

    type_query(&mut browser, "pdf");
    assert!(browser.searching);

    browser.clear_search();
    assert!(!browser.searching, "the results are gone");
    assert!(browser.search.is_none(), "and so is the field");
    assert_eq!(browser.current_path(), origin, "back where it started");
}

/// Emptying the field is the same as leaving: a query of nothing is not a
/// search for everything.
#[test]
fn emptying_the_field_puts_the_listing_back() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    let origin = browser.current_path();

    type_query(&mut browser, "pdf");
    if let Some(input) = browser.search.as_mut() {
        input.set_value(String::new());
    }
    browser.run_search();

    assert!(!browser.searching);
    assert_eq!(browser.current_path(), origin);
    assert!(browser.search.is_some(), "the field itself stays open");
}

/// Results have no hierarchy, so Miller columns are refused — but list and
/// grid both read a result set perfectly well.
#[test]
fn results_can_be_a_list_or_a_grid_but_never_columns() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    type_query(&mut browser, "pdf");

    browser.set_mode(ViewMode::List);
    assert_eq!(browser.mode, ViewMode::List);
    browser.set_mode(ViewMode::Grid);
    assert_eq!(browser.mode, ViewMode::Grid);

    browser.set_mode(ViewMode::Columns);
    assert_eq!(browser.mode, ViewMode::Grid, "Columns was refused");
    assert!(browser.status.is_some(), "and said why");
}

/// The tiles are real files, but they come from a dozen directories at
/// once and the pane has none of them behind it — so every command that
/// needs a folder is refused.
#[test]
fn a_result_cannot_be_acted_on_as_though_it_were_in_a_folder() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    type_query(&mut browser, "pdf");
    assert!(browser.is_synthetic());

    browser.start_rename();
    assert!(browser.rename.is_none(), "rename was refused");
    assert!(browser.status.is_some(), "and said why");
}

/// Previewing is most of what Recent and search are for: you go there
/// because you cannot quite name the file, and looking at it is how you
/// tell which one it is. It was refused while the selection was keyed by
/// name and the rows were fabricated; both of those are gone.
#[test]
fn a_result_can_be_previewed_even_though_it_is_not_in_a_folder() {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("otto-files-qv-{}.txt", std::process::id()));
    std::fs::write(&file, b"preview me").unwrap();

    let mut browser = browser_at(&dir);
    browser.columns[0].snapshot.entries = vec![model::entry_for_path(&file).expect("a real file")];
    browser.searching = true;
    browser.select(0, 0);

    let started = browser.begin_peek();
    std::fs::remove_file(&file).ok();

    let (path, _, _) = started.expect("Peek opened on a result");
    assert_eq!(path, file, "and on the file the cursor is actually on");
    assert!(browser.peek.is_some(), "with the panel up");
}

/// A search is not a place, so the sidebar lights nothing while one is on
/// screen. Results and Recent are both listings with no directory behind
/// them; while they shared one sentinel path, the sidebar matched it
/// against the Recent place and lit it as soon as anyone typed — telling
/// the person they had navigated somewhere they had not.
#[test]
fn searching_does_not_light_up_recent_in_the_sidebar() {
    let mut browser = browser_at(&std::env::temp_dir());
    let recent_place = browser
        .places
        .iter()
        .position(|p| p.recent)
        .expect("Recent leads the sidebar");

    type_query(&mut browser, "invoice");
    assert!(browser.searching, "results are on screen");
    assert!(!browser.recent, "and this is not Recent");

    let lit = browser
        .places
        .iter()
        .position(|p| p.path == browser.columns[0].path);
    assert_ne!(lit, Some(recent_place), "Recent is not lit");
    assert_eq!(
        lit, None,
        "and neither is anything else: a search is nowhere"
    );
}

/// Finding a folder and opening it is most of what a search is for. It
/// cannot descend *into* a result — there is no hierarchy under one — so
/// it goes there instead, leaving the search behind Back.
#[test]
fn opening_a_folder_from_the_results_goes_to_it() {
    let root = std::env::temp_dir().join(format!("otto-files-open-{}", std::process::id()));
    let target = root.join("Projects");
    std::fs::create_dir_all(&target).unwrap();

    let mut browser = browser_at(&root);
    type_query(&mut browser, "projects");
    // Standing in for what the index would have found.
    browser.columns[0].snapshot.entries = vec![model::entry_for_path(&target).unwrap()];
    browser.select(0, 0);

    browser.open_selection();
    let landed = browser.current_path();
    let searching = browser.searching;
    let back = browser.back.len();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(landed, target, "it went to the folder it found");
    assert!(!searching, "and stopped being a search");
    assert!(back > 0, "with the search behind Back");
}

/// A search is a place the window has been, so Back steps out of it to
/// the folder it was asked from and Forward steps back into it — asking
/// the question again rather than replaying the answer, because the disk
/// may have moved on since.
#[test]
fn back_and_forward_step_through_a_search() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    let folder = browser.current_path();

    type_query(&mut browser, "invoice");
    assert!(browser.searching);

    browser.go_back();
    assert!(!browser.searching, "Back leaves the search");
    assert!(browser.search.is_none(), "and takes the strip down with it");
    assert_eq!(browser.current_path(), folder, "back where it was asked");

    browser.go_forward();
    assert!(browser.searching, "Forward steps back into it");
    assert_eq!(
        browser.query().as_deref(),
        Some("invoice"),
        "with the question it was asking"
    );
}

/// Refining a query is the same page asking again, not a new one. One
/// history entry per Return would make Back a way to walk your own typing
/// backwards, which is not what anyone reaches for it to do.
#[test]
fn refining_a_query_does_not_stack_up_history() {
    let mut browser = browser_at(&std::env::temp_dir());
    type_query(&mut browser, "inv");
    let after_entering = browser.back.len();

    for query in ["invo", "invoi", "invoice"] {
        if let Some(input) = browser.search.as_mut() {
            input.set_value(query.to_string());
        }
        browser.run_search();
    }
    assert_eq!(browser.back.len(), after_entering, "still one way back");

    browser.go_back();
    assert!(!browser.searching, "and it leaves the search outright");
}

/// Two rows can name the same folder — a configured shortcut pointing at
/// Downloads, say. Matching the highlight on the path alone lit whichever
/// came first, so clicking the other one navigated correctly and left the
/// row you pressed dark, which reads as a click that did not work.
#[test]
fn the_row_that_was_clicked_is_the_row_that_lights() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);

    // Two rows, one folder.
    browser.places = vec![
        model::Place {
            label: "Downloads".into(),
            path: dir.clone(),
            icon: "folder-download".into(),
            recent: false,
        },
        model::Place {
            label: "Inbox".into(),
            path: dir.clone(),
            icon: "folder".into(),
            recent: false,
        },
    ];

    // Nothing clicked yet: the first row that matches, as before.
    assert_eq!(browser.selected_place(), Some(0));

    browser.active_place = Some(1);
    assert_eq!(
        browser.selected_place(),
        Some(1),
        "the second row lights when it is the one that was pressed"
    );

    // Somewhere the clicked row does not lead: back to matching by path.
    browser.columns[0].path = PathBuf::from("/");
    assert_eq!(browser.selected_place(), None);
}

/// Picking a place in the sidebar leaves the search. Results are not a
/// place, and a window still calling itself a search while showing a
/// folder refuses half its own menu — rename, paste and New Folder all
/// check whether the listing is synthetic.
#[test]
fn choosing_a_place_leaves_the_search_behind() {
    let mut browser = browser_at(&std::env::temp_dir());
    type_query(&mut browser, "invoice");
    assert!(browser.searching && browser.search.is_some());

    // What the sidebar's handler does with a place that is not Recent.
    let was_searching = browser.close_search();

    assert!(
        was_searching,
        "and says results were up, so it can navigate"
    );
    assert!(!browser.searching, "the window is no longer a search");
    assert!(browser.search.is_none(), "the strip is down");
    assert!(!browser.is_synthetic(), "and acts like a folder again");
    assert_eq!(view::search_band_h(), 0.0, "the band gave its line back");
}

/// Closing without going anywhere is the point of the split: the caller
/// navigates. Escape still restores the folder the search began in.
#[test]
fn closing_a_search_leaves_the_caller_to_navigate() {
    let dir = std::env::temp_dir();
    let mut browser = browser_at(&dir);
    let before = browser.current_path();
    type_query(&mut browser, "invoice");

    assert!(browser.close_search());
    assert!(
        !browser.close_search(),
        "a second close has no results to report"
    );

    // Escape, by contrast, puts the folder back.
    let mut browser = browser_at(&dir);
    type_query(&mut browser, "invoice");
    browser.clear_search();
    assert_eq!(browser.current_path(), before);
}

/// The bar earns its line in the listings that have no folder behind
/// them: a search result is a file from anywhere on the disk, and the
/// title says what you asked for rather than where the answer came from.
#[test]
fn the_path_bar_names_where_a_selected_result_actually_lives() {
    let root = std::env::temp_dir().join(format!("otto-files-pb-{}", std::process::id()));
    let nested = root.join("Projects").join("otto");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("notes.txt");
    std::fs::write(&file, b"x").unwrap();

    let mut browser = browser_at(&root);
    browser.columns[0].snapshot.entries = vec![model::entry_for_path(&file).unwrap()];
    browser.searching = true;
    browser.select(0, 0);

    let crumbs = browser.path_crumbs();
    let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();

    // With nothing selected there is no folder to fall back on: the
    // sentinel standing in for one is not a path worth spelling out.
    browser.clear_selection();
    let empty_handed = browser.path_crumbs();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(labels.last(), Some(&"notes.txt"), "{labels:?}");
    assert!(
        labels.contains(&"otto") && labels.contains(&"Projects"),
        "and names the folders holding it: {labels:?}"
    );
    assert!(empty_handed.is_empty(), "and says nothing when it cannot");
}

/// The bands below the file area have to add back up to the window.
/// Anything drawn the full height of it — the sidebar, the divider beside
/// it — is placed from this sum, and a band added without being counted
/// here left the desktop showing through the strip nothing reached.
#[test]
fn the_bands_below_the_file_area_add_up_to_the_window() {
    let mut browser = browser_at(&std::env::temp_dir());
    browser.size = (900.0, 600.0);
    assert_eq!(
        browser.content_h() + browser.path_bar_h() + browser.footer_h(),
        browser.size.1
    );
    assert_eq!(
        browser.path_bar_h(),
        view::PATH_BAR_H,
        "the browser always gives the bar its line, whatever it has to say"
    );
}

/// Two files with the same name in one listing is ordinary in Recent and
/// in search results — they come from different folders. Keyed by name,
/// clicking one selected all of them, and every command that acts on the
/// selection then acted on all of them.
#[test]
fn same_named_results_from_different_folders_select_one_at_a_time() {
    let root = std::env::temp_dir().join(format!("otto-files-dup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for sub in ["one", "two", "three"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
        std::fs::write(root.join(sub).join("Cargo.toml"), b"x").unwrap();
    }

    let mut browser = browser_at(&root);
    browser.columns[0].snapshot.entries = ["one", "two", "three"]
        .iter()
        .map(|sub| model::entry_for_path(&root.join(sub).join("Cargo.toml")).unwrap())
        .collect();
    browser.searching = true;

    browser.select(0, 1);
    let selected: Vec<&str> = browser.columns[0]
        .snapshot
        .entries
        .iter()
        .filter(|e| browser.columns[0].selection.contains(&e.selection_key()))
        .map(|e| e.path.to_str().unwrap())
        .collect();

    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(selected.len(), 1, "one row, not three: {selected:?}");
    assert!(selected[0].contains("two"), "and the one clicked");
}

/// Poll until the pane in front has its listing, so a test can look at
/// what a navigation *settled* into rather than what it started as.
fn settle_listing(browser: &mut Browser) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while browser.columns[0].loading() && std::time::Instant::now() < deadline {
        browser.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    browser.poll();
    browser.rebuild_recent_sections();
}

/// Opening Recent and, before its scan has answered, being sent to a
/// folder — the palette's Home, or any other place — must leave Recent
/// entirely. The pane was replaced; the flag it hid behind must go with
/// it, or the folder arrives under Recent's title, day headings and
/// locked grid, and every place-bound command still says there is no
/// location.
#[test]
fn going_to_a_place_while_recent_is_loading_leaves_recent_behind() {
    let root = std::env::temp_dir().join(format!("otto-files-recent-race-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("note.txt"), "x").unwrap();

    let mut browser = browser_at(&root);
    browser.show_recent();
    assert!(browser.recent, "Recent is up");
    assert!(browser.columns[0].loading(), "and still scanning");

    let request = command::Request {
        id: command::id::GO_TO_PLACE.to_string(),
        arg: Some(root.to_string_lossy().into_owned()),
    };
    browser.run_request(&request, 0).expect("the folder exists");
    settle_listing(&mut browser);

    let landed = browser.current_path();
    let recent = browser.recent;
    let synthetic = browser.is_synthetic();
    let flat = browser.recent_sections.is_flat();
    let names: Vec<String> = browser.columns[0]
        .snapshot
        .entries
        .iter()
        .map(|e| e.name.clone())
        .collect();
    let back_to_recent = matches!(
        browser.back.last().map(|l| &l.synthetic),
        Some(Some(Synthetic::Recent))
    );
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(landed, root, "the window is on the folder it was sent to");
    assert!(!recent, "and Recent is no longer up");
    assert!(!synthetic, "the folder is a real location");
    assert!(flat, "no day headings over a directory listing");
    assert_eq!(names, vec!["note.txt".to_string()], "showing the folder");
    assert!(back_to_recent, "with Recent left behind Back");
}

/// The same race for a search: sent somewhere while the results are
/// still coming, the window must show the folder and not a search.
#[test]
fn going_to_a_place_while_a_search_is_running_closes_the_search() {
    let root = std::env::temp_dir().join(format!("otto-files-search-race-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("note.txt"), "x").unwrap();

    let mut browser = browser_at(&root);
    type_query(&mut browser, "invoice");
    assert!(browser.searching, "results are on their way");
    assert!(browser.columns[0].loading(), "and not here yet");

    let request = command::Request {
        id: command::id::GO_TO_PLACE.to_string(),
        arg: Some(root.to_string_lossy().into_owned()),
    };
    browser.run_request(&request, 0).expect("the folder exists");
    settle_listing(&mut browser);

    let landed = browser.current_path();
    let searching = browser.searching;
    let field_open = browser.search.is_some();
    let names: Vec<String> = browser.columns[0]
        .snapshot
        .entries
        .iter()
        .map(|e| e.name.clone())
        .collect();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(landed, root, "the window is on the folder it was sent to");
    assert!(!searching, "and no longer searching");
    assert!(!field_open, "the strip went with it");
    assert_eq!(names, vec!["note.txt".to_string()], "showing the folder");
}

/// Up from Recent has nowhere to go: the sentinel's parent is not a
/// folder anyone asked for.
#[test]
fn up_from_recent_stays_put() {
    let mut browser = browser_at(&std::env::temp_dir());
    browser.show_recent();
    browser.go_up();
    assert!(browser.recent, "still Recent");
    assert!(
        crate::recent::is_sentinel(&browser.columns[0].path),
        "not the sentinel's parent: {}",
        browser.columns[0].path.display()
    );
}
