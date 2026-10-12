# otto-authorize

The session's polkit authentication agent. When a program asks polkit for something that needs a password (`pkexec`, a system service's action, a protected Otto setting), polkitd asks this agent, and it shows Otto's password panel as a dialog on a layer-shell overlay. The password goes to polkit's own helper, never to Otto. The compositor starts it and keeps it running.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-auth-ui` with its `pam` feature, for the panel and the conversation types (`Attempt::with_conversation`). `otto-kit` for drawing and i18n.
- **External:**
  - polkit's agent library `libpolkit-agent-1`, through `polkit-agent-rs`. It registers the agent for the logind session (`XDG_SESSION_ID`), serves polkitd's requests and runs polkit's helper, which runs PAM and reports to polkitd itself. The library runs its own GLib main loop on a thread of its own.
  - `wlr-layer-shell` from the compositor.

## Develop
- For real the compositor starts `otto-authorize --polkit-agent` once per session, on a socketpair (`WAYLAND_SOCKET`) marked as its own component, so the panel holds the keyboard. It restarts it with a back-off if it dies; a clean exit is not retried. `polkit_agent = false` in the compositor config turns it off. See `src/polkit_agent.rs` in the compositor.
- Started by hand without `--polkit-agent` it prints a hint and exits with status 2.
- A compositor built with the `dev` feature runs the `otto-authorize` beside its own binary when there is one: `cargo build -p otto-authorize`, then `cargo run --features dev -- <backend>`. Otherwise it is looked up in `$PATH`.
- polkit keeps one agent per session, and the agent exits cleanly when the session already has one. A nested compositor inside a running Otto session runs in that session, so its agent is not the one polkit asks. Test it as the session's own agent and trigger it with `pkexec true`.
- Logs: `RUST_LOG=otto_authorize=debug`. Tests: `cargo test -p otto-authorize`.

## Where things are
- `src/main.rs` checks the flag and starts the agent.
- `src/polkit/` is the agent: the request queue, throttling and the dialog loop in `mod.rs`, polkit's listener subclass in `listener.rs`, the helper's conversation relay in `session.rs`.
- `src/dialog.rs` is the panel on its layer-shell overlay and the conversation behind it.

## Rules
- The agent never answers for anyone. Only polkit's helper can say yes; this process can only cancel. Do not add a path that reports success itself.
- One request at a time: another arriving while the dialog is up is cancelled.
- Escape, Cancel, a minute without an answer and three wrong passwords each end a request. A program whose requests keep being dismissed is held off for a while (`Throttle`), so nothing can put the panel up again and again.
- The panel drawing is shared with otto-lock and otto-greeter. Change it in `otto-auth-ui`.

## See also
- [Permissions](../../docs/developer/permissions.md) for how Otto uses polkit.
- [otto-auth-ui](../otto-auth-ui/AGENTS.md) for the panel.
