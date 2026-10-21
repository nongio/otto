# First-Run Setup

**Status:** draft  
**Related specs:** [bug-reports.md](bug-reports.md) (the same terminal interface), [settings-app.md](settings-app.md) (Settings › Agents edits the same file)

## Summary

`otto-setup` is a terminal program that takes a new user through what Otto cannot do for itself on first run. It starts with the agents: Ask works only once an agent is configured, the agent service can find it, Otto's skills are linked and the service is running, and today each of those is a manual step.

## Goals

- A user with an agent installed and logged in gets a working Ask by running one command and pressing Apply.
- A user missing something (Node.js, a harness, an ACP adapter, cc-connect) is told exactly what to run.
- Every change is shown before it is made, exactly as it will be written or run.
- Running it again on a machine that is set up changes nothing and says so.
- It works without a terminal interface (`--check`, `--apply`), for scripts and for reading in a bug report.

## Non-Goals

- Installing software of any kind, or running a package manager, `npm` or `sudo`. Setup explains; the user installs.
- Signing in to a harness, or making a chat bot. Otto runs what the user has; it does not manage accounts or keys.
- Changing or removing agents the user already has. That is Settings › Agents.
- Setup of anything but the agents, for now. The command takes a topic (`agents`) so more can follow.

## Behavior

A welcome comes first: setup configures Otto by bootstrapping what a part of it needs, explains what it cannot do, and leaves the finer configuration to Settings. It asks what to set up; for now the one choice is the Otto Agents, which answer in Ask and run Sessions and are used from Otto itself, and more dependencies can join that list later. Choosing it opens three pages, switched with Tab (and ← →): Agents ("Which agents do you want to set up?"), Extras, Review. Every row opens its instructions: what is there, with where it was found, and the commands to run for what is not.

- **Agents.** Claude Code, Codex, OpenCode, Hermes and pi are each one of:
  - *in your agent list*: it cannot be ticked;
  - *ready*: its own program, and its adapter or `npx` to run the adapter with, are on the user's PATH; it can be ticked;
  - *needs …*: names what is missing; ticking it says to open the instructions, which give the npm package (or, for Hermes, its own installer), the adapter, and how to sign in.
- **Instructions.** One choice for the agents added now: each plugin agent there is (Otto's first), or the harness's own. Settings › Agents changes it later.
- **Initial ticks.** With no agent list at all, every ready harness is ticked. With a list, none is. Otto's instructions are picked when there are any.
- **Extras.** Node.js (its version, or how to install it with the system's package manager or nvm), the ACP adapters (which are installed; how to install the rest), and the chat bridge: running, set up but off, ready (cc-connect and its configuration found), or the steps still missing (install cc-connect; make a Telegram bot; write its configuration, given with the paths filled in). Only the bridge can be ticked, and only when ready or off.
- **Programs already installed are used.** A new agent on a harness whose adapter is installed starts the adapter, not `npx`.
- **Steps**, in order, each done, to do, or blocked with why:
  1. *Agent list*: adds one agent per ticked harness to the user's `agents.toml`, written the way Settings › Agents writes a new agent, with the chosen instructions and `permissions = "ask"`; and, when the bridge is ticked and ready, a `[bridge]` that runs cc-connect with its configuration by full path. Comments and unknown keys are kept. When the agents came from the system file, its list is copied in first. Ids are unique. A file that does not parse blocks the step and nothing is written to it.
  2. *Service PATH*, and *Bridge PATH* when the bridge runs or will: when a program a unit starts (an agent's command; Node for `npx`, the adapters and an npm-installed cc-connect) is on the user's PATH but not the unit's, adds the folders, through a drop-in of setup's own, on top of the PATH the unit has now; then reloads the user manager.
  3. *Hermes profile*: when a Hermes agent is added with instructions and the profile named after them does not exist, creates it.
  4. *Otto's knowledge*: runs `otto-agents plugins install` when the skills are not linked yet or anything else changes.
  5. *Agent service*, and *Chat bridge* when ticked or running: enables and starts the unit when it is not; restarts it when it is running and a step it reads at startup changed; otherwise done.
- **Apply** runs the steps that are to do, in order, stops at the first that fails, then runs `otto-agents doctor` (retrying briefly while the service comes up) and shows each result, with what it printed.
- **Header.** Shows how many steps are left, or "all set".
- **Without a terminal**, or with `--check`, it prints the harnesses, the instructions, the extras and the steps, and exits 1 when something is left to do. `--apply` applies with the initial choices, without asking.

## Constraints & Edge Cases

- No user manager (a session without systemd): the PATH step is done (agents inherit the session's PATH) and the service step is blocked with how to run the service by hand.
- The service unit not installed: the service step is blocked.
- A drop-in the user wrote (including `override.conf`) is never written or removed.
- A command written as a full path needs no PATH and is not checked.
- The interface restores the terminal on exit and on a panic.

## Rationale

- *Installs nothing.* Package managers want `sudo`, npm installs land in different places for system Node and nvm, and a setup tool that runs them owns every way they fail. Instructions with the exact command keep setup small and the user in charge of their system.
- *Only adds.* A first-run tool that can remove agents is a tool that can lose a configuration; Settings already edits and removes, with the user looking at each agent.
- *Same presets as Settings.* One place knows what a harness needs in its block, so a block written here is a block Settings reads back.
- *A drop-in of its own.* `systemctl edit` writes `override.conf`, and users write their own files; setup must never overwrite them. Building on the PATH the unit has now means the drop-in only adds.
- *The harness's own CLI is not checked against the service's PATH.* `claude --resume` and the like run in the user's terminal; the service only starts each agent's command.
- *The same terminal interface as `otto-msg doctor`.* Otto's command-line tools should look like one family; both are built on `otto-term-kit`.

## Open Questions

- Should setup detect whether a harness is logged in, and how, without running it?
- Should agents already in the list that run through `npx` be offered the installed adapter?
- Further topics: displays, autostart, the bridge.
