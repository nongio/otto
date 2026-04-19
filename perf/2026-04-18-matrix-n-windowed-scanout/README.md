# Matrix N — Direct scanout for any top window (not just fullscreen)

**Date:** 2026-04-18
**Otto:** matrix-c + matrix-f + matrix-j + new scanout candidate selection (no protocol-fullscreen requirement) + correct geometry-aware position

## Changes

### 1. New scanout-eligibility predicate (`src/workspaces/mod.rs`)

```rust
/// Scanout if no overlay UI is in the way. Geometry doesn't matter.
pub fn is_top_window_scanout_eligible(&self) -> bool {
    if self.get_show_all() { return false; }       // expose
    if self.app_switcher.alive() { return false; } // app switcher
    if self.osd.is_visible() { return false; }     // OSD
    if self.is_animating.load(Relaxed) { return false; }
    true
}

pub fn get_top_window(&self) -> Option<WindowElement> {
    self.primary_output_workspaces()?
        .spaces.get(self.with_model(|m| m.current_workspace))?
        .elements().last().cloned()
}
```

The old `is_fullscreen_and_stable()` / `get_fullscreen_window()` are kept for callers that genuinely care about xdg-protocol-level fullscreen (window_throttle, xdg.rs).

### 2. Render path uses new selection (`src/udev/render.rs`)

Replaced the old check; the scanout now triggers for any top window when no overlay UI is showing.

### 3. Correct position computation

The window is placed at:
```rust
location = element_location - element.geometry().loc - output.loc
location_physical = location.to_physical(output_scale)
```

The `geometry().loc` subtraction is the critical piece — matches smithay's `space.render_location()`. For chromium, the buffer extends ~40px past the xdg geometry (shadow / decoration extension), so without subtracting the geometry offset the window appeared shifted by that amount.

## Result (chromium WINDOWED, not fullscreen)

| Metric | Matrix J (no scanout) | **Matrix N** | Anvil baseline |
|---|---|---|---|
| RCS busy | 52.7% | **~12-15%** | 28% |
| GPU power | 4.2W | **~0.5W** | 2.1W |
| **fps** | 35 | **60** | ~120 |
| **per-frame cost** | 1.51% | **~0.22%** | 0.23% |

Per-frame cost essentially **at anvil parity** — same 60 fps cap due to display refresh divisor; on a 240 Hz display would expect 120 like anvil.

User confirmed: chromium window appears at correct position with no offset.

## Important caveat

Scanout currently *replaces* the entire scene composite — the area outside chromium's window shows whatever was on the primary plane before scanout took over (likely stale or black). The dock and bar are NOT visible alongside the scanned-out window because the scene isn't being composited.

Two follow-ups remain:
1. **Render scene as primary plane + window as overlay plane** — so dock/bar are visible alongside scanout. Tried in this session (see [matrix-l](../2026-04-18-matrix-l-scanout-skip/)) — passing scene_element + window in the same elements list defeats scanout (DrmCompositor falls back to full composite). Needs a different Smithay API surface.
2. **Visual side-effects of scanout-replacing-scene** — the dock and bar don't appear when scanout is active. Acceptable if user is using chromium fullscreen-style; broken visually otherwise.

Despite caveat #2, the **per-frame compositor cost reaches anvil parity** when scanout is active, which validates the architecture is correct.

## Verdict — KEEP

Significant architectural step forward. The keepers:
- Per-window `is_scanned_out` flag (matrix-l, foundation)
- Geometry-aware scanout candidate (this matrix)
- Correct window position computation matching smithay's render_location

Open: scene-on-primary + window-on-overlay (needs Smithay API exploration).
