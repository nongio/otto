# otto-network

Otto's view of the network through NetworkManager. One read of the system bus becomes a `Snapshot`:
- devices, with each one's link and wired speed
- the Wi-Fi networks in range, folded one per name, and the access point in use with its link rate
- every saved profile with its IPv4 and IPv6 configuration
- VPNs
- the connection in use and its addresses

The crate also has the actions a network UI takes:
- Wi-Fi on and off
- join, by saved profile, with a password, or by a hidden name
- disconnect
- scan
- VPN up and down
- forget
- auto-join
- save and reapply TCP/IP and DNS

It has the secret agent's request and reply shapes, and a watcher that says when to read again. NetworkManager keeps all the state. Nothing here caches it.

- **Layer:** service. Used by `otto-bar` (the network indicator) and `otto-settings` (the Network pane).

## Dependencies
- **Internal:** `otto-dbus` for the NetworkManager proxies (`networkmanager`).
- **External:** NetworkManager on the system bus. `zbus` and `tokio` (current-thread runtime for `blocking`).

## Develop
- `cargo test -p otto-network` runs the model's tests. They need no bus.
- Against a real NetworkManager, run the bar or Settings. `nmcli monitor` shows the changes the watcher reacts to.

## Where things are
- `src/model.rs`: the snapshot types and every decision the UIs share:
  - one row per network name
  - sort order
  - signal bars
  - security from access point flags
  - the indicator's glyph
- `src/profile.rs`: saved connections, read and written in NetworkManager's `a{sa{sv}}` shape:
  - IP configuration, with its validation
  - the keys a dialog asks for (`KeyKind`)
  - the secret agent's `SecretRequest`
- `src/client.rs`: `read`, the actions, `watch` (which says what changed: strengths, saved connections, or anything else) and `follow` (read, then keep reading as little as each change needs), all async on the caller's runtime.
- `tests/mock_networkmanager.rs` runs `read` and every action against a stand-in NetworkManager on a private `dbus-daemon`. It is skipped where there is none.
- `src/blocking.rs`: the same calls for a plain thread, each on a runtime of its own.

## Rules
- No toolkit and no Wayland. The crate is the model and the bus calls, and the UIs draw.
- A UI follows NetworkManager with `follow`, not by calling `read` from a `watch` callback: a `blocking` call made inside the watch's runtime panics.
- Proxies come from `otto-dbus` and are built uncached (`CacheProperties::No`), because a snapshot is read once and dropped.
- A write to a profile changes only the properties it edits, each in one spelling only: `address-data`, never the older `addresses`; `dns-data` (1.42+) where the profile already uses it or an entry needs it, the older `dns` otherwise. Settings come without secrets, and an update without secrets keeps the stored ones. Never send a profile's secrets back.
- Decisions about what to show (order, grouping, the glyph) belong in `model.rs` with a test, not in a UI, so the bar and Settings never disagree.
- The behaviour is specified in `specs/network.md`. A change to it updates the spec.

## See also
- [Network spec](../../specs/network.md), [otto-dbus](../otto-dbus/AGENTS.md).
