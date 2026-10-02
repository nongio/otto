//! What the launcher can offer, and how a choice is carried out.
//!
//! A source is a list of [`Item`]s plus the meaning of activating one. The
//! launcher itself only knows how to filter and draw them, so adding files,
//! clipboard history or a calculator is a matter of another [`Source`], not
//! another launcher.
//!
//! Sources are asked for their items once, when the launcher opens. Nothing
//! here does I/O while someone is typing: a keystroke must never wait on a
//! disk scan.

use otto_agents_kit::item::Item;

/// A provider of items.
pub trait Source {
    /// Short name, shown as the row's badge — "App", "Window".
    fn label(&self) -> &'static str;

    /// The items as they stand now. Called when the launcher opens and
    /// whenever the source says it has changed.
    fn items(&mut self) -> Vec<Item>;

    /// Do whatever picking `index` means. Returning `Ok(())` closes the
    /// launcher.
    fn activate(&mut self, index: usize) -> Result<(), String>;

    /// What to show before anything has been typed.
    ///
    /// Opening the launcher onto every application installed is a wall of
    /// names nobody reads. A source that has a shorter answer to "what did you
    /// want?" gives it here; the default is everything it has, which is right
    /// for a list that is already short.
    fn resting(&mut self) -> Vec<Item> {
        self.items()
    }

    /// An item derived from the query itself, shown first and never ranked.
    ///
    /// Ranking compares a query against the items it might have meant, which
    /// is the wrong shape for a source whose item *is* the query worked out:
    /// the answer to `24.5*3` is `73.5`, and nothing about `73.5` matches what
    /// was typed. An answer is pinned above the matches instead.
    fn answer(&mut self, _query: &str) -> Option<Item> {
        None
    }

    /// Whether the item list has changed since it was last read — a window
    /// opening or closing while the launcher is up.
    fn changed(&mut self) -> bool {
        false
    }

    /// A file descriptor the launcher should wake on, for a source that has
    /// somewhere else to listen. Handed to
    /// [`App::poll_fds`](otto_kit::App::poll_fds).
    fn poll_fd(&self) -> Option<std::os::fd::RawFd> {
        None
    }

    /// Read whatever is waiting on [`Source::poll_fd`]. Called every loop
    /// iteration, so it must not block.
    fn pump(&mut self) {}
}
