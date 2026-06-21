# Window Scanout Promotion

**Status:** draft  
**Related specs:** window-tiling, workspaces-multi-output

## Summary

A performance feature that scans out the client buffer of eligible non-fullscreen windows directly on dedicated display-hardware overlay planes, so the compositor does zero GPU compositing while a window's content updates (video, scrolling, games). The static frame — wallpaper, window shadows, chrome, and non-promoted windows — is composited once and is not re-rendered while a promoted window's content changes; only that window's plane flips. This generalises the existing fullscreen direct-scanout behaviour to floating, maximized, and tiled windows.

## Goals

- Display an eligible window's content via a dedicated display-hardware plane instead of compositing it, while its content keeps updating.
- Avoid re-rendering the static frame when only a promoted window's content changes (a content-only buffer update must not trigger a full recomposite).
- Keep each promoted window's shadow and rounded-corner cutout drawn in the composited frame even while its content scans out.
- Promote multiple windows simultaneously when they are mutually disjoint and unoccluded.
- Guarantee tear-free presentation: a promoted buffer is shown only after the client's render into it has completed.
- Fall back transparently to normal compositing for any window that cannot be promoted, with no visible glitch on promotion or demotion.
- Run tiled windows at full frame rate so video in a non-focused tile does not judder.

## Non-Goals

- Scanning out wlr-layer-shell surfaces (bars, docks, panels, notifications) on their own planes. (Phase 2.)
- Promoting windows that have client subsurfaces above the main buffer.
- Preserving compositor-applied rounded content corners while a window is promoted (square corners are accepted while promoted).
- Promoting more windows than the display hardware has free planes for (the rest stay composited).
- Any user-facing configuration of which windows are promoted.

## Behavior

Promotion is recomputed every frame. A window that becomes eligible is promoted; one that loses eligibility is demoted back to normal compositing the same frame.

### Which windows are promoted (selection)

Eligible windows are considered top-to-bottom (frontmost first):

- The **topmost** eligible regular window is always promoted.
- A **lower** eligible window is promoted only if its rectangle is disjoint from every window above it (whether or not those above are themselves promoted).
- Result: promoted windows are mutually disjoint and unoccluded; each rides an independent plane. Nothing is ever drawn above a promoted window's rectangle.

A window is eligible for promotion only when all of the following hold:

- It is mapped (not unmapped).
- It is not minimized and not running a minimize/restore animation.
- It has no active per-window effect — full opacity, no in-progress scale/genie transform.
- It has no client subsurfaces above its main buffer.
- It is opaque-ish (translucent windows are still allowed; the system blends correctly based on the buffer's opaque region).
- Its rectangle is not behind (overlapped by) a visible layer-shell **Top or Overlay** surface — panels, the top bar, or a notification daemon's surfaces — nor the dock when visible. This is overlap-aware: the always-present top bar only blocks the windows it actually covers, not every window.
- It has no open popup, unless that popup is itself promoted to a higher plane.

### When scanout is globally disabled

In any of these situations no window is promoted this frame; everything composites normally:

- A window is in fullscreen mode, or its fullscreen transition is animating (the dedicated fullscreen scanout path owns that case).
- Expose / show-all is open or transitioning (workspace-selection mode).
- The window-selection picker (expose picker) is active.
- The app switcher is visible.
- An OSD is visible.
- A context menu is open.
- A workspace-selection transition is in progress.
- A workspace swipe gesture is active.
- The tiling drop-zone preview overlay is showing.
- The dock has a context menu open, is hovered/magnifying, or is running a layout animation.
- A screenshot capture is pending (screencopy) or a recording session is active (screenshare) — capture must read a fully-composited frame, so planes are folded back in.

### Shadows and static-frame stability

- While a window is promoted, its content is hidden from the composited scene but its shadow (and rounded-corner cutout) stays drawn in the scene.
- A buffer commit on a promoted window must not trigger a recomposite of the static frame; only that window's plane is flipped.
- Moving a promoted window or changing its focus/activation must not rebuild the static frame (position is applied as a transform, focus opacity as a layer opacity — neither re-rasterises the shadow).
- The static frame is rebuilt only on a genuine change: window resize, chrome change, or a change to a non-promoted window.

### Tear-free guarantee

- A promoted buffer is flipped to its plane only after the client signals that its render into that buffer is complete.
- For clients using explicit synchronisation (`wp_linux_drm_syncobj`), the surface commit is held until the client's acquire fence signals before the plane flip; the plane's in-fence reflects the real completion, not an assumed-signalled one.
- For clients using implicit synchronisation, the commit is held on the buffer's implicit fence as before.
- Setting `OTTO_NO_EXPLICIT_SYNC=1` disables the explicit-sync protocol so clients fall back to implicit fences (a diagnostic escape hatch; with the fence handling correct it is no longer required for tear-free output).

### Demotion

- **Plane assignment fails the same frame:** the window's current buffer is composited above the static frame instead — z-correct because nothing is ever drawn above a promoted window's rectangle. A frame is drawn whenever a promoted window has new surface damage, so its plane flips (near-zero GPU) or it composites.
- **Cross-frame demotion** (a global gate trips, or the window starts animating / gains a subsurface / overlaps something): the window's current buffer is re-imported into the composited scene before its content is unhidden, so the first composited frame after demotion shows current content, never a stale frame.

### Tiled windows

- A tiled window (snapped to a half or maximized) runs at full output-refresh frame rate, not the reduced unfocused rate, so video in a non-focused tile stays smooth.

## Constraints & Edge Cases

- **No overlapping planes:** the selection rule guarantees promoted windows never overlap, so the feature does not depend on display-hardware plane blending, z-position mutability, or per-device blend support.
- **Square corners while promoted:** a promoted window carries the client's square buffer; any compositor-applied content rounding is dropped while promoted and restored on demotion. The shadow's rounded cutout remains.
- **Behind a layer surface:** a window overlapped by a visible Top/Overlay layer-shell surface (panels, the top bar, notification daemons) or the dock cannot be promoted — its scanout plane would otherwise sit above that chrome and hide it. The check is per-window overlap, not a blanket gate, so chrome only blocks the windows it actually covers.
- **Popups:** a popup over a promoted window must itself be promoted to a higher plane; otherwise the whole stack must demote to a composited frame.
- **Capture mid-playback:** an in-flight screenshot or recording forces a fully-composited frame for that capture; there is a single-frame mode transition when entering/leaving scanout.
- **Maximized buffer smaller than output:** plane source/destination geometry must match the committed buffer so the plane never samples outside the buffer (which would show black strips) and a smaller buffer is positioned correctly.
- **Buffer lifetime:** a scanned-out buffer must be held until it is off the plane, so the client does not recycle it while it is still being displayed.

## Rationale

- **Disjoint-and-unoccluded selection** keeps every promoted window on an independent plane with nothing drawn above it. This makes the GPU-composite fallback z-correct, removes any dependence on hardware plane blending, and prevents a lower window peeking through a higher window's rounded corner.
- **Skipping the scene import while promoted** is what actually prevents the static-frame recomposite: hiding the content layer alone is not enough because any scene transaction still marks the scene dirty; not feeding the buffer into the scene at all is what keeps the static frame stable.
- **Shadow stays composited** so windows keep their depth cue and rounded corners even though the square client buffer is on a plane — matching the effects-off-while-promoted approach used by other compositors.
- **Re-import on demotion** is required because the buffer was never fed into the scene while promoted; without it the first composited frame after demotion would be stale.
- **Holding the commit on the explicit acquire fence** is mandatory under explicit sync: the display path bypasses the GL finish that masked the race on the composite path, so without waiting on the real fence the plane flip would race the client's GPU and tear.
- **Tiled windows forced to full rate** because both halves are primary targets; the reduced unfocused rate judders video in the non-active half.
- **Global capture/overlay/swipe gates** exist because capture must read a complete composited frame and overlay UI / in-motion layouts cannot be represented as disjoint planes.

## Open Questions

- Should compositor-applied content rounding be preserved while promoted (e.g. via a hardware rounding capability), rather than accepting square corners?
- Should layer-shell surfaces be promoted (phase 2), and if so how should scarce planes be prioritised between a frequently-updating window and a rarely-updating bar?
- When planes are scarce, by what priority should promotion be assigned (e.g. by update frequency) rather than purely top-to-bottom?
- Should promotion respect a per-window or global configuration toggle, or remain fully automatic?
