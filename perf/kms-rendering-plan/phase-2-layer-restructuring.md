# Phase 2 — Layer restructuring

**Status**: ✅ done  
**Effort**: 1–2 days  
**Depends on**: nothing (pure scene graph refactor)  
**Blocks**: phases 3–5

## Goal

Reorganise the lay-rs scene graph so each plane group has its own container
`NodeRef`. `render_node_tree` renders each plane independently.

## What was built

### `OutputWorkspaces` additions (`src/workspaces/mod.rs`)

```rust
pub background_plane: Layer,   // child of workspaces_layer → scrolls free
pub windows_plane: Layer,      // child of workspaces_layer → scrolls free
pub overlay_plane: Layer,      // child of output_layer, sized phys_w×phys_h
pub dock_plane: Option<Layer>, // Some(dock.wrap_layer) primary, None secondary
```

`background_plane` and `windows_plane` are children of `workspaces_layer`, so
they scroll in sync with workspace transitions automatically.

`overlay_plane` is sized explicitly to output dimensions and updated on resize.
It groups: `layer_shell_top`, `workspace_selector`, `app_switcher`,
`layer_shell_overlay`, `overlay_layer` (DnD+OSD), `popup_overlay`.

`expose_layer` was already a container; it stays as-is and is used directly.

### `Workspace` changes (`src/workspaces/workspace.rs`)

- `workspace_background: Layer` — renamed from `plane_background`, public field.
- `windows_layer` — existing field, now also public for plane attachment.
- Neither is appended to the scene tree in `new()`; the caller (`Workspaces`)
  attaches them to `background_plane` / `windows_plane` containers.
- `update_layout()` positions both at the same x-offset as `workspace_layer`,
  keeping them in sync during workspace scroll.

### Z-order in `output_layer` (bottom → top)

```
workspaces_layer (contains background_plane + windows_plane as children)
expose_layer
dock.wrap_layer
overlay_plane
```
