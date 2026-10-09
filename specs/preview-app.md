# Preview

**Status:** draft  
**Related specs:** [peek.md](peek.md), [peek-ocr.md](peek-ocr.md), [window-decorations.md](window-decorations.md), [open-with.md](open-with.md), [localisation.md](localisation.md)

## Summary

Preview is an ordinary application window that shows one file, drawn exactly
as Peek draws it over the file list. It is what opens a picture, a PDF or a
Markdown document from anywhere that is not Files' Space bar: a double-click
in another app, `xdg-open`, the command line.

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
- Editing, annotating, rotating, or saving anything from the window's own
  tools. Changes come from an agent, through the chat (see below and
  `components/otto-agents/docs/plans/0016-agent-edited-documents.md`).
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
- Trailing: the eye, a toggle that stays down (struck through) while the
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
- `--chat` opens the file with the chat showing; `--session URI` carries on
  that agent session beside the file (a session this app started, picked
  from a list) and implies `--chat`. A later start hands both over the bus
  (`OpenChat`, `OpenSession`).
- The panel is the Ask chat from otto-agents-kit, as in the launcher: the log
  on top, the answers to a waiting question or input request above the
  field, and the field at the bottom ("Ask about NAME").
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
  in the badge.
- Hovering a badge shows a cross and a bin cursor; a click deletes the mark,
  the person's or the agent's. Backspace takes back the person's last mark
  not yet sent.
- The eye hides every mark, to see the document as it is, and turns the pen
  off; badges can't be hovered or deleted while hidden. The marks show again
  from the eye, when the pen is turned on, or when the agent draws.
- The person's marks not yet sent go with the next message: the line over
  the field says which. They are written to `$XDG_RUNTIME_DIR/otto-preview/`
  as `marks-*.json` (shapes and bounds in the document's units) and, for a
  picture, `marks-*.png` (the picture with the marks drawn and numbered), and
  attached without being listed. Sent marks stay, fainter.

### Document tools

- `org.otto.Preview1` also offers, for the window showing a path: `Info`
  (what it shows, as JSON), `Marks` (every mark, as JSON), `Draw` (replace
  one of the agent's named layers of marks), `Clear` (a layer, every agent
  mark, or the person's marks), `Reload` (read the file again, keeping the
  view) and `Render` (the picture as shown, with or without marks, as a PNG
  path). A path no window shows is an error.
- `otto-preview --mcp` is a stdio MCP server over those methods, for the
  file in `OTTO_PREVIEW_DOC`: `preview_info`, `preview_marks`,
  `preview_draw`, `preview_clear`, `preview_reload` and `preview_render`
  (which returns the image). Its `initialize` answer carries the same
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
- Ctrl+W and Ctrl+Q close the window.

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

## Rationale

- One file per window keeps the app a viewer rather than a second file
  browser; stepping through a folder is what Files and Peek are for.
- One process for every window: opening the same file twice brings its
  window back rather than stacking copies, and each further file costs a
  window, not another renderer, font set and runtime.
- The preview state and drawing are Peek's, reused rather than copied, so the
  two can never show the same file differently.
- The window is opaque because it is a document window: a translucent
  material behind a photograph or a page only muddies what is being looked at.
- Preview offers no "open in" button because it is the default viewer for
  the types it shows; handing the file on would usually mean handing it back
  to Preview.

## Open Questions

- Should the toolbar grow Rotate, Share or Print?
- Should the decode be repeated at a larger size when the window is made much
  bigger than it opened at?
