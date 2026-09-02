# Context Menus

**Status:** draft
**Related specs:** topbar.md

## Summary

Context menus are hierarchical popup menus that display items for user selection. They support submenus, keyboard and mouse navigation, and are rendered as popup surfaces anchored to their parent. Context menus are used throughout Otto — in the topbar (global menus and tray icon context menus), and in application windows.

## Goals

- Provide a consistent, reusable context menu component for all Otto UI surfaces.
- Support nested submenus with independent visual state at each level.
- Allow both keyboard and mouse interaction with proper focus handling.
- Render menus with configurable styling (colors, fonts, padding, animations).
- Integrate seamlessly with Wayland popup protocol (xdg_popup).

## Non-Goals

- Context menus are not window management tools — they do not create persistent surfaces or interact with workspace logic.
- Custom menu markup or XML parsing — menu structure is defined in code or via D-Bus (dbusmenu).
- Accessibility features beyond keyboard navigation (screen reader support is deferred).

## Behavior

### Menu Structure & Rendering

1. A context menu contains an ordered list of items. Each item is one of: **action** (clickable with optional label, icon, keyboard shortcut), **submenu** (has a label and child menu), or **separator** (visual divider).
2. Menus are rendered as xdg_popup surfaces anchored below (or beside) their parent surface. The popup is positioned and sized by the parent using an xdg_positioner.
3. Menu width is computed from the widest item label + icon + shortcut, with a minimum configurable width. Separators do not affect width.
4. Menu height is the sum of all item heights plus vertical padding.
5. Items are rendered with a rounded-rect highlight when selected. Separators are not selectable.
6. Disabled items are rendered with reduced opacity and do not respond to user interaction.
6a. The menu background uses the theme's popup material, which is more opaque than the chrome materials used for bars and sidebars: a menu floats over arbitrary window content and its labels must stay readable against it.

### Keyboard Navigation

7. When a menu is open and has keyboard focus, arrow keys move selection: **UP** moves to the previous selectable item (wrapping to the last), **DOWN** moves to the next (wrapping to the first).
8. **RIGHT arrow** (or **RETURN** when hovering): If the selected item is a submenu, open the submenu and move focus to its first item. The parent menu's selection is cleared so only one item is highlighted across the entire menu tree.
9. **LEFT arrow**: If in a submenu, close the submenu and return focus to the parent menu. The parent item that had the open submenu is re-selected.
10. **RETURN** or **SPACE**: Activate the selected action item. If a submenu, same as RIGHT.
10a. **HOME** / **END**: Move the selection to the first / last selectable item at the current depth.
10b. Whenever the keyboard moves the selection (arrows, Home, End), a menu whose list is capped and scrolling scrolls itself so the newly selected row is inside its box, clear of the scroll arrows at either end. A selection already in view leaves the scroll offset alone.
10c. **Type-ahead**: a host that hands the menu the text a key carried (a pop-up button does; see `DropdownMenu::handle_text`) moves the selection to the first value starting with what has been typed, case-insensitively, scrolling it into view. Successive keys compose one word; a second of silence starts a fresh one; the same character again cycles through the values beginning with it, resuming past the current selection and wrapping. Nothing is filtered and no search field appears — the only sign it happened is the selection moving, the same gesture as the file browser's (see [file-picker.md](./file-picker.md) under *Keyboard*). A word matching nothing leaves the selection where it is but stays in the buffer. Keys carrying no text (the arrows) or a control character (Return, Escape) are not type-ahead, and neither is a chord — a modified key belongs to whoever binds it.
11. **ESCAPE**: Close the menu (or all open submenus if in a submenu, returning to the root).
12. Typing a character may activate a menu item with a matching keyboard shortcut (if implemented).
12a. **Type-ahead**: typing moves the selection to the first item whose label begins with what has been typed, scrolling it into view as the arrows would. Characters typed within a second of each other accumulate into one prefix; a longer gap starts a new one. A character that matches nothing after the accumulated prefix starts a fresh search from itself rather than being swallowed, and a prefix that matches nothing at all falls back to matching anywhere in a label. Matching ignores case, and is fed the key's text as the layout produced it rather than its keycode.

### Mouse Interaction

13. Hovering over a menu item selects it and renders the highlight.
14. Hovering over a submenu item does NOT immediately open the submenu. The submenu opens after a configurable delay (default 300ms) if the mouse remains over the item.
15. Moving the mouse away from an item before the delay expires cancels submenu opening.
16. Clicking a selectable item activates it and closes the menu.
17. Clicking a submenu item opens the submenu if not already open; clicking again (or moving to a different submenu) closes the old submenu and opens the new one.
18. Moving the mouse between menu depths (e.g., from parent item to submenu item) clears the selection at all other depths, leaving only the hovered depth selected.
19. If the mouse leaves the entire menu tree (all popups), selections are cleared and submenus remain open until the user moves the mouse back or presses a key.

### Selection State & Popup Focus

20. Exactly zero or one item is selected (highlighted) across the entire menu tree at any time. When a submenu is visible, its items can be selected but the parent menu's item is not highlighted.
21. When a submenu opens, the root popup retains keyboard focus. Submenu popups do not request a keyboard grab — they forward keyboard events to the root menu.
22. When the root menu loses keyboard focus (e.g., the user clicks outside the menu, or the focused window changes), the menu closes immediately. The focused surface while the grab is active is the root *popup*, so it is that surface's `wl_keyboard.leave` that closes the menu — the parent's `leave`, which fires as the grab hands focus to the popup, must be ignored.
23. Right-clicking outside the menu also closes it.
23a. A press anywhere on the owning client's *own* surfaces that is not part of the menu tree — elsewhere in the parent window, another window of the same app — also closes the menu. The compositor's popup grab is "owner-events": it dismisses the popup only when the press lands on a *different* client, so same-client outside clicks are the menu's own responsibility. The dismissal is applied at the *end* of the pointer batch the press arrived in — after every other pointer callback and the app's own `on_pointer_event` — so that a control which owns the menu (a dropdown field toggling itself shut) gets to handle its own press first and the menu is never closed out from under a reopen. Deferring to the next batch instead would leave the menu up whenever no further pointer event follows.

### Submenu Lifecycle

24. Submenus are created as child xdg_popups. The parent popup surface is the menu's layer-shell or window surface.
25. When a submenu is opened via keyboard or mouse, it is rendered as a new popup surface positioned to the right of (or to the left of, for RTL layouts) the parent menu.
26. A submenu remains open until: the user navigates away (keyboard LEFT or mouse moves to a different branch), the user presses ESCAPE, or the menu closes.
27. All submenus are destroyed when the root menu closes.

### Visual Behavior & Animations

28. Menus fade in and slide slightly upward (or in the appropriate direction based on available screen space) when opened.
29. Menus fade out when closed.
30. Menu items highlight with a rounded-rect fill on selection.
31. Disabled items do not respond to highlight on hover.

### Menu Closure & Callbacks

32. The menu fires an `on_item_click` callback with the selected action item's ID when the user activates an action.
33. The menu fires an `on_close` callback when the menu closes (either by user action or loss of focus).
34. After an action is activated, the menu closes automatically.

### Lifecycle

35. A context menu is created with an initial list of items.
36. The menu is shown by calling `show()` with a parent surface (layer-shell or xdg_surface) and xdg_positioner.
37. The menu is hidden by calling `hide()` or `hide_animated()`, which destroys all popup surfaces and resets internal state.
38. Once hidden, the menu can be shown again (or destroyed and recreated).

## Constraints & Edge Cases

- **No parent surface:** The menu cannot be shown without a valid parent surface and positioner.
- **Items change while menu is open:** The current implementation does not support dynamic item updates while the menu is visible. Close and re-open to refresh the menu.
- **Submenu of submenu:** Submenus can themselves contain submenus, with unlimited nesting depth. Selection state correctly excludes all unrelated depths.
- **Empty menu:** A menu with no selectable items is not useful and should not be shown; the parent (e.g., a tray icon) should not display a context menu trigger if there are no items.
- **Very long menu:** A menu given a `max_height` (as a dropdown's is) is capped at it and its list scrolls inside that box, under the wheel, with the keyboard selection, or by typing a row's name; scroll arrows mark which ends have more. Without a cap, an over-tall menu is positioned above the anchor point or cropped by the xdg_positioner. Only the rows intersecting the visible band are built and drawn — a font list is over a thousand rows while a dozen are on screen, and each row costs a clone and a text layout whether or not the clip then throws it away.
- **HiDPI / fractional scaling:** Menu dimensions are in logical pixels; the compositor handles scaling to physical pixels.
- **RTL locales:** Submenus appear to the left instead of right. The xdg_positioner and parent logic handles this.
- **Pointer leaves menu tree during submenu open:** The submenu remains visible until explicitly closed or the root menu loses focus.
- **Duplicate item IDs:** If two action items have the same ID, the callback will be called with that ID; the parent must disambiguate by context if needed.

## Rationale

- **Popup surfaces without grab (submenus):** Submenu popups do not request a keyboard grab because the root menu already holds the grab. Requesting a grab at any depth would cause xdg_popup.grab semantics to steal focus, which closes the menu. Instead, submenus forward all input to the root menu's event handler.
- **Only one item selected tree-wide:** This matches standard desktop menu behavior (e.g., macOS, GNOME). It avoids visual confusion from multiple highlights and makes keyboard navigation intuitive (UP/DOWN move between the current depth's items, and moving between depths clears parent selections).
- **Submenu delay on mouse hover:** A configurable delay (not instant open) prevents submenus from opening unintentionally when the user is just moving through the menu. This is standard UX from macOS and GNOME.
- **Type-ahead over arrows:** a list of every installed font cannot be walked with the arrows or the wheel in any reasonable time, and a pop-up button has no room for a search field. Typing the start of a name is what makes such a menu usable, and it is what every desktop's pop-up buttons have done for decades.
- **Scrolling only when capped:** Most menus fit on screen and are left to the xdg_positioner. A list that cannot — every installed font, every cursor theme — sets a `max_height` and scrolls inside it, rather than making the compositor slide a full-height popup around.
- **Client-side dismissal on same-client clicks:** `xdg_popup.grab` is an owner-events grab — the compositor keeps delivering input to the grabbing client and only dismisses the popup when input goes to another client. This is what lets a menu span several of its own surfaces. The cost is that "click elsewhere in my own window closes the menu" cannot come from the compositor and has to be implemented by the menu, which is why `ContextMenu` watches every pointer batch, not only the ones landing on its own popups, and acts on what it saw through `AppContext::register_pointer_batch_end_callback` once the batch is fully spoken for. This was invisible while every menu hung off a layer-shell surface (there is nothing else of that client to click); it surfaced the first time a menu was parented to an ordinary toplevel.
- **Keyboard focus held by root popup:** This simplifies input handling and ensures the menu behaves as a cohesive unit, even with multiple popup surfaces.

## Open Questions

- Should items support mnemonics (e.g., underlined character) for keyboard activation, or only full shortcut bindings?
- Should the menu support custom rendering hooks (e.g., app-specific item appearance) beyond the standard item types?
- Should menu item activation (click) be immediate or debounced to avoid accidental multiple activations in rapid clicks?
