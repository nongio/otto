# Audio Routing

**Status:** draft
**Related specs:** [settings-app.md](./settings-app.md) (the Sound pane),
[dynamic-island.md](./dynamic-island.md) (the music island),
[portal-access-dialog.md](./portal-access-dialog.md) (microphone and camera use)

## Summary

`otto-audio` is a standalone app for everything the sound server can do
beyond picking a device and a volume: the live PipeWire graph as a patchbay,
the engine's rate and latency, device profiles and codecs, effects, virtual
devices and per-app routing rules. A companion service, `otto-audio-service`,
keeps what the app sets up in force across app launches, device plugs and
restarts of the sound server, so a route made once stays made.

## Goals

- Every audio, MIDI and video node of the PipeWire graph, its ports and its
  links are shown live, and any two compatible ports can be linked or
  unlinked by hand.
- A link made in the app can be saved. A saved link comes back whenever both
  ends exist again: after an app relaunches, a device is plugged back in, the
  sound server restarts or the session starts.
- The engine's sample rate and buffer size (quantum) can be read and changed
  live, and the latency, load and dropouts (xruns) of every node are visible
  while they happen.
- Each card's profile, port, Bluetooth codec and allowed sample rates can be
  set from one place.
- Effects (equaliser, dynamics, noise suppression, convolution) and virtual
  devices (combined outputs, null sinks, loopbacks) can be created, edited
  live and removed, and exist in every later session.
- Per-app rules route an app to a device or effect whenever it plays or
  records, without the user moving it by hand each time.
- Named setups capture the whole routing and switch it in one action.
- Nothing the app does needs a terminal, a hand-edited configuration file or
  a restart of the sound server, except where Behaviour says so.

## Non-Goals

- Replacing the Settings Sound pane. That pane stays the everyday mixer;
  this app is where routing and the engine live.
- A digital audio workstation: no recording, timelines or plugin hosting
  beyond the effects listed here.
- LV2/LADSPA plugin browsing and arbitrary filter-chain graphs in the first
  version. The effects are a fixed, curated set (see Open Questions).
- Managing JACK or PulseAudio servers that are not PipeWire. With no
  PipeWire server the app explains that and shows nothing else.
- Network audio (AirPlay, RTP, ROC, AES67) set-up.

## Behavior

### The app

`otto-audio` opens one window. Running it again raises that window. It has
a desktop entry so it appears in the launcher and can be kept in the dock.
The Settings Sound pane ends with a row that opens it; nothing else in
Settings depends on it.

The window has a sidebar with six sections: **Graph**, **Devices**,
**Engine**, **Effects**, **Virtual devices** and **Rules**, and a setup
switcher in the title bar (see Setups). It opens on Graph, or on the section
that was open when it was last closed.

Everything shown follows the sound server live: a node appearing,
disappearing, changing state or being linked by any program (the app, the
service, WirePlumber, qpwgraph, `pw-link`) shows in the window within one
frame of the server announcing it. The app never polls.

With no PipeWire server answering, the window shows a single message saying
so, with a button to start the user's PipeWire units, and reconnects by
itself when the server appears.

### Graph

A canvas of nodes, each drawn as a card with its input ports on the left
and its output ports on the right, and links as wires between them.

- **Grouping.** Cards are laid out in four columns by role: sources
  (capture devices, apps that produce sound), effects and virtual devices,
  sinks (playback devices, apps that record), and MIDI/video. A card the
  user has moved stays where it was put, remembered by the node's identity
  (see Identity), across launches. A new node with no remembered place is
  placed at the end of its column without moving any other card.
- **Media filter.** Toggles for Audio, MIDI and Video show and hide those
  nodes and their links. Audio and MIDI are on by default, Video off.
- **Channels.** A node's ports are folded by default into one port per
  direction, and the links between two folded nodes into one wire labelled
  with the channel count when it is not two. Unfolding a card shows every
  port by its channel name (FL, FR, AUX0…).
- **Linking.** Dragging from a port to a compatible port of another node
  links them. Dragging between two folded ports links channels by name
  (FL→FL, FR→FR); when the names do not match, they are linked in order, and
  a mono port fans out to every channel of the other side. Ports of
  different media types never link, and the drag shows that by dimming them.
  A wire can be removed by selecting it and pressing Delete, or from its
  context menu.
- **Meters.** Each audio wire shows the level of the signal on it, and each
  card a peak meter per direction. Meters run only while the window is on
  screen and the Graph is the open section.
- **Card details.** Selecting a card shows a side panel with its name, the
  app or device behind it, media class, format, rate, quantum it asked for,
  latency, state (running, idle, suspended) and every PipeWire property in a
  collapsible list.
- **Save.** A link made by hand is temporary until saved. Temporary wires
  are drawn dashed, saved wires solid. **Save link** on a wire, or
  **Save routing** for everything on the canvas, saves them into the current
  setup. A saved link can be **Forgotten**, which keeps the wire until it is
  removed but no longer restores it.
- **Undo.** Linking, unlinking, saving, forgetting and moving cards can be
  undone and redone with Cmd+Z and Cmd+Shift+Z, for the life of the window.
- **Search.** Typing filters the canvas to nodes whose name, app or
  description matches, dimming the others.
- **Navigation.** The canvas pans with two-finger scroll or a middle-button
  drag and zooms with pinch or Cmd+scroll. Cmd+0 fits every visible card.

A link the app did not make, such as one WirePlumber made, is drawn like a
temporary wire and can be saved like one.

### Saved links and the service

`otto-audio-service` runs once per session, started with it, independent of
the window. It holds the saved routing of the current setup and keeps it in
force:

- When both ends of a saved link exist and are not linked, it links them
  within 100 ms of the second end appearing.
- When a saved link is removed by another program, it is put back, unless
  it was removed from the app, which forgets it or, with the link temporary,
  just removes it.
- A saved route from an app stream replaces the automatic route the session
  manager would give it: the stream is not also played on the default
  device. A saved route from a device to another node adds to whatever the
  device is already linked to.
- When one end of a saved link is gone, the link waits. It is never deleted
  because a device is unplugged.

With the service not running, the app shows a banner on Graph and Rules
saying saved links and rules are not being kept, with a button to start it.
The routing the service set up stays as it was until something else changes
it.

### Identity

Nodes come and go with new ids every time, so everything saved names nodes
by identity instead:

- A device node by its node name, which stays the same across plugs and
  boots for the same device.
- An app stream by its app (application name, falling back to the process
  binary) and its role: playback or capture, plus the media role when the
  app gives one. An app with several streams of the same role matches each
  of them.
- A port by its node's identity and its channel or port name.

The details panel shows a node's identity, and every saved item can be
edited to match more loosely (any stream of this app, any node whose name
starts with…) or more tightly (only the stream titled…).

### Devices

One group per card, then one per Bluetooth device:

- **Profile**, among the profiles that can run now, including Pro Audio.
- **Port** for each of its sinks and sources, with unplugged jacks marked.
- **Sample rates** the device may be opened at, from those it supports.
- **Volume control**: hardware mixer, or software only.
- **Bluetooth codec** (SBC, SBC-XQ, AAC, aptX, aptX HD, LDAC, LC3…) and,
  for headsets, the headset profile used while a microphone is open. Only
  codecs both sides support are offered. A codec that needs the device to
  reconnect says so before switching.

Changes apply at once and are kept by the sound server's own device state,
so they hold without the service.

### Engine

- **Sample rate** in force and the rates the engine may switch between.
  Choosing *Follow the content* lets the engine switch to the rate a stream
  plays at when nothing else is running at another rate.
- **Quantum**: the buffer size the engine runs at now, and its range
  (minimum, default, maximum), or *Fixed* at one value. Each value shows
  the latency it gives at the current rate in milliseconds.
- **Live table**, as `pw-top` shows it, one row per running node: quantum,
  rate, wait and busy time, load as a share of the cycle, and the number of
  dropouts since the window opened, highlighted for a second when it goes
  up. The table updates once a second.
- A **DSP load** graph of the last minute.

Rate and quantum changes apply live. The service puts them back when the
sound server restarts, so they hold across sessions. **Reset** returns each
value to the server's own default and stops the service from setting it.

### Effects

An effect is a virtual device that processes what goes through it. It is
either an **output effect** (apps play into it, and it plays on a device)
or an **input effect** (it records from a device, and apps record from it).
The kinds are:

- **Equaliser**: parametric, up to 16 bands of peak, shelf, high-pass and
  low-pass filters, with a frequency response curve the bands are dragged
  on, and presets that can be saved and loaded (including AutoEQ headphone
  profiles from a file).
- **Dynamics**: compressor and limiter, for loudness levelling.
- **Noise suppression**: for microphones. Offered only when a suppression
  plugin is installed; otherwise the kind says which package provides it.
- **Convolution**: an impulse response file, for room correction or
  headphone virtualisation.

Effects can be chained: an effect's output can be another effect. Every
parameter change is heard immediately, without a gap in the sound. Each
effect has a bypass toggle that passes the sound through unchanged, and
shows on the graph as a card like any device.

Effects exist while the service runs and are created when it starts. They
are part of the current setup.

### Virtual devices

- **Combined output**: one output that plays on several devices at once,
  each with its own volume and delay to line them up.
- **Null sink**: an output that goes nowhere, for routing between apps
  (its monitor is a source any app can record).
- **Loopback**: plays a source on a sink, to hear a microphone or an input
  live, with its own volume and latency.

Like effects, they exist while the service runs and belong to the current
setup. Each can be renamed; the name is what apps and the Sound pane show.

### Rules

A rule matches app streams by identity and, while it is enabled:

- routes them to a device, effect or virtual device, falling back to the
  default when that target does not exist;
- optionally sets their starting volume and mute;
- optionally keeps them from being moved by the session manager when the
  default device changes.

Rules are applied when a matching stream appears. A rule never moves a
stream the user has moved by hand since it appeared. The list is ordered,
and the first matching rule wins. A stream that is playing can be turned
into a rule from its context menu on the graph.

### Setups

A setup is a named routing: its saved links, effects, virtual devices and
rules. There is always a current setup; the first is called *Default*.
The switcher in the title bar can create, duplicate, rename, delete and
switch setups. Switching applies the new setup within half a second without
restarting the sound server: links, effects and virtual devices the new
setup does not have are removed, and ones it has are created. Engine and
device settings are shared by every setup.

A setup can be exported to a file and imported from one.

### The top bar sound menu

The top bar carries a sound indicator, the everyday control for what the
app does in depth. It follows the graph live, like the app, and never polls.

- **The indicator.** A speaker glyph showing the default output's volume
  in three steps, struck through while muted. Scrolling over it changes the
  volume in steps of 5 %; a middle click mutes and unmutes.
- **Output.** A slider for the default output's volume, with its mute
  button, and the output's name. Under it, every output device and virtual
  output by its name, the default one check-marked; picking one makes it
  the default, and playing streams move to it unless a rule or a saved link
  holds them.
- **Input.** The same for the default input: a slider with mute, and the
  inputs to pick from. A level meter under the slider moves while the menu
  is open, so a person can see the microphone hears them.
- **Apps.** Every stream playing sound, by app, each with its own slider
  and mute. An app with several streams shows once.
- **Setup.** When more than one setup exists, the active setup and the
  others to switch to.
- **Audio…** The last row opens the app.

Volumes are the session manager's: a change here shows in the app, in
Settings and in any other mixer, and the server remembers it. The menu
reads and changes the same graph as the app through `otto-audio-graph`.

### Interactions with the rest of Otto

- Meters record from nodes, but the app never counts as an app using the
  microphone or camera: no mic-in-use indicator, no portal prompt.
- The Sound pane and the top bar's sound menu show effects and virtual
  devices like any device, by their names.
- Default device changes from anywhere are reflected in the app, and the
  graph marks the default sink and source.

## Constraints & Edge Cases

- **Other routers.** WirePlumber, qpwgraph's own patchbay or a script may
  also make and remove links. The service only restores links it owns and
  only removes links a setup switch removes; it must not fight another
  program over a link in a loop. A link removed again within 2 s of being
  restored three times is left removed and reported once on Graph.
- **Exclusive apps.** A node that refuses to be linked (a device opened
  exclusively, a stream marked not to be linked) shows its ports dimmed and
  saved links to it wait.
- **Rate changes mid-stream.** Changing the rate or quantum while audio
  plays may glitch once; it must never drop a stream or a link.
- **Bluetooth reconnects.** A Bluetooth device reconnecting gets new nodes;
  identity by node name must still match them.
- **Hot unplug.** A device unplugged with effects or combined outputs on it
  leaves those effects in place, idle, and they resume when it returns.
- **Large graphs.** The canvas stays smooth with 200 nodes and 1000 links,
  and meters cost nothing while the window is hidden.
- **Feedback loops.** Linking that closes a loop (an output fed back to its
  own input) is allowed but warned about on the wire, since it can be loud.
- **Service crash.** Effects and virtual devices vanish with the service;
  streams on them move to the default device. The service restarts once
  on failure and recreates them.
- **Video nodes.** Cameras and screencasts appear under the Video filter;
  linking them works like audio, but they have no meters.

## First version

The first version builds Graph, Engine and the service; Devices, Effects,
Virtual devices, Rules, Setups and meters come later, and the sidebar shows
only the sections that exist. Where it consciously differs from the above:

- **Saved routing file.** The saved links live in
  `$XDG_CONFIG_HOME/otto/audio/routing.toml`, written by the app and
  followed by the service through a file watch; card positions live in
  `$XDG_STATE_HOME/otto/audio/layout.toml`. There is one implicit setup.
- **Links the service makes are its own.** They last while the service runs
  and go with it when it stops; the service makes them again when it
  starts. Links made in the app outlive the app.
- **App streams.** A saved route from an app stream is added to the session
  manager's automatic route rather than replacing it.
- **Loop guard.** A link the service gives up on is reported in the
  service's log, not on Graph.
- **Engine.** Rate and quantum can be forced to one value each or left to
  the engine (*Auto*); there is no *Follow the content*, rate list, quantum
  range or live table. Forced values are kept in the routing file, and the
  service puts them back when the sound server starts. Each value shows its
  latency; the defaults and range are shown as text.
- **Graph.** No undo, search, context menus or feedback-loop warnings. A
  wire is removed with Delete or the toolbar's Remove. The details panel
  shows the name, state, app, format, latency, media class and identity,
  every connection with whether it is saved (an unsaved one can be saved
  there), and every property, folded away until opened. The title bar
  shows the engine's rate, buffer and latency; the sidebar's foot says
  whether the service is keeping saved links.
- **Top bar.** No sound menu yet (Plan, step 3).
- **One window.** Running the app again opens a second window.

## Plan

What is left, in the order it is built. Each step ships on its own and
updates this spec's First version notes.

1. **Graph polish.** Search over node names, which dims what does not
   match and pans to the first that does; a context menu on cards and wires
   (save, forget, remove, fold, show in Devices); undo for links made and
   removed; a warning on a link that closes a feedback loop; the default
   output and input marked on their cards; momentum on the canvas pan.
2. **Meters.** A level bar on every running card and in the details panel,
   read from a monitor stream per visible node while the window is open,
   none while it is hidden. Needs a capture path in `otto-audio-graph` that
   never shows as an app using the microphone.
3. **Top bar sound menu.** The indicator and menu above, in `otto-bar`:
   default sink and source from the `default` metadata, volume and mute
   through the node's Props, per-app streams. The meter reuses step 2.
4. **Devices.** One card per device: profile, port, sample rates, volume
   control, and Bluetooth codec and quality, set through the device's
   params. HDMI and other profile clashes are said before they happen.
5. **Engine, live.** The DSP load chart, dropouts, the driver, and the
   table of running nodes from the profiler; the allowed rates and
   *Follow the content*; a quantum range next to the fixed value.
6. **Virtual devices.** Null sinks and sources and loopbacks made and
   owned by the service, kept in the routing file, shown on Graph in
   column 1.
7. **Effects.** A parametric equaliser with presets and AutoEQ import, a
   limiter, a compressor, and noise suppression, hosted by the service as
   filter chains, each inserted between streams and a device. This covers
   what JamesDSP does for most people.
8. **Rules.** Which device an app goes to, as session manager targets,
   with the app picked from what has played.
9. **Setups.** Named sets of saved links, rules, effects and engine
   settings; switching from the title bar and the sound menu; export and
   import.

## Rationale

- **A separate app, not a Settings pane.** The graph needs a large,
  resizable canvas, and routing is a tool used in sessions, not a setting
  set once. Settings keeps the everyday mixer.
- **A service, not only WirePlumber configuration.** WirePlumber's rules
  can choose a stream's target but cannot keep arbitrary port-to-port links.
  A service watching the graph can, and it lets effects and virtual devices
  change live without restarting the sound server. Rules use the session
  manager's own targeting so it links the stream itself and does not
  compete with the service.
- **Effects hosted by the service.** Hosting them in the sound server's
  configuration would need a server restart for every effect added or
  removed, cutting all audio. The cost is that they exist only while the
  service runs; the service restarting itself covers that.
- **Device settings stay with the server.** Profiles, ports and codecs are
  already remembered by the session manager; keeping a second copy would let
  them disagree.
- **Identity, not ids.** Ids change on every appearance, so anything saved
  by id would be lost on the first replug.
- **All media types.** MIDI and video share the same graph; leaving them out
  would make the app wrong about what is linked to what.

## Open Questions

- The app's name in the launcher: *Audio*, *Sound Studio*, *Patchbay*?
- Should arbitrary LV2/LADSPA plugins be insertable as effects in a later
  version, and with which plugin UI?
- Should a setup switch automatically when a device appears (a headset, a
  dock)?
- Should Engine settings be per setup after all, for a low-latency
  recording setup next to a power-saving everyday one?
