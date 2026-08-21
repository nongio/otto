# Remote-desktop indicator

While `otto-rdp` is serving a remote client, the desktop shows a red dot in the
top bar's tray with a menu to stop sharing. This page is the contract between
the bridge and whatever draws that indicator.

## Is there an existing indicator to join?

There was none. Before this, nothing in Otto surfaced "something can see your
screen" to the user:

- `src/screenshare/` implements `org.otto.ScreenCast` — sessions, streams,
  cursor modes, PipeWire nodes. It is a capture *API*, consumed by
  `xdg-desktop-portal-otto` and by `otto-rdp --connector`. It has no notion of
  an indicator, no "capture is active" property, and no UI consumer.
- `components/otto-bar/` had a tray, a clock and an app menu. No privacy UI.
- The dock's "running" dots and `src/workspaces/osd.rs` are unrelated —
  application state and volume/brightness OSDs.

So this is the first such indicator, not a second parallel system. It
deliberately reuses the tray protocols the bar already speaks rather than
introducing an Otto-specific one, which means a future compositor-side
screencast indicator can be a second StatusNotifierItem and the two will queue
up in the same place with the same appearance, instead of competing.

## The contract

`otto-rdp` publishes a **StatusNotifierItem** with a **dbusmenu**. Both are
standard, already-implemented-by-otto-bar protocols; no Otto-specific D-Bus
interface is involved.

| | |
|---|---|
| Bus name | `org.kde.StatusNotifierItem-<pid>-1` (session bus) |
| Item object | `/StatusNotifierItem`, interface `org.kde.StatusNotifierItem` |
| Menu object | `/MenuBar`, interface `com.canonical.dbusmenu` |
| Registered with | `org.kde.StatusNotifierWatcher.RegisterStatusNotifierItem` |

Because the name embeds the pid, several bridges (one per output) each get
their own item and their own dot.

### Item properties

| Property | Value |
|---|---|
| `Category` | `SystemServices` — the session acting on the user, not an app's own icon |
| `Id` | `otto-rdp` |
| `Title` | `Screen shared with <peer host>` |
| `Status` | `Active`, always. Never `Passive`: hosts may hide passive items, and a privacy signal the user cannot see is worse than none |
| `IconName` | `media-record` |
| `IconPixmap` | 22px and 44px red discs, drawn by the bridge. ARGB32, network byte order, unpremultiplied |
| `ItemIsMenu` | `true` — hosts open the menu rather than synthesising an activation |
| `Menu` | `/MenuBar` |
| `ToolTip` | title as above, description `<output> is being shared since HH:MM` |

`Activate` / `SecondaryActivate` / `ContextMenu` are accepted and do nothing:
there is no window to raise, and everything the user can do lives in the menu.

The pixmap is drawn rather than looked up so the indicator does not depend on
the icon theme shipping `media-record`. Otto's bar prefers a pixmap over a name,
so the drawn disc is what actually renders.

### Menu

A flat, fixed layout. `GetLayout(0, -1, [])` returns:

```
1  "Sharing <output> with <peer host>"   disabled
2  "Since HH:MM"                         disabled
3  ─────────────────────────────────
4  "Stop Sharing"                        enabled
```

`Event(4, "clicked", …)` stops the session. Ids 1–3 are inert.

The layout never changes within a session, because hosts cache it — Otto's bar
prefetches it at registration time and renders from that cache. That is why the
"since" line is an absolute clock time rather than a live elapsed duration, and
why the transport codec is not shown at all: the codec is only settled after the
client's EGFX capability exchange, so any value put in the menu at registration
time would risk being wrong.

## When the indicator is up

The indicator is published **only while a client is actually being served
frames**, and it is not configurable.

| State | Indicator |
|---|---|
| `otto-rdp` not running | hidden |
| running, listening, no client | hidden |
| TCP connected, handshake not finished | hidden |
| client being served frames | **shown** |
| client disconnects, bridge keeps listening | hidden |
| bridge exits, crashes, or is `SIGKILL`ed | hidden |

The signal goes up when the display handler starts serving that client
(`VirtualOutputDisplay::updates`), not at TCP accept — a port scan or an
abandoned handshake must not claim someone is watching.

### Why it cannot go stale

SNI hosts drop an item when its bus name loses its owner. There is no reliable
unregister in practice: the KDE spec's `StatusNotifierItemUnregistered` is
emitted by watchers, not items, and hosts do not universally act on it. So the
item lives on its **own `zbus::Connection`**, opened when sharing starts:

- normal end of session — the bridge calls `release_name`;
- crash, `SIGKILL`, OOM — the socket closes and the bus daemon releases the name.

Both paths end at the same `NameOwnerChanged`, which is what every host already
watches. A stale "you are being recorded" icon is not reachable.

Two details this depends on, both easy to get wrong:

- **`Connection` is reference-counted.** Proxies and signal streams built from
  it hold their own clones, so dropping the stored handle does not close the
  connection. The name is therefore released explicitly, and the
  re-registration watcher task is aborted first.
- **A `hide` can land while `publish` is still in flight.** A separate `wanted`
  flag, not the presence of the stored connection, decides whether a
  just-published item is kept or immediately retracted.

### Host restarts

If no `StatusNotifierWatcher` is running, registration fails quietly — the
bridge still serves, there is just nowhere to draw the icon. The bridge watches
`NameOwnerChanged` for `org.kde.StatusNotifierWatcher` and re-registers whenever
a host appears, so restarting otto-bar mid-session brings the dot back.

## Stop Sharing

`Stop Sharing` ends the **whole bridge**, not just the current client:

1. the bridge marks itself stopping and retracts the indicator;
2. `ServerEvent::Quit` drops the live connection;
3. the connection handler returns `PostConnectionAction::Stop`, which breaks the
   accept loop, so the listening port closes and the process exits.

Merely disconnecting the client would leave the port open for an immediate
reconnect, which is not what a user means by stopping sharing.

## Implementation

- `components/otto-rdp/src/indicator.rs` — the item, the menu, the lifecycle.
- `components/otto-rdp/src/rdp.rs` — `updates()` raises the indicator.
- `components/otto-rdp/src/main.rs` — wires the connection handler and the stop
  path into the ironrdp server.

Nothing in `components/otto-bar/` changed: the bar's existing
`StatusNotifierWatcher`/host (`src/tray.rs`) and dbusmenu client
(`src/dbusmenu.rs`) render the indicator as they would any other tray item.

See also `specs/rdp-bridge.md` and `docs/developer/rdp-virtual-output.md`.
