# 0005 — Trust, permissions and skills for the agent stack

Status: in progress (September 2026). A1–A6 and B7–B10 landed.

A review of otto-agents and Ask against the hosts that embed the same
agents (Claude Code's editor extension and Agent SDK, codex-acp, OpenCode,
Copilot agent mode, Zed) found the permission model thin but honest, the
transport under it unauthenticated, and the skills delivery a home-grown
mechanism no other host uses. This plan is the work that follows from it,
in the order it ships.

## A. Transport and process trust

1. [x] **Unix socket instead of loopback TCP.** The service listens on
   `$XDG_RUNTIME_DIR/otto-agents/agents.sock` (mode 0600, directory 0700).
   `OTTO_AGENTS_LISTEN` / `--listen` accept a path or `host:port`; TCP stays
   for tests and development and logs a warning that it is unauthenticated.
   `OTTO_AGENTS_URL` / `--url` accept `unix:///path` (the default) or `ws://`.
   Every client (launcher, the `ask` example, the CLI) follows.
2. [x] **State on disk is private.** The sessions directory is created 0700 and
   records are written 0600.
3. [x] **Agents start from a known version.** The built-in Claude fallback pins
   the adapter version instead of `@latest`; the docs example does too.
4. [x] **`OTTO_AGENTS_PLUGINS` is a development knob.** Documented as such and
   logged at warn level when set, because it turns any directory into a
   trusted plugin.
5. [x] **The unit is hardened.** `NoNewPrivileges`, `ProtectSystem=strict` with
   the state and runtime dirs writable, `PrivateTmp`, `RestrictSUIDSGID`.
   Agents need the home directory, so `ProtectHome` stays off.
6. [x] **A watched question still reaches the person.** A permission request
   that a subscribed client has not answered within a grace period escalates
   to the island dialog anyway, so a silent subscriber cannot hold it.

## B. Permissions

7. [x] **One place picks the option.** agents chooses the default option by
   ACP `kind` (narrowest of the right intent, rejects first when the agent
   marks `defaultToNo`); the launcher and the dialog take that default
   rather than re-deriving it.
8. [x] **The prompt shows the action.** The request's `locations`, `rawInput`
   and diff content reach clients as the tool call's `tool_input` and
   `edits`; the `_meta.permission` presentation hints (title, description,
   defaultToNo) that the Claude and Codex adapters send are honoured.
9. [x] **Cancelling a turn cancels its requests.** Pending permission requests
   answer with the ACP `cancelled` outcome on `session/cancel`, not with a
   refusal.
10. [x] **Dialog words come from the catalogue.** The tool-kind phrases and
    "Open in Ask" move to Fluent; the two parallel matches on `ToolKind`
    become one table.
11. [x] **Agent modes are first class.** `session/new` returns the agent's own
    modes (Claude: Manual, Accept edits, Plan, Auto; Codex: read-only,
    agent, full access). agents publishes them in session `_meta`, applies
    a per-agent `mode = "<id>"` from `agents.toml` after the session opens,
    and accepts a `setMode` request; Ask shows the current mode and lets the
    person switch. `permissions = allow|deny` stays accepted for now but the
    docs steer to modes, since they carry the agent's own sandboxing.

## C. System prompt and skills

12. [x] **Skills default to off**, and the built-in fallback agent agrees with
    the serde default: `skills = "claude"` loads them as a plugin when the
    agent is Claude, and `skills` unset or `false` leaves the session alone
    (the agent reads `~/.agents/skills` instead, see 13).
13. [x] **Skills are installed where agents already look.** `otto-agents
    plugins install` (`skills install` is still an alias) links the system
    plugin's skills into `~/.agents/skills/` (the cross-vendor path the Agent
    Skills spec, Copilot and Claude read) and prints what it did;
    `plugins status` shows it.
14. [x] **Claude gets a real system prompt.** With plugin delivery, the session
    opens with `_meta.systemPrompt = {append: <otto preamble>}` so the
    `claude_code` preset's own safety text stays.
15. [x] **The briefing is removed.** There used to be a third route, a skill
    list sent as a content block ahead of a new session's first prompt
    (`skills = true` / `"briefing"`). It is gone: those settings are now a
    config error that points at `skills = "claude"` and `otto-agents plugins
    install`. Sessions opened while it existed still replay it in their first
    user message, so `session/load` drops a chunk marked
    `_meta.otto.briefing = true` or one opening with the briefing's fixed
    first sentence, keeping any text the agent joined after the skill list.
16. [x] **Docs say what the host does not do.** No `fs/*` or `terminal/*`
    capability is advertised; every file and shell operation runs inside the
    agent under its own permission engine, and the dialog is advisory.

## D. Loose ends

17. [x] Plan 0004 drops the `icon` and `guide` keys that the parser rejects.
18. [x] The deb/rpm asset lists ship `resources/plugins`.
19. [x] `docs/developer/agents.md` describes the `PresentQuestions` path.
20. [x] New en-GB strings are translated into the other locales.
21. [x] **Plugin agents, on every harness.** The `otto` plugin carries
    `agents/otto.md`, a Claude Code plugin agent — frontmatter and a system
    prompt — that answers desktop questions from the `otto` skill.
    `skills.rs` reads `agents/*.md` (`name`, `description`, `tools`, `model`,
    `skills`, and the body) next to the skills; a plugin with agents and no
    skills still counts. Each agent publishes as an `AgentCustomization`
    child of its plugin, the Claude preamble says the agent is there, and the
    deb/rpm asset lists ship `agents/*.md`. Claude runs as it through
    `agent = "otto"` (`--agent otto:otto`). For the rest, the decision was
    an installer in the service, in Rust, from that one source: `vendors.rs`
    is a table of harnesses — where the file goes, which frontmatter it
    takes, how a launch selects it — and `otto-agents plugins install`
    renders the agent per harness with the body unchanged (OpenCode: an
    agent file plus `OPENCODE_CONFIG_CONTENT`; Hermes: the profile's
    `SOUL.md`; Codex: a plain file passed as `developer_instructions` through
    `{file:…}` in `args`; pi: a plain file plus a wrapper `PI_ACP_PI_COMMAND`
    points at). Files carry a marker, are rewritten only when the rendering
    changed, and a file that is not ours is left alone; `plugins status`
    shows each. Selection lives in `agents.toml`, never in the harness's own
    config, so a harness in a terminal is unchanged. The table with versions
    and what was verified is in `docs/developer/agents.md`.
