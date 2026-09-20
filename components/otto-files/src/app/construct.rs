//! Building a browser: the plain window, the trash and the picker.

use super::*;

impl Browser {
    pub(super) fn new(start: PathBuf) -> Self {
        Self {
            columns: vec![Column::new(start)],
            active: 0,
            places: model::places(),
            mode: ViewMode::Columns,
            sort: SortKey::Name,
            ascending: true,
            sort_pinned: false,
            show_hidden: false,
            list_columns: view::ListColumnWidths::default(),
            column_resize: None,
            miller_w: view::MILLER_W,
            miller_resize: None,
            last_miller_click: None,
            last_row_click: None,
            press_pending: None,
            opening: None,
            last_boundary_click: None,
            rename: None,
            pending_rename: None,
            palette: None,
            palette_selection: None,
            palette_offset: (0.0, 0.0),
            caret_clock: None,
            palette_memory: None,
            palette_memory_on_disk: false,
            palette_scroll: ScrollView::new(Rect::new_empty()),
            palette_display: None,
            palette_caret: None,
            palette_drag: None,
            palette_quickview: false,
            commands: {
                let mut registry = command::Registry::builtin();
                registry.add(Box::new(rename::RenameProvider));
                registry.add(Box::new(scripts::ScriptProvider::new()));
                registry
            },
            path_entry: None,
            typeahead: None,
            pending_restore: false,
            pending_empty_ask: false,
            pending_pick: None,
            back: Vec::new(),
            forward: Vec::new(),
            nav_pressed: None,
            pan: ScrollView::horizontal(view::content_viewport(
                view::WINDOW_W,
                view::WINDOW_H,
                ViewMode::Columns,
            )),
            gesture_axis: None,
            size: (view::WINDOW_W, view::WINDOW_H),
            clipboard: model::Clipboard::default(),
            drag_armed: None,
            marquee: None,
            drop_target: None,
            quickview: None,
            preview: None,
            preview_generation_seed: 0,
            thumbs: thumbnails::Store::new(),
            quickview_pending: false,
            quickview_closing: None,
            quickview_auto: std::env::var_os("OTTO_FILES_QV_AUTO").is_some(),
            palette_auto: std::env::var("OTTO_FILES_PALETTE_AUTO").ok(),
            quickview_generation: 0,
            quickview_recognising: false,
            quickview_text_cursor: false,
            ocr_seen: std::collections::HashSet::new(),
            ocr_queue: std::collections::VecDeque::new(),
            ocr_reading: std::collections::HashSet::new(),
            quickview_follow: false,
            trash: false,
            recent: false,
            search: None,
            search_where: String::new(),
            search_scope: model::SearchScope::default(),
            search_origin: None,
            searching: false,
            recent_sections: view::GridSections::default(),
            trash_pressed: None,
            status: None,
            undo: Vec::new(),
            info: None,
            info_text: None,
            info_error: None,
            info_close_hovered: false,
            info_dirty: false,
            controls: WindowControlsState::new(),
            focused: true,
            blur_available: false,
            dirty: true,
            scroll_moved: false,
            listing_dirty: false,
            picker: None,
            save_name: None,
            confirm: None,
            save_probe: RefCell::new(None),
            footer_hover: None,
            path_crumb_hover: None,
            active_place: None,
            footer_pressed: None,
            quickview_drag: None,
            last_quickview_title_click: None,
            quickview_placed_offset: None,
            quickview_display: None,
            quickview_close_hovered: false,
            quickview_expand_hovered: false,
            quickview_panel: None,
            quickview_focus: None,
            quickview_pinch: None,
        }
    }

    /// A picker window serving `session`, opened at the directory the request
    /// asks for.
    /// The Trash window: the trash can's own directory, and nothing around it.
    ///
    /// The sidebar, the nav pair and the view switcher are dropped in the view
    /// layer (see [`view::Shell`]) rather than here — they are chrome, and the
    /// browser's geometry is written against where the file area starts. What
    /// is set here is only what the *listing* is.
    pub(super) fn for_trash() -> Self {
        // Two things are set, and this is the only place either is: the view
        // layer's shell, which decides the chrome and is process-wide because
        // a window does not change shell, and this browser's own flag, which
        // decides what the commands do. Nothing else may set the global —
        // that is what keeps the window's behaviour and its chrome agreeing.
        view::set_shell(view::Shell::Trash);
        Self::listing_the_trash()
    }

    /// The Trash window's listing and commands, without the process-wide
    /// chrome switch [`Browser::for_trash`] throws. Split out so tests can
    /// exercise the behaviour without the geometry of every other test in the
    /// binary changing underneath them.
    pub(super) fn listing_the_trash() -> Self {
        let root = model::trash_files_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut browser = Self::new(root);
        // One directory, listed flat, with a column saying where each row came
        // from — the Miller stack has nothing to descend into that the user
        // put there, and the grid has nowhere to show an origin.
        browser.set_mode(ViewMode::List);
        browser.trash = true;
        browser.places = Vec::new();
        browser
    }

    pub(super) fn for_picker(session: picker::Session, start: PathBuf) -> Self {
        let mut browser = Self::new(start);
        // The picker is a dialog, not a document window: one directory at a
        // time reads as a file dialog, where the Miller stack reads as the
        // browser. The user can still switch views.
        browser.set_mode(ViewMode::List);
        if session.request.mode.names_a_file() {
            let name = session.request.initial_name();
            let selection = picker::name_stem_range(&name);
            let mut input =
                TextInput::editing(name, view::save_field_style(AppContext::current_theme()));
            input.state.select_range(selection);
            browser.save_name = Some(input);
        }
        browser.picker = Some(session);
        browser.pan = ScrollView::horizontal(view::content_viewport(
            browser.size.0,
            browser.size.1 - view::FOOTER_H,
            ViewMode::List,
        ));
        browser
    }

    // -- The Recent place ---------------------------------------------------
}
