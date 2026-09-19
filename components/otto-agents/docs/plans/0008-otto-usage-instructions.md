# 0008: Otto usage instructions for agents

**Status:** In progress

## Goal

An agent started on an Otto desktop knows it is on Otto. It knows what the apps are,
how to script the desktop, where configuration lives, and how to take screenshots and
read logs. This is Otto agents milestone 5 ("An Otto Guide for Agents"); otto-agents owns
the sessions, so otto-agents delivers the guide. AHP clients can see exactly what a session
was given.

## Content

- **Sources.** Write from `../otto/docs/user/` so the guide follows the docs rather than
  drifting from them:
  - `scripting.md` (`otto-msg`, `org.otto.Shell1`)
  - `configuration.md`
  - `keyboard-shortcuts.md`
  - `launcher.md`
  - `dynamic-island.md`
  - `files-custom-commands.md` (milestone 6)
  - `screen-sharing.md`
  - `troubleshooting.md`
- **D-Bus contracts.** Include what an agent may call:
  `docs/developer/shell-dbus-api.md` and `settings-dbus-api.md`.
- **Keep "using Otto" separate from "developing Otto".** The existing user-level skills
  (`~/.claude/skills/otto-remote-control`, `otto-scene-debugger`, `otto-layer-designer`)
  are proven recipes, but they target development: nested winit sessions, the dev-only
  scene debugger, log offsets. Reuse them only where they apply to a running desktop.

## Delivery

| Form | Pros | Cons |
|---|---|---|
| Instructions text | Every agent can take it | Always in context; grows with the docs |
| **Skill directory** (Open Plugins shape) | Loaded on demand; maps 1:1 to AHP `SkillCustomization` | Agents without skill support need a fallback |
| MCP tool server over `Shell1`, `Settings`, screencopy and virtual input | Real actions rather than prose | Needs the trust model that Otto's spec defers ("agents acting on Otto apps") |

**Recommendation:**
- Generate a skill directory from `docs/user/` as part of Otto's build, installed for
  example to `/usr/share/otto/agent-guide/`.
- Add an MCP server once the trust model exists. ACP's `session/new` already carries
  `mcpServers`, so wiring it in later is straightforward.

**What shipped so far.** Otto installs a plugin directory to
`/usr/share/otto/plugins/otto/` — `resources/plugins/` in the Otto tree, packaged by
the three PKGBUILDs — holding hand-written skills rather than ones generated from
`docs/user/`: `configure-otto` and `extend-otto-files`. otto-agents reads it in
`src/skills.rs`, publishes each plugin as a read-only `PluginCustomization` with its
skills as children, and delivers it by one of two routes: Claude loads the plugin
directory through claude-agent-acp's session `_meta` (`skills = "claude"`), and every
other agent finds the skills in `~/.agents/skills` once `otto-agents plugins install`
has linked them there. The briefing route that once sat between the two is gone. The
generated guide, the `guide = true` switch from [0004](0004-agent-configuration.md)
(the flag is `skills`, defaulting to off) and the session toggle are still open.

## otto-agents' part

- **Attach per agent.** For agents with `guide = true` ([0004](0004-agent-configuration.md)),
  otto-agents attaches the guide to every new session. How it reaches the agent depends on
  what ACP and the agent support, to be verified:
  - a skill directory the agent loads
  - instructions prepended to the first prompt
  - later, an MCP server in `session/new`
- **Show what was attached.** List the guide in `AgentInfo.customizations` and
  `SessionState.customizations` as a read-only `DirectoryCustomization` with
  `contents: skill` and `writable: false`, children listed.
- **Toggling.** `session/customizationToggled` turns the guide off for a session, but
  only before the first turn; after that, the toggle is rejected.

## Testing

- **Otto side:** a docs-drift check fails when `docs/user/` changes without
  regenerating the guide
- **otto-agents side:** the guide appears in session snapshots, the fake ACP agent receives
  it on its first turn, and toggles after that turn are rejected

## Open questions

- Which delivery mechanism does each supported agent actually honour over ACP?
- Should user-specific context (installed apps, custom Files commands) be added at
  session start?
- Should the guide be versioned with Otto releases and reported in session `_meta`?
