# Dynamic Island

`otto-islands` is the floating pill at the top-centre of the screen. It is
Otto's notification centre, live-activity display and permission-dialog surface,
all in one morphing element.

It is a separate program. Start it from your config:

```toml
[[exec_once]]
cmd = "otto-islands"
```

## At rest

With nothing to show, the island is two circles side by side — a large `O` and a
small `o`, echoing the Otto logo — sitting in the empty centre of the
[top bar](topbar.md).

## Notifications

Otto Islands implements the standard `org.freedesktop.Notifications` D-Bus
service, so it is a drop-in notification daemon: anything that sends a desktop
notification (`notify-send`, your mail client, a build script) shows up here.

Run only one notification daemon at a time — starting `otto-islands` alongside
`dunst` or `mako` means whichever claims the bus name first wins and the other
silently does nothing.

Try it:

```sh
notify-send "Build finished" "42 tests passed"
```

### Grouping and modes

All notifications from the same application are grouped into **one island**.
Each island has three presentations:

| Mode | Looks like | How to get there |
|------|------------|------------------|
| **Mini** | Small circle with the app icon and a count badge | Default for unfocused islands |
| **Compact** | A pill with icon, app name, count and a chevron | Click a mini circle |
| **Expanded** | The pill, with the notification cards stacked below it | Click the compact pill |

Clicking cycles **Mini → Compact → Expanded → Compact**. Only one island can be
compact or expanded at a time; focusing one shrinks the previous back to a mini
circle.

Multiple islands sit as a centred horizontal row, oldest on the left.

Hovering any island grows it slightly — an invitation, not a mode change.

### Actions

Notifications carrying actions render them as buttons on the card. Clicking one
sends the standard `ActionInvoked` signal back to the sending application.

## Live activities

Beyond notifications, any program can push an **activity** into the island over
D-Bus — a long-running thing with a title, an icon, and optionally a progress
bar. A build, a file transfer, a backup, the currently playing track.

The interface is `org.otto.Island1` at `/org/otto/Island`:

```sh
# Create an activity; prints the activity id
gdbus call --session \
  --dest org.otto.Island \
  --object-path /org/otto/Island \
  --method org.otto.Island1.CreateActivity \
  "my-script" "Syncing photos" "folder-download" 0.0 0 "normal" true
```

| Argument | Meaning |
|----------|---------|
| `app_id` | Who is sending it |
| `title` | The text shown |
| `icon` | An icon name from your theme |
| `progress` | `0.0`–`1.0` for a progress bar; negative for none |
| `timeout_ms` | Auto-dismiss after this long; `0` to stay |
| `priority` | `"low"`, `"normal"`, `"high"` or `"critical"` |
| `live` | `true` if it updates continuously — the island shows an animated indicator |

Then update or dismiss it by id:

```sh
gdbus call --session --dest org.otto.Island \
  --object-path /org/otto/Island \
  --method org.otto.Island1.UpdateActivity 1 "Syncing photos (40%)" 0.4

gdbus call --session --dest org.otto.Island \
  --object-path /org/otto/Island \
  --method org.otto.Island1.DismissActivity 1
```

`UpdateActivity` takes an empty title to leave the text unchanged, and a
negative progress to clear the bar.

This makes the island a general-purpose status surface for your own scripts.

## Permission dialogs

When an application asks to share your screen, the request surfaces as an
interactive island panel: what is being requested, by whom, and a list of things
to pick from (which monitor to share, which AirPlay receiver to send to).

This is brokered by the portal and rendered by `otto-islands` over the
`org.otto.Dialog1` interface, which mirrors the freedesktop
`org.freedesktop.impl.portal.Access` contract. See
[Screen Sharing](screen-sharing.md).

If `otto-islands` is not running there is nothing to render the dialog, and the
request resolves to a safe default — denied — rather than hanging.

## Volume and brightness

Volume and screen brightness changes show a **separate on-screen indicator**
drawn by the compositor itself, not by the island. Pressing
`XF86AudioRaiseVolume` or `XF86MonBrightnessUp` adjusts the level, shows the
indicator, and (for volume) plays a feedback sound if
[audio feedback](audio.md) is enabled.

Volume steps by 5% per press.

## Visuals

The island is drawn with the `otto-surface-style-unstable-v1` protocol rather
than as a flat picture: each element is its own surface whose size, position,
corner radius, blur, shadow and colour are animated by the **compositor** using
springs. That is why it morphs between shapes fluidly instead of cross-fading —
the geometry itself is animated, and the content is drawn once at the target
size and revealed as the shape grows.

## Troubleshooting

**Nothing appears when I send a notification.** Check that `otto-islands` owns
the bus name:

```sh
busctl --user status org.freedesktop.Notifications
```

If another daemon holds it, stop that one first.

**The island is behind my top bar, or in the wrong place.** It anchors to the
top-centre as an overlay layer surface, assuming a bar around 30 points tall. A
much taller custom panel will overlap it.

**Screen sharing hangs or is instantly denied.** `otto-islands` is what renders
the consent dialog — make sure it is running.

## Not yet supported

- A notification history or "do not disturb" mode
- Persisting notifications across restarts
- Configuration file (position, size, timeouts are fixed)
- Multi-monitor: the island shows on the primary monitor only
