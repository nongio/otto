# apps-manager

`apps-manager` is a command-line probe for Otto's `ext_foreign_toplevel_list_v1` support. It connects as a Wayland client and prints each window as it opens, changes title or app id, and closes, until it is stopped with Ctrl+C. Despite the name it is not a launcher and draws nothing. It is a work in progress.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** none.
- **External:** `ext_foreign_toplevel_list_v1` from the compositor, through `wayland-client` and `wayland-protocols`.

## Develop

- Start a dev compositor with `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p apps-manager` and open some windows.
- No tests.

## Code map

- [src/main.rs](../../components/apps-manager/src/main.rs): the whole tool, a registry listener and the toplevel handle events.

## See also

- [README](../../components/apps-manager/README.md)
- [Foreign toplevel](foreign-toplevel.md)
