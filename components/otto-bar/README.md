# otto-bar

Menu bar for the Otto compositor. Displays the app menu, system tray, battery and clock.

## Running

```sh
# Inside an Otto session
otto-bar

# Or via autostart in otto config
# [[exec_once]]
# cmd = "otto-bar"
# args = []
```

## Configuration

otto-bar looks for a TOML config file in this order:

1. `/etc/otto/otto-bar.toml`
2. `~/.config/otto/otto-bar.toml`
3. `./otto-bar.toml`

### Options

| Key            | Default                  | Description                  |
|----------------|--------------------------|------------------------------|
| `clock_format` | `"%B %-d, %A %H:%M"`    | Clock format ([chrono strftime](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)) |

### `[battery]`

| Key                 | Default        | Description                                        |
|---------------------|----------------|----------------------------------------------------|
| `show`              | `"auto"`       | `"auto"` (only with a battery), `true`, `false`     |
| `colored`           | `true`         | Tint the fill by level; `false` uses the text colour |
| `percentage`        | `"inside"`     | `"inside"`, `"beside"`, `"off"`                     |
| `low_level`         | `20`           | Below this, the fill turns amber                    |
| `critical_level`    | `10`           | Below this, red                                     |
| `width` / `height`  | `28` / `13`    | Glyph size in points                                |
| `color_normal`      | `"#34C759"`    | `#RGB`, `#RRGGBB` or `#AARRGGBB`                    |
| `color_low`         | `"#FF9F0A"`    |                                                     |
| `color_critical`    | `"#FF3B30"`    |                                                     |
| `color_charging`    | `"#34C759"`    |                                                     |
| `update_interval`   | `10`           | Seconds between readings                            |
| `menu`              | `true`         | Whether clicking opens the power menu               |
| `show_battery_info` | `true`         | Percentage and time remaining in the menu           |
| `show_cpu_info`     | `true`         | CPU frequency and governor in the menu              |
| `profile_backend`   | `"auto"`       | `"auto"`, `"power-profiles"`, `"commands"`          |
| `settings_command`  | `"otto-settings"` | Menu's last entry; empty string hides it         |

`[[battery.profiles]]` entries define the switchable profiles. When any are
set they take precedence over power-profiles-daemon, unless `profile_backend`
is `"power-profiles"`. Each takes `label`, `command`
(string or argv array) and optionally `governor` / `epp`, which decide when the
entry is check-marked.

### Example

```toml
clock_format = "%H:%M"

[battery]
percentage = "beside"
colored = false

[[battery.profiles]]
label = "Power Saver"
command = ["pkexec", "cpupower", "frequency-set", "-g", "powersave"]
governor = "powersave"
```
