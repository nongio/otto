# Saving a look

A look folder holds `look.toml` and, when its licence allows, the wallpaper.
`otto-look` applies it again, and it can be shared. Keep the person's in
`~/.local/share/otto/looks/<id>/`, `<id>` being the name in lower case with
dashes. The full format is the README of
`https://github.com/nongio/otto-looks`.

## Exact commands

```sh
mkdir -p ~/.local/share/otto/looks/ID
cp WALLPAPER ~/.local/share/otto/looks/ID/wallpaper.jpg

# The pin a theme's download needs, when the look is to be shared
curl -fsSL ARCHIVE-URL | sha256sum

# Apply the saved look again (it sets the dock to the bottom and untinted unless the file says otherwise)
otto-look ~/.local/share/otto/looks/ID
```

Ask for anything you cannot know: their GitHub username for `author`, and
who made the wallpaper and under what licence. A picture of their own is
theirs to credit (`author` their name, `licence` whatever they choose). If a
picture cannot be credited, it does not go in.

## look.toml

```toml
[look]
name = "Harbour"
version = 1
author = "their-github-username"
description = "Grey sea, one orange buoy."

[wallpaper]
title = "Harbour at dawn"
author = "Whoever made the image"
source = "https://link-to-the-original"
licence = "CC BY 4.0"
url = "wallpaper.jpg"

[icons]
theme = "Papirus"
author = "Papirus Development Team"
source = "https://github.com/PapirusDevelopmentTeam/papirus-icon-theme"
licence = "GPL-3.0"

[settings]
theme_scheme = "Dark"
accent_color = "#E8793A"
background_color = "#2B3138"
dock.position = "bottom"
```

- `[cursors]` is shaped like `[icons]`. Every `[wallpaper]`, `[icons]` and
  `[cursors]` needs `author`, `source` and `licence`, or `otto-look` refuses
  it. For sharing, a theme also needs its makers' archive as `url` plus
  `sha256`.
- `[settings]` takes only: `theme_scheme`, `accent_color`, `background_color`,
  `cursor_size`, `rounded_corners`, `frosting`,
  `window_controls_side`, `show_maximize_button`, `dock.position`,
  `dock.size`, `dock.magnification`, `dock.autohide`, `dock.colorize_icons`,
  `dock.colorize_color`, `dock.colorize_intensity`, `dock.genie_scale`,
  `dock.genie_span`, `appswitcher.colorize_icons`. The desk and the widget are
  not part of a look file, and looks don't set the font; say so if it
  matters to this one.
- A look without `dock.position` puts the dock at the bottom, and one without
  `dock.colorize_icons` turns the tint off.
- Only put the wallpaper in the folder if its licence allows sharing it.

To share it with everyone, it goes to the otto-looks repository as a pull
request; its README says how.
