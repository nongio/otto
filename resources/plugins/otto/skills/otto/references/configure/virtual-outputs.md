# Virtual outputs

A virtual output is a monitor with no physical display behind it: Otto renders
it like any other screen and pushes the frames to a PipeWire stream, where OBS,
a recorder or the RDP bridge can pick them up. It has its own workspaces, its
own exposé, and windows can be dragged onto it.

## Exact commands

```sh
# Create one: name, width, height, refresh_hz, interactive, persist
# It answers with the PipeWire node id to capture from
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    AddVirtualOutput suudbb virtual-1 1920 1080 60.0 true true

# Remove it again
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    RemoveVirtualOutput s virtual-1

# Check it came up
busctl --user --json=short call org.otto.Settings /org/otto/Settings org.otto.Settings ListOutputs
```

Set `interactive` to `true` for a screen to be controlled remotely, `false` for
a view-only feed. Set `persist` to `true` to have it come back next session.

## On the running session

```sh
# name, width, height, refresh_hz, interactive, persist → the PipeWire node id
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    AddVirtualOutput suudbb virtual-1 1920 1080 60.0 true true

busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    RemoveVirtualOutput s virtual-1
```

Unlike a display profile this takes effect **immediately** — a virtual screen
you have to restart for is useless for what it is mostly wanted for. The answer
is the PipeWire node id to capture from.

`persist` also writes a `[[virtual_outputs]]` entry, but only after the output
actually came up. `RemoveVirtualOutput` tears one down and drops its entry, and
refuses a physical output — unmapping one would black out a real screen.

## In the file

```toml
[[virtual_outputs]]
name = "virtual-1"
resolution = { width = 1920, height = 1080 }
refresh_hz = 60.0
position = { x = 3840, y = 0 }
interactive = true
```

`interactive = true` accepts remote pointer and keyboard aimed at this output —
what RDP needs. Leave it `false` for a view-only feed.

## Worth knowing

- **Give it a position past the real screens.** With none it defaults to the
  same origin as the first monitor, overlaps it, and is placed automatically
  instead.
- **The node id is logged at startup**: `Virtual output 'virtual-1' started
  (PipeWire node 42)`. Capture it with any PipeWire client:
  `gst-launch-1.0 pipewiresrc path=42 ! videoconvert ! autovideosink`.
- **A virtual output with no consumer produces no frames**, by design. If a
  stream looks dead, check something is actually reading it.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/display.md
- https://github.com/nongio/otto/blob/main/docs/user/remote-desktop.md
