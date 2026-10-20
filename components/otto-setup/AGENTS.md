# otto-setup

`otto-setup` sets Otto up the first time, from a terminal. It starts with the agents: it finds the harnesses that are installed (Claude Code, Codex, OpenCode, Hermes, pi), lets the user tick the ones Otto should run, and then writes them into `agents.toml`, gives the agent service the PATH they live on, creates the Hermes profile, links Otto's skills (`otto-agents plugins install`) and starts or restarts `otto-agents.service`. Each step can be previewed exactly as it will write or run it, nothing changes until Apply, and it ends with `otto-agents doctor`.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-agents-client` (the harness presets, `harness.rs`, shared with Settings › Agents so both write the same block), `otto-term-kit` (the terminal and the look it shares with `otto-msg doctor`), `otto-foundations` (XDG directories, `tilde_in`).
- **External:** `toml_edit`, so `agents.toml` keeps its comments; `libc` for `isatty`. Commands: `systemctl --user` (`show`, `show-environment`, `daemon-reload`, `enable --now`, `restart`), `otto-agents plugins install` and `otto-agents doctor`, `hermes profile create`.

## Develop
- `cargo run -p otto-setup -- --check` prints what is set up and what is left, without a terminal interface; it exits 1 when something is left to do.
- The interface needs a terminal on stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`). Apply acts on the real user manager: with a fake `HOME` the files land in the fake home, but `systemctl --user` still reaches the real `otto-agents.service`.
- Tests: `cargo test -p otto-setup`. The plan is tested against made-up machines (`plan.rs`), the readers against `systemctl` output (`probe.rs`).

## Where things are
- `src/main.rs`: options, and `--check` / `--apply` without the interface.
- `src/probe.rs`: reading the machine into `Facts`: the agent lists, where each program is on the shell's PATH and on the service's, the unit's state.
- `src/plan.rs`: the steps from the facts and the picks, and the `agents.toml` they write. Pure: no files, no processes.
- `src/apply.rs`: running the steps in order, stopping at the first that fails, then the check.
- `src/tui.rs`: the interface.

## Rules
- Nothing is written before Apply, and the preview shows exactly what Apply writes or runs.
- Setup only adds. It never removes or changes an agent the user already has, and never writes over a file it cannot parse. Changing or removing agents is Settings › Agents' job.
- The PATH goes into a drop-in of its own, `otto-agents.service.d/otto-setup.conf`, never `override.conf` (what `systemctl --user edit` writes) or another file of the user's. Its PATH is built on the one the unit has now, so it only adds.
- A new agent is written with the presets in `otto-agents-client`'s `harness.rs`, the same as one added in Settings, with `permissions = "ask"`.

## See also
- User guide [Ask and Agents](../../docs/user/agents.md); [Agents](../../docs/developer/agents.md).
- [otto-agents](../otto-agents/AGENTS.md), [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-term-kit](../otto-term-kit/AGENTS.md).
