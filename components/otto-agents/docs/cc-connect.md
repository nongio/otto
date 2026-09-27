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
- **Permission requests reach the chat and the desktop.** A tool call that
  needs approval is asked in Telegram at once, and as Otto's dialog 20 seconds
  later (sooner nobody watches). Whichever you answer first wins.
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
# `client`: permission requests go to the chat as well as the desktop.
args = ["acp", "--agent", "claude", "--permissions", "client"]
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
   `create notes.txt with a haiku in it`. Telegram asks; answer there, or on
   the desktop once Otto's dialog appears. The otto-agents journal says who
   answered (`from_client=true` for the chat).
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

## 6. What the phone may do without asking

Every tool call that needs permission is a question on the phone. Claude reads
its permission rules from the folder the session works in, so the bridge's
folder can allow the few that sending a screenshot takes, and keep out skills
meant for the desk. `~/.local/state/otto/remote/.claude/settings.json`:

```json
{
  "permissions": {
    "allow": [
      "Skill(otto:send-to-phone)",
      "Skill(otto:otto-help)",
      "Bash(/usr/share/otto/plugins/otto/skills/send-to-phone/scripts/screenshot:*)",
      "Bash(cc-connect send:*)",
      "Read(~/Pictures/Screenshots/**)"
    ],
    "deny": []
  }
}
```

- **A skill's own `allowed-tools` is not enough.** Through ACP, Claude still
  asks for a command the skill lists; the rules above are what it honours.
- **The screenshot is one command.** The skill's `scripts/screenshot` brings
  the app forward, captures and prints the file, so one rule covers it; `grim`
  chained with `mkdir` and `sleep` would ask every time.
- **Skills of your own** in `~/.claude/skills` reach the bridge's agent too.
  One that also talks about screenshots can win over `send-to-phone`: list it
  under `deny` as `Skill(<name>)`.
- `cc-connect send` goes to your own chat, but anything it sends leaves the
  computer. Leave it out of `allow` to be asked each time.

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
- **cc-connect's session commands** (`/list`, `/switch`) were not tried with
  Otto; `otto-agents acp` doesn't serve `session/list`.

## Cleaning up

```sh
# Settings › Agents › Chat bridge off, or:
systemctl --user disable --now otto-agents-bridge
rm -rf ~/.cc-connect ~/.local/bin/cc-connect
# and /deletebot in @BotFather, so the token is worth nothing
```

Sessions from the chat stay until you forget them: `otto-agents forget <id>`.
