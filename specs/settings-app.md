# Settings App

**Status:** draft — compositor side of the settings interface implemented
**Wire contract:** [docs/developer/settings-dbus-api.md](../docs/developer/settings-dbus-api.md)
**Related specs:** [multi-output.md](./multi-output.md), [lock-screen.md](./lock-screen.md), [login-mode.md](./login-mode.md), [lid-power.md](./lid-power.md), [topbar.md](./topbar.md)

## Summary

A graphical settings application for Otto (`otto-settings`) that lets a user
inspect and change compositor configuration without editing TOML. The
compositor remains the sole owner of the configuration file; the app is a
D-Bus client that reads a described schema, sets values, and observes changes.

## Goals

- A user can change any exposed setting from a GUI and see it take effect
  immediately, without restarting the compositor, for every setting the
  compositor can apply live.
- A setting changed in the app survives a compositor restart.
- A setting the compositor *cannot* apply live is still changeable, is clearly
  marked as requiring a restart, and is persisted.
- Hand-editing the configuration file continues to work, and a running app
  reflects such an edit without being restarted.
- Changing a setting never materialises unrelated inherited defaults into the
  user's configuration file.
- A user can arrange multiple displays (position, primary, resolution, refresh
  rate) graphically.
- The app is usable with keyboard alone, and every setting it presents is
  reachable by search.

## Non-Goals

- Wallpaper selection and management. Otto exposes only `background_color` /
  `background_image`; those two keys are in scope, and `background_image` is
  chosen through the desktop portal's file picker
  ([file-picker.md](./file-picker.md)) rather than by typing a path. Cropping,
  per-output wallpapers and slideshow behaviour still belong to a separate
  application.
- Configuring anything Otto does not already have a configuration key for.
  This app exposes the existing surface; it does not motivate new features.
- Application-level settings for other Otto components (bar layout, launcher
  behaviour) unless they are already compositor configuration keys.
- Exposing the whole configuration surface. The app presents a curated set of
  user-facing preferences. Session plumbing (startup commands, layer-shell
  limits, virtual outputs, renderer flags) stays hand-edited, and there is no
  generic catch-all pane for it.
- Editing the system-wide configuration, or any configuration layer other than
  the user's writable one.
- A configuration migration or versioning system.

## Behavior

### Configuration ownership

The compositor owns the writable configuration file. It is the only process
that writes it. The app never reads or writes configuration files.

The compositor's in-memory configuration is mutable at runtime. When a value
changes — from any source — the compositor must, in this order:

1. Validate the new value against the schema. An invalid value is rejected and
   nothing else happens.
2. Apply the value to the running system, as far as it can.
3. Persist the value to the writable configuration file.
4. Announce the change to all observers.

If applying fails, the value must not be persisted and the caller must be told
why. A value that is valid but cannot be applied live is persisted, announced,
and reported as pending a restart.

### The settings interface

The compositor exposes a settings service with these operations (the exact
method names, types and error names are fixed by the wire contract):

- **Describe** — returns the schema: for every setting, its identifier, type,
  default value, allowed range or enumeration, the section it belongs to, a
  human-readable label and description, and whether changing it takes effect
  immediately or requires a restart.
- **Get all** — returns the current effective value of every setting.
- **Set** — sets one setting to one value. Fails with a distinguishable error
  for: unknown identifier, wrong type, value out of range, and apply failure.
- **Reset** — removes a setting from the user's configuration, so its value
  reverts to whatever the lower configuration layers provide. This is distinct
  from setting it to its default value: after a reset, a later change to a
  lower layer is again visible.
- **Changed signal** — emitted whenever any setting's effective value changes,
  carrying the identifiers and their new values. It is emitted for changes
  originating anywhere: this app, another client, an in-compositor interaction
  such as dragging the dock handle, or an external edit of a configuration
  file.

Two further read operations exist for the app's benefit: **Get**, one value,
and **Get overridden**, the identifiers currently set in the writable file —
what the app needs to decide which settings offer a reset.

A setting identifier is a stable dotted path matching the configuration
structure (for example `dock.size`, `input.pointer_accel_speed`). Identifiers
are part of the contract and must not change once shipped. The path is the path
into the configuration structure itself, so a setting is read and written
generically rather than through per-setting code; a schema row whose identifier
does not resolve in the configuration is a bug, and is caught by a test.

The schema describes a curated set, not the whole configuration. Settings that
are lists rather than scalars — dock bookmarks, keyboard shortcuts, display
profiles — have no identifier and are not settable through this interface.

Whether a setting applies live is a property of the compositor, not of the
setting: today all of `dock.*` (size, position, autohide, magnification,
magnification amount and spread, icon tint, tint colour and tint strength),
`accent_color`, `background_image` and `background_color`, and the
touchpad/pointer half of `input.*` (tap, tap-drag,
drag lock, click method, disable-while-typing, natural scroll, left-handed,
middle-click emulation, scroll speed, pointer acceleration speed and profile)
are reconciled with a changed configuration, and every other setting — including the keyboard
`input.xkb_*` settings — is described as requiring a restart. Marking a
setting `live` is a promise that it takes effect, so the schema is widened
only as apply paths are written.

### Choosing a file

A setting whose value is a path is presented as the path plus a **Choose…**
button, not as a text field: a path typed by hand is a path that can be
wrong, and the picker already knows how to browse, filter and validate.

The button opens the file chooser through the **portal frontend**
(`org.freedesktop.portal.Desktop`), the same door any other application uses,
rather than calling Otto's backend directly. That is deliberate: it exercises
the `portals.conf` routing that decides Otto's own picker serves the request
at all, and a misrouted frontend is exactly the failure this would otherwise
hide.

The call blocks for as long as the dialog is up, so it runs on a thread of
its own and wakes the main loop when it is answered — the same arrangement
the `Changed` listener uses, and for the same reason: the draw path and input
both run on the main thread and cannot wait on a user browsing their disk.

Cancelling changes nothing. A request that fails is reported, and the setting
keeps the value it had; the app never writes an empty path because a dialog
went wrong.

### Persistence

Persisting a setting writes only that setting's key. Every other key in its
section, and every other section, is left byte-for-byte alone, including
comments and formatting. Resetting a setting removes exactly that key, and
removes its containing table if that leaves the table empty.

Resetting cannot know what the lower layers hold without asking them, so it
removes the key and then re-reads the whole layered configuration, applying and
announcing everything that moved. An unrelated edit made to a file since the
last read is therefore picked up by a reset, which is the same reconciliation
the file watcher will use.

The rule this protects: the writable configuration file is the highest-priority
layer. A default value written into it is indistinguishable from a deliberate
choice, and permanently shadows the same key in every lower layer.

Which file that is follows from the same rule: the writable file is the
highest-priority layer that is actually present, so what is written is what
takes effect. Writing into any lower layer would apply the setting live and let
it revert on the next reload, which reads as a settings app that does not save.
The system-wide layer is never written — it belongs to the package rather than
to the user — and when no layer above it exists, the user's own config is
created rather than a file in whatever directory the session started from. A
session whose writable file is not the user's own config says so in the log,
since from `~/.config/otto` alone that is invisible.

### External edits

When a configuration file changes on disk, the compositor reloads the layered
configuration, applies every value that differs from the running state, and
emits the changed signal for those values. A value the compositor cannot apply
live is reported as pending a restart rather than being silently ignored.

An edit made by the compositor's own persistence step must not be observed as
an external edit.

The reconciliation this needs — reload the layers, diff against the running
configuration, apply and announce what moved — exists and is what a reset uses.
The file watch that would trigger it does not yet.

### The application

`otto-settings` is a standalone windowed application. On launch it fetches the
schema and current values, and subscribes to the changed signal. A change
arriving over the signal updates the displayed value, whether or not this app
caused it.

The window presents a list of panes and the selected pane's contents. The panes
are:

- **General** — appearance (light/dark), accent colour, font family, background
  colour and image, cursor theme and size, icon theme, locales.
- **Displays** — see below.
- **Dock** — size, position, autohide, magnification, minimise effect.
- **Keyboard** — repeat delay and rate, then shortcuts.
- **Trackpad & Mouse** — the pointer and touchpad settings.
- **Sound** — enabled, theme.
- **Power** — lid switch handling, power button action.
- **Lock & Login** — automatic lock timeout, which locker runs the lock screen,
  which greeter runs the login screen.

Settings outside these panes are not shown. The schema may describe settings
the app does not present; the app must ignore them rather than render them
generically.

A search field filters the presented settings by label, description, and
identifier, and selecting a result reveals that setting in its pane.

Each setting shows whether it currently differs from its inherited value, and
offers a per-setting reset when it does.

A setting the compositor reports as requiring a restart is shown with that
status after being changed.

Changes take effect on interaction. There is no apply or save step, and no
confirmation dialog for ordinary settings.

The sidebar and the titlebar are translucent materials over the compositor's
backdrop blur; the pane's ground is opaque. The sidebar is tinted heavily
enough to read as frost rather than as a hole in the window, and the titlebar
only slightly, so the frost runs across the whole top of the window instead of
stopping at the sidebar's edge. Without the surface-style protocol both are
painted flat — a tint over an unblurred desktop is not a material.

The selected pane scrolls independently of the window's chrome. The window
surface paints the chrome — titlebar, sidebar, divider and the grounds behind
them — and nothing else; the pane's background, content and scrollbar are
separate surfaces the compositor crops and moves, so a frame of scrolling costs
the app no drawing at all. The content surface is only repainted when a scroll
approaches the edge of what has been drawn, or when something other than the
scroll changes what a row looks like. A configure that repeats the size the
window already has changes nothing about the pane and repaints nothing, so an
idle window draws no frames at all. Pointer events over the pane land on
those surfaces and are translated back into the pane's coordinates before
hit-testing; the window's resize edges along the pane's right and bottom stay
live.

### Shortcuts

The keyboard pane lists every shortcut with its action. Recording a new
shortcut captures the next key combination pressed, including modifiers, while
suppressing that combination from reaching the rest of the system.

If a captured combination is already bound, the conflict is shown and the user
chooses whether to reassign it. Reassigning clears the previous binding. A
combination that cannot be bound is rejected with a reason.

Shortcuts can be reset individually and as a group.

### Displays

The displays pane shows every connected output as a proportionally sized
rectangle in a canvas reflecting their arrangement. Dragging a rectangle
changes that output's position; released positions snap to edge alignment with
neighbouring outputs and may not leave an output overlapping another or
disconnected from the arrangement.

Selecting an output exposes its resolution, refresh rate, scale, and whether it
is primary. Resolution and refresh rate offer only modes the output actually
advertises: the arrangement and both mode lists are read from `wl_output`,
which carries each output's name, its place in the desktop and every mode its
connector can be driven at. That is the app's display probe — it needs no
privileged access and it follows hotplug. Refresh rates are listed for the
resolution currently selected, since a display does not offer every rate at
every size.

Position and primary changes apply immediately. A resolution, refresh rate, or
scale change applies immediately, and must be confirmed by the user within a
short timeout; if it is not confirmed, the previous configuration is restored.
This protects against a mode the display cannot in fact show.

An output that disconnects while the pane is open disappears from the canvas;
the configuration recorded for it is retained so reconnecting restores it.

Per-output settings are stored against a stable identity for that display, so
they follow the display rather than the connector it happens to occupy.

## Constraints & Edge Cases

- Two settings clients may set values concurrently; last write wins, and both
  observe the result through the changed signal.
- The app must function when the compositor is running but some settings cannot
  be applied — for example, display settings under a windowed development
  backend. Such settings are shown as unavailable rather than hidden, with the
  reason.
- The app must not require the compositor to be built with development
  features.
- The changed signal must be coalesced: a continuous interaction such as
  dragging a size slider must not produce one persistence write per frame.
  Values apply live during the drag; persistence happens when the interaction
  settles.
- Existing in-compositor settings interactions — dragging the dock handle to
  resize, dock context-menu toggles — must go through the same set path, so
  they announce and persist identically. They must not keep private shadow
  copies of configuration state.
- A configuration file that fails to parse must leave the running configuration
  untouched and surface an error, not fall back to defaults.
- Resetting a setting whose value comes from a lower layer must not write an
  empty override.
- Settings that are read once at startup and cannot be re-read (backend
  selection, session-level flags) must be marked restart-required in the schema
  rather than silently failing to apply.
- Display mode changes on the production backend require rebuilding the output's
  scanout surface; the arrangement must survive that rebuild, and windows must
  be re-placed rather than lost when an output's geometry changes.
- Removing the last enabled output must be prevented.
- The locker and greeter are external commands, and a value that does not start
  leaves the user unable to unlock or unable to log in. Both must be validated
  as resolvable executables before being persisted, and offered as a choice
  among detected candidates rather than as free text where possible. A locker
  change takes effect at the next lock, not immediately; a greeter change takes
  effect at the next login and is reported as such.

## Rationale

**The compositor owns the file, the app is a client.** The alternative — the
app writes TOML and the compositor watches for changes — creates two writers
for one file. The compositor already writes it when the dock is resized, so the
race is real rather than hypothetical. Client-only writes also force every
client to re-implement the surgical-merge discipline described under
Persistence, and make "the setting took effect" and "the setting was saved"
independently failable. With a single owner they cannot disagree.

**Reset is distinct from set-to-default.** Otto's configuration is layered.
Writing a default into the top layer permanently pins the value, defeating the
layering. Users expect a reset to mean "stop overriding this".

**Schema is served, not compiled into the app.** The app renders from a
description supplied by the compositor, so labels, ranges, defaults and
restart-required status cannot drift between the two, and a range change needs
no new app build.

**Every pane is bespoke, and the app shows less than the schema does.** A
generic catch-all pane would guarantee nothing is unreachable, but it would put
greeter commands and renderer flags in front of a user looking for their
trackpad settings, and it would grow a new row every time a developer adds a
key. Configuration that only makes sense to someone reading the source is
better served by the TOML file. The cost is accepted deliberately: adding a
user-facing setting means also placing it in a pane.

**Display mode changes are confirmed, position changes are not.** A bad mode
can leave a display showing nothing, and the user cannot then click to undo it.
A bad position is always recoverable.

**Wallpaper is excluded.** It needs file browsing, thumbnails, per-output
assignment and scaling modes — a different application with a different shape.
The two raw configuration keys are still exposed here so the setting is not
unreachable in the meantime.

## Open Questions

- Should the app be able to change settings for a display that is configured
  but not currently connected?
- Shortcuts are a keyed collection rather than a scalar and do not fit
  `Set(id, value)`; they are excluded from the schema for now and will need
  their own operations. Deferred until the Keyboard pane exists — nothing else
  in the interface depends on the answer.
- Does the schema need to express relationships between settings (one setting
  only meaningful when another is enabled), or is it enough for panes to
  hard-code that?
- Should shortcut bindings be a settings identifier like any other, or their
  own operations on the interface, given they are a keyed collection rather
  than a scalar?
- Is per-output scale a display setting here, given `screen_scale` is currently
  global?
