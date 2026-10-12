# otto-stash

`otto-stash` collects things to ask about (selected text, files, a screen region) into one card, then hands them to Ask together. It is a Wayland client and a D-Bus service: one long-running instance owns `org.otto.Stash1`, and the `add`, `add-file`, `add-region`, `send` and `cancel` subcommands reach it over the bus. It is a proof of concept.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` (surfaces, drawing, the stashed-items view, the Stash, Files and Settings proxies), `otto-peek` (thumbnails, decoded in a sandboxed worker).
- **External:** from the compositor, `zwp_input_method_v2` (the focused field's selection), wlr-data-control (the primary selection), layer shell, and optionally `otto-canvas-v1` version 3 (the card in the side canvas) and version 4 (a drop target during a drag). `slurp` and `grim` for `add-region`. Built on smithay-client-toolkit and zbus.

## Develop
- Otto autostarts it and binds the subcommands to shortcuts (see `otto_config.example.toml`).
- To run your build, stop the running instance with `pkill -x otto-stash`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-stash`.
- Drive it with `cargo run -p otto-stash -- add` (or `send`, `cancel`, `add-file PATH`, `add-region`).
- `OTTO_STASH_LAUNCHER` names the launcher `send` opens (default `otto-launcher`). Logs follow `RUST_LOG`.
- Tests: `cargo test -p otto-stash`.

## Where things are
- `src/main.rs` holds the subcommands and the event loop; `src/request.rs` is what is stashed, in order, until it is sent.
- `src/dbus.rs` is `org.otto.Stash1`, and asks Files for its selection.
- `src/card.rs` places the card in the side canvas (`src/canvas.rs`) or as a floating balloon (`src/panel.rs`); `src/balloon.rs` draws it.
- The sources each have a module: primary selection, region capture, dropped files and the drop target.

## Rules
- otto-stash never takes keyboard focus: the app in front keeps its selection and caret.
- `otto_peek::run_worker_if_requested()` stays the first call in `main`. The thumbnail worker is this same executable.
- The bus interface is declared in `otto-dbus` (`components/otto-dbus/src/stash.rs`). Change it there and in `src/dbus.rs` together.

## See also
- [Stash spec](../../specs/stash.md), [Side canvas](../../docs/developer/side-canvas.md).
- [otto-dbus](../../docs/developer/otto-dbus.md), [otto-peek](../otto-peek/AGENTS.md).
