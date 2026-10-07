//! Selectable text in the info panels: the Photos view's panel, the column
//! view's preview caption, and Get Info.
//!
//! Each panel lays its text out as runs (see
//! [`otto_kit::components::selectable_text`]) that it both draws and hands
//! here to hit-test, so a drag selects exactly what is on screen. One
//! selection lives in the main window at a time, tied to the panel and to what
//! the panel was describing; Get Info, a window of its own, has its own.

use super::*;
use otto_kit::components::selectable_text::{TextRun, TextSelection};
use otto_kit::theme::Theme;

/// Which panel in the main window a text selection belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextPanel {
    Photos,
    Preview,
}

/// A text selection in one of the main window's panels, and what that panel
/// was showing when it was made.
pub(super) struct PanelText {
    pub(super) panel: TextPanel,
    /// What the panel was about — a file's path, a count — so the selection
    /// is dropped rather than carried onto the next file's facts.
    pub(super) subject: String,
    pub(super) selection: TextSelection,
}

/// The palette the runs are laid out with for hit-testing. Only where the
/// text is matters there, and that does not depend on its colour.
fn layout_theme() -> Theme {
    Theme::light()
}

impl Browser {
    /// The text of each panel showing in the main window, as runs in window
    /// coordinates, with what the panel is about.
    fn panel_runs(&self, panel: TextPanel) -> Option<(String, Vec<TextRun>)> {
        let theme = layout_theme();
        match panel {
            TextPanel::Photos => {
                if !self.photos.has_panel() {
                    return None;
                }
                let data = self.photos_info_data()?;
                let rect = self.photos.panel_rect(self.size.0, self.content_h());
                Some((data.subject(), view::photos_info_runs(rect, &data, &theme)))
            }
            TextPanel::Preview => {
                if !self.preview_visible() {
                    return None;
                }
                let entry = self.selected_entry()?;
                let rect = view::preview_pane_rect(
                    self.columns.len(),
                    self.content_h(),
                    self.pan.offset(),
                    &self.miller_widths(),
                );
                let info = preview_info(
                    &entry,
                    self.decoded_preview(),
                    self.preview.as_ref().and_then(|pane| pane.text),
                );
                let runs = view::preview_caption_runs(rect, &entry.name, &info, &theme);
                Some((entry.path.to_string_lossy().into_owned(), runs))
            }
        }
    }

    /// The panels a press could select text in, in this view.
    fn text_panels(&self) -> &'static [TextPanel] {
        match self.mode {
            ViewMode::Photos => &[TextPanel::Photos],
            ViewMode::Columns => &[TextPanel::Preview],
            ViewMode::List | ViewMode::Grid => &[],
        }
    }

    /// A left press in the main window: on a panel's text it starts or
    /// extends a text selection and is taken; anywhere else it clears any
    /// selection and is left for the rest of the window.
    pub(super) fn panel_text_press(&mut self, x: f32, y: f32) -> bool {
        let now = std::time::Instant::now();
        for &panel in self.text_panels() {
            let Some((subject, runs)) = self.panel_runs(panel) else {
                continue;
            };
            if TextSelection::hit(&runs, x, y).is_none() {
                continue;
            }
            // The same panel over the same thing keeps its selection, so a
            // second and third click count as a double and a triple.
            let mut selection = match self.panel_text.take() {
                Some(text) if text.panel == panel && text.subject == subject => text.selection,
                _ => TextSelection::new(),
            };
            selection.press(&runs, x, y, now);
            self.panel_text = Some(PanelText {
                panel,
                subject,
                selection,
            });
            self.dirty = true;
            return true;
        }
        if self.panel_text.take().is_some() {
            self.dirty = true;
        }
        false
    }

    /// The pointer moved with the button down. Returns whether a text drag
    /// has it.
    pub(super) fn panel_text_drag(&mut self, x: f32, y: f32) -> bool {
        let Some(panel) = self
            .panel_text
            .as_ref()
            .filter(|text| text.selection.is_dragging())
            .map(|text| text.panel)
        else {
            return false;
        };
        let runs = self.panel_runs(panel).map(|(_, runs)| runs);
        if let (Some(runs), Some(text)) = (runs, self.panel_text.as_mut()) {
            if text.selection.drag(&runs, x, y) {
                self.dirty = true;
            }
        }
        true
    }

    pub(super) fn panel_text_release(&mut self) {
        if let Some(text) = self.panel_text.as_mut() {
            text.selection.release();
        }
    }

    /// Whether `(x, y)` is over selectable panel text: where the pointer is
    /// an I-beam.
    pub(super) fn over_panel_text(&self, x: f32, y: f32) -> bool {
        self.text_panels().iter().any(|&panel| {
            self.panel_runs(panel)
                .is_some_and(|(_, runs)| TextSelection::is_over_text(&runs, x, y))
        })
    }

    /// The selection to highlight in `panel`, while it is still about
    /// `subject`.
    pub(super) fn panel_selection(
        &self,
        panel: TextPanel,
        subject: &str,
    ) -> Option<&TextSelection> {
        self.panel_text
            .as_ref()
            .filter(|text| text.panel == panel && text.subject == subject)
            .map(|text| &text.selection)
            .filter(|selection| selection.has_selection())
    }

    /// The selected panel text, if there is any and the panel still shows
    /// what it was selected from.
    pub(super) fn panel_selected_text(&self) -> Option<String> {
        let text = self.panel_text.as_ref()?;
        let (subject, runs) = self.panel_runs(text.panel)?;
        (subject == text.subject)
            .then(|| text.selection.selected_text(&runs))
            .flatten()
    }

    /// Ctrl+C with panel text selected: the text goes to the clipboard
    /// rather than the files. Returns whether there was text to copy.
    pub(super) fn copy_panel_text(&mut self, serial: u32) -> bool {
        match self.panel_selected_text() {
            Some(text) => {
                clipboard::set_text(&text, serial);
                true
            }
            None => false,
        }
    }

    // -- Get Info ------------------------------------------------------------

    /// Get Info's text, as runs in the sheet's own coordinates.
    pub(super) fn info_runs(&self) -> Vec<TextRun> {
        let Some(info) = self.info.as_ref() else {
            return Vec::new();
        };
        view::info_runs(
            Rect::from_wh(view::INFO_W, view::INFO_H),
            info,
            self.info_text,
            &layout_theme(),
        )
    }

    /// Ctrl+C in Get Info: the selected text, if there is any.
    pub(super) fn copy_info_text(&mut self, serial: u32) -> bool {
        match self.info_selection.selected_text(&self.info_runs()) {
            Some(text) => {
                clipboard::set_text(&text, serial);
                true
            }
            None => false,
        }
    }
}
