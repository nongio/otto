# Surface Output Placement

**Status:** draft
**Related specs:** [quickview.md](./quickview.md), [file-browser.md](./file-browser.md)

## Summary

Additions to `otto_surface_style_v1` (version 3) that let a surface be placed
and sized against the **output** rather than against its parent:
`request_output_frame` with its `output_frame` event, plus
`set_output_placement` and `set_output_relative_size`.

**`request_output_frame` is the one to reach for.** The compositor answers with
the output's rect *in the coordinates the client already sets positions in*, and
the client does the arithmetic — which means the client moves its own content
along with it. The other two have the compositor do the placing, and that only
moves what the compositor draws, so on a subsurface they split the card's frame
from its contents. See *How to centre a surface on the display* for the recipe
and *What not to do* for the symptoms of each wrong turn.

## Goals

- A surface can sit centred on the display it is shown on, whatever its parent
  is doing, without the client knowing where its own window is.
- A surface can take a share of the display — half its width, three quarters of
  its height — without the client tracking output modes.
- Both compose with everything `otto_surface_style_v1` already does: blur,
  shadow, corner radius, transactions, spring timing.
- A client that binds an older version of the protocol is unaffected.

## Non-Goals

- Telling the client where its window is. The compositor resolves the position;
  it does not export the coordinates. A client that wants to do its own
  arithmetic needs a different request (see *Open Questions*).
- Replacing `wlr-layer-shell`. A surface still needs a role, and layer shell is
  the role for an overlay that is independent of any window. This is placement
  for a surface that already has one.
- Anchoring to an edge or a corner. Only centring is defined; other placements
  can be added to the enum without breaking clients.

## Behavior

### Why this exists

Positions set through `set_position` are relative to the surface's parent. A
surface that has to sit in a fixed place on the display cannot express that: a
Wayland client is never told where its own window sits, so it cannot work out
the offset. The compositor knows both. These requests let the client ask.

### `request_output_frame` and the `output_frame` event

`request_output_frame` asks where the surface's output is. The compositor
replies with `output_frame(x, y, width, height)`, expressed relative to the
surface's parent, in the physical pixels this interface's `set_position` and
`set_size` take — not the surface-local coordinates `wl_subsurface.set_position`
takes. Centring is then `x + (width - surface_width) / 2`, computed by the
client.

The rect is the output's **usable** area: what is left after the dock and any
layer-shell exclusive zones have taken their share. Centring in it puts a panel
where the eye expects rather than where the arithmetic alone would.

The answer is a snapshot: a client that cares about the surface moving between
outputs, or a mode change, asks again.

This is the only mechanism that moves a *subsurface* correctly, because the
client keeps doing its own positioning — see the constraint below.

### How to centre a surface on the display

This is the recipe, and `components/otto-files/src/pane_surfaces.rs` is the
worked example — `resting_for` and `centered_resting`.

1. **Ask, once per placement.** Call `request_output_frame` when the surface is
   about to be placed — when a panel opens, and again when it starts closing.
   Not every frame: the answer is a snapshot, and re-deriving a resting place
   from an old snapshot is what walks a panel off the screen.
2. **Clear the previous answer before asking**, so a stale one cannot be
   mistaken for the reply you are waiting for.
3. **Read the reply from `output_frame`**: `x`, `y`, `width`, `height`, relative
   to the surface's parent, in physical pixels. It is the *usable* area — the
   dock and any exclusive zones are already taken out.
4. **Convert if the surface is a subsurface.** `otto_surface_style_v1` works in
   physical pixels; `wl_subsurface.set_position` works in surface-local
   coordinates. Divide by the surface's scale before handing a position to
   `wl_subsurface`.
5. **Do the arithmetic**: `x + (width - surface_width) / 2`, same for `y`.
6. **Hold the result** until the next open or close, and place both the buffer
   (`wl_subsurface.set_position`) and the layer (`set_position` on the style)
   from it.

The entrance animation is unaffected. Because the answer arrives in the parent's
coordinates, an anchor rect in window space and a resting rect on the display
are in the same space, so a card can still fly from a file's icon to the middle
of the screen with no translation anywhere.

### What not to do

Four wrong turns, each of which produces a specific and recognisable symptom.

- **Reaching for `set_output_placement` on a subsurface.** Symptom: the card's
  frame — its background, blur, shadow and rounded corners — is centred on the
  display while its contents stay over the window. The request moves the style
  layer, and a subsurface's pixels are not in that layer. This is the most
  likely wrong turn, because the request reads like the obvious one.
- **Recomputing the resting rect every frame.** Symptom: the panel is correct
  when it opens and then drifts off the display when the window is moved,
  tiled, or maximised. The answer was relative to the window, so re-centring
  against it after the window moves compounds the offset. Freeze it per opening
  and per closing instead; a subsurface travelling with its parent in between is
  correct.
- **Forgetting the pixels-to-points conversion.** Symptom: on a 2x display the
  panel sits at twice the offset it should, usually off the bottom right. The
  two protocols do not agree on units and nothing warns you.
- **Expecting the compositor to keep it centred.** Symptom: a panel that was
  centred stays where it was after a mode change or an output hotplug. Nothing
  re-resolves a client-computed position; ask again.

### `set_output_placement`

Takes an `output_placement` value:

- `parent` (0, the default) — the position is whatever `set_position` last set,
  relative to the parent.
- `output_centered` (1) — the compositor keeps the surface centred on the
  output it is shown on.

While a surface is `output_centered`:

- Its position must be the middle of that output: `(output.width - size.width)
  / 2`, `(output.height - size.height) / 2`, in the output's own coordinates.
- A `set_size` or `set_output_relative_size` must re-centre it, since centring
  is measured against the size. A surface that resized without re-centring
  would sit centred for its *old* size.
- `set_position` from the client must not move it. A client that wants the
  position back sets the placement to `parent`.

The parent relationship is untouched: it still governs stacking, clipping and
lifetime. Only the position is resolved elsewhere. A subsurface of a window may
therefore be centred on the display while remaining that window's child.

The output is the one containing the surface's centre, so a window dragged
between displays takes its centred children with it. A surface that is on no
output at all falls back to the first, because it still has to be somewhere.

### `set_output_relative_size`

Takes four fixed-point values: `width` and `height` as fractions of the
output's width and height, and `min_width` and `min_height` as a floor in
surface pixels applied after the fraction.

- Fractions are clamped to `0.0 - 1.0`.
- A fraction of `0.0` leaves that axis at whatever `set_size` last established,
  so one axis can be output relative while the other is fixed.
- The floor exists because a sensible share of a large display is useless on a
  small one.
- Like `set_size`, this marks the bounds as client owned.

### Transactions

Both requests participate in transactions like every other animatable
property. Setting `output_centered` inside a transaction animates the surface
to the centre with that transaction's duration and timing function, rather than
snapping.

## Constraints & Edge Cases

- **`set_output_placement` moves the layer, not a subsurface's buffer.** The
  style surface's layer carries what the compositor paints — background, blur,
  shadow, rounded corners. A subsurface's *content* is placed by
  `wl_subsurface.set_position`. Centring the layer therefore leaves a card's
  frame in the middle of the display and its contents back over the window.
  This is why `request_output_frame` exists, and why a subsurface should use
  it instead.
- **The buffer does not resize itself.** `set_output_relative_size` sets the
  *layer* bounds. The client's buffer is whatever the client painted, and if
  the two disagree the buffer is scaled to fit under the surface's
  `contents_gravity`. A client that wants crisp output-relative content has to
  know the resolved size, and nothing currently tells it — see *Open
  Questions*. Until then, `set_output_relative_size` is only appropriate for
  content that tolerates scaling.
- **Centring is applied when something asks for it**, not continuously: on the
  placement request itself, and on any size change. A surface that is centred
  and then left alone while its *output* changes mode is not re-centred until
  its next size change.
- Multi-output positions are computed from each output's geometry scaled by its
  own fractional scale. This is exact for a single output and correct for
  side-by-side outputs at a shared scale; mixed-scale layouts are untested.

## Rationale

**Why not a new overlay protocol?** An independent overlay needs a *role*, and
`zwlr_layer_shell_v1` already is one — output association, configure/ack,
keyboard interactivity, a `closed` event. Writing a new protocol would rebuild
that list. `otto_surface_style_v1` explicitly augments a surface of *any* role,
so placement composes with whichever role the surface has.

**Why centring rather than exporting the window's position?** Exporting
coordinates invites every client to do its own arithmetic against state that
changes underneath it — outputs come and go, windows move, scales differ. A
declarative placement is resolved by the party that knows the answer, which is
the same argument Core Animation makes with `CAConstraintLayoutManager`:
constraints are expressed against another layer, never against a screen.

**Why an enum rather than a boolean?** So edge and corner placements can be
added without a new request.

## Open Questions

- **Should `set_output_placement` survive at all?** `request_output_frame`
  covers the same need without the layer/buffer split, and does so for every
  surface role rather than just the ones whose layer carries their pixels.
- **Should placement re-resolve on output changes?** Re-centring on mode
  changes and output hotplug is clearly correct; it needs a hook in whatever
  the compositor already runs when outputs change.
- **Entrance animations are no longer a problem.** Because the client is told
  where the output is *in its own coordinates*, an anchor in window space and a
  resting place on the display are in the same space, and an entrance can run
  between them unchanged. This was the argument for a `present_from` request;
  it is no longer needed.
