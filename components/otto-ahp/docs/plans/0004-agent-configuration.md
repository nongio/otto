# 0004: Configuration

**Status:** Draft

## Goal

Configuration, not code, decides which agents the service can run. No credentials go
in it ([0005](0005-authentication.md)).

## Where it lives

Agents have a file of their own, the way the files app has `files.toml`. It is TOML,
with later files overriding earlier ones.

1. `/etc/otto/agents.toml`
2. `$XDG_CONFIG_HOME/otto/agents.toml` (`~/.config/otto/agents.toml`)

It holds `[[agents]]`, shaped as `../otto/specs/agents.md` shows, plus an `[ahp]`
section. Unknown keys are errors, since nothing else shares the file. `--config <file>`
points at another file with the same shape. The POC reads `[[agents]]` from these
paths; `[ahp]` is not read yet. It also reads a top-level `terminal`, the command a
session is entered in with the agent's `enter` appended, such as
`terminal = ["ghostty", "--working-directory={cwd}", "-e"]` (`{cwd}` is the session's
folder, `{session}` the agent's id for it). This answers 0003's open question for now;
it may move under `[ahp]`. A top-level `default_agent` is read too, the POC of
`[ahp].default_agent`: it must name a configured agent, and is published by listing that
agent first in `RootState.agents` rather than through `RootState.config`. Without it,
the first listed agent is the default.

```toml
[ahp]
listen = "127.0.0.1:4800"      # loopback only unless TLS is configured (0005)
socket = true                  # also serve $XDG_RUNTIME_DIR/otto-ahp/ahp.sock
default_agent = "claude"
default_folder = "~/"

[[agents]]
id = "claude"                  # AgentInfo.provider
name = "Claude"                # AgentInfo.displayName
icon = "claude"
command = "claude-agent-acp"   # ACP over stdio (0003)
args = []
env = {}                       # non-secret only
model = "haiku"                # optional; set on each new session via ACP session/set_config_option
enter = ["claude", "--resume", "{session}"]   # optional; enables x-otto/enterSession
guide = true                   # attach the Otto guide (0008)
```

## How it maps to AHP

- **Agent catalogue.** Each agent becomes an entry in `RootState.agents`. `id` becomes
  `provider` and `name` becomes `displayName`; the icon and whether the agent can be
  entered go in `_meta`. Models are asked from the agent at startup.
- **Hidden fields.** `command`, `args`, `env` and `enter` never reach clients.
- **Unavailable agents.** An agent whose `command` isn't found is listed with its
  unavailability noted. `createSession` for it fails with `ProviderNotFound` (`-32002`).
- **Settings clients may change.** `default_agent` and `default_folder` are published
  as `RootState.config` (schema plus values). Accepted `root/configChanged` values are
  written back with comments preserved, as Otto's config writer already does.
- **Per-session config.** `resolveSessionConfig` returns a schema containing the agent,
  enumerated from config, and a folder.
- **Hot reload.** Watch the files and broadcast `root/agentsChanged` when agents change.
  Running sessions keep the agent definition they started with. `listen` changes need a
  restart, which is logged.
- **Validation.** Fail fast, naming the file, the line and the key.

## Testing

- **Parsing:** config fixtures, including load-order overrides
- **Catalogue:** golden `RootState.agents` output, including unavailable agents
- **Hot reload:** one config change produces exactly one `root/agentsChanged`
- **Write-back:** `root/configChanged` round-trips and preserves comments

## Open questions

- Should otto-settings show `[ahp]` and `[[agents]]` through `org.otto.Settings`? That
  would require Otto to register the keys ([0007](0007-otto-desktop-integration.md)).
- Should per-project overrides, such as a default agent per folder, be supported?
