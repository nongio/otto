# 0017: The chat, out of the launcher

**Status:** In progress: milestones 1–5 done; 6 (Preview hosts it) is 0016's milestone 3

## Goal

Any Otto app can host the Ask chat: the conversation with an agent, its tool
calls, its questions, the files that go with a request, and the field you type
into. Today only the launcher can, because the chat lives in `otto-launcher`
and is wired into its card. Preview needs it next, beside the document
([0016](0016-agent-edited-documents.md), milestone 3), and the side canvas may
want it after that. 0016 depends on this plan for its milestones 2 and 3.

This plan moves the chat into `otto-agents-kit`, where the session list and the
keys already are, in steps that keep the launcher working after each one. It
changes no behaviour: the launcher looks and works the same at the end.

## Decisions

- **Grow `otto-agents-kit`, no new crate.** Its own docs already call it "the
  shared parts of Otto's agent UIs", it already depends on `otto-kit`, `ahp`,
  `otto-agents-client`, SCTK and tokio, which is everything the chat needs, and
  both apps that will use it depend on `otto-kit` anyway. 0016 named an
  `otto-ask-kit`; a second crate with the same dependencies would only add a
  boundary to keep in step.
- **No model-only crate yet.** Splitting the model (no Skia) from the drawing
  only pays off with a user that has no window. There is none: the otto-agents
  CLI talks AHP directly, and otto-islands gets its questions over D-Bus. The
  model goes in its own modules with no drawing in them, so the split stays a
  file move if such a user appears.
- **The kit draws onto a canvas, not into a scene.** The log is painted a band
  at a time into a `ScrollPane`, which works the same in the launcher's
  `lay-rs` card and in Preview's plain otto-kit window. The kit never touches
  `lay-rs`; the card, its material and its layers stay in the launcher.
- **The width is the host's.** The log is laid out and painted at a width the
  host gives, not at the launcher's `CARD_W`. A side panel is narrower than the
  card and changes size.
- **The host keeps its keys and its field.** The kit gives the field's rows
  (agents, the agent's question, input requests) as `Item`s, as it does now, and
  answers what a key or a click on the log means. What Escape does, where the
  field sits and how the card grows stay with each app.
- **One background-connection pattern.** `Ask` and `SessionFeed` each start a
  thread with a current-thread runtime, a channel and a wake-up socket, with the
  same `Reporter` written twice. They share one.
- **The log's inset is the kit's.** `LogPainter` paints the log's text
  `log::paint::INSET` in from either edge of its pane; the width a host gives
  is the text's, so the launcher passes `LOG_W = CARD_W - 2 * INSET`.
- **The transcript is its own module.** `Said`, `Picture` and `Attachment` sit
  in `chat::transcript`, apart from the connection, so the log's layout takes
  them without `Ask`. `chat` re-exports them.
- **The host finds windows.** `Terminal::focus` takes the function that brings
  a matching window forward; the launcher passes its foreign-toplevel lookup,
  so the kit needs no Wayland protocol of its own for it.
- **`ChatView` answers the conversation's keys, not the composer's.** It
  takes the keys that scroll the log, stop the turn, change the mode, copy and
  select all; Return, Tab completion, Escape and the field stay the host's.
  It works in the log's content coordinates: the host says where a point on
  its surface falls in the log, because the pane and its scroll are the
  host's, and it reports what a press or release came to (`Pressed`,
  `Released`) rather than acting on the host's state, as for a pending
  attachment that may be otto-stash's.
- **Islands and the service keep their own questions.** otto-agents asks
  otto-islands over `org.otto.Dialog1` when no client watches the chat, from ACP
  types, before AHP sees the question. That is a different layer, not a copy of
  the launcher's, and stays as it is.

## What there is

Where each part of the agents UI lives today. *Shared* means a crate other apps
use; *launcher* means only `otto-launcher` has it.

| Part | Where | Used by | State |
|---|---|---|---|
| Transport, socket path, `file://` and session URIs | `otto-agents-client` | otto-agents, launcher, agents-kit | Shared |
| Connect and handshake | `otto-agents-kit/src/sessions.rs:47` | canvas, launcher (`ask.rs:2339` wraps it, client name fixed to `otto-launcher`) | Shared |
| Background thread, channel, wake socket | `sessions.rs:187` (`Reporter`), `sessions.rs:246`; again in `ask.rs:2108`, `ask.rs:853` | canvas; launcher | Duplicated |
| Session rows, subtitle, activity | `otto-agents-kit/src/sessions.rs:73`, `:112`, `:124` | canvas, launcher agents mode | Shared |
| Session list kept up to date | `SessionFeed`, `sessions.rs:222`; the launcher lists on its own connection (`ask.rs:821` `sessions`, `ask.rs:1069`) | canvas; launcher | Two feeds, one row builder. Fine: the launcher follows a chat on the same connection |
| Rows, ranking, row painting, field style | `otto-kit` `item_list`, re-exported as `otto_agents_kit::{item, rows}` | launcher, canvas, settings | Shared |
| List and field keys | `otto-agents-kit/src/keys.rs` | launcher, canvas | Shared |
| Chat model: requests, answers, tool-call steps, status, modes, terminal hand-off, agent colours, skills | `otto-launcher/src/ask.rs` (`Ask` at `:821`, `Transcript` at `:484`, `Step` at `:285`, `Status` at `:210`, `Terminal` at `:681`) | launcher | Launcher only |
| Permission questions in the chat | `ask.rs:313` `Question`, `:360` `action_lines`, `:1276` `question_rows` | launcher | Launcher only |
| Input requests (choices, values, links) | `otto-launcher/src/input.rs` | launcher | Launcher only |
| Permission and question dialogs when nobody watches | `otto-agents/src/dialog.rs` (ACP side) → `org.otto.Dialog1` → `otto-islands/src/dialog.rs` | otto-agents, islands | Separate by design |
| Attachments: model, layout, thumbnails | `otto-kit/src/components/attachments.rs` | launcher, stash | Shared |
| Following otto-stash | `otto-kit/src/components/stashed.rs`; `Ask::set_stashed` `ask.rs:917` | launcher | Mirror shared, use launcher only |
| Markdown | `otto-md-kit`, `otto_kit::preview::document` | launcher, peek, preview | Shared |
| Log layout: `Block`, `Footer`, `Line`, `Kind`, `lay_out` | `otto-launcher/src/log.rs:188`, `:212`, `:135`, `:81`, `:233` | launcher | Launcher only |
| Log painting and fonts | `view.rs:789` `paint_log`, `:614` `measure_log`, `:618` `log_font`, `:94` `log_body` | launcher | Launcher only, inside `Palette` with the card |
| Log hit testing | `view.rs:594` `attachment_at`, `:725` `steps_at`, `:751` `code_at`, `:763` `link_at` | launcher | Launcher only |
| Selecting log text | `otto-launcher/src/selection.rs`, `view.rs:636` `log_spans`, click counting at `main.rs:1442` | launcher | Launcher only; `otto-kit/src/components/selectable_text.rs` does the same for Files' info panel |
| Log view state and glue: lines, spans, selection, hovers, open steps, `Transcript` → `Block`s | `main.rs:182`–`:230` (fields), `main.rs:1118` `relayout_log`, `:1337`–`:1460` hovers and clicks, `:1797` `LogRows` | launcher | Launcher only; every host would write it again |
| Ask-mode keys: Return sends or answers, Ctrl+C cancels, Shift+Tab changes mode, Tab completes a skill | `main.rs:2092` `on_key_event` | launcher | Launcher only |
| Log for screen readers | `log.rs:142` `Line::text`, `main.rs:2658` | launcher | Launcher only |
| Agent colour on the card | `ask.rs:779` `colours_from_meta`, `main.rs:827` `card_tint` | launcher | Launcher only |

Coupling that stops the chat files moving as they are:

- `log.rs:16` takes `Said` and `Attachment` from `crate::ask`, `log.rs:459` takes
  `crate::ask::Picture`, and `log.rs:446` takes its body style from
  `crate::view::log_body`.
- `input.rs:24` and `ask.rs:68` take `Style` from `crate::log`; `ask.rs:67` takes
  `crate::input`. The three are one unit and move together.
- `view.rs:124` fixes the log's width as `LOG_W = CARD_W - 2 * LOG_INSET`, and
  every log method in `view.rs` uses it.
- `view.rs:194`: `Palette` holds the card's layers and the log's caches
  (pictures, attachment layouts) in one struct.
- `ask.rs:2339` introduces every client as `otto-launcher`.

## Target layout

```
otto-agents-kit/src/
  lib.rs
  keys.rs          (as now)
  sessions.rs      (as now, on link.rs)
  link.rs          the connection thread: runtime, channel, wake socket
  chat/            no drawing
    mod.rs         Ask, Transcript, Entry, Said, Picture, Step, Status, Note,
                   Question, Modes, Terminal  (from ask.rs)
    input.rs       InputRequest and its rows  (from input.rs)
  log/
    layout.rs      Block, Footer, Line, Kind, Style, lay_out  (from log.rs)
    paint.rs       LogPainter: fonts, pictures, attachment layouts, paint,
                   measure, hit tests, spans  (from view.rs)
    selection.rs   Span, Caret, Selection  (from selection.rs)
    view.rs        ChatView: the host glue from main.rs
```

The launcher keeps `main.rs`, the card half of `view.rs`, `apps`, `calc`,
`windows` and `source`.

## What a host uses

```rust
use otto_agents_kit::chat::Ask;
use otto_agents_kit::log::{ChatView, Hit};

let mut ask = Ask::connect(Client::new("otto-preview"), url, folder);
let mut chat = ChatView::new(dark);

// poll_fds: ask.poll_fd()
// on wake:
if ask.pump() {
    chat.lay_out(&ask, width);
}

// ChatView is a ScrollContent: hand it to a ScrollPane.
// Pointer: the view tracks hovers and selection, and says what a click means.
match chat.press(point, time) {
    Some(Hit::Link(url)) => open(url),
    Some(Hit::Attachment(hit)) => ask.toggle_attachment(hit.item),
    Some(Hit::Copied) | None => {}
}
// Keys the host doesn't claim:
chat.key(&mut ask, &event, &mut field);   // Return, Ctrl+C, Shift+Tab, Tab, Ctrl+A

// Rows under the field, painted with otto_agents_kit::rows:
let rows = ask.question_rows(0);   // or input_rows, agent_rows
```

The names are a sketch; milestone 5 settles them against what Preview needs.

## Milestones

Each one leaves the launcher building, its tests passing and its ask mode
unchanged on screen.

1. **Seams, in place.** In the launcher: `Style` and `log_body` move into
   `log.rs`, which takes `Said`, `Picture` and `Attachment` from a small
   `transcript` module rather than from `ask.rs`. `Ask::connect` takes the
   client's name. No file leaves the crate.
2. **The log at any width.** Split `Palette` into the card and a `LogPainter`
   that owns the log's fonts, pictures and attachment layouts and takes the width
   as a parameter. `LOG_W` stays only as the width the launcher passes.
3. **One connection thread.** `link.rs` in `otto-agents-kit`, with `Reporter`
   and the thread start. `SessionFeed` uses it, then `Ask` does.
4. **Move the chat.** `ask.rs`, `input.rs`, `log.rs`, `LogPainter` and
   `selection.rs` move to `otto-agents-kit` with their tests. The launcher
   imports them from there; `otto-launcher/examples/preview.rs` still renders.
5. **`ChatView`.** The log's state and glue in `main.rs` (`relayout_log`, the
   hover and click handlers, `LogRows`, Ctrl+A and copy, the screen-reader lines)
   and the ask-mode keys move into `ChatView`. `main.rs` keeps the card, the
   modes and the list.
6. **Preview hosts it.** This is 0016's milestone 3, and the check on this plan:
   if Preview needs anything from `otto-launcher`, it goes back to milestone 5.

## Testing

- The tests move with the code: `ask.rs`'s against an unreachable service and
  built chat states, `log.rs`'s layout tests, and `view.rs`'s
  `every_kind_of_line_in_the_log_can_be_pointed_at` and
  `a_bare_link_in_the_log_is_under_the_pointer`, now against `LogPainter`.
- New for milestone 2: the log laid out and painted at two widths keeps every
  line pointable, and a narrow width wraps rather than clips.
- New for milestone 3: `SessionFeed` and `Ask` both report an unreachable
  service through `link.rs`.
- `scripts/ci.sh` and the launcher's preview example after every milestone.
- **Manual, after each milestone:** ask a question with an attachment, open the
  tool calls, answer a permission question, answer an input request, select and
  copy text and a code block, click a link, change mode with Shift+Tab, cancel
  with Ctrl+C. Same as before, in the same places.

## Open questions

- **One text selection or two.** `otto-kit`'s `selectable_text` and the
  launcher's `selection` are both run-based selection with click counting.
  Milestone 4 moved the launcher's as it is, to `log::selection`; merging the
  two into `otto-kit` is still open.
- **The composer.** Preview needs a field with the question rows under it, as
  the launcher has. Is that `ChatView`'s, or does each host build it from
  `TextInput` and `rows`? For now the host builds it: Return (send or answer)
  and Tab (complete a skill) stay in the launcher's `main.rs`.
- **Agent colours.** The launcher tints the whole card with the agent's
  material. A panel in Preview probably shouldn't. Does the colour stay in
  `Ask` for hosts to use as they like?
- **Session list on the chat connection.** The launcher lists sessions on the
  connection that follows its chat; the canvas uses `SessionFeed`. Worth making
  `Ask` use a `SessionFeed`'s list, at the cost of a second connection?
- **The canvas.** Once the chat is in the kit, the side canvas could carry a
  conversation on instead of running `otto-launcher --session`. Out of scope
  here.
