# Lock screen

Which locker runs, what it is given, and when the session locks itself. Otto
only hides the session behind the locker — it links no PAM code and never sees
a password.

## Exact commands

```sh
# Lock after 5 minutes of no input. 0 never locks
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv lock.auto_lock_timeout i 300

# Which locker runs. Any ext-session-lock-v1 client
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv lock.locker_command s "otto-lock"

# Arguments for it; the number says how many follow
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv lock.locker_args as 0
```

**Ask before changing `lock.locker_command`.** A locker that does not start
leaves the session unlockable.

**Apply** is `live` (now) or `restart` (at the next login).

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `lock.locker_command` | string | live | `otto-lock` | free text | `s "hyprlock"` | The locker launched to lock the session. |
| `lock.locker_args` | string-list | live | `[]` | list of strings | `as 0` | Arguments passed to the locker. |
| `lock.auto_lock_timeout` | int | live | `0` | 0 – 86400, step 60 | `i 300` | Seconds of inactivity before locking. 0 never locks. |

## In the file

```toml
[lock]
locker_command = "otto-lock"
locker_args = []
auto_lock_timeout = 300   # seconds of no input; 0 never locks

[keyboard_shortcuts]
"Logo+L" = "LockSession"
```

Any `ext-session-lock-v1` locker fits — `otto-lock`, `swaylock`, `hyprlock`,
`gtklock`.

## Worth knowing

- **`Ctrl+Alt+Escape` always locks.** It is read from the raw keycode ahead of
  the config, so it works whatever the layout is and whatever holds the
  keyboard. A binding is only for a second, more convenient key.
- **The PAM service file must exist.** `otto-lock` authenticates through
  `/etc/pam.d/otto-lock`; without it PAM falls through to `other`, which denies
  everything. Arch and Fedora packages install it; on Debian and Ubuntu copy
  `/usr/share/doc/otto/otto-lock.pam.example` and swap `system-auth` for
  `common-auth` / `common-account`.
- **Fingerprint unlock** needs `auth sufficient pam_fprintd.so` listed
  explicitly in that file — a distribution's shared stack usually does not
  include it. Enrol with `fprintd-enroll` first; on a machine with no reader,
  delete the line.
- **The idle countdown measures all input** — keyboard, pointer, touch,
  tablet — and any event resets it. A client holding an
  `idle-inhibit-unstable-v1` inhibitor (a video player, a presentation) holds
  locking off and *restarts* the countdown when it lets go, so the timer runs
  from when the video stopped.
- **The locker reads Otto's configuration**, including the user's own, so it
  matches the session's theme.
- If the locker crashes the screen stays blank rather than revealing what was
  behind it. That is the protocol's guarantee and the reason for it.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/lock-screen.md
- [greeter.md](greeter.md) — the same panel, at login
