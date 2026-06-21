# Overlapping Overlay-Plane Scanout: Hardware Capabilities & Detection Logic

Research synthesis for Otto's KMS multi-plane scanout work (`feat/topmost-window-scanout`
in Otto, `feat/dmabuf-scanout` in our Smithay fork). Covers **which hardware can scan out
*overlapping* (alpha-blended) overlay planes** and **how to detect, per-device, exactly what
the hardware supports** (zpos mutability, blend mode, a reliable test).

Sources: kernel DRM docs (`drm_blend.c`, `drm_plane.c`), the Smithay Matrix archive,
libliftoff/wlroots/KWin source, and the zamundaaa (KWin) KMS-offloading blog posts.
Tags: **[DOC]** = documented in kernel/driver source; **[REPORTED]** = empirically observed
by compositor developers.

---

## 0. The core reframe

There is **no static query** for "can this GPU scan out N overlapping blended planes." Plane
count, blend capability, bandwidth, scaler budget, and format support are **interdependent and
runtime-dependent** (per-mode, per-format, per-CRTC-assignment). The *only* authoritative
oracle is to propose a full plane configuration and submit one **`DRM_MODE_ATOMIC_TEST_ONLY`**
commit; commit it if accepted, drop overlays and GPU-composite if rejected. This is exactly what
KWin, wlroots, and libliftoff do. [DOC/REPORTED]

**"Hardware forbids overlapping planes" is largely a myth.** It traces to *Weston's userspace
policy* of refusing overlap when zpos was ill-defined on early hardware — not a hardware ban.
Intel and AMD both blend overlapping planes in hardware. The real gates are bandwidth, scaler
limits, format, and (on some SoCs) **fixed Z-order**, not geometric overlap itself.

### What this means for our Smithay fork

Upstream Smithay's `DrmCompositor` (`src/backend/drm/compositor/mod.rs`) **vetoes overlap purely
by geometry, before ever asking the hardware**:

- `:3967` — refuses a non-opaque overlay element that overlaps primary-plane content.
- `:3974-3994` — `overlaps_with_plane_underneath`: refuses any element whose geometry overlaps a
  lower-zpos plane that already has an element assigned.

That conservative rule *is* the "scanout logic that does not allow overlapping elements." It is
correct as a safe default but leaves performance on the table on hardware that genuinely blends
overlapping planes. Our `test_overlay_planes()` (added in `feat/dmabuf-scanout`, the uncommitted
work in `compositor/mod.rs`) is the missing primitive to **replace the blanket geometric veto with
a real per-hardware `TEST_ONLY` probe** — on hardware that passes, overlap is allowed; on hardware
that rejects, we fall back exactly as today.

---

## 1. Hardware compatibility for overlapping scanout

Three classes, by what the hardware lets you do with stacking order:

### Class A — Fully reorderable (mutable zpos): overlap tier works as designed

| Family | Overlay planes | zpos | Notes |
|---|---|---|---|
| **AMD** amdgpu / DCN | ≤ HUBP pipe count (4–6), **shared globally across all CRTCs** | mutable (overlay 0–254; primary immutable but its value = #overlays since ~6.10; cursor 255) | MPC block blends multiple planes. **Only vendor that allows overlay *below* primary** (true underlays). Overlay format **ARGB8888/XRGB8888 only**; downscale ≥¼×, upscale ≤16×, min 12px. ⚠ **Slow TEST_ONLY (tens of ms)**, freeze reports → Plasma 6.5 ships it **off by default**. |
| **Intel** i915 / xe | gen9 ≈ 2 overlay/pipe (topmost hidden, exclusive w/ cursor); gen11+/Xe up to ~7 universal/pipe | mutable on modern gens (normalized); **read the flag** | i915 & xe **share the same display code** — identical behavior. No overlap ban. Gate is CDCLK/DBUF/memory-bandwidth (→ `-EINVAL`/`-ERANGE`) + ~2 pipe scalers/pipe. **Requires `pixel blend mode = Pre-multiplied`** on ARGB overlays or TEST_ONLY returns EINVAL (the i915 fix we already committed). Generally the *safest default*. |
| **Rockchip VOP2** RK3568/RK3588 | ≤8 windows (3568) / 10 layers (3588), ≤4 VPs | mutable (0..nwins−1) | Free reorder + blend. |
| **ARM Mali komeda** D71/D32/D77 | ≤8 composed layers, 2 pipelines | mutable (0–8) | `compiz` blend. |
| **Allwinner** DE2/DE3 | VI+UI channels, 4 overlays/channel | mutable (0..cnt−1) | premulti/coverage blend. |
| **Qualcomm MSM/DPU** | SSPP planes → blend stages (≤ `max_mixer_blendstages`, ~7+) | mutable (0–255) | per-plane alpha + None/Premulti/Coverage. |

### Class B — Fixed Z-order (immutable or absent zpos): overlap blends, but you must *honor* the hardware stack

| Family | Planes | zpos | Notes |
|---|---|---|---|
| **NXP i.MX8 DCSS** | exactly 3 (zpos 0/1/2) | **immutable** | blends, order fixed. |
| **ARM Mali malidp** DP500/550/650 | LV1, LV2, LG, LS | **none** (fixed by layer type) | has `alpha` + `pixel blend mode`; ordering fixed. |
| **Rockchip VOP** RK3399/RK3288 | win0/1/3 = primary/overlay/cursor | **none** (fixed by window) | known "cursor disappears" bug is a direct symptom of fixed Z. |

For Class B: don't *set* zpos — read the immutable order and pack your layers into the existing
stack. Overlap still blends.

### Class C — No overlay planes at all: tier is inert

| Family | State |
|---|---|
| **NVIDIA** nvidia-drm (proprietary + nvidia-open), through 570.x | **Primary + cursor only.** Verbatim, unchanged 2019→2025: *"The NVIDIA DRM KMS implementation does not yet register an overlay plane: only primary and cursor planes are currently provided."* 2024–26 work (explicit sync in 555.58, HW cursor) fixed sync/cursor, **not** overlay offload. |
| **NVIDIA nouveau** | Unconfirmed — treat as no overlay until probed. |

Otto already disables overlays on NVIDIA by driver-name match — that's mechanically correct
(there are no `DRM_PLANE_TYPE_OVERLAY` objects to assign to).

---

## 2. Detection logic — building a per-hardware capability profile

Two-stage pattern (libliftoff / wlroots / KWin all converge on this).

### Stage 1 — static pre-filter, read once at output setup

Per plane, via `drmModeObjectGetProperties(fd, plane_id, DRM_MODE_OBJECT_PLANE)` then
`drmModeGetProperty` on each id, match by `->name`:

1. **`type`** (immutable) — PRIMARY / CURSOR / OVERLAY. No OVERLAY ⇒ no offload tier (NVIDIA).
2. **`possible_crtcs`** — which CRTC(s) this plane can serve.
3. **`zpos`** — present? **`flags & DRM_MODE_PROP_IMMUTABLE`** → fixed order (honor it, Class B);
   clear → mutable, read min/max range (Class A). This single flag is how you distinguish
   "custom zpos" hardware from "fixed zpos" hardware.
4. **`pixel blend mode`** — present? which of None / **Pre-multiplied** (default) / Coverage are
   in the enum. Set Pre-multiplied for ARGB overlays (mandatory on i915).
5. **`alpha`** — present? plane-wide opacity 0..0xffff.
6. **`rotation`** — supported rotate/reflect bits.
7. **`IN_FORMATS`** (+ `IN_FORMATS_ASYNC` for async flips) — parse the `drm_format_modifier_blob`
   into a (format, modifier) whitelist; reject incompatible buffers before any commit.
8. **Plane count by type** + total enabled-plane budget. Treat as a **shared dynamic pool**
   (especially AMD pipes): acquire by trying until TEST_ONLY fails; be ready to revoke on hotplug.

### Stage 2 — authoritative validation per candidate config

9. **`DRM_MODE_ATOMIC_TEST_ONLY`**, the only reliable check. **Must include the primary plane** —
   some drivers (i915 bandwidth validation; libliftoff allocates primary first) reject otherwise.
   This is precisely what our `test_overlay_planes(primary, overlays)` does.
10. **Candidate gating before testing** (KWin-style, to bound test count): dmabuf-backed,
    completely unobstructed, effect-free, updated ≥~20fps.
11. **Cache last-known-good config**; only re-search when plane config (set/size/format/scale/CRTC)
    changes — **critical on AMD** where each test is tens of ms. Don't re-test on buffer-content
    change alone.
12. **All-or-nothing fallback** to GPU composition on any test failure (the `LayerScanoutElement`
    fallback contract Otto already has).

### zpos handling ("custom zpos")

- Mutable zpos: the kernel normalizes arbitrary values to a dense 0..N−1 via
  `drm_atomic_normalize_zpos()` (only if the driver sets the `normalize_zpos` flag). **Sort rule:
  by zpos, ties broken by plane object ID (higher id = on top).** Equal zpos without normalization
  is *undefined* — always assign distinct values.
- Smithay already classifies underlay vs overlay purely by comparing a plane's zpos to the
  primary's (`compositor/mod.rs:2230,3950`). Underlays (overlay below primary) **only work on AMD**
  and need the primary plane to have **≥8-bit alpha** (ABGR1010102's 2-bit alpha is not enough —
  drakulix, Smithay archive).

### Decoding *why* a test failed

There is **no commit-failure feedback API yet** (drakulix, archive) — you only get an errno from
TEST_ONLY. Our `test_overlay_planes` already extracts it from `AccessError::source.raw_os_error()`:

- **`-EINVAL` (22)** — geometry / format / unsupported state / missing `pixel blend mode`.
- **`-ERANGE` (34)** / `-ENOSPC` — bandwidth / watermark / FIFO / DBUF oversubscription
  (mode-dependent: a set that passes at 1080p60 can fail at 4K144 — re-test per mode).
- **`-EDEADLK`** — lock contention; retry the whole atomic transaction.
- **`-ENOMEM` (12)** — allocation failure.

Treat EINVAL/ERANGE/ENOSPC as "backtrack / drop this overlay"; other errno as fatal.

---

## 3. Reliability caveats (apply to every vendor)

1. A **PASS is not permanent** — results change after VT-switch / DPMS / suspend-resume / bandwidth
   load. Re-test; don't cache across these events.
2. **Legacy (non-atomic) drivers can't test** — our `test_overlay_planes` returns `true`
   unconditionally there (documented caveat in the method). Gate the whole tier on atomic support.
3. **TEST_ONLY is not free** — tens of ms on AMD when changing overlay state. Minimize config churn.
4. **Property mutability is per-driver** (drakulix, archive): *"properties that can be IMMUTABLE on
   some drivers and not on others… you test it on Intel, merge it, and it blows up on AMD."* Never
   assume zpos/blend is settable — read the flag per device.
5. **Plane count is not a stable per-output number** — it's a shared pool (AMD pipe-split can
   consume extra pipes). wlroots' rule: keep taking planes until it fails; revoke on reshuffle.

---

## 4. Recommended capability profile for Otto

Build a `PlaneCapabilities` struct per output at setup, then gate the scanout tier on it:

```text
struct PlaneCapabilities {
    atomic: bool,                 // tier requires atomic; else disabled
    overlay_planes: Vec<PlaneId>, // enumerated DRM_PLANE_TYPE_OVERLAY
    zpos: ZposMode,               // Mutable { min, max } | Immutable(order) | Absent
    blend_modes: BlendSupport,    // which of None/Premultiplied/Coverage
    has_plane_alpha: bool,
    formats: FormatSet,           // from IN_FORMATS, intersected w/ render formats
    underlay_capable: bool,       // overlay zpos can go below primary (AMD)
    vendor: Vendor,               // for allowlist gating
}
```

Decision flow per frame:

1. If `!atomic` or `overlay_planes.is_empty()` → tier off (NVIDIA, legacy).
2. If `vendor == AMD` → tier behind opt-in/allowlist (freeze reports); cache aggressively.
3. Pick candidate subtrees (unobstructed, dmabuf, effect-free, ≥20fps).
4. Assign zpos: if `Mutable`, set distinct descending values; if `Immutable`, honor the fixed order.
5. Pre-multiply ARGB buffers at native scanout size (1:1 dst) + set `Pre-multiplied`.
6. `test_overlay_planes(Some(primary), &overlays)` — commit on success, decode errno + fall back on
   failure.
7. Cache the winning config; re-test only on config change or VT-switch/DPMS/resume.

This replaces upstream Smithay's geometric `overlaps_with_plane_underneath` veto with a
hardware-truthful probe, while keeping the existing fallback contract intact on rejection.

---

## 5. Open items / prior art from the Smithay archive

- The maintainers (cmeissl, drakulix) have a **planned refactor** (PR #1785 landed
  `Kind::ScanoutCandidate` filtering) toward "test whole states, N-best permutations, multi-frame
  amortized testing with a time + count cap, remembering what failed." Our per-hardware profile +
  cached-config approach is compatible with that direction.
- No archive message names a GPU as *supporting overlapping blended planes* — they treat overlap as
  a **cost/ranking** problem (cmeissl's "tree of non-overlapping stacks"), not a queried capability.
  This research fills that gap: it's queryable only via TEST_ONLY, and the per-family table above is
  the static prior to bound the search.
- libliftoff bounds its search by **pruning + best-score early-out, not a wall-clock deadline**
  (correction to a common claim — verify against current `alloc.c`).
