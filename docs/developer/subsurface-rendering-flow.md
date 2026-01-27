# Subsurface Rendering Flow Analysis

## Current Flow

### 1. **Surface Tree Traversal** (`get_render_elements` in `state/mod.rs`)

```rust
with_surface_tree_downward(
    surface,
    initial_location,  // Start at (0.0, 0.0)
    |_, states, location| {
        // Accumulate offsets as we traverse down the tree
        location += view.offset.to_f64().to_physical(scale_factor);
        location -= surface_geometry.loc.to_f64().to_physical(scale_factor);
        TraversalAction::DoChildren(location)  // Pass accumulated location to children
    },
    |surface, states, location| {
        // Create WindowViewSurface with ABSOLUTE positions
        let wvs = WindowViewSurface {
            log_offset_x: location.x as f32,  // ABSOLUTE position in tree
            log_offset_y: location.y as f32,  // ABSOLUTE position in tree
            
            phy_dst_x: view.offset.x as f32 * scale as f32 - surface_geometry.loc.x as f32,
            phy_dst_y: view.offset.y as f32 * scale as f32 - surface_geometry.loc.y as f32,
            // ...
        };
    }
)
```

**Key Point**: `log_offset_x` and `log_offset_y` contain **accumulated absolute positions** from traversing down the surface tree.

### 2. **WindowViewSurface Structure**

```rust
pub struct WindowViewSurface {
    pub(crate) id: ObjectId,
    
    // Source region in texture
    pub(crate) phy_src_x: f32,
    pub(crate) phy_src_y: f32,
    pub(crate) phy_src_w: f32,
    pub(crate) phy_src_h: f32,
    
    // Destination size and position (relative to view offset)
    pub(crate) phy_dst_x: f32,
    pub(crate) phy_dst_y: f32,
    pub(crate) phy_dst_w: f32,
    pub(crate) phy_dst_h: f32,
    
    // ABSOLUTE position from tree traversal
    pub(crate) log_offset_x: f32,  // ← ACCUMULATED from parent chain
    pub(crate) log_offset_y: f32,  // ← ACCUMULATED from parent chain
    
    pub(crate) texture_id: Option<u32>,
    pub(crate) commit: CommitCounter,
    pub(crate) transform: Transform,
}
```

### 3. **Layer Tree Building** (`view_render_elements` in `workspaces/utils/mod.rs`)

```rust
LayerTreeBuilder::default()
    .key("window_content")
    .children(
        render_elements.iter().map(|wvs| {
            LayerTreeBuilder::default()
                .key(format!("surface_{:?}", wvs.id))
                .position((
                    Point {
                        x: wvs.phy_dst_x + wvs.log_offset_x,  // ← Uses ABSOLUTE position
                        y: wvs.phy_dst_y + wvs.log_offset_y,  // ← Uses ABSOLUTE position
                    },
                    None,
                ))
                .size(...)
                .content(...)
        })
    )
```

**Current Behavior**: All layers are created as **siblings** under "window_content" with **absolute positions**.

### 4. **Layer Creation** (`create_layer_for_surface` in `shell/xdg.rs`)

```rust
pub(crate) fn create_layer_for_surface(&mut self, surface: &WlSurface) {
    self.surface_layers.entry(surface.id().clone())
        .or_insert_with(|| {
            let layer = self.layers_engine.new_layer();
            layer.set_key(&key);
            layer  // NOT PARENTED - standalone layer
        });
}
```

**Current Behavior**: Creates layers but does **NOT** parent them according to Wayland subsurface hierarchy.

## The Problem

### Current State (FLAT HIERARCHY)
```
window_content
├─ surface_A (position: absolute 0, 0)
├─ surface_B (position: absolute 100, 50)  ← subsurface of A
└─ surface_C (position: absolute 120, 80)  ← subsurface of B
```

All surfaces use **absolute positions** because they're siblings.

### Desired State (HIERARCHICAL)
```
window_content
└─ surface_A (position: 0, 0)
   └─ surface_B (position: relative 100, 50)  ← child of A
      └─ surface_C (position: relative 20, 30)  ← child of B
```

Subsurfaces should be **children** with **relative positions**.


### Option B: Build hierarchical tree in `view_render_elements`

**Pros**:
- Has all position data
- Can compute relative positions

**Cons**:
- `WindowViewSurface` is flat (Vec)
- Would need to restructure into a tree
- More complex data flow

**Critical Fix Needed**:
- Convert absolute positions to relative when parenting layers
- Store parent relationship to compute relative offsets

## Next Steps

1. Add parent tracking to understand hierarchy
2. Modify `create_layer_for_surface` to parent layers
3. Adjust position calculations to be relative to parent
4. Test with layer debugger to verify hierarchy
