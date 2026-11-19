# otto-audio-graph

The PipeWire graph, followed live. A `Connection` talks to PipeWire on a thread of its own and reports every change as an `Event`:
- nodes, with every property and their state (bound, so their full properties and state changes arrive)
- ports, with direction, `port.name`, `audio.channel` and media type
- links, and which client made each
- clients, by `application.name`
- the engine settings in the `settings` metadata (`clock.rate`, `clock.quantum`, `clock.force-*`)
- the default output and input, from the `default` metadata, and every audio node's volume and mute, from its `Props`

It takes commands: make a link (lingering, or owned by this connection), remove one, set an engine setting, set the default output or input, set a node's volume or mute. A `Graph` applies the events and is what a UI or a daemon reads. When the server goes away it reports `Disconnected` and connects again by itself.

The crate also has what saving a routing needs:
- `NodeIdentity`: a device by `node.name`, an app's stream by app, direction and media role
- `Routing`: the saved links and forced engine settings, in `$XDG_CONFIG_HOME/otto/audio/routing.toml`
- `pair_channels`: how two folded ports are linked channel by channel
- `LoopGuard`: when to stop putting back a link another program keeps removing

- **Layer:** service. Used by `otto-audio` (the window and `otto-audio-service`) and `otto-bar` (the sound menu).

## Dependencies
- **Internal:** `otto-foundations` (the config directory, `write_atomically`).
- **External:** `pipewire` 0.9 (`deny.toml` lists this crate among its wrappers), `serde` and `toml` for the routing file.

## Develop
- `cargo test -p otto-audio-graph` runs the model, identity, pairing, routing and guard tests, which need no server.
- `cargo run -p otto-audio-graph --example dump` prints the live graph once it has synced, then every change. It only listens.
- `cargo test -p otto-audio-graph --test live_link -- --ignored` links and unlinks two null sinks it makes with `pactl` and removes afterwards. Needs a running server.
- `cargo run -p otto-audio-graph --example mixer` prints the defaults, every output and input with its volume, and the apps playing. It only listens.
- `cargo test -p otto-audio-graph --test live_mixer -- --ignored` sets the volume and mute of a null sink it makes and removes.
- `pw-cli ls Node`, `pw-link -l` and `pw-metadata -n settings` show what the events describe.

## Where things are
- `src/connection.rs`: the thread, binding nodes and the `settings` metadata, and the commands.
- `src/model.rs`: `Graph`, `Node`, `Port`, `Link`, `Client`, `Event` and the setting keys.
- `src/mixer.rs`: the defaults, volumes and app streams: their events, the `Graph` queries, the `Connection` commands and the thread side (the `default` metadata, cards' `Route`s, nodes' `Props`).
- `src/identity.rs`, `src/routing.rs`, `src/pairing.rs`, `src/guard.rs`: the parts above.

## Rules
- No toolkit and no Wayland. Callers get plain types; nothing from `pipewire` appears in the public API.
- `on_event` runs on the PipeWire thread. A UI pushes the event somewhere and wakes its own loop; it never draws from there.
- A link the window makes is `Lingering`, so it outlives the window. A link the service restores is `Owned`: it lasts while the service runs.
- The routing file is replaced whole with `write_atomically`; the service follows it by watching its directory. Keep the format readable by hand, and bump `VERSION` only with a migration.
- Never link or unlink the person's own nodes from a test. Make throwaway nodes, as `tests/live_link.rs` does.
- A volume is changed where the session manager keeps it: the card's `Route` (with `save`) for a node that belongs to a card, the node's `Props` otherwise, as `wpctl` does. Volumes are cubic on the wire; `Volume::linear` is what a slider shows.
- A test never changes the person's own volume, mute or default device: `tests/live_mixer.rs` works on a null sink, and the `default` write path has only unit tests.

## See also
- [Audio routing spec](../../specs/audio-routing.md), [otto-audio](../otto-audio/AGENTS.md).
