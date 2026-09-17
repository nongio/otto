# otto-agentsd

A Rust server for the [Agent Host Protocol](https://microsoft.github.io/agent-host-protocol/)
(AHP): a JSON-RPC protocol that gives many clients a synchronized view of shared AI agent
sessions.

**Status:** proof of concept ([plan 0010](docs/plans/0010-poc.md)). otto-agentsd runs
agents over ACP, serves sessions over AHP, and lists them in the terminal.

## Quick start

```sh
cargo run -p otto-agentsd -- serve                      # listens on ws://127.0.0.1:4800
cargo run -p otto-agentsd --example ask -- "summarise the README"
cargo run -p otto-agentsd -- sessions                   # ID, status, agent, age, folder, title
cargo run -p otto-agentsd -- show                       # transcript of the newest session
cargo run -p otto-agentsd -- show 1a2b --follow         # a session by id prefix, streaming until done
```

- **Agents.** They come from `[[agents]]` in `~/.config/otto/agents.toml` (after
  `/etc/otto/agents.toml`), or from `serve --config <file>`. With none configured,
  `claude` runs through `npx @agentclientprotocol/claude-agent-acp`, using your existing
  Claude login.

  ```toml
  [[agents]]
  id = "claude"
  name = "Claude"
  command = "claude-agent-acp"
  model = "haiku"        # optional: each session switches to it; Claude takes haiku, sonnet, opus
  permissions = "ask"    # ask in an otto-islands dialog (denied if it can't be shown); or "deny", "allow"
  ```

- **Echo agent.** `serve --echo` runs a built-in agent that repeats every prompt, for
  trying clients without a real agent.
- **Logging.** Set verbosity with `RUST_LOG`, a `tracing` filter, e.g.
  `RUST_LOG=debug`.

## Running the service

Otto's packages install `otto-agentsd` with a systemd user unit, off by default.
Start it, and at every login from then on:

```sh
systemctl --user enable --now otto-agentsd
journalctl --user -u otto-agentsd -f    # its log
```

What it needs:

- **An agent to run.** Each entry in `agents.toml` names a command that speaks ACP.
  The default, Claude, runs through `npx @agentclientprotocol/claude-agent-acp`, so
  it needs Node.js and a Claude login.
- **Those commands on its PATH.** The unit does not get a login shell's PATH, so
  anything under nvm or `~/.local/bin` is not found. Give the full path in
  `agents.toml`, or add the directories with `systemctl --user edit otto-agentsd`:

  ```ini
  [Service]
  Environment=PATH=%h/.config/nvm/versions/node/v24.7.0/bin:%h/.local/bin:/usr/bin
  ```

- **otto-islands**, to show permission requests. Without it, a request that needs
  an answer is denied.

## Repository layout

| Path | Contents |
|---|---|
| `src/` | Server library and binary |
| `spec/` | Vendored AHP spec, pinned to an upstream release ([details](spec/README.md)) |
| `scripts/` | Spec sync and CI scripts |
| `docs/plans/` | Milestone plans |
| `docs/testing.md` | Test layers and where they run |

## Scripts

| Command | Purpose |
|---|---|
| `scripts/ci.sh` | Run everything CI runs: fmt, clippy, tests and the spec drift check |
| `scripts/sync-spec.sh [--ref TAG]` | Vendor the upstream spec into `spec/upstream/` |
| `scripts/sync-spec.sh --check` | Verify `spec/upstream/` matches the pinned ref |
| `scripts/spec-status.sh` | Check whether a newer spec release exists |
