# Making a look

For "something moody and Japanese", "match my Bauhaus poster", "make my desktop
look like this image", "a new theme", "rice my desktop". A look is a set of
choices that belong together: wallpaper, light or dark, one accent, an icon
theme, a cursor theme, the dock, the desk, and sometimes a desktop widget.

Read the brief, compose, **show it and ask**, then apply. Nothing changes
before the person has agreed to the look.

## Exact commands

```sh
# The monitor's size in logical pixels (rect), and the scale; physical = logical x scale
otto-msg -t get_outputs -r
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Get s screen_scale

# The main colours of an image, most common first; accents are the saturated rows further down
magick IMAGE -resize 200x200 -colors 16 -format "%c" histogram:info:- | sort -rn

# Icon and cursor themes installed (a line per theme, "cursors" when it has them)
for d in /usr/share/icons/* ~/.local/share/icons/* ~/.icons/*; do [ -f "$d/index.theme" ] && echo "${d##*/} $([ -d "$d/cursors" ] && echo cursors)"; done 2>/dev/null | sort -u

# Looks already published, ready to install
otto-look list

# Read a value before changing it (repeat for each id you will set)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Get s accent_color

# Set one (the type letter must match; see references/configure.md, Step 4)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv accent_color s "#2F8F7A"
```

## Step 1: read the brief

**From an image.** Look at it, then sample it with the `magick` line above
rather than guessing colours. Write down, in a line each:

- **Palette**: the dominant colour, two or three accent candidates (small,
  saturated areas), and whether it is light or dark overall.
- **Mood and era**: calm, loud, nocturnal, printed, faded; a movement if it
  has one (Bauhaus, Swiss, ukiyo-e, Memphis, brutalist).
- **Type**: serif or sans, geometric or humanist, set tight or loose.
- **Geometry**: grids, rules, crosses, circles, diagonals, a lot of empty space.

**From words.** If the mood leaves something open, ask with the question tool,
at most two or three questions, one decision each: "Light or dark?", "Which
colour should stand out?" (three from the mood, plus "Pick for me"), "Keep
the dock where it is?". Do not ask what the brief already answers.

## Step 2: compose

Choose the wallpaper first; everything else answers it.

1. **Wallpaper.** The image itself, when it works as a wallpaper and it is the
   person's own or free to use: landscape, at least the monitor's physical
   size, nothing important where the dock sits. A portrait poster or a small
   image does not fill a screen well. Set it on a canvas of its own dominant
   colour instead:
   `mkdir -p ~/.local/share/backgrounds && magick POSTER -resize xH -gravity center -background "#RRGGBB" -extent WxH ~/.local/share/backgrounds/NAME.png`
   (W and H physical). Otherwise describe the wallpaper that would suit and
   let them pick one; do not download images from the web on your own.
2. **Light or dark** follows how bright the wallpaper is.
3. **Accent**: one colour from the palette, not a new one. Check it against
   the chrome with this line: at least 3 against the scheme's
   panels (`#F6F6F6` light, `#2B2B2B` dark). If the exact hex fails, darken
   or lighten it a step, or use the nearest palette name (`red`, `teal`,
   ...), which is tuned for both schemes.
   `python3 -c "import sys;L=lambda h:(lambda c:0.2126*c[0]+0.7152*c[1]+0.0722*c[2])([((v/255)/12.92 if v/255<=0.04045 else (((v/255)+0.055)/1.055)**2.4) for v in bytes.fromhex(h.strip('#'))]);a,b=sorted(map(L,sys.argv[1:]));print(round((b+0.05)/(a+0.05),2))" "#2F8F7A" "#F6F6F6"`
4. **Background colour**: the wallpaper's dominant edge colour, as hex. It
   shows while the image loads.
5. **Icons and cursor**: from what is installed (the loop above). Match the
   wallpaper's texture: flat with flat, detailed with detailed. The cursor
   must read clearly on the wallpaper. If nothing installed fits, name one
   that would and say it needs installing; do not install packages yourself.
6. **Shape**: `rounded_corners` off for hard-edged, printed or grid looks;
   `frosting` off for flat, opaque ones.
7. **Dock**: edge and size. Tint the icons (`dock.colorize_*`) only when the
   look is monochrome; the tint is luminance times colour, so tint bright.
8. **Desk** (the files on the desktop): whether it shows, and in
   `~/.config/otto/files.toml` under `[desk]`: `icon_size`, `overflow`
   (`scroll` or `stack`), and `anchor`/`size`/`position` to keep the icons
   off the part of the wallpaper that matters. It follows the file live.
9. **Widget**, only when the image implies one (a calendar, a grid, a line of
   type): the three built in are `calendar`, `stay_focused`, `dont_be_busy`,
   and they need ewwii (`command -v ewwii` prints a path). A custom one is
   [look/widgets.md](look/widgets.md).

Taste, in short: every piece fits the wallpaper, and you can say why in one
line. One accent. Fewer, quieter choices beat many loud ones. No novelty
cursors or icon packs that fight the image. If `otto-look list` already has a
look that fits the brief, offer that as one of the choices.

## Step 3: show it, then ask

Summarise the look in four to six short lines: the wallpaper, light or dark,
the accent (and which part of the image it comes from), icons and cursor, the
dock and desk, the widget. Then ask with the question tool:

- `Apply it`: "Changes the wallpaper, colours, icons, cursor and dock now."
- `Adjust something`: then ask what, one question.
- `Leave my desktop as it is`.

With no question tool, write the summary and wait for a yes.

## Step 4: apply

1. **Read every value you are about to change first** with `Get`, and keep
   the list. That list is the undo; put it in your reply at the end too, one
   line per setting, so it outlives this conversation.
2. Set each one, in this order: `background_image` (absolute path),
   `background_color`, `theme_scheme`, `accent_color`, `icon_theme`,
   `cursor_theme`, `rounded_corners`, `frosting`, the `dock.*` settings,
   `desk.enabled`, `desktop.widget`. Types and values are in
   [configure/appearance.md](configure/appearance.md) and
   [configure/dock.md](configure/dock.md); read the result of each as
   configure.md Step 5 says.
3. Edit `[desk]` in `~/.config/otto/files.toml` only if they agreed to a desk
   change: read the file, change those keys, leave the rest.
4. Say what changed in two sentences, and how to go back: "Say *undo the
   look* and I'll put the previous values back." To undo, `Set` each id to the
   value you noted, not `Reset`, which goes to Otto's default instead.

A published look is one command, which installs its themes and sets
everything itself: `otto-look NAME`.

## Step 5: save it (optional)

Offer to keep it as a look folder that `otto-look` can apply again and that
can be shared. How, and the file format: [look/save.md](look/save.md).

## Rules

- **Do not change anything before they chose `Apply it`**, and then only what
  the summary listed.
- **Do not use a picture you cannot credit** in a saved look.
- Do not change the font, the scale, the language or anything outside
  appearance, the dock and the desk.
- Do not compare the look to other desktops or operating systems.

## Read next

- https://nongio.github.io/otto/theming/
- https://nongio.github.io/otto/desktop-widgets/
