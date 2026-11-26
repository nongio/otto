# Ask and Agents

Ask puts a coding agent behind a text field on the desktop. You type a request,
the agent works in a folder, and Otto shows what it is doing. It stops to ask
you when it needs permission or has a question.

![Ask answering a question about moving windows between workspaces, then offering a list of workspaces to choose from](images/ask.gif)

Otto does not ship an agent of its own. It runs the ones you already have
installed and logged in, and answers as **Otto** whichever one is underneath.

There are two ways your sessions can run. With [herdr](https://herdr.dev)
installed, each one is the agent's own program in a herdr terminal, and Ask
shows the same conversation (see [Where sessions run](#where-sessions-run)).
Without it, Otto runs the agents itself. Sessions in herdr are in Otto's
nightly builds; the latest release runs every agent in Otto.

## Before it works

The quickest way is `otto-setup`, in a terminal. It finds what you have
installed, lets you tick the agents Otto should run and pick who they are
(Otto, or the harness itself), and does steps 2 and 3 below for you, plus the
PATH fix under [When something is wrong](#when-something-is-wrong). Its first
row says where sessions run (see [Where sessions run](#where-sessions-run)),
and what it asks for follows: herdr and the agents' own programs, or Node.js
and the ACP adapters. It installs nothing itself: for what is missing it
shows the command to run. Each step shows exactly what it will write or run,
and nothing changes until you apply. `otto-setup --check` only reports what
is left. By hand:

1. **Install an agent** and log it in: Claude Code, OpenCode, Codex, Hermes or
   pi. Otto runs the command you name; it does not manage accounts or keys.
   For sessions in herdr, install herdr too (`hc` and `herdr-app`); there it
   runs Claude Code, pi and OpenCode.
2. **Start the service**, which is off until you ask for it:

   ```sh
   systemctl --user enable --now otto-agents
   ```

3. **Give Otto its knowledge.** This links the desktop's skills where each
   agent looks for them, and writes the Otto instructions in each one's dialect:

   ```sh
   otto-agents plugins install
   otto-agents plugins status    # what it found, and where each copy stands
   ```

4. **Ask something**: run `otto-ask`, or bind it to a key in your config. In
   the launcher, the field says who the request is going to: "Ask @Otto…",
   your default agent. `Down` lists the others and `Return` takes the
   highlighted one. `Ctrl+L` switches between your sessions and a fresh
   request, and `Ctrl+O` continues the session in a terminal (in herdr, it
   shows the session's tab).
   `otto-launcher --agents` opens straight on the list of sessions you have
   going.

   ![The Ask field listing four configured agents, the highlight moving down them, and the field changing to the one picked](images/ask-agents.gif)

![An answer in the Ask panel: two paragraphs about the dock, a link to the documentation, and the skill it loaded to answer](images/ask-answer.jpg)

Agents are configured in `~/.config/otto/agents.toml`, one `[[agents]]` block
each. The Agents pane in [Settings](settings.md) edits the everyday ones
(default agent, name, harness, permissions, model, folder, colour) and restarts the service
for you. See [Configuration](configuration.md) for where that sits. Two settings
there are worth knowing before you start: `permissions`, which decides how an
agent's requests are answered and is **`deny` unless you set it**, and
`folder`, which says where that agent's sessions start. To put an agent of
your own in that list, see [Add an agent of your own](#add-an-agent-of-your-own).

## What leaves your machine

Otto does not send anything anywhere itself, and collects nothing: there is no
telemetry in the agent service. But it is the thing that hands your request to
an agent, and the agent talks to whoever it is logged into. Worth knowing:

- **Your request, and whatever the agent reads, go to that agent's provider.**
  Ask a question and the text goes to Anthropic, OpenAI, OpenRouter: whoever
  the agent you picked is signed in to. So does the content of any file the
  agent opens while answering. Otto is the messenger, not the sender.
- **The folder is the reach.** Everything under a session's folder is something
  the agent can read, and `permissions` only covers what the agent stops to
  *ask* about. A harness reading a file on its own never reaches that gate.
  Ask starts sessions in a scratch folder, `~/.local/state/otto/ask`, for that
  reason. Give an agent a wider folder with `folder` in `agents.toml` only when
  it needs one, and keep it as narrow as the work allows.
- **The first line of your request names the session**, which means it shows in
  the window title if you continue in a terminal, where other programs you run
  can read it.
- **`npx` fetches from npm.** When Otto runs the agents, the default Claude
  entry and the Codex one run their adapters through `npx`, which downloads
  the package the first time. Point `command` at an installed binary if you
  would rather it did not. herdr runs the agents' own programs, so nothing is
  fetched there.
- **What is written down.** Sessions are listed in
  `~/.local/state/otto-agents/sessions/` (`herdr-sessions/` beside it for
  sessions in herdr), readable only by you. They hold the session's folder,
  its title and its status, not the conversation, which stays with the agent.
  Nothing prunes them yet; delete a session in Ask to remove its record.
- **Pictures an agent sends are kept on disk.** An agent can answer with a
  picture (a screenshot, a diagram) and Ask shows it. The file goes in
  `~/.cache/otto/agents/images`, readable only by you, and the folder is trimmed
  to 128 MB with the oldest going first. Delete it whenever you like; a picture
  whose file has gone shows as its name.
- **What `plugins install` writes.** `~/.agents/skills/<name>` (links),
  `~/.config/opencode/agents/`, `~/.hermes/profiles/`, and renderings under
  `~/.local/share/otto/agents/`. It only ever touches files carrying its own
  marker line, and never creates a directory for a harness you do not have.
  `otto-agents plugins status` lists every one of them. Removal is by hand.

Otto tells the agent it is running on a Wayland desktop and names the skills
available to it. It does not send your hostname, your hardware, your window
titles or screenshots.

## What each agent can do

Not every agent can do everything, and the differences are worth knowing
before you pick one. This is what each was checked to do here, with Otto
running the agents; in herdr it is a little different (see
[Where sessions run](#where-sessions-run)):

| Agent | Otto's knowledge | Answers as Otto | Asks you questions | Modes on `Shift+Tab` | Continue in a terminal | Model from `agents.toml` |
|---|---|---|---|---|---|---|
| **Claude Code** | as a plugin, loaded every session | yes | yes, as a dialog you answer | five: default, accept edits, plan, auto, bypass | yes | yes: `haiku`, `sonnet`, `opus` |
| **Codex** | from `~/.agents/skills` | yes | yes, as a dialog you answer | three: ask for approval, approve for me, full access | yes | yes |
| **Hermes** | from `~/.agents/skills` | yes | in prose, in the conversation | three: default, accept edits, don't ask | yes | no; set it in Hermes' own profile |
| **OpenCode** | from `~/.agents/skills` | yes | in prose, in the conversation | none offered | yes | yes, as `provider/model` |
| **pi** | from `~/.agents/skills` | yes | yes, as a dialog you answer, picking from the answers it offers | six, but they set reasoning effort, not permissions | yes | yes, as `provider/model` |

A few notes on the awkward cells:

- **Asks you questions.** An agent that can ask properly gets a dialog with the
  choices in it, and Otto waits. One that cannot just writes the question into
  the conversation and carries on guessing, so you only see it when you look.
  Codex needs its plan mode for this, which Otto turns on for you. pi gets a
  `question` tool from `otto-agents plugins install`; it can offer answers to
  pick from, but not a box to type your own.

  You do not have to keep Ask open waiting for it. Close the window and the
  request carries on; when the agent needs you, the question arrives on the
  island, with the same choices and a way back into the conversation.

  ![A request typed into Ask, the window closed, and the question arriving later on the island with three choices and a Send button](images/ask-island.gif)
- **Modes** are the agent's own presets for what it may do without asking, and
  every agent names them differently. `Shift+Tab` in Ask walks through
  whichever ones it offers; an agent that offers none simply has one behaviour.
  A mode the agent then refuses — Claude's `auto` is not available on every
  model — drops out of the list.
- **Permission requests** come to the same dialog from every agent. What an
  agent asks about, though, is its own business: one may ask before every edit
  and another only before running a command.

  ![An agent laying out its plan for tidying a folder and waiting on a yes before it moves anything](images/ask-permission.gif)

## Pictures in an answer

An agent can send a picture rather than describe one: a screenshot of the page
it just built, a diagram it drew. Ask draws it in the conversation, right
where the agent put it. Pictures are scaled to the width of the card and no
taller than about a third of it, so a shot of a whole display does not push the
rest of the conversation away.

Pictures come from what the agent *does*, not from what it writes: ask it about a
screenshot and it reads the file, and the picture it read appears in the answer.
Nothing needs turning on, and no agent has to be told. PNG, JPEG, GIF, WebP, BMP, AVIF and HEIF are drawn; an SVG
is not, and shows as its name instead.

## Choosing one

- **Claude Code** is the most complete here: it asks real questions, has the
  most modes, and takes Otto's skills as a plugin.
- **Codex** is close behind, and asks questions too.
- **pi** asks questions as well, as long as the answers can be picked from a
  list.
- **Hermes** and **OpenCode** work and know about Otto, but they cannot stop
  to ask you a question. Watch the conversation rather than waiting for a
  dialog.

Whichever you pick, `default_agent` in `agents.toml` says which one a request
goes to when you do not choose.

## Where sessions run

Sessions in herdr are new, and in Otto's nightly builds only. Otto doesn't
install herdr for you yet: get it from [herdr.dev](https://herdr.dev).

When [herdr](https://herdr.dev) is installed, every session runs in it, as
the agent's own program in a herdr terminal: Claude Code, pi or OpenCode, with
the screen you know from using it by hand. Ask, Sessions and Studio show the
same session: what you write in Ask is typed into the terminal, and what you
write in the terminal shows up in Ask. Work in whichever is closer.

- **It keeps running.** Close Ask, Studio, even herdr-app, and the agent
  carries on in herdr. The sessions are still listed the next time you look.
  One whose agent has quit shows a grey ring; opening it starts the agent
  again where it left off.
- **`Ctrl+O` shows the session in herdr-app**, in its own tab, opening the app
  if it is closed and starting the agent again if it had quit. Unlike a
  terminal opened from Otto, the session stays in Ask too.
- **Sessions open where you work.** A session goes in the herdr-app space
  whose folder is the session's. With none, a space is made there, named
  after the agent's `space` in `agents.toml`, or after the folder.
- **Claude Code you start yourself is picked up.** Run `claude` in a herdr
  tab and the session appears in Ask, like one started from Otto.
- **Removing a session in Ask** ends its agent and closes its terminal. A
  Claude Code you started by hand in a shell is only let go: the shell stays.
- **Questions and permission requests** from Claude Code come to Otto, as
  they do when Otto runs the agent: in Ask, or on the island when no window
  is watching. A permission request offers Claude's own "don't ask again"
  choices. Left unanswered for about nine minutes, Claude asks in its terminal
  instead. pi and OpenCode ask in their terminal for now, so press `Ctrl+O`
  when one stops to ask.

herdr runs Claude Code, pi and OpenCode, so only those agents are offered
there. Codex, Hermes and Gemini run only when Otto runs the agents. With none
of your agents able to run in herdr, it offers plain Claude Code.

In herdr an agent's `command` is the agent's own program, `claude`, `pi` or
`opencode`, with no `args` (the `npx` adapter line of an agent set up for Otto
works too). herdr starts it with the PATH a login shell gets, so a program
that runs in a new terminal runs there too. Some keys in `agents.toml` have no
effect in herdr: `permissions`, `env`, `config` and the `enter` lines. What
an agent may do without asking is up to the agent's own settings and `mode`,
the permission mode its sessions start in (Claude Code's, such as
`acceptEdits`; `auto` also lets OpenCode approve on its own). `name`,
`model`, `folder`, `agent`, `skills`, `colour` and `space` are used.

When sessions run in herdr and some of your agents are still set up for Otto,
`otto-setup` offers **Rewrite for herdr** on its Review page. It is off until
you tick it, and its details show each agent before and after. Each one is
written again the way setup writes a new herdr agent, keeping only its `id`,
`name`, `model`, `mode`, `skills`, `folder`, `colour`, `space` and comments. Who it answers as follows the
Otto box on the Agents page: a persona of its own, its `permissions`, `env`,
`config` and any other keys are not carried over. Codex and Hermes agents are
never offered, since herdr doesn't run them. Apply copies `agents.toml` to
`agents.toml.bak` first. Run without the interface,
`otto-setup --apply --rewrite` does the same; plain `--apply` leaves your
agents as they are.

**Picking where they run.** Settings › Agents › **Sessions run in** has
Automatic (herdr when it is installed, else Otto), herdr and Otto. In
`agents.toml` that is `backend = "herdr"` or `backend = "acp"`, with no
`backend` for Automatic. `otto-setup` offers the same choice, and writes it
only when it differs from what would happen anyway. Sessions in herdr and
sessions Otto ran are listed apart, so switching shows the other set.

## Add an agent of your own

The four agents above are the ones Otto is set up with, not the only ones it
can have. An agent is two things: a file saying who it is, and a block in
`agents.toml` saying how to run it. Here is a marketing assistant called Nora,
who works in one repository and nothing else.

**1. Write who it is.** Otto reads agents from plugins: a directory with a
name, holding skills, agent files, or both. Yours goes in
`~/.local/share/otto/plugins/`:

```
~/.local/share/otto/plugins/marketing/
├── .claude-plugin/plugin.json    {"name": "marketing", "version": "1.0.0"}
└── agents/nora.md
```

`nora.md` is Markdown with a little YAML at the top:

```markdown
---
name: nora
description: Writes and plans the marketing. Knows the product and its audience.
model: opus
tools: Read, Write, Edit, Grep, Glob, Bash, WebSearch, WebFetch
---

You are Nora. You look after how this project is described to the people who
might use it …
```

Everything under the second `---` is the agent's instructions: who it is, what
it does, how it should answer. Write it as if you were telling a new colleague
what the job is.

`otto-agents plugins status` says whether Otto found it.

**2. Give it a seat.** Add a block to `~/.config/otto/agents.toml`. The
simplest one copies an agent already there and changes four lines:

```toml
[[agents]]
id = "marketing"
name = "Nora"                # what the launcher shows
description = "Marketing, on Claude Code"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
model = "opus"
permissions = "ask"
skills = "claude"            # `agent` needs this
agent = "nora"               # the `name` from nora.md
folder = "~/dev/my-project"  # all it can reach
colour = "magenta"           # the material its card wears
enter = ["claude", "--resume", "{session}"]
enter_new = ["claude", "--session-id", "{session}"]
```

`folder` is worth a moment: it is the reach you are handing over, so name the
narrowest folder the agent needs rather than your home.

In herdr, `command = "claude"` with no `args` is enough, and the `enter`
lines and `permissions` are not used (see
[Where sessions run](#where-sessions-run)).

**3. Restart the service.** Run `systemctl --user restart otto-agents`. The
configuration is read when it starts, so nothing changes until you do.

**4. Ask it something.** Open Ask, press `Down`, and Nora is in the list.

### On another harness

`agent` is Claude Code's route: it reads the plugin file where it lies. Every
other harness wants the same instructions in its own dialect and its own place,
so `otto-agents plugins install` writes them, and `otto-agents plugins status`
lists what it wrote and what is still missing. Which file is used is then part
of the `[[agents]]` block: OpenCode takes a `default_agent` in
`OPENCODE_CONFIG_CONTENT`, Codex a `CODEX_CONFIG` pointing at the rendering,
pi a wrapper in `PI_ACP_PI_COMMAND`, Hermes a profile named after the agent.
Copy the block of the agent you are borrowing from and change the one line
that names the file.

## When something is wrong

- **Nothing happens.** Check the service: `systemctl --user status otto-agents`
  and `journalctl --user -u otto-agents -f`.
- **"command not found" in the log.** The service does not get a login shell's
  PATH, so anything under nvm or `~/.local/bin` is invisible to it. Run
  `otto-setup`, which adds the folders your agents live in; or put the full
  path in `agents.toml`, or add the directory with
  `systemctl --user edit otto-agents`.
- **The agent says it is Claude, or Codex, rather than Otto.** Run
  `otto-agents plugins install` again, then `otto-agents plugins status`: a
  rendering that is not `current` is one the agent will not have read.
- **Your own agent answers as plain Claude.** Its `name` is probably one Claude
  Code already knows. An agent of the same name in `~/.claude/agents/` wins,
  and the plugin's file is dropped without a word. Give yours a name nothing
  else uses.
- **No dialog appears when it needs you.** Permission requests and questions
  are drawn by otto-islands; without it, a permission request is denied and a
  question waits in the conversation. In herdr, pi and OpenCode ask in their
  terminal: press `Ctrl+O` to answer there.
- **An agent is missing from the list in herdr.** herdr runs Claude Code, pi
  and OpenCode only. Switch Sessions run in to Otto for the others.
- **Sessions in herdr don't start.** `otto-agents doctor` says whether the
  service finds `hc`, `herdr-app` and each agent's program (and `sqlite3`,
  which it reads OpenCode's sessions with). The service needs `hc` and
  `herdr-app` on its own PATH; `otto-setup` adds them.
