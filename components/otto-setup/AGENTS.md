# otto-setup

`otto-setup` configures Otto the first time, from a terminal. It bootstraps the requirements; Settings has the detailed configuration. A welcome says that and asks what to set up: the Otto Agents, which answer in Ask and run Sessions, or Dictation. Each topic opens three pages, the same shape.

The agents (`otto-setup agents`, the default):

- **Agents:** the harnesses (Claude Code, Codex, OpenCode, Hermes, pi), each shown as in the agent list, ready, or what it is missing, and two boxes for Otto's plugin: Otto (the new agents answer as Otto, or are the plain harness) and Studio (Studio's chat, which needs a Claude Code agent).
- **Requirements:** required, Node.js and the ACP adapters of the agents there are or are being added; optional, the chat bridge (cc-connect), to message the agents from Telegram. Each with where it was found.
- **Review:** checks for the agents selected (Node.js, their ACP adapters, cc-connect, Otto's skills, each harness's profile of Otto's agent), then the steps and Apply. The steps write the new agents into `agents.toml`, add the folders the programs live in to each unit's PATH, create the Hermes profile, link Otto's skills (`otto-agents plugins install`), and start or restart `otto-agents.service` and, when ticked, `otto-agents-bridge.service`. It ends with `otto-agents doctor`.

Dictation (`otto-setup dictation`):

- **Engine:** Parakeet, Whisper and CrispASR, one picked, each installed (where `components/otto-dictate/engines/install.sh` puts it), running, or not installed with how to build it from Otto's source.
- **Requirements:** required, the microphone (PipeWire) and a GPU driver (Vulkan); to install an engine, the build tools and a checkout of Otto's source; optional, otto-dictate, the dictation in every other field, with its autostart entry and the key `config.toml` binds to it.
- **Review:** the speech server (which engine runs, whether `127.0.0.1:8080` answers), the microphone, the GPU driver, otto-dictate, then the steps and Apply. The steps write `engine` into `dictation.toml` (the key Settings › Dictation edits), enable and start the engine's unit and disable the others, and write otto-dictate's autostart entry. It ends by knocking on the server's port.

Every row opens its instructions: what is there, and always the full steps with the exact commands, those already done marked so.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-agents-client` (the harness presets and packages in `harness.rs`, the instruction sets in `instructions.rs`, both shared with Settings › Agents), `otto-term-kit` (the terminal and the look it shares with `otto-msg doctor`), `otto-foundations` (XDG directories, `tilde_in`, `desktop_file` for the autostart entry).
- **External:** `toml_edit`, so `agents.toml` and `dictation.toml` keep their comments; `libc` for `isatty`. Commands it runs: `systemctl --user` (`show`, `show-environment`, `daemon-reload`, `enable --now`, `disable`, `restart`), `otto-agents plugins install` and `otto-agents doctor`, `hermes profile create`, `node --version`, `otto-agents plugins status`. For dictation it also connects to `127.0.0.1:8080` to see whether the speech server answers.

## Develop
- `cargo run -p otto-setup -- --check` prints what is there and what is left, without a terminal interface; it exits 1 when something is left to do.
- The interface needs a terminal on stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`). Apply acts on the real user manager: with a fake `HOME` the files land in the fake home, but `systemctl --user` still reaches the real units.
- Tests: `cargo test -p otto-setup`. The plan is tested against made-up machines (`plan.rs`), the readers against `systemctl` output and agent files (`probe.rs`).
- On a clean machine: `scripts/test-otto-setup.sh` builds otto-setup and otto-agents, copies them into a clean Debian-family image (`ubuntu:24.04`; `OTTO_SETUP_BASE` picks another) and runs the scenarios in `tests/clean-machine/check.sh` there: nothing installed, the distribution's too-old Node.js, a stand-in `claude` that `--apply` writes down. `screen.py` beside it drives the interface in a pseudo-terminal and prints the screen; `check.sh` greps it. There is no systemd in the image, so the service steps are not run and the check at the end fails, which the apply scenario expects.

## Where things are
- `src/main.rs`: options, and `--check` / `--apply` without the interface.
- `src/probe.rs`: reading the machine into `Facts`: the agent lists and the bridge, where each program is on the shell's PATH and on each unit's, Node's version, the units' state, the Hermes profiles, cc-connect's configuration.
- `src/plan.rs`: readiness, the steps from the facts and the `Choices`, and the `agents.toml` they write. Pure: no files, no processes.
- `src/guide.rs`: the instructions each row opens, marked line by line so the interface can style them: `#you`/`#setup`/`#done` steps (who does it), `$ ` commands (one per line, nothing else on it), `│ ` file contents, the rest prose.
- `src/plugins.rs`: `otto-agents plugins status` read into Otto's skills and each harness's renderings.
- `src/dictation.rs`: the Dictation topic's facts (the engines' files and units, the server's port, PipeWire, the Vulkan driver, the build tools, a source checkout, otto-dictate, its autostart entry and shortcut), choices, and steps; `src/dictation_guide.rs` its instructions.
- `src/apply.rs`: running the steps in order, stopping at the first that fails, then the topic's check.
- `src/tui.rs`: the interface: the topics, and the three pages of the one chosen.

## Rules
- Setup installs nothing. No package manager, no `npm install`, no downloads, no `sudo`: it runs only Otto's own commands and `systemctl --user`. For what is missing it shows the commands in the instructions.
- Where each harness keeps its profile is `otto-agents`' to know: setup reads `otto-agents plugins status`, never the paths themselves.
- What is installed is used as it is: an adapter already on the PATH is what new agents start, rather than `npx`; an existing cc-connect configuration and `[bridge]` are kept.
- The welcome lists the topics (`TOPICS` in `tui.rs`); a new dependency to set up is a new topic with its own pages, not more rows on the agents' pages.
- Nothing is written before Apply, and the preview shows exactly what Apply writes or runs.
- Setup only adds. It never removes or changes an agent the user already has, and never writes over a file it cannot parse. Changing or removing agents is Settings › Agents' job.
- The PATH goes into a drop-in of its own, `<unit>.d/otto-setup.conf`, never `override.conf` (what `systemctl --user edit` writes) or another file of the user's. Its PATH is built on the one the unit has now, so it only adds.
- A new agent is written with the presets in `otto-agents-client`'s `harness.rs`, the same as one added in Settings, with `permissions = "ask"`.
- Dictation's engines are `install.sh`'s to build and place; setup looks where it puts them (`DATA_DIR`, the units' names) and never downloads a model or builds a server. `dictation.toml` gets only its `engine` key, the rest stays; the autostart entry is otto-dictate's own, switched on by `Hidden=false` rather than rewritten, as Settings › Dictation does.

## See also
- User guide [Ask and Agents](../../docs/user/agents.md); [Agents](../../docs/developer/agents.md); [the cc-connect bridge](../otto-agents/docs/cc-connect.md); [spec](../../specs/first-run-setup.md).
- [otto-agents](../otto-agents/AGENTS.md), [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-term-kit](../otto-term-kit/AGENTS.md).
