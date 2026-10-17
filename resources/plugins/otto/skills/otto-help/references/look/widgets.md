# A desktop widget for a look

For a look whose image implies a page on the desktop: a calendar set like a
poster, a grid of rules, one line of big type. Otto's widgets are windows of
one ewwii configuration that the compositor runs itself. A new one is a new
window in the person's own copy of it.

Build it, test it on a copy, show them, then install it. Ask before anything
appears on their screen.

## Exact commands

```sh
# Is ewwii there? Nothing prints: say it is needed (on Arch, the AUR package ewwii) and stop
command -v ewwii

# Once, at the start: a draft to work in, from the person's own copy if they have one
D=/tmp/otto-widget-draft; rm -rf "$D"; cp -r ~/.config/otto/widgets/ewwii "$D" 2>/dev/null || cp -r /usr/share/otto/widgets/ewwii "$D"

# The screen, in logical pixels: the rect of the primary output
otto-msg -t get_outputs -r

# A font's file and its metrics in em (ascent, descent, cap height); needs fontTools (python-fonttools)
fc-match -f '%{file}\n' 'Inter'
python3 -c "import sys;from fontTools.ttLib import TTFont;f=TTFont(sys.argv[1],fontNumber=0);u=f['head'].unitsPerEm;print(f['hhea'].ascent/u,-f['hhea'].descent/u,f['OS/2'].sCapHeight/u)" FONTFILE
```

## Step 1: the draft and the window

1. Work in the draft, `/tmp/otto-widget-draft` (above), never in
   `~/.config/otto/widgets/ewwii` directly: Otto uses that folder instead of
   the shipped one as soon as it exists.
2. Add the new window at the end of the draft's `ewwii.nbcl`, with a name of
   your own (`poster`), and its styles at the end of its `ewwii.scss`.
   Scripts go in `scripts/`, backdrop generators in `generators/`. Shape the
   window like the shipped ones: `monitor = 0`, `stacking = "bg"`,
   `exclusive = false`, `focusable = "none"`, geometry `100%` by `100%`,
   anchored `center`.

## Step 2: test it

Otto prepares a theme before running it: it puts `$theme-dir` and what the
generators print ahead of `ewwii.scss`. Do the same from the draft into
`/tmp/otto-widget-test`, and run that. W and H are the logical width and
height. Run the first line again after every edit to the draft; ewwii picks
the change up by itself. It updates the folder in place, because the running
daemon works from it.

```sh
D=/tmp/otto-widget-draft T=/tmp/otto-widget-test W=1440 H=960; mkdir -p "$T" && tar -C "$D" --exclude=./ewwii.scss -cf - . | tar -C "$T" -xf - && { printf '$theme-dir: "file://%s";\n' "$T"; for g in "$T"/generators/*; do (cd "$T" && "$g" $W $H); done; cat "$D/ewwii.scss"; } > "$T/ewwii.scss.new" && mv "$T/ewwii.scss.new" "$T/ewwii.scss"
ewwii --config /tmp/otto-widget-test daemon
```

Before opening it, ask with the question tool: "Show the widget on your
desktop for a moment to check it?" `Show it` / `Not now`. Then:

```sh
ewwii --config /tmp/otto-widget-test open poster
grim /tmp/otto-widget-test.png                  # look at it, then say what you see
ewwii --config /tmp/otto-widget-test active-windows
ewwii --config /tmp/otto-widget-test close poster
ewwii --config /tmp/otto-widget-test kill       # always, when done
```

Errors are in `~/.cache/ewwii/` (the newest `ewwii_*.log`), or run the daemon
with `--no-daemonize` to see them. Keep test windows on `bg`, under their
windows; never on `overlay`, over their work. Do not send input to the desktop.

## Step 3: install it

Ask first, then put the draft in place. It holds the shipped widgets too, so
they keep working:

```sh
mkdir -p ~/.config/otto/widgets/ewwii && cp -r /tmp/otto-widget-draft/. ~/.config/otto/widgets/ewwii/
```

**`desktop.widget` takes the shipped names and every other `Window` in the
theme in use**: once the copy in `~/.config/otto/widgets/ewwii` has
`Window "poster"`, `Set` with `s "poster"` shows it at once, and Settings
lists it as "Poster" (underscores and dashes become spaces) the next time it
opens. Before the copy is installed, `Set` with that name fails with
`must be one of`.

## The ewwii field guide

These cost hours to find out. Follow them.

**The `.nbcl` file**

- `Poll "name" { interval = "10m" cmd = "scripts/x.py field" initial = "" }`;
  `Listen "name" { cmd = "..." initial = "" }` for a process that prints a
  line whenever the value changes.
- Read a value with `global("name")`. Reuse layout with
  `component Name (a, b) { ... }`, used as `Name { a = "x" b = "y" }`.
- `Window "name" { monitor stacking focusable geometry = { ... } Child {} }`.
- Widgets: `Box` (`orientation`, `space_evenly`, `spacing`), `Label`
  (`text` or `markup`, `xalign`), `OverLay` (children stacked, first one sets
  the size), plus `halign`, `valign`, `hexpand`, `vexpand`, `class`.
- **No `//` comments in `.nbcl`**: one at the top is a parse error and
  nothing loads. Comments belong in the `.scss`.
- **A poll keeps only the first line** of its command's output. One value
  per poll; a script that takes the field name as its argument, like
  `scripts/month.py`, keeps that tidy.
- `stacking`: `bg` sits under the desk's icons (the desk is on the bottom
  layer), `bottom` over them, `fg` and `overlay` above windows. A widget is
  `bg`.

**The stylesheet (GTK4 CSS, not browser CSS)**

- **Negative margins are ignored.** To put a rule on a line of type, overlay
  the rule on the label in an `OverLay` and push it in with positive margins.
- `transform: translate(X, Y) rotate(-90deg)` works, turning about the
  centre; a Label's `angle` does not. Give a turned label a `min-width`.
- **Background images load only from an absolute `file://` URL**: write
  `url("#{$theme-dir}/poster/grid@2x.png")`. Relative `url()` and
  `-gtk-scaled()` do not load.
- A widget's `class` lands on a wrapper, and the `* { all: unset; ... }`
  reset hits the inner label again. Override fonts with `.cls, .cls *`.

**Lining type up**

A label's box runs from the font's ascent to its descent. At font size S,
the baseline is `ascent x S` from the top of the box, and the cap line
`(ascent - cap) x S`. Inter: ascent 0.969, descent 0.241, cap 0.728. So a rule
overlaid with `valign = "end"` and `margin-bottom: descent x S` sits on the
baseline; with `valign = "start"` and `margin-top: (ascent - cap) x S`, on the
cap line. Other fonts: measure them with the line above.

**Grids and backdrops**

Draw them as PNGs with a generator in `generators/`: an executable script run
with the logical width and height, which writes its images (a `@2x` one for
sharp lines) into the theme and prints SCSS variables for the positions the
text needs. `generators/lines-grid.py` and `focus-grid.py` are working
examples, in plain Python with no extra modules. Make the script executable.

**Keeping it light**

- Poll slowly: `10m` for dates, `1m` for a clock showing minutes. `1s` only
  for seconds on screen.
- Several values from one source: a `Listen` on one script beats many fast
  polls.
- Heavy work belongs in a script, not in a long pipeline inside `cmd`.

**Before you call it done**

1. Every script runs on its own and prints one line.
2. The daemon starts with no errors in the log.
3. The window opens, and the screenshot matches the image's grid and type.
4. The test daemon is killed, and `/tmp/otto-widget-test`, its screenshot
   and the draft are gone once it is installed.
5. The person has been told what changed and when it will show.

Never say it works if it has not been opened and looked at.
