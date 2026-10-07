# 0015: The otto plugin reaches every harness

**Status:** Draft

## Goal

Both halves of the `otto` plugin, the `otto-help` skill and the `otto` agent
(`agents/otto.md`), reach every coding agent a person has, whether it is
started from Ask or from a terminal. What they reach stays the packaged copy,
so an Otto upgrade upgrades it everywhere.

Today `otto-agents plugins install` links each skill into `~/.agents/skills` and
stops there (0005, item 13). Codex, Cursor, Gemini CLI and OpenCode read that
folder; Claude Code in a terminal reads `~/.claude/skills` and does not, so
`otto-help` is missing from `claude` sessions on an Otto desktop. Nearly eighty
other harnesses each have a folder of their own.

The agent is narrower still. `vendors.rs` renders it for OpenCode, Hermes,
Codex and pi, and Claude gets it only through Ask (`--agent plugin:otto`). A
`claude` started in a terminal has no `otto` agent, and neither does any
harness outside those four.

[`skills`](https://github.com/vercel-labs/skills) (`npx skills`, MIT, Vercel
Labs) already keeps that table: one entry per harness with its global folder and
how to tell it is installed. Otto should use it rather than grow its own.

## How `npx skills` behaves (checked with 1.7.1)

- `npx skills add <dir> -g --skill <name> -y` installs from a local plugin
  directory. It finds skills through `.claude-plugin/plugin.json`, so
  `/usr/share/otto/plugins/otto` works as it is. Run against the repository, it
  finds `otto-help` and nothing else.
- It detects harnesses by the existence of each one's home folder (`~/.claude`,
  `~/.codex`, …) and installs only to those. It never creates a folder for a
  harness the person does not have.
- **It writes a canonical copy to `~/.agents/skills/<name>`, then symlinks each
  harness's folder to it** (`~/.claude/skills/otto-help ->
  ../../.agents/skills/otto-help`). Harnesses it believes read `~/.agents/skills`
  directly (Codex, Cursor, Gemini CLI, OpenCode) get no link of their own.
- **It replaces whatever was at the canonical path with a frozen copy**,
  including Otto's symlink into `/usr/share`, even when the source is that same
  directory. `pathsOverlap` (`src/installer.ts`) compares path strings and does
  not resolve symlinks. After that, an upgrade of the package no longer reaches
  the skill.
- It records the install in `~/.agents/.skill-lock.json`:
  `"otto-help": {"source": "/usr/share/otto/plugins/otto", "sourceType": "local", …}`.
- `npx skills ls -g --json` lists each skill with the harnesses it reaches.
- Without `-a`, it reports one spurious failure: "PromptScript does not support
  global skill installation".

Putting Otto's symlink back over the frozen copy fixes everything: every
harness link is relative to the canonical path, so they all resolve to
`/usr/share` again. Checked in a scratch `HOME` with Claude Code, Codex, Cursor,
Gemini CLI and OpenCode set up.

## Decisions

- **`npx skills` does the fan-out; otto-agents owns the canonical link.**
  `~/.agents/skills/<name>` stays a symlink into the plugin directory, made and
  checked by `skills::install` as today. `npx skills` only adds the per-harness
  links that point at it.
- **The skill through `npx skills`, the agent through `vendors.rs`.**
  `npx skills` installs skills and nothing else (its only notion of agents is
  Eve's subagent skill folders), so `agents/otto.md` keeps Otto's own route and
  that route grows: one more `Vendor` per harness that reads agent files (see
  "The `otto` agent" below). The Claude plugin route through claude-agent-acp's
  `_meta` is unchanged.
- **One command, both halves.** The agent declares `skills: otto-help`, so an
  agent without its skill is an agent that cannot do its job. `plugins install`
  installs both, and `plugins status` reports a harness that has one without
  the other.
- **Pinned.** The version is a constant in `skills.rs` (`skills@1.7.1`), bumped
  by hand after checking the behaviour above again. Never `@latest`: the CLI is
  young and its install layout has changed between releases.
- **No telemetry.** Every call sets `DISABLE_TELEMETRY=1`. The source is a local
  path, and nothing about the person's desktop should leave the machine to
  install it.
- **Local sources only.** otto-agents installs from the plugin directories that
  `skills::discover` found (`$XDG_DATA_HOME/otto/plugins`, then
  `/usr/share/otto/plugins`), never from GitHub, so the skill matches the
  installed Otto.
- **Never take what is not ours.** A skill whose canonical path is
  `LinkState::Taken` before the run is left out of the `--skill` list, as
  `install` leaves it alone today. After the run, a directory at the canonical
  path is replaced with our link only when the lock entry's `source` is the
  plugin directory we passed.
- **Node is optional.** Without `npx` on `PATH`, `plugins install` does what it
  does today and says what it skipped: "npx not found: Otto's skills are in
  ~/.agents/skills only. Claude Code and other agents that look elsewhere will
  not see them until Node is installed."

## The `otto` agent

`agents/otto.md` is written in Claude Code's own agent dialect: frontmatter
`name`, `description`, `skills: otto-help`, `tools`, then the prompt. Each new
target is a `Vendor`, rendered with the marker line, written only when the
harness's folder already exists, and checked against the installed version the
way the existing four were (the version and what was verified go in the
variant's docs).

- **Claude Code: `~/.claude/agents/otto.md`.** The source file almost as it is:
  the same frontmatter, plus the marker. Claude Code then offers the agent in a
  terminal (`claude --agent otto`, or as a subagent). Check: a user agent named
  `otto` and the plugin's `plugin:otto` loaded through Ask do not collide, and
  `skills: otto-help` resolves against `~/.claude/skills/otto-help`.
- **Candidates, each to be verified before it gets a variant:** GitHub Copilot
  CLI (`~/.copilot/agents/`), Gemini CLI (`~/.gemini/agents/`), Cursor
  (`~/.cursor/agents/`). Only harnesses whose agent file can name or carry the
  skill are worth adding; one that cannot gets the skill alone and reads the
  prompt's intent from `otto-help`'s own page.
- **Codex and pi stay as they are.** They have no agent files; the instructions
  already reach them through `CODEX_CONFIG` and `--append-system-prompt`, for
  sessions started from Ask. A terminal `codex` keeps the skill only. Writing
  the prompt into `~/.codex/AGENTS.md` would put it in every project, which
  `vendors.rs` already decided against.

## `plugins install`, after the change

1. Prune stale links (`skills::prune`, unchanged).
2. Link `~/.agents/skills/<name>` for every discovered skill (`skills::install`,
   unchanged). Note which skills are `Linked`.
3. For each plugin directory with linked skills, run
   `npx -y skills@1.7.1 add <plugin dir> -g -y --skill <linked names…>` with
   `DISABLE_TELEMETRY=1`. Stdout and stderr are captured, not printed.
4. **Re-link.** For each skill passed in step 3: if the canonical path is now a
   real directory and its `.skill-lock.json` entry has `sourceType: "local"`
   and `source` equal to the plugin directory, remove it and restore the
   symlink. Anything else is reported and left alone.
5. Ask `npx skills ls -g --json` which harnesses now reach each skill, and
   print one line per skill: `otto-help: Claude Code, Codex, Cursor, Gemini
   CLI, OpenCode`.
6. Write agent files (`vendors::install`), now including the Claude Code
   variant and any others verified in "The `otto` agent".

`--vendor <id>` keeps skipping steps 1–5, as `only` does today.

`plugins status` adds the harness line from step 5, and flags a canonical path
that is a copy with a local lock entry pointing at one of our plugin
directories: "otto-help: replaced by a copy from npx skills; run `otto-agents
plugins install` to link it again". A person who runs `npx skills add` by hand
ends up there.

## Phases

### 1. Fan-out behind the re-link (otto-agents)

- [ ] `skills::fan_out(plugins, linked) -> FanOut`: spawn the CLI, read the lock
      file, re-link (steps 3–4). Lock file parsing is `serde_json` with only the
      fields above.
- [ ] `skills::harnesses() -> BTreeMap<String, Vec<String>>` from `ls -g --json`.
- [ ] `cli.rs`: wire into `install_plugins` and `plugins_status`; the npx-missing
      message.
- [ ] Tests: re-link logic against a scratch directory with a copy and a lock
      file (ours, someone else's, malformed, missing). One `#[ignore]` test that
      runs the real CLI in a scratch `HOME`, for checking a version bump.
- [ ] Docs: `docs/user/agents.md` step 3 and "What `plugins install` writes"
      (the harness folders, the lock file, the Node requirement);
      `docs/developer/agents.md`; a pointer from 0005 item 13 to this plan.

### 2. The `otto` agent in more harnesses (otto-agents)

- [ ] `Vendor::ClaudeCode`: render `agents/otto.md` to `~/.claude/agents/otto.md`
      with the marker; verify against the installed Claude Code, including the
      `plugin:otto` collision check.
- [ ] Verify the candidates (Copilot CLI, Gemini CLI, Cursor) and add a variant
      for each that holds up.
- [ ] `plugins status`: per harness, skill and agent side by side, flagging one
      without the other.
- [ ] Docs: the agent table in `docs/user/agents.md` gains the terminal column
      ("`otto` agent in a terminal": Claude Code yes, Codex no, …).

### 3. Upstream: resolve symlinks in `pathsOverlap`

- [ ] PR to vercel-labs/skills: compare real paths, so a canonical path that is
      already a link to the source is skipped instead of replaced. Include a test
      with exactly Otto's layout.
- [ ] Once released: bump the pin. Step 4 becomes a check that finds nothing to
      do; keep it, because older CLIs and manual runs still copy.

### 4. Publish the plugin for people not on Otto yet

Separate from the migration, and only worth doing once 1 and 2 are in, so the
installs do not fight on an Otto desktop.

- [ ] Root `.claude-plugin/marketplace.json` pointing at `resources/plugins/otto`.
      It makes `npx skills add nongio/otto` explicit (skill only), and the
      repository a Claude Code plugin marketplace
      (`/plugin marketplace add nongio/otto`), which installs the skill and the
      agent together.
- [ ] `otto-help` says so when it is not on Otto: no `org.otto.*` names on the
      session bus means it answers from the guides and does not run commands.
- [ ] README and website: `npx skills add nongio/otto`, and
      `npx skills use nongio/otto@otto-help | claude` to try it without installing.
      Installs count towards the skills.sh listing.

## Open questions

- **Agent files can't be links.** Skills stay live through the symlink; a
  rendered agent file is a copy, refreshed only when `plugins install` runs.
  For Claude Code the source needs no rendering beyond the marker, so a symlink
  to `/usr/share/otto/plugins/otto/agents/otto.md` would stay live too, but
  loses the marker that tells `vendors.rs` the file is ours. Link or render?

- **`npx skills update -g`.** Does it re-copy a `local` entry over our link? If
  so, `plugins status` catches it and `plugins install` repairs it, but the
  person sees a frozen skill in between. Check before phase 1 lands; phase 2
  would make it moot.
- **When it runs.** Harness detection happens at install time, so an agent
  installed later is missed until `plugins install` runs again. Run it on the
  first start after a package upgrade (the service already notices plugin
  changes only at startup), behind a button in Settings, or both?
- **Removing.** `npx skills remove -g otto-help` removes our canonical link
  along with the harness links. Acceptable, given removal is by hand today, or
  should `plugins status` offer to restore?
- **Offline first run.** `npx -y` downloads the CLI once. Vendor it in the
  package instead (a `node_modules` under `/usr/share/otto`), or accept the
  download as Otto already does for claude-agent-acp?
