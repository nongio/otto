# Performance experiments

Each subdirectory is one measurement session: a snapshot of CPU + GPU under a specific compositor / workload / Otto build. Use these to track regressions, compare Otto against reference compositors, and validate optimizations.

## Methodology

For every experiment, capture three things over the same ~10s window:

1. **CPU profile** — `perf record -F 99 -g -p <pid> sleep 10` then `perf report --stdio --no-children --sort symbol | head -20` → save as `otto-cpu.txt` (or `kwin-cpu.txt`, `anvil-cpu.txt`).
2. **GPU usage** — `intel_gpu_top -s 1000 -o - | head -12` → save as `intel-gpu-top.txt`. Requires `CAP_PERFMON` on the binary (see `scripts/grant-gpu-perf.sh`).
3. **Process CPU** — `top -b -n 1 -p <pid>` → mention in README.

Workload should be reproducible: list apps, window count, idle vs interactive.

## CPU vs GPU work

The two profilers measure orthogonal things — read both per experiment.

### What `perf` sees (CPU)

Sampled stack traces of Otto's process, broken into four categories:

| Category | Examples | Notes |
|---|---|---|
| **Otto / lay-rs scene mgmt** | `subtree_has_visible_drawables`, `Layer::set_size`, `Arc::into_any_arc`, `RenderLayer::update_with_model_and_layout`, `taffy::round_layout_inner` | Pure Rust; ours to optimize |
| **Skia CPU work** | `SkSurface::recordingContext`, `SkRect::round`, `SkMatrix::setConcat`, `GrFragmentProcessor::*`, `GrShape::bounds`, `GrSkSLFP::~GrSkSLFP` | CPU side of building the GL command stream |
| **Mesa / Gallium driver** | unresolved hex addresses (e.g. `0x00000000007ae1b2`), `libgallium_dri.so` if symbolised | Driver work to translate GL into hardware commands. Big chunk (~17% in some traces). Not ours to optimize but the *frequency* and *size* of submissions is. |
| **Smithay / Wayland plumbing** | `AtomicDrmSurface::page_flip`, `wayland_backend::flush`, `MultiCache::get`, `UserDataMap::insert_if_missing`, `wl_resource_get_version` | Driver-style cost from protocol dispatching. Mostly fixed overhead. |

### What `intel_gpu_top` sees (GPU)

Engine-level utilisation of the actual GPU silicon — completely invisible to `perf`:

| Field | Meaning | Otto-relevant? |
|---|---|---|
| **RCS busy %** | Render command streamer busy time (rasterisation, shading, blending) | Direct cost of compositing |
| **VCS / VECS busy %** | Video decode / encode | Usually 0 unless playing a video |
| **BCS busy %** | Blitter | Used by some compositors for surface copies; Otto rarely |
| **Power (GPU vs Package)** | GPU-only power vs whole-SoC power | High GPU power = real GPU work; high package power without GPU power = CPU-side cost |
| **Frequency (actual MHz)** | GPU clock speed | The driver scales clock to load — high freq + low busy% = bursty work; low freq + high busy% = steady light work |
| **RC6 %** | Time GPU spent in deep sleep | Inverse of "is the GPU being kept alive" |
| **Per-client RCS %** | From `/proc/<pid>/fdinfo` | Splits the engine-busy time across clients holding DRM fds. Note: chromium's GPU process may not appear here (uses ANGLE or fdinfo not exposed) |

### Quick rules of thumb

- **Otto's CPU at < 10% but GPU RCS at > 50%** → bottleneck is GPU work driven by Otto (Skia paint + GL submission), not Otto Rust code.
- **Top perf symbol < 3%** + flat distribution → no single CPU hotspot; cost is in command volume, not single-function expense.
- **High package power without high GPU power** → CPU cost dominant; check Otto + Mesa share.
- **High GPU power** + low package-minus-GPU → pure GPU rendering cost; need to send fewer/cheaper commands.
- **GPU freq low + RCS busy moderate** → "well-pipelined": many small ops keep the engine busy at low clock. Healthy.
- **GPU freq pegged + RCS high** → driver scaled up to keep up with demand. The cost is real (chrome canvas case).

## Plan

See [PLAN.md](PLAN.md) for the prioritised list of follow-ups derived from the experiments in this folder.

## Index

| Date | Experiment | Compositor | Workload | One-liner |
|---|---|---|---|---|
| 2026-04-18 | [anvil-baseline](2026-04-18-anvil-baseline/) | Anvil (Smithay reference) | chrome | Bare-Smithay baseline; 7.5% CPU, 24% GPU |
| 2026-04-18 | [matrix-a](2026-04-18-matrix-a/) | Otto + damage-refactor (default) | idle / ghostty / chrome-animated / anvil-chrome | 3-way scenario set + anvil control. Otto 4.3× anvil's compositor share, 4.5×/frame |
| 2026-04-18 | [matrix-b](2026-04-18-matrix-b/) | Otto + `image_cached(true)` on workspace_layer | ghostty + chrome | Negative result: caching workspace_layer doesn't help (cache invalidates per-frame) |
| 2026-04-18 | [matrix-c](2026-04-18-matrix-c/) | Otto + always-on damage-region clipping | ghostty + chrome | **Win**: removes `subtree_has_visible_drawables` 6.8% from hot path; -3pts otto share, -200 MHz GPU freq |
| 2026-04-18 | [matrix-d-expose](2026-04-18-matrix-d-expose/) | Otto + image_cached toggle on workspace_layers during expose | expose mode | Negative: cache invalidates per-frame from chromium animation, GPU power +37%, pkg power +64%. Reverted. |
| 2026-04-18 | [matrix-e-fps](2026-04-18-matrix-e-fps/) | Otto + fps counter in frame_finish | chromium animated | Reveals Otto delivers only **~35fps** on 120Hz display → per-frame cost is ~7.4× anvil's, not 4.5× |
| 2026-04-18 | [matrix-f-dock-cache](2026-04-18-matrix-f-dock-cache/) | Otto + picture+image cache on dock icon layer | chromium animated | **Win**: -3.3pts RCS, -3.6pts Otto share, +3.8pts RC6 sleep. The dock icon's shadow blur was the cost being skipped. |
| 2026-04-18 | [matrix-g-dock-bg-cache](2026-04-18-matrix-g-dock-bg-cache/) | Otto + picture+image cache on dock background bar (shadow + BackgroundBlur) | chromium animated | Mixed/regression. BackgroundBlur cancels the cache. Reverted. Confirms pattern: caching fails when layer reads animated content behind it. |
| 2026-04-18 | [matrix-h-text-cache](2026-04-18-matrix-h-text-cache/) | Otto + picture+image cache on workspace_selector text label | chromium animated | **Surprising regression**: ~7pts RCS even though cached layer is hidden during the test. Suggests lay-rs has hidden-layer cache cost. Reverted. |
| 2026-04-18 | [matrix-i-no-blur](2026-04-18-matrix-i-no-blur/) | Otto with dock_background_bar BackgroundBlur disabled | chromium animated | **Counter-intuitive regression**: +2.7pts RCS. The blur was acting as opacity occlusion (chromium under dock didn't need to be drawn). Reverted. |
| 2026-04-18 | [matrix-j-shadow-cache](2026-04-18-matrix-j-shadow-cache/) | Otto: cache shadow_layer instead of outer window_layer | chromium animated | **BIG WIN**: -6 to -8 pts RCS, -100MHz GPU freq. Per-frame cost 1.73% → 1.51%. The shadow blur was being re-cached every chrome commit because the outer cache invalidated; caching just the shadow keeps it stable. |
| 2026-04-18 | [matrix-l-scanout-skip](2026-04-18-matrix-l-scanout-skip/) | Per-window `is_scanned_out` flag + commit-handler skip when scanned out | chromium fullscreen | **Flag kept, gate reverted**: when active, dropped Otto GPU share to ~0% (12.8% → 10.1% RCS, 0.64W → 0.35W). But scanout-exit transition broke window sizing (render elements stale). Needs proper transition refresh before re-enabling. |

## Conventions

- Directory name: `YYYY-MM-DD-<short-slug>`. Multiple sessions on the same day → add `-a`, `-b`, etc.
- Each experiment directory has a `README.md` with: hypothesis, Otto commit SHA, workload, summary table, conclusion.
- Raw `perf.data` files are gitignored — only the text reports are committed.
- Don't put CPU/GPU numbers in commit messages or PR bodies; reference the perf entry instead.
