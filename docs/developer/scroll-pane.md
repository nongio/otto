# Scroll panes

How an otto-kit application scrolls long content (a file listing, a grid of
icons, a settings pane, a palette's results, a stack of columns) at the
display's rate without repainting its window. It covers the rules that keep a
scroll cheap and the otto-kit components that implement them, so no application
has to work them out again.

> Status: the rules below are proven (otto-files' column view scrolls at 120 Hz
> on them, frosted), and the components in *The kit* are in otto-kit, used by
> otto-files, otto-settings, the launcher and the emoji picker. *Plan* says
> what is left.

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

1. **Scroll by moving, not painting.** Content lives in a *band*: a subsurface
   taller (or wider) than the viewport, cropped to it with `wp_viewport`. A
   step of the scroll is a new viewport source rect on the band: no paint, no
   buffer, no upload. The client paints a new band only when the scroll nears
   its edge or the content changes. It works the same on any compositor with
   `wp_viewporter`; nothing in it needs Otto's surface style.
2. **One step per presented frame.** A band move asks for a frame callback on
   the band and the next step waits for it. Stepping whenever the loop wakes
   produced ~360 commits a second of sub-pixel moves nobody saw.
3. **The window does not commit for a scroll.** Pointer and wheel input only
   wake the loop; the pane moves itself. A single `request_frame()` from an
   input handler turns every scroll step back into a whole-window repaint.
4. **Nothing on screen may change continuously by reallocating.** The
   scrollbar thumb squashes during a bounce; it is stretched through its
   viewport destination, and painted again only when the stretch would show.
5. **Paint only what moves; let the window keep what does not.** The band is
   transparent: rows only. The ground under it, an active tint, an empty or
   loading message stay in the window, which does not repaint while scrolling.
6. **Say what changed, and what is opaque.** When the window does paint, it
   reports damage (`Window::request_frame_damaged`), not the whole buffer. It
   declares its opaque area (`Window::set_opaque_region`), so a frosted window's
   blur is not drawn under content that covers it. That draw was ~1–2 ms of
   GPU per frame on its own.
7. **Describe only what is on screen.** Accessibility for a scrolling list is
   bounded by the band, like painting.
8. **Measure on the wire.** Every A/B here was checked against the client's
   `WAYLAND_DEBUG` (`attach`, `damage_buffer`) and Otto's per-plane damage
   before its numbers were believed. Twice an A/B measured nothing because the
   change never reached the wire.

The compositor half of these is in Otto and lay-rs and needs nothing from
applications: a subsurface that only moved is repositioned without repainting
its layer; a window update does not re-attach (and repaint) the root surface;
a kept blur is replayed rather than redone, and not under the opaque region.

## Clipping is the client's

No compositor clips a subsurface to its parent, and Otto's
`set_clip_children` exists only on Otto, so the kit clips every surface it
places itself (`components/scroll/crop.rs`). Each surface is cropped to the
part of it its pane shows: a viewport source rect over its buffer, a
destination the size of that part, and a `wl_subsurface` position where that
part sits. A surface the pane does not reach at all is unmapped, since a
viewport cannot be empty, and painted again when it comes back.

- Positions and destinations are whole points; the fraction a scroll stands
  at goes into the source rect, snapped to the output's pixel grid.
- A crop is sent before the paint whose buffer it fits: a source rect
  committed over a buffer it does not lie inside is a protocol error.
- A position is the parent's state, so a move commits the parent. It is
  sent before the surface's own commit too: Smithay compositors, Otto among
  them, apply it with that commit rather than the parent's, so a position
  sent after the paint that maps a surface waits for its next change.
- Hiding unmaps. Opacity 0 leaves a surface on screen wherever the style
  protocol is missing.
- A pane nested in a container is not clipped by it: the host hands each
  nested pane what the container shows (`ScrollPane::visible()` of the
  container, or `ScrollSurfaces::shown_content()`) through `set_clip_rect`,
  and a `Fill` or `PlacedSurface` riding in the container gets the same rect
  through `set_window`.

The surface style is left for what a crop cannot do: rounded corners,
shadows, frost, output placement and compositor-run fades.

## The kit

An application implements one trait and owns one value per scrolling thing;
what sits beside a pane is a `Fill` or a `PlacedSurface`.

### `ScrollContent`: what the application provides

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
}
```

Accessibility stays with the host, bounded by `ScrollPane::visible()`.

### `ScrollPane`: one scrolling viewport

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
pane.reveal(lo, hi);                // keyboard cursor into view

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

Dropping a pane destroys its surfaces. A band move waits for its frame to be
presented, but no longer than `FRAME_ANSWER_TIMEOUT` (500 ms, the rate Otto
keeps for windows out of sight): a surface the compositor never shows cannot
hold a pane still for good.

A host that owns its scroll state somewhere a surface cannot live (behind a
lock, in headless tests) drives `ScrollSurfaces::sync_state` with that state
and its speed instead, and does `ScrollPane`'s bookkeeping itself: invalidate
when what the band shows changes, count `waiting()` as the scroll dealt with.
otto-files' columns and palette list work this way.

### `Fill`: a flat rect that moves with a pane

One pixel of colour, stretched by its viewport and rounded by the compositor
(where the surface style is missing, a rounded fill paints its shape at full
size instead): what the
highlight is made of, and what a host uses for anything flat that has to ride
in a pane rather than be painted into one: the divider down a column's edge.
Recolouring, moving or resizing it is a request, never a paint.

```rust
let mut divider = Fill::new(stack.band_surface())?;
divider.set_style(theme.fill_tertiary, 0.0);
divider.set_rect(Rect::from_xywh(column.right - 0.5, 0.0, 1.0, column.height()));
divider.set_window(Some(shown));    // what of the stack is on screen
```

### `PlacedSurface`: a painted panel beside a pane

A child surface the client places and sizes itself, painted only when what it
shows changes: a status line over an empty column, a preview card, a palette
dragged clear of its window. It keeps two rules so its host does not have
to: a move commits the parent, since a subsurface's position is the
parent's state; and a hidden surface is unmapped, off the screen and out of
the pointer's way, and painted again when it comes back. Until a resize is
painted, the old buffer shows at its own size, never stretched.

```rust
let mut card = PlacedSurface::new(parent, rect)?;
card.set_rect(rect);                             // a resize waits for the paint
card.set_window(Some(shown));                    // cropped to what shows
match card.paint(key, |canvas| draw(canvas)) {   // in the surface's own points
    Paint::Unchanged | Paint::Painted => {}
    Paint::Held => keep_the_loop_turning(),      // last frame not shown yet
}
card.set_takes_input(true);                      // for one outside its window
card.ask_output_frame();                         // where its display is,
card.output_frame();                             // a round trip later
```

### `ScrollGroup`: which pane a gesture belongs to

Wheel and touchpad gestures are routed to the pane under the pointer, with the
axis chosen by the first delta and locked until the gesture ends; wheel end
and discrete notches are handled once, here. A touchpad hold stops every pane.

```rust
group.axis(event, &mut [&mut column, &mut stack]); // innermost first
group.hold(&mut [&mut column, &mut stack]);
```

No application uses it yet: otto-files routes its own gestures, because its
scroll views are not panes (see above).

**Nesting.** A pane can be the parent of other panes: `ScrollPane::band_surface()`
is a valid parent. A horizontal pane whose band holds vertical panes is a
column stack: panning moves one band, and the columns ride inside it. Each
column crops itself to the container's `visible()` through `set_clip_rect`,
which is the one per-column request a pan costs.

### `RowLayout` and `GridLayout`: closed-form geometry

Fixed-pitch rows and uniform cells (with optional section headers), in content
coordinates only: `RowLayout::{rect, index_at, range, visible, length}` and
`GridLayout::{cell_rect, index_at, range, cells_in, headers, pinned_header,
length}`. The same layout answers the paint walk, the hit test, the accessible
bounds, `reveal` and which thumbnails to fetch, so what is drawn and what is
clickable cannot drift apart. otto-files' `RowStrip` is a thin wrapper over
`RowLayout` that folds in the scroll offset.

## What stays in the application

Row and cell appearance, selection runs, the cursor ring, thumbnails (as part
of `revision`), a rename field, the drop ring and the marquee, drawn in the
window over the pane using `content_to_parent` or into the band. Chrome that
does not move stays in the application too: dividers, tints, headers.

## Plan

1. **Axis-generic bands.** *Done.* `Band` and `ScrollSurfaces` on either axis,
   with `examples/scroll_nested_probe.rs` scrolling a vertical pane, a
   horizontal one, and vertical panes nested in a horizontal container.
2. **`ScrollPane`, `ScrollContent`, `ScrollGroup`, `RowLayout`, `GridLayout`**
   in otto-kit. *Done.* otto-settings is on `ScrollPane`: a transparent pane
   over the ground its window paints, input through the window.
3. **otto-files.** *Columns done.* The stack is a horizontal container clipped
   to the file area and each column a vertical pane placed once inside it;
   everything that moves with the stack is in the stack: the active tint (the
   clip's colour), the dividers (stretched pixels), status lines and the
   docked preview. So neither a column scroll nor a pan commits the window,
   and the pan bar is the container's own. otto-files drives `ScrollSurfaces`
   directly rather than `ScrollPane`: its scroll views live in the browser
   state, behind its lock and in its headless tests, where no surface can.
   List and grid stay painted into the window with partial damage and the
   opaque region (one large band costs more to composite than the window's
   damaged strip), and their geometry is `RowLayout` / `GridLayout`, mapped
   into the file area. The palette's rows are a pane inside its card, so the
   card repaints only for its field.
4. **Everyone else.** *Launcher and emoji done.* The launcher's rows are a
   `ScrollPane` inside its card, with the selection a pane highlight
   (`ScrollPane::set_highlight`) and wheel scrolling. otto-emoji's categories
   are vertical panes in a horizontal container driven by its own paging
   physics, with the highlight on the selected cell. Left: Peek's pan as a
   two-axis pane.
5. **Beside the panes.** *Done.* `PlacedSurface` carries otto-files' status
   lines, docked preview and its video, the palette card and Peek.

Exit, for each step that moves an application: zero window commits during a
fling, Otto passes in budget ≥ 80% at 120 Hz with frost on, client CPU under
5% of a core.

## Decisions

- **Input stays with the window.** Pane surfaces pass the pointer through; the
  host hit-tests in its window's coordinates and converts with
  `parent_to_content`. One coordinate space for every application.
- **A pane that only holds panes has no painted band.** Its band spans the
  whole content with a transparent 1×1 buffer stretched by its viewport
  destination; panning it moves one surface and the panes inside keep fixed
  positions. It is never cropped, since a crop would move its origin.
- **Clipping and placement never depend on the surface style.** The same
  viewport crop runs on Otto and on every other compositor, so the path other
  desktops take is the one used every day.

## Open questions

- **Band memory** on a very wide horizontal band at 2x: the overdraw floor may
  need to be axis-specific.
- **Pinned section headers.** `GridLayout::pinned_header` knows which header
  is pinned; drawing it as a surface of its own over the band, repainted only
  when the pinned section changes, is not built yet.
- **A pane over state it does not own.** otto-files keeps its scroll views in
  its browser state, so it cannot use `ScrollPane` and repeats its
  bookkeeping. A pane that borrows a `ScrollView` per update would let it.

## Measuring

`components/otto-kit/examples/virtual_fling.rs` plays touchpad scroll gestures
through `zwlr_virtual_pointer_v1`; `examples/damage_probe.rs` repaints a window
every frame with a chosen damage, to isolate what the compositor pays per
damaged megapixel. The takeover scripts that drive them against a traced Otto
on bare metal, and the analysis of the frame trace and per-plane damage, are
kept outside the tree.
