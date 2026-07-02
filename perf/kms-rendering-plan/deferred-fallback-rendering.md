# Deferred: Plane assignment fallback rendering

**Question:** When we render an element into its own SceneDmabufElement plane and the DRM
compositor fails to assign it to a hardware overlay, what happens?

Currently `SceneDmabufElement::draw()` is a no-op — so if plane assignment fails the
element is invisible (black/transparent gap).

**Desired behaviour:** When plane assignment fails and `draw()` is called, we should
composite the element's dmabuf content into whatever surface `draw()` is rendering into
(the primary plane's GPU compositor pass). This means instead of a no-op, `draw()` should
blit the existing dmabuf texture into the destination framebuffer.

**Related question:** When we render a SceneDmabufElement plane, we currently clear the
GBM buffer before drawing. But if we fall back to GPU compositing, we are rendering into
the *previous* overlay (the primary plane's accumulated buffer). Should we:

1. Keep the clear before drawing (correct for hardware scanout, but wrong for fallback).
2. Skip the clear on fallback and composite into the existing buffer.
3. Implement `draw()` as a real GPU blit of the dmabuf so the content appears in the
   primary plane's composited output regardless of hardware assignment.

**Preferred approach:** Option 3 — implement `draw()` as a GPU blit (`import_dmabuf` +
`render_texture`). This way the plane always displays correctly whether it lands on a
hardware overlay or falls through to GPU compositing on the primary plane.
