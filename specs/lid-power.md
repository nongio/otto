# Lid Switch & Power Management

**Status:** draft
**Related specs:** [multi-output.md](./multi-output.md), [rdp-bridge.md](./rdp-bridge.md), [screenshare.md](./screenshare.md)

## Summary

Otto owns the laptop lid switch: closing the lid disables the internal panel
(preserving all workspace state) and, when nothing else needs the machine
awake, suspends the system. Reopening the lid restores the panel to its exact
pre-close arrangement. systemd-logind is expected to be configured with
`HandleLidSwitch=ignore` so Otto can gate the suspend decision.

## Goals

- Closing the lid on a plain laptop (no external monitor, no remote session)
  suspends the system — the machine must not keep running hot in a bag.
- Closing the lid with an external monitor connected enters clamshell mode:
  internal panel off, session keeps running on the external monitor.
- Closing the lid while a remote client is actively consuming frames (RDP
  bridge or portal screenshare) keeps the session running so the remote user
  is not cut off.
- Reopening the lid restores the internal panel with its previous position,
  primary status, workspaces, windows, and dock — no visual difference from
  before the close.

## Non-Goals

- Idle-timeout suspend / DPMS blanking (not implemented).
- Migrating primary-only chrome (dock, app switcher) to another output while
  the panel is suspended.
- Hibernate, hybrid sleep, or battery-level policies.

## Behavior

Configuration (`[power_management]`):

- `manage_lid_switch` (default `true`) — when `false`, Otto ignores the lid
  switch entirely and all handling is delegated to systemd-logind.
- `on_lid_close` — `"auto"` (default) or `"disable_internal_screen"`.
- `on_power_button` — `"lock"` (default), `"suspend"`, `"shutdown"` or
  `"ignore"`.

On lid close (`manage_lid_switch = true`):

1. Every internal (laptop-panel) output is suspended: its DRM surface and
   Wayland output global are torn down, but workspaces, windows, and scene
   layers are kept intact. The output's position and primary status are
   recorded.
2. In `"auto"` mode, if no external monitor is connected and no remote client
   is actively consuming frames, Otto invokes a system suspend (via logind).
   Otherwise the system keeps running.
3. In `"disable_internal_screen"` mode the system never suspends (kiosk /
   display-manager use); the panel is kept off even with the lid open.

"Remote client actively consuming frames" means: at least one portal
screenshare session exists, or at least one virtual output's PipeWire stream
is in `Streaming` state (a consumer such as the RDP bridge is linked and
pulling frames). A stream that is merely created but `Paused` (no consumer)
does not block suspend.

On lid open:

1. The internal panel is reconnected as a new DRM surface/output.
2. If the connector was previously suspended, its recorded position and
   primary status are restored — it must NOT be auto-placed after outputs
   (e.g. virtual outputs) that kept running while it was suspended, and it
   must reclaim primary if it was primary before, so the dock and window
   positions return exactly to their pre-close state.
3. If the recorded position now overlaps another output, it falls back to
   auto-placement (outputs never overlap).

The suspend decision is edge-triggered on the panel teardown: a repeated
power-state evaluation with the lid still closed (e.g. wake with the lid
shut) does not immediately re-suspend.

On power button press:

- With `on_power_button = "ignore"` Otto does not touch the key: it goes
  through the normal keyboard path and is delivered to the focused client as
  `XF86PowerOff`, leaving the policy to systemd-logind's `HandlePowerKey`.
- Otherwise — including the `"lock"` default — the key is intercepted from its
  raw keycode (evdev `KEY_POWER`),
  before lock-screen / greeter / exclusive-layer keyboard grabs, so it works
  from a locked session or a fullscreen client — the same treatment VT
  switching and Ctrl+Alt+Escape get. The key is then not delivered to any
  client.
- `"lock"` takes the same path as the `LockSession` action: it launches the
  configured locker (no-op if the session is already locked).
- `"suspend"` and `"shutdown"` delegate to logind (`systemctl suspend` /
  `systemctl poweroff`).

## Constraints & Edge Cases

- Suspending is delegated to logind (`systemctl suspend`); Otto does not
  freeze itself. On resume the session pause/activate path revalidates DRM
  state.
- The saved suspend record is keyed by connector name and consumed on
  reconnect; a real disconnect (`unmap_output`) discards it.
- While the panel is suspended, another output (often a virtual one) becomes
  primary and the flattened model follows it; primary-only chrome does not
  migrate (see Non-Goals) and simply reappears with the panel.
- systemd-logind must be configured with `HandleLidSwitch=ignore` (and
  variants); otherwise logind suspends unconditionally before Otto can gate
  the decision. The same applies to `HandlePowerKey`, which must be `ignore`
  for the default `on_power_button = "lock"` to be reached — otherwise logind
  powers the machine off first.

## Rationale

- Otto gates suspend (GNOME/KWin model) instead of logind, because the
  compositor is the only component that knows whether a virtual output is
  being consumed remotely — the primary use case for keeping a closed laptop
  awake.
- Suspend-on-lid-close is the default because leaving the machine running in
  a closed bag is a thermal hazard; remote workflows that need the machine
  awake keep it awake automatically (active client) or explicitly
  (`disable_internal_screen`).

## Open Questions

- Should an *expected* remote session (RDP bridge running but no client
  connected yet) block suspend, so one can close the lid first and connect
  later? Today it does not — use `disable_internal_screen` for that.
