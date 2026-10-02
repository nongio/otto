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
            palette_peek: false,
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
            pending_select: None,
            entering: None,
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
            peek: None,
            preview: None,
            preview_generation_seed: 0,
            thumbs: thumbnails::Store::new(),
            peek_pending: false,
            peek_closing: None,
            peek_auto: std::env::var_os("OTTO_FILES_QV_AUTO").is_some(),
            palette_auto: std::env::var("OTTO_FILES_PALETTE_AUTO").ok(),
            peek_generation: 0,
            peek_recognising: false,
            peek_pages_pending: std::collections::HashSet::new(),
            peek_text_asked: false,
            peek_cursor: CursorShape::Default,
            ocr_seen: std::collections::HashSet::new(),
            ocr_queue: std::collections::VecDeque::new(),
            ocr_reading: std::collections::HashSet::new(),
            peek_follow: false,
            trash: false,
            desk: false,
            recent: false,
            search: None,
            search_where: String::new(),
            search_scope: model::SearchScope::default(),
            search_origin: None,
            searching: false,
            index: crate::search::IndexWatch::default(),
            recent_sections: view::GridSections::default(),
            photos: view::PhotosLayout::default(),
            photos_key: None,
            photo_dims: crate::photos::Dims::new(),
            photo_hover: None,
            photo_swatch_hover: None,
            panel_text: None,
            info_selection: Default::default(),
            folder_views: Vec::new(),
            photos_row_h: view::PHOTOS_ROW_H,
            grid_icon: view::DEFAULT_GRID_ICON,
            photos_group: crate::photos::Grouping::default(),
            folder_previews: crate::photos::FolderPreviews::new(),
            photos_slider: Default::default(),
            zoom_pinch: None,
            photos_group_open: false,
            photos_copied: None,
            photos_anchor: None,
            scroll_to_after_layout: None,
            trash_pressed: None,
            status: None,
            job: None,
            undo: crate::undo_history::UndoHistory::for_session(),
            info: None,
            info_text: None,
            info_error: None,
            info_close_hovered: false,
            info_dirty: false,
            open_with: None,
            open_with_dirty: false,
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
            peek_drag: None,
            last_peek_title_click: None,
            peek_placed_offset: None,
            peek_display: None,
            peek_close_hovered: false,
            peek_expand_hovered: false,
            peek_panel: None,
            peek_focus: None,
            peek_pinch: None,
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

    /// The desk: `config`'s folder as an icon grid, with nothing around it.
    ///
    /// Like [`Browser::for_trash`], the chrome is dropped in the view layer —
    /// see [`view::Shell::Desk`] — and what is set here is what the listing
    /// is: one folder, a grid in the configured order, no places and nowhere
    /// else to go.
    pub(super) fn for_desk(config: &crate::desk::DeskConfig) -> Self {
        view::set_shell(view::Shell::Desk);
        view::set_desk_layout(view::DeskLayout {
            anchor: config.anchor,
            size: config.size,
            padding: config.padding,
        });
        view::set_grid_icon(config.icon_size);
        Self::listing_the_desk(config)
    }

    /// The desk's listing and behaviour, without the process-wide chrome
    /// switches [`Browser::for_desk`] throws — for tests, the way
    /// [`Browser::listing_the_trash`] is.
    pub(super) fn listing_the_desk(config: &crate::desk::DeskConfig) -> Self {
        let mut browser = Self::new(config.folder.clone());
        browser.set_mode(ViewMode::Grid);
        browser.sort = config.sort;
        // Newest first by date and A first by name, the way a fresh click on
        // a list column reads.
        browser.ascending = config.sort != SortKey::Modified;
        browser.sort_pinned = true;
        browser.places = Vec::new();
        // Keyboard focus is the desk's only on a click; until then it draws
        // the way a window in the background does.
        browser.focused = false;
        browser.desk = true;
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
