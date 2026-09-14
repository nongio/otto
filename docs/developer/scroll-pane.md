# Scroll panes

How an otto-kit application scrolls a long piece of content — a file listing,
a settings pane, a palette's results — without repainting its window on every
frame of the scroll. There is one way to do it, `ScrollPane`; this page is its
design and the measurements it answers to.

> Status: design, being proven on otto-files' list view. Sections marked
> *planned* describe work not yet in the tree.

## What it has to fix

Measured on otto-files at `20c005b8`, scrolling a 5000-entry directory in a
nested Otto (60 Hz, scale 2, frosted window) with a scripted touchpad session —
two seconds of drag each way and eight flings. See *Measuring* below for the
harness.

| view | client CPU | presents | gap p50 / p90 | damage per present |
|---|---|---|---|---|
| list | 175% | 446 in 16 s | 24.6 / 33.7 ms | whole window |
| grid | 172% | — | — | whole window |
| columns | 79% | — | — | whole window |
| list, no AT-SPI | 33% | 834 in 16 s | 8.4 / 30.9 ms | whole window |

Four separate costs sit in those numbers:

1. **The accessible tree is rebuilt for every entry on every pass of the run
   loop.** An Otto session always runs AT-SPI, so every kit app is described.
   otto-files names, formats and diffs 5000 rows per pass, and a second thread
   serialises the resulting AT-SPI traffic. That is ~140% of a core, and it
   grows with the directory, not with what is on screen.
2. **Presents are not paced to the display.** No `wl_surface.frame` is ever
   requested, so the window both paints frames that are never shown (p50 8 ms
   against a 16.7 ms refresh) and stalls (p90 31 ms). The cause is in the
   kit: a surface with a registered frame callback is assumed to be running a
   frame loop, so `draw` stops asking for frames — and a callback registered
   without an initial request never runs, so no frame is ever asked for.
3. **Every frame of a scroll is the whole window.** One toplevel buffer, damaged
   from `(0, 0)` to `INT_MAX`, re-uploaded and recomposited — sidebar, header,
   path bar and the frosted backdrop behind all of them — to move the rows.
4. **A frame is built over every entry, not every visible row.** otto-files'
   frame snapshot expanded each column's selection into a mask over all of
   its entries — a path-to-string allocation per entry, per column, per frame.
   In columns view, where several directories are on screen, that was most of
   the client's CPU once accessibility was fixed.
5. **Painting is cheap.** The list's paint is ~2.5 ms. The win is in not doing
   it, and not describing it, rather than in doing it faster.

## The shape

```text
window (xdg_toplevel)          chrome: sidebar, header, path bar, footer.
│                              Painted when the chrome changes. Never by a scroll.
└── clip   (subsurface)        fixed at the viewport; clips its children (style)
    ├── band  (subsurface)     the content, taller than the viewport; moved by
    │                          otto_surface_style_v1 to scroll
    └── thumb (subsurface)     the scrollbar; moved and faded by the compositor
```

A frame of scrolling is a style `set_position` on the band and the thumb: no
paint, no buffer, no damage on the window. The client paints the band only
when the scroll nears its edge (`Band::refill`), when the content changes, or
when the viewport's width, scale or theme changes. This is `ScrollSurfaces`
today; `ScrollPane` keeps its surfaces and band policy and adds the parts every
consumer has had to write itself.

## `ScrollPane`

One value owns everything a scroll view is:

- **Physics** — the `ScrollView`: wheel, finger, momentum, rubber band.
- **Surfaces** — clip, band, thumb, and the band policy.
- **Pacing** — while the view is moving, the pane asks for a frame callback on
  its clip surface and advances the physics by the presented interval. At rest
  it asks for nothing.
- **Input** — pointer events on the clip or band arrive with the pane's surface
  ids; `ScrollPane::pointer` turns them into content coordinates, feeds axis
  events to the physics itself, and hands presses and motion back to the host
  already in content space.
- **Accessibility** — the pane describes only what the band covers, and only
  when the band or the content revision moves.

The host supplies the content:

```rust
pub trait ScrollContent {
    /// Total length along the scroll axis, in points, at this width.
    fn length(&self, width: f32) -> f32;

    /// Bumped whenever what `paint` would draw changes: a selection, a
    /// rename, a thumbnail landing. The pane repaints the band when it moves.
    fn revision(&self) -> u64;

    /// Paint `band` (content coordinates) — called on refill and revision
    /// change only, never per frame of a scroll.
    fn paint(&self, canvas: &Canvas, band: Rect);

    /// Describe the items intersecting `band` into the pane's group.
    fn describe(&self, band: Rect, tree: &mut A11yTree);
}
```

Fixed-pitch content — every list otto-kit apps show — does not implement the
geometry itself. `RowLayout` (pitch, count, insets) and `GridLayout` (cell size,
spacing, sections) are closed-form: `rect(index)`, `index_at(point)` and
`range(band)` in content coordinates. The same layout answers the paint walk,
the hit test, the accessible bounds and the keyboard's scroll-into-view, so
what is drawn and what is clickable cannot drift apart. They replace
otto-files' `RowStrip` and `grid_visible_range_in`,
and the per-view `*_rect` / `*_at` helpers built on them.

Usage, the whole of it:

```rust
let pane = ScrollPane::new(window.wl_surface(), viewport, Axis::Vertical)?;

// every pass of the run loop
pane.set_viewport(viewport);              // no-op unless it moved
pane.sync(&content, &theme);              // moves, refills, describes

// in the pointer handler
if let Some(event) = pane.pointer(&event) {
    // event.position is in content coordinates
}
```

## Rules the design keeps

- **The window never paints for a scroll.** Anything that moves with the
  content lives in the band; anything that stays still lives in the chrome.
  The scrollbar lives in the thumb.
- **One owner per property.** The pane owns the band's position; the host never
  moves content inside the band to scroll it.
- **Describe what is on screen.** Accessibility cost is bounded by the band, as
  paint cost is.
- **Paint on the frame callback, not the wakeup pipe.** A scroll advances once
  per presented frame, by the presented interval.

## Frosted windows

The frost is the toplevel's style, so the compositor blurs what is behind the
window and tints it; the pane's clip surface carries no ground of its own and
the band is drawn with alpha. Moving the band damages only the pane in the
compositor — the band is blended over the window at its new position — and
the chrome around it is neither repainted by the client nor recomposited.

The blurred backdrop itself is kept between frames (lay-rs replays it unless
something *beneath* the window changed), so a scroll never blurs again. What a
scroll did still pay for was replaying that kept blur under content that covers
it: a listing's paper is opaque. A window says where it is opaque with
`Window::set_opaque_region` (`wl_surface.set_opaque_region`); Otto carries the
region onto the surface's layer and lay-rs leaves it out of the backdrop. With
it, a frosted list scrolls at the cost of an unfrosted one.

## Measuring

`components/otto-kit/examples/virtual_fling.rs` plays touchpad scroll gestures
through `zwlr_virtual_pointer_v1`. Pointed at a nested Otto it scrolls whatever
is under its pointer there and nothing in the session it runs inside. The
measurement scripts that drive it (nested compositor, per-thread CPU, a
`WAYLAND_DEBUG` summary of commits, attaches, damage and present intervals, and
`perf` sampling) are kept outside the tree while the design is proven.

## Plan

1. **Kit fixes that stand alone.** *Done.* One outstanding frame request per
   surface, so a window with a frame callback is paced like any other; the
   accessible tree rebuilt only after a paint, and otto-files describing only
   the rows on screen; the frame snapshot looking selection up per drawn row.
   List view with AT-SPI active went from 175% to 40% of a core, grid from
   172% to ~50%, columns from 79% to 69%. What is left is building the frame
   snapshot and repainting the window at all — which is what the pane removes.
2. **`ScrollPane`, `ScrollContent`, `RowLayout`, `GridLayout`** in otto-kit,
   with `examples/scroll_pane.rs` as the reference for writing a scroll view.
3. **otto-files list view** on a pane. Exit: zero window commits during a
   fling, client CPU under 10% of a core, present gap p90 within one refresh.
4. **Grid** on a pane with `GridLayout`.
5. **Columns**: one pane per column inside the horizontal pan. *Done*, on
   `ScrollSurfaces` (clip, band, thumb), with the pointer passed through to the
   window and the rows on a transparent band over the column's ground. A vertical scroll moves the band and
   paints nothing; the window is not repainted and the compositor
   recomposites only the column. Getting there needed three compositor fixes
   — the root surface layer was re-appended (and so repainted, whole-window)
   on every window update; a move-only surface commit re-installed its draw
   content; and damage from a child moving inside a clip was not clipped —
   plus pacing band moves to presented frames and stretching the thumb
   instead of repainting it during an overscroll.
6. **Settings, palette, launcher** move from `ScrollSurfaces` / hand-rolled
   lists to `ScrollPane`; `ScrollSurfaces` and otto-files' `PaneSurfaces`, its
   environment flags and the per-view geometry helpers are deleted.

## Open questions

- **Selection colours.** A selected row changes text colour, so selecting
  repaints the band. At ~2× a viewport's paint that is a few milliseconds on a
  click — acceptable — but a hover highlight that repaints per motion is not;
  hover may need an overlay surface.
- **Overlays that follow the content** — the rename field, the drag marquee,
  a drop highlight — either paint into the band (and revise it) or sit in a
  small overlay subsurface positioned from content coordinates.
- **Nested panes.** Columns pan horizontally and scroll vertically. Whether the
  compositor composes a style position on a band inside a moving parent band
  correctly is to be proven before step 5.
