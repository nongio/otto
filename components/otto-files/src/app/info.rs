//! Get Info.

use super::*;

impl Browser {
    /// Open Get Info for the selection.
    ///
    /// The read is synchronous here, unlike a directory listing: it is one
    /// `stat` plus two account lookups for one file the user just asked about,
    /// and it happens on a keystroke rather than during scrolling. If the
    /// account database turns out to block in practice this moves to a worker
    /// like everything else.
    pub(super) fn open_info(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        self.info = Some(model::read_info(&entry.path));
        self.info_error = None;
        self.info_dirty = true;
    }

    pub(super) fn close_info(&mut self) {
        self.info = None;
        self.info_error = None;
        self.info_close_hovered = false;
        self.info_dirty = true;
    }

    /// Toggle one permission bit and apply it.
    ///
    /// Applied immediately rather than behind an OK button — there is no
    /// pending state to get out of step, and a refusal is reported in place.
    /// Only the toggled bit changes, so setuid/setgid/sticky survive.
    pub(super) fn toggle_permission(&mut self, who: usize, what: usize) {
        let Some(info) = &self.info else { return };
        let path = info.path.clone();
        let next = info.mode ^ model::permission_bit(who, what);

        match model::set_mode(&path, next) {
            Ok(()) => {
                // Re-read rather than assuming: the filesystem may have applied
                // something other than what was asked (a mount's umask, an
                // acl), and the sheet must show what is true.
                self.info = Some(model::read_info(&path));
                self.info_error = None;
            }
            Err(reason) => self.info_error = Some(reason),
        }
        self.info_dirty = true;
    }
}
