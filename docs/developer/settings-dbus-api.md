## `org.otto.Settings` — the settings D-Bus contract

> **Status**: implemented on the compositor side (`src/settings/`,
> `src/settings_service.rs`), ahead of the app. This document is the interface
> both sides build against, so the compositor and the settings app can be
> written independently. Behavioural requirements live in
> [specs/settings-app.md](../../specs/settings-app.md); this is the wire.
>
> What the compositor serves today: 47 settings, of which fifteen are `live`
> and the rest are `restart`. The four `dock.*` ones — `size`, `position`,
> `autohide`, `magnification` — reconfigure the dock in place. The eleven
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

The two existing getters (`GetColorScheme`, `GetIconTheme`) stay exactly as
they are — `xdg-desktop-portal-otto` depends on them and must not be disturbed.

### Values

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

### Identifiers

A stable dotted path matching the configuration structure: `dock.size`,
`input.pointer_accel_speed`, `power_management.on_lid_close`. Top-level keys
have no prefix: `accent_color`, `cursor_size`.

**Identifiers are a permanent contract.** Once shipped, an identifier is never
renamed or repurposed — an app built against an older compositor must keep
working, and the app is the thing that hardcodes these strings.

### Methods

```
Describe()                 → aa{sv}    schema, one dict per setting
GetAll()                   → a{sv}     current effective value of every setting
Get(id: s)                 → v         one value
GetOverridden()            → as        ids currently set in the writable file
Set(id: s, value: v)       → s         status, see below
Reset(id: s)               → s         status
```

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
| `choices` | `as` | optional, required for `enum` |

Unknown keys must be ignored by clients, so the schema can grow without
breaking them.

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

### Signal

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

### Errors

| Error name | When |
| --- | --- |
| `org.otto.Settings.Error.UnknownSetting` | no such identifier |
| `org.otto.Settings.Error.InvalidType` | variant type does not match the schema |
| `org.otto.Settings.Error.OutOfRange` | outside `min`/`max`, or not in `choices` |
| `org.otto.Settings.Error.Unsupported` | `apply` is `unsupported` on this system |
| `org.otto.Settings.Error.ApplyFailed` | valid, but the compositor could not apply it; message says why |

`ApplyFailed` means nothing was persisted and the running state is unchanged.

### What a client can rely on

- The schema is authoritative for labels, ranges, defaults and apply
  behaviour. A client renders from `Describe` rather than hardcoding them.
- `GetAll` returns a value for every identifier `Describe` lists.
- Every identifier in `GetOverridden` appears in `Describe`.
- Values are effective values: whatever the layered configuration resolved to.

### Worked example

```
Describe() → [
  { id: "dock.size", type: "double", section: "dock",
    label: "Size", description: "Dock size multiplier",
    default: <1.0>, min: <0.5>, max: <2.0>, apply: "live" },
  { id: "dock.position", type: "enum", section: "dock",
    label: "Position on screen", default: <"bottom">,
    choices: ["bottom", "left", "right"], apply: "live" },
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

### Open

- Shortcuts are a keyed collection, not a scalar, and do not fit `Set(id,
  value)` cleanly. They likely need their own methods
  (`SetShortcut`/`ResetShortcut`/`ListShortcuts`) rather than an identifier per
  binding. Deferred until the Keyboard pane is built; nothing else in the API
  depends on the answer.
- Per-output display settings need a stable identity for a display, so
  settings follow the panel rather than the connector. That identity is not
  yet defined, so display identifiers are not final.
