# The lid and the power button

Otto handles the laptop lid and the hardware power button itself rather than
leaving them to systemd-logind, because it knows things logind does not: whether
an external monitor is attached, and whether someone is watching a remote
session.

All three settings apply live.

## Exact commands

```sh
# Who handles the lid: true = Otto, false = logind
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.manage_lid_switch b true

# What closing the lid does: "auto", "lock" or "disable_internal_screen"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.on_lid_close s "lock"

# What the power button does: "lock", "suspend", "shutdown" or "ignore"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.on_power_button s "suspend"
```

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `power_management.manage_lid_switch` | bool | live | `true` | `true`, `false` | `b false` | Let Otto act on the lid rather than leaving it to logind. |
| `power_management.on_lid_close` | enum | live | `auto` | `auto`, `lock`, `disable_internal_screen` | `s "lock"` | What happens when the laptop lid is closed. |
| `power_management.on_power_button` | enum | live | `lock` | `ignore`, `lock`, `suspend`, `shutdown` | `s "suspend"` | What happens when the hardware power button is pressed. |

## First: logind has to stay out of the way

**None of this works until logind is told not to act first.** Otto never sees
the event otherwise, and its settings do nothing.

```ini
# /etc/systemd/logind.conf
HandleLidSwitch=ignore
HandlePowerKey=ignore
```

Then `sudo systemctl restart systemd-logind`, or reboot.

This is a root-owned system file. **Ask the person before editing it**, and tell
them it needs sudo. If they would rather not, set
`power_management.manage_lid_switch` to `false` and let logind keep the job —
their policy then lives in `logind.conf`, not in Otto.

## What each lid value does

| Value | What happens when the lid closes |
|---|---|
| `auto` | Turn off the internal panel, then suspend — **unless the session is still in use** (see below) |
| `lock` | The same, but lock the session first, so the machine wakes to the lock screen |
| `disable_internal_screen` | Turn off the internal panel and keep running. Never suspends, never locks |

**"Still in use" means one of two things**, and under `auto` or `lock` either
one stops the suspend:

1. **An external monitor is connected** — clamshell mode. The internal panel
   goes dark, the session carries on on the external screen, and it does *not*
   lock: the person is sitting in front of it.
2. **A remote client is actually consuming frames** — a screenshare session, or
   an RDP client pulling from a virtual output. Closing the lid does not cut off
   a remote user. A stream that exists but is paused, with nothing reading it,
   does not count.

Otherwise the session is out of reach and Otto suspends, because a laptop that
keeps running in a bag gets hot.

**Reopening restores everything**: same monitor layout, same primary, same
workspaces, windows and dock. Closing and reopening is visually a no-op.
`disable_internal_screen` is the exception — the panel stays off even with the
lid open, which is what a kiosk or a display-manager host wants.

## What each power-button value does

| Value | What happens |
|---|---|
| `lock` | Launch the configured locker — the default |
| `suspend` | Suspend, through logind |
| `shutdown` | Power off, through logind |
| `ignore` | Otto stays out of it: logind's `HandlePowerKey` decides, and the key reaches the focused application as `XF86PowerOff` |

Anything other than `ignore` needs `HandlePowerKey=ignore` in `logind.conf`.

The button is read from its **raw hardware key code**, so it works whatever the
keyboard layout is and whatever has grabbed the keyboard — a fullscreen game,
the lock screen, a greeter. There is no state in which pressing it does nothing.

## Two setups to copy

A laptop that suspends and wakes locked:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.manage_lid_switch b true
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.on_lid_close s "lock"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.on_power_button s "lock"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv lock.auto_lock_timeout i 600
```

A machine that must never suspend — a media box, an always-on machine with a
display:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.manage_lid_switch b true
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.on_lid_close s "disable_internal_screen"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv power_management.on_power_button s "ignore"
```

## What Otto does not do

Do not offer these — there is no setting for them:

- **The screen does not blank on idle.** No DPMS timer; only auto-lock is
  timer-driven, and it is in [lock-screen.md](lock-screen.md).
- **No idle suspend.** Otto only suspends on lid close.
- **No hibernate or hybrid sleep.**
- **No battery-level policy** — nothing like "suspend at 5%".
- **The dock and top bar stay on the primary monitor** in clamshell mode; they
  do not move to the external screen.

`systemd-logind` and tools like `swayidle` still work alongside Otto for the
missing pieces.

## If it does not work

**It suspends on lid close even with an external monitor attached.** logind is
still set to handle the lid, so it suspends before Otto's clamshell check ever
runs. Fix `HandleLidSwitch=ignore` and restart logind.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/power-management.md
- [lock-screen.md](lock-screen.md) — idle locking, and which locker runs
