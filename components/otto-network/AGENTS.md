# otto-network

Otto's view of the network through NetworkManager. One read of the system bus becomes a `Snapshot` (devices, the Wi-Fi networks in range folded one per name, saved networks, VPNs, the connection in use and its addresses). The crate also has the actions a network UI takes (Wi-Fi on and off, join, disconnect, scan, VPN up and down, forget, auto-join) and a watcher that says when to read again. NetworkManager keeps all the state. Nothing here caches it.

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
- `src/client.rs`: `read`, the actions and `watch`, all async on the caller's runtime.
- `src/blocking.rs`: the same calls for a plain thread, each on a runtime of its own.

## Rules
- No toolkit and no Wayland. The crate is the model and the bus calls, and the UIs draw.
- Proxies come from `otto-dbus` and are built uncached (`CacheProperties::No`), because a snapshot is read once and dropped.
- Decisions about what to show (order, grouping, the glyph) belong in `model.rs` with a test, not in a UI, so the bar and Settings never disagree.
- The behaviour is specified in `specs/network.md`. A change to it updates the spec.

## See also
- [Network spec](../../specs/network.md), [otto-dbus](../otto-dbus/AGENTS.md).
