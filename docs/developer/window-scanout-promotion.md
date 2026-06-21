# Window scanout promotion (non-fullscreen overlay-plane scanout)

Status: in progress on `feat/window-scanout-promotion`.

## Goal

Scan out the client buffers of eligible **regular (non-fullscreen) windows** directly on
KMS overlay planes, so the compositor does **zero GPU compositing** while a window's content
updates (video, scrolling, games). The primary plane renders only the *static frame*
(wallpaper + window shadows + chrome + non-promoted windows) and is **not re-rendered when a
promoted window's content changes** — only its plane flips.

This generalises the existing fullscreen direct-scanout path (`get_fullscreen_window`) to
floating/maximized windows.

## Mechanism

Per window (`WindowView`):
- `content_hidden: AtomicBool` + `set_content_hidden(bool)` → hides the `content_layer` in the
  lay-rs scene while **keeping `shadow_layer` visible**. The scene therefore draws the shadow
  (and rounded-corner cutout) but not the surface buffer.

Per frame (`udev/render.rs`):
1. Compute the promoted set (see *Selection*). `set_scanout_windows()` diffs vs last frame and
   toggles `content_hidden` (idempotent).
2. Render the scene element as usual (now with holes where promoted content was).
3. Append each promoted window's surface tree **above** the scene element with
   `Kind::ScanoutCandidate` (the default `Kind::Unspecified` is rejected for overlay planes).
   Position is driven from the **lay-rs layer bounds** (animated visible position), minus the
   CSD `geometry().loc` inset, falling back to the smithay `Space` location before first layout.
4. Render with `FrameFlags |= ALLOW_OVERLAY_PLANE_SCANOUT` (plus existing primary+cursor).
   Smithay's `DrmCompositor` promotes what it can fit; anything it can't **GPU-composites above
   the scene** (correct, because we only promote windows with nothing above them — see below).

Damage / no-shadow-rerender (works with current lay-rs `update() -> bool`, no lay-rs change):
- `update_window_view` (`workspaces/mod.rs`) is where a window's buffer enters lay-rs (surface
  tree appended under `content_layer`). **While a window is promoted we skip that import
  entirely** — so a buffer commit pushes *no* lay-rs transaction, `scene_element.update()`
  returns `false`, and the primary swapchain is not redrawn. The buffer is instead rendered by
  smithay as a `Kind::ScanoutCandidate` element → its own plane flips.
  (This is why we don't need the reference branch's `nodes_repainted` change: lay-rs `update()`
  returns true on *any* transaction, even for a hidden layer, so hiding alone isn't enough —
  not feeding the buffer in at all is.)
- **Shadow invalidated only on size change** (explicit requirement): `update_window_view`
  updates the shadow `View` model's `w/h` only when they actually change; position is applied as
  a layer transform (cached image reused, no re-raster). Activation/focus opacity is applied via
  layer opacity, not a content re-raster, so focus changes don't rebuild the blurred shadow.
  Net: the primary is rebuilt only on window **resize** (and genuine chrome/other-window
  changes), never on move or focus or content update of a promoted window.

## Selection (overlap rule)

Windows ordered **top-to-bottom** (front first). Maintain `covered` = union of rects of all
windows seen so far (above the current one).

```
covered = ∅
for W in windows_top_to_bottom:
    if eligible(W) and (W.rect ∩ covered == ∅):
        promote(W)          # nothing above overlaps it
    covered = covered ∪ W.rect   # always, promoted or not
```

Result: the **topmost** eligible window is always promoted; a **lower** window is promoted only
if it is disjoint from *every* window above it. Promoted windows are therefore **mutually
disjoint and unoccluded** ⇒ each rides an independent plane:
- no overlapping planes ⇒ no dependence on hardware zpos/blend (`overlay-scanout-hardware.md`),
- nothing is drawn above a promoted window's rect ⇒ the GPU-composite fallback (when no plane is
  free) is also z-correct,
- no lower window can peek through a higher window's rounded corner, because they don't overlap.

## Eligibility / the "stable" gate

Scanout is **globally disabled** (no candidates this frame) when any of these hold — these are
the *stable check*:

| Condition | Why | Symbol |
|---|---|---|
| Fullscreen mode active/animating | dedicated fullscreen path owns it | `workspace.get_fullscreen_mode()/_animating()` |
| Workspace swipe gesture active | geometry in motion | `swipe_gesture.is_active()` (call-site) |
| Workspace selection mode (expose / show-all) | whole layout transforms | `get_show_all()` / `is_expose_transitioning()` |
| Window selection mode (expose picker) | windows transform | `window_selector` active (`!root.hidden()`) |
| App switcher visible | overlay UI above windows | `app_switcher.alive()` |
| OSD visible | overlay UI above windows | `osd.is_visible()` |
| Context menu open | overlay UI above windows | `context_menu.is_active()` |
| A layer-shell **Overlay** surface is visible | draws above everything | layer map query (call-site) |
| **Dock menu open** | dock context menu composites above windows | `dock.context_menu.read().is_some()` |
| **Dock hovered / magnifying** | dock is animating magnification, composites above windows | `dock.magnification_position` ≠ rest (hover) |
| **Dock layout animation running** | dock geometry in motion | `dock.last_layout_animation` live |
| **Screenshot pending** (screencopy) | capture reads the primary buffer; planes would be missing | `!pending_screencopy_frames.is_empty()` (call-site) |
| **Recording active** (screenshare) | same — capture must see a fully-composited frame | `!screenshare_sessions.is_empty()` (call-site) |

Capture and overlay/swipe gates live at the **call-site** (`Otto<UdevData>`), because
`pending_screencopy_frames`, `screenshare_sessions`, the layer map and the swipe gesture are
owned by `Otto`, not `Workspaces`. Workspace-mode gates live inside `get_scanout_candidates`.

**Per-window** rejection (skip just that window, scanout still applies to the rest):
- minimised / minimizing animation / unmapped,
- active per-window effect: opacity < 1, scale/genie transform in progress,
- has subsurfaces (v1: skip — a subsurface above the main buffer can't share the single plane),
- overlaps a visible chrome reserved area (dock / topbar / layer-shell Top) — would be drawn
  under chrome but the plane sits above the primary.

## Demotion robustness ("suddenly render the window normally")

Because the buffer isn't fed into lay-rs while promoted, demotion must refresh lay-rs:

- **Same-frame, plane assignment fails:** automatically safe. Each promoted window is appended as
  a `Kind::ScanoutCandidate` smithay element built fresh from its surface tree every frame; if
  `DrmCompositor` can't fit it on a plane it GPU-composites *that* element (current buffer) above
  the scene. `should_draw` must therefore be true whenever a promoted window has new surface
  damage (not just when the lay-rs scene is dirty), so `render_frame` is always called to either
  flip the plane (≈0 GPU) or composite.
- **Cross-frame demotion** (gate trips: switcher/expose/recording/overlay, or the window starts
  animating / gains a subsurface / overlaps something): `set_scanout_windows` diffs the set; for
  each *departing* window the render call-site calls `update_window_view(window)` that same frame
  to re-import the current buffer into `content_layer` **before** `set_content_hidden(false)`
  unhides it — so the first composited frame is current, never stale.

This is why `update_window_view` must be the demotion refresh point and why the forced re-import
lives at the `Otto<UdevData>` call-site (it owns `update_window_view`), not inside `Workspaces`.

## Edge cases

- **Rounded corners**: the overlay carries the client's square buffer. If window *content* is
  compositor-rounded, promoted windows show square corners. Decision: accept square corners
  while promoted (effects-off-when-promoted, like KWin/Mutter), restore on demotion. *(Verify
  whether Otto actually rounds content vs only shadow before finalising.)*
- **Translucent windows**: providing the element + letting smithay decide is safe — it blends or
  composites based on opaque regions; `content_hidden` only prevents double-draw.
- **Popups**: a popup on a promoted window must also be a scanout candidate at higher zpos, else
  smithay demotes the whole stack to the primary composite. Tracked via `scanout_popups` +
  `set_scanout_popups` / `PopupOverlayView::set_popup_content_hidden`.
- **Capture mid-playback**: screenshot/recording forces a full composite frame (gate above);
  there is a one-frame mode transition (`was_direct_scanout` reset) which is already handled.
- **Mode transition**: entering/leaving any scanout mode resets buffers (`mode_changed`).

## Phase 2: layer-shell surface scanout

wlr-layer-shell surfaces (bars, docks, panels, notifications) are *easier* candidates than
windows: edge-anchored ⇒ mutually disjoint, undecorated ⇒ no shadow/rounded-corner problem,
rectangular + usually opaque. They fold into the **same** top-to-bottom overlap selection — just
add them to the candidate list at their z-layer (Overlay/Top above windows, Bottom/Background
below).

This also *relaxes* the window rule: a window currently can't be promoted when a Top/Overlay
bar overlaps it (the bar is in the composited primary, above the plane). If the bar is *also*
promoted to a higher-zpos plane, the window under it can be promoted too — they compose as
independent planes.

Caveat — **plane scarcity**: Gen12 exposes only a handful of overlay planes. A static bar that
repaints rarely gains ~nothing from a plane (the primary wasn't being redrawn for it). The
payoff scales with update frequency, so the assignment must prioritise frequently-updating
surfaces (video/game window) and let the rest stay composited. Gather all candidates
(layer Overlay/Top + windows) z-ordered, apply the disjoint rule, assign planes up to the
hardware limit by priority; smithay's `DrmCompositor` TEST_ONLY does the final fit.

## Known artifacts to fix (observed on first hardware run)

Maximized `vo=gpu` video showed "texture not fully mapped / black areas / tearing". Hypotheses,
to verify in order:

1. **Plane src/dst geometry off** (primary suspect — "not fully mapped"). The scanout element's
   `physical_loc` = layer bounds − `geometry().loc` inset, scaled. If the inset or fractional
   scale is wrong, the plane samples outside the buffer → black strips. Also check a maximized
   buffer smaller than the output (black margins). Verify against the smithay `Space` location
   fallback and the actual committed buffer size.
2. **Buffer sync / premature release** ("tearing"). Because `content_layer` is hidden and the
   lay-rs import skipped, the normal buffer-hold accounting may release the dmabuf while it's
   still on the plane → mpv recycles it → tearing/garbage. Ensure the scanned-out buffer is held
   until off-plane, and that the plane commit honors the buffer's acquire/implicit fence
   (relates to `renderer_sync` / explicit-sync).

## Dock gates (to apply post-fork)

- **Per-window**: reject promotion when the window rect overlaps the dock (`dock.cached_dock_bounds`,
  converted to the window's logical space) while the dock is visible (`!dock.is_hidden()`).
- **Global**: disable scanout when the dock has a context menu open
  (`dock.context_menu.read().is_some()`), is hovered/magnifying (`magnification_position` ≠ rest),
  or has a live layout animation (`last_layout_animation`).

## Files

- `src/workspaces/window_view/view.rs` — `content_hidden` + `set_content_hidden`.
- `src/workspaces/mod.rs` — `scanout_windows`/`scanout_popups` sets, `get_scanout_candidates`
  (gate + overlap selection), `set_scanout_windows`/`set_scanout_popups`.
- `src/render_elements/scene_element.rs` — `update()` → `nodes_repainted > 0`.
- `src/udev/render.rs` — build elements, `ALLOW_OVERLAY_PLANE_SCANOUT`, call-site gates.
- `src/workspaces/popup_overlay.rs` — `set_popup_content_hidden`.
