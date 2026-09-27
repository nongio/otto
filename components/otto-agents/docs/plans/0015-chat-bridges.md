# 0015: Chat bridges

**Status:** Draft

## Goal

You message Otto's agents from the chat apps on your phone. A Telegram or Matrix chat
is tied to an agent session: what you write there is queued on the session, the
agent's answer comes back to the chat, and its questions and permission requests
reach you there as well as on the desktop. Which chats are listened to, who may
write in them, and which session each one feeds are set in configuration.

The desktop sees the same sessions. A conversation started from the phone shows in
Sessions and can be entered at the desk, and the other way round.

This is the gateway model of OpenClaw, built from what Otto already has: otto-agents
is the host, and each chat network is one more AHP client.

## Decisions

- **A separate component, `otto-bridge`, is an AHP client.** It connects to
  otto-agents over the Unix socket, like every Otto surface (0007), using
  `otto-agents-client` and `ahp`. It is not code inside otto-agents:
  - Chat networks need HTTP, TLS and, for Matrix, end-to-end encryption. otto-agents
    has none of these dependencies and should not grow them.
  - Bot tokens live with the bridge, never in `agents.toml` (0004 keeps credentials
    out of it) and never in AHP state.
  - A network outage, a bad token or a ban stops the bridge, not the agents.
  - It runs as its own systemd user unit, off by default, like otto-agents.
- **No changes to otto-agents are needed for the first version.** What the bridge
  uses exists today:
  - `createSession` with a client-chosen `ahp-session:/<uuid>`, a `provider` and one
    folder (`host.rs:417`).
  - **Queued messages** (`chat/pendingMessageSet`, kind Queued, `host.rs:2118`) run
    at once when the chat is idle and after the current turn otherwise, and wake an
    agent stopped by the idle timeout. The bridge never uses `chat/turnStarted`, so
    it never races a busy session.
  - Permission requests and agent questions are in the chat state, and any
    subscriber may answer them; the first answer wins over the otto-islands dialog
    (`host.rs:1484`, `host.rs:716`).
  - Attachments as `file://` resources, which the agent may read without asking
    (`attached.rs`).
  - Sessions persist and resume across restarts (`store.rs`, `acp.rs:674`).
- **"Channel" is already an AHP word,** so this plan says *network* for Telegram,
  Matrix and the like, and *chat* for a conversation on one of them.
- **Deny by default.** A chat that no route names is ignored. A sender a route
  doesn't list is ignored. Nothing is answered, so the bot gives nothing away.
  Both are logged.
- **Outbound connections only.** Telegram through long polling of the Bot API,
  Matrix through `/sync`. The bridge opens no port and needs no public address.

## Configuration

Its own file, `bridges.toml`, read in the same order as `agents.toml`:
`/etc/otto/bridges.toml`, then `$XDG_CONFIG_HOME/otto/bridges.toml`, with
`--config` to point elsewhere. Unknown keys are errors.

```toml
# The networks the bridge connects to.
[[networks]]
id = "tg"
kind = "telegram"
token_file = "~/.config/otto/secrets/telegram"   # mode 0600; the token is never inline

[[networks]]
id = "home"
kind = "matrix"
homeserver = "https://matrix.example.org"
user = "@otto:example.org"
token_file = "~/.config/otto/secrets/matrix"

# Which chats are listened to, and where their messages go.
[[routes]]
network = "tg"
chat = "123456789"              # the network's chat id; the bridge logs ids it ignores
from = ["123456789"]            # who may write; empty is nobody
agent = "claude-remote"         # an [[agents]] id; unset is the default agent
folder = "~/"                   # a new session's folder; unset is the agent's folder
session = "per-chat"            # see Sessions below

[[routes]]
network = "home"
chat = "!abcdef:example.org"
from = ["@riccardo:example.org"]
session = "8f3c"                # pinned to an existing session, by id prefix
mention = true                  # in a group, only messages that mention the bot
```

- **The agent is chosen with the route.** A dedicated `[[agents]]` entry for remote
  use is recommended: `permissions = "ask"`, a narrow `folder`, perhaps a cheaper
  `model`. The bridge checks each route's agent against `RootState.agents` at start
  and logs a route whose agent is missing; its messages are refused with a reply.
- **Secrets.** `token_file` first. Reading from the Secret Service is a later option
  (open question).
- **Hot reload** is not in the first version, the same as otto-agents.

## Sessions

`session` on a route decides which session a message feeds.

| Value | Behaviour |
|---|---|
| `"per-chat"` (default) | The first message creates a session. Later messages are queued on it. `/new` in the chat starts a fresh one and ties the chat to it. |
| `"per-message"` | Every message creates a session. For one-off requests, and later for inbound messages that should never share context. |
| an id or id prefix | Pinned to an existing session. If it has been disposed, the message is refused with a reply that says so, unless `on_missing = "new"`. |

- **Bindings are state, not configuration.** The chat → session table lives in
  `$XDG_STATE_HOME/otto-bridge/bindings.json` (0600) and survives restarts. A pinned
  route never writes to it.
- **Switching sessions from the chat.** With `switch = true` on a route, `/sessions`
  lists the sessions (title, agent, age, status) and `/use <id>` ties the chat to
  one of them. Off by default, because it reaches every session on the machine.
- **Sessions created by the bridge** get `ahp-session:/<uuid>` like any other; the
  title comes from the first message, as it does today.

## Message flow

1. **Receive.** A message arrives from a network. The bridge finds the route for
   `(network, chat)`, then checks the sender against `from`, and in a group with
   `mention = true`, that the bot is mentioned.
2. **Chat commands** are handled by the bridge and never reach the agent:
   `/new`, `/stop` (dispatches `chat/turnCancelled`), `/status`, and with `switch`,
   `/sessions` and `/use`.
3. **Attachments.** Files, photos and voice notes are downloaded to
   `$XDG_CACHE_HOME/otto/bridge/<network>/<chat>/` (0700) and attached as `file://`
   resources. Inline data is not accepted by otto-agents (`host.rs:3148`), so this
   is the only form. A size limit per route, default 20 MB.
4. **Queue.** The text and attachments go to the bound session as a queued message.
   In a chat with more than one allowed sender, the text is prefixed with the
   sender's name, so the agent can tell who is asking. The bridge acknowledges on the
   network: a typing indicator while the turn runs, and a *queued* reaction when the
   session is busy.
5. **Follow.** The bridge stays subscribed to every chat bound to a route while a
   turn is active or a request is open, and unsubscribes once it is idle.
6. **Reply.** When the turn ends, its final text is sent to the chat, split at the
   network's length limit, with markdown converted to what the network renders.
   Images the agent produced (`file://` links under the images cache) are uploaded.
   `reply = "final"` is the default; `"stream"` edits one message as text arrives,
   where the network allows editing; `"none"` sends nothing back.
7. **Errors** go back to the chat in one line: the agent failed to start, the turn
   failed, the session is gone.

## Permission requests and questions

These already reach every subscriber of the chat, so the bridge renders them and
answers with the same actions a desktop client uses.

- **Tool permission.** The confirmation title and its options are sent as a message
  with buttons: Telegram's inline keyboard, and on Matrix a numbered list answered
  by replying with the number. A tap dispatches `chat/toolCallConfirmed`. Only
  senders in `from` may answer.
- **Agent questions** (`InputRequestResponsePart`): single choice and short text are
  asked in the chat and answered with `chat/inputAnswerChanged` and
  `chat/inputCompleted`. A form with anything else is announced with *Answer this on
  the desktop*.
- **Whichever side answers first wins.** When the phone answers, otto-agents
  withdraws the islands dialog, and the other way round. The bridge edits its
  message to show the outcome, so a stale button can't be tapped.
- **The bridge counts as a watcher.** While it is subscribed, the desktop dialog
  appears only after the 20-second grace (`WATCHED_GRACE`, `host.rs:103`). That is
  right when you are away, and a short wait when you are at the desk and write from
  the phone. See the open questions.

## Networks

Each network implements one trait, so routing, sessions and replies are written once:

```rust
trait Network {
    async fn next(&mut self) -> Result<Inbound>;          // message, command or button tap
    async fn send(&self, chat: &ChatId, out: Outbound) -> Result<MessageId>;
    async fn edit(&self, chat: &ChatId, id: &MessageId, out: Outbound) -> Result<()>;
    async fn download(&self, file: &FileRef, to: &Path) -> Result<PathBuf>;
    fn limits(&self) -> Limits;                           // text length, editing, buttons
}
```

- **Telegram first.** The Bot API over HTTPS long polling: small, official, and
  enough to exercise every part of the design. The polling offset is saved, so
  a restart neither loses nor repeats a message. It needs an HTTP client
  (`reqwest` with rustls), in the bridge only.
- **Matrix second.** Through `matrix-sdk`, for end-to-end encryption, with its
  store under `$XDG_STATE_HOME/otto-bridge/matrix/`. Matrix bridges (mautrix-*)
  then reach WhatsApp, Signal and Google Messages without further work here.
- **Later:** Signal through signal-cli, and phone links (KDE Connect SMS,
  Bluetooth MAP) as networks whose routes are `per-message` and whose agent has no
  tools that act (see *Untrusted senders*).

## Security

- **A new boundary.** 0005 covers local clients reaching otto-agents; the bridge is
  one of them and needs nothing new there. The new edge is a remote sender reaching
  the bridge, and it is guarded by the network's own identity (a Telegram user id, a
  Matrix user id) checked against `from`.
- **Answers are checked too.** A button tap or reply from someone not in `from` is
  ignored, including in a group where others can see the buttons.
- **Untrusted senders.** A route whose senders aren't you, such as inbound messages
  from a phone link, must use an agent with `permissions = "deny"` and no tools that
  change anything. The bridge warns at start when a route with more than one sender
  uses an agent with `permissions = "allow"`.
- **Files.** Downloads stay in the cache directory, mode 0600, and are removed with
  the session's binding or after 7 days.

## Testing

- **Routing and bindings:** unit tests over config fixtures: ignored chats and
  senders, each `session` mode, `/new`, `/use`, and bindings across a restart.
- **End to end without a network:** an in-memory `Network` against `otto-agents
  serve --echo` on a temporary socket: a message is queued, echoed and replied; a
  second message while busy is queued; a restart of either side loses nothing.
- **Confirmations:** an echo-agent mode that asks for permission, answered from the
  in-memory network; the dialog is withdrawn (with a fake `Prompter`).
- **Telegram and Matrix:** request/response fixtures for the parts written by hand;
  a manual checklist against a real bot.

## Milestones

1. Crate, `bridges.toml`, routing, bindings, the in-memory network, tests against
   the echo agent.
2. Telegram: text in, final reply out, chat commands.
3. Permission requests and questions from the chat.
4. Attachments in both directions; `reply = "stream"`.
5. Matrix.

## Open questions

- **Should the bridge be a quiet watcher?** A `_meta` flag on the subscription could
  tell otto-agents to show the desktop dialog at once when the bridge is the only
  watcher. That is a change in `host.rs`, small but not free.
- **Where do tokens live** beyond `token_file`: the Secret Service, or systemd
  credentials (`LoadCredential=`)?
- **Pairing.** Instead of finding chat ids in the log, a `/pair` message could show a
  code in an otto-islands dialog and, once approved, add the route. It would have to
  write `bridges.toml` or a state file of approved chats.
- **Showing where a session came from.** A `_meta.otto.origin` on sessions ("via
  Telegram") would let Sessions and the launcher mark them; otto-agents would need to
  keep it in the session record.
- **Automations (0006).** A `per-message` route is close to an event trigger. When
  otto-agents implements AHP automations, inbound messages could become triggers
  instead of bridge-created sessions.
- **Name.** `otto-bridge` borrows Matrix's word. Alternatives: `otto-relay`,
  `otto-inbox`.
