# 0013: Gather and ask (a balloon for context from anywhere)

**Status:** Idea

## Goal

You can collect things from anywhere on the desktop into one balloon, then hand them
to an agent together. What you collect can be:

- text you say (dictation),
- text you select in any app,
- a part of the screen you drag out with the pointer,
- files picked in Files.

They mix: a screenshot of an error plus "why is this failing", or a selected
paragraph plus "make it shorter". You start collecting, can pause and pick it up
again, and send when you are ready. The answer either comes back into the balloon
(and from there into the app you were in), or runs as a background job that shows
up in Sessions.

This grows out of the dictation proof of concept (`components/otto-dictate`) and
the voice plan ([0012](0012-voice.md)). Dictation stays useful on its own: with
nothing else collected and no agent involved, it is plain typing by voice.

## Flow

1. **Start.** A shortcut starts a gathering session. A small indicator shows it is
   on, because while it is on the desktop is listening and collecting.
2. **Collect.** Each source adds a chip to the balloon, in order, so the request
   reads as a timeline:
   - **Speech:** each settled phrase becomes a text chip (the agreement logic from
     otto-dictate).
   - **Selection:** the selected text in the app you are in, with where it came
     from (window, and the range when the app reports it).
   - **Region:** drag a rectangle; the crop becomes an image chip.
   - **Files:** the selection in Files, picked up by the same shortcut as a text
     selection, without opening the palette. Ask… in Files and dropping on the
     balloon work too. Dropping accepts
     anything, so apps with no special support can still add to it.
   Collecting never takes keyboard focus, so the app keeps its selection and caret.
3. **Pause.** The mic stops and nothing new is picked up, but the chips stay. You
   can work normally, then resume. While paused you can review: remove a chip,
   reorder, correct a transcript. This is when the balloon takes focus.
4. **Send.** The last thing you said or typed is the instruction; earlier chips are
   context. At send you choose:
   - **One-shot:** the answer streams into the balloon, with actions.
   - **Background:** the balloon shrinks into a progress island, the session is in
     Sessions, and a notification brings back the result.
5. **Deliver.** The default action follows from what was collected:
   - selection collected: **Replace** the selection;
   - dictation only: **Insert** at the caret, without an agent;
   - otherwise: **show** the answer, with Copy and Continue in Ask.
   A one-shot session is forgotten after delivery unless you continue it.

Esc at any point throws the gathering away. Nothing is sent before you send.

## Shape

- **One service owns the gathering.** A long-running user process (otto-dictate
  grown up) holds the chips, the capture stream and the balloon. Everything else is
  a trigger that talks to it: the shortcut, Files' Ask…, the region picker. It
  exposes a small D-Bus interface: start, pause, resume, add text, add file, add
  image, send, cancel. The Unix socket toggle of the proof of concept goes away.
- **The balloon is not the input method popup.** An input popup exists only while a
  text field has focus and sits where the compositor puts it, so it can't follow a
  session across windows. The balloon gets its own surface (island or overlay). It
  may move next to the caret for review and delivery; where it lives while
  collecting is open (see below).
- **Reading the selection.**
  - Apps with text-input-v3 report `surrounding_text` with cursor and anchor, which
    gives the selected text and its exact range.
  - Otherwise the PRIMARY selection, which Otto can always read.
- **Replacing the selection is explicit.** `delete_surrounding_text` over the
  recorded range, then `commit_string`, as one edit the app can undo. Before
  replacing, check the field still reports the same text around the range; if it
  changed, copy the answer and say so instead.
- **Who sends text into apps.** Today that requires being the input method, and
  only one input method fits per seat, so this would fight fcitx5 or ibus (and
  smithay currently disables the existing input method rather than refusing the new
  one). Otto can instead send `commit_string` and `delete_surrounding_text` to the
  focused text-input itself, through a small protocol or D-Bus call used by the
  service. The input method slot stays free for the user's IME. Apps without
  text-input fall back to clipboard plus Ctrl+V through the virtual keyboard, or to
  copy only (terminals).
- **Region capture** reuses screencopy. Otto shows the drag rectangle; the crop is
  a PNG chip, scaled down before it is sent.
- **The agent side.** Chips become one prompt: text as text, images as image
  content, files as resource links. One-shot needs a "send one prompt, stream the
  answer, forget the session" call in otto-agents (today only
  `examples/ask.rs` does this). Background is an ordinary session.
- **Audio** follows 0012: PipeWire capture, an external Whisper engine, the
  `[speech_to_text]` config table.

### Apps need no changes

Selection, replace, region and drop all go through what apps already do
(text-input-v3, PRIMARY, drag and drop) or through Otto itself. The new wiring is
between Otto's own parts: the service and the compositor, and the triggers and the
service. Files is the exception, below, because it is ours and has more to say
than a text field.

### Files talks to the desktop

Files gets a D-Bus interface next to `org.otto.FilePicker1`, say `org.otto.Files1`,
with two halves.

- **Its selection, without opening the palette.**
  - `Selection()` returns the selected items of the focused Files window (paths,
    plus the folder it shows); `SelectionOf(window)` for a given one.
  - A `SelectionChanged` signal, so the balloon can show "3 files in Files" live.
  - While gathering, the Files selection is a source like the text selection: the
    same "add" shortcut picks it up when Files is focused. Ask… in the palette
    stays as one more way in.
- **A few palette commands an agent can run:** select matching, move to trash,
  open a location, search recent files, undo. Not file manipulation in general; see
  [0014](0014-mcp-gateway.md) for the list.
  - They run the palette's own code, not a shell `rm`: the change shows in the
    window and lands in Files' undo history, so Cmd+Z undoes what the agent did
    the same as what you did.
  - Methods that change things take an `a{sv} options` with a generic `origin`,
    so the undo entry says who asked. Files doesn't know what an agent is.
  - Reveal also answers `org.freedesktop.FileManager1.ShowItems`, so other apps'
    "show in folder" opens Files at the file.
- **How agents reach it.** Through the MCP gateway ([0014](0014-mcp-gateway.md)):
  `org.otto.Files1` is a plain D-Bus interface, and the gateway turns the allowed
  methods into agent tools, sets `origin` from the session, and marks the
  destructive ones so the harness asks first.
- The interface is on the user's session bus; there is no sandboxed-app access to
  it. A portal can come later if a sandboxed app needs the selection.

### As built (proof of concept)

- **otto-gather owns the gathering** and keeps it until it is sent, or its last
  item is removed or it is cancelled. `org.otto.Gather1` has `Add`, `AddFile`,
  `AddRegion`, `Send` (opens Ask), `Cancel`, and for Ask: `Items() -> a(sb)`,
  a `Changed(a(sb))` signal, `Toggle(u)`, `Remove(u)`, `Sent()` and `Hold()`.
  Items travel as files, text as `selection-N.txt` in the gathering's runtime
  directory, each with whether it is struck out.
- **Ask renders it.** The launcher in Ask mode follows the gathering, shows it
  with the next request and forwards strikes and removals to otto-gather. `Hold`
  hides the card while the launcher is on the bus; closed without sending, the
  card comes back. Sending calls `Sent`, which ends the gathering.
- **`otto-launcher --selection`** calls `Add` before its card takes the keyboard,
  so Ask opens with the selection gathered, alone or added to what is there.
- **Files' selection** is read by `Add` when no text field reports one: Files
  serves `org.otto.Files1.FocusedSelection() -> as` in each window's process
  (they queue for the name), and otto-gather asks every queued owner. Then the
  primary selection.
- **Dropping files** on the card adds them.

## Milestones

1. **Dictation as a service.** otto-dictate keeps working as it does, but the
   trigger becomes the D-Bus interface, and text reaches the app through Otto's
   own commit path instead of an input method binding. Fixes the fcitx5 clash.
2. **Selection replace.** Shortcut on a selection, type an instruction in the
   balloon, one-shot answer replaces the selection. Needs the one-shot call in
   otto-agents. This is the smallest useful agent flow.
3. **Gathering.** Start, pause, resume; multiple chips; speech and selection as
   sources; review while paused.
4. **Region and files.** The region picker, drop onto the balloon, and
   `org.otto.Files1` selection: the Files selection as a source, and Ask… in the
   palette.
5. **Files commands for agents.** Select matching, trash, open location, recent
   search and undo on `org.otto.Files1`, plus `FileManager1.ShowItems`, exposed
   to agents by the 0014 gateway. This can land before the balloon: an agent in
   Ask can already use it.
6. **Background jobs.** Send to background, progress island, notification with the
   result.

Each milestone ships on its own and gets a spec from `specs/SPEC-TEMPLATE.md`
(`specs/gather.md`, name to be settled).

## Privacy

- The on indicator is always visible while gathering, and distinct from paused.
- The mic is open only while collecting, never while paused.
- Every chip shows exactly what will be sent and can be removed.
- Nothing leaves the machine before send; a local engine keeps speech local too.

## Open questions

- **Anchoring.** At the pointer or caret, fixed as an island, or a small fixed
  indicator that opens at the caret for review and delivery.
- **Selection capture.** Every selection made while gathering, or only on an
  explicit "add" key. Automatic is smoother but picks up noise.
- **One or many.** A single gathering at a time, or several kept like drafts.
- **Surviving a pause.** Does a paused gathering survive a lock or a restart.
- **Speech as instruction or content.** "Last thing said is the prompt" is a
  guess; an explicit cue may be needed.
- **Relation to Ask.** Is the balloon the small form of Ask (no new name), or its
  own thing. The Ask card already takes a prompt and files.
- **Otto's commit path.** Protocol extension or D-Bus, and how it stays safe: only
  the gathering service may use it, and only on the field that was focused when the
  gathering started.
