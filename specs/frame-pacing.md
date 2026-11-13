# Frame Pacing

**Status:** draft  
**Related specs:** [plane-scanout](./plane-scanout.md) (per-window frame-callback throttling), [multi-output](./multi-output.md)

## Summary

How the udev backend decides when to compose and when to tell clients to
paint, so that a client that needs most of a refresh period for a frame still
runs at the panel's rate, and one that needs little gets its frame on the
next VBlank with little latency.

## Goals

- A single window whose frame takes most of a period (GTK at 120 Hz: about
  7 ms of CPU and GPU out of 8.3 ms) is presented on every VBlank.
- A fast client is composed into the frame right after its commit, not one
  VBlank later.
- No client is driven faster than the refresh rate: at most one of its frames
  is in flight ahead of a compose, so its work is not superseded and wasted.
- When the compositor itself cannot compose every period (several heavy
  windows), clients are paced by what is actually presented.
- Every visible window receives `wp_presentation` `presented` feedback for the
  flip that showed it, with the hardware VBlank timestamp.

## Non-Goals

- Per-window plane scanout, which has its own pacing (see plane-scanout).
- The winit and x11 backends: they send callbacks right after rendering.
- Reducing a client's own frame time.

## Behavior

- The compositor composes late in the period: `draw_delay_us` picks the
  latest start at which its measured CPU plus GPU need, doubled for safety,
  still lands before the next VBlank.
- Frame callbacks are sent from the VBlank handler, as early in the period as
  possible, to every window and layer surface on the output, each with its
  visibility throttle.
- A commit is applied only when its buffer's GPU work is finished. A commit
  applied after the VBlank send, whose surface has had no callback for at
  least 95 % of a period, gets its callback as it is applied. A commit
  applied sooner than that waits for the VBlank cadence.
- Only a surface the last VBlank send served at full rate is served on
  commit, and the period is that of the output that served it. A throttled
  surface keeps its tier's rate; a surface no VBlank has served yet (a new
  one, or any surface under winit, x11 or headless) waits for the regular
  path.
- For two periods after each flip, a timer ticks at the VBlank cadence and
  sends callbacks for a VBlank at which nothing was flipped, provided the
  compositor's own CPU plus GPU need is under 60 % of the period. A real
  VBlank cancels the ticks.
- At render time callbacks are sent only when nothing was rendered, since no
  flip and so no VBlank handler will follow.
- Presentation feedback is collected for every window on the flipping output
  that is not occluded, minimized or on an inactive workspace, and marked
  presented when the flip
  completes, with the DRM timestamp and `Vsync | HwClock | HwCompletion`.

## Constraints & Edge Cases

- A client whose commit misses a VBlank send by a few hundred microseconds
  must not wait a whole extra period (that locks GTK to half the refresh);
  the on-commit send covers it.
- Sending the callback on every commit without a gap lets a client cycle at
  its own speed; two of its frames then meet one compose and the next finds
  none. Measured as dropped frames, not extra ones.
- Gating the tick on the compose interval deadlocks at half rate: a client at
  60 fps makes the compositor compose every other period, which reads as
  "not keeping up". The gate is the compositor's measured need instead.
- Windows are composited through the scene, not as per-surface render
  elements, so Smithay never records a primary scanout output for them and
  would leave their presentation feedback to be discarded on the next commit.
  Feedback is taken for every shown window on the output's flip instead.

## Rationale

With callbacks sent at render time, a client woken then has only the
compositor's budget left to produce its next buffer before the following
compose: a 7 ms GTK frame fit a 120 Hz period one time in six less than it
should (100 fps). VBlank sends alone locked it to 60 fps because its commit,
held back by the GPU fence, landed after the send. The combination measured:
single window 116 fps with frosting on and 117 with it off (from 100 and
116), five windows unchanged at 59, superseded client frames about 10 %.

## Open Questions

- The tick could align to the output's predicted VBlank instead of the
  handler's wall time plus one period.
- Presented feedback is reported for every shown window, including one whose
  buffer did not change in that compose; per-surface tracking would be
  stricter.
