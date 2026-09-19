# otto-agents

A Rust server for the [Agent Host Protocol](https://microsoft.github.io/agent-host-protocol/)
(AHP): a JSON-RPC protocol that gives many clients a synchronized view of shared AI agent
sessions.

**Status:** proof of concept ([plan 0010](docs/plans/0010-poc.md)). otto-agents runs
agents over ACP, serves sessions over AHP, and lists them in the terminal.

## Quick start

```sh
cargo run -p otto-agents -- serve                      # on $XDG_RUNTIME_DIR/otto-agents/agents.sock
cargo run -p otto-agents --example ask -- "summarise the README"
cargo run -p otto-agents -- sessions                   # ID, status, agent, age, folder, title
cargo run -p otto-agents -- show                       # transcript of the newest session
cargo run -p otto-agents -- show 1a2b --follow         # a session by id prefix, streaming until done
cargo run -p otto-agents -- new otto                   # start a session with an agent and enter it in this terminal
cargo run -p otto-agents -- enter 1a2b                 # take a session up in this terminal, in the agent's interface
cargo run -p otto-agents -- plugins install            # link Otto's skills into ~/.agents/skills, render its agent per harness
cargo run -p otto-agents -- plugins status             # what was found, and where each harness's copy stands
```

- **Agents.** They come from `[[agents]]` in `~/.config/otto/agents.toml` (after
  `/etc/otto/agents.toml`), or from `serve --config <file>`. With none configured,
  `claude` runs through `npx -y @agentclientprotocol/claude-agent-acp@0.79.0`, using
  your existing Claude login.

  ```toml
  [[agents]]
  id = "claude"
  name = "Claude"
  command = "claude-agent-acp"
  model = "haiku"        # optional: each session switches to it; Claude takes haiku, sonnet, opus
  mode = "acceptEdits"   # optional: the agent's own mode id a new session starts in; unknown ids are ignored
  permissions = "ask"    # ask in an otto-islands dialog (denied if it can't be shown); or "deny", "allow". Unset is "deny"
  folder = "~/dev"       # optional: where this agent's sessions start; it is the reach the agent gets
  skills = "claude"      # optional: Claude loads Otto's skills as a plugin
  agent = "otto"         # optional: run as the plugin's `otto` agent (needs skills = "claude")
  config = { collaboration_mode = "plan" }  # optional: the agent's own session options
  ```

- **Skills and the Otto agent.** Otto's skills (`/usr/share/otto/plugins/otto`)
  are off for an agent unless `skills` says otherwise. Claude takes them as a
  plugin with `skills = "claude"` and runs as the plugin's agent with
  `agent = "otto"`; every other agent finds the skills once
  `otto-agents plugins install` has linked them into `~/.agents/skills`, and
  the same command renders the agent for OpenCode, Hermes, Codex and pi, each
  in its own dialect and place, from the one file in the plugin. `agents.toml`
  then points each launch at it (`args` or `env`; `{file:<path>}` in `args`
  is read when the agent starts). See
  [docs/developer/agents.md](../../docs/developer/agents.md#the-otto-agent-on-every-harness).

- **Echo agent.** `serve --echo` runs a built-in agent that repeats every prompt, for
  trying clients without a real agent.
- **Where it listens.** A Unix socket, `$XDG_RUNTIME_DIR/otto-agents/agents.sock`,
  mode 0600. `serve --listen` (`OTTO_AGENTS_LISTEN`) takes another path or `host:port`;
  TCP is unauthenticated and logged as such, for development. Clients take
  `--url` (`OTTO_AGENTS_URL`): `unix:///path` or `ws://host:port`.
- **Logging.** Set verbosity with `RUST_LOG`, a `tracing` filter, e.g.
  `RUST_LOG=debug`.

## Running the service

Otto's packages install `otto-agents` with a systemd user unit, off by default.
Start it, and at every login from then on:

```sh
systemctl --user enable --now otto-agents
journalctl --user -u otto-agents -f    # its log
```

What it needs:

- **An agent to run.** Each entry in `agents.toml` names a command that speaks ACP.
  The default, Claude, runs through `npx -y @agentclientprotocol/claude-agent-acp@0.79.0`,
  so it needs Node.js and a Claude login.
- **Those commands on its PATH.** The unit does not get a login shell's PATH, so
  anything under nvm or `~/.local/bin` is not found. Give the full path in
  `agents.toml`, or add the directories with `systemctl --user edit otto-agents`:

  ```ini
  [Service]
  Environment=PATH=%h/.config/nvm/versions/node/v24.7.0/bin:%h/.local/bin:/usr/bin
  ```

- **otto-islands**, to show permission requests and agents' questions when no
  client is watching. Without it, a permission request is denied and a question
  waits in the chat.

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
