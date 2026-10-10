# Chat apps with cc-connect

How to reach Otto's agents from a chat app through
[cc-connect](https://github.com/chenhg5/cc-connect), with `otto-agents acp` as
its agent. Telegram is shown; cc-connect also speaks Matrix, Slack, Discord and
nine more. Plan [0015](plans/0015-chat-bridges.md) explains the design.

```
Telegram ──► cc-connect ──(ACP over stdio)──► otto-agents acp ──(AHP)──► otto-agents ──► the agent
```

Tried on 2026-09-27 with cc-connect v1.5.0 and a Telegram bot: messages became
a desktop session, replies came back in seconds, and a file edit was approved
from the phone.

## What happens

- **One session per chat.** Your first message starts a desktop session in the
  bridge's folder. Later messages from the same chat go to that session. It
  shows in Sessions, and `otto-agents enter <id>` picks it up at the desk.
- **Permission requests are answered at the desk.** By default
  (`--permissions desktop`) a tool call that needs approval is asked only as
  Otto's dialog on the desktop. With `--permissions client` it is also asked
  in Telegram at once, and as Otto's dialog 20 seconds later (sooner nobody
  watches); whichever you answer first wins. Turn that on only once
  `allow_from` is right: see [Who can reach the agents](#who-can-reach-the-agents).
- **"Always" means always.** An option such as Claude's "Always allow" reaches
  the chat as an always option, not as a one-off, so a bridge that answers by
  a policy can tell the two apart.
- **Links sent from the chat are not attached.** A file attached at the desk
  is something the agent may read without asking; one named in a chat message
  is not attached, so reading it is a tool call like any other and is asked.
- **The agent knows you are on your phone.** `otto-agents acp` marks each
  message as written in the chat app (`--remote Telegram`; under cc-connect
  the platform is taken from `CC_SESSION_KEY`), and the agent is told so ahead
  of it. Otto's agent then replies briefly, asks in the message rather than
  with a dialog, and offers to send what it finds. The desktop's transcript
  shows the message as written.
- **Files and pictures go back with `cc-connect send`.** Replies reach the chat
  as text only; cc-connect drops anything else an agent sends over ACP. Otto's
  `send-to-phone` skill tells the agent to run
  `cc-connect send --image|--file <path>`, and how to take a screenshot for it.
- **The folder is not a fence.** The agent works in the bridge's folder but
  can reach elsewhere when you approve it. The permission prompt is the guard,
  so a bridge's agent should have `permissions = "ask"`.

## 1. Install cc-connect

A single binary from its releases, checked against the published checksums:

```sh
cd /tmp
v=v1.5.0   # https://github.com/chenhg5/cc-connect/releases
gh release download $v -R chenhg5/cc-connect -p "cc-connect-$v-linux-amd64.tar.gz" -p checksums.txt
grep linux-amd64.tar.gz checksums.txt | sha256sum -c -
tar -xzf cc-connect-$v-linux-amd64.tar.gz
install -m755 cc-connect-$v-linux-amd64 ~/.local/bin/cc-connect
cc-connect --version
```

(`npm install -g cc-connect` works too.) Its repository carries no licence: use
it as a tool, don't copy its code into Otto.

## 2. A Telegram bot

1. In Telegram, open **@BotFather**, send `/newbot`, and pick a name and a
   username ending in `bot`. It replies with a token such as `123456789:AAH…`.
2. Your numeric user ID: send the new bot any message, then
   ```sh
   curl -s "https://api.telegram.org/bot<TOKEN>/getUpdates" | grep -o '"from":{"id":[0-9]*'
   ```
   or ask **@userinfobot**.

## 3. Configure

`~/.cc-connect/config.toml`, mode 0600 (it holds the token):

```toml
language = "en"

[log]
level = "info"

[[projects]]
name = "otto"
# No admin_from: cc-connect's own /dir, /shell, /restart, /upgrade and the exec
# forms of /commands and /cron are refused for everyone. They are also turned
# off, so the bot's only reach is the Otto agent and its permissions.
disabled_commands = ["shell", "dir", "cd", "restart", "upgrade", "cron", "commands"]

[projects.agent]
type = "acp"

[projects.agent.options]
work_dir = "/home/you/.local/state/otto/remote"
cmd = "/usr/local/bin/otto-agents"          # `which otto-agents`
# Permission requests are answered on the desktop. Add "--permissions",
# "client" to answer them from the chat too (see "Who can reach the agents").
args = ["acp", "--agent", "claude"]
display_name = "Otto"

[[projects.platforms]]
type = "telegram"

[projects.platforms.options]
token = "123456789:AAH…"
allow_from = "123456789"                     # your user ID; nobody else
```

```sh
chmod 700 ~/.cc-connect && chmod 600 ~/.cc-connect/config.toml
mkdir -p ~/.local/state/otto/remote
```

- **`--agent`** names an `[[agents]]` entry in `agents.toml`. A separate entry
  for the bridge (its own `folder`, a cheaper `model`, `permissions = "ask"`)
  keeps what a phone can do apart from what the desk can.
- **`allow_from`** is what keeps the bot yours. Leave `"*"` out.

## 4. Try it

```sh
cc-connect --config ~/.cc-connect/config.toml
```

The log says `telegram: connected`. Then, in the chat with the bot:

1. **A question,** such as `who are you?`. The first reply takes a few seconds
   while the agent starts. `otto-agents sessions` lists the new session in
   `~/.local/state/otto/remote`.
2. **Something that needs permission,** such as
   `create notes.txt with a haiku in it`. Otto's dialog asks on the desktop.
   With `--permissions client`, Telegram asks too; answer there, or on the
   desktop once the dialog appears. The otto-agents journal says who answered
   (`from_client=true` for the chat).
3. **At the desk:** `otto-agents show <id>` prints the conversation;
   `otto-agents enter <id>` carries it on in a terminal.

## 5. Start it from Settings

In Settings › Agents › **Chat bridge**:

1. **Command:**
   `/home/you/.local/bin/cc-connect --config /home/you/.cc-connect/config.toml`,
   full paths, since the unit has its own PATH. Apply.
2. **Bridge:** turn it on. It enables `otto-agents-bridge`, which starts at
   every login and shows *Running*. Its log is
   `journalctl --user -u otto-agents-bridge -f`.

Stop any cc-connect you started by hand first: two copies with one config
refuse to run (`--force` would kill the other), and two pollers on one bot
token fight. Don't also use `cc-connect daemon install`, its own service.

## Who can reach the agents

Everyone cc-connect lets in speaks to the agent as you do, so the bridge is
only as private as its configuration:

- **Who may write.** Only the accounts in each platform's `allow_from`. Anyone
  who can post as one of them (a stolen phone, a shared Telegram login, a
  group the bot was added to with that member in it) reaches the agent.
  Without `admin_from` and with the commands in `disabled_commands` off,
  cc-connect itself runs nothing for them; the agent does.
- **Which sessions.** Not only the chat's own: `/list` shows the desktop's
  sessions in the bridge's folder, and `/switch <id>` (`session/load` or
  `session/resume`, by the start of an id) takes up **any** desktop session,
  wherever it works, with whatever its agent was already allowed in it, such
  as "always allow" rules given at the desk. That is deliberate, so a
  conversation begun at the desk can be carried on from the phone, and it is
  not limited yet. Keep `allow_from` to yourself.
- **Who approves.** By default only the person at the desk: a tool call that
  needs permission waits for Otto's dialog. With `--permissions client`,
  everyone in `allow_from` can approve it from the chat, an "always" option
  included, which adds a standing rule to that agent.
- **What runs without asking.** Whatever the agent's own permission rules and
  the loaded skills pre-approve (see below), the same as at the desk. Files
  named in a chat message are not attached, so reading one is asked.

## 6. What the phone may do without asking

With `--permissions client`, every tool call that needs permission is a
question on the phone. Three things keep those few:

- **Otto's own skills load without asking.** otto-agents allows
  `Skill(<plugin>:<skill>)` for every skill of the desktop's plugins when it
  starts a Claude session.
- **A skill pre-approves what it runs.** While `send-to-phone` is loaded, its
  `allowed-tools` let `scripts/screenshot` and `cc-connect send` run without a
  question. The screenshot is one script, rather than `grim` chained with
  `mkdir` and `sleep`, so that one rule covers it.
- **Your own rules** in `~/.claude/settings.json` apply as they do at the desk.

A `.claude/settings.json` in the bridge's folder does **not** work: Claude
leaves out the permission rules of a folder it has not been told to trust.

**Skills of your own** in `~/.claude/skills` reach the bridge's agent too, and
one that also talks about screenshots can win over `send-to-phone`. Give it a
description narrow enough not to match, or move it into a project's
`.claude/skills` where only that project sees it.

`cc-connect send` goes to your own chat, but anything it sends leaves the
computer: the skill tells the agent to send only what was asked for.

## 7. In the chat

- **Replies only.** cc-connect sends every tool call and every thought as a
  message of its own by default. Set
  ```toml
  [display]
  mode = "compact"
  ```
  above `[[projects]]` to send only the agent's words; the desktop's Sessions
  still shows all of it.
- **Sessions.** `/list` shows the desktop's sessions in the bridge's folder,
  `/switch <id>` carries one on (the start of an id will do), `/new` starts
  another, and `/stop` stops the agent's turn.
- **Modes.** `/mode` lists the agent's modes, such as Claude's "Accept
  edits", and `/mode <name>` switches; the desktop's session switches with
  it.
- **The session list on the desktop** shows a pill with the chat app on every
  session written to from the phone.

## Troubleshooting

| What you see | Where to look |
|---|---|
| No reply at all | cc-connect's log: `message received` means Telegram reached it; `session spawned` means `otto-agents acp` started |
| `session spawned` but no answer | `journalctl --user -u otto-agents`: the agent's own start (npx, login) |
| A tool call is refused at once | the agent's `permissions` is `deny`, or it was asked outside a turn |
| Messages from someone else are ignored | as meant: `allow_from` |
| The Bridge switch goes back off | no command saved, or `otto-agents-bridge.service` is not installed |

## Not yet

- **Questions the agent asks** (forms) are answered on the desktop only.
- **Pictures and files sent from the chat** are saved by cc-connect under the
  bridge's folder and named in the prompt; not tried with Otto yet.
- **A screenshot of one window:** Otto can't crop to a window yet, so the skill
  brings it forward and captures the whole monitor.
- **`/model`, `/allow`, `/usage` and `/reasoning`** belong to cc-connect's own
  Claude Code and Codex adapters, not to ACP, and do nothing here. The agent's
  model is set in `agents.toml`.

## Cleaning up

```sh
# Settings › Agents › Chat bridge off, or:
systemctl --user disable --now otto-agents-bridge
rm -rf ~/.cc-connect ~/.local/bin/cc-connect
# and /deletebot in @BotFather, so the token is worth nothing
```

Sessions from the chat stay until you forget them: `otto-agents forget <id>`.
