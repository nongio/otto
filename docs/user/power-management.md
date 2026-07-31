# Power Management

Otto handles the laptop lid switch and the hardware power button itself, rather
than leaving them to systemd-logind. This lets it make smarter decisions — it
knows whether you have an external monitor attached, or whether someone is
watching a remote session.

## Prerequisite: tell logind to stay out of the way

For any of this to work, logind must be configured not to act first:

```ini
# /etc/systemd/logind.conf
HandleLidSwitch=ignore
HandlePowerKey=ignore
```

Then `sudo systemctl restart systemd-logind` (or reboot).

If you skip this, logind suspends the machine before Otto sees the event, and
Otto's settings have no effect.

## The lid switch

```toml
[power_management]
manage_lid_switch = true
on_lid_close = "auto"
```

### `manage_lid_switch`

`true` (the default) means Otto owns the lid. `false` hands everything back to
logind — set your policy there instead.

### `on_lid_close`

| Value | Behaviour |
|-------|-----------|
| `"auto"` | Turn off the internal panel, then suspend **unless** the session is still in use |
| `"lock"` | Same as `"auto"`, but lock the session first, so the machine wakes to the lock screen |
| `"disable_internal_screen"` | Turn off the internal panel and keep running. Never suspend, never lock. |

### What "still in use" means

With `"auto"` or `"lock"`, Otto does **not** suspend when either is true:

- **An external monitor is connected** — clamshell mode. The internal panel goes
  dark, the session keeps running on the external screen, and it does *not*
  lock: you are sitting in front of it.
- **A remote client is actively consuming frames** — a portal
  [screenshare](screen-sharing.md) session exists, or a virtual output's
  PipeWire stream is actually streaming (an [RDP](remote-desktop.md) client is
  connected and pulling frames). Closing the lid does not cut off a remote user.

  A stream that merely exists but is paused, with nothing consuming it, does not
  block suspend.

Otherwise the session is out of reach, and Otto suspends — because a laptop that
keeps running in a bag gets hot.

### Reopening

The internal panel comes back exactly as it was: same position in the monitor
layout, same primary status, same workspaces, same windows, same dock. Closing
and reopening the lid is visually a no-op.

`disable_internal_screen` is the exception — it keeps the panel off even with
the lid open, which is what you want for a kiosk or a display-manager host.

## The power button

```toml
[power_management]
on_power_button = "lock"
```

| Value | Behaviour |
|-------|-----------|
| `"lock"` | Launch the configured locker (the default) |
| `"suspend"` | Suspend via logind |
| `"shutdown"` | Power off via logind |
| `"ignore"` | Otto stays out of it — logind's `HandlePowerKey` decides, and the key reaches the focused application as `XF86PowerOff` |

Anything other than `"ignore"` needs `HandlePowerKey=ignore` in `logind.conf`,
otherwise logind acts first.

Otto reads the power button from its **raw hardware key code**, so it works
whatever your keyboard layout is and whatever has grabbed the keyboard — a
fullscreen game, a lock screen, a greeter. There is no state in which pressing
the power button does nothing.

## Idle locking

Auto-lock after a period of inactivity is configured separately, under `[lock]`:

```toml
[lock]
auto_lock_timeout = 300   # seconds; 0 disables
```

See [Lock Screen](lock-screen.md) for the full picture, including how
`idle-inhibit` clients (video players, presentations) hold the countdown off.

## What is not implemented

- **DPMS blanking on idle.** The screen does not turn off by itself. Only
  auto-lock is timer-driven.
- **Idle suspend.** Otto never suspends on a timer, only on lid close.
- **Hibernate and hybrid sleep.**
- **Battery-level policies** — no "suspend at 5%".
- **Moving the dock and top bar to the external monitor** while the internal
  panel is off in clamshell mode. They stay assigned to the primary monitor.

For the missing pieces, `systemd-logind` and tools like `swayidle` still work
alongside Otto.

## A couple of recipes

**Laptop that suspends and wakes locked:**
```toml
[power_management]
manage_lid_switch = true
on_lid_close = "lock"
on_power_button = "lock"

[lock]
locker_command = "otto-lock"
auto_lock_timeout = 600
```

**Machine that should never suspend (media box, always-on server with a display):**
```toml
[power_management]
manage_lid_switch = true
on_lid_close = "disable_internal_screen"
on_power_button = "ignore"
```

## Troubleshooting

**The machine suspends on lid close even with an external monitor attached.**
Check that logind is set to `HandleLidSwitch=ignore` — otherwise it is logind
suspending, not Otto, and clamshell detection never runs.

**Closing the lid cut off my screen share.** Otto only detects an *actively
streaming* PipeWire node. If the consumer had disconnected or paused, the
session counted as unused. Check the log around the lid-close event.

**The power button does nothing.** With `on_power_button = "ignore"` that is
expected: the key goes to the focused application. Any other value requires
`HandlePowerKey=ignore` in `logind.conf`.

**The session does not lock on lid close.** Clamshell and remote sessions
deliberately stay unlocked — the session is still in use. It also needs a
working locker; see [Lock Screen](lock-screen.md).
