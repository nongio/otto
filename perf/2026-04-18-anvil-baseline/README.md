# Anvil baseline — chrome workload

**Date:** 2026-04-18
**Compositor:** Anvil (Smithay reference compositor)
**Workload:** chrome (multiple tabs/renderers)
**Hardware:** Intel TigerLake-LP GT2 Iris Xe Graphics

## Hypothesis

Anvil is the closest "bare Smithay" baseline — same Wayland/Mesa/EGL plumbing as Otto, no scene graph, no Skia compositing. Establishes the floor for any Smithay-based compositor running real chrome content.

## Results

| Metric | Value |
|---|---|
| Anvil CPU | ~7.5% |
| GPU Render (RCS) | 24% steady |
| GPU power | 0.7W |
| Package power | 11.5W |
| GPU frequency | 200 MHz (of ~1300 max) |
| RC6 (sleep) | 70% |

Top hotspots: `TextureSync::update_read` 4.9%, libgallium ~17% (unresolved), `GlesFrame::render_texture_from_to` 4.5%, `import_dmabuf` 2.7%, `EGLFence::create` 1.9%.

## Conclusion

Bare Smithay + GLES blit-per-surface keeps GPU at low clock (200 MHz) with 70% sleep time and < 1W GPU power. Any Otto overhead above this floor is scene-graph / Skia compositing.
