# otto-msg

`otto-msg` drives the Otto compositor from a script. It runs i3-syntax commands, prints the window tree, workspaces, outputs and inputs as JSON, and follows events. It is a command-line client of the compositor's `org.otto.Shell1` D-Bus service, with no Wayland connection of its own.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app. The Otto plugin's skills call it.

## Dependencies

- **Internal:** `otto-dbus` (the `org.otto.Shell1` proxy).
- **External:** a running Otto on the session bus. Blocking zbus and serde_json.

## Develop

- `cargo run -p otto-msg -- focus right`, or `cargo run -p otto-msg -- -t get_tree`. `--help` lists the options.
- It talks to whichever Otto owns `org.otto.Shell1` on the session bus.
- Tests: `cargo test -p otto-msg` (argument parsing). The commands themselves are tested in the compositor, without the bus: `cargo test --features headless --test tiling_scripting`.

## Code map

- [src/main.rs](../../components/otto-msg/src/main.rs): the whole tool. Argument parsing, one call per `-t` type, and JSON printing.
- The service lives in the compositor, in `src/shell_service.rs`.

## Rules

- The wire contract is [shell-dbus-api.md](shell-dbus-api.md). A change to the commands or the JSON goes there and into the user guide, [scripting.md](../user/scripting.md).
- The user guides are copied into the Otto plugin's help skill. After editing one, run `scripts/sync-skill-docs.sh`; CI checks the copies with `--check`.

## See also

- [Shell D-Bus API](shell-dbus-api.md)
- [otto-dbus](otto-dbus.md)
- [Scripting guide](../user/scripting.md)
- [Tiling plan](tiling-plan.md)
