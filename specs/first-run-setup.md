# First-Run Setup

**Status:** draft  
**Related specs:** [bug-reports.md](bug-reports.md) (the same terminal interface), [settings-app.md](settings-app.md) (Settings › Agents edits the same file)

## Summary

`otto-setup` is a terminal program that takes a new user through what Otto cannot do for itself on first run. It starts with the agents: Ask works only once an agent is configured, the agent service can find it, Otto's skills are linked and the service is running, and today each of those is a manual step.

## Goals

- A user with an agent installed and logged in gets a working Ask by running one command and pressing Apply.
- Every change is shown before it is made, exactly as it will be written or run.
- Running it again on a machine that is set up changes nothing and says so.
- It works without a terminal interface (`--check`, `--apply`), for scripts and for reading in a bug report.

## Non-Goals

- Installing harnesses, or logging them in. Otto runs what the user has; it does not manage accounts or keys.
- Changing or removing agents the user already has. That is Settings › Agents.
- Setup of anything but the agents, for now. The command takes a topic (`agents`) so more can follow.

## Behavior

- **Harnesses.** Claude Code, Codex, OpenCode, Hermes and pi are each listed as one of:
  - *in your agent list*: the agent list already runs it; it cannot be ticked;
  - *installed*: everything it needs is on the user's PATH, shown with where it was found; it can be ticked;
  - *not found*: names the program missing; ticking it explains what to install.
- **Initial ticks.** With no agent list at all, every installed harness is ticked. With a list, none is: the user has already chosen.
- **Steps.** From the facts and the ticks, in order, each marked done, to do, or blocked with why:
  1. *Agent list*: adds one agent per ticked harness to the user's `agents.toml`, written the way Settings › Agents writes a new agent (Otto's instructions, `permissions = "ask"`). Comments and unknown keys are kept. When the agents came from the system file, its list is copied in first. Ids are unique. A file that does not parse blocks the step and nothing is written to it.
  2. *Service PATH*: when a program an agent starts (its `command`, and `node` for `npx`) is on the user's PATH but not on the PATH the service starts with, adds the folders it lives in, through a drop-in of setup's own, on top of the PATH the service has now; then reloads the user manager.
  3. *Hermes profile*: when Hermes is configured or ticked and Otto's profile does not exist, creates it.
  4. *Otto's knowledge*: runs `otto-agents plugins install` when the skills are not linked yet or anything else changes.
  5. *Agent service*: enables and starts the service when it is not; restarts it when it is running and a step it reads at startup changed; otherwise done.
- **Preview.** Every step opens to show its full effect: the complete new file, or the commands.
- **Apply** runs the steps that are to do, in order, stops at the first that fails, then runs `otto-agents doctor` (retrying briefly while the service comes up) and shows each result, with what it printed.
- **Header.** Shows how many steps are left, or "all set".
- **Without a terminal**, or with `--check`, it prints the same list and steps and exits 1 when something is left to do. `--apply` applies with the initial ticks, without asking.

## Constraints & Edge Cases

- No user manager (a session without systemd): the PATH step is done (agents inherit the session's PATH) and the service step is blocked with how to run the service by hand.
- The service unit not installed: the service step is blocked.
- A drop-in the user wrote (including `override.conf`) is never written or removed.
- A command written as a full path needs no PATH and is not checked.
- The interface restores the terminal on exit and on a panic.

## Rationale

- *Only adds.* A first-run tool that can remove agents is a tool that can lose a configuration; Settings already edits and removes, with the user looking at each agent.
- *Same presets as Settings.* One place knows what a harness needs in its block, so a block written here is a block Settings reads back.
- *A drop-in of its own.* `systemctl edit` writes `override.conf`, and users write their own files; setup must never overwrite them. Building on the PATH the unit has now means the drop-in only adds.
- *The harness's own CLI is not checked against the service's PATH.* `claude --resume` and the like run in the user's terminal; the service only starts each agent's command.
- *The same terminal interface as `otto-msg doctor`.* Otto's command-line tools should look like one family; both are built on `otto-term-kit`.

## Open Questions

- Should setup detect whether a harness is logged in, and how, without running it?
- Further topics: displays, autostart, the bridge.
