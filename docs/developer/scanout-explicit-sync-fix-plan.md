# Fixing direct-scanout tearing under explicit sync

Status: investigation + plan (no code changed yet)
Branch: `feat/window-scanout-promotion`
Author: research pass, 2026-06-21

## TL;DR root cause

Otto advertises `wp_linux_drm_syncobj` (explicit sync) but never holds the
surface commit until the client's *explicit acquire* fence signals. Otto's
pre-commit hook in `src/shell/mod.rs:108-136` only builds a blocker from the
*implicit* dmabuf fence (`dmabuf.generate_blocker(Interest::READ)`); it has no
branch for the explicit acquire point stored in smithay's
`DrmSyncobjCachedState`. Under explicit sync the client attaches **no** implicit
fence to the dmabuf, so that blocker is a no-op and the commit is applied
immediately. smithay's `DrmCompositor` then treats the buffer as ready: in
`ScanoutBuffer::acquire_point` (`compositor/mod.rs:226-238`) it sees
`buffer.acquire_point().is_some()` and returns `SyncPoint::signaled()` together
with `signaled_fence` — *assuming a `DrmSyncPointBlocker` already waited*
(documented contract in `drm_syncobj/mod.rs:6-7`). It never did. The plane is
flipped (`atomic.rs:1270-1280` sets `IN_FENCE_FD` to the already-signaled fence)
before the client's GPU render completes → scanout tearing. The compositing path
masks this because `blit_eglimage_to_2d_texture` in `src/skia_renderer.rs` does a
`glFinish`, but scanout bypasses GL entirely. Setting `OTTO_NO_EXPLICIT_SYNC=1`
(`src/udev/init.rs:451-459`) disables the syncobj global, the client falls back
to implicit dmabuf fences, the implicit blocker at `shell/mod.rs:122` *does* fire,
and tearing disappears — exactly the observed A/B result.

## Environment / versions

- smithay pinned rev: `f9d4d7ffd82d743fb89737b3ffaed57505b477d1` (v0.7.0),
  `Cargo.toml:66-68`, `Cargo.lock:4735-4737`.
- Local checkout used for this investigation:
  `~/.cargo/git/checkouts/smithay-312425d48e59d8c8/f9d4d7f`.
- This rev **has full explicit-sync scanout support** — no smithay fork or bump
  is required. The missing piece is entirely on the Otto side.

## How explicit sync is supposed to flow (this smithay rev)

1. Client binds `wp_linux_drm_syncobj`, calls `GetSurface`, then per frame
   `SetAcquirePoint` / `SetReleasePoint` + `attach` + `commit`.
2. smithay's `DrmSyncobjState` dispatch stores the points into the surface's
   `DrmSyncobjCachedState` pending state
   (`drm_syncobj/mod.rs:399-451`, fields at `:90-94`).
3. smithay's own `commit_hook` (`drm_syncobj/mod.rs:208-262`) only **validates**
   the points (NoBuffer / NoReleasePoint / ConflictingPoints / non-dmabuf). It
   does **not** add a blocker. The module doc is explicit:
   > "the implementation here assumes acquire fences are already signalled when
   > the surface transaction is ready. Use `DrmSyncPointBlocker`."
   (`drm_syncobj/mod.rs:6-7`)
4. **The compositor is responsible** for adding a `DrmSyncPointBlocker` in its
   own pre-commit hook so the transaction is delayed until the acquire fence
   signals. The blocker API is `DrmSyncPoint::generate_blocker()` returning
   `(DrmSyncPointBlocker, DrmSyncPointSource)`
   (`drm_syncobj/sync_point.rs:175`, `:257-261`). The source is an
   event-loop source; when it fires the compositor calls `blocker_cleared`,
   the transaction applies, and the buffer becomes current.
5. On apply, `RendererSurfaceState::update_buffer`
   (`renderer/utils/wayland.rs:143-178`) moves the cached acquire/release points
   onto the `Buffer` (`acquire_point: syncobj_state.acquire_point.take()`,
   `:173`). `Buffer::acquire_point()` (`wayland.rs:115-117`) now returns `Some`.
6. In `DrmCompositor::render_frame`, when an element's underlying storage is a
   wayland buffer it is wrapped as `ScanoutBuffer::Wayland`
   (`compositor/mod.rs:241-248`). For a candidate plane, the `PlaneConfig.sync`
   field is filled by `element_config.buffer.buffer.acquire_point(self.signaled_fence.as_ref())`
   (`compositor/mod.rs:3980-3989`). Because the blocker already waited,
   `acquire_point()` returns `Some((SyncPoint::signaled(), signaled_fence))`
   (`compositor/mod.rs:230-235`).
7. That `sync` becomes the plane `IN_FENCE_FD` on the atomic commit
   (`drm/surface/atomic.rs:1270-1280`, `:1519-1530`). With the signaled fence the
   flip is correct **only because step 4 already blocked on the real fence**.

The contract is: *Otto must block in step 4*. Otto skips step 4 for explicit
sync, so the "already signalled" assumption in step 6 is false and the flip races
the GPU.

## Where the fence is lost in Otto

- `src/shell/mod.rs:107-140` `new_surface` registers the only pre-commit hook.
  It reads `SurfaceAttributes.pending().buffer`, gets the dmabuf, and builds an
  **implicit** blocker via `dmabuf.generate_blocker(Interest::READ)`
  (`:122`). It never reads `DrmSyncobjCachedState` and never builds a
  `DrmSyncPointBlocker`. This is the single missing branch.
- `src/udev/render.rs` is *not* where the fence is lost. The scanout element
  builders (`render_surface`, fullscreen path at `:1108-1115`, overlay path at
  `render_elements_from_surface_tree(..., Kind::ScanoutCandidate)` at
  `:1158-1173`) produce `WaylandSurfaceRenderElement`s whose underlying storage
  is the wayland `Buffer`. The acquire point rides on that `Buffer` object
  automatically — *if* it was set in step 5. smithay extracts it itself in
  `render_frame`. So no change is needed in `render.rs` to *carry* the fence; the
  fence is dropped earlier because the buffer's `acquire_point` is whatever was
  taken at apply time, and the commit applied without ever waiting.
- `src/udev/mod.rs:87-92` correctly implements `DrmSyncobjHandler` and
  `delegate_drm_syncobj!`. `src/udev/init.rs:440-465` correctly creates
  `DrmSyncobjState`. Protocol plumbing is complete; only the blocker is missing.

## The fix

Mirror anvil's pre-commit hook (`anvil/src/shell/mod.rs:117-171` at this rev),
which handles **both** explicit and implicit fences. Add an explicit-acquire
branch to Otto's hook in `src/shell/mod.rs:107-140`:

```rust
fn new_surface(&mut self, surface: &WlSurface) {
    add_pre_commit_hook::<Self, _>(surface, move |state, _dh, surface| {
        // NEW: read the pending explicit-sync acquire point (udev backend only).
        let mut acquire_point = None; // Option<DrmSyncPoint>
        let maybe_dmabuf = with_states(surface, |surface_data| {
            // smithay::wayland::drm_syncobj::DrmSyncobjCachedState
            acquire_point.clone_from(
                &surface_data
                    .cached_state
                    .get::<DrmSyncobjCachedState>()
                    .pending()
                    .acquire_point,
            );
            surface_data
                .cached_state
                .get::<SurfaceAttributes>()
                .pending()
                .buffer
                .as_ref()
                .and_then(|assignment| match assignment {
                    BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                    _ => None,
                })
        });
        if let Some(dmabuf) = maybe_dmabuf {
            // NEW: prefer the explicit acquire fence when present.
            if let Some(acquire_point) = acquire_point {
                if let Ok((blocker, source)) = acquire_point.generate_blocker() {
                    if let Some(client) = surface.client() {
                        let res = state.handle.insert_source(source, move |_, _, data| {
                            let dh = data.display_handle.clone();
                            data.client_compositor_state(&client).blocker_cleared(data, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                            return; // do NOT also add the implicit blocker
                        }
                    }
                }
            }
            // existing implicit-fence path stays as the fallback
            if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                /* ... unchanged ... */
            }
        }
    });
}
```

Required new imports in `src/shell/mod.rs`:
`smithay::wayland::drm_syncobj::DrmSyncobjCachedState`. `add_blocker`,
`add_pre_commit_hook`, `with_states`, `get_dmabuf`, `BufferAssignment`,
`SurfaceAttributes` are already imported (`shell/mod.rs:23-24` and the dmabuf
import used at `:117`).

smithay API used: `DrmSyncPoint::generate_blocker()` →
`(DrmSyncPointBlocker, DrmSyncPointSource)` (`drm_syncobj/sync_point.rs:175`),
`compositor::add_blocker`, and the existing `blocker_cleared` flow. No new
smithay API, no fork, no version bump.

### Backend-genericity caveat

Otto's `new_surface` is implemented on `impl<BackendData: Backend>
CompositorHandler for Otto<BackendData>` (`src/shell/mod.rs:92`), i.e. shared by
udev / winit / x11. `DrmSyncobjCachedState` is gated behind smithay's
`backend_drm` feature, which is only enabled for the udev/x11 builds (see
`Cargo.toml` feature wiring around `:100-126`). Two acceptable approaches:

1. Read `DrmSyncobjCachedState` unconditionally — it is harmless on winit
   (always `None` because no syncobj global is created), provided `backend_drm`
   is compiled in for that build. Check the winit feature set first; if
   `backend_drm` is absent there the type won't resolve.
2. Guard the new branch with `#[cfg(feature = "udev")]` (or whatever feature
   enables the syncobj global). Anvil uses `#[cfg(feature = "udev")]` for exactly
   this reason (`anvil/src/shell/mod.rs:119,122,142`). This is the safer choice
   and matches the upstream pattern.

## Risks / edge cases

- **Per-commit fence lifetime.** The acquire point is taken out of the cached
  pending state at apply time (`wayland.rs:173` `.take()`), and the `DrmSyncPoint`
  is cloned into the blocker closure before that. `generate_blocker` dups the
  underlying syncobj handle, so the blocker owns its own reference — no
  use-after-free. The release point is auto-signaled when the last `Buffer` Arc
  drops (`drm_syncobj/mod.rs` destruction hook + module doc `:13-14`); the fix
  does not touch release handling.
- **`return` after explicit blocker.** Must `return` so we don't *also* add the
  implicit blocker on the same commit (anvil does this at `:153`). Adding both is
  redundant and could needlessly delay if the implicit fence is unsignaled.
- **Fallback to compositing.** When a candidate can't take a plane, smithay
  GPU-composites the same element. The composite path imports the buffer through
  the renderer which respects the (now correctly waited) buffer — and the
  pre-commit blocker already guaranteed the GPU finished, so the import is also
  race-free. The fix improves both paths.
- **Fullscreen vs overlay.** Both paths feed wayland surface buffers into
  `render_frame` and both rely on `ScanoutBuffer::Wayland::acquire_point`. One
  hook fixes both; no path-specific change needed.
- **Multi-plane.** Each promoted surface is a distinct wayland buffer with its
  own acquire point; the per-surface pre-commit hook covers every plane
  independently.
- **Multi-GPU.** Explicit-sync acquire on a non-primary render node: the blocker
  waits on the client's GPU regardless of which node scans out. The existing
  multi-gpu copy path is unaffected; if a surface is copied rather than scanned
  out, the implicit/explicit wait still completed before the copy.
- **`renderer_sync` feature interaction.** `pending_gpu_fence`
  (`src/udev/render.rs:362-368, 1253-1263`) only stores the EGL fence from
  `PrimaryPlaneElement::Swapchain` — i.e. the *compositor's own* GL render into
  the primary swapchain buffer. It has nothing to do with client scanout buffers
  and does not cover the explicit-sync case. The fix is orthogonal; no change to
  the `renderer_sync` path.
- **Clients that set an acquire point but the syncobj is already signaled.**
  `generate_blocker` + the event source fire immediately in that case — correct
  and cheap.

## Verification

1. **A/B regression (primary).** Build current tree, run a known-tearing
   explicit-sync client fullscreen and as a non-fullscreen scanout window
   (e.g. mpv `--vo=gpu-next`/`gpu` or a Vulkan game; both use
   `wp_linux_drm_syncobj`). Confirm tearing *with* the fix and *without*
   `OTTO_NO_EXPLICIT_SYNC`. Then confirm `OTTO_NO_EXPLICIT_SYNC=1` is no longer
   needed (both modes tear-free). The whole point is to make explicit sync match
   the implicit-fallback behavior.
2. **Confirm `IN_FENCE_FD` is meaningfully set.** Add a temporary `trace!` in the
   pre-commit hook logging when the explicit branch is taken and the blocker is
   added (`add_blocker` reached). Cross-check with smithay atomic logs: enable
   `RUST_LOG=smithay::backend::drm=trace` and look for the plane commit using the
   `sync` path (`atomic.rs:1270-1280`). Because the blocker waited, presentation
   should be glitch-free at the next vblank.
3. **No deadlock / stall.** Verify frame callbacks still flow (client keeps
   rendering) and that a slow client only delays its own surface, not the whole
   compositor — the blocker is per-surface via `add_blocker`.
4. **Implicit clients unaffected.** Run an implicit-fence client (e.g. a GL app
   without syncobj) and confirm the existing fallback path still runs (no
   explicit branch taken, no behavior change).

## Effort estimate

- Code: ~25-30 lines in `src/shell/mod.rs` (one new branch + one import),
  modeled directly on anvil at the pinned rev. Low risk.
- No smithay fork, no version bump — the pinned rev already ships
  `DrmSyncPoint::generate_blocker` and the scanout `acquire_point` plumbing.
- Decide the cfg-gating approach (genericity caveat above) before coding;
  recommend `#[cfg(feature = "udev")]` to match upstream.
- Estimated total: ~1-2 hours including the A/B verification.
