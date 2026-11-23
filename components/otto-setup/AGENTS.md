# otto-setup

`otto-setup` configures Otto the first time, from a terminal. It bootstraps the requirements; Settings has the detailed configuration. A welcome says that and asks what to set up; for now the one choice is the Otto Agents, which answer in Ask and run Sessions, and it opens three pages:

- **Agents:** first *Sessions run in*: herdr or Otto, what `otto-agents` would use (`backend` in `agents.toml`, else herdr when `hc` is found). Space switches it; the choice is written as `backend` only when it differs from what Automatic gives, and a `backend` the user already set is shown, not changed. Then the harnesses (Claude Code, Codex, OpenCode, Hermes, pi), each shown as in the agent list, ready, what it is missing, or, in herdr, that it runs only in Otto (Codex, Hermes: not tickable). Then two boxes for Otto's plugin: Otto (the new agents answer as Otto, or are the plain harness) and Studio (Studio's chat, which needs a Claude Code agent).
- **Requirements:** in herdr, herdr (`hc`, `herdr-app`) and the agents' own programs (`claude`, `pi`, `opencode`, and `sqlite3` with OpenCode); in Otto, Node.js and the ACP adapters of the agents there are or are being added. Each with where it was found.
- **Review:** checks for the agents selected (the requirements above, Otto's skills, each harness's profile of Otto's agent, none in herdr), then the steps and Apply. The steps write the new agents (and `backend`, when chosen) into `agents.toml`, add the folders the programs the service runs live in to the unit's PATH, create the Hermes profile, link Otto's skills (`otto-agents plugins install`), and start or restart `otto-agents.service`. It ends with `otto-agents doctor`.

herdr is not a topic of its own: it decides how the agents run, so it is a choice on the Agents page and a requirement on the next, as Node.js is in Otto.

Every row opens its instructions: what is there, and always the full steps with the exact commands, those already done marked so.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-agents-client` (the harness presets and packages in `harness.rs`, the instruction sets in `instructions.rs`, both shared with Settings › Agents), `otto-term-kit` (the terminal and the look it shares with `otto-msg doctor`), `otto-foundations` (XDG directories, `tilde_in`).
- **External:** `toml_edit`, so `agents.toml` keeps its comments; `libc` for `isatty`. Commands it runs: `systemctl --user` (`show`, `show-environment`, `daemon-reload`, `enable --now`, `restart`), `otto-agents plugins install` and `otto-agents doctor`, `hermes profile create`, `node --version`, `otto-agents plugins status`. It only looks for herdr's `hc` and `herdr-app`, never runs them.

## Develop
- `cargo run -p otto-setup -- --check` prints what is there and what is left, without a terminal interface; it exits 1 when something is left to do.
- The interface needs a terminal on stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`). Apply acts on the real user manager: with a fake `HOME` the files land in the fake home, but `systemctl --user` still reaches the real units.
- Tests: `cargo test -p otto-setup`. The plan is tested against made-up machines (`plan.rs`), the readers against `systemctl` output and agent files (`probe.rs`).
- On a clean machine: `scripts/test-otto-setup.sh` builds otto-setup and otto-agents, copies them into a clean Debian-family image (`ubuntu:24.04`; `OTTO_SETUP_BASE` picks another) and runs the scenarios in `tests/clean-machine/check.sh` there: nothing installed, the distribution's too-old Node.js, a stand-in `claude` that `--apply` writes down, and stand-ins for `hc`, `herdr-app` and `claude` in a home of their own, where the sessions run in herdr. `screen.py` beside it drives the interface in a pseudo-terminal and prints the screen; `check.sh` greps it. There is no systemd in the image, so the service steps are not run and the check at the end fails, which the apply scenario expects.

## Where things are
- `src/main.rs`: options, and `--check` / `--apply` without the interface.
- `src/probe.rs`: reading the machine into `Facts`: the agent lists and `backend`, where each program is on the shell's PATH and on the unit's, Node's version, the unit's state, the Hermes profiles.
- `src/plan.rs`: the backend (`automatic`, `effective`, `writes_backend`), readiness, the steps from the facts and the `Choices`, and the `agents.toml` they write. Pure: no files, no processes.
- `src/guide.rs`: the instructions each row opens, marked line by line so the interface can style them: `#you`/`#setup`/`#done` steps (who does it), `$ ` commands (one per line, nothing else on it), `│ ` file contents, the rest prose.
- `src/plugins.rs`: `otto-agents plugins status` read into Otto's skills and each harness's renderings.
- `src/apply.rs`: running the steps in order, stopping at the first that fails, then the check.
- `src/tui.rs`: the interface.

## Rules
- Setup installs nothing. No package manager, no `npm install`, no downloads, no `sudo`: it runs only Otto's own commands and `systemctl --user`. For what is missing it shows the commands in the instructions.
- Where each harness keeps its profile is `otto-agents`' to know: setup reads `otto-agents plugins status`, never the paths themselves.
- What is installed is used as it is: an adapter already on the PATH is what new agents start, rather than `npx`.
- The welcome lists the topics (`TOPICS` in `tui.rs`); a new dependency to set up is a new topic with its own pages, not more rows on the agents' pages.
- Nothing is written before Apply, and the preview shows exactly what Apply writes or runs.
- Setup only adds. It never removes or changes an agent the user already has, and never writes over a file it cannot parse. Changing or removing agents is Settings › Agents' job.
- The PATH goes into a drop-in of its own, `<unit>.d/otto-setup.conf`, never `override.conf` (what `systemctl --user edit` writes) or another file of the user's. Its PATH is built on the one the unit has now, so it only adds.
- A new agent is written with the presets in `otto-agents-client`'s `harness.rs`, the same as one added in Settings, with `permissions = "ask"`. In herdr its command is the harness's own program (`herdr_program`) with no args, and `write_herdr` writes the rest, without enter lines.
- What runs the sessions is decided as `otto-agents` decides it (`backend`, else herdr when `hc` is found). In herdr the agents' programs are looked for on the shell's PATH, which is close to the login-shell PATH herdr starts them with; only what the service runs itself (`hc`, `herdr-app`, `opencode`, `sqlite3`) goes on the unit's PATH.

## See also
- User guide [Ask and Agents](../../docs/user/agents.md); [Agents](../../docs/developer/agents.md); [spec](../../specs/first-run-setup.md).
- [otto-agents](../otto-agents/AGENTS.md), [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-term-kit](../otto-term-kit/AGENTS.md).
