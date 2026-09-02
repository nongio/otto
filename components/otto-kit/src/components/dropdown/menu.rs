//! Client half: opening the menu against a real Wayland surface.
//!
//! This is the part [`super::field`] deliberately has no access to.
//! [`DropdownMenu`] wraps a single [`ContextMenu`] and reuses it across every
//! open — see "Why one `ContextMenu` per dropdown, reused" below for why that
//! matters.
//!
//! # Interaction ownership
//!
//! `ContextMenu` registers its own pointer callback and expects to own the
//! interaction for as long as it is up — it does not know it is being used
//! for a dropdown, or that there might be other dropdowns open on the same
//! window. Three things had to be worked out to make that safe here:
//!
//! **Multiple dropdowns don't collide.** `ContextMenu`'s pointer callback
//! filters events by the `wl_surface` id of its own popup(s)
//! (`registered_surfaces`), so a second dropdown's `ContextMenu` — a second,
//! independent instance — simply never sees events for the first one's
//! popup. No shared "current menu" global is needed; each `DropdownMenu`
//! keeps its own `ContextMenu` and they are mutually invisible to each
//! other.
//!
//! **Selection routes back to the right dropdown.** `on_item_click` is set
//! fresh on every [`open`](DropdownMenu::open) call, closing over that
//! call's `on_select`. Because the callback is only ever invoked by *this*
//! `ContextMenu` instance's own popup, there's no dispatch-by-index needed —
//! the closure already belongs to the one dropdown that opened it.
//!
//! **Why one `ContextMenu` per dropdown, built up front.**
//! `ContextMenu::new` permanently registers a pointer callback in
//! `AppContext`'s thread-local callback list (`register_pointer_callback`
//! only ever pushes — there is no unregister). Two consequences follow:
//!
//! - Building a fresh `ContextMenu` on every open would leak one closure
//!   into that list per click, forever, for the life of the process. So
//!   `DropdownMenu` builds its `ContextMenu` exactly once, in
//!   [`DropdownMenu::new`], and keeps it for the dropdown's whole lifetime —
//!   swapping only its items and `on_item_click`/`on_close` closures on each
//!   [`open`](DropdownMenu::open) call.
//! - That construction cannot happen lazily inside
//!   [`open`](DropdownMenu::open) — building it there was the first thing
//!   tried, and it panics (`RefCell already borrowed`, confirmed live
//!   against a running Otto): `open` is normally called from inside a
//!   pointer-event callback, which `AppContext` dispatches by iterating its
//!   callback list with a live borrow; `ContextMenu::new`'s own
//!   `register_pointer_callback` call tries to borrow that same list to push
//!   onto it, and the two borrows collide. Building the `ContextMenu` up
//!   front in `new` — called during window setup, never from inside pointer
//!   dispatch — sidesteps this entirely. A caller of this module never needs
//!   to know the rule; `DropdownMenu`'s shape enforces it.
//!
//! The field's "open" interaction state does not need a push notification:
//! `ContextMenu::is_visible` ([`is_open`](DropdownMenu::is_open)) is polled
//! at draw time. But the window still needs telling *when* to redraw after a
//! dismissal that didn't originate from the caller's own event handling —
//! ESC, or a click outside the menu — so [`open`](DropdownMenu::open) also
//! takes an `on_dismiss` callback, wired through the additive
//! `ContextMenu::on_close` hook (see `context_menu.rs`), which fires exactly
//! when the menu closes without a selection.

use skia_safe::Rect;

use smithay_client_toolkit::reexports::client::protocol::wl_keyboard;
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use wayland_protocols::xdg::shell::client::{xdg_positioner, xdg_surface};

use crate::app_runner::AppContext;
use crate::components::context_menu::{ContextMenu, ContextMenuStyle};
use crate::components::menu_item::MenuItem;

/// Label size and row height for a pop-up button's menu.
///
/// The size is the field's own — `super::field` draws its label at
/// [`crate::typography::styles::BODY`], and a menu that drops out of a control
/// to list that control's values has to read as the same text, not as a larger
/// echo of it. The row stays taller than a menu bar's 22pt all the same: a
/// pop-up button is read one row at a time, not scanned along a crowded strip.
const ITEM_FONT_SIZE: f32 = crate::typography::styles::BODY.size;
const ITEM_HEIGHT: f32 = 26.0;

/// Tallest a pop-up button's menu is drawn before its list starts scrolling.
///
/// A menu listing every installed font or cursor theme is hundreds of rows
/// long; past a screenful it stops being a menu you read and becomes one you
/// hunt through, and the compositor would be sliding a full-height popup
/// around to keep it on screen. Capped here, the surplus scrolls under the
/// wheel instead.
const MAX_HEIGHT: f32 = 360.0;

/// How much wider than the button its menu may grow to fit a long value.
///
/// The menu is the button's own column by default. A value too long for it
/// widens the menu rather than being cut down to nothing — but only this far,
/// and the menu stays anchored to the button's right edge so the growth goes
/// leftwards into the form's margin instead of out over its labels.
const MAX_GROWTH: f32 = 1.6;

/// What an item's label loses to the menu's own padding and to the checkmark
/// column, so the text it is elided to actually fits the row.
const TEXT_INSET: f32 = 26.0;

/// How long a type-ahead buffer survives without another key.
///
/// Long enough to spell a word at a normal pace, short enough that coming
/// back to the menu later starts a fresh name rather than extending a stale
/// one. Matches otto-files, so the gesture feels the same in both.
const TYPEAHEAD_EXPIRY: std::time::Duration = std::time::Duration::from_secs(1);

/// The buffer to search with after `ch` is typed, and whether this is the
/// same single character again — which cycles through the values beginning
/// with it rather than looking for a doubled letter almost no value has.
///
/// `live` is the previous buffer if it has not expired. Split out from
/// [`DropdownMenu::handle_text`] so the rule is testable without a surface.
fn advance(live: Option<String>, ch: char) -> (String, bool) {
    let typed = ch.to_lowercase().to_string();
    match live {
        Some(buffer) if buffer == typed => (buffer, true),
        Some(mut buffer) => {
            buffer.push_str(&typed);
            (buffer, false)
        }
        None => (typed, false),
    }
}

/// The first of `names` at or after `from`, wrapping, that starts with
/// `buffer`. `names` are already lowercased, as `buffer` is.
fn search(names: &[String], buffer: &str, from: usize) -> Option<usize> {
    if names.is_empty() {
        return None;
    }
    (0..names.len())
        .map(|step| (from + step) % names.len())
        .find(|&index| names[index].starts_with(buffer))
}

/// The font the menu sets its rows in, for measuring.
fn item_font() -> skia_safe::Font {
    crate::typography::TextStyle {
        family: "Inter",
        weight: 400,
        size: ITEM_FONT_SIZE,
    }
    .font()
}

/// How wide `text` is in a menu row.
fn measure(text: &str) -> f32 {
    item_font().measure_str(text, None).0
}

/// Trim `text` until it fits `width`, marking the cut with a trailing
/// ellipsis. Returns it unchanged when it already fits.
fn elide(text: &str, width: f32) -> String {
    let font = item_font();
    if width <= 0.0 || font.measure_str(text, None).0 <= width {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        end -= 1;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = format!("{}\u{2026}", &text[..end]);
        if font.measure_str(&candidate, None).0 <= width {
            return candidate;
        }
    }
    "\u{2026}".to_string()
}

/// Owns the popup lifecycle for one dropdown. The caller keeps one of these
/// per dropdown field (it is not `Clone` — there is no reason to share it),
/// alongside whatever selected-index state the field itself needs. Construct
/// it during window setup, not from inside a pointer-event handler — see the
/// module docs above for why that matters.
pub struct DropdownMenu {
    menu: ContextMenu,
    /// The values the menu was last opened with, unelided.
    ///
    /// The rows themselves hold whatever fitted the button's column, which is
    /// what a sighted user reads. A screen reader has no column to fit and
    /// must be told the whole value: "Adwaita-dark" and "Adwai…" are not the
    /// same answer.
    options: std::cell::RefCell<Vec<String>>,
    /// The type-ahead buffer and when it was last appended to. Typing keeps
    /// extending one word; a pause of [`TYPEAHEAD_EXPIRY`] starts a new one.
    typeahead: std::cell::RefCell<Option<(String, std::time::Instant)>>,
}

impl Default for DropdownMenu {
    fn default() -> Self {
        Self {
            menu: ContextMenu::new(Vec::new()).with_style(
                ContextMenuStyle::default().with_item_metrics(ITEM_FONT_SIZE, ITEM_HEIGHT),
            ),
            options: std::cell::RefCell::new(Vec::new()),
            typeahead: std::cell::RefCell::new(None),
        }
    }
}

impl DropdownMenu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the menu is currently up. Drive [`super::field::DropdownInteraction::Open`]
    /// from this at draw time rather than tracking it separately — it can't
    /// drift from what `ContextMenu` actually has on screen.
    pub fn is_open(&self) -> bool {
        self.menu.is_visible()
    }

    /// Dismiss the menu if it is open. Safe to call unconditionally (e.g.
    /// before opening a sibling dropdown, or on window close).
    pub fn close(&self) {
        *self.typeahead.borrow_mut() = None;
        self.menu.hide_animated();
    }

    /// Feed a key to the open menu: the arrows move the highlight (scrolling
    /// the list to keep it in view), Home and End go to the ends, Enter or
    /// Space chooses, Escape closes with nothing chosen.
    ///
    /// A pop-up button opened from the keyboard has to be usable from it, and
    /// the menu is a surface of its own — the keyboard is on it, not on the
    /// field, so the application has to hand its keys over for as long as it
    /// is up. Call it from `on_keyboard_event` (or with `KeyEvent::raw_code`)
    /// while [`is_open`](Self::is_open) holds, and check `is_open` afterwards:
    /// Enter, Space and Escape all close, and the field has to stop drawing
    /// itself open.
    ///
    /// Does nothing when the menu is closed, so a caller may pass every key
    /// through without checking first.
    pub fn handle_key(&self, key: u32, state: wl_keyboard::KeyState) {
        if !self.is_open() {
            return;
        }
        // `ContextMenu` is a handle over shared state, so a clone is the same
        // menu — which is what lets a dropdown held in an immutable map still
        // be driven.
        self.menu.clone().handle_key(key, state);
    }

    /// Feed a whole key event to the open menu: [`handle_key`](Self::handle_key)
    /// for the navigation keys, then type-ahead for anything printable.
    ///
    /// This is the call to make from `on_key_event` — the two halves want the
    /// same event, one for its keycode and one for its text, and no caller
    /// needs both apart. A chord belongs to whoever binds it, so skip this
    /// (or use `handle_key` alone) when Ctrl, Alt or Super is down.
    pub fn handle_key_event(
        &self,
        event: &smithay_client_toolkit::seat::keyboard::KeyEvent,
        state: wl_keyboard::KeyState,
    ) {
        self.handle_key(event.raw_code, state);
        if state == wl_keyboard::KeyState::Pressed {
            if let Some(text) = event.utf8.as_deref() {
                self.handle_text(text);
            }
        }
    }

    /// Move the highlight to the first value starting with what has been
    /// typed, case-insensitively — reaching an entry in a long list of fonts
    /// or themes without arrowing down to it.
    ///
    /// Nothing is filtered and nothing is drawn but the highlight moving,
    /// which is the whole point of the gesture. Keys typed in quick
    /// succession compose one word; the buffer expires after a second of
    /// silence. Repeating a single character cycles through the values
    /// beginning with it.
    ///
    /// `text` is a key event's `utf8`; the keys that mean something else to
    /// the menu carry either nothing (the arrows) or a control character
    /// (Enter, Escape) and are ignored here, so a caller may pass every key
    /// through. Returns whether the highlight moved. Does nothing when the
    /// menu is closed.
    pub fn handle_text(&self, text: &str) -> bool {
        if !self.is_open() {
            return false;
        }
        let Some(ch) = text
            .chars()
            .next()
            .filter(|ch| !ch.is_control() && *ch != ' ')
        else {
            return false;
        };

        let names: Vec<String> = self
            .options
            .borrow()
            .iter()
            .map(|option| option.to_lowercase())
            .collect();
        if names.is_empty() {
            return false;
        }

        let now = std::time::Instant::now();
        let live = self
            .typeahead
            .borrow_mut()
            .take()
            .filter(|(_, last)| now.duration_since(*last) < TYPEAHEAD_EXPIRY)
            .map(|(buffer, _)| buffer);
        let (buffer, cycling) = advance(live, ch);

        // Cycling resumes just past the highlight and wraps; a buffer that
        // grew answers from the top, so the same keys always land on the same
        // value.
        let from = match (cycling, self.highlighted()) {
            (true, Some(index)) => index + 1,
            _ => 0,
        };
        let hit = search(&names, &buffer, from);

        // Kept even when nothing matched: the miss is part of the word being
        // typed, and dropping it would make the next character search for a
        // prefix the user never asked for.
        *self.typeahead.borrow_mut() = Some((buffer, now));
        if let Some(index) = hit {
            self.menu.select_and_reveal(index);
        }
        hit.is_some()
    }

    /// The values the open menu is listing, in order, as they were given —
    /// not as they were elided to fit.
    ///
    /// Empty when the menu is closed. For describing the menu to an assistive
    /// technology: an open pop-up is a list on screen, and a tree that omits
    /// it leaves a screen reader reading the button underneath something it
    /// cannot see past.
    pub fn options(&self) -> Vec<String> {
        if !self.is_open() {
            return Vec::new();
        }
        self.options.borrow().clone()
    }

    /// Which row the highlight is on, if any.
    pub fn highlighted(&self) -> Option<usize> {
        self.menu.state().borrow().selected()
    }

    /// Put the highlight on one item without choosing it.
    ///
    /// For opening from the keyboard: the menu should come up on the value the
    /// button is showing, so the first arrow press moves from *there* rather
    /// than from the top of the list. A pointer-opened menu leaves it alone —
    /// nothing is highlighted until the pointer is over something, which is
    /// what a highlight means to someone using one.
    pub fn highlight(&self, index: Option<usize>) {
        self.menu.state().borrow_mut().select(index);
    }

    /// Open the menu, anchored to `field_rect` — the same rect passed to
    /// [`super::field::draw`], in the window's local coordinate space —
    /// parented to `parent_xdg`.
    ///
    /// `options[selected]`, if `selected` is `Some`, is marked with a
    /// leading checkmark; `MenuItem` has no dedicated "checked" flag, so
    /// this is the least intrusive way to show it without changing that
    /// type. `serial` should come from the pointer press that triggered the
    /// open. `on_select` fires with the chosen index; `on_dismiss` fires if
    /// the menu instead closes with nothing chosen (ESC or an outside
    /// click) — use it to `request_frame()` the window so the field's
    /// "open" look clears promptly instead of waiting for the next
    /// unrelated redraw.
    #[allow(clippy::too_many_arguments)]
    pub fn open<S, D>(
        &self,
        parent_xdg: &xdg_surface::XdgSurface,
        field_rect: Rect,
        serial: u32,
        options: &[String],
        selected: Option<usize>,
        on_select: S,
        on_dismiss: D,
    ) where
        S: Fn(usize) + 'static,
        D: Fn() + 'static,
    {
        if options.is_empty() {
            return;
        }

        *self.options.borrow_mut() = options.to_vec();
        *self.typeahead.borrow_mut() = None;

        let menu = &self.menu;

        // The menu starts as the button's own column — a list of that button's
        // values, under that button — and grows leftwards only as far as
        // `MAX_GROWTH` if the longest value needs it. Anything still too long
        // is elided; letting the menu take whatever width its longest label
        // wants leaves the two controls' edges disagreeing by however long
        // that label happens to be.
        let field_w = field_rect.width().max(1.0);
        let widest = options
            .iter()
            .map(|label| measure(label))
            .fold(0.0_f32, f32::max)
            + TEXT_INSET;
        let width = widest.clamp(field_w, field_w * MAX_GROWTH);
        menu.clone().with_style(
            ContextMenuStyle::default()
                .with_item_metrics(ITEM_FONT_SIZE, ITEM_HEIGHT)
                .with_width(width)
                .with_min_width(width)
                .with_max_height(MAX_HEIGHT),
        );

        let items: Vec<MenuItem> = options
            .iter()
            .enumerate()
            .map(|(i, label)| {
                // The checkmark rides in the trailing slot rather than in
                // front of the label: a leading mark indents the chosen row
                // out of line with every other one, and the values then no
                // longer share a left edge to be scanned down.
                let item = MenuItem::action(elide(label, width - TEXT_INSET))
                    .with_action_id(i.to_string());
                if Some(i) == selected {
                    item.with_shortcut("\u{2713}")
                } else {
                    item
                }
            })
            .collect();
        let overflow = {
            let mut state = menu.state().borrow_mut();
            state.set_items(items);
            // A menu that scrolls opens on the value it is showing, not at the
            // top: in a list of every installed font, the top is the one place
            // the current one almost certainly is not. Centred in the box, so
            // the values either side of it are visible too.
            let overflow = ContextMenuStyle::default()
                .with_item_metrics(ITEM_FONT_SIZE, ITEM_HEIGHT)
                .with_max_height(MAX_HEIGHT);
            let overflow = crate::components::context_menu::ContextMenuRenderer::overflow(
                state.items(),
                &overflow,
            );
            let centred = selected
                .map(|i| i as f32 * ITEM_HEIGHT - MAX_HEIGHT / 2.0 + ITEM_HEIGHT / 2.0)
                .unwrap_or(0.0);
            state.set_scroll(centred, overflow);
            overflow
        };
        let _ = overflow;

        menu.clone().on_item_click(move |action_id| {
            if let Ok(index) = action_id.parse::<usize>() {
                on_select(index);
            }
        });
        menu.clone().on_close(on_dismiss);

        let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) else {
            return;
        };
        let (menu_w, menu_h) = menu.get_size_at_depth(0);
        positioner.set_size(menu_w as i32, menu_h as i32);
        // Anchor rect is the field itself; the menu drops from its
        // bottom-left, sliding/flipping to stay on screen near an edge.
        positioner.set_anchor_rect(
            field_rect.left as i32,
            field_rect.top as i32,
            field_rect.width().max(1.0) as i32,
            field_rect.height().max(1.0) as i32,
        );
        // Anchored to the button's bottom-RIGHT corner, growing down and to
        // the left from it: the menu and the button share a right edge, so a
        // menu that had to grow does so into the form's margin rather than
        // over the row's label.
        positioner.set_anchor(xdg_positioner::Anchor::BottomRight);
        positioner.set_gravity(xdg_positioner::Gravity::BottomLeft);
        positioner.set_offset(0, 4);
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::SlideX
                | xdg_positioner::ConstraintAdjustment::SlideY
                | xdg_positioner::ConstraintAdjustment::FlipX
                | xdg_positioner::ConstraintAdjustment::FlipY,
        );

        menu.show(parent_xdg, &positioner, serial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_lowercase()).collect()
    }

    #[test]
    fn a_letter_jumps_to_the_first_value_starting_with_it() {
        let options = names(&["Adwaita", "Breeze", "Bibata", "Capitaine"]);
        let (buffer, cycling) = advance(None, 'b');
        assert_eq!(buffer, "b");
        assert!(!cycling);
        assert_eq!(search(&options, &buffer, 0), Some(1));
    }

    #[test]
    fn more_letters_compose_one_word() {
        let options = names(&["Adwaita", "Breeze", "Bibata", "Capitaine"]);
        let (buffer, _) = advance(None, 'b');
        let (buffer, cycling) = advance(Some(buffer), 'i');
        assert_eq!(buffer, "bi");
        assert!(!cycling);
        // The composed word answers from the top, so the same keys always
        // land on the same value.
        assert_eq!(search(&options, &buffer, 0), Some(2));
    }

    #[test]
    fn the_same_letter_again_cycles_rather_than_seeking_a_doubled_one() {
        let options = names(&["Adwaita", "Breeze", "Bibata", "Capitaine"]);
        let (buffer, cycling) = advance(Some("b".to_string()), 'b');
        assert_eq!(buffer, "b");
        assert!(cycling);
        // Resumes past the highlight and wraps back round.
        assert_eq!(search(&options, &buffer, 2), Some(2));
        assert_eq!(search(&options, &buffer, 3), Some(1));
    }

    #[test]
    fn matching_ignores_case() {
        let options = names(&["Adwaita", "Breeze"]);
        let (buffer, _) = advance(None, 'A');
        assert_eq!(buffer, "a");
        assert_eq!(search(&options, &buffer, 0), Some(0));
    }

    #[test]
    fn a_word_no_value_starts_with_moves_nothing() {
        let options = names(&["Adwaita", "Breeze"]);
        assert_eq!(search(&options, "bz", 0), None);
        assert_eq!(search(&[], "b", 0), None);
    }

    #[test]
    fn a_fresh_dropdown_menu_is_closed() {
        let menu = DropdownMenu::new();
        assert!(!menu.is_open());
    }

    #[test]
    fn a_closed_menu_swallows_typing() {
        // No surface, so nothing to move a highlight on — and no panic from
        // a caller that passes every key through without checking first.
        let menu = DropdownMenu::new();
        assert!(!menu.handle_text("b"));
    }

    // `close()`/`open()` beyond this aren't unit-testable in isolation: both
    // eventually reach `ContextMenu::hide_animated`/`show`, which read
    // `AppContext`'s thread-local statics (the live Wayland globals) the same
    // way the rest of `ContextMenu` does — there is no bare-canvas path for
    // the client half, unlike `field`. That's exercised live instead, by the
    // `dropdown_demo` example against a running Otto session.
}
