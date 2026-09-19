# Night Shift

Otto has no built-in colour-temperature setting. Instead it implements
`zwlr-gamma-control-v1`, the protocol the established tools use, so they work
against Otto directly and drive the display hardware's gamma tables — no
software post-processing, no cost per frame.

## wlsunset

Follows sunrise and sunset for your location.

```sh
sudo pacman -S wlsunset      # Arch
sudo dnf install wlsunset    # Fedora
sudo apt install wlsunset    # Debian/Ubuntu
```

Start it with your latitude and longitude:

```toml
[[exec_once]]
cmd = "wlsunset"
args = ["-l", "48.8", "-L", "2.3"]
```

`-l` is latitude, `-L` is longitude — negative for south and west.

Fixed temperatures instead of sun-following:

```sh
wlsunset -t 3500 -T 6500     # night 3500K, day 6500K
wlsunset -T 6500 -t 6500     # effectively off
```

## gammastep

A fork of redshift with more options — manual times, a systemd unit, and a
config file.

```sh
gammastep -l 48.8:2.3
```

```ini
# ~/.config/gammastep/config.ini
[general]
temp-day=6500
temp-night=3800
location-provider=manual

[manual]
lat=48.8
lon=2.3
```

Then:

```toml
[[exec_once]]
cmd = "gammastep"
```

or `systemctl --user enable --now gammastep`.

## Brightness

Backlight brightness is separate from colour temperature and is handled by Otto:

```toml
[keyboard_shortcuts]
"XF86MonBrightnessUp"   = "BrightnessUp"
"XF86MonBrightnessDown" = "BrightnessDown"
```

Each press shows an on-screen indicator. This adjusts the actual hardware
backlight, so it saves power — unlike gamma-based dimming, which only makes the
picture darker.

For external monitors that expose DDC/CI, use `ddcutil`:

```sh
ddcutil setvcp 10 50    # 50% brightness
```

## Choosing a temperature

| Kelvin | Feels like |
|--------|------------|
| 6500K | Daylight — no shift, the display's native point |
| 5000K | A gentle warmth, usable all day |
| 4000K | Clearly warm; a common evening setting |
| 3400K | Halogen bulb; the usual "night" default |
| 2700K | Incandescent; very orange |
| 1900K | Candlelight; only for late-night reading |

## Troubleshooting

**"Failed to bind gamma control".** Otto must be running as your compositor —
the protocol is not available on the `--winit` backend, where the host
compositor owns the display hardware.

**Colours do not change.** Some drivers ignore gamma tables on some outputs.
Try a different output, and check the tool's own output for errors.

**Colours stay shifted after the tool exits.** Otto resets the ramp when a gamma
client disconnects, so this should not happen. If it does, the shift is coming
from somewhere else — check for a second tool still running.

**A second tool will not start.** Only one gamma client per output is allowed.
Otto refuses the newcomer, so the tool that got there first keeps the ramp; stop
it before starting another.

## Not planned

Otto is unlikely to grow its own night-shift implementation — `wlsunset` and
`gammastep` do the job well, and the protocol exists so that compositors do not
have to.
