# Phase 3 — Background on primary plane

**Status**: ✅ done (active, needs visual confirmation after sync fix)  
**Effort**: 2 days  
**Depends on**: Phase 1, Phase 2

## Goal

Render `background_plane` into a `SceneDmabufElement` swapchain and expose it
as the primary plane via `UnderlyingStorage::Dmabuf`.

## What was built

### `SurfaceData` addition (`src/udev/types.rs`)

```rust
pub(super) scene_dmabuf_element: Option<SceneDmabufElement>,
```

### Render path (`src/udev/render.rs`)

- Allocated with `Fourcc::Abgr2101010`, `opaque: false`.
  (10-bit format for quality; `opaque: false` because `scene_element` was
  removed — no fallback needs occlusion culling.)
- NodeRef set to `ows.background_plane.id` each frame.
- Pushed last (lowest z-order) so Smithay assigns it to the primary plane.
- `submitted()` called at VBlank.

### Why `opaque: false`

When `opaque: true`, the damage tracker skips rendering elements below the
background plane (the old `scene_element`). If plane assignment failed and
`draw()` was a no-op, the primary plane showed black. Setting `opaque: false`
allows the GPU composite fallback to render correctly. `scene_element` has since
been removed from the normal path; `opaque` stays `false` as it is no longer
relevant to performance.

## Open / remaining

- Confirm visually: no black background, no artifacts on first frame.
- Damage-skip not yet implemented: re-renders every frame even if background
  is static. See "Damage optimisation" notes in Phase 1.
