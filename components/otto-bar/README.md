# otto-bar

Menu bar for the Otto compositor. Displays the app menu, system tray, and clock.

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

### Example

```toml
clock_format = "%H:%M"
```
