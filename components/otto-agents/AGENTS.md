# otto-agents

Otto's agent service. It runs ACP agents as child processes and serves their sessions over the Agent Host Protocol (AHP) on a Unix socket, so many clients share one synchronised view of each session. The launcher, the side canvas and Studio (`otto-preview`) are AHP clients of it. The same binary also has terminal views (`sessions`, `show`, `new`, `enter`), the `acp` facade that presents the service as one ACP agent, and the chat `bridge`.

How it works: [Agents](../../docs/developer/agents.md).

- **Layer:** app. Used by nothing (an app); clients reach it over AHP through `otto-agents-client`.

## Dependencies
- **Internal:** `otto-agents-client` (transport, socket path and URIs shared with every client), `otto-dbus` (the `dialog` proxy for permission prompts and agents' questions), `otto-foundations` (XDG directories, logging setup).
- **External:** `ahp`, `ahp-types` and `ahp-ws` (wire types, reference client) and `agent-client-protocol` (talking to agents) define the design; `tokio` runs it. `zbus` is outbound only, to `otto-islands`, which shows dialogs; without it a permission request is denied. At run time it needs the agent commands named in `agents.toml`; the default Claude agent runs through `npx`, so it needs Node.js and a Claude login.

## Develop
- `cargo run -p otto-agents -- serve` listens on `$XDG_RUNTIME_DIR/otto-agents/agents.sock`. `serve --echo` runs a built-in agent that repeats every prompt, for trying clients with no credentials. `--listen` (`OTTO_AGENTS_LISTEN`) and, on clients, `--url` (`OTTO_AGENTS_URL`) pick another address.
- Packages install the `otto-agents` systemd user unit (`otto-agents.service`, off by default) and `otto-agents-bridge.service` for the chat bridge. Stop the unit (`systemctl --user stop otto-agents`) before serving a dev build on the same socket; read its log with `journalctl --user -u otto-agents -f`.
- `cargo run -p otto-agents --example ask -- "..."` sends one prompt to a running service. `RUST_LOG` filters logging.
- Tests: `cargo test -p otto-agents`. The layers (unit, sessions, ACP backend, end-to-end, spec pin, conformance) are in [docs/testing.md](docs/testing.md).
- `scripts/ci.sh` runs fmt, clippy, the tests and the spec drift check; `SKIP_SPEC_CHECK=1` skips the drift check, which needs network access.

## Where things are
- The wire side: `src/server.rs` (WebSocket, one JSON-RPC message per text frame) and `src/rpc.rs`.
- `src/host.rs` is the authoritative state (sessions, chats, subscriptions); `src/store.rs` keeps sessions on disk.
- Agent backends are `src/agent.rs` and `src/acp.rs`; `[[agents]]` config is `src/config.rs` (`/etc/otto/agents.toml`, then `~/.config/otto/agents.toml`).
- Prompts to the person (`src/dialog.rs`, `src/elicitation.rs`), the terminal views and bridge (`src/cli.rs`), the `acp` facade (`src/facade.rs`), and skills per harness for `plugins install` (`src/skills.rs`, `src/vendors.rs`).

## Rules
- `spec/upstream/` is a vendored copy of the AHP spec and is read-only. Change it only with `scripts/sync-spec.sh`. The normative rules are in `spec/upstream/specification/`; the type source of truth is `spec/upstream/types/*.ts`.
- Use `ahp-types` for wire types. The `ahp`, `ahp-types` and `ahp-ws` versions must equal `AHP_PROTOCOL_VERSION` in `spec/upstream.env`; `tests/spec_pin.rs` enforces it.
- otto-agents takes the service role described in Agents. Otto's surfaces are AHP clients (plans `0003`, `0007` and `0011` in `docs/plans/`).
- Read the relevant plan in `docs/plans/` before starting a milestone, and update its status when work lands.
- Tests pass in any locale. A test that asserts English calls `i18n::pin_source_locale()` first; check with `LC_ALL=de_DE.UTF-8 cargo test -p otto-agents`.
- Run `scripts/ci.sh` before calling work done.

## See also
- [README](README.md): commands, `agents.toml`, running the service, the chat bridge.
- [Plans](docs/plans/README.md), [spec coverage](docs/spec-coverage.md), [vendored spec](spec/README.md), [cc-connect bridge](docs/cc-connect.md).
- [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-agents-kit](../otto-agents-kit/AGENTS.md), user guide [Agents](../../docs/user/agents.md).
