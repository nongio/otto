# Scroll panes

How an otto-kit application scrolls long content — a file listing, a grid of
icons, a settings pane, a palette's results, a stack of columns — at the
display's rate without repainting its window. This page is what building that
for otto-files taught, and the components otto-kit should offer so no
application has to learn it again.

> Status: the rules below are proven (otto-files' column view scrolls at 120 Hz
> on them, frosted). The components in *The kit* are the design; *Plan* says
> what exists and what is next.

## What a scroll must not cost

Measured on a 2880×1920, 120 Hz panel, a column view scrolling a directory,
frost on. Otto's render pass has 8.3 ms.

| | Otto pass p50 | GPU wait p50 | passes in budget | client CPU |
|---|---|---|---|---|
| rows painted into the window, whole-window damage | 8.0 ms | 6.7 ms | 1% | 11% |
| rows painted into the window, file-area damage | 5.7 ms | 4.4 ms | 82% | 13% |
| one subsurface per column, repainted per step | 8.5 ms | 7.3 ms | 4% | 12% |
| **a band per column, moved by the compositor** | **3.3–4.0 ms** | **2.6–2.8 ms** | **82–87%** | **1–3%** |

Everything slower than the last row was one of the mistakes below.

## Rules

Each of these cost a measurable frame budget before it was found.

1. **Scroll by moving, not painting.** Content lives in a *band* — a subsurface
   taller (or wider) than the viewport, inside a *clip* subsurface that crops
   it. A step of the scroll is `otto_surface_style_v1.set_position` on the
   band: no paint, no buffer, no upload. The client paints a new band only
   when the scroll nears its edge or the content changes.
2. **One step per presented frame.** A band move asks for a frame callback on
   the band and the next step waits for it. Stepping whenever the loop wakes
   produced ~360 commits a second of sub-pixel moves nobody saw.
3. **The window does not commit for a scroll.** Pointer and wheel input only
   wake the loop; the pane moves itself. A single `request_frame()` from an
   input handler turns every scroll step back into a whole-window repaint.
4. **Nothing on screen may change continuously by reallocating.** The
   scrollbar thumb squashes during a bounce; it is stretched through its style
   size, and painted again only when the stretch would show.
5. **Paint only what moves; let the window keep what does not.** The band is
   transparent: rows only. The ground under it, an active tint, an empty or
   loading message stay in the window, which does not repaint while scrolling.
6. **Say what changed, and what is opaque.** When the window does paint, it
   reports damage (`Window::request_frame_damaged`), not the whole buffer. It
   declares its opaque area (`Window::set_opaque_region`), so a frosted window's
   blur is not drawn under content that covers it — that draw was ~1–2 ms of GPU
   per frame on its own.
7. **Describe only what is on screen.** Accessibility for a scrolling list is
   bounded by the band, like painting.
8. **Measure on the wire.** Every A/B here was checked against the client's
   `WAYLAND_DEBUG` (`attach`, `damage_buffer`) and Otto's per-plane damage
   before its numbers were believed. Twice an A/B measured nothing because the
   change never reached the wire.

The compositor half of these is in Otto and lay-rs and needs nothing from
applications: a subsurface that only moved is repositioned without repainting
its layer; a window update no longer re-attaches (and repaints) the root
surface; damage from a child moving inside a clip is cut to the clip; a kept
blur is replayed rather than redone, and not under the opaque region.

## The kit

Four pieces. An application implements one trait and owns one value per
scrolling thing.

### `ScrollContent` — what the application provides

```rust
pub trait ScrollContent {
    /// Extent along the scroll axis, in points, for this cross-axis extent.
    fn length(&self, cross: f32) -> f32;
    /// Changes whenever `paint` would draw something different: a selection,
    /// a rename, a thumbnail landing. The pane repaints its band when it moves.
    fn revision(&self) -> u64;
    /// Paint `band` — a rect in content coordinates — on a transparent canvas.
    /// Called when the band is refilled or the revision moves, never per step.
    fn paint(&self, canvas: &Canvas, band: Rect);
    /// Describe what intersects `visible`, in content coordinates.
    fn describe(&self, _visible: Rect, _tree: &mut A11yTree) {}
}
```

### `ScrollPane` — one scrolling viewport

Owns everything a scroll view is: the physics (`ScrollView`: wheel, finger,
momentum, rubber band, thumb), the surfaces (clip, band, thumb) and their band
policy, pacing (rule 2), and revision tracking. **Either axis**: the band, the
input region, the thumb strip and the refill policy are all axis-generic.

```rust
let mut pane = ScrollPane::new(parent_surface, viewport, Axis::Vertical)?;

// every pass of the update loop — returns whether the pane is still moving
pane.set_viewport(viewport);        // a move keeps the band; a resize refills
pane.update(&content, &theme);      // steps, refills on revision, moves

// geometry for the host, all in the parent's coordinates
pane.content_to_parent(point);      // a rename field, a drop ring
pane.parent_to_content(point);      // hit-testing a press
pane.visible();                     // content rect on screen: a11y, thumbnails
pane.reveal(span);                  // keyboard cursor into view

// the selection, under the content, sliding between items
pane.set_highlight(Some(rect), color, radius);

// the pointer, as the host's handler sees it
pane.pointer_motion(point);         // also the scrollbar's hover and drag
pane.wheel_at(point, delta, discrete, stop);
pane.pointer_leave();
pane.hovered();                     // content point under a still pointer, now

// a container moved by other physics — paging, say
stack.update_container_at(length, offset, &theme);
```

Input stays with the host's window handler (the pane's surfaces pass the
pointer through), so an application keeps hit-testing in one coordinate space
and converts with `parent_to_content`. A selection that follows the pointer
asks `hovered()` on every update: a fling keeps moving after the fingers lift,
and the item under a still pointer changes with no event to say so.

The highlight is a surface of its own between the pane's ground and its band:
moving the selection repaints nothing, and while the content scrolls the
highlight goes straight to its item rather than trailing behind it.

### `Fill` — a flat rect that moves with a pane

One pixel of colour, stretched and rounded by the compositor: what the
highlight is made of, and what a host uses for anything flat that has to ride
in a pane rather than be painted into one — the divider down a column's edge.
Recolouring, moving or resizing it is a request, never a paint.

### `ScrollGroup` — which pane a gesture belongs to

Wheel and touchpad gestures are routed to the pane under the pointer, with the
axis chosen by the first delta and locked until the gesture ends; wheel end
and discrete notches are handled once, here. A touchpad hold stops every pane.
Replaces the per-application `gesture_axis`, `pane_under` and wheel branching.

```rust
group.axis(&event, pointer, &mut [&mut pan, &mut columns[..]]);
```

**Nesting.** A pane can be the parent of other panes: `ScrollPane::band_surface()`
is a valid parent. A horizontal pane whose band holds vertical panes is a
column stack — panning moves one band, and the columns ride inside it with no
per-column work.

### `RowLayout` and `GridLayout` — closed-form geometry

Fixed-pitch rows and uniform cells (with optional section headers), in content
coordinates only: `rect(index)`, `index_at(point)`, `range(rect)`, `length()`.
The same layout answers the paint walk, the hit test, the accessible bounds,
`reveal` and which thumbnails to fetch, so what is drawn and what is clickable
cannot drift apart. Replaces otto-files' `RowStrip`, the `*_in(… scroll)` grid
helpers and every `scroll` parameter threaded through them.

## What stays in the application

Row and cell appearance, selection runs, the cursor ring, thumbnails (as part
of `revision`), a rename field, the drop ring and the marquee — drawn in the
window over the pane using `content_to_parent`, or into the band — and chrome
that does not move: dividers, tints, headers.

## Plan

1. **Axis-generic bands.** *Done.* `Band` and `ScrollSurfaces` on either axis,
   with `examples/scroll_nested_probe.rs` scrolling a vertical pane, a
   horizontal one, and vertical panes nested in a horizontal container.
2. **`ScrollPane`, `ScrollContent`, `ScrollGroup`, `RowLayout`, `GridLayout`**
   in otto-kit. *Done.* otto-settings is on `ScrollPane`: a transparent pane
   over the ground its window paints, input through the window.
3. **otto-files.** *Columns done.* The stack is a horizontal container clipped
   to the file area and each column a vertical pane placed once inside it;
   everything that moves with the stack — the active tint (the clip's colour),
   the dividers (stretched pixels), status lines and the docked preview — is in
   the stack, so neither a column scroll nor a pan commits the window, and the
   pan bar is the container's own. otto-files drives `ScrollSurfaces` directly
   rather than `ScrollPane`: its scroll views live in the browser state, behind
   its lock and in its headless tests, where no surface can.
   List and grid stay painted into the window with partial damage and the
   opaque region — one large band costs more to composite than the window's
   damaged strip — and their geometry is `RowLayout` / `GridLayout`, mapped
   into the file area. The palette's rows are a pane inside its card, so the
   card repaints only for its field.
4. **Everyone else.** *Launcher and emoji done.* The launcher's rows are a
   `ScrollPane` inside its card, with the selection a pane highlight
   (`ScrollPane::set_highlight`) and wheel scrolling it did not have. otto-emoji's
   categories are vertical panes in a horizontal container driven by its own
   paging physics, with the highlight on the selected cell. Left: Quick View's
   pan as a two-axis pane.

Exit, for each step that moves an application: zero window commits during a
fling, Otto passes in budget ≥ 80% at 120 Hz with frost on, client CPU under
5% of a core.

## Decisions

- **Input stays with the window.** Pane surfaces pass the pointer through; the
  host hit-tests in its window's coordinates and converts with
  `parent_to_content`. One coordinate space for every application.
- **Pinned section headers are a surface of their own**, stacked over the
  pane's band and moved by the compositor like it: repainted only when the
  pinned section changes, never per step.
- **A pane that only holds panes has no painted band.** Its band spans the
  whole content with a transparent 1×1 buffer stretched through its style
  size; panning it moves one surface and the panes inside keep fixed positions.

## Open questions

- **Nested bands** — answered: Otto composes a band's style position inside a
  container band that is itself moving, in the probe and in otto-files' stack.
- **Band memory** on a very wide horizontal band at 2x: the overdraw floor may
  need to be axis-specific.

## Measuring

`components/otto-kit/examples/virtual_fling.rs` plays touchpad scroll gestures
through `zwlr_virtual_pointer_v1`; `examples/damage_probe.rs` repaints a window
every frame with a chosen damage, to isolate what the compositor pays per
damaged megapixel. The takeover scripts that drive them against a traced Otto
on bare metal, and the analysis of the frame trace and per-plane damage, are
kept outside the tree.
