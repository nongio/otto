//! Test helpers on the browser.

use super::*;

impl Browser {
    /// Whether the row called `name` in pane `depth` is selected.
    ///
    /// Tests set their listings up by name, but the selection is keyed by
    /// path — see [`Entry::selection_key`] — so this is the translation
    /// between the two, in one place rather than at every assertion.
    pub(super) fn selected_named(&self, depth: usize, name: &str) -> bool {
        let selection = &self.columns[depth].selection;
        self.visible(depth)
            .iter()
            .any(|e| e.name == name && selection.contains(&e.selection_key()))
    }
}
