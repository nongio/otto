# The Debug Action Hook

Otto's keyboard shortcuts are matched inside the libinput handler, before the
key ever reaches a client. Keys synthesised through the virtual-keyboard
protocol (`wtype`, `wlrctl`, the RDP bridge) deliberately skip that filter and
go straight to the focused surface — see
`src/state/virtual_keyboard_handler.rs`. A script therefore *cannot* drive
compositor UI by pressing keys.

The action hook is the way in. Once per event-loop iteration each backend
reads a file; if it holds the name of a builtin shortcut action, Otto runs
that action exactly as if its key had been pressed, then requests a redraw so
the scheduled `lay-rs` transactions actually tick.

```sh
echo ExposeShowAll > /tmp/otto-action
```

The file is consumed (deleted) when it is read.

## `OTTO_ACTION_FILE`

The path defaults to `/tmp/otto-action` and is overridden by the
`OTTO_ACTION_FILE` environment variable, read from Otto's own environment:

```sh
OTTO_ACTION_FILE=/run/user/1000/otto-harness.action cargo run --release
...
echo ApplicationSwitchNext > /run/user/1000/otto-harness.action
```

Give each session its own path when more than one Otto is running on the
machine — a harness on a tty and a nested `--winit` instance otherwise fight
over the same well-known file.

## Action names

Any name `parse_builtin_name` accepts (`src/config/shortcuts.rs`) — the same
names the `[shortcuts]` config table uses:

| Group | Names |
|-------|-------|
| Overview | `ExposeShowAll`, `ExposeShowDesktop` |
| App switcher | `ApplicationSwitchNext`, `ApplicationSwitchPrev`, `ApplicationSwitchNextWindow`, `ApplicationSwitchQuit` |
| Windows | `ToggleMaximizeWindow`, `TileWindowLeft`, `TileWindowRight`, `CloseWindow`, `ToggleDecorations` |
| Session | `Quit`, `LockSession` |
| OSD | `BrightnessUp`, `BrightnessDown`, `VolumeUp`, `VolumeDown`, `VolumeMute` |
| Media | `MediaPlayPause`, `MediaNext`, `MediaPrev`, `MediaStop` |
| Debug dumps | `SceneSnapshot` (alias `ExportSceneJson`), `SkpSnapshot` (alias `ExportSceneSkp`) |

`Workspace` and `Screen` need an index, which the bare-name parser cannot
carry, so they are not reachable through the hook — a workspace switch bound
to a shortcut still resolves to `WorkspaceNum` and runs, but only via a real
key press. Backend-specific actions (`VtSwitch`, `ScaleUp`, `ScaleDown`,
`RotateOutput`, `Screen`) are dispatched by the per-backend keyboard handlers;
reaching them from the hook logs a warning rather than doing anything.

An unknown name logs `unknown debug action: …` and is otherwise ignored.

## Where it lives

`Otto::poll_debug_action_file` and `process_debug_key_action` in
`src/input/actions.rs`. Both backends — `src/winit.rs` and `src/udev/init.rs`
— call the same function and differ only in how they ask for the redraw
afterwards, so the two are guaranteed to accept the same set of actions.

Note that `process_common_key_action` warns and drops anything it does not
own; the window-management actions live in the per-backend *keyboard*
dispatchers. `process_debug_key_action` handles those explicitly, which is why
the hook can drive the app switcher and the tiling actions at all.
