# KDE Plane Testing — Research Notes

Source: `/home/riccardo/dev/kwin/src/backends/drm/`

## How KWin tests plane configs

KWin builds a `DrmAtomicCommit` object (in `drm_commit.cpp`), populates it with
DRM object properties, then fires a single `DRM_IOCTL_MODE_ATOMIC` ioctl with the
`DRM_MODE_ATOMIC_TEST_ONLY | DRM_MODE_ATOMIC_NONBLOCK` flags — no pixels drawn, no
page flip, no display impact.

```cpp
// drm_commit.cpp
bool DrmAtomicCommit::test() {
    return doCommit(DRM_MODE_ATOMIC_TEST_ONLY | DRM_MODE_ATOMIC_NONBLOCK);
}

bool DrmAtomicCommit::doCommit(uint32_t flags) {
    // ... pack properties into drm_mode_atomic struct ...
    return drmIoctl(m_gpu->fd(), DRM_IOCTL_MODE_ATOMIC, &commitData) == 0;
}
```

A framebuffer handle is added per plane via `addBuffer`:

```cpp
void DrmAtomicCommit::addBuffer(DrmPlane *plane,
                                const std::shared_ptr<DrmFramebuffer> &buffer,
                                const std::shared_ptr<OutputFrame> &frame) {
    addProperty(plane->fbId, buffer ? buffer->framebufferId() : 0);
    ...
}
```

The `DrmFramebuffer` is created by importing a GBM BO or dmabuf:
`gbm_bo_import(GBM_BO_IMPORT_FD_MODIFIER, ...)` → `drmModeAddFB2WithModifiers`.

## Integration point — DrmPipeline::testScanout

```cpp
bool DrmPipeline::testScanout(const std::shared_ptr<OutputFrame> &frame) {
    if (gpu()->atomicModeSetting()) {
        return DrmPipeline::commitPipelinesAtomic({this}, CommitMode::Test, frame, {}) == Error::None;
    }
    ...
}
```

KWin calls `testScanout` once per scanout candidate per frame, before deciding
whether to direct-scan-out the window or fall back to GPU compositing.

## Key difference from Smithay

Smithay's `DrmSurface::test_state(planes, allow_modeset)` wraps the same ioctl
via `AtomicCommitFlags::TEST_ONLY`. To use it from Otto we need `framebuffer::Handle`
values, which require importing the dmabuf through `GbmFramebufferExporter` first.

The `GbmFramebufferExporter::add_framebuffer(drm, ExportBuffer::Dmabuf(&dmabuf), false)`
path calls `framebuffer_from_dmabuf` → `gbm_bo_import` + `drmModeAddFB2WithModifiers`
internally — the same path KWin takes.

## Conclusion

Add `test_overlay_planes()` to Smithay's `DrmCompositor` that:
1. Imports each dmabuf via `framebuffer_exporter.add_framebuffer(ExportBuffer::Dmabuf(...))`
2. Builds `PlaneState` items (handle + PlaneConfig with fb handle)
3. Calls `surface.test_state(states, false)` — no modeset needed for overlay tests

This mirrors KWin's approach without duplicating the DRM property mapping that
Smithay already encapsulates.
