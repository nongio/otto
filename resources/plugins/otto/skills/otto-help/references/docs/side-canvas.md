# Side Canvas

The side canvas is a column that slides in over your desktop from the right
edge of the screen. Applications put panels in it, stacked top to bottom, and
it stays out of the way until you pull it in. Nothing underneath moves: your
windows stay where they are and the canvas slides over them, fullscreen
windows included.

Otto ships `otto-canvas`, a small program that places one empty panel in the
canvas so there is something to see. It starts with the session when it is
listed under `[[exec_once]]` in your config, as it is in the default one.

## Opening and closing

- **Touchpad:** put two fingers near the right edge of the touchpad and swipe
  left. The canvas follows your fingers; let go more than halfway out, or with
  a quick flick, and it opens. Swipe right with two fingers anywhere else on
  the touchpad to close it.
- **Keyboard:** bind the `CanvasToggle` action (see below). Escape closes it.
- **Pointer:** click anywhere outside the canvas to close it. That click goes
  no further, so it will not land on the window underneath.

The canvas opens on the screen the pointer is on. If no application has put
anything in it, it does not open.

Click a panel to type into it. When the canvas closes, the keyboard goes back
to the window you were using before.

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
- Panels appear in the order their applications created them.
