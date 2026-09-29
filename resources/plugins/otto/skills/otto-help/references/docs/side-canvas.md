# Side Canvas

The side canvas is a column that slides in over your desktop from the right
edge of the screen. Applications put panels in it, stacked top to bottom, and
it stays out of the way until you pull it in. Nothing underneath moves: your
windows stay where they are and the canvas slides over them, fullscreen
windows included.

Otto ships `otto-canvas`, which puts an Agents panel in the canvas: the
agent sessions the agent service has, the same list the launcher shows with
`otto-launcher --agents`. It starts with the session when it is listed under
`[[exec_once]]` in your config, as it is in the default one.

## Agents

Each row is a session: its title, the agent it belongs to, what it is doing
and the folder it works in. The dot on the left is in your accent colour
while the agent works, yellow when it is waiting for an answer, and grey when
it is idle.

Click a session to carry it on: the launcher opens on that conversation and
the canvas slides away. **Ask**, at the top of the panel, starts a new
request in the launcher instead.

The search field above the list has the keyboard as soon as the canvas
opens, so you can just start typing: the list narrows to the sessions whose
titles contain what you typed. The keys are the launcher's:

| Keys | What they do |
|------|--------------|
| Down, Up (or Ctrl+N, Ctrl+P, Tab, Shift+Tab) | Move the highlight |
| Page Down, Page Up | Move the highlight a page at a time |
| Enter | Open the highlighted session. If nothing matches, ask about what you typed |
| Right, with nothing typed | Open the highlighted session |
| Ctrl+L | Start a new request, like Ask |
| Escape | Clear the search; with nothing typed, close the canvas |

The list is fetched each time the canvas opens and follows the agent service
while it stays open: new sessions, finished turns and questions show up as
they happen. The panel grows with the list, up to about eight rows, and scrolls
past that. If the agent service is not running, the panel says so.

## Stash

When you stash something with `otto-stash` (see the stash shortcuts in
[Keyboard shortcuts](keyboard-shortcuts.md)), its "Ask about…" card sits at
the top of the canvas, and the canvas slides in to show what you added. Your
app keeps the keyboard, so your selection and caret stay where they were,
and clicking the card doesn't take the keyboard either. Drop files on the
card to add them.

You can start a stash by dropping files too: drag them from Files (or any
app) over to the right side of the screen and hold them there for a moment. The
canvas opens with a place to drop them, and dropping them there starts the
stash.

The canvas stays open until you close it. Since you didn't open it yourself,
Escape stays with your app. Each new add brings the canvas back. When you
send the stash or clear it, the card leaves the canvas.

## Opening and closing

- **Touchpad:** put two fingers near the right edge of the touchpad and swipe
  left. The canvas follows your fingers; let go more than halfway out, or with
  a quick flick, and it opens. Swipe right with two fingers anywhere else on
  the touchpad to close it.
- **Keyboard:** bind the `CanvasToggle` action (see below). Escape closes it;
  when a panel has the keyboard, the panel decides what Escape does first
  (the Agents panel clears its search).
- **Pointer:** click anywhere outside the canvas to close it. The click still
  reaches the window you clicked, and the canvas slides away when you let go
  of the button. If you pressed to drag something instead, the canvas stays
  open so you can drop it on a panel.
- **Dragging:** while you drag something, hold it near the right edge of
  the screen for a moment and the canvas opens, so you can drop it on a panel.
  Drop it anywhere else and the canvas closes again.

The canvas opens on the screen the pointer is on. If no application has put
anything in it, it does not open.

A panel can take the keyboard as the canvas opens, as the Agents panel does;
otherwise, click a panel to type into it. When the canvas closes, the
keyboard goes back to the window you were using before.

## Settings

```toml
[canvas]
width = 400   # width of the column, in points (200 to 1200)
margin = 12   # space between the column and the screen edges
gap = 12      # space between two panels
```

The width can also be changed from the Settings bus (`canvas.width`) and takes
effect at once; every panel is redrawn at the new width.

The canvas has no shortcut by default. To add one:

```toml
[keyboard_shortcuts]
"Ctrl+Alt+c" = "CanvasToggle"
```

## Limits

- Panels that do not fit in the height of the screen are cut off at the
  bottom; the canvas does not scroll yet.
- Panels appear in the order their applications created them, unless an
  application asks for a place (the stash card goes on top).
