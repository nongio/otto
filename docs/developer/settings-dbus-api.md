# `org.otto.Settings` — the settings D-Bus contract

The D-Bus interface the compositor serves for reading and writing settings,
and the contract a settings client can build against.

> **Status**: implemented on the compositor side (`src/settings/`,
> `src/settings_service.rs`), ahead of the app. This document is the interface
> both sides build against, so the compositor and the settings app can be
> written independently. Behavioural requirements live in
> [specs/settings-app.md](../../specs/settings-app.md); this is the wire.
>
> What the compositor serves today: 47 settings, of which twenty-one are `live`
> and the rest are `restart`. The nine `dock.*` ones — `size`, `position`,
> `autohide`, `magnification`, `genie_scale`, `genie_span`, `colorize_icons`,
> `colorize_color`, `colorize_intensity` — reconfigure the dock in place. The eleven
> `input.*` touchpad/pointer ones — `tap_enabled`, `tap_drag_enabled`,
> `tap_drag_lock_enabled`, `touchpad_click_method`, `touchpad_dwt_enabled`,
> `touchpad_natural_scroll_enabled`, `touchpad_left_handed`,
> `touchpad_middle_emulation_enabled`, `scroll_speed`, `pointer_accel_speed`,
> `pointer_accel_profile` — reconfigure the connected libinput devices (or, for
> `scroll_speed`, take effect on the next scroll event, since Otto reads the
> live config per event rather than caching it). The keyboard `input.xkb_*`
> settings still need a restart. No setting is `unsupported` yet. Values that
> are lists rather than scalars (dock bookmarks, shortcuts, display profiles)
> have no identifier and are not in the schema.

Bus name `org.otto.Settings`, object path `/org/otto/Settings`, interface
`org.otto.Settings`. The compositor owns the name.

The portal getters (`GetColorScheme`, `GetIconTheme`, `GetAccentColor`) stay
exactly as they are — `xdg-desktop-portal-otto` depends on them and must not be
disturbed.

## Values

Every setting value crosses the bus as a variant. The variant's contained type
is fixed per setting and given in the schema:

| Schema `type` | D-Bus type | Notes |
| --- | --- | --- |
| `bool` | `b` | |
| `int` | `i` | |
| `double` | `d` | |
| `string` | `s` | free text (paths, command lines) |
| `enum` | `s` | one of `choices` |
| `string-list` | `as` | ordered, e.g. locales |

Sending a variant whose type does not match the schema is an error; the
compositor never coerces.

## Identifiers

A stable dotted path matching the configuration structure: `dock.size`,
`input.pointer_accel_speed`, `power_management.on_lid_close`. Top-level keys
have no prefix: `accent_color`, `cursor_size`.

**Identifiers are a permanent contract.** Once shipped, an identifier is never
renamed or repurposed — an app built against an older compositor must keep
working, and the app is the thing that hardcodes these strings.

## Methods

```
Describe()                 → aa{sv}    schema, one dict per setting
GetAll()                   → a{sv}     current effective value of every setting
Get(id: s)                 → v         one value
GetOverridden()            → as        ids currently set in the writable file
Set(id: s, value: v)       → s         status, see below
Reset(id: s)               → s         status

GetColorScheme()           → u         0 none, 1 dark, 2 light
GetIconTheme()             → s         icon theme name, empty to auto-detect
GetAccentColor()           → (ddd)     accent as sRGB in 0.0..=1.0

ListOutputs()              → aa{sv}    every output, physical and virtual
SetOutputProfile(connector: s, width: u, height: u,
                 refresh_hz: d, x: i, y: i,
                 primary: b)           → s   status, always pending-restart
AddVirtualOutput(name: s, width: u, height: u,
                 refresh_hz: d, interactive: b,
                 persist: b)           → u   PipeWire node id
RemoveVirtualOutput(name: s)           → ()

ConfigPath()               → s         the file a change is written to
```

### Portal getters

These three answer in the shapes `org.freedesktop.appearance` defines, so the
portal backend can pass them through untouched. `GetAccentColor` resolves the
stored accent *name* against the current palette before converting — the name
is what `Get("accent_color")` returns, and it is the value `Set` takes.

### Outputs

Outputs are deliberately **not** settings. The schema is a table fixed at
compile time, and outputs come and go with the hardware — there is no honest
identifier for "the second display". They get their own three methods instead.

`ListOutputs` returns one dictionary per output: `name`, `connector`, `width`,
`height`, `refresh` (millihertz), `x`, `y`, `scale`, and `virtual`. A client
drawing a display arrangement reads it from here rather than inventing one.

`SetOutputProfile` writes `displays.named.<connector>` — the same profile the
compositor resolves when it brings that output up — so a resolution, refresh
rate, position or primary choice survives a restart. It is keyed by connector
because that is the handle the config has; see the Open section for why that
identity is not the last word. **It applies at the next start, never now**, and
says so: a modeset made from under a running session cannot be undone if the
display does not come back, and that is worse than a restart. A zero width or
height leaves the resolution unset and a zero rate leaves the refresh unset, so
moving a display need not name a mode for it. Primary is a choice *among*
displays, so a client turning it on for one must write it off for the others.

`AddVirtualOutput` creates a PipeWire-backed output on the running compositor
and answers with the node id to capture from. It takes effect immediately: a
virtual screen you must restart to get is useless for the thing it is mostly
wanted for. `persist` also writes a `[[virtual_outputs]]` entry to the writable
config so it returns next session; the entry is written only after the output
actually came up, for the same reason `Set` persists only after a successful
apply. `RemoveVirtualOutput` tears one down and drops its config entry, and
refuses physical outputs — unmapping one would black out a real screen.

### The configuration file

`ConfigPath` answers with the file a changed setting is written to. Otto's
configuration is layered and the writable layer is whichever sits on top — a
local `otto_config.toml` next to the running binary outranks
`~/.config/otto/config.toml` — so a client cannot work it out for itself. It is
worth showing: everything a settings app changes lands there, and anything it
does not offer is edited by hand.

**`Describe`** returns one dictionary per setting. Keys:

| Key | Type | Meaning |
| --- | --- | --- |
| `id` | `s` | the identifier |
| `type` | `s` | from the table above |
| `section` | `s` | config section, e.g. `dock`; empty for top-level |
| `label` | `s` | short human-readable name |
| `description` | `s` | one sentence; may be empty |
| `default` | `v` | the built-in default |
| `apply` | `s` | `live`, `restart`, or `unsupported` |
| `min` | `v` | optional, numeric types only |
| `max` | `v` | optional, numeric types only |
| `step` | `v` | optional, numeric types only; granularity to snap to |
| `choices` | `as` | optional, required for `enum` |
| `choice_labels` | `as` | optional; human names for `choices`, same order |

Unknown keys must be ignored by clients, so the schema can grow without
breaking them.

`step` exists because a slider maps pixels to values: without it a drag lands
on whatever float the pixel happened to hit, and a pointer speed the user meant
to leave alone reads `-0.01`. Where a setting has a range it has a step, and
the step always divides the range so the advertised maximum stays reachable.
Clients should also snap to `default` when the drag comes within half a step of
it, so a value can land exactly back on the inherited one.

`choice_labels` exists because the strings in `choices` are configuration
tokens, and those are part of the permanent contract — `clickfinger` has to
stay `clickfinger` on the wire and in the file, however badly it reads in a
menu. When present it has exactly one entry per choice, and a client shows it
in place of the token while continuing to `Set` the token. When absent, the
tokens are already fit to show.

`apply` is what the setting does when set: `live` takes effect immediately;
`restart` is persisted but needs a compositor restart; `unsupported` cannot be
changed on this system or in this build (a display setting under a windowed
backend, say) and `Set` will reject it. It must be truthful — a `Set` that
silently does nothing is worse than one that refuses.

**`Set`** performs validate → apply → persist → announce, in that order. It
returns a status string:

- `applied` — live now, and persisted.
- `pending-restart` — persisted, takes effect on restart.

Any failure is a D-Bus error, not a status, and nothing is persisted.

**`Reset`** removes the identifier from the writable config file so its value
falls back to whatever the lower configuration layers provide. This is *not*
the same as setting it to its default: the writable file is the
highest-priority layer, so a default written into it permanently shadows every
lower layer. Resetting an identifier that is not in the writable file succeeds
and changes nothing. Returns the same status strings as `Set`.

**`GetOverridden`** returns the identifiers currently present in the writable
file — what the app needs to show a per-setting revert affordance. Effective
values always come from `GetAll`.

## Signal

```
Changed(values: a{sv})
```

Emitted after any effective value changes, from any source: this API, an
in-compositor interaction such as dragging the dock handle, or an external edit
of a configuration file. Carries only the identifiers that changed.

A client that called `Set` also receives the signal — it must not assume it is
the only writer, and must not suppress its own echo.

The signal is coalesced: a continuous interaction such as dragging a slider
applies live throughout but emits and persists when the interaction settles.

## Errors

| Error name | When |
| --- | --- |
| `org.otto.Settings.Error.UnknownSetting` | no such identifier |
| `org.otto.Settings.Error.InvalidType` | variant type does not match the schema |
| `org.otto.Settings.Error.OutOfRange` | outside `min`/`max`, or not in `choices` |
| `org.otto.Settings.Error.Unsupported` | `apply` is `unsupported` on this system |
| `org.otto.Settings.Error.ApplyFailed` | valid, but the compositor could not apply it; message says why |

`ApplyFailed` means nothing was persisted and the running state is unchanged.

## What a client can rely on

- The schema is authoritative for labels, ranges, defaults and apply
  behaviour. A client renders from `Describe` rather than hardcoding them.
- `GetAll` returns a value for every identifier `Describe` lists.
- Every identifier in `GetOverridden` appears in `Describe`.
- Values are effective values: whatever the layered configuration resolved to.

## Worked example

```
Describe() → [
  { id: "dock.size", type: "double", section: "dock",
    label: "Size", description: "Dock size multiplier",
    default: <1.0>, min: <0.5>, max: <2.0>, step: <0.05>, apply: "live" },
  { id: "dock.position", type: "enum", section: "dock",
    label: "Position on screen", default: <"bottom">,
    choices: ["bottom", "left", "right"],
    choice_labels: ["Bottom", "Left", "Right"], apply: "live" },
  { id: "input.touchpad_click_method", type: "enum", section: "input",
    label: "Click method", default: <"clickfinger">,
    choices: ["clickfinger", "buttonareas"],
    choice_labels: ["Click with fingers", "Click in corners"], apply: "live" },
  ...
]

Set("dock.size", <1.25>) → "applied"
  … Changed({ "dock.size": <1.25> })

Set("login.greeter_command", <"otto-greeter">) → "pending-restart"

Set("dock.size", <9.0>) → error OutOfRange
Set("dock.size", <"big">) → error InvalidType

Reset("dock.size") → "applied"
  … Changed({ "dock.size": <1.0> })      // whatever the lower layers give
```

## Open

- Shortcuts are a keyed collection, not a scalar, and do not fit `Set(id,
  value)` cleanly. They likely need their own methods
  (`SetShortcut`/`ResetShortcut`/`ListShortcuts`) rather than an identifier per
  binding. Deferred until the Keyboard pane is built; nothing else in the API
  depends on the answer.
- Per-output display settings need a stable identity for a display, so
  settings follow the panel rather than the connector. That identity is not
  yet defined, so display identifiers are not final.
