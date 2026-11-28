# otto-agents

Otto's agent service. It runs agent sessions and serves them over the Agent Host Protocol (AHP) on a Unix socket, so many clients share one synchronised view of each session. The launcher, the side canvas and Studio (`otto-preview`) are AHP clients of it. The same binary also has terminal views (`sessions`, `show`, `new`, `enter`), and the `acp` facade that presents the service as one ACP agent.

Two backends run the sessions. With herdr installed (`hc` on `PATH`), the herdr backend runs each one as the agent's own program (Claude Code, pi or opencode) in a herdr-core terminal and mirrors it; herdr is the source of truth for which sessions exist. This ships in nightly builds, not a release yet (plan `0016`). Otherwise the ACP backend runs each agent as a child process over ACP. `backend` in `agents.toml` (`"herdr"` or `"acp"`) or `serve --herdr` / `--acp` picks one.

How it works: [Agents](../../docs/developer/agents.md).

- **Layer:** app. Used by nothing (an app); clients reach it over AHP through `otto-agents-client`.

## Dependencies
- **Internal:** `otto-agents-client` (transport, socket path and URIs shared with every client), `otto-dbus` (the `dialog` proxy for permission prompts and agents' questions), `otto-foundations` (XDG directories, logging setup).
- **External:** `ahp`, `ahp-types` and `ahp-ws` (wire types, reference client) and `agent-client-protocol` (talking to agents) define the design; `tokio` runs it. `zbus` is outbound only, to `otto-islands`, which shows dialogs; without it a permission request is denied. At run time it needs, under herdr, herdr-core's `hc` and `herdr-app` on its PATH (plus `opencode` and `sqlite3` for opencode agents), and the agents' own programs on the login-shell PATH herdr starts them with; under ACP, the agent commands named in `agents.toml`, where the default Claude agent runs through `npx`, so it needs Node.js. Either way, the agents logged in.

## Develop
- `cargo run -p otto-agents -- serve` listens on `$XDG_RUNTIME_DIR/otto-agents/agents.sock`. `serve --herdr` or `--acp` overrides the backend. `serve --echo` runs a built-in agent that repeats every prompt, for trying clients with no credentials. `--listen` (`OTTO_AGENTS_LISTEN`) and, on clients, `--url` (`OTTO_AGENTS_URL`) pick another address.
- Packages install the `otto-agents` systemd user unit (`otto-agents.service`, off by default). Stop the unit (`systemctl --user stop otto-agents`) before serving a dev build on the same socket; read its log with `journalctl --user -u otto-agents -f`.
- `cargo run -p otto-agents --example ask -- "..."` sends one prompt to a running service. `RUST_LOG` filters logging.
- Tests: `cargo test -p otto-agents`. The layers (unit, sessions, ACP backend, end-to-end, spec pin, conformance) are in [docs/testing.md](docs/testing.md).
- `scripts/ci.sh` runs fmt, clippy, the tests and the spec drift check; `SKIP_SPEC_CHECK=1` skips the drift check, which needs network access.

## Where things are
- The wire side: `src/server.rs` (WebSocket, one JSON-RPC message per text frame) and `src/rpc.rs`.
- `src/host.rs` is the authoritative state (sessions, chats, subscriptions); `src/store.rs` keeps sessions on disk.
- The `Backend` trait and the echo backend are `src/agent.rs`; the backends are `src/herdr.rs` (herdr) and `src/acp.rs` (ACP). `src/herdr_tools.rs` holds `herdr-ask` and `herdr-permission`, which a herdr session's Claude Code runs to reach Otto. `backend` and `[[agents]]` config is `src/config.rs` (`/etc/otto/agents.toml`, then `~/.config/otto/agents.toml`).
- Prompts to the person (`src/dialog.rs`, `src/elicitation.rs`), the terminal views (`src/cli.rs`), the `acp` facade (`src/facade.rs`), and skills per harness for `plugins install` (`src/skills.rs`, `src/vendors.rs`).

## Rules
- `spec/upstream/` is a vendored copy of the AHP spec and is read-only. Change it only with `scripts/sync-spec.sh`. The normative rules are in `spec/upstream/specification/`; the type source of truth is `spec/upstream/types/*.ts`.
- Use `ahp-types` for wire types. The `ahp`, `ahp-types` and `ahp-ws` versions must equal `AHP_PROTOCOL_VERSION` in `spec/upstream.env`; `tests/spec_pin.rs` enforces it.
- otto-agents takes the service role described in Agents. Otto's surfaces are AHP clients (plans `0003`, `0007`, `0011` and `0016` in `docs/plans/`).
- Read the relevant plan in `docs/plans/` before starting a milestone, and update its status when work lands.
- Tests pass in any locale. A test that asserts English calls `i18n::pin_source_locale()` first; check with `LC_ALL=de_DE.UTF-8 cargo test -p otto-agents`.
- Run `scripts/ci.sh` before calling work done.

## See also
- [README](README.md): commands, `agents.toml`, running the service.
- [Plans](docs/plans/README.md), [spec coverage](docs/spec-coverage.md), [vendored spec](spec/README.md).
- [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-agents-kit](../otto-agents-kit/AGENTS.md), user guide [Agents](../../docs/user/agents.md).
