# First-Run Setup

**Status:** draft  
**Related specs:** [bug-reports.md](bug-reports.md) (the same terminal interface), [settings-app.md](settings-app.md) (Settings › Agents edits the same file)

## Summary

`otto-setup` is a terminal program that takes a new user through what Otto cannot do for itself on first run. It has two topics. The agents: Ask works only once an agent is configured, the agent service can find it, Otto's skills are linked and the service is running, and today each of those is a manual step. Dictation: speech is recognised by a local speech server that is not packaged, and dictating in every field needs otto-dictate started at login and a key bound to it.

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
- Building a speech engine, or downloading a model. `install.sh` in Otto's source does; setup points at it.
- Setup of anything but the agents and dictation, for now. The command takes a topic (`agents`, `dictation`) so more can follow.

## Behavior

A welcome comes first: setup configures Otto by bootstrapping what a part of it needs, explains what it cannot do, and leaves the finer configuration to Settings. It asks what to set up: the Otto Agents, which answer in Ask and run Sessions, or Dictation, speaking into the launcher, the Ask chat and any text field; more dependencies can join that list later. Choosing one opens three pages, switched with Tab (and ← →): what to set up (Agents, or Engine), Requirements, Review. Every row opens its instructions: what is there, with where it was found, and the commands to run for what is not.

### Agents

- **Agents.** Claude Code, Codex, OpenCode, Hermes and pi are each one of:
  - *in your agent list*: it cannot be ticked;
  - *ready*: its own program, and its adapter or `npx` to run the adapter with, are on the user's PATH; it can be ticked;
  - *needs …*: names what is missing; ticking it says to open the instructions, which give the npm package (or, for Hermes, its own installer), the adapter, and how to sign in.
- **Also set up.** Two boxes for Otto's own plugin, never the user's: *Otto*, ticked, makes the agents added now answer as Otto; unticked, they are the plain harness. *Studio* is Studio's chat, which runs only on Claude Code: ticking it ticks Claude Code when it is ready, and without Claude Code it says so. Settings › Agents changes an agent later.
- **Initial ticks.** With no agent list at all, every ready harness is ticked. With a list, none is. Otto is ticked when its plugin is there, and Studio when a Claude Code agent is there or ticked.
- **Requirements.** Required: Node.js (its version, or how to install it with the system's package manager or nvm) and the ACP adapters of the agents there are or are being added (each installed, fetched with `npx`, or missing, with how to install it). Optional: the chat bridge, to message the agents from Telegram: running, set up but off, ready (cc-connect and its configuration found), or the steps still missing (install cc-connect; make a Telegram bot; write its configuration, given with the paths filled in). Only the bridge can be ticked, and only when ready or off.
- **Review checks**, for the agents there are or are being added: Node.js, their ACP adapters, cc-connect, Otto's skills (linked, to link, or something of the user's in the way) and each harness's profile of Otto's agent (current, or to write), the last two read from `otto-agents plugins status`.
- **Instructions always.** Every row's instructions give the full steps with the exact commands, those already done marked so.
- **Programs already installed are used.** A new agent on a harness whose adapter is installed starts the adapter, not `npx`.
- **Steps**, in order, each done, to do, or blocked with why:
  1. *Agents*: adds one agent per ticked harness to the user's `agents.toml`, written the way Settings › Agents writes a new agent, with the chosen instructions and `permissions = "ask"`; and, when the bridge is ticked and ready, a `[bridge]` that runs cc-connect with its configuration by full path. Comments and unknown keys are kept. When the agents came from the system file, its list is copied in first. Ids are unique. A file that does not parse blocks the step and nothing is written to it.
  2. *Program paths*, and *Bridge paths* when the bridge runs or will: when a program a unit starts (an agent's command; Node for `npx`, the adapters and an npm-installed cc-connect) is on the user's PATH but not the unit's, adds the folders, through a drop-in of setup's own, on top of the PATH the unit has now; then reloads the user manager.
  3. *Hermes profile*: when a Hermes agent is added with instructions and the profile named after them does not exist, creates it.
  4. *Teach the agents*: runs `otto-agents plugins install` when a skill is to link, a selected harness's profile is stale or missing, or anything else changes.
  5. *Agent service*, and *Chat bridge* when ticked or running: enables and starts the unit when it is not; restarts it when it is running and a step it reads at startup changed; otherwise done.
- **Apply** runs the steps that are to do, in order, stops at the first that fails, then runs `otto-agents doctor` (retrying briefly while the service comes up) and shows each result, with what it printed.
- **Header.** Shows what is left: "N to fix" for what the user must do by hand (a required program missing, a skill name taken), "N to apply" for steps, or "nothing to apply".
- **Without a terminal**, or with `--check`, it prints the harnesses, Otto and Studio, the requirements, the bridge and the steps, and exits 1 when something is left to do: a step to do, or one blocked on something the user must do first. `--apply` applies with the initial choices, without asking.

### Dictation

`otto-setup dictation`, or Dictation on the welcome. The engines live where `components/otto-dictate/engines/install.sh` puts them (`~/.local/share/otto-voice-poc`, units in `~/.config/systemd/user`); setup never builds or downloads one.

- **Engine.** Parakeet (the default), Whisper and CrispASR, one picked with Space, radio-style. Each is *installed* (its server program and model are there and systemd knows its unit), *running*, or *not installed*, naming what is missing; only an installed one can be picked. The instructions give the build tools for the system's package manager (and rustup where the distribution's Rust is old), `git clone` of Otto's source when no checkout is found (the one setup was built from, or `~/otto`, `~/dev/otto`, `~/src/otto`, `~/code/otto`, `~/projects/otto`), the `install.sh <engine>` command with the checkout's path, and that Apply picks it and starts it.
- **Initial pick.** The engine `dictation.toml` names when it is installed, else the one running, else the one enabled, else the first installed; none when none is installed.
- **Requirements.** Required: the microphone (PipeWire installed and running, by its socket in the runtime directory) and a GPU driver (a Vulkan loader manifest in `/usr/share/vulkan/icd.d`, `/etc/vulkan/icd.d` or `/usr/local/share/vulkan/icd.d`), each with the install command for the system's package manager. To install an engine: the build tools `install.sh` needs (git, cmake, a C++ compiler, glslc, cargo) and a checkout of Otto's source, not needed once an engine is installed. Optional: *Other fields*, otto-dictate, the dictation in every text field: not installed, installed, or starting at login; a box when installed and not yet started at login. Its instructions give `cargo install` from the checkout, that Apply writes the autostart entry, and the `[keyboard_shortcuts]` line to bind a key, found when `config.toml` binds one to otto-dictate.
- **Review checks**: the speech server (the engine running, and whether anything answers at `127.0.0.1:8080`; something answering with no engine running is called out, not touched), the microphone, the GPU driver, otto-dictate.
- **Steps**, in order:
  1. *Settings file*: writes `engine = "<id>"` into `~/.config/otto/dictation.toml`, keeping its other keys and comments; done when it says so already; blocked when the file does not parse or no engine is installed.
  2. *Speech server*: `systemctl --user disable` for the other engines' units that are enabled, then `enable --now` for the picked one; done when it is running and enabled; blocked when it is not installed or there is no user manager.
  3. *Dictation at login*, when otto-dictate is ticked or already starts at login: writes `~/.config/autostart/otto-dictate.desktop`, switching the user's own entry (or a copy of the system's) back on with `Hidden=false` and `X-GNOME-Autostart-enabled=true`, or a new entry running otto-dictate where it was found. Blocked when otto-dictate is not installed.
- **Apply** ends by knocking on `127.0.0.1:8080` for up to 30 seconds, since an engine loads its model before it listens, and says whether it answers.
- **Header.** "N to fix" counts no engine installed, PipeWire missing, no Vulkan driver.
- **`--check`** prints the engines, the server, the requirements, otto-dictate and its shortcut, and the steps; `--apply` applies the initial pick.

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
- *Same presets as Settings.* One place knows what a harness needs in its block, so a block written here is a block Settings reads back.
- *A drop-in of its own.* `systemctl edit` writes `override.conf`, and users write their own files; setup must never overwrite them. Building on the PATH the unit has now means the drop-in only adds.
- *The harness's own CLI is not checked against the service's PATH.* `claude --resume` and the like run in the user's terminal; the service only starts each agent's command.
- *The same terminal interface as `otto-msg doctor`.* Otto's command-line tools should look like one family; both are built on `otto-term-kit`.

## Testing

- The plan against made-up machines and the readers against captured output: `cargo test -p otto-setup`.
- On a clean machine: `scripts/test-otto-setup.sh` runs the binaries on a clean Ubuntu or Debian image and checks the screens and `--check`/`--apply` output for nothing installed (both topics), the distribution's too-old Node.js, and an agent ready to add (`components/otto-setup/tests/clean-machine/`).

## Open Questions

- Should setup detect whether a harness is logged in, and how, without running it?
- Setup writes no `model`. pi then uses the default from `~/.pi/agent/settings.json` or what `/login` set up; should the instructions say so, or should setup ask for a model when none is set?
- Should agents already in the list that run through `npx` be offered the installed adapter?
- Further topics: displays, autostart, the bridge.
