# Phase 4 — Windows + expose on a shared overlay plane

**Status**: ✅ done (active, needs visual confirmation)  
**Effort**: 2 days  
**Depends on**: Phase 1, Phase 2

## Goal

Render all non-top windows into a single `SceneDmabufElement` overlay plane.
Expose mode reuses the same element via `expose_dmabuf_element`.

## What was built

### `SurfaceData` additions (`src/udev/types.rs`)

```rust
pub(super) windows_dmabuf_element: Option<SceneDmabufElement>,
pub(super) expose_dmabuf_element:  Option<SceneDmabufElement>,
```

### Render path (`src/udev/render.rs`)

Both allocated with `Fourcc::Argb8888`, `opaque: false`.

**Expose / windows mutual exclusion**:
```rust
if expose_active {
    push_plane!(surface.expose_dmabuf_element.clone());
} else {
    // top window on its own scanout plane (when no overlay UI active)
    if !overlay_ui_active {
        // render_elements_from_surface_tree for top window
    }
    push_plane!(surface.windows_dmabuf_element.clone());
}
```

`overlay_ui_active = app_switcher.alive() || osd.is_visible()`. When active,
top window is not added as a scanout candidate so the overlay UI plane is
unobstructed.

NodeRefs:
- `windows_dmabuf_element` → `ows.windows_plane.id`
- `expose_dmabuf_element` → `ows.expose_layer.id`

## Open / remaining

- Top window exclusion from `windows_plane` subtree: currently the top window's
  layer is still in `windows_plane`. It renders twice (once in windows_plane,
  once as a surface scanout candidate). Need to hide the top window layer
  from `windows_plane` render or use a dedicated container.
- Expose plane rendering accuracy: verify expose grid renders into
  `expose_layer` correctly.
