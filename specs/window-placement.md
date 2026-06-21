# Window Placement

**Status:** draft  
**Related specs:** window-scanout-promotion, window-tiling, workspaces-multi-output

## Summary

When a new toplevel window is created, Otto chooses its initial on-screen position automatically. It picks the spot — within the usable area of the output under the pointer — that overlaps existing windows the least, so new windows tend to land disjoint from what is already on screen. This replaces the previous fixed 40px cascade and is motivated by direct scanout: disjoint, unoccluded windows stay eligible for KMS-plane promotion.

## Goals

- Place a new window at the position that minimizes its overlap with existing windows on the current workspace, so new windows land disjoint when the geometry allows.
- Never place a new window over reserved layer-shell areas (exclusive zones for bars/panels) or over the dock when it is visible.
- Favour placements that keep windows disjoint and unoccluded, so more windows remain eligible for direct scanout.
- Place the new window on the output under the pointer.

## Non-Goals

- Per-window or user-facing configuration of placement strategy.
- Honoring a client-requested initial position.
- Multi-output placement heuristics beyond "the output under the pointer" (with a fallback to the first output).
- Re-placing existing windows or re-flowing the layout when a new window appears.
- Using the client's eventual real size for placement (the client's size is not yet known at creation time).

## Behavior

### Usable area

Placement happens within the **usable area** of one output, computed in logical pixels:

- Start from the chosen output's geometry.
- Subtract the layer-shell exclusive zones (reserved bands for bars, panels, and similar surfaces), leaving the non-exclusive region.
- When the dock is visible and would intrude into that region, reduce the usable height so the bottom edge stops above the top of the dock.

The result is a single rectangle that excludes all reserved chrome. New windows are always placed inside it.

### Output selection

- The window is placed on the output under the pointer.
- If no output is under the pointer, it falls back to the first available output.
- If no output geometry is available at all, placement falls back to a default rectangle.

### Assumed window size

The client's real size is unknown until it configures its surface, so placement assumes a default window of **800x600** logical pixels. The assumed size is clamped to the usable area (so on a small usable area the assumed window is shrunk to fit).

### Least-overlap selection

Placement evaluates a set of candidate top-left positions and picks the one whose assumed-size rectangle overlaps existing windows the least:

1. **Candidates**, generated in priority order:
   - The four corners of the usable area, clockwise from top-left: top-left, top-right, bottom-right, bottom-left.
   - For each existing window on the current workspace: a position snapped to that window's right edge (same top), and a position snapped to that window's bottom edge (same left).
2. **Clamping:** every candidate is clamped so the assumed window stays fully inside the usable area.
3. **Scoring:** for each candidate, the total overlap **area** of the assumed window against every existing window rectangle is summed.
4. **Selection:** the candidate with the smallest total overlap wins. Ties are broken by candidate order, so the clockwise corners take precedence and a fully disjoint top-left placement is preferred.

Existing windows considered are those on the current workspace, in logical-pixel coordinates.

### First window

With no existing windows, all candidates score zero overlap, so the first candidate — the usable-area top-left corner — wins.

## Constraints & Edge Cases

- **Logical pixels only:** the usable area, candidate positions, existing window rectangles, and the returned location are all in logical pixels.
- **Small usable area:** when the usable area is smaller than the assumed window, the assumed size is clamped down and candidates collapse toward the top-left corner; placement still returns a valid in-bounds position.
- **Many overlapping windows:** when every candidate overlaps something (a crowded workspace), the least-overlap rule still picks the position that minimizes covered area rather than cascading off-screen.
- **Assumed vs. real size:** because placement uses an assumed 800x600 size, a window that ultimately configures to a very different size may still end up partially overlapping; placement is a best-effort initial position, not a guarantee maintained after the client resizes.
- **Dock visibility:** the dock only constrains the usable area when it is currently visible; when hidden, the full non-exclusive region is available.

## Rationale

- **Least-overlap over a fixed cascade:** a count-based cascade steps every new window down-right by a fixed offset, which stacks windows on top of each other and pushes them off usable space. Least-overlap instead seeks empty space, keeping windows disjoint when room exists.
- **Disjoint placement aids scanout:** see [window-scanout-promotion](./window-scanout-promotion.md). A window that is disjoint from and unoccluded by others can be promoted to its own KMS overlay plane and scanned out without GPU compositing. Biasing initial placement toward disjoint positions keeps more windows promotable.
- **Corners first, then window edges:** corners are tried first because they are the most likely to be empty on a sparse workspace; edge-snapped candidates pack new windows neatly beside existing ones when corners are taken.
- **Tie-break by candidate order:** keeping the first candidate on ties makes placement deterministic and biases toward the clockwise corners and a clean top-left start.
- **Assumed default size:** the client has not committed a buffer at placement time, so a typical default size is the only basis available; clamping keeps the assumption from producing out-of-bounds positions.

## Open Questions

- Should placement re-run or adjust once the client commits its real size, to correct overlaps caused by the assumed-size guess?
- Should the candidate set include positions snapped to the left/top edges of existing windows, not only right/bottom, for denser packing?
- Should placement consider windows on other workspaces or only the current one (currently only the current workspace)?
