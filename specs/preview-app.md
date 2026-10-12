# Preview

**Status:** draft  
**Related specs:** [peek.md](peek.md), [peek-ocr.md](peek-ocr.md), [window-decorations.md](window-decorations.md), [open-with.md](open-with.md), [localisation.md](localisation.md)

## Summary

Preview is an ordinary application window that shows one file, drawn exactly
as Peek draws it over the file list. It is what opens a picture, a PDF or a
Markdown document from anywhere that is not Files' Space bar: a double-click
in another app, `xdg-open`, the command line.

People know it as **Studio**: that is its name in the desktop entry, the top
bar, the dock and the agents' "Open in Studio" button, in every language.
Underneath it is still `otto-preview` (the program, its app id and the
`org.otto.Preview1` bus name), and this spec calls it Preview.

## Goals

- `otto-preview PATH` opens one window on `PATH`, titled with the file's name.
- One Preview runs per session. The first start owns the session-bus name
  `org.otto.Preview1`; a later start calls its `Open(path, token)` and exits.
  The running Preview opens the file in a new window, or, when a window
  already shows that file (links resolved), brings that window forward
  through xdg-activation — with the launcher's `XDG_ACTIVATION_TOKEN` when
  there is one, else with a token it requests itself, which Otto honours
  right after a press.
- The content is indistinguishable from Peek's panel for the same file:
  pictures (animated ones play), SVG, PDF as a scrolling strip of pages
  rasterised as they come into view, Markdown with working links, text,
  listings, cards, video with its transport, and selectable text (a PDF's own
  text layer, or words recognised in a picture).
- The window wears the same chrome as other Otto windows: a titlebar with
  the traffic lights on the side the desktop puts them, the name centred,
  and a toolbar under it. The window is opaque everywhere but its rounded
  corners: no blur and no translucent material anywhere in it. Its ground is
  Peek's, in its solid form, so nothing behind the window shows through the
  margins around a picture.
- The window is installed as the handler for common picture types, PDF and
  Markdown, and nothing else.

## Non-Goals

- Stepping through a folder, a sidebar, thumbnails or any browsing: another
  file is another window.
- Editing pictures and Markdown, rotating, or saving anything from the
  window's own tools: those changes come from an agent, through the chat (see
  below and `components/otto-agents/docs/plans/0016-agent-edited-documents.md`).
  A PDF's form and markup are the exception (see *Form and markup*).
- Digital signatures (signing with a certificate). The signature tool places
  a drawn signature; certificate signing is a later step of its own.
- Claiming plain text, archives, audio or video as a default handler. Preview
  can show them when asked by path, but does not advertise them.

## Behavior

### Opening

- The window opens at once, showing the file's icon and "Opening preview…",
  and the decoded content replaces that when the sandboxed decoder answers.
- A picture opens in a window of its own shape, with the chrome on top,
  fitted into 1100 × 800 points and into 75 % of the width and 80 % of the
  height of the room the compositor suggests for a new window (its configure
  bounds). A picture smaller than that opens at its own size, one pixel to a
  point. A PDF or Markdown file opens in a portrait 760 × 860 window;
  anything else in a general-purpose 960 × 720 one; both shrink to the same
  limit. The window is never smaller than 480 × 360.
- The size is settled on the first configure. A compositor that sizes the
  window itself (a tile, a maximized window) is followed as it is.
- A path that does not exist or cannot be read still opens a window, showing
  the reason in place of the content, as Peek does.
- `--version` and `--help` print and exit without connecting to the display.
  With no path the usage is printed and the exit status is 2.
- The window's app id is `otto-preview`, matching its desktop entry.

### Following the file

- When the file changes on disk (an agent, an editor or a script saved it),
  the window shows it again by itself. The folder is watched rather than the
  file, so a tool that saves by writing a new file and renaming it over the
  old one is followed too.
- A change is the file's inode, size or modification time moving; another file
  changing in the same folder does nothing. Bursts are taken once, after
  100 ms.
- The old content stays on screen until the new decode replaces it, and the
  zoom, the scroll and the page showing are kept.
- A file deleted or moved away keeps showing what it was.

### Titlebar

- The file's name is centred, ellipsised to stay clear of the lights.
- The traffic lights are the kit's: close and minimise, and zoom only when
  the desktop shows the zoom dot (`show_maximize_button`), as in every other
  Otto window. Close closes that window, and closing the last one quits the
  process; minimise and zoom do what they do
  everywhere.
- Dragging the bar (or the toolbar's empty space) moves the window; a double
  click zooms it. The window edges resize it.
- When the window is tiled the bar follows the tile decoration setting, as
  every Otto window does.

### Toolbar

- Leading: zoom out, zoom to fit, zoom in. They are disabled for previews
  that do not magnify (text, listings, cards, Markdown, a playing video, and
  while the decode is in flight); zoom out and fit are disabled at fit, and
  zoom in at the maximum.
- Centre, only for a document of more than one page: previous page, "N / M"
  showing the page with most of the window, next page. Each button is
  disabled at its end.
- Trailing: Save, while a PDF has unsaved changes (see *Form and markup*);
  undo and redo, once the file has more than one version (see *Versions*)
  or there are unsaved changes to take back; the markup button, for a PDF that can be edited (see *Form
  and markup*), a toggle that stays down while the markup strip shows; the
  eye, a toggle that stays down (struck through) while the
  marks are hidden; the pen, a toggle that stays down while it is on (see
  *Marks*); and the chat button, a toggle that stays down while the chat
  shows.
- A button fires when the press and the release both land on it.
- There is no button to open the file in another application: Preview is
  the viewer for the types it handles.

### Chat

- The chat button or Ctrl+K shows a 360-point panel at the window's trailing
  edge, under the toolbar. A floating window with room to spare grows by the
  panel's width, so the document keeps its size, and shrinks back when the
  chat hides; a tiled, maximized or too-wide window narrows the document
  instead, keeping its place. Hiding the panel keeps the conversation for as
  long as the window is open.
- Dragging the panel's leading edge (a column-resize cursor) widens or
  narrows it, from 280 to 720 points and leaving the document at least 240;
  the document keeps its place. Windows opened after take the last width
  dragged to.
- `--chat` opens the file with the chat showing; `--session URI` carries on
  that agent session beside the file (a session this app started, picked
  from a list) and implies `--chat`. A later start hands both over the bus
  (`OpenChat`, `OpenSession`).
- The panel is the Ask chat from otto-agents-kit, as in the launcher: the log
  on top, the answers to a waiting question or input request above the
  field, and the field at the bottom ("Ask about NAME"). The field is filled
  a step lighter than the panel, and its text starts on the log's left edge,
  at the log's size, so the two read as one column. The footer names the
  agent Studio.
- Long words in the log, paths and addresses, break after a `/`, `-`, `_` or
  `.` rather than run past the edge. A tool call shows its first line, in
  at most two lines; closed, a run of calls is the last of them on one line.
- Ctrl+D dictates into the field, with the keys of an agent chat in
  [dictation.md](./dictation.md#dictation-in-an-agent-chat). Return during a
  dictation sends once the last words are in, with the marks not yet sent,
  as Return does. Giving the keyboard back to the document stops the
  dictation without sending.
- Opening the panel connects to otto-agents in the background; nothing
  reaches an agent until something is sent. Hiding and showing it again
  connects again when the service couldn't be reached.
- The first message creates the session with `_meta.otto`: `app`
  (`otto-preview`) and `subject` (the file's URI), so lists open it here
  again; `agent` (`studio`), the plugin agent for working on one file, so
  Otto's desktop helper stays out of it; `instructions`, which otto-agents hands the agent ahead of its first
  turn (it is working on the file shown beside the chat, edits it in place
  and calls `preview_reload`); and `mcpServers`, this program in `--mcp` mode
  with the file in `OTTO_PREVIEW_DOC` (see *Document tools*).
- The file goes with the first message, but is never shown as an attachment,
  since it is open beside the chat. Marks going with a message are not shown
  as attachments either.
- Pictures the agent gets from Preview's own tools (`preview_render`) or from
  reading the file itself stay with the agent and out of the log: they are
  the window beside it again.
- Return sends. Up and Down pick an answer while one is waited for; Return
  or a click gives it. Ctrl+C stops a running turn, or copies the selection.
- A press in the panel gives it the keyboard; a press on the document, or
  Escape in the field, gives the keyboard back to the document. Ctrl+C with
  text selected in the log copies it, wherever the keyboard is.
- When otto-agents can't be reached, the panel says why.

### Marks

- The pen turns marking on: a drag on a picture or a page draws a freehand
  mark, a drag with Shift a box, and the cursor is a pencil. Escape turns
  the pen off. Text, listings and video can't be marked.
- Marks are kept in the document's own units, a picture's pixels or a page's
  PDF points, so they stay on what they mark through zoom, scrolling, the
  chat opening and the file being reloaded.
- The person's marks are red and numbered in drawing order, the number in a
  badge at the mark's start. The agent's are blue, with its label (or a dot)
  in the badge. The agent can also lay a picture over a box of the document
  (a variant, a logo, a crop to compare), at an opacity it chooses.
- The badge is the mark's handle: dragging it (a grab cursor) moves the mark,
  pen or not, in the document's units. A mark of the person's moved after it
  was sent goes again with the next message.
- Hovering a badge shows a round cross beside it; over the cross the cursor
  is a bin, and a click deletes the mark, the person's or the agent's. Backspace takes back the person's last mark
  not yet sent.
- The eye hides every mark, to see the document as it is, and turns the pen
  off; badges can't be hovered or deleted while hidden. The marks show again
  from the eye, when the pen is turned on, or when the agent draws.
- Every mark, the person's and the agent's, is kept with the agent session
  (in its `_meta.otto.marks`) once the chat has a session, and comes back
  when the session is opened in Preview again.
- The person's marks not yet sent go with the next message: the line over
  the field, in a band of its own on the log's left edge, says which. They are written to `$XDG_RUNTIME_DIR/otto-preview/`
  as `marks-*.json` (shapes and bounds in the document's units) and, for a
  picture, `marks-*.png` (the picture with the marks drawn and numbered), and
  attached without being listed. Sent marks stay, fainter.

### Form and markup

A PDF's form is filled in, and the PDF marked up, in the window itself,
and saved into the file, so any PDF reader shows the result. The engine is
PDFium, through the `otto-pdf` crate, loaded from `libpdfium.so` (see
*Constraints*). Without it, or for a file PDFium can't open (one with a user
password included), Preview shows the PDF as before, with no form tint and
no markup button.

- **Fields.** A PDF's form fields are live whenever the window shows it: a
  faint accent tint shows each one that can be filled (stronger under the
  pointer, outlined when the form marks it required). A click on a text
  field puts a caret in it, over the field and in the page's own scale;
  Return or a click elsewhere writes it, Escape drops it, Tab and Shift+Tab
  write it and go to the next or previous text field, turning the page if
  that field is off screen. Shift+Return makes a new line in a field that
  takes several. A click on a checkbox ticks or unticks it; on a radio button
  ticks it and unticks the rest of its group. A choice or list field drops
  down its options as a menu from the field. A click on an empty signature
  field places the person's signature fitted to it. Read-only fields, push
  buttons and signed signature fields take no input.
- **The markup strip.** The markup button (or Edit › Markup Tools,
  Ctrl+Shift+E) shows a strip under the toolbar. It holds Select, then
  Highlight, Underline, Strike Out, Draw, Text and Signature; six colours,
  highlighter yellow first; and a bin that deletes what is selected. Taking
  up the pen (see *Marks*) puts the strip away, and the strip puts the pen
  down: both want the press.
- **Highlight, Underline, Strike Out** work on the PDF's own text: a drag
  selects words as usual and the release marks them, one quad per line, in
  the chosen colour. Picking one of these tools with words already selected
  marks them at once.
- **Draw** makes a freehand stroke, 1.5 pt wide in the chosen colour, written
  as an ink annotation on release; a click is a dot.
- **Text** puts a caret where the page is clicked; what is typed is written
  as a text box (a FreeText annotation, 12 pt) on Return or a click
  elsewhere. Text in highlighter yellow is written in the darkest colour
  instead, to be readable.
- **Signature**: the first use opens a pad over the window to draw the
  signature on (Clear, Cancel, Done; Escape cancels). It is kept as strokes
  in `$XDG_DATA_HOME/otto/signature.json` and placed after that with a
  click, 150 pt wide and centred on it, as dark blue ink.
- **Select** picks up what is on the page: a click on a highlight, drawing,
  text box, note or stamp selects it (an accent outline); a drag moves it
  (Shift keeps the move to one axis); a colour swatch recolours it; the bin,
  Backspace, Delete or Edit › Delete Selection deletes it; Escape lets it go.
  A double-click on a text box edits its text; emptying it deletes it.
  Annotations the agent added are selected, moved and deleted the same way;
  locked or read-only ones are not.
- **One at a time, in order.** Every change, undo, redo and save of a
  document waits its turn in one queue of its own and runs on its thread, in
  the order it was asked for, the person's and the agent's alike: typing
  into a field then pressing Ctrl+S saves the text typed, and a quick undo
  then redo leaves the change made. A change names fields and annotations by
  their place on the page as the window saw them; when a delete or an undo
  has renumbered the page since, each is found again by what it is, and a
  change to something no longer there is refused, never made on what took
  its place. A change that fails is not kept: whatever part of it was made
  is taken out again, and the window shows the document as it really is.
  A dot by the bin shows while the queue is working.
- **Saving.** Changes are made in memory and shown at once; the file is
  written only when the person saves: the Save button (in the toolbar while
  there are unsaved changes), File › Save, or Ctrl+S. Until then the title
  reads "NAME (edited)". A save goes through a new temporary file beside the
  original, with an unguessable name, never opened through a link or over a
  file already there, given the file's permissions from the start, then
  renamed into place; encryption is kept, and a signed PDF is saved as an
  incremental update, so its signature stays valid for what it covers. Each
  save is kept as a version (see *Versions*), the file as it was before the
  first save included.
- **A file changed on disk.** Preview remembers the file's stamp (inode,
  size, modification time) when it opens it and each time it saves it. A
  save that finds the stamp changed (another app saved the file meanwhile)
  writes nothing and asks "NAME changed on disk" in the question popup, with
  Save Anyway, Cancel and Save As… (the default): Save Anyway writes over it,
  Save As… asks where to save a copy.
- **A save that fails** (no room, no permission, a folder gone) is asked
  about the same way: "NAME couldn't be saved", the reason on a line of its
  own, with Cancel and Save As…. The changes stay in the window, unsaved.
- **Undo and Redo** take back and make again the changes not yet saved, by
  opening the file as it was last opened or saved, kept in memory, and
  making all but the last; so they work whatever the file on disk has become
  since. Once there are none they step through the file's versions as
  before. A save clears them.
- **Save As** (File › Save As…, Ctrl+Shift+S) asks for a name and a folder
  in the desktop's save dialog (the file-chooser portal's `SaveFile`,
  starting on the file's own name and folder, PDFs only; a name without an
  extension gets `.pdf`), writes the document there with its changes, and
  the window carries on with that file: its title, its folder watched, its
  own versions. The file it was opened from is left as it was saved last.
- **Closing with unsaved changes** asks "Save changes to NAME?" with Don't
  Save, Cancel and Save, in a popup of its own over the middle of the window,
  rounded, blurred and casting the same shadow as a menu (Return gives the
  accent button's answer, Escape or a click outside cancels). Save closes the
  window once the file is written, or at once when nothing is left to write
  and no save is in flight; a close waits only on that save, never on a
  later one. If the save asks about a change on disk
  or fails, the window closes once the answer's save (or Save As) is done,
  and stays open on Cancel. Ctrl+W, the close light and the compositor's
  close all ask.
- **Redrawing.** The pages a change touched (every page a field's name is
  on, for a field) and their thumbnails are drawn again by PDFium from the
  document in memory and put in place of the old drawing, so a change shows
  in a few tens of milliseconds and nothing blinks. Text just written is
  drawn over its field until then. The file changing because of this
  window's own save is not taken for someone else's change, and a change on
  disk from anywhere else is ignored while there are unsaved changes (or
  changes still in the queue), until a save asks about it; otherwise it
  reopens the document.
- **The agent** reads and changes the same form and annotations with
  `preview_fields`, `preview_annotations` and `preview_edit` (see *Document
  tools*), so what it fills in is an ordinary field value the person can
  change, and what it adds is markup the person can select, move and delete.
  Its changes are unsaved like the person's, and wait in the same queue: the
  person saves them. Numbers no page holds (not finite, beyond ±100 000
  points, a width or text size out of bounds) and text past 100 000
  characters are refused before anything is made. `preview_reload` and
  `preview_revert` are refused while the window holds unsaved changes, which
  they would throw away; the agent asks the person to save (or undo) them.

### Versions

- From the first message sent in the chat, Preview keeps the file's
  versions: the file as it was then, and the file each time it changes after
  that, whoever changed it. They are kept under
  `$XDG_STATE_HOME/otto-preview/versions/`, survive closing the window, and
  are taken up again when the file is opened later.
- Undo and redo in the toolbar, or Ctrl+Z and Ctrl+Shift+Z with the document
  focused, step back and forward: the version is copied over the file
  (through a file renamed into place), and the window follows the change as
  for any other.
- The file matching a version already kept is that version, not a new one. A
  change after stepping back drops the versions stepped back from.
- At most 50 versions are kept; the oldest go first, except the first.

### Document tools

- `org.otto.Preview1` also offers, for the window showing a path: `Info`
  (what it shows, as JSON), `Marks` (every mark, as JSON), `Draw` (replace
  one of the agent's named layers of marks), `Clear` (a layer, every agent
  mark, or the person's marks), `Reload` (read the file again, keeping the
  view, and keep it as a version with the agent's note), `Versions` (the
  versions, as JSON), `Revert` (put one back; it and `Reload` fail while a
  PDF has unsaved changes), `Render` (the picture as shown, with or without marks, as a PNG
  path), and for a PDF that can be edited `Fields` (its form, as JSON, one
  entry per field name with every place it shows), `Annotations` (its
  annotations, each with an id `page:index`) and `Edit` (fill fields by name,
  add, change and delete annotations, from JSON, shown and left unsaved for
  the person to save; it waits its turn behind the person's changes and
  returns the fields and annotations after, or why the change was refused). A path no window shows is an error.
- `otto-preview --mcp` is a stdio MCP server over those methods, for the
  file in `OTTO_PREVIEW_DOC`: `preview_info`, `preview_marks`,
  `preview_draw`, `preview_clear`, `preview_reload` (with a note),
  `preview_versions`, `preview_revert`, `preview_render`
  (which returns the image), `preview_fields`, `preview_annotations` and
  `preview_edit`. Its `initialize` answer carries the same
  instructions as the session. It never connects to the display.

### Content input

- Wheel and two-finger scroll pan a zoomed picture or a document, with the
  same momentum, rubber band and scroll bars as Peek; they scroll text and
  listings by rows.
- Ctrl + wheel and a two-finger pinch zoom about the pointer.
- Dragging a zoomed picture or a document with the primary button moves it.
- A press on a recognised word starts a selection; Ctrl+C copies it, Ctrl+A
  selects every word, Escape clears it.
- A click on a Markdown link opens it with the desktop's opener; a relative
  link is resolved against the document's folder.
- A video plays on opening; its transport works as in Peek, and Space
  toggles playback.

### Keys

- Ctrl+= / Ctrl++ zoom in, Ctrl+- zoom out, Ctrl+0 back to fit.
- Page Down / Page Up (and Space) turn a page of a document, and otherwise
  move by a screenful.
- Arrow keys move the content by a step; Left and Right turn a page when
  there is nothing to pan sideways.
- Home and End go to the start and the end.
- Ctrl+W and Ctrl+Q close the window, asking first when a PDF has unsaved
  changes; Ctrl+S saves them, Ctrl+Shift+S saves them as another file.
- Ctrl+Z steps the file back a version and Ctrl+Y (or Ctrl+Shift+Z) forward.
- From anywhere, the chat included: Ctrl+K shows or hides the chat,
  Ctrl+Shift+A takes up or puts down the pen, Ctrl+Shift+H shows or hides
  the marks, Ctrl+Shift+E shows or hides the markup strip, and Ctrl+/ shows
  the keyboard shortcuts sheet.
- While text is being typed on the page, every key goes to it, before any of
  the window's own.
- Cmd works wherever Ctrl does: Otto hands it to apps as either.

### Menus and the shortcuts sheet

- The window's menus show in the top bar while it has focus (served over
  `com.canonical.dbusmenu`, pointed at with `org_kde_kwin_appmenu`; see
  `otto_kit::app_menu`): File (Save, Save As…, Close Window), Edit (Undo, Redo, Copy, Select
  All, Markup Tools, Delete Selection), Marks (Pen, Show Marks, Delete Last Mark), View (zoom, pages, Show
  Pages, Show Chat) and Help (Keyboard Shortcuts).
- Each item shows its key, and is greyed out when it cannot run. Pen, Show
  Marks, Show Pages and Show Chat are toggles and say whether they are on.
- One list of commands (`commands.rs`) feeds the menus, the keys and the
  sheet, so a key the menu shows is a key that works.
- Help › Keyboard Shortcuts, or Ctrl+/, lays a card over the window listing
  every command with its key, then the keys that are in no menu (the
  document's and the chat's). Escape or a click puts it away.

### Resizing

- A resize lays the content out again. Nothing is decoded again (unless the
  file changes, as above), except that
  a document asks for the pages newly in view, at the width they are now
  drawn at, as scrolling does.

## Constraints & Edge Cases

- The decoder is the same sandboxed worker Peek uses, which is this binary
  re-executed; the program must hand control to it before doing anything
  else.
- The decode is sized for the window as it is when first configured. Growing
  the window afterwards shows the same pixels larger, up to the headroom the
  decode was given.
- A document's pages are rasterised at most three at a time, so a fast
  scroll does not queue work for pages already gone past.
- Text recognition follows the `[peek]` section of `files.toml`: off there
  means off here, and words already recognised for a picture are shown at
  once from the shared cache.
- Measuring a picture's size for the first window reads only its header, and
  never from anything that is not a regular file (a pipe would block).
- `libpdfium.so` is looked for at `$OTTO_PDFIUM`, beside the executable, in
  `../lib/otto/` from it, `/usr/lib/otto/`, `/usr/local/lib/otto/`,
  `~/.local/lib/otto/`, then by the system loader. The packages install a
  prebuilt one at `/usr/lib/otto/libpdfium.so`
  (`scripts/packaging/fetch-pdfium.sh` pins it). PDFium is not thread-safe:
  one thread owns it and every open document.
- A text field is edited on one line; a field that takes several shows the
  new lines once written.

## Rationale

- One file per window keeps the app a viewer rather than a second file
  browser; stepping through a folder is what Files and Peek are for.
- One process for every window: opening the same file twice brings its
  window back rather than stacking copies, and each further file costs a
  window, not another renderer, font set and runtime.
- The preview state and drawing are Peek's, reused rather than copied, so the
  two can never show the same file differently. Both apps embed the same
  session from `otto-peek`; neither depends on the other.
- The window is opaque because it is a document window: a translucent
  material behind a photograph or a page only muddies what is being looked at.
- Preview offers no "open in" button because it is the default viewer for
  the types it shows; handing the file on would usually mean handing it back
  to Preview.

## Open Questions

- Should the toolbar grow Rotate, Share or Print?
- Signing with a certificate: PDFium reads signatures but can't make them,
  so it needs a signing step of its own, appended after every other change.
- Should the decode be repeated at a larger size when the window is made much
  bigger than it opened at?
