# Phase 5 — Dock + Overlay UI on overlay planes

**Status**: ✅ done (active, needs visual confirmation)  
**Effort**: 2 days  
**Depends on**: Phase 1, Phase 2

## Goal

Render dock and overlay UI each into their own `SceneDmabufElement` overlay plane.

## What was built

### `SurfaceData` additions (`src/udev/types.rs`)

```rust
pub(super) overlay_dmabuf_element: Option<SceneDmabufElement>,
pub(super) dock_dmabuf_element:    Option<SceneDmabufElement>,
```

### Render path (`src/udev/render.rs`)

Both allocated with `Fourcc::Argb8888`, `opaque: false`.

NodeRefs:
- `overlay_dmabuf_element` → `ows.overlay_plane.id`
- `dock_dmabuf_element` → `ows.dock_plane.id` (primary output only)

Element order (top → bottom z):
```
cursor
dock_dmabuf_element
overlay_dmabuf_element
top_window surface (when !overlay_ui_active)
windows_dmabuf_element  (or expose_dmabuf_element)
scene_dmabuf_element    (background, primary plane)
```

`overlay_plane` contains: `layer_shell_top`, `workspace_selector`,
`app_switcher`, `layer_shell_overlay`, `overlay_layer` (DnD+OSD),
`popup_overlay`.

`overlay_plane` is sized to full output dimensions so all children can be
positioned in scene-space without extra offset arithmetic.

## Constraints

- Dock background blur: deferred to Phase 6. Dock renders without blur for now.
- App switcher frosted glass: same deferral.

## Open / remaining

- Dock plane is only allocated for the primary output. Secondary outputs with
  docks are not handled.
- Confirm overlay_plane children z-order is correct (popups above app switcher).
