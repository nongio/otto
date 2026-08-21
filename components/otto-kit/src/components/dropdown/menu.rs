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

use smithay_client_toolkit::shell::xdg::XdgPositioner;
use wayland_protocols::xdg::shell::client::{xdg_positioner, xdg_surface};

use crate::app_runner::AppContext;
use crate::components::context_menu::{ContextMenu, ContextMenuStyle};
use crate::components::menu_item::MenuItem;

/// Label size and row height for a pop-up button's menu.
///
/// Deliberately larger than a menu-bar menu's 13pt/22pt: this menu drops out
/// of a form control and is read next to that form's own labels, where the
/// bar's compact metrics look undersized. A menu bar is scanned along a
/// crowded strip; a pop-up button is read one row at a time.
const ITEM_FONT_SIZE: f32 = 15.0;
const ITEM_HEIGHT: f32 = 28.0;

/// Owns the popup lifecycle for one dropdown. The caller keeps one of these
/// per dropdown field (it is not `Clone` — there is no reason to share it),
/// alongside whatever selected-index state the field itself needs. Construct
/// it during window setup, not from inside a pointer-event handler — see the
/// module docs above for why that matters.
pub struct DropdownMenu {
    menu: ContextMenu,
}

impl Default for DropdownMenu {
    fn default() -> Self {
        Self {
            menu: ContextMenu::new(Vec::new()).with_style(
                ContextMenuStyle::default().with_item_metrics(ITEM_FONT_SIZE, ITEM_HEIGHT),
            ),
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
        self.menu.hide_animated();
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

        let menu = &self.menu;

        let items: Vec<MenuItem> = options
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let text = if Some(i) == selected {
                    format!("\u{2713} {label}")
                } else {
                    label.clone()
                };
                MenuItem::action(text).with_action_id(i.to_string())
            })
            .collect();
        menu.state().borrow_mut().set_items(items);

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
        positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
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

    #[test]
    fn a_fresh_dropdown_menu_is_closed() {
        let menu = DropdownMenu::new();
        assert!(!menu.is_open());
    }

    // `close()`/`open()` beyond this aren't unit-testable in isolation: both
    // eventually reach `ContextMenu::hide_animated`/`show`, which read
    // `AppContext`'s thread-local statics (the live Wayland globals) the same
    // way the rest of `ContextMenu` does — there is no bare-canvas path for
    // the client half, unlike `field`. That's exercised live instead, by the
    // `dropdown_demo` example against a running Otto session.
}
