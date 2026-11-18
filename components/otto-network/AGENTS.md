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
- the private hardware address and metered setting of a profile
- read a saved Wi-Fi password for sharing (`wifi_share`, through `GetSecrets`, after polkit confirms it's the person)

It has the secret agent's request and reply shapes, and a watcher that says when to read again. NetworkManager keeps all the state. Nothing here caches it.

- **Layer:** service. Used by `otto-bar` (the network indicator) and `otto-settings` (the Network pane).

## Dependencies
- **Internal:** `otto-dbus` for the NetworkManager proxies (`networkmanager`) and polkit's (`polkit`).
- **External:** NetworkManager on the system bus, and polkit's Authority there (`org.otto.network.reveal-password`, `resources/polkit/org.otto.network.policy`). `zbus` and `tokio` (current-thread runtime for `blocking`).

## Develop
- `cargo test -p otto-network` runs the model's tests, which need no bus, and the stand-in NetworkManager tests below.
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
- `src/rfkill.rs`: the kernel's Wi-Fi kill switches, named by a `Radios` value (`Radios::system()` is `/sys/class/rfkill` and `/dev/rfkill`). It reads the blocks, follows them, and lifts a soft block.
- `src/client.rs`: `read`, the actions, `watch` (which says what changed: strengths, saved connections, or anything else, and ends when the bus closes) and `follow` (read, then keep reading as little as each change needs, retrying a failed read), all async on the caller's runtime. The calls touching the kill switches have `_with` variants taking `Radios`.
- `tests/mock_networkmanager.rs` runs `read` and every action against a stand-in NetworkManager, and a stand-in polkit Authority for `wifi_share`, on a private `dbus-daemon`. It is skipped where there is none. Its calls take `Radios::none()` and name their bus, so a test never reads or changes the host's radios and never sets the environment.
- `src/reveal.rs`: the polkit check before a password is read (`ACTION`), and `ShareError`, which says whether a failed read was a refusal.
- `src/blocking.rs`: the same calls for a plain thread, each on a runtime of its own.

## Rules
- No toolkit and no Wayland. The crate is the model and the bus calls, and the UIs draw.
- A UI follows NetworkManager with `follow`, not by calling `read` from a `watch` callback: a `blocking` call made inside the watch's runtime panics.
- Proxies come from `otto-dbus` and are built uncached (`CacheProperties::No`), because a snapshot is read once and dropped.
- A write to a profile changes only the properties it edits, each in one spelling only: `address-data`, never the older `addresses`; `dns-data` (1.42+) where the profile already uses it or an entry needs it, the older `dns` otherwise. Settings come without secrets, and an update without secrets keeps the stored ones. Never send a profile's secrets back. A password read for sharing travels as `Password`, whose `Debug` hides it; never log one.
- Decisions about what to show (order, grouping, the glyph) belong in `model.rs` with a test, not in a UI, so the bar and Settings never disagree.
- The behaviour is specified in `specs/network.md`. A change to it updates the spec.

## See also
- [Network spec](../../specs/network.md), [otto-dbus](../otto-dbus/AGENTS.md).
