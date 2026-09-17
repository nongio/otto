# Plans

One file per milestone or significant design decision, named `NNNN-short-title.md`.
Each plan starts with a status line:

- **Draft**: being shaped; open questions remain
- **Accepted**: agreed; not started
- **In progress**: being implemented
- **Done**: implemented; kept as a record of the decisions
- **Superseded by NNNN**: replaced by a later plan

Update the status as work lands. A plan records intent and the reasons for decisions;
the code and the tests remain the source of truth for behaviour.

## Architecture in one line

otto-agentsd is the agents service: it runs ACP agents and serves them over AHP. The launcher is the GUI, and every Otto surface is an AHP client (see 0003 and 0007).

| # | Plan | Status | Depends on |
|---|---|---|---|
| 0001 | [Repository and spec setup](0001-repository-and-spec-setup.md) | Done | |
| 0002 | [Core protocol loop](0002-core-protocol-loop.md) | Draft | 0001 |
| 0003 | [Agent backend (ACP)](0003-acp-agent-backend.md) | Draft | 0002 |
| 0004 | [Configuration](0004-agent-configuration.md) | Draft | 0003 |
| 0005 | [Authentication](0005-authentication.md) | Draft | 0002 |
| 0006 | [Spawning tasks to agents](0006-task-spawning.md) | Draft | 0003, 0004, 0005 |
| 0007 | [Otto desktop integration](0007-otto-desktop-integration.md) | Draft | 0003, 0005 |
| 0008 | [Otto usage instructions for agents](0008-otto-usage-instructions.md) | Draft | 0003, 0004 |
| 0009 | [otto-agents app](0009-otto-agents-app.md) | Superseded by 0011 | 0002, 0004, 0005, 0007 |
| 0010 | [Proof of concept](0010-poc.md) | In progress | 0001 |
| 0011 | [Launcher ask, second version](0011-launcher-ask-v2.md) | In progress | 0010 |
