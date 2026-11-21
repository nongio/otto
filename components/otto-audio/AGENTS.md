# otto-audio

Audio: the PipeWire graph as a patchbay, and the sound server's rate and buffer size. Two binaries:
- `otto-audio`, the window. The Graph section shows every node as a card and links as wires: drag from a port to a port to link, select a wire to save, forget or remove it. Its toolbar switches to a Matrix (outputs against inputs, a cell per pair; press one to link or select it) and to Lanes (each path from a source to a destination, read-only). The Devices section switches each sound device's profile, ports and Bluetooth codec through the device's params, which the server keeps, and ends with the AirPlay speakers card: a switch kept in the routing file, what discovery lacks here (`airplay_missing`, read when the section opens), and the speakers found. The Engine section forces the sample rate and quantum through the `settings` metadata. The Effects section lists the equalisers, limiters, compressors and noise suppressors kept in the routing file, adds and removes them, and edits one: an equaliser on its response curve and band cards, with Flat and AutoEQ import, the others with sliders. The Virtual devices section lists the outputs, inputs and loopbacks kept in the routing file and adds, renames and removes them.
- `otto-audio-service`, a headless daemon. It reads the saved routing and links both ends of every saved link whenever they exist and are not linked, and puts forced engine settings back when the server starts. It makes the virtual devices and effects the file lists and removes ones taken out of it; they last while it runs. It moves the app streams on an effect's device onto the effect through their `target.object`, and back when the effect is removed. With `airplay = true` in the file it keeps AirPlay discovery loaded (`Connection::keep_airplay`) and logs where it stands.

The two share nothing at run time but the routing file, `$XDG_CONFIG_HOME/otto/audio/routing.toml`. The window writes it; the service reads it again whenever it changes. The window tells whether the service runs by looking for a PipeWire client named `otto-audio-service`.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` (window, drawing, controls, i18n, logging, the XDG state directory), `otto-audio-graph` (the graph, its connection, identity and the routing file).
- **External:** a PipeWire server; for AirPlay speakers PipeWire's RAOP modules (`pipewire-zeroconf`) and `avahi-daemon`; for the limiter and compressor `swh-plugins`, for noise suppression `noise-suppression-for-voice` (LADSPA). The file chooser portal for AutoEQ import. `systemctl --user` for the Start buttons (the service unit, PipeWire's units). `libc` for the service's inotify watch. `toml` and `serde` for the card layout in `$XDG_STATE_HOME/otto/audio/layout.toml`.

## Develop
- `cargo run -p otto-audio` opens the window; against a dev compositor, `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-audio`.
- `cargo run -p otto-audio --bin otto-audio -- --png out.png [--engine | --devices | --effects | --virtual | --matrix | --lanes]` draws the window offscreen, light and dark side by side, from the live graph. It only listens. With `--effects` and no effects saved it shows three sample ones (Headphone EQ, a limiter after it, Mic cleanup) on the default devices, without making them. With `--virtual` and no virtual devices saved it shows three sample ones (Stream mix, Call input, Mic monitor); run your own service on a temp file naming those three to see their live state.
- `cargo run -p otto-audio --bin otto-audio-service` runs the service; `--routing PATH` keeps another file instead of the person's. Its unit is `otto-audio-service.service`, off until enabled with `systemctl --user enable --now otto-audio-service`.
- To try virtual devices, run your own service on a temp file (`--routing /tmp/…/routing.toml`) with `[[virtual]]` entries; everything it made goes when it stops.
- To try effects, do the same with `[[effect]]` entries whose `device` is a null sink you made, and play into the sink with `pw-play --target`: the service moves the stream onto the effect. The window always writes the person's own routing file, so edit the temp file by hand rather than through the window. `pw-dump` shows a chain's controls under its node's `Props` once it has run.
- To try linking, make throwaway nodes: `pactl load-module module-null-sink sink_name=otto-audio-test-a`, and `pactl unload-module ID` afterwards.
- Logs: `RUST_LOG=otto_audio=debug,otto_audio_service=debug`. Tests: `cargo test -p otto-audio`.

## Where things are
- `src/main.rs` is the app: the window, the PipeWire inbox drained in `on_update`, keys and pinch. `src/preview.rs` is `--png`.
- `src/ui/mod.rs` is the chrome, pointer handling and the actions; each section has its own file: `ui/graph_pane.rs` (the toolbar with the view switch, banner and details panel), `ui/views_pane.rs` (the Matrix and Lanes views and their scroll views), `ui/devices_pane.rs` (the device cards and what each offers, and the AirPlay card under them) `ui/engine_pane.rs`, `ui/effects_pane.rs` (the effect list, New effect form and editor; its drags, scrubs and scroll hook into `press`, `motion`, `release` and `scroll` in `mod.rs`, a value dragged is set on the node live and saved on release, and the AutoEQ import's answer is picked up by `Ui::poll_import` from `on_update`), and `ui/virtual_pane.rs` (the Virtual devices form, with otto-kit's `TextInput` for the name fed by `Ui::text_key` before the window's own keys, and its cards). A pop-up a press asks for is a `MenuRequest` that `main.rs` opens with the kit's `DropdownMenu`.
- `src/graph_view.rs` (in the lib, with its tests) lays the canvas out: columns, folded rows, wires, hit-testing and drawing. It also builds the Matrix's cells (`matrix`) and the Lanes' paths (`lanes`) from the same scene, so all three views share rows, wires and saved state.
- `src/keeper.rs` decides what the service links; `src/effect_streams.rs` which streams it moves onto effects; `src/bin/otto-audio-service.rs` runs both, watches the routing file and hands its virtual devices, effects and AirPlay switch to `Connection::keep_virtual_devices`, `keep_effects` and `keep_airplay`.
- `src/layout.rs` remembers card positions by node identity, and the Graph view last shown.

## Rules
- Nothing polls. The window repaints when an event arrives or the person does something; the service acts on graph events and file changes.
- Every control records its rect and action while it is drawn (`targets` in `ui/mod.rs`), and presses test that list.
- Removing a saved wire forgets it in the routing file before unlinking, so the service does not put it back.
- The window never makes virtual devices or effects itself: it writes the file and the service makes them. It only sets an existing effect's controls while a value is dragged.
- Do not link or unlink the person's nodes while testing, and do not change their engine settings for long; use null sinks. Never switch the profile or port of their real devices from a test: the `--png` preview only reads them. Never put an effect on their devices or move their streams: use null sinks. Never turn AirPlay discovery on in their routing file from a test or a trial; use a service on a temp file.
- The behaviour is specified in `specs/audio-routing.md`. A change to it updates the spec.

## See also
- [Audio routing spec](../../specs/audio-routing.md), [otto-audio-graph](../otto-audio-graph/AGENTS.md), [otto-kit](../otto-kit/AGENTS.md).
