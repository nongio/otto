# Desktop Widget

**Status:** draft  
**Related specs:** [settings-app.md](./settings-app.md), [desk.md](./desk.md), [login-mode.md](./login-mode.md)

## Summary

A desktop widget is a full-screen page drawn over the wallpaper and behind
the windows: a calendar, or a quiet reminder on a drafting grid. Otto ships a
few of them as themes for ewwii, an optional widget engine, and runs ewwii
itself for whichever one is chosen, so picking a widget in Settings is all it
takes.

## Goals

- One setting, `desktop.widget`, picks the widget: `none` (the default),
  `calendar`, `cross_pad` or `grid_pad`.
- The chosen widget appears at login and follows a change of the setting
  live, without logging out.
- ewwii is optional. Otto installs and runs without it; where it is missing,
  Settings says so instead of offering a choice that cannot work.
- A user can customise the themes without touching the installed copy.
- The pages sit inside the usable area, clear of the top bar, the dock and
  any other panel, wherever those are and however large.

## Non-Goals

- A widget framework of Otto's own. ewwii draws the pages; Otto only prepares
  the theme and runs ewwii.
- Several widgets at once, widgets on other outputs than the first, or
  per-workspace widgets.
- Running or managing ewwii configurations the user starts by hand. Otto's
  daemon uses its own copy of the theme and never touches another.

## Behavior

### The themes

- The shipped themes form one ewwii configuration, installed as
  `/usr/share/otto/widgets/ewwii` (looked up as `otto/widgets/ewwii` in each
  of `$XDG_DATA_DIRS`). Each widget is a window of that configuration, named
  as its value of `desktop.widget`.
- If `$XDG_CONFIG_HOME/otto/widgets/ewwii` holds an `ewwii.nbcl`, that
  configuration is used instead of the installed one.
- **Calendar** — the current month: its name, today's number, the weeks
  listed with today in bold, and a header with the number of days and
  weekends, the first and last weekday, the ISO week and the year.
- **Cross pad** — a grid of small crosses filling the usable area, the title
  "Stay Focused." in the middle, the date reading up the left edge and a short
  note in the bottom-right corner.
- **Grid pad** — a drafting grid with rulers and a diagonal, its rows filling
  the usable area's height and its side columns the rest of the width, the
  title "Don't be busy." in the middle row, the date up the left side column
  and a note in the end of the bottom row.
- The pages' text is English; the month and weekday names follow the
  session's locale.

### Preparing a theme

Every time a widget is to be shown (at login, at each change to a widget
other than `none`, and when the usable area changes size):

1. The theme is copied into `$XDG_CACHE_HOME/otto/widgets/ewwii`, in place,
   rewriting only the files whose contents differ.
2. Every file in the theme's `generators/` folder is run from the theme's
   root with the logical width and height of the primary output's usable
   area as its two arguments. A generator draws its backdrop images into the
   theme and prints SCSS variable declarations on standard output.
3. The theme's `ewwii.scss` is written with `$theme-dir` (the prepared
   folder's absolute `file://` URL), `$usable-width` and `$usable-height`
   (the size the backdrops were drawn for, which also makes the stylesheet
   differ, and ewwii reload, whenever that size changes) and every
   generator's output ahead of the original stylesheet. The stylesheet is
   replaced in one step, never written partially, and not at all when it
   would come out the same.

Preparing runs off the compositor's main loop. If any step fails (a
generator exits unsuccessfully, a file cannot be written), the failure is
logged and the widget is not shown; whatever was on screen stays.

### Running ewwii

- When a widget is chosen, ewwii is on `PATH`, and the session is not in login
  mode, the compositor starts `ewwii --config <prepared folder>
  --no-daemonize daemon` with the session's environment, once the theme is
  prepared, and asks it to close every window and open the chosen one.
  Opening is retried for several seconds while the daemon comes up.
- Choosing another widget while the daemon runs prepares the theme again,
  closes the open window and opens the new one on the same daemon.
- Choosing `none` terminates the daemon.
- If the daemon exits unsuccessfully while a widget is chosen, it is started
  again after a back-off that starts at one second and doubles to a minute;
  a daemon that stayed up for 30 seconds counts its next crash as the first.
  A successful exit (someone ran `ewwii kill` on it) is left alone until the
  setting changes.
- A new daemon waits for one that is still shutting down, since both would
  use the same socket.
- The daemon is terminated with the compositor.
- A value other than `none` that is not one of the shipped widgets opens the
  window of that name, so a customised theme can add its own. The settings
  service offers, after the shipped ones, every other `Window "name"` of the
  theme in use (in file order), labelled with the name, underscores and
  dashes as spaces and the first letter capitalised, and accepts them.

### The windows

- The shipped windows are layer-shell surfaces on the background layer, on
  the first output, the output's size, with no exclusive zone and no keyboard
  focus. They sit over the wallpaper and under the desk and every window.

### The reserved area

- The usable area is the one maximized windows fill: the primary output less
  the exclusive zones of layer-shell panels and the dock's band (none while
  the dock autohides).
- The compositor writes the space between the widget's window and the usable
  area to `reserved-area` in the prepared theme: one line, a CSS padding
  shorthand in logical points, top first and clockwise
  (`0px 0px 108px 0px`). The file is replaced in one step, and only written
  when the value changes or the theme was prepared again.
- It is measured from the edges of the window as Otto draws it: anchored and
  sized against the whole output, whatever the panels' exclusive zones (the
  layer map's own arrangement, which moves the window below the top bar, is
  not where it is drawn). Until the window is mapped (found by the daemon's
  process id), the output stands in for it.
- The value is brought up to date whenever a layer-shell surface maps,
  changes or goes away, and once the dock has settled after a change of edge,
  size or autohide.
- When the usable area changes size, the theme is prepared again for it. A
  stylesheet that comes out different makes ewwii reload the window; the
  compositor does not reopen a window that is already shown.
- Each shipped window follows the file with a `Listen` (`tail -F`) into the
  `reserved_area` variable and pads its page by it, so the page's own margins
  are measured from the usable area. The file is not an ewwii configuration
  file, so rewriting it never reloads the daemon.

### Settings

- Settings ▸ Appearance ▸ Desktop has a *Background widget* pop-up listing None,
  Calendar, Cross pad and Grid pad. The setting applies live.
- Where ewwii is not on `PATH`, the pop-up is dimmed, cannot be opened, and
  the row says ewwii is needed.

## Constraints & Edge Cases

- ewwii keeps only the first line of a polled command's output, so every
  value a page shows comes from its own poll.
- GTK only loads a stylesheet image from an absolute `file://` URL, hence the
  prepared copy and `$theme-dir`.
- The backdrops are drawn for the usable area's size, and redrawn when it
  changes with a panel or the dock. A change of the output's mode or scale
  alone is picked up the next time the usable area changes, or the widget
  starts or is chosen again.
- A window's `Listen` starts with no padding, so for a moment after it opens
  or reloads the page may sit against the screen's edges.
- Only the primary output is measured; the themes have one window each, on
  the first output.
- ewwii reloads its configuration, and reopens its window, whenever a file of
  it changes, which is why the stylesheet is replaced whole and unchanged
  files are left alone: switching widgets on the same screen then costs one
  close and one open.
- If ewwii is uninstalled while a widget is chosen, the next start logs that
  it is missing and shows nothing; the setting keeps its value.

## Rationale

- **The compositor runs ewwii**, like the desk, rather than an autostart
  entry: the setting has to apply live, and only the compositor sees the
  change and knows the primary output's size.
- **ewwii is optional.** It is a sizeable GTK4 program many users will never
  want; Otto lists it as an optional dependency and degrades to a dimmed row.
- **One configuration for all the widgets.** Switching is then a close and an
  open on a daemon that is already running.
- **The themes are prepared again on every change**, because it is cheap
  (well under a second) and picks up the output size and the user's own edits
  without any change tracking.
- **The background layer.** The desk is on the bottom layer, and the widget
  belongs under the desk's files.
- **A file, not `ewwii update`.** ewwii can set a variable from outside, and
  a widget's `style` can be bound to one, but every variable is cleared when
  the last window closes, which switching widgets does, and on a daemon
  restart. A file followed by a `Listen` is read again whenever the window
  opens, survives reloads and restarts, and costs the compositor one small
  write instead of a client process per change.
- **Insets, not a resized window.** ewwii sizes a window only as a share of
  the monitor and cannot ask for an exclusive zone of -1, so the window stays
  the output's size and the page is padded inside it.

## Open Questions

- Should the backdrops be redrawn when the primary output's mode or scale
  changes during the session without the usable area moving?
