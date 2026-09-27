# Testing with OpenClaw

How to reach Otto's agents from Telegram through [OpenClaw](https://docs.openclaw.ai),
using `otto-agents acp` as OpenClaw's agent. Plan
[0015](plans/0015-chat-bridges.md) explains the design.

```
Telegram ──► OpenClaw gateway ──(acpx, ACP over stdio)──► otto-agents acp ──(AHP)──► otto-agents ──► Claude Code
```

The OpenClaw side follows its docs as of 2026-09-27 and **has not been run
against Otto yet**. The steps marked *check* are the ones its docs leave open.
The Otto side (`otto-agents acp`, the chat bridge unit and its Settings switch)
is tested.

## What you need

- **otto-agents with `acp`.** A build of branch `worktree-agents-channels-plan`
  or later. `otto-agents acp --help` should list `--permissions`.
- **otto-agents running.** `systemctl --user status otto-agents`.
- **Node.js 24.16+ or 26.1+.** OpenClaw refuses anything older. Install a new
  Node next to the one your agents use, and leave that one alone:
  ```sh
  nvm install 24      # or: nvm install 26
  node --version      # 24.16 or later
  ```
  Your agents' unit PATH still names the old Node, so nothing else changes.
- **A Telegram account,** to create a bot and talk to it.

## 1. An agent for remote use

Messages from a phone should get less reach than you get at the desk. Add an
agent for them to `~/.config/otto/agents.toml`, then restart the service
(`systemctl --user restart otto-agents`, or Apply in Settings › Agents):

```toml
[[agents]]
id = "remote"
name = "Otto (remote)"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
model = "haiku"
permissions = "ask"            # every tool call is asked on the desktop
folder = "~/remote"            # the only folder it can reach
skills = "claude"
agent = "otto"
```

`mkdir -p ~/remote`. To start with, you can use your existing `claude` agent
instead; it is what the steps below call `--agent remote`.

## 2. Install OpenClaw

```sh
npm install -g openclaw@latest --allow-scripts=openclaw
openclaw --version
```

Then onboard. Onboarding writes `~/.openclaw/openclaw.json`, including
`gateway.mode = "local"`, without which the gateway will not start:

```sh
openclaw onboard
```

It asks for a model provider for OpenClaw's own `main` agent. That agent
answers messages that are not bound to Otto; any provider works, since the
test binds the chat to Otto.

## 3. A Telegram bot

1. In Telegram, open **@BotFather**, send `/newbot`, and pick a name. It
   replies with a token such as `123456:ABC…`.
2. Give it to OpenClaw:
   ```sh
   openclaw channels add --channel telegram --token '123456:ABC…'
   ```
   DMs default to `dmPolicy: "pairing"`: strangers get a code and nothing else.

## 4. Otto as OpenClaw's ACP agent

Install the ACP backend:

```sh
openclaw plugins install @openclaw/acpx
openclaw config set plugins.entries.acpx.enabled true
```

Then edit `~/.openclaw/openclaw.json` (JSON5; the gateway reloads it) and merge
in:

```json5
{
  acp: {
    enabled: true,
    dispatch: { enabled: true },
    backend: "acpx",
    defaultAgent: "otto",
    allowedAgents: ["otto"],
    stream: { deliveryMode: "live" },
  },
  plugins: {
    entries: {
      acpx: {
        enabled: true,
        config: {
          probeAgent: "otto",
          // acpx never asks anyone: it answers permission requests by
          // policy. `--permissions desktop` keeps them away from it, so
          // they reach you as Otto's own dialog instead.
          nonInteractivePermissions: "deny",
          agents: {
            otto: {
              command: "/usr/local/bin/otto-agents",   // `which otto-agents`
              args: ["acp", "--agent", "remote", "--permissions", "desktop"],
            },
          },
        },
      },
    },
  },
}
```

- **The full path** for `command`: the gateway's PATH may not have the
  directory `otto-agents` is in.
- **Check** that OpenClaw takes a new agent id. Its docs only show overriding a
  built-in one. If `/acp doctor` or the gateway log rejects `otto`, rename the
  entry to `claude` everywhere above (`defaultAgent`, `allowedAgents`,
  `probeAgent` and the key under `agents`).

## 5. Run it

In a terminal, in the foreground so you can watch it:

```sh
openclaw gateway
```

and in another:

```sh
openclaw channels status --probe      # telegram: connected
journalctl --user -u otto-agents -f   # Otto's side
```

## 6. Pair, bind, talk

1. **Pair.** Send your bot any message. It replies with a code. Approve it:
   ```sh
   openclaw pairing list telegram
   openclaw pairing approve telegram <CODE>
   ```
   This also makes you the owner, which the `/acp` commands need.
2. **Bind the chat to Otto.** In the DM with the bot:
   ```
   /acp spawn otto --bind here --cwd /home/<you>/remote
   /acp status
   ```
   `otto-agents acp` starts, and a new session appears on the desktop:
   `otto-agents sessions` lists it with folder `~/remote`. Its first run can
   take a minute while `npx` fetches the harness.
   **Check:** OpenClaw's docs don't say whether a Telegram *DM* can be bound
   (persistent bindings are documented for forum topics). If `--bind here` is
   refused, make a group with Topics on, add the bot, and bind a topic
   instead, either with the same command in the topic or in the config:
   ```json5
   bindings: [
     { type: "acp", agentId: "otto",
       match: { channel: "telegram", accountId: "default",
                peer: { kind: "group", id: "-100…:topic:<n>" } },
       acp: { cwd: "/home/<you>/remote", mode: "persistent" } },
   ]
   ```
3. **Talk.** Send "what's in this folder?". The answer streams back into
   Telegram, and the same conversation shows in Otto's Sessions.
4. **A permission request.** Ask for something that needs one, such as "create
   notes.md with a haiku in it". Otto's own dialog asks on the desktop. Because
   the bridge is watching the session, it appears after 20 seconds rather than
   at once. Answer it there; the reply carries on in Telegram. Nobody at the
   desk means the request waits, and `/acp cancel` gives up the turn.
5. **Stop and close.** `/acp cancel` stops a turn. `/acp close` ends the ACP
   session. The Otto session stays, in Sessions and on disk.
6. **Carry on at the desk.** `otto-agents enter <id>` opens the same
   conversation in a terminal, and Ask's Sessions shows it too.

## 7. Start it from Settings

Once it works in the foreground, let Otto run it. In Settings › Agents ›
**Chat bridge**:

1. **Command:** the gateway with the full path to the new Node's `openclaw`,
   for example
   `/home/<you>/.config/nvm/versions/node/v24.x.y/bin/openclaw gateway`.
   Apply.
2. **Bridge:** turn it on. It enables `otto-agents-bridge`, which starts at
   every login and shows as *Running*. Its log is
   `journalctl --user -u otto-agents-bridge -f`.

Don't also run `openclaw gateway install`, OpenClaw's own service: two gateways
on one port and one bot token fight.

## Troubleshooting

| What you see | Where to look |
|---|---|
| The gateway won't start | `gateway.mode` unset: rerun `openclaw onboard` |
| `/acp spawn` fails | `/acp doctor`; `allowedAgents` must name `otto`; the `command` path |
| The session never starts | `journalctl --user -u otto-agents`: the agent's own start (npx, login) |
| No reply, no error | `otto-agents show <id> --follow` shows what the agent is doing |
| A tool call is refused at once | the agent's `permissions` is `deny`, or the islands dialog is unavailable |
| Bridge switch goes back off | no command saved, or the unit is not installed (`otto-agents-bridge.service`) |
| `SESSION_OWNER_MIGRATION_REQUIRED` | `openclaw doctor --fix` |

## What the bridge can't do yet

- **Questions the agent asks** (forms) are answered on the desktop only.
- **Pictures and voice notes** from Telegram are not passed to the agent.
- **Picking up an existing Otto session** from Telegram: `otto-agents acp`
  supports `session/load` by id or prefix, but `/acp spawn` always starts a new
  session. OpenClaw's `sessions_spawn` tool takes a `resumeSessionId`.

## Cleaning up

```sh
# Stop the bridge: Settings › Agents › Chat bridge off, or
systemctl --user disable --now otto-agents-bridge
npm uninstall -g openclaw && rm -rf ~/.openclaw   # config, pairing and all
# and /deletebot in @BotFather, so the token is worth nothing
```

Otto's sessions from the test stay until you forget them:
`otto-agents forget <id>`.

## Sources

- OpenClaw: [getting started](https://docs.openclaw.ai/start/getting-started),
  [configuration](https://docs.openclaw.ai/gateway/configuration),
  [Telegram](https://docs.openclaw.ai/channels/telegram/setup),
  [access control](https://docs.openclaw.ai/channels/telegram/access-control),
  [ACP agents setup](https://docs.openclaw.ai/tools/acp-agents-setup),
  [bindings](https://docs.openclaw.ai/tools/acp-agents/bindings),
  [acpx custom agents](https://github.com/openclaw/acpx/blob/main/docs/custom-agents.md)
