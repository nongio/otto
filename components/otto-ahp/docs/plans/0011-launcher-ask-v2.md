# 0011: Launcher ask, second version

**Status:** In progress

## Goal

The launcher stays open after Return and shows the agent working, so you can read the
answer where you asked. Closing the launcher never affects the session. This builds on
the POC ask mode ([0010](0010-poc.md)), on branch `launcher-ask` in `../otto-4`.

## Flow

1. **Typing.** The card is the prompt line alone.
   - The typed text is no longer repeated as a row below the prompt.
   - **One agent:** there is no pane; Return sends the prompt.
   - **More than one agent** (changed 2026-09-15): Return sends to the default agent,
     the first in the root state's `agents`, which otto-ahp orders by `default_agent`
     in `agents.toml`. Down lists the agents under the field, each with its name and
     description, with the default selected; Up and Down pick one and Return sends to
     it, and Up from the first puts the list away. The typed text is the prompt, so it
     never filters the list.
2. **Running.** The launcher is a conversation (changed 2026-09-15). The log opens
   above the field, and the field stays where it is on screen while the card grows
   upwards. Return moves the typed request into the log and empties the field, which
   then says *Ask a follow-up…*. Typing and pressing Return again queues the next
   request on the same session; it shows as *Queued* until its turn starts.
   - **Log:** the answer as it streams. It scrolls, follows the end while text
     arrives, and stops following once you scroll up; scrolling back to the end
     follows again. The card grows to a maximum height, then scrolls inside. Up,
     Down, Page Up, Page Down, Home and End scroll it too.
   - **Status line:** the log's last line, dimmed: *Starting {agent}…*, then
     *Thinking…* or *Working…*, then *Done*, *Cancelled* or *Failed: {error}*. It
     sits at the end rather than the top, so it stays in view while the log follows
     the answer.
   - **Reasoning is transient,** as in the Claude Code harness. While the agent
     reasons, the status line reads *Thinking…*. Reasoning is never written into the
     log.
3. **Closing.** Esc or a click outside closes the launcher, as it does today. The
   session carries on, and `otto-ahp show --follow` or otto-agents can pick it up.

## Decisions

- **The hand-off stays queued.** The launcher creates the session and queues the prompt
  with `chat/pendingMessageSet`, exactly as in 0010, then stays subscribed to the chat.
  Closing at any moment is therefore safe, because the service already owns the request.
- **The folder is `$HOME`.** Choosing a folder comes later.
- **Cancelling needs its own shortcut, because Esc closes.** Proposed: Ctrl+C while the
  agent is working, which dispatches `chat/turnCancelled`. The prompt field is read-only
  by then, so Ctrl+C has nothing to copy. Confirm it by trying it.
- **A background thread owns the connection.** The launcher's loop has no async runtime.
  - A thread with a current-thread tokio runtime holds the WebSocket and sends updates
    over a channel, together with a wake-up file descriptor.
  - The launcher returns that descriptor from `poll_fds`. On each wake-up it drains the
    channel, applies the actions to its `ChatState` with `ahp::reducers`, and redraws.
  - This follows the pattern the window list already uses (`poll_fd` and `pump`), and it
    replaces 0010's blocking send with its 3-second timeout.
- **The log is a `ScrollContent`.** The results list's `ScrollPane` shows the log once
  a request is made, so no new scroll widget is needed. The log is word-wrapped in
  `log.rs` and drawn with otto-kit's font-fallback runs; markdown is shown as typed
  for now. `ScrollPane::scroll_to` keeps the end in view.
- **The card can be moved** (added 2026-09-15). Pressing on the field or the log and
  dragging moves the card, kept on the output; the rows are not a handle, so clicks on
  them still pick. The input region follows the card. Where it was dragged is not
  remembered between launches.
- **Errors are shown, not only logged.**
  - A service that isn't running shows as the card's message line while you type.
    Return still starts the request, which fails at once.
  - A failed agent start, a failed turn and a rejected action show in the status line.
- **Closing waits for the hand-off.** If the request is still on its way to the
  service, the card closes but the process waits, for up to 3 seconds, until the
  service has it.
- **Cancel covers a running turn.** Ctrl+C cancels only when nothing is selected in
  the field; with a selection it copies. Before a turn starts, its request is only
  queued, and the host doesn't yet accept a client removing a queued message.
- **Agent questions go where the user is** (implemented 2026-09-15). Under
  `permissions = "ask"`, otto-ahp opens each ACP permission request in the chat as a
  tool call waiting for confirmation (`chat/toolCallStart`, then `chat/toolCallReady`
  with the agent's own options). It then routes it:
  - **A client is subscribed to the chat,** such as this launcher: otto-ahp waits for
    its `chat/toolCallConfirmed`. The launcher shows the question in the log and the
    agent's answers ("Always Allow", "Allow", "Reject") as rows under the field. Up
    and Down pick one, starting on the narrowest allow; Return with an empty field, or
    a click, answers.
  - **Nobody is subscribed,** or the last watcher unsubscribes or disconnects: the
    question goes to the islands dialog, and a plain yes or no picks the narrowest
    matching option.
  - The first answer wins; a later one is refused. The islands dialog isn't withdrawn
    when the launcher answers first, because `org.otto.Dialog1` has no way to withdraw
    it.
  - Allowed tool calls complete when the agent reports them finished, and show in the
    log as ✓, ✗ or Denied.

## Attachments, opening sessions, agents mode (added 2026-09-15)

- **Files on start.** `otto-launcher --ask --file PATH [--file PATH]…` attaches the
  files to the first request. While you type, they are listed as rows under the field,
  each with its folder, after the agent list when there is one. They are only shown:
  never highlighted, and clicking one does nothing. Once sent, they leave the rows and
  the log shows *Attached: a.png, notes.md* under that request, also for requests read
  back from the chat. Each file goes as a `resource` message attachment
  (`file://` URI, the file name as label). otto-ahp passes it to the agent as an ACP
  `resource_link` content block, which every ACP agent accepts, and the agent reads the
  file with its own tools. Other attachment kinds are dropped with a warning.
- **"Ask…" in Files.** `components/otto-files/scripts/ask` is a files-script offering
  *Ask…* for any selection. It starts `otto-launcher --ask --file …` in a session of its
  own (`$OTTO_LAUNCHER` overrides the binary). Copy it to `~/.config/otto/files-scripts/`.
- **Opening a session.** `otto-launcher --session ID` (a URI, an id or the start of one)
  follows an existing session instead of creating one: *Opening the session…*, then
  its whole conversation, including a running turn and pending questions. Requests
  typed after that queue on it. Sessions live in the otto-ahp process, so a restarted
  service has none to open.
- **Agents mode.** `otto-launcher --agents` lists the service's sessions, most recently
  changed first, with title, status and folder. Typing narrows them by title, and Return
  or a click opens the selected one as with `--session`.
- **Entering a session in a terminal** (added 2026-09-15). Ctrl+O in an open or
  started session opens it in the agent's own interface in a terminal, and closes the
  launcher.
  - Configured in `agents.toml`: a top-level `terminal`, such as
    `["ghostty", "--working-directory={cwd}", "-e"]`, and the agent's `enter`, such as
    `["claude", "--resume", "{session}"]`.
  - Once the agent's id for the session is known, otto-ahp publishes the joined command
    and folder in the session's `_meta` as `otto.terminal` (`session/metaChanged`),
    recomputed from the configuration on restart. The launcher reads it from its session
    subscription and spawns it in a process group of its own.
  - This is the POC of `x-otto/enterSession` ([0003](0003-acp-agent-backend.md)) without
    its guard: the service's agent process isn't ended and a working session isn't
    refused, so a turn from the launcher and the terminal must not overlap. A spawn that
    fails is only logged.
- **Back to the sessions** (added 2026-09-16). Left with an empty field, in a session
  that was opened or started, goes back to the list of sessions, as agents mode shows it.
  The launcher drops its connection and opens a new one, which lists the sessions afresh,
  and the session carries on as it would if the launcher had closed. It is refused while
  a request is still being handed off.
- **Switching modes (planned).** Left and Right switch between ask mode and agents mode,
  whichever one the launcher opened in. It works only before a request is sent or a
  session is opened; after that the launcher is a conversation, and Left with an empty
  field goes back to the sessions (above).
  - The arrows move the caret in the field, so they switch modes only while the field is
    empty. Otherwise they move the caret as usual.
  - The field's placeholder names the mode, as it does today (*Ask an agent…* or
    *Search agent sessions…*). A small hint could show the other mode is one arrow away.
  - Files given with `--file` stay attached across the switch, and go with the first
    request, or the first follow-up in an opened session.

## Depends on otto-ahp

- **Nothing new for the first cut.** The agent list, response parts, deltas, reasoning
  and turn ends are already served.
- **Step-by-step progress** ("Reading README.md", "Running cargo test") needs ACP tool
  calls mapped to AHP tool call parts ([0003](0003-acp-agent-backend.md)). Until then,
  the log is the answer text and the permission-asked tool calls.
- **Attachments** need `resource` attachments passed on as ACP resource links
  (implemented: `host.rs` `attachments`, `acp.rs`).

## Open questions

- **Cancel shortcut:** is Ctrl+C right once it's in use?
- **Picking an agent from the keyboard,** such as an `@name` prefix. Later, if the list
  proves slow to use.

## Milestones

- [x] **Typing view.** No echo row; the agent list when there's more than one agent.
- [x] **Connection thread.** The wake-up socket, the chat state kept up to date, and
      errors shown. `components/otto-launcher/src/ask.rs`
- [x] **Log view.** The status line, the streamed answer, follow and scroll, and the
      card growing to its maximum height. `src/log.rs`, `Palette::update_log` and
      `paint_log` in `src/view.rs`
- [x] **Cancel shortcut.** Ctrl+C
- [x] **Tests.** Wrapping, progress from chat state, an unreachable service, and an
      ignored end-to-end test against `otto-ahp serve --echo`
- [x] **Conversation.** Log above the field, Return queues follow-ups on the same
      session, per-request *Queued* / *Cancelled* / *Failed* notes. The live test
      queues a second request straight after the first.
- [ ] **Try it in Otto.**
- [x] **Questions in the launcher,** routed as above. otto-ahp: `host.rs` (questions,
      routing, escalation), `tests/questions.rs`. Launcher: answer rows, `Question` in
      `ask.rs`, the log pane above the field.
- [x] **Attachments.** `--file`, the *Attached* line, resource links to the agent
      (`tests/acp.rs`), and the Files *Ask…* script.
- [ ] **Switching modes.** Left and Right switch between ask and agents mode while the
      field is empty and nothing has been sent or opened.
- [x] **Opening sessions.** `--session` and `--agents`. Unit tests for picking and
      opening, and an ignored live test that reopens a session with its attachment and
      carries it on.
