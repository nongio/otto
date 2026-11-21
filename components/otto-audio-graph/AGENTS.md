# otto-audio-graph

The PipeWire graph, followed live. A `Connection` talks to PipeWire on a thread of its own and reports every change as an `Event`:
- nodes, with every property and their state (bound, so their full properties and state changes arrive)
- ports, with direction, `port.name`, `audio.channel` and media type
- links, and which client made each
- clients, by `application.name`
- the engine settings in the `settings` metadata (`clock.rate`, `clock.quantum`, `clock.force-*`)
- devices (cards, Bluetooth devices, cameras), bound, with their `EnumProfile`, `Profile`, `EnumRoute` and `Route` params; a device is reported whole once a core sync says every result of an enumeration is in
- the default output and input, from the `default` metadata, and every audio node's volume and mute, from its `Props`

It takes commands: make a link (lingering, or owned by this connection), remove one, set an engine setting, switch a device's profile or port (`set_profile`, `set_route`, kept by the server), keep a set of virtual devices made, keep a set of effects made (`keep_effects`) and set an effect's controls on its node (`set_effect_controls`), keep AirPlay discovery loaded (`keep_airplay`, reported back as `Event::AirPlay` and `Graph::airplay`), set the default output or input, set a node's volume or mute, and move a stream by setting its `target.object` (`set_target`). A Bluetooth codec is a profile, so a codec switch is a profile switch. A `Graph` applies the events and is what a UI or a daemon reads. When the server goes away it reports `Disconnected` and connects again by itself.

The crate also has what saving a routing needs:
- `NodeIdentity`: a device by `node.name`, an app's stream by app, direction and media role
- `Routing`: the saved links, forced engine settings, virtual devices, effects and the AirPlay switch (`airplay`), in `$XDG_CONFIG_HOME/otto/audio/routing.toml`
- `VirtualDevice`: an output (null sink), input (`Audio/Source/Virtual`) or loopback (`libpipewire-module-loopback`, loaded into the connection's own context), named `otto-virtual.<id>` (a loopback's sides `.in` and `.out`) after an id that never changes, so saved links survive renames and restarts
- `Effect`: an equaliser, limiter, compressor or noise suppressor sitting on a device by `node.name`, made with `libpipewire-module-filter-chain` in the connection's own context and named `otto-effect.<id>` (its stream `.play` or `.rec`). Every value is a control of the chain, so edits are live through the node's `Props`; `support` says whether a kind's LADSPA plugin is installed, `response_db` and `Coefficients` are the equaliser's biquads, `parse_autoeq` reads an AutoEQ `ParametricEQ.txt`
- AirPlay: `Node::is_airplay` tells a RAOP sink (`node.name` starting `raop_sink.`, or `raop.ip`), `airplay_missing` whether the RAOP module is installed and `avahi-daemon` runs (file checks only), and the connection that keeps it loads `libpipewire-module-raop-discover` into its own context, which makes a sink per receiver and removes them when unloaded
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
- `cargo test -p otto-audio-graph --test live_effects -- --ignored` puts an equaliser on a null sink it makes, turns it off through its controls from a second connection, and removes both. Needs `pw-dump` and `pw-play`.
- `pw-cli ls Node`, `pw-link -l` and `pw-metadata -n settings` show what the events describe.

## Where things are
- `src/connection.rs`: the thread, binding nodes and the `settings` metadata, and the commands.
- `src/model.rs`: `Graph`, `Node`, `Port`, `Link`, `Client`, `Event` and the setting keys.
- `src/devices.rs`: `Device`, `Profile`, `Route`, the Bluetooth codec names, following a device's params on the thread and the pods that set them.
- `src/mixer.rs`: the defaults, volumes and app streams: their events, the `Graph` queries, the `Connection` commands and the thread side (the `default` metadata, cards' `Route`s, nodes' `Props`).
- `src/identity.rs`, `src/routing.rs`, `src/pairing.rs`, `src/guard.rs`: the parts above.
- `src/virtual_devices.rs`: `VirtualDevice`, the `Routing` methods that add, rename and remove one, and `Made`, which makes them on the PipeWire thread and makes them again after a reconnect.
- `src/effects/`: `mod.rs` (`Effect`, its kinds and values, the `Routing` methods), `chain.rs` (the plugins, the filter graph and its controls), `biquad.rs`, `autoeq.rs`, and `made.rs` (making them on the thread and setting their `Props`).
- `src/airplay.rs`: `AirPlay`, `airplay_missing`, the RAOP sink test and `Discovery`, which keeps the discovery module loaded on the thread.
- `src/module.rs`: loading a module into the connection's context, the crate's only direct `libpipewire` FFI. It listens for the module's `destroy`, since a filter chain or loopback destroys itself when its streams fail.

## Rules
- No toolkit and no Wayland. Callers get plain types; nothing from `pipewire` appears in the public API.
- `on_event` runs on the PipeWire thread. A UI pushes the event somewhere and wakes its own loop; it never draws from there.
- A link the window makes is `Lingering`, so it outlives the window. A link the service restores is `Owned`: it lasts while the service runs. Virtual devices are owned too (`object.linger = false`, or a module in the connection's context).
- A loopback's, effect's or AirPlay discovery's module must be destroyed before its context: `State::clear` drops discovery, the effects and `Made` first.
- Never load AirPlay discovery from a test: it finds and makes the person's speakers. Test `is_airplay` and the routing field without a server.
- An effect's chain keeps one shape (10 `bq_raw` biquads per channel, a wet/dry mixer), so a change to its values is a `Props` change, never a new chain. A `bq_raw` cannot start without a `config` of coefficients. A filter chain that has never run reports the controls it was first given; it applies later ones when it runs.
- The routing file is replaced whole with `write_atomically`; the service follows it by watching its directory. Keep the format readable by hand, and bump `VERSION` only with a migration.
- Never link or unlink the person's own nodes from a test. Make throwaway nodes, as `tests/live_link.rs` does.
- Never switch the person's devices' profiles or ports from a test either; the param pods are unit-tested by reading them back.
- A volume is changed where the session manager keeps it: the card's `Route` (with `save`) for a node that belongs to a card, the node's `Props` otherwise, as `wpctl` does. Volumes are cubic on the wire; `Volume::linear` is what a slider shows.
- Never put an effect on the person's devices or move their streams from a test; put it on a null sink, as `tests/live_effects.rs` does.
- A test never changes the person's own volume, mute or default device: `tests/live_mixer.rs` works on a null sink, and the `default` write path has only unit tests.

## See also
- [Audio routing spec](../../specs/audio-routing.md), [otto-audio](../otto-audio/AGENTS.md).
