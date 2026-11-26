# First-Run Setup

**Status:** draft  
**Related specs:** [bug-reports.md](bug-reports.md) (the same terminal interface), [settings-app.md](settings-app.md) (Settings › Agents edits the same file)

## Summary

`otto-setup` is a terminal program that takes a new user through what Otto cannot do for itself on first run. It starts with the agents: Ask works only once an agent is configured, the agent service can find it, Otto's skills are linked and the service is running, and today each of those is a manual step.

## Goals

- A user with an agent installed and logged in gets a working Ask by running one command and pressing Apply.
- A user missing something (herdr, a harness, Node.js, an ACP adapter) is told exactly what to run.
- Every change is shown before it is made, exactly as it will be written or run.
- Running it again on a machine that is set up changes nothing and says so.
- It works without a terminal interface (`--check`, `--apply`), for scripts and for reading in a bug report.

## Non-Goals

- Installing software of any kind, or running a package manager, `npm` or `sudo`. Setup explains; the user installs.
- Signing in to a harness, or making a chat bot. Otto runs what the user has; it does not manage accounts or keys.
- Changing or removing agents the user already has. That is Settings › Agents. The one exception is *Rewrite for herdr* (step 1 below), which runs only when the user ticks it.
- Setup of anything but the agents, for now. The command takes a topic (`agents`) so more can follow.

## Behavior

A welcome comes first: setup configures Otto by bootstrapping what a part of it needs, explains what it cannot do, and leaves the finer configuration to Settings. It asks what to set up; for now the one choice is the Otto Agents, which answer in Ask and run Sessions and are used from Otto itself, and more dependencies can join that list later. Choosing it opens three pages, switched with Tab (and ← →): Agents ("Which agents do you want to set up?"), Requirements, Review. Every row opens its instructions: what is there, with where it was found, and the commands to run for what is not.

- **Sessions run in.** The first row of the Agents page: herdr or Otto. It starts as what `otto-agents` would use: `backend` in `agents.toml` (the user's file over the system's), else herdr when `hc` is on the user's PATH, else Otto. Space switches it, and the ticks follow (a harness the new backend cannot run is unticked; with no agent list every ready one is ticked). Apply writes `backend` only when the choice differs from what Automatic gives. A `backend` already set is shown and never changed: the row points to Settings › Agents › Sessions run in. herdr is a row here, not a topic of its own, because it decides how the agents run rather than being another part of Otto to set up.
- **Agents.** Claude Code, Codex, OpenCode, Hermes and pi are each one of:
  - *in your agent list*: it cannot be ticked; in herdr, Codex and Hermes add "runs only in Otto";
  - *ready*: in herdr, its own program is on the user's PATH; in Otto, its own program, and its adapter or `npx` to run the adapter with, are; it can be ticked;
  - *needs …*: names what is missing; ticking it says to open the instructions, which give the npm package (or, for Hermes, its own installer), in Otto the adapter, and how to sign in;
  - *runs only in Otto* (herdr only, Codex and Hermes): herdr does not run it, so it cannot be ticked; the instructions say to switch Sessions run in to Otto. Not offering it is clearer than writing an agent the service would leave out.
- **Also set up.** Two boxes for Otto's own plugin, never the user's: *Otto*, ticked, makes the agents added now answer as Otto; unticked, they are the plain harness. *Studio* is Studio's chat, which runs only on Claude Code: ticking it ticks Claude Code when it is ready, and without Claude Code it says so. Settings › Agents changes an agent later.
- **Initial ticks.** With no agent list at all, every ready harness is ticked. With a list, none is. Otto is ticked when its plugin is there, and Studio when a Claude Code agent is there or ticked.
- **Requirements.** Depend on what runs the sessions. Nothing on this page can be ticked.
  - *In herdr*: herdr (`hc` and `herdr-app`; setup installs nothing and points to https://herdr.dev) and the agents' own programs (`claude`, `pi`, `opencode`, with `sqlite3` when there is an OpenCode agent, which the service reads OpenCode's sessions with), each with how to install it and sign in. No Node.js, no adapters.
  - *In Otto*: Node.js (its version, or how to install it with the system's package manager or nvm) and the ACP adapters of the agents there are or are being added (each installed, fetched with `npx`, or missing, with how to install it).
- **Review checks**, for the agents there are or are being added: the requirements above, Otto's skills (linked, to link, or something of the user's in the way) and, in Otto, each harness's profile of Otto's agent (current, or to write), the last two read from `otto-agents plugins status`. In herdr no profile is needed: the service gives Claude Code and pi Otto's agent on their command line, and OpenCode in the config it starts with.
- **Instructions always.** Every row's instructions give the full steps with the exact commands, those already done marked so.
- **Programs already installed are used.** In Otto, a new agent on a harness whose adapter is installed starts the adapter, not `npx`.
- **Steps**, in order, each done, to do, or blocked with why:
  1. *Rewrite for herdr*: only when the sessions run in herdr and the agent list the service reads has agents set up for Otto's ACP sessions whose harness herdr runs (Claude Code, pi, OpenCode; never Codex or Hermes). Such an agent has an adapter as its command, or `args`, `enter`, `enter_new`, `env` or `config`, which herdr ignores. The harness is recognised the way the rest of setup recognises it (`Harness::detect`). Rewriting one writes its block again from the herdr preset, the same way the *Agents* step writes a new herdr agent (`harness.rs`'s `herdr_program` and `write_herdr`, `permissions = "ask"`), keeping only `id`, `name`, `model`, `mode`, `skills`, `folder`, `colour`, `space` and the entry's comments. Who it answers as follows the Otto box on the Agents page, not the old entry; everything else in the old block (`command`, `args`, `enter`, `enter_new`, `env`, `config`, `permissions`, a persona of its own, unknown keys) is replaced by what the preset writes. The step is a box, off by default: unticked it shows as offered and does nothing; Space ticks it. Its instructions say what is kept and what is not carried over, show each agent before and after, and the backup. On Apply it first copies `agents.toml` to `agents.toml.bak`, then writes the rewritten file. When the agents came from the system file, the list is copied into the user's file and rewritten there, with no backup since there was no file. A file that does not parse offers nothing. The *Agents* step then adds on top of the rewritten file.
  2. *Agents*: adds one agent per ticked harness to the user's `agents.toml`, written the way Settings › Agents writes a new agent, with the chosen instructions and `permissions = "ask"`, and `backend` when the choice needs writing. In herdr an agent's command is the harness's own program (`claude`, `pi`, `opencode`) with no args and no enter lines; Claude Code and pi take the instructions as `agent`, OpenCode gets what its preset writes. Comments and unknown keys are kept. When the agents came from the system file, its list is copied in first. Ids are unique. A file that does not parse, or a `backend` the service refuses, blocks the step and nothing is written to it.
  3. *Program paths*: when a program the agent service runs itself is on the user's PATH but not the unit's, adds the folders, through a drop-in of setup's own, on top of the PATH the unit has now; then reloads the user manager. In Otto those are the agents' commands, and Node for `npx` and the adapters. In herdr they are `hc` and `herdr-app`, and with OpenCode `opencode` (the service makes each session with `opencode import`) and `sqlite3`; herdr starts the agents' own programs with its own, login-shell-like PATH, so those are checked on the user's PATH only.
  4. *Hermes profile*: in Otto, when a Hermes agent is added with instructions and the profile named after them does not exist, creates it.
  5. *Teach the agents*: runs `otto-agents plugins install` when a skill is to link, a selected harness's profile is stale or missing, or anything else changes.
  6. *Agent service*: enables and starts the unit when it is not; restarts it when it is running and a step it reads at startup changed; otherwise done.
- **Apply** runs the steps that are to do, in order, stops at the first that fails, then runs `otto-agents doctor` (retrying briefly while the service comes up) and shows each result, with what it printed. In herdr, doctor checks `hc`, `herdr-app` and the program herdr runs for each agent (and `sqlite3` with OpenCode), and notes the agents herdr leaves out without counting them as problems.
- **Header.** Shows what is left: "N to fix" for what the user must do by hand (a required program missing, a skill name taken), "N to apply" for steps, or "nothing to apply".
- **Without a terminal**, or with `--check`, it prints what runs the sessions and why, the harnesses, Otto and Studio, the requirements and the steps, and exits 1 when something is left to do, an offered *Rewrite for herdr* included. `--apply` applies with the initial choices, without asking, and leaves the agents to rewrite as they are; `--apply --rewrite` rewrites them too. Agents the user has are never changed unasked, so a script asks for it by name.

## Constraints & Edge Cases

- No user manager (a session without systemd): the PATH step is done (agents inherit the session's PATH) and the service step is blocked with how to run the service by hand.
- The service unit not installed: the service step is blocked.
- A drop-in the user wrote (including `override.conf`) is never written or removed.
- A command written as a full path needs no PATH and is not checked.
- The interface restores the terminal on exit and on a panic.

- Node.js older than 22 (what Debian and Ubuntu's own `nodejs` package is): shown as something to fix, since the Claude Code adapter needs 22 or newer; the instructions say so and give the nvm command.

- **Words.** No jargon on screen: agents, not harnesses; adapters, with ACP spelled out once; Otto's agent files, not plugins; "added", not the agent list; "name taken", not "in the way". Every step says who does it, and commands stand on their own lines.

## Rationale

- *Installs nothing.* Package managers want `sudo`, npm installs land in different places for system Node and nvm, and a setup tool that runs them owns every way they fail. Instructions with the exact command keep setup small and the user in charge of their system.
- *Only adds.* A first-run tool that can remove agents is a tool that can lose a configuration; Settings already edits and removes, with the user looking at each agent.
- *Rewriting for herdr is the one change, and it is opt-in.* An agent set up for Otto's ACP sessions runs in herdr without its adapter's settings, or not at all, and fixing each by hand means knowing which keys herdr ignores. Rewriting from the preset rather than editing the old block in place keeps one way of writing a herdr agent, so a rewritten agent is the same as a new one. The step shows exactly what changes, starts unticked, and keeps a backup.
- *Same presets as Settings.* One place knows what a harness needs in its block, so a block written here is a block Settings reads back.
- *A drop-in of its own.* `systemctl edit` writes `override.conf`, and users write their own files; setup must never overwrite them. Building on the PATH the unit has now means the drop-in only adds.
- *The harness's own CLI is not checked against the service's PATH.* In Otto, `claude --resume` and the like run in the user's terminal; the service only starts each agent's command. In herdr, herdr-core starts the program with its own login-shell-like PATH, not the service's.
- *Writes `backend` only when needed.* Left unset, the service follows herdr being installed or not; setup keeps that unless the user picks otherwise, and a key the user set is theirs.
- *The same terminal interface as `otto-msg doctor`.* Otto's command-line tools should look like one family; both are built on `otto-term-kit`.

## Testing

- The plan against made-up machines and the readers against captured output: `cargo test -p otto-setup`.
- On a clean machine: `scripts/test-otto-setup.sh` runs the binaries on a clean Ubuntu or Debian image and checks the screens and `--check`/`--apply` output for nothing installed, the distribution's too-old Node.js, an agent ready to add, and herdr installed (stand-ins for `hc`, `herdr-app` and `claude`) (`components/otto-setup/tests/clean-machine/`).

## Open Questions

- Should setup detect whether a harness is logged in, and how, without running it?
- Setup writes no `model`. pi then uses the default from `~/.pi/agent/settings.json` or what `/login` set up; should the instructions say so, or should setup ask for a model when none is set?
- Should agents already in the list that run through `npx` be offered the installed adapter?
- Further topics: displays, autostart.
