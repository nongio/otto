# KMS Plane Utilisation Plan

Hardware: Intel GPU, 8 planes per CRTC — 1 Primary + 6 Overlays + 1 Cursor.

When an element is assigned to a plane the display controller composites it in
hardware. The GPU does zero work for that surface on idle frames.

---

## Target plane layout (per output, bottom → top)

### Tier3 layout (confirmed on eDP-1, i915 Tiger Lake)

```
Z  KMS plane   Render element                     lay-rs subtree root
─────────────────────────────────────────────────────────────────────
3  Overlay 2   overlay_dmabuf_element             overlay_plane  (UI + dock combined)
2  Overlay 1   top_window_dmabuf_element          scanout_windows
1  Overlay 0   windows_dmabuf_element             windows_plane
               OR expose_dmabuf_element           expose_plane   (mutually exclusive)
0  Primary     scene_dmabuf_element               background_plane
   ─────────────────────────────────────────────────────────────────
   Cursor      cursor texture                     (no lay-rs node)
```

**Dock is a sublayer of overlay_plane** (not a separate plane). The combined
overlay captures: app switcher, workspace selector, layer_shell_overlay, OSD,
DnD, popups, and dock — all composited into one dmabuf by the GPU, then
scanned out on Overlay 2.

### Conditional presence

- **Overlay 2 (UI+dock)**: always pushed when tier ≥ Tier1. Even when no
  overlay UI is active, dock is still present in the subtree. The per-plane
  damage skip means idle frames cost nothing.
- **Overlay 1 (top window)**: pushed only when `scanout_windows` is non-empty
  (i.e. at least one window is parked there). Tier ≥ Tier3.
- **Overlay 0 (windows/expose)**: pushed when there are non-parked windows.
  Tier ≥ Tier2. Skipped entirely when all windows are in scanout_windows.
- **Primary (background)**: pushed as separate plane when tier ≥ Tier2.
  Below Tier2 the full GPU-composited frame goes to primary as usual.

### Lower-tier fallbacks

| Tier     | Primary       | Overlay 0     | Overlay 1   | Overlay 2    |
|----------|---------------|---------------|-------------|--------------|
| Fallback | full scene    | —             | —           | —            |
| Tier1    | full scene    | —             | —           | UI+dock      |
| Tier2    | background    | windows/expose| —           | UI+dock      |
| Tier3    | background    | windows/expose| top window  | UI+dock      |

---

## Implementation status

| Phase | Description                        | Status |
|-------|------------------------------------|--------|
| 1     | SceneDmabufElement swapchain infra | ✅ done |
| 2     | Layer restructuring (plane groups) | ✅ done |
| 3     | Background on primary plane        | ✅ done |
| 4     | Windows + expose overlay plane     | ✅ done |
| 5     | Dock + overlay UI planes           | ✅ done |
| —     | Per-plane damage skip              | ✅ done |
| —     | KMS tier grading (TEST_ONLY probe) | ✅ done — Tier3 confirmed, Tier4 rejected (i915 Tiger Lake eDP-1, 6 overlays) |
| 8     | Tier activation (wire tiers to render) | ✅ done |
| 6     | Cross-plane backdrop blur          | 🔲 todo |
| 7     | Telemetry + frame callbacks        | 🔲 todo |

---

## Phase 8 — Tier activation

Replace all `DBG_PLANE_*` keyboard guards with tier-based automatic enabling.
Debug flags remain as manual overrides (OR-combined with tier check).

### Step 1 — Move dock into overlay_plane subtree (`workspaces/mod.rs`)

Currently dock.wrap_layer and overlay_plane are siblings under output_layer:

```
output_layer
  ├── dock.wrap_layer       ← to be moved
  └── overlay_plane
        └── ... ui children
```

Change (primary output path only):

```
output_layer
  └── overlay_plane
        ├── ... ui children
        └── dock.wrap_layer   ← added last = topmost in subtree
```

`overlay_dmabuf_element` already renders `overlay_plane`, so it will
automatically capture dock after this change. The `dock_dmabuf_element` field
and its alloc/push code in render.rs can be removed.

### Step 2 — Wire tier gates (`udev/render.rs`)

Compute once per frame, near the top of the plane-push block:

```rust
let tier = surface.current_tier.unwrap_or(RenderTier::Fallback);
let use_bg_plane      = matches!(tier, Tier2 | Tier3 | Tier4);
let use_windows_plane = matches!(tier, Tier2 | Tier3 | Tier4);
let use_topwin_plane  = matches!(tier, Tier3 | Tier4);
let use_ui_dock_plane = matches!(tier, Tier1 | Tier2 | Tier3 | Tier4);
```

Replace each `DBG_PLANE_X.load(…)` with `use_X_plane || DBG_PLANE_X.load(…)`.

Push order (top → bottom so Smithay assigns overlays front-first):
1. `overlay_dmabuf_element` — when `use_ui_dock_plane`
2. `top_window_dmabuf_element` — when `use_topwin_plane && scanout_windows non-empty`
3. `expose_dmabuf_element` — when `use_windows_plane && expose_active`
4. `windows_dmabuf_element` — when `use_windows_plane && !expose_active && !all_parked`
5. `scene_dmabuf_element` — when `use_bg_plane`

### Step 3 — Remove dock_dmabuf_element

After Step 1, `dock_dmabuf_element` is redundant. Remove:
- `SurfaceData::dock_dmabuf_element` field (`types.rs`)
- `alloc_plane!(surface.dock_dmabuf_element, …)` alloc block (`render.rs`)
- `el.set_node_ref(dock_layer.id)` node assignment (`render.rs`)
- The unconditional `push_plane!(surface.dock_dmabuf_element.clone())` (`render.rs`)

### Open issues (deferred past phase 8)

- **draw() fallback**: `SceneDmabufElement::draw()` is a no-op. If a plane
  fails KMS assignment at runtime the layer goes black. Acceptable short-term
  since the tier probe gates entry. Long-term: GPU blit of dmabuf content.
- **Tier re-probe on mode change**: `current_tier` is never cleared after a
  CRTC mode change. Add a reset hook so the probe re-runs if the display mode
  changes (e.g. external monitor hotplug).

---

## Per-plane damage skip

**Implemented** in `src/render_elements/scene_dmabuf_element.rs`.

Each `SceneDmabufElement::render()` calls `engine.subtree_damage(node_ref)` at
the top. If the subtree has no damage and a valid dmabuf already exists, `render()`
returns early without touching the swapchain or advancing `commit_counter`.
Smithay's `damage_since()` therefore returns empty and the DRM compositor reuses
the existing framebuffer — zero GPU and zero KMS work on idle planes.

---

## Phase 6 — Cross-plane backdrop blur

See `phase-6-cross-plane-blur.md`. Deferred until phase 8 is confirmed
stable. Requires `SkiaRenderer::import_image_from_dmabuf`.

## Phase 7 — Telemetry + frame callbacks

See `phase-7-polish.md`. Plane assignment success rate logging and
frame-callback rate tied to plane assignment state.
