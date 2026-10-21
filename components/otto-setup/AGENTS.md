# otto-setup

`otto-setup` configures Otto the first time, from a terminal. It bootstraps the requirements; Settings has the detailed configuration. A welcome says that and asks what to set up; for now the one choice is the Otto Agents, which answer in Ask and run Sessions, and it opens three pages:

- **Agents:** the harnesses (Claude Code, Codex, OpenCode, Hermes, pi), each shown as in the agent list, ready, or what it is missing, and two boxes for Otto's plugin: Otto (the new agents answer as Otto, or are the plain harness) and Studio (Studio's chat, which needs a Claude Code agent).
- **Requirements:** required, Node.js and the ACP adapters of the agents there are or are being added; optional, the chat bridge (cc-connect), to message the agents from Telegram. Each with where it was found.
- **Review:** checks for the agents selected (Node.js, their ACP adapters, cc-connect, Otto's skills, each harness's profile of Otto's agent), then the steps and Apply. The steps write the new agents into `agents.toml`, add the folders the programs live in to each unit's PATH, create the Hermes profile, link Otto's skills (`otto-agents plugins install`), and start or restart `otto-agents.service` and, when ticked, `otto-agents-bridge.service`. It ends with `otto-agents doctor`.

Every row opens its instructions: what is there, and always the full steps with the exact commands, those already done marked so.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-agents-client` (the harness presets and packages in `harness.rs`, the instruction sets in `instructions.rs`, both shared with Settings › Agents), `otto-term-kit` (the terminal and the look it shares with `otto-msg doctor`), `otto-foundations` (XDG directories, `tilde_in`).
- **External:** `toml_edit`, so `agents.toml` keeps its comments; `libc` for `isatty`. Commands it runs: `systemctl --user` (`show`, `show-environment`, `daemon-reload`, `enable --now`, `restart`), `otto-agents plugins install` and `otto-agents doctor`, `hermes profile create`, `node --version`, `otto-agents plugins status`.

## Develop
- `cargo run -p otto-setup -- --check` prints what is there and what is left, without a terminal interface; it exits 1 when something is left to do.
- The interface needs a terminal on stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`). Apply acts on the real user manager: with a fake `HOME` the files land in the fake home, but `systemctl --user` still reaches the real units.
- Tests: `cargo test -p otto-setup`. The plan is tested against made-up machines (`plan.rs`), the readers against `systemctl` output and agent files (`probe.rs`).

## Where things are
- `src/main.rs`: options, and `--check` / `--apply` without the interface.
- `src/probe.rs`: reading the machine into `Facts`: the agent lists and the bridge, where each program is on the shell's PATH and on each unit's, Node's version, the units' state, the Hermes profiles, cc-connect's configuration.
- `src/plan.rs`: readiness, the steps from the facts and the `Choices`, and the `agents.toml` they write. Pure: no files, no processes.
- `src/guide.rs`: the instructions each row opens, marked line by line so the interface can style them: `#you`/`#setup`/`#done` steps (who does it), `$ ` commands (one per line, nothing else on it), `│ ` file contents, the rest prose.
- `src/plugins.rs`: `otto-agents plugins status` read into Otto's skills and each harness's renderings.
- `src/apply.rs`: running the steps in order, stopping at the first that fails, then the check.
- `src/tui.rs`: the interface.

## Rules
- Setup installs nothing. No package manager, no `npm install`, no downloads, no `sudo`: it runs only Otto's own commands and `systemctl --user`. For what is missing it shows the commands in the instructions.
- Where each harness keeps its profile is `otto-agents`' to know: setup reads `otto-agents plugins status`, never the paths themselves.
- What is installed is used as it is: an adapter already on the PATH is what new agents start, rather than `npx`; an existing cc-connect configuration and `[bridge]` are kept.
- The welcome lists the topics (`TOPICS` in `tui.rs`); a new dependency to set up is a new topic with its own pages, not more rows on the agents' pages.
- Nothing is written before Apply, and the preview shows exactly what Apply writes or runs.
- Setup only adds. It never removes or changes an agent the user already has, and never writes over a file it cannot parse. Changing or removing agents is Settings › Agents' job.
- The PATH goes into a drop-in of its own, `<unit>.d/otto-setup.conf`, never `override.conf` (what `systemctl --user edit` writes) or another file of the user's. Its PATH is built on the one the unit has now, so it only adds.
- A new agent is written with the presets in `otto-agents-client`'s `harness.rs`, the same as one added in Settings, with `permissions = "ask"`.

## See also
- User guide [Ask and Agents](../../docs/user/agents.md); [Agents](../../docs/developer/agents.md); [the cc-connect bridge](../otto-agents/docs/cc-connect.md); [spec](../../specs/first-run-setup.md).
- [otto-agents](../otto-agents/AGENTS.md), [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-term-kit](../otto-term-kit/AGENTS.md).
