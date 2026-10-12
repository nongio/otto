# otto-lock

Otto's screen locker. The compositor starts it when the session locks (the `LockSession` action, or before the machine suspends). It takes the lock with `ext-session-lock-v1`, draws the shared password panel on every output, authenticates the session's own user against PAM, and only then gives the session back.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** [otto-auth-ui](otto-auth-ui.md) with its `pam` feature, for the panel, the PAM conversation and the power buttons. `otto-kit` for drawing and i18n.
- **External:**
  - PAM (`libpam`), through the `otto-lock` service. `otto-lock.pam` in the crate is the stack to install as `/etc/pam.d/otto-lock`. Without it the service falls back to `system-auth`, then `login`.
  - logind on the system bus for suspend, restart and shut down, through otto-auth-ui.
  - `ext-session-lock-v1` from the compositor.

## Develop

- The compositor offers `ext_session_lock_manager_v1` only to the locker it starts itself, on a socketpair. Running `cargo run -p otto-lock` by hand does not lock anything.
- Test an uninstalled build: `cargo build -p otto-lock`, then `OTTO_LOCKER_COMMAND=target/debug/otto-lock cargo run -- --winit` and lock with Ctrl+Alt+Escape. The variable overrides `[lock] locker_command` and is split on whitespace.
- The compositor starts the locker again if it dies, and the new one takes over the standing lock. A crashed locker leaves a blank screen, never the desktop.
- `examples/lock_cycle.rs` locks, waits `HOLD` seconds (default 3) and unlocks with no PAM involved, to exercise the protocol lifecycle. It needs to be the compositor's locker too: build it with `cargo build -p otto-lock --example lock_cycle` and point `OTTO_LOCKER_COMMAND` at `target/debug/examples/lock_cycle`.
- `OTTO_LOCK_PAM_SERVICE` names a different PAM service.
- Logs: `RUST_LOG=otto_lock=debug`. Tests: `cargo test -p otto-lock`.

## Code map

- `src/main.rs` is the whole client: the lock lifecycle, one lock surface per output, input, the PAM attempt and the power buttons.
- `otto-lock.pam` is the PAM stack shipped with the packages.
- `examples/lock_cycle.rs` is the protocol probe described above.

## Rules

- Never unlock without a successful PAM answer, and never draw anything that reveals the session. The lock lives in the compositor; the locker only proves who is at the keyboard.
- PAM runs off the main thread (otto-auth-ui's `pam` module) because a module such as `pam_fprintd` can hold the conversation for as long as it likes. Keep the main thread drawing.
- The panel drawing is shared with otto-greeter and otto-authorize. Change it in otto-auth-ui, not here.
- Do not try a lock on the session you are working in. A lock that fails to unlock can only be left by switching VT.

## See also

- [Lock screen spec](../../specs/lock-screen.md) and the user guide [Lock screen](../user/lock-screen.md).
- [otto-auth-ui](otto-auth-ui.md) for the panel.
