# Lock Screen

Locking hides your session behind an opaque surface on every monitor and routes
all input to the locker. Everything underneath — windows, workspaces, focus,
running programs — is untouched and comes back exactly as you left it.

Otto uses `ext-session-lock-v1`, the Wayland protocol designed for this. Its
key guarantee: if the locker crashes, the screen **stays blank** rather than
revealing what was behind it.

Otto ships `otto-lock` as the default locker, but any `ext-session-lock-v1`
client works.

## Setup

### 1. Install the PAM service

`otto-lock` authenticates through PAM using a dedicated service. **The service
file must exist**, otherwise PAM falls through to `other`, which denies
everything.

Arch and Fedora packages install it for you. On Debian and Ubuntu, copy it
yourself:

```sh
sudo cp /usr/share/doc/otto/otto-lock.pam /etc/pam.d/otto-lock
```

Then edit it: the shipped file is written for distributions with a `system-auth`
stack (Arch, Fedora, openSUSE). On Debian and Ubuntu, replace `system-auth` with
`common-auth` and `common-account`:

```
auth      sufficient pam_fprintd.so
auth      include    common-auth
account   include    common-account
```

`otto-lock` does notice a missing service file and falls back to `system-auth`,
then `login` — but the fallback is not the configuration anyone reviewed, so
install the file properly.

### 2. Bind the lock action

`Ctrl+Alt+Escape` locks the session. It is handled ahead of the config from the
raw hardware key code, so it works whatever your layout is and whatever holds
the keyboard.

You can bind it elsewhere too:

```toml
[keyboard_shortcuts]
"Logo+L" = "LockSession"
```

### 3. Optionally, choose a different locker

```toml
[lock]
locker_command = "otto-lock"
locker_args = []
```

Any `ext-session-lock-v1` locker fits here — `swaylock`, `hyprlock`, `gtklock`.

## Using it

The `otto-lock` panel is a frosted card with your avatar, a password field and —
when a fingerprint reader is configured — a Touch ID mark. It shows a clock,
which keeps time however long you are away.

Type your password and press Enter, or touch the reader.

A refused attempt returns you to the field with the error shown and offers
another try. The delay between attempts comes from PAM's own rate limiting, not
from the locker.

The lock screen picks up your Otto configuration, including your own user
config, so it matches the session's theme.

## Fingerprint unlock

The shipped PAM file lists `pam_fprintd` explicitly:

```
auth sufficient pam_fprintd.so
```

It has to be listed explicitly because a distribution's `system-auth` usually
does not include it — a reader configured for `sudo` or polkit is configured in
*those* services, not in the shared stack.

`sufficient` means a recognised finger is enough, and anything else falls
through to the password prompt below. The module holds the conversation open
until it times out, and anything you type meanwhile waits for the prompt that
follows — which is what the panel's "Enter Password" button is for.

Enroll fingers with `fprintd-enroll` first. On a machine with no reader, delete
the line.

## Automatic locking

```toml
[lock]
auto_lock_timeout = 300   # seconds; 0 (the default) never auto-locks
```

The countdown measures the absence of keyboard, pointer, touch and tablet
input. Any input event resets it, regardless of what the compositor does with
it afterwards.

### Idle inhibitors

A client holding an `idle-inhibit-unstable-v1` inhibitor — a video player during
playback, a presentation tool — holds auto-lock off, and **restarts** the
countdown when it releases the inhibitor. So the timer runs from when the video
stops, not from your last keypress.

Otto only honours an inhibitor while its surface is alive and its window is not
minimized. The protocol leaves that judgment to the compositor precisely
because clients forget to drop inhibitors, and one stale inhibitor would
otherwise disable locking for the whole session.

The check runs on the timer's tick, so an inhibitor released just after a tick
can delay the lock by up to one further timeout.

## What still works while locked

| | |
|---|---|
| `Ctrl+Alt+F1`…`F12` | VT switching — always available |
| `Ctrl+Alt+Escape` | Lock (already locked, so a no-op) |
| Power button | Runs your `on_power_button` action |
| Everything else | Nothing. The locker owns the keyboard; all configured shortcuts are inactive. |

Multiple monitors are all covered, including ones plugged in, unplugged, or
mode-changed while locked.

If the locker crashes, Otto restarts it — rate-limited — so a crash is
recoverable without a VT switch, and the screen never uncovers in the meantime.

## Locking from a script

Anything that wants to lock runs the same command:

```sh
otto-lock
```

That is what the shortcut and the auto-lock timer both do. A suspend hook or an
external idle daemon (`swayidle`) can call it directly.

## Locking and the lid

`on_lid_close = "lock"` locks before suspending, so the machine wakes to the
lock screen. Clamshell and remote sessions deliberately stay unlocked — the
session is still in use. See [Power Management](power-management.md).

## Troubleshooting

**Nothing happens when I press `Ctrl+Alt+Escape`.** The locker failed to launch.
Run `otto-lock` from a terminal inside the session to see the error.

**My password is rejected even though it is correct.** The PAM service file is
missing or wrong. Check `/etc/pam.d/otto-lock` exists and uses the right stack
for your distribution (`system-auth` vs `common-auth`). The log says which
fallback the locker resorted to.

**The screen is blank but there is no password panel.** The locker died after
locking. Otto keeps the screen blank — that is the protocol working as
designed. Switch VT with `Ctrl+Alt+F2`, log in, and check the logs.

**The fingerprint reader is not offered.** Confirm `pam_fprintd.so` is in
`/etc/pam.d/otto-lock` and that you have enrolled a finger
(`fprintd-list $USER`).

**The session never auto-locks.** `auto_lock_timeout` defaults to `0`, which
means never. If it is set and still does not fire, a client is probably holding
an idle inhibitor — check for a paused-but-not-closed video.

## See also

- [Login Greeter](login-greeter.md) — the same panel, for logging *in*
- [Power Management](power-management.md) — locking on lid close and power button
