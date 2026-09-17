# Login greeter

Otto started with `--login` is a login screen: a host compositor for a greeter
client, with `otto-greeter` as the panel. Authentication belongs entirely to
greetd — Otto links no PAM code and never touches the password.

## Exact commands

```sh
# Which greeter runs in login mode. Both need a restart
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv login.greeter_command s "otto-greeter"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv login.greeter_args as 0
```

**Ask before changing either.** A greeter that does not start is a machine
nobody can log into. These are also usually set in `/etc/otto/config.toml`, not
the user's own file — see below.

**Apply** is `live` (now) or `restart` (at the next login).

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `login.greeter_command` | string | restart | `otto-greeter` | free text | `s "otto-greeter"` | The greeter launched in login mode. |
| `login.greeter_args` | string-list | restart | `[]` | list of strings | `as 0` | Arguments passed to the greeter. |

Both need a restart, which for a greeter means restarting greetd rather than
logging out.

## Setting it up

```toml
# /etc/greetd/config.toml
[terminal]
vt = 1

[default_session]
command = "otto --login"
user = "greeter"
```

```sh
sudo systemctl enable --now greetd
sudo systemctl disable gdm     # or sddm, lightdm — only one display manager
```

```toml
# /etc/otto/config.toml
[login]
greeter_command = "otto-greeter"
greeter_args = []
```

## Worth knowing

- **It reads `/etc/otto/config.toml`, not yours.** The greeter runs as the
  `greeter` user and cannot see `~/.config/otto/config.toml`. Theme, wallpaper
  and scale for the login screen go in the system file — this is the usual
  reason a greeter looks nothing like the session.
- **Login mode is a different desktop.** One monitor (the first connector
  brought up; every other is ignored), no dock, no app switcher, no exposé, no
  `exec_once`, no XDG autostart, no auto-lock. Exactly one client is launched.
- **`--login` is orthogonal to the backend**, so it combines with `--tty-udev`
  in production and `--winit` for development.
- **The greeter dies with Otto** — it gets `SIGTERM` if the compositor goes.
- **The username is pre-filled** with the login account of lowest UID, as a
  suggestion: typing replaces it, Escape empties it.
- A greeter that does not start is a machine nobody can log into. Check the
  command exists before writing it, and keep a VT or an SSH session open while
  testing.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/login-greeter.md
- https://github.com/nongio/otto/blob/main/docs/user/lock-screen.md
