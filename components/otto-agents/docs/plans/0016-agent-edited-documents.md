# 0016: Documents you look at, agents edit

**Status:** Idea

## Goal

Preview becomes the app that opens almost anything: pictures and RAW photos,
PDF, Markdown, text, tables, video, audio. It has no editing tools. Editing is a
conversation: Preview has the Ask chat beside the document, and the agent does
the work with command-line tools (RawTherapee, darktable, ImageMagick, ffmpeg,
pandoc, qpdf, …) taught by skills. You watch the result appear in the viewer,
point at what you mean, step back through versions, and save when you like it.

```
┌──────────────────────────────┬──────────────────────┐
│                              │ Ask                  │
│   IMG_0412.cr3               │                      │
│   [ before | after ]         │ > warmer, and lift   │
│                              │   the shadows here ⬚ │
│   ◀  v6   v7   v8  ▶         │ ✓ WB +300K,          │
│                              │   shadows +20 (v8)   │
├──────────────────────────────┤                      │
│ aside: histogram             │                      │
└──────────────────────────────┴──────────────────────┘
```

Three things make this more than "an image next to a chat":

- **Edits go through the app,** so every edit is a version with its recipe, and
  undo works the same for you and the agent.
- **The agent can show things,** not only change the document: a histogram, a
  table, four variants to pick from, marks drawn on the document.
- **You can point by drawing,** and your marks go to the agent with your
  message. The agent draws on the same layer to point back.

## Decisions

- **The app shows, the agent edits.** Preview gains no editing tools of its own.
  Its job is rendering many formats well, keeping versions, and taking pointing as
  input. New editing abilities come from skills, not from app code.
- **Edits go through Preview, never straight to the file.** This is 0014's test
  for a desktop tool: the app adds what the shell doesn't have, here versions,
  undo and showing you the result. The original stays untouched until you save.
- **Preview speaks D-Bus, the gateway speaks agents** (as 0014). Preview offers
  document methods on `org.otto.Preview1`; `otto-mcp` turns them into tools from
  a `preview.toml` tool list. Preview has no agent-specific code beyond hosting
  the chat.
- **Document sessions are ordinary sessions.** Same service, same AHP, same
  history and permissions. Only where they open is different (see below).
- **The chat is shared, not copied.** The Ask model and chat drawing move out of
  the launcher into `otto-agents-kit`, which both apps use (see 0017).
- **The chat is a sidebar you open.** A toolbar button (and a shortcut) toggles
  it; Preview opens as a plain viewer without it. The session starts when you
  first send, so opening the panel costs nothing.
- **One mark layer, two authors.** Your strokes and the agent's drawings are the
  same kind of thing, in the same format and coordinates, so either side can
  refer to the other's marks.
- **The viewer is the generic output surface.** Anything the agent wants to show
  is a file in a format Preview already renders. No separate widget vocabulary.

## Document sessions

A session opened from Preview, or about a file Preview owns, carries:

```jsonc
"_meta": { "otto": {
  "app": "otto-preview",                       // who opens this session
  "subject": ["file:///home/me/Photos/IMG_0412.cr3"],
  "document": "<document id>"                  // versions, see below
}}
```

- **The Sessions panel** shows such a session with a thumbnail of its subject
  and opens it in Preview instead of running `otto-launcher --session`.
  `org.otto.Preview1` gets `OpenSession(uri)`, on top of `Open(path)` from the
  single-instance branch.
- **`otto.app` is a session handler, not a special case.** The panel looks the
  handler up (a desktop-file key such as `X-Otto-Session-Handler=true`, or a
  D-Bus name), the way a file manager looks up a MIME handler. The terminal
  handoff (`otto.terminal`) is the first handler; Preview is the second.
- **The launcher still opens it** as a plain chat: it is text and file links. It
  offers "Open in Preview", like the terminal handoff.
- **Several clients at once** work as AHP already allows: chat in the launcher,
  document in Preview, both live.
- **Opening a file that has sessions** offers to continue the latest one or start
  a new one.

## Document tools

Preview serves these on `org.otto.Preview1`; the gateway's `preview.toml` lists
them as `preview_*` tools. A document is addressed by the session's document id,
which the gateway fills in from `--session`, so an agent only reaches the
document of its own session.

**Editing the document**

| Method | Tool | What it does |
|---|---|---|
| `Info()` | `preview_info`, read-only | Subject, format, current version, viewer state (page, zoom, time) and the current selection |
| `Workspace()` | `preview_workspace` | A scratch folder holding a copy of the current version. The agent edits there with any tool |
| `Commit(path, recipe, note)` | `preview_commit` | Takes a file from the workspace as a new version, with the recipe that made it and a one-line note. The viewer shows it |
| `Write(text)`, `Patch(diff)` | `preview_write`, `preview_patch` | New version of a text document (Markdown, code, CSV) without the scratch step |
| `Versions()`, `Revert(v)` | `preview_versions`, `preview_revert` | The ◀ ▶ list, and jumping in it |
| `Undo()`, `Redo()` | `preview_undo`, `preview_redo` | Move the current version |
| `Save()`, `Export(path, format)` | `preview_save` (destructive), `preview_export` | The only writes outside the version store |

**Showing things**

| Method | Tool | What it does |
|---|---|---|
| `Show(source, slot, title, id)` | `preview_show` | Shows a file (or small inline data with a MIME type) without making it a version. Same `id` again replaces it in place, so a histogram can follow the edits |
| `Draw(marks, layer)` | `preview_draw` | Draws marks (see [Marks](#marks)) on the overlay: dust spots, detected faces, an arrow, a note, a sentence in a PDF. Same `layer` again redraws it in place, so a detection can follow the edits |
| `Marks(by) → marks` | `preview_marks`, read-only | The marks on the document, yours and the agent's, with their numbers |
| `Propose(mark, prompt) → mark` | `preview_propose` | Draws a mark you can adjust (a crop, a selection) and returns it as you confirm it. `Choose` for geometry. The call waits |
| `Clear(id)` | `preview_clear` | Removes a shown item or a layer of marks |
| `Choose(items, prompt) → pick` | `preview_choose` | Shows options (variants, crops) and returns the one you click. The call waits for the click |
| `Render(page, region, marks) → image` | `preview_render`, read-only | What the viewer actually shows, as a picture, with or without the marks, so the agent can check its result as you see it |

Slots for `Show`:

- **aside**: a panel under or beside the document. The default: histograms,
  tables, notes.
- **main**: replaces the document for a while, with a way back. Variant grids,
  long reports.
- **overlay**: on top of the document. `Draw` is the usual way in.

Shown items are kept with the session, so reopening it brings them back. The chat
gets a short reference ("Histogram, in the side panel") so the history reads
right in the launcher too.

Charts are SVG the agent writes (a few lines of Python). A chart format is for
later, if SVG turns out awkward.

## Versions and recipes

- **Store.** `$XDG_STATE_HOME/otto/documents/<id>/`: an index, the version files,
  and each version's recipe (a `.pp3`, an ffmpeg command line, a script). Not
  next to the original: no clutter in your folders.
- **Version 0 is the original**, read in place, never copied unless it changes
  behind the app's back.
- **History is a tree, shown as a line.** Committing after an undo starts a
  branch; nothing is lost, and the ◀ ▶ strip shows the current line.
- **One history, two drivers.** Ctrl+Z in Preview and `preview_undo` move the same
  pointer. When you step back, the agent learns it on its next turn: a line of
  context ("you went back to v5") and `preview_info`.
- **Attribution** uses 0014's `origin`: each version records whether it came from
  you, a session (by title) or outside. The agent's `Undo` passes its origin and
  only takes back its own versions, as with Files.
- **Recipes make it repeatable.** The same recipe applies to the next 200 photos;
  that is a batch job for the agent, not an app feature.
- **Changes from outside.** Preview watches the subject. A change it did not
  make becomes a version with origin "outside", so nothing is lost and the viewer
  stays current.

## Keeping the agent to the tools

- **Skills say it.** Every editing skill follows one loop: `preview_workspace` →
  command-line tool → `preview_show` while trying → `preview_commit` when it's a
  step worth keeping. Never write the subject file.
- **Permissions enforce it.** In a document session, otto-agents denies writes to
  the subject and allows them in the workspace. `preview_save` is the one tool
  that touches the original, and it asks.

## Marks

The overlay is a vector layer over the document. You draw on it to point; the
agent draws on it to show and to point back.

**Your drawing tools:** pen, box, lasso, brush (for masks), arrow, text note,
eraser. The selections you already make (a text selection, a page, a time range,
a cell range) become marks too.

**Pending marks** appear as chips in the chat input ("② region, 120×80") and go
out with your next message. Drawing while the chat is closed opens it. After
sending, the marks stay in the session history, dimmed, and can be shown again.

**Every mark gets a number,** drawn as a badge, so "brighten 1, remove 2" works,
and so does the agent's "2 looks like a sensor spot".

```jsonc
{ "id": "m2", "n": 2, "by": "user",            // or "agent"
  "anchor": { "version": 7, "page": 3 },       // or "time": 12.4 for video
  "shape": { "rect": [x, y, w, h] },           // ellipse, polygon, path + width,
                                               // arrow [from, to], text + at,
                                               // mask (a PNG in the store)
  "style": { "color": "#ff3b30", "fill": 0.2 },
  "label": "dust?",
  "text": "…"                                  // the text under it, for PDF/Markdown
}
```

Coordinates are in document space (source pixels for pictures, PDF points per
page), so marks stay put through zoom, pan and new versions.

### What the model gets

Models read pointing best in more than one form, so a message with marks
carries all of these:

1. **The view with the marks burned in,** numbers included. Vision models follow
   drawn circles and arrows well, and this works with any multimodal model.
2. **The clean document and a crop of each mark** at full resolution, so detail
   isn't hidden under the strokes.
3. **The marks as JSON,** with coordinates also given as fractions of the
   document. Tools act on these, not on pixels: a crop, a darktable or
   RawTherapee mask, an inpainting call that takes an image and a mask, or a
   segmentation model refining a rough lasso into an object.
4. **For text documents, the text under the strokes.** A stroke across a PDF
   paragraph or a Markdown section snaps to the text it covers and goes as a
   quote with its page and position, not as a doodle over glyphs.

`preview_marks` and `preview_render(marks: true)` give the agent the same views
later, so it can look again without you resending.

## Formats

Peek already renders pictures, SVG, PDF, Markdown, text, listings and video.
Next, in rough order of value for editing:

1. **RAW photos:** the embedded preview at once, then a real render.
2. **Tables:** CSV and TSV.
3. **Diffs and code.**
4. **Audio:** a waveform and playback.
5. **Office documents:** converted to PDF for viewing.

A format Preview can't render is the agent's job: it converts it to one Preview
can show.

## Skills

A skill pack, installed with the others through `npx skills`. One skill per
domain, each built on command-line tools and the loop above:

| Skill | Tools |
|---|---|
| Photos | RawTherapee (`rawtherapee-cli`, `.pp3` is plain text), darktable (`darktable-cli`, styles and XMP), ImageMagick |
| Video | ffmpeg |
| Audio | sox, ffmpeg |
| PDF | qpdf, pdftk, Ghostscript |
| Documents | pandoc |
| Vector | Inkscape command line |

RawTherapee comes first: its profiles are text an agent can read and write.
darktable keeps module settings as binary data, so an agent mostly applies
styles you made.

## Milestones

1. **Reload and ask.** Preview watches its file and reloads. An Ask button runs
   `otto-launcher --ask --file <path>`. Two windows, but the loop works today.
2. **Shared chat.** Move the `Ask` model, `log.rs` layout and the chat half of the
   launcher's `view.rs` into `otto-agents-kit`, as
   [0017](0017-agents-ui-kit.md) plans. The launcher is its first user and
   doesn't change.
3. **Chat in Preview.** The sidebar button and the panel, starting a document
   session with the subject attached on first send.
4. **Versions and the editing tools.** The store, `Workspace`, `Commit`,
   `Write`/`Patch`, `Versions`, `Undo`/`Redo`, `Save`/`Export`, the ◀ ▶ strip and
   before/after, `preview.toml` in the gateway, the permission rule.
5. **Document sessions in the Sessions panel.** `otto.app`, the handler lookup,
   `OpenSession`, thumbnails, "Open in Preview" in the launcher.
6. **Showing things.** `Show`, `Draw`, `Clear`, `Render`, then `Choose` and
   `Propose`.
7. **Drawing.** Your drawing tools, numbered marks, chips in the input, and the
   four forms a message carries; then text snapping, time ranges and masks.
8. **The photo skill,** then the others.

## Testing

- **Versions:** commit, undo, redo, branch after undo, revert, a change from
  outside, attribution, and `Undo` with an origin refusing someone else's
  version. Against a temp state folder, no window.
- **D-Bus:** each `org.otto.Preview1` method against a headless Preview,
  through the gateway's existing fake-bus harness.
- **Permissions:** in a document session against the echo backend, a write to
  the subject is denied, a write to the workspace is allowed, `preview_save` asks.
- **Marks:** a mark's document coordinates survive zoom and a new version; the
  burned-in render, crops and JSON agree; a stroke over a PDF line snaps to its
  text.
- **Manual:** open a RAW, ask for "warmer, lift the shadows", circle two spots and
  ask to "remove 1, brighten 2", step back twice with Ctrl+Z, ask for "three looks to pick
  from", pick one, save. Close Preview, open the session from the Sessions panel:
  same document, same version, same history.

## Open questions

- **Name.** Once it edits through an agent, "Preview" undersells it. Settle with
  the copywriter.
- **Quick actions.** Rotate, crop and flip through a model is slow and costs
  money. Buttons that run a skill's recipe directly, with no model, keep the
  editing in skills. Which ones, and does that bend "the app has no editing
  tools"?
- **Document identity.** Path, inode, content hash, or a mix: what keeps a
  document's versions when the file is moved or renamed?
- **Several subjects.** "Match the colours of these five photos": a filmstrip,
  and is each photo its own document?
- **The spec's non-goals.** `specs/preview-app.md` keeps Preview a light,
  one-file viewer. Version history isn't folder stepping, but the spec needs
  rewriting for this.
- **MCP Apps.** Real controls (sliders, crop handles) would want views the agent's
  tools bring along. MCP Apps is the standard for that, but needs a web engine
  and the gateway proxying the agent's MCP servers. Later, if `Choose` and
  marks aren't enough.
- **Masks as assets.** Keep a brush mask per version ("the sky") so later edits
  and the next photos reuse it? Likely yes for photos.
- **Where agent marks live.** With the session (the default here), or kept with
  a version when `Commit` asks?
- **Session grouping.** Document sessions with the others in the Sessions panel,
  or in their own section?
