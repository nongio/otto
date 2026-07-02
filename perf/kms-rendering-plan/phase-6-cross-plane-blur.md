# Phase 6 — Cross-plane backdrop blur

**Status**: todo  
**Effort**: 2 days  
**Depends on**: Phase 1, Phase 3 (background dmabuf), Phase 5 (dock dmabuf)

## Goal

Restore background blur (frosted glass) for the dock and overlay UI.
With separate planes, the dock's Skia surface can't sample pixels from planes
below it directly — we reimport those dmabufs as `skia::Image` objects to
reconstruct the backdrop.

## Implementation

### 1. `SkiaRenderer::import_image_from_dmabuf`

```rust
pub fn import_image_from_dmabuf(
    &mut self,
    dmabuf: &Dmabuf,
) -> Result<skia::Image, GlesError> {
    let egl_image = self.egl_context().display()
        .create_image_from_dmabuf(dmabuf)
        .map_err(GlesError::BindBufferEGLError)?;
    let tex = self.import_egl_image(egl_image, false, None)?;
    self.import_skia_image_from_texture(&tex, false)
        .ok_or(GlesError::MappingError)
}
```

All primitives already exist (`import_egl_image`, `import_skia_image_from_texture`).
`import_skia_image_from_texture` needs `pub(crate)` visibility.

### 2. Blur pass in SceneDmabufElement::update()

For elements that need backdrop blur (dock, overlay UI), before rendering
own content:

```rust
// 1. Import lower plane dmabufs as Skia images
let bg_image  = renderer.import_image_from_dmabuf(background_plane.current_dmabuf())?;
let win_image = renderer.import_image_from_dmabuf(windows_plane.current_dmabuf())?;

// 2. Composite lower planes into a scratch surface (blur region only)
let blur_rect = dock_blur_region(); // small rect, not full output
let mut scratch = SkiaSurface::new(blur_rect.size);
scratch.canvas().draw_image(&bg_image,  -blur_rect.origin, None);
scratch.canvas().draw_image(&win_image, -blur_rect.origin, None);

// 3. Apply blur
let blurred = scratch.image_snapshot()
    .new_with_filter(&image_filters::blur((20.0, 20.0), None, None, None), ...);

// 4. Draw blurred backdrop, then own content on top
canvas.draw_image(&blurred, blur_rect.origin, None);
// ... render dock layers ...
```

### 3. Cache invalidation

Track `last_bg_commit: CommitCounter` and `last_win_commit: CommitCounter`.
Only redo the blur blit if either lower plane has advanced its commit counter.
Static background + static windows = zero extra GPU work.

### 4. Lower plane references

Pass `&SceneDmabufElement` slices for planes below into the element at
construction. Store as `Arc` references to avoid lifetime issues.

## Scope

- Dock frosted glass blur.
- App switcher / workspace selector frosted glass.
- Any future overlay with `backdrop-filter`-style blur.

## Validation

- Dock shows blurred background content behind it.
- Changing background (e.g. wallpaper) updates the dock blur on next frame.
- No blur re-computation when background and windows are static.
