# Studio

Studio is the window a picture, a PDF or a Markdown file opens in. It shows
one file, drawn exactly as [Peek](files.md#peek) draws it over the file list,
and adds what a window can carry that a Space-bar panel cannot: a toolbar,
page thumbnails, filling in and marking up a PDF, a pen for marks, a chat
with an agent about the file, and a history of the file's versions.

The program is `otto-preview`; Studio is its name everywhere you see it.

> **Early version.** It views and marks up; it does not edit pictures or
> Markdown by itself. Changes to those come from an agent, through the chat.
> A PDF's form fields and markup are the exception: those you fill in and
> save yourself.

## Opening a file

Studio is the default handler for common picture formats (PNG, JPEG, GIF,
WebP, BMP, SVG, AVIF, HEIF), PDF and Markdown. Double-click one in Files,
open it from any application, or run `xdg-open` on it, and it opens here.
From the command line:

```sh
otto-preview ~/Pictures/plan.png
otto-preview --chat ~/Documents/draft.md    # with the chat already open
```

One Studio runs per session. Opening a second file opens a second window;
opening a file that is already showing brings its window forward.

A picture opens in a window of its own shape, one pixel to a point when it
fits, and no larger than about three quarters of the screen. A PDF or a
Markdown file opens in a portrait window. Both can be resized from any edge,
and a tiled or maximized window is followed as it is. Plain text, listings,
and video open too when asked by path, with the same transport as Peek; Studio
just does not claim them as a default.

## Following the file

When the file changes on disk, Studio shows the new content by itself,
keeping the zoom, the scroll position and the page you were on. That is what
makes the chat work: an agent edits the file, and the window updates. It also
holds for an editor or a script saving next to it. A file that is deleted or
moved keeps showing what it was.

## The toolbar

Left to right:

- **Zoom out, zoom to fit, zoom in.** Greyed out for content that does not
  magnify (text, Markdown, a playing video).
- **Pages**, for a document with more than one: previous, "N / M", next.
- **Save**, while a PDF has unsaved changes.
- **Undo and redo**, once there is something to take back: unsaved PDF
  changes first, then the file's versions (see below).
- **Markup** toggles the markup strip for a PDF.
- **The eye** hides and shows the marks.
- **The pen** turns marking on and off.
- **Pages** shows and hides the thumbnail sidebar.
- **Chat** shows and hides the agent panel.

The thumbnail sidebar opens by itself for a document of more than one page
and stays shut for a single page. Clicking a thumbnail goes to that page.

Drag the titlebar or the toolbar's empty space to move the window, double
click it to zoom. The window's menus (File, Edit, Marks, View, Help) show in
the top bar while it has focus, and Help › Keyboard Shortcuts, or `Ctrl+/`,
lays a card over the window listing every command with its key.

## Getting around

| Keys | What they do |
|------|--------------|
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Zoom in, out, back to fit |
| `Ctrl` + wheel, two-finger pinch | Zoom about the pointer |
| `Page Down` / `Page Up` / `Space` | Next and previous page, or a screenful |
| Arrow keys | Move the content; `Left` and `Right` turn a page when there is nothing to pan |
| `Home` / `End` | Start and end of the document |
| `Ctrl+C` / `Ctrl+A` | Copy the selected text, select all of it |
| `Ctrl+W` / `Ctrl+Q` | Close the window |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo and redo |
| `Ctrl+K` | Show or hide the chat |
| `Ctrl+Shift+A` / `Ctrl+Shift+H` | Pen on or off, marks shown or hidden |
| `Ctrl+Shift+E` | Markup tools |
| `Ctrl+S` / `Ctrl+Shift+S` | Save, Save As |
| `Ctrl+/` | The shortcuts sheet |

`Cmd` works wherever `Ctrl` does. Wheel and two-finger scroll pan a zoomed
picture or a document; dragging with the primary button moves it too.

Text is selectable: a PDF's own text layer, or the words recognised in a
picture when `tesseract` is installed (see [Peek](files.md#peek)). A click on
a Markdown link opens it with the desktop's opener.

## Filling in and marking up a PDF

A PDF's form fields are live as soon as the window shows it. Each fillable
field has a faint accent tint, stronger under the pointer and outlined when
the form marks it required.

- **Text fields.** Click to put a caret in the field, type, then `Return` or
  click elsewhere to write it; `Escape` drops it. `Tab` and `Shift+Tab` move
  to the next and previous field, turning the page when needed.
  `Shift+Return` makes a new line in a multi-line field.
- **Checkboxes and radio buttons** toggle with a click. A choice field drops
  down its options.
- **Signature fields.** A click places your signature, fitted to the field.
  The first time, a pad opens to draw it on; it is kept for later.

The markup button, or `Ctrl+Shift+E`, shows a strip of tools under the
toolbar: Select, Highlight, Underline, Strike Out, Draw, Text and Signature,
six colours, and a bin.

- **Highlight, Underline and Strike Out** work on the PDF's own text: drag
  over words and release. Picking one of them with words already selected
  marks them at once.
- **Draw** makes a freehand stroke; **Text** puts a text box where you click;
  **Signature** places your drawn signature, centred on the click.
- **Select** picks up anything on the page: click to select it, drag to move
  it (`Shift` keeps to one axis), a colour swatch recolours it, the bin or
  `Backspace` deletes it. Double-click a text box to edit its text.

Nothing is written until you save. The title reads "NAME (edited)"
meanwhile, and `Ctrl+Z` takes changes back one at a time. **Save** writes the
changes into the file itself, so any PDF reader shows them; a signed PDF is
saved as an incremental update so its signature stays valid for what it
covers. **Save As** (`Ctrl+Shift+S`) writes a copy and carries on with it.

If another application changed the file while you were working, saving asks
first: Save Anyway, Save As, or Cancel. Closing a window with unsaved changes
asks too.

What the agent fills in or adds through the chat is the same kind of field
value or markup: you can change it, move it, and it is unsaved until you
save, like your own.

Filling and markup need PDFium, which Otto's packages install as
`/usr/lib/otto/libpdfium.so`. Without it a PDF still shows, with no form
tint and no markup button. The same holds for a PDF with a user password.

## Marks

Marks are for pointing at things when you talk to an agent, and they are not
written into the file. With the pen on, a drag on a picture or a page draws a
freehand mark, a drag with `Shift` a box. `Escape` puts the pen down.

Your marks are red and numbered in the order you drew them; the agent's are
blue, with its label. Each mark's badge is its handle: drag it to move the
mark, hover it and click the cross beside it to delete it. `Backspace` takes
back your last mark not yet sent. The eye hides every mark so you can see the
document as it is.

Marks stay on what they mark through zoom, scrolling and the file being
reloaded. Once the chat has a session, they are kept with it and come back
when the session is opened in Studio again.

## Chat

The chat button, or `Ctrl+K`, opens the [Ask](agents.md) panel at the side of
the window. A floating window grows to make room so the document keeps its
size; a tiled or maximized one narrows the document instead. Drag the panel's
edge to make it wider or narrower.

The conversation is about the file beside it. The agent knows which file it
is and has tools to look at the window as you see it, read your marks, draw
marks of its own, lay a picture over part of the document, reload the file
after editing it, and, for a PDF, read and fill the form and add markup.
Your marks not yet sent go with your next message; a line over the field
says which.

The same keys as in Ask apply: `Return` sends, `Up` and `Down` pick an answer
when the agent asks a question, `Ctrl+C` stops a running turn, `Ctrl+D`
dictates into the field. `Escape` in the field, or a click on the document,
gives the keyboard back to the document. Hiding the panel keeps the
conversation for as long as the window is open.

Sessions started here show up in the agents list with the file as their
subject, and opening one from there opens the file in Studio with that
conversation beside it.

## Versions

From the first message sent in the chat, Studio keeps the file's versions:
the file as it was then, and the file each time it changes after that,
whoever changed it. Each save of a PDF is a version too, the file as it was
before the first save included.

Undo and redo in the toolbar, or `Ctrl+Z` and `Ctrl+Shift+Z` with the
document focused, step back and forward through them: the version is put
back in the file's place and the window follows. A change after stepping
back drops the versions in front of it. Up to 50 are kept under
`~/.local/state/otto-preview/versions/`, and they survive closing the window.

## Not there yet

- Editing pictures and Markdown in the window itself.
- Signing a PDF with a certificate. The signature tool places a drawn
  signature; it does not sign cryptographically.
- Browsing: no stepping through a folder, no thumbnails of other files.
  Another file is another window.
