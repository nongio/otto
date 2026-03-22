# Top Bar

**Status:** draft  
**Related specs:** dynamic-island.md, notification-daemon.md

## Summary

The Top Bar is a persistent, full-width panel anchored to the top edge of the primary output. It provides three functional zones: a left zone showing the active application name and its global menu, a center zone intentionally kept minimal to leave visual space for the Dynamic Island, and a right zone hosting system tray icons and a clock. The bar is a standalone Wayland client application built with otto-kit, using standard compositor protocols for placement and window tracking.

## Goals

- Display the focused window's application name and global menu in a macOS-style menu bar on the left.
- Host StatusNotifierItem (SNI) tray icons on the right, with tooltips and context menus.
- Show a system clock on the far right.
- Coexist with the Dynamic Island by leaving the horizontal center of the bar visually unoccupied.
- Remain compositor-agnostic: use only standard Wayland protocols and D-Bus interfaces.
- Support both native Wayland apps (menu via dbusmenu D-Bus) and XWayland legacy apps.
- Respond to theme changes (light/dark) without restart.

## Non-Goals

- Replacing or competing with the Dynamic Island — the bar and island are independent elements occupying different horizontal regions of the screen.
- Hosting notifications or live activities — those belong to the Dynamic Island.
- Embedding full window management controls (workspace switcher, window list) — those are dock/expose concerns.
- Implementing a custom Wayland menu protocol — dbusmenu over D-Bus is sufficient for both Wayland and XWayland clients.

## Behavior

### Placement & Sizing

1. The bar is anchored to the top edge of the screen, full-width, with a fixed height (default 28 logical points, configurable).
2. The bar declares an exclusive zone equal to its height so that maximized windows and layer-shell clients do not overlap it.
3. On multi-output setups, the bar appears on the primary output only (where the Dynamic Island is also shown). A separate, minimal bar (clock + tray only) may optionally appear on secondary outputs.
4. The bar surface uses the `otto-surface-style-unstable-v1` protocol to apply compositor-composited visual properties:
   - **Background blur**: `set_blend_mode(background_blur)` — the compositor blurs content behind the bar surface, producing a frosted-glass effect.
   - **Rounded corners**: `set_corner_radius` applied to the bottom-left and bottom-right corners only (top corners are flush with the screen edge).
   - **Drop shadow**: `set_shadow` with a soft downward offset to lift the bar off the desktop.
   - **Border**: `set_border` with a subtle 1 logical-point separator at the bottom edge using a theme-adaptive color.
5. The bar's background fill color (drawn by Skia onto the surface before compositing) uses a semi-transparent material color from otto-kit's `Theme`, chosen based on the active color scheme.
6. The bar adapts to system light/dark mode by reading `org.freedesktop.appearance color-scheme` from the XDG Settings portal (`org.freedesktop.portal.Settings`). It listens for `SettingChanged` signals and re-applies theme colors within one second. Color scheme values: `1` = prefer dark, `2` = prefer light.

### Layout: Three Zones

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  [App Name]  [File] [Edit] [View] [Help]  ··· [island] ···  [icons]  [clock] │
└──────────────────────────────────────────────────────────────────────────────┘
  ◄── Left zone ──────────────────►         ◄── Right zone ──────────────────►
                                   ◄Center►
```

5. **Left zone** (left-aligned): application name (bold), followed by top-level menu entries (File, Edit, …). Clicking a top-level entry opens the corresponding submenu as a popup.
6. **Center zone**: reserved empty space. No content is rendered here to leave visual room for the Dynamic Island.
7. **Right zone** (right-aligned): SNI tray icons (rightmost first), then clock.

### Active Window Tracking

8. The bar tracks the currently focused window using the `ext-foreign-toplevel-list-v1` Wayland protocol. When focus changes, the bar updates the left zone within one frame.
9. If no window is focused, the left zone shows the desktop/compositor name without any menu entries.
10. The application name shown is derived from the `app_id` of the focused toplevel (mapped to a human-readable name via the desktop entry database).

### Global Menu (Left Zone)

11. Menu data is sourced via the `com.canonical.dbusmenu` D-Bus interface. The bar looks for the menu at the well-known bus name registered for the focused window.
12. The bar queries the menu structure once per focus change and caches it. It listens for `ItemsPropertiesUpdated` and `LayoutUpdated` signals to refresh the cache incrementally.
13. Clicking a top-level menu entry renders a dropdown popup as a new layer-shell surface (`overlay` layer) positioned below the bar at the correct horizontal offset.
14. Keyboard navigation within a menu follows standard conventions: arrow keys move selection, Enter activates, Escape closes. The bar requests keyboard grab from the compositor while a menu is open.
15. If no dbusmenu is registered for the focused app, the left zone shows only the application name with no menu entries.
16. Menu entries support: labels, icons, keyboard shortcuts (displayed right-aligned), separators, checkboxes, radio groups, and submenus.
17. Disabled menu entries are rendered at reduced opacity and do not respond to activation.

### System Tray — StatusNotifierItem (Right Zone)

18. The bar implements the `org.kde.StatusNotifierHost` D-Bus role and monitors `org.kde.StatusNotifierWatcher` for icon registrations.
19. Each registered SNI item is rendered as an icon in the right zone. Icons are ordered by registration time, newest leftmost.
20. Tray icon images are fetched via the SNI `IconPixmap` or `IconThemePath`/`IconName` properties. A fallback generic icon is shown if none is available.
21. On hover, a tooltip is shown below the tray icon, sourced from the SNI `ToolTip` property.
22. Left-click on a tray icon calls the SNI `Activate(x, y)` method. Right-click calls `ContextMenu(x, y)` and renders the resulting dbusmenu popup.
23. Middle-click calls the SNI `SecondaryActivate(x, y)` method.
24. The bar listens for `NewIcon`, `NewStatus`, and `NewToolTip` signals to update icons without polling.
25. Icons with `Status = Passive` may be hidden by user configuration (hidden icons tray, revealed on click of a chevron button).

### Clock (Right Zone)

26. The clock displays the current local time. Default format: `HH:MM` (24-hour). Configurable to include date or switch to 12-hour format.
27. Clicking the clock opens a calendar popup (future milestone; not in initial implementation).

### Animations & Visual Behavior

28. When a new SNI icon registers, it slides in from the right with a spring animation.
29. When an SNI icon deregisters, it fades out and the remaining icons slide to close the gap.
30. Menu entry highlight uses a rounded-rect fill with spring-based scale feedback on press.
31. Menus open with a fade-in + slight upward slide. Menus close with a fade-out.
32. The right panel width animates smoothly when tray icons are added or removed, using an exponential ease-out interpolation. On the first frame after creation, the width snaps to the content-driven target without animating.

## Constraints & Edge Cases

- **No focused window:** Show compositor/desktop name, no menu entries. Tray and clock remain visible.
- **App has no dbusmenu:** Show only app name, no menu entries. Do not show an empty menu bar.
- **SNI watcher absent:** Tray section is hidden. The bar must not crash — re-probe every 30 seconds.
- **Menu root changes while open:** Close the current open menu and re-fetch before re-opening.
- **HiDPI / fractional scaling:** The bar must render at the output's native scale. All sizes are in logical points; the bar converts to physical pixels using the output's scale factor.
- **Theme change:** Re-apply colors within one second without restarting. Use the color-scheme D-Bus portal (`org.freedesktop.impl.portal.Settings`) to track system theme.
- **Multiple monitors:** Primary-output bar shows all three zones. Secondary-output bars (if enabled) show only tray + clock.
- **Right-to-left locales:** Zone order reverses (right zone on left, left zone on right). Menu popup alignment mirrors accordingly.

## Rationale

- **External Wayland client (not embedded in compositor):** Makes the bar replaceable, testable in isolation, and free of compositor coupling. Standard protocols (layer-shell, foreign-toplevel, dbusmenu, SNI) provide all necessary integration surface.
- **dbusmenu over a custom Wayland protocol:** dbusmenu is already implemented by GTK, Qt, and Electron apps on Linux. A custom Wayland menu protocol would require patching all toolkits.
- **Compositor-side focus tracking via ext-foreign-toplevel-list:** This protocol provides reliable, race-free focus events and app_id without requiring any cooperation from the client app.
- **Center zone reserved for Dynamic Island:** The Dynamic Island spec places a pill in the top-center. Keeping the bar center empty avoids visual clutter and text-behind-pill rendering artifacts.
- **SNI over Wayland tray protocols:** No stable Wayland tray protocol exists. SNI (via D-Bus) is the de-facto standard supported by virtually all Linux desktop apps.

## Open Questions

- Should the bar appear on secondary outputs at all, and if so what content?
- Should "passive" SNI icons be hidden by default or shown? What triggers the reveal chevron?
- How should the bar coordinate its exclusive zone with the Dynamic Island's vertical offset? The island floats on top of the bar; the bar's exclusive zone height may need to be taller than the bar itself to account for the island's expanded state.
- Should the clock include a calendar popup in the initial implementation or defer to a later milestone?
- How should the bar handle apps that register a dbusmenu but the bus name goes away while the menu is open?
