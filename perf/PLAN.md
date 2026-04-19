# Otto performance plan

Derived from the matrix in this folder (matrix-a/b/c + anvil-baseline + ydotool-driven exploration). Ordered by **expected ROI**, not by phase.

## TL;DR — known costs (chromium animated canvas, 1× display 2880×1920 @ 120Hz)

|  | Otto default | Otto matrix-c (clip-always) | Anvil reference |
|---|---|---|---|
| GPU RCS busy | 62.6% | 60.7% | 28.0% |
| GPU power | 5.91W | 5.53W | 2.10W |
| GPU freq | 972 MHz | 775 MHz | 717 MHz |
| Compositor's GPU share | 60.5% | 57.6% | 14.1% |
| Per-frame cost (vs anvil, fps-adjusted) | ~4.5× | ~4.0× | 1× |

Hot CPU work in Otto matrix-c (top symbols, all CPU):
- lay-rs scene/layout: ~6% (`update_with_model_and_layout` 2.2 + `taffy::round_layout_inner` 2.1 + `set_node_layout_size` 1.5)
- Skia CPU prep: ~3% (`KeyBuilder::addBits` 1.4 + …)
- Mesa/Gallium driver: ~17% (unresolved hex symbols)
- Smithay/Wayland plumbing: ~5% (`MultiCache::get`, `UserDataMap::insert_if_missing`, `wl_resource_get_version`)

The GPU share is the dominant real cost (Otto is GPU-bound, not CPU-bound).

---

## P0 — land the matrix-c win

**Action:** Remove the `if occluded_ref.is_some()` gate in `src/render_elements/scene_element.rs:357-365`; pass `damage_region.as_ref()` unconditionally.

- **Validation:** matrix-c. Compositor share 60.5 → 57.6%, GPU freq 972 → 775 MHz, GPU power -6.4%, lay-rs `subtree_has_visible_drawables` falls out of the hot-symbol top.
- **Risk:** Trivial. The original comment already noted "correctness is preserved — only extra tree traversal is incurred."
- **Effort:** PR with a 4-line change + reference perf/2026-04-18-matrix-c/.

---

## P1 — kill per-frame taffy recomputation

**Symptom:** With matrix-c the new top symbols are `taffy::round_layout_inner` 2.07%, `set_node_layout_size` 1.53%, `RenderLayer::update_with_model_and_layout` 2.21% — totalling ~6% CPU on a chromium animation that doesn't change layout.

**Hypothesis:** lay-rs is calling taffy every frame even when no layout-affecting property changed. Should short-circuit on `taffy_computed == false` from the new `UpdateStats`.

**Action plan:**
1. Add tracing in lay-rs: log when `taffy_computed` is true and what triggered it (per-frame for one second).
2. If it's true every frame for a static layout, find the spurious property write.
3. If it's a real layout dependency on a per-frame value (animation transform?), fix the dependency.

**Expected impact:** -6% Otto CPU, more importantly should drop GPU prep cost too (less GL state churn from re-laying-out the scene).

**Effort:** 1 day investigation in lay-rs + Otto, possibly a fix in lay-rs.

---

## P2 — image-cache workspace mirrors during expose

**Symptom (observed live):** Triggering `Page Up` (ExposeShowAll) immediately produces a flood of lay-rs warnings:
```
WARN as_content() mirroring a layer that is not image-cached — this re-traverses the full subtree every frame
layer_id=NodeRef(119) key=workspace_view_2
```
Continuous at ~60Hz while expose is open.

**Cause:** Expose mode mirrors each `workspace_view_*` layer as content for the expose grid. Each mirror traverses the full source workspace's subtree every frame because the source isn't cached.

**Action plan:**
1. Find where `as_content()` is called on workspace_view layers in expose code (`src/workspaces/window_selector.rs` and friends).
2. When entering expose, set `image_cached(true)` on each workspace_view layer being mirrored. Set back to `false` on exit.
3. Verify warnings disappear and capture new perf during expose.

**Expected impact:** Substantially cheaper expose mode. Expose is currently the heaviest scenario in normal use.

**Effort:** Half day. The warning explicitly points at the fix.

**Note:** Matrix B already showed that *unconditionally* image-caching workspace_layer hurts because the cache invalidates per-frame when chromium animates inside it. The fix here is **temporary** caching during expose only, when the workspace contents shouldn't be live-updating anyway (or the live-update should happen at a lower rate).

---

## P3 — instrument framerate to confirm the 60-vs-120 hypothesis

**Symptom (user-observed):** Anvil renders the same chromium animation visually ~2× faster than Otto. Suggests Otto delivers ~60fps while anvil delivers ~120fps on the same 120Hz display.

**Why it matters:** Per-frame numbers in this plan currently assume Otto = 60fps. If Otto is actually at, say, 90fps, the per-frame penalty is smaller than 4.5×. Need ground truth.

**Action plan:**
1. Hook into `wp-presentation-time` feedback or add a counter in the udev render loop that increments on every successful page flip.
2. Log fps once per second behind a `--features profiling` flag.
3. Re-run matrix-a chrome scenario, record fps + RCS together.

**Expected outcome:** Either confirms Otto-at-60 (validates per-frame analysis) or reveals a different story (e.g. Otto at 90, with damage-driven repaint skips). Either way a useful number.

**Effort:** Half day.

---

## P4 — Skia shader / program cache churn

**Symptom (matrix-a observation):** `GrSkSLFP::~GrSkSLFP()` 60% in idle chrome scenario (extreme outlier), `GrGLProgram::updateUniforms` 2.0% + `ProgramCache::findOrCreate` 1.5% in active scenario.

**Hypothesis:** Otto generates new Skia paints/shaders per frame instead of reusing them. Could be from:
- New `SkPaint` constructed per draw call (without caching)
- Color filters or image filters with non-cacheable params
- `SkImage` rebuilt per frame from same underlying texture

**Action plan:**
1. Capture a Skia `.skp` during chrome animation (Alt+K) — look for repeated paint definitions.
2. Use Skia debugger to see if paints could be reused.
3. Cross-check with `GrDirectContext::dumpJSON()` to see program cache stats.

**Expected impact:** Hard to estimate without data; potentially significant in chrome-animated scenario.

**Effort:** 1-2 days investigation, fix scope unknown.

---

## P5 — make repaint damage-proportional (architectural, deferred)

**Symptom:** Otto does full-scene Skia paint each frame. Anvil's GLES renderer does per-surface blits scoped to damage. This is the structural reason Otto's compositor share is 4.3× anvil's.

**Action plan (deferred — needs design):**
1. Per-frame, compute the smallest output rect that needs to be repainted given client damage + animation deltas.
2. Skia paint scoped to that rect (clip + reuse picture for unchanged regions).
3. Possibly: per-output picture caches that invalidate on damage region only.

**Expected impact:** Largest of any item here. Could close most of the anvil gap.

**Effort:** Significant. 1-2 weeks. Likely a new lay-rs feature.

---

## Cross-cutting infrastructure (in parallel with P1-P3)

These don't move metrics but make all subsequent work faster:

### A. `--features profiling` build flag
Wire one flag that enables:
- `tracing-tracy` subscriber (per-frame zones in Tracy timeline)
- `pprof-rs` with SIGUSR1 dump (CPU snapshot from a running Otto)
- lay-rs `UpdateStats` logging at 1 Hz
- Frame counter for fps logging (P3)

**Effort:** 1 day. Pays back on every subsequent investigation.

### B. Scenario script library
Each plan item should re-run the same controlled scenarios. Codify in `perf/scenarios/`:
- `idle.sh` — Otto + autostart only
- `ghostty.sh` — Otto + 1 ghostty
- `chrome-anim.sh` — Otto + ghostty + chromium with `/tmp/anim.html`
- `expose.sh` — chrome-anim + ydotool Page Up to trigger expose
- `appswitch.sh` — chrome-anim + ydotool Ctrl+Tab loop

Each script: `bash scenarios/<name>.sh perf/<dated-dir>/<scenario>` → reuses `measure.sh`.

**Effort:** 2 hours.

### C. Comparison plot script
Python over multiple JSONL outputs to produce:
- Bar chart of stage times across builds
- Per-client GPU breakdown across scenarios
- Damage area vs render time scatter (when P3 lands)

**Effort:** half day.

---

## Suggested order

1. **P0** (today) — land matrix-c, ship the validated win.
2. **A + B** (in parallel) — finalise infrastructure so P1-P5 work re-runs cleanly.
3. **P2** (this week) — expose mirror caching, biggest user-visible win, cheap.
4. **P1** (this week) — taffy short-circuit, addresses the new bottleneck after P0.
5. **P3** (next) — framerate instrumentation, settles the 60-vs-120 question.
6. **P4** then **P5** — investigation-heavy, schedule based on what P1-P3 reveal.

Each P-item should produce: a perf entry under `perf/2026-MM-DD-<slug>/` showing before/after, plus a PR referencing it.
