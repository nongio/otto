# otto-greeter

Otto's login screen. Otto started with `--login` is the greeter session greetd runs, and it launches `otto-greeter` in place of the usual autostart entries. The greeter is a Wayland client on a fullscreen `wlr-layer-shell` overlay with exclusive keyboard input. It drives the login conversation with greetd over `$GREETD_SOCK` and never links PAM.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** [otto-auth-ui](otto-auth-ui.md) without its `pam` feature, for the panel and the power buttons. `otto-kit` for drawing and i18n.
- **External:**
  - greetd, through its IPC (`greetd-ipc(7)`): a native-endian `u32` length prefix and a JSON payload, over the unix socket in `$GREETD_SOCK`.
  - Session `.desktop` files in `/usr/share/wayland-sessions` and `/usr/local/share/wayland-sessions`, for the session picker.
  - logind for the power buttons, through otto-auth-ui.
  - `wlr-layer-shell` from the compositor.
  - `zeroize` wipes typed answers and the frames they were sent in.

## Develop

- Without `GREETD_SOCK` the greeter uses a mock backend that accepts the password `otto`. It takes over the screen and the keyboard, so run it in a nested compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-greeter`.
- The whole login mode: `OTTO_GREETER_COMMAND=target/debug/otto-greeter cargo run -- --winit --login`. The variable overrides `[login] greeter_command` and is split on whitespace.
- The real wire protocol without root: `cargo run -p otto-greeter --example fake_greetd -- /tmp/fake-greetd.sock`, then `GREETD_SOCK=/tmp/fake-greetd.sock cargo run -p otto-greeter`. `FAKE_GREETD_PASSWORD` sets the password (default `otto`); `FAKE_GREETD_SCENARIO` is `simple`, `fingerprint` (the default), `two-factor` or `locked`; `FAKE_GREETD_READER_WAIT` sets how long the fingerprint scenario holds (default 15 s).
- `OTTO_GREETER_SESSION` replaces session discovery with one argv. `OTTO_GREETER_DEFAULT_SESSION` picks the preselected session by name (exact, then a case-insensitive substring); set it in greetd's config.
- For real, greetd starts `otto --login`; its logs are in `journalctl -u greetd`.
- Logs: `RUST_LOG=otto_greeter=debug`. Tests: `cargo test -p otto-greeter`.

## Code map

- `src/main.rs` is the client: the layer surface, input, the session picker and the conversation state.
- `src/greetd.rs` is the greetd IPC client and the mock backend.
- `src/session.rs` discovers sessions and picks the default one.
- `examples/fake_greetd.rs` is the stand-in greetd daemon.

## Rules

- The greeter never links libpam. Do not turn on otto-auth-ui's `pam` feature here; authentication belongs to greetd.
- Answers to secret prompts stay in `Zeroizing` buffers, including the serialised frame. Keep new secret paths the same.
- Which session is offered first is a deployment decision (`OTTO_GREETER_DEFAULT_SESSION`), not a hardcoded preference.

## See also

- [Login mode spec](../../specs/login-mode.md) and the user guide [Login greeter](../user/login-greeter.md).
- [otto-auth-ui](otto-auth-ui.md) for the panel.
