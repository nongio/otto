//! What the window shows: its title, subtitle and frame snapshot.

use super::*;

impl Browser {
    pub(super) fn title(&self) -> String {
        // The trash can's directory on disk is called "files", which is not
        // what this window is.
        if self.trash {
            return otto_kit::t_owned!("files-trash");
        }
        // The sentinel behind the recent listing is not a place name, and it
        // is not shown anywhere else either.
        if self.recent {
            return otto_kit::t_owned!("files-recent");
        }
        // Results are titled by what was asked for. The field itself is on the
        // subtitle row and holds the caret, not the answer.
        if let Some(query) = self.query().filter(|_| self.searching) {
            return query;
        }
        let path = if self.mode == ViewMode::Columns {
            self.columns[self.active].path.clone()
        } else {
            self.current_path()
        };
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    }

    /// The path bar's caption: how much of the listing is selected, while
    /// anything is. One item counts too — the header only speaks up past one,
    /// but the bar is where the selection is read at a glance.
    pub(super) fn path_bar_note(&self) -> Option<String> {
        if self.columns.is_empty() {
            return None;
        }
        let depth = self.active.min(self.columns.len() - 1);
        let selected = self.columns[depth].selection.len();
        (selected > 0).then(|| {
            otto_kit::t_owned!(
                "files-status-selected",
                count = selected as i64,
                total = self.visible_len(depth) as i64
            )
        })
    }

    pub(super) fn subtitle(&self) -> String {
        // A first preview pays D-Bus activation, so this can be visible for a
        // moment. Saying so beats a keystroke that appears to do nothing.
        if self.quickview_pending {
            return otto_kit::t_owned!("files-status-opening-preview");
        }
        if let Some(status) = &self.status {
            return status.clone();
        }
        let depth = self.active.min(self.columns.len() - 1);
        if self.columns[depth].loading() {
            return otto_kit::t_owned!("files-loading");
        }
        // A result set counts what was found, not what a folder holds — and
        // says so plainly when it found nothing, since an empty grid under a
        // query reads as broken rather than as an answer.
        //
        // Unless nothing was able to look: search goes to the desktop's index,
        // and with the indexer off there is no answer at all. Saying "nothing
        // found" there would be a wrong answer rather than an empty one, and
        // it would send the person looking for a file that is on the disk.
        if (self.searching || self.recent) && !self.columns[depth].search_available {
            return otto_kit::t_owned!("files-search-unavailable");
        }
        if self.searching {
            let count = self.visible_len(depth);
            return if count == 0 {
                otto_kit::t_owned!("files-search-none")
            } else {
                otto_kit::t_owned!("files-search-found", count = count as i64)
            };
        }
        let selected = self.columns[depth].selection.len();
        if selected > 1 {
            return otto_kit::t_owned!(
                "files-status-selected",
                count = selected as i64,
                total = self.visible_len(depth) as i64
            );
        }
        let count = self.visible_len(depth);
        let hidden = self.columns[depth]
            .snapshot
            .entries
            .iter()
            .filter(|e| e.hidden)
            .count();
        let items = if count == 0 {
            otto_kit::t_owned!("files-status-no-items")
        } else {
            otto_kit::t_owned!("files-status-items", count = count as i64)
        };
        if hidden > 0 && !self.show_hidden {
            // Composed rather than one key per case: the hidden count is an
            // aside on whatever the count line already says, and a translator
            // needs to be able to move it around that line.
            otto_kit::t_owned!(
                "files-status-items-hidden",
                items = items.as_str(),
                hidden = hidden as i64
            )
        } else {
            items
        }
    }

    /// The palette this window draws with.
    ///
    /// The system theme, with the accent muted while the window is in the
    /// background: an unfocused window's selection, drop rings and pills all
    /// step back with the title, so the accent points at the window the user
    /// is actually working in.
    pub(super) fn theme(&self) -> Theme {
        let mut theme = AppContext::current_theme();
        if !self.focused {
            theme.with_muted_accent();
        }
        theme
    }

    /// Build the per-frame view data.
    pub(super) fn frame<'a>(&'a self, theme: &'a Theme, title: &'a str) -> view::Frame<'a> {
        let panes = (0..self.columns.len())
            .map(|depth| {
                let entries = self.visible(depth);
                let column = &self.columns[depth];
                view::PaneData {
                    selection: Some(&column.selection),
                    cursor: column.cursor,
                    entries,
                    scroll: column.scroll.offset(),
                    bar: Some(&column.scroll.state),
                    velocity: column.scroll.velocity(),
                    loading: column.awaiting_first_listing(),
                    error: column.snapshot.error.as_deref(),
                }
            })
            .collect();

        let preview_entry: Option<&'a Entry> = self
            .preview_visible()
            .then(|| {
                let depth = self.active.min(self.columns.len().saturating_sub(1));
                self.columns[depth]
                    .cursor
                    .and_then(|index| self.visible(depth).get(index).copied())
            })
            .flatten();
        let preview = preview_entry.map(|entry| view::PreviewData {
            name: entry.name.as_str(),
            icon_chain: entry.icon_chain(),
            decoded: self.preview.as_ref().and_then(|p| p.decoded.as_ref()),
            video: self.preview.as_ref().and_then(|p| p.video.as_ref()),
            // The player is on its own subsurface, over the column.
            video_on_surface: true,
            first_row: 0,
            info: preview_info(entry),
        });

        view::Frame {
            width: self.size.0,
            // The *file area's* bottom, not the window's — see
            // [`view::Frame::action_row`]. Every piece of geometry the frame
            // carries stops short of the picker's action row because of this
            // one line.
            height: self.content_h(),
            theme,
            title,
            subtitle: self.subtitle(),
            places: &self.places,
            selected_place: self.selected_place(),
            cut: if self.clipboard.cut {
                self.clipboard.paths.clone()
            } else {
                Default::default()
            },
            mode: self.mode,
            grid_sections: &self.recent_sections,
            // Recent has no path bar and no common parent, so a tile has to
            // name its own folder or there is nothing saying where a file came
            // from.
            // Results share no parent either — unless the search was scoped
            // to one folder, where naming it under every tile says nothing.
            show_folders: self.recent
                || (self.searching && self.search_scope == model::SearchScope::Everywhere),
            mode_locked: self.recent,
            search: self.search.as_ref().map(|input| input.value()),
            // Only the synthetic panes depend on the index; a directory
            // listing is read straight off the disk and cannot be unavailable.
            index_available: !(self.searching || self.recent)
                || self.columns[self.active.min(self.columns.len() - 1)].search_available,
            search_focused: self
                .search
                .as_ref()
                .is_some_and(|input| input.state.focused()),
            search_placeholder: &self.search_where,
            search_scope: self.search_scope,
            panes,
            active: self.active,
            pan: self.pan.offset(),
            pan_bar: (self.mode == ViewMode::Columns).then_some(&self.pan.state),
            miller_w: self.miller_w,
            sort: self.sort,
            ascending: self.ascending,
            list_columns: self.list_columns,
            opening: self.opening_progress(),
            renaming: self.rename.as_ref().map(|r| (r.depth, r.index)),
            controls: self.controls,
            focused: self.focused,
            // A translucent material needs something blurred behind it to be
            // translucent over. See [`view::opaque`].
            blurred: self.focused && self.blur_available && otto_kit::frosting::enabled(),
            can_go_back: !self.back.is_empty(),
            can_go_forward: !self.forward.is_empty(),
            nav_pressed: self.nav_pressed,
            preview,
            action_row: self.picker.as_ref().map(|session| view::FooterData {
                accept_label: &session.accept_label,
                accept_enabled: self.picker_accept_enabled(),
                save_name: session.request.mode.names_a_file(),
                save_problem: self.save_problem().map(|key| otto_kit::t!(key)),
                filters: &session.filter_labels,
                current_filter: session.current_filter,
                filter_open: session.filter_open,
                hovered: self.footer_hover,
                pressed: self.footer_pressed,
            }),
            footer: self.footer_h(),
            quickview_close_hovered: self.quickview_close_hovered,
            quickview_expand_hovered: self.quickview_expand_hovered,
            quickview_expanded: self.quickview.as_ref().is_some_and(|s| s.expanded),
            thumbs: Some(&self.thumbs),
            drop_target: self.drop_target.as_ref().map(DropTarget::highlight),
            marquee: self.marquee_band(),
            path_bar: self.path_crumbs(),
            path_bar_h: self.path_bar_h(),
            path_crumb_hover: self.path_crumb_hover,
            path_bar_note: self.path_bar_note(),
            path_entry: self.path_entry.is_some(),
            trash: self.trash.then(|| view::TrashChrome {
                // Put Back acts on the selection; Empty Trash acts on the can.
                // Each is dead when there is nothing for it to do, which is
                // what an empty Trash looks like.
                can_put_back: !self.columns[0].selection.is_empty(),
                can_empty: !self.visible(0).is_empty(),
                pressed: self.trash_pressed,
            }),
        }
    }
}
