# Performance Analysis: otto-islands + compositor (2026-04-05)

## Setup
- Otto running in winit mode with music playing (Spotify via MPRIS)
- otto-islands: notification + music island active, EQ animating at 24fps
- Release build, no profiler/debugger features

## otto-islands: 0.9% CPU, 189MB RSS

### Rendering rates
- EQ subsurface: 24fps (42ms throttle) — separate child subsurface, tiny buffer
- Main pill (expanded): 1fps full redraw (progress bar + time)
- Main pill (compact/mini): 0fps (static, redraws only on track/mode change)
- PipeWire audio level: skips 5/6 buffers, computes only every ~60ms
- playerctl: single subprocess every 1.5s (was 6 processes, now 1)

### Profile (perf, task-clock, 30s sample)
Completely flat — no single hotspot above 2.5%:
- `clock_gettime` 2.4% — Instant::now() calls
- `pthread_mutex_lock` 2.4% — shared state locking
- PipeWire data-loop ~6% combined — PipeWire's own audioconvert/audiomixer (not our code)
- `libloading` 1.5% — EGL symbol lookup (Skia internal)
- Our PipeWire callback: <1%
- Skia rendering: ~3% scattered (SkPaint, CircleGeometry, AutoLayer)
- playerctl subprocess: eliminated from profile after single-call optimization

### Optimization history
1. Started at 4% CPU (full surface redraw at 30fps)
2. Throttled idle tick and PipeWire updates → still 4% (dirty loop)
3. Added `music_last_redraw` gate → 1% (stopped redundant redraws)
4. Split EQ into child subsurface → 0.9% (main surface stays static)
5. Single playerctl call → no measurable change (was ~0.1%)
6. PipeWire early skip → no measurable change (computation was cheap)

### What's left
Nothing actionable on the islands side. The 0.9% is:
- PipeWire's own audio pipeline thread (~0.4%)
- EQ Skia draw 24fps on tiny buffer (~0.3%)
- System overhead: timers, locks, wayland dispatch (~0.2%)

## Otto compositor: 9-10% CPU, 250MB RSS

### Profile (perf, task-clock, 15s sample, no dev features)
Very flat — no hotspot above 1.7%:
- `import_dmabuf` 1.7% — importing island subsurface buffers (24fps EQ commits)
- `malloc/free` 2.8% — general allocations
- `epoll::wait` 1.5% — event loop idle wait
- `lock_user_data` 1.4% — Smithay surface state access
- `layers::get_layer` 1.2% — scene graph layer lookups
- EGL/Mesa 1.1% — GL context management
- `update_layer_shell_surface` 0.9% — processing island surface commits
- `SkPerlinNoiseShader` 0.7% — background noise effect
- `layers::Attribute::value` 0.6% — animation interpolation
- Wayland dispatch ~1.5% — request parsing, flushing clients

### Islands' impact on compositor
~2.6% from island surface commits:
- `import_dmabuf` 1.7% — EQ subsurface buffer import at 24fps
- `update_layer_shell_surface` 0.9% — surface state updates

### Note on dev features
Previous profile with `--features dev` showed:
- `ryu::format32` 3.9% + `serde_json` 2.9% — scene debugger JSON serialization
- `lz4_flex::compress` 2.4% + `puffin::serialize` 1.0% — puffin profiler
- Total ~10% overhead from dev features alone

### Potential future optimizations
- Reduce EQ fps (currently 24 → could try 15fps)
- Scene graph: cache `get_layer` lookups
- Batch dmabuf imports when multiple surfaces commit in same frame
- Investigate Perlin noise shader — is it needed at idle?
