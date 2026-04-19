# Scenario: idle, no user clients

**Date:** 2026-04-18
**Otto commit:** 75fdc64 + lay-rs damage refactor (working tree)
**Workload:** Otto + autostart only (otto-bar, xdg-desktop-portal-otto, blueman-applet, etc.); no terminal, no browser; no input

## Result

- Otto CPU: **0%**
- GPU RC6: **100%** (deep sleep)
- GPU power: **0 W**
- perf record at 99Hz: **0 samples**
- intel_gpu_top: 100% sleep over 10s

## Conclusion

When no user-facing client is committing buffers, Otto correctly idles to zero. The render loop does not run when nothing has changed. This is the floor.

Useful as the "must equal this" check: any future measurement showing CPU/GPU with no clients is a regression.
