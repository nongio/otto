# Login Mode

**Status:** draft
**Related specs:** [multi-output](./multi-output.md), [workspaces-multi-output](./workspaces-multi-output.md)

## Summary

Otto can run as the host compositor for a login screen instead of a user
session. In this mode it drives a single output, shows no session chrome, and
hosts exactly one client — the greeter — which authenticates the user through
[greetd](https://sr.ht/~kennylevinsen/greetd/). Otto itself never handles
credentials.

This is the role `cage` plays for `gtkgreet`, or `weston --shell=fullscreen-shell`
plays for SDDM's Qt greeter.

## Goals

- Starting Otto with `--login` produces a screen showing only the greeter.
- No dock, app switcher, expose, or workspace selector is reachable, by input
  or by animation, for the lifetime of the greeter.
- Only one output is driven. Other connectors are left unmodeset and get no
  Wayland output global, so the greeter cannot be mirrored or moved onto them.
- The greeter is an ordinary Wayland client, replaceable via configuration.
- Otto links no PAM code and runs no privileged authentication logic.
- The greeter can be developed and styled inside a normal Otto session, without
  root or a spare VT.

## Non-Goals

- **Screen locking.** Locking an existing session has a different lifecycle
  (the session outlives the lock) and belongs to `ext-session-lock-v1`.
  The two share the "one exclusive surface, no chrome" behavior but not the
  process model.
- **Owning authentication.** Otto does not call PAM, enumerate shadow entries,
  or manage VTs. greetd does all of that.
- **Seamless handoff to the session.** greetd terminates the greeter and execs
  the user session in its place; see Constraints.
- **Multi-seat.** One greeter on one seat.

## Behavior

### Entering login mode

- Otto enters login mode when started with the `--login` flag. The mode is
  fixed for the process lifetime; there is no runtime transition into or out of
  it.
- `--login` is orthogonal to the backend flag and may be combined with
  `--tty-udev` (production) or `--winit` (development).

### Output policy

- The first desktop connector brought up becomes the primary output and is the
  only one driven.
- Every subsequent desktop connector — at startup or on hotplug — is ignored:
  no modeset, no `wl_output` global, no workspace mapping.
- Non-desktop connectors (VR headsets and similar) continue to be offered for
  DRM leasing, unchanged.

### Chrome suppression

- The dock is never shown.
- The app switcher, expose view, and workspace selector are never visible,
  including via keyboard shortcuts or gestures.
- The scene graph keeps its usual shape; suppressed chrome is present but
  hidden, so no other subsystem has to special-case a missing node.

### Greeter lifecycle

- Instead of running the autostart entries (`exec_once`, XDG autostart), Otto
  launches exactly one client: the configured greeter.
- The greeter command comes from `login.greeter_command` and
  `login.greeter_args`, defaulting to `otto-greeter`.
  `$OTTO_GREETER_COMMAND` overrides both, parsed as a whitespace-separated
  argv, for testing uninstalled builds.
- The greeter inherits `$GREETD_SOCK` from Otto's environment, which greetd
  sets when it spawns Otto as the greeter session.
- The greeter is tied to Otto's lifetime: if Otto dies, the greeter is sent
  `SIGTERM`.

### Greeter surface

- The greeter presents itself as a `wlr-layer-shell` surface on the **overlay**
  layer, anchored to all four edges with size `0x0`, so the compositor sizes it
  to the full output.
- It requests an exclusive zone of `-1`, so any panel's exclusive zone is
  ignored and the greeter covers the whole output.
- It requests `exclusive` keyboard interactivity, so no other surface can
  receive keyboard input while it is mapped.

### Panel

- What the greeter draws lives in `components/otto-auth-ui`, not in the greeter
  binary: a `lay-rs` scene of the frosted card, the avatar, the field and the
  chrome, built on otto-kit's layer engine. The card is a real
  `BlendMode::BackgroundBlur` layer, and state changes carry transitions rather
  than being redrawn from scratch.
- The crate knows nothing of greetd or PAM. A client turns its own conversation
  into a `View`, hands it to `Panel::update`, and asks `Panel::action_at` where
  a click landed. Sizes are logical points, on the canvas otto-kit provides
  with the buffer scale already applied.
- This is what a screen locker would reuse: same panel, different surface
  (`ext-session-lock-v1`) and different backend (PAM). A lock screen passes no
  session name, which is what removes the session picker.
- Appearance — wallpaper, background, accent, font — is read from Otto's own
  config, so the login screen matches the session it leads into. A greeter user
  can only read `/etc/otto/config.toml`; a lock screen also picks up the user's.
- Because the panel animates, the client must keep painting while a transition
  runs. The engine advances animations on its own thread, but only the client
  can put the result on screen.
- While a fingerprint is expected the field carries an animated Touch ID mark,
  and `Panel::wants_frames` says so. Such content is not driven by a property
  changing, so the client calls `Panel::animate` before each paint: the engine
  records a layer's draw closure into a picture and replays it until something
  damages the layer, and a closure that reads the clock has to declare that
  damage itself or it is painted once and frozen.
- A recognised fingerprint must be *seen*. greetd kills the greeter as soon as
  `start_session` succeeds, so the pause belongs before the request, not after:
  on `success` the greeter enters an `Accepted` stage, the panel finishes the
  mark's draw-in early, settles it into the accepted colour and holds it, and
  `start_session` follows once `Panel::wants_frames` falls. A timeout bounds the
  wait so a panel that never settles cannot strand the login, and input is
  ignored throughout — an Escape in that window would otherwise cancel a session
  greetd has already authenticated. A password login has no mark and is not
  delayed.

### Authentication conversation

The greeter drives greetd's IPC (`greetd-ipc(7)`): a native-endian `u32` length
prefix followed by a JSON payload, in both directions.

- On submitting a username, the greeter sends `create_session`.
- greetd replies with `auth_message`, `success`, or `error`:
  - `auth_message` of type `secret` — prompt, input masked.
  - `auth_message` of type `visible` — prompt, input echoed.
  - `auth_message` of type `info` or `error` — displayed, and acknowledged
    immediately with a null response; no user input is expected. A fingerprint
    prompt (`pam_fprintd`) arrives this way.
  - `success` — the conversation is complete; the greeter sends
    `start_session` with the selected session's argv.
  - `error` — displayed; the greeter sends `cancel_session` and returns to the
    username field.
- After `start_session` succeeds, greetd terminates the greeter and execs the
  session. The greeter shows that the session is starting and stops accepting
  input.
- If the greeter is still running a few seconds after `start_session`
  succeeded, the exec did not happen. It must report that the session failed to
  start and return to the username field rather than waiting indefinitely — a
  greeter frozen on "Starting session…" leaves no way to log in.

### Session selection

- Sessions are discovered from `.desktop` files in
  `/usr/share/wayland-sessions` and `/usr/local/share/wayland-sessions`,
  sorted by name, skipping entries marked `Hidden=true` or `NoDisplay=true`.
- Only the `[Desktop Entry]` group is read; `Name` and `Exec` are used and
  field codes (`%f`, `%U`, …) are stripped from `Exec`.
- If nothing is installed, a single fallback session running `otto` is offered.
- `$OTTO_GREETER_SESSION` overrides discovery entirely with one argv.

### Development backend

- When `GREETD_SOCK` is unset, the greeter uses a self-contained mock backend:
  it accepts the password `otto`, rejects everything else, and never starts a
  session. This makes the greeter runnable as a plain client inside a normal
  Otto session.

## Constraints & Edge Cases

- **The handoff is a process restart, not a transition.** greetd's greeter *is*
  its default session; on `start_session` greetd tears the greeter down and
  execs the user's session on the same VT. Otto in login mode exits and the
  session's Otto starts cold. There is no Wayland mechanism to hand a session
  over, so a visually seamless login→desktop transition would have to be faked:
  leave the CRTC scanning out the last greeter frame rather than disabling it,
  have the incoming compositor modeset with an identical mode, and seed its
  first frame from that image.
- **The session's stdio is the console.** greetd execs the session through
  `/bin/sh -c` with the VT as stdin/stdout/stderr, and the VT is back in text
  mode by then — the greeter's compositor died and took graphics mode with it.
  Anything the session writes before it modesets is painted on the console, so
  a compositor logging at `info` shows up as a terminal flashing between the
  greeter and the desktop. The session entry must therefore point at a wrapper
  that redirects its output (`scripts/otto-session`), not at the bare binary.
  What remains after that is the console itself; taking fbcon off the greeter's
  VT (`fbcon=vc:2-6` on the kernel command line) keeps it from being drawn at
  all.
- **Primary output selection is arrival-ordered.** The first connector to come
  up wins, which need not be the connector marked `primary` in the display
  config. On a laptop with an external monitor already attached at boot, which
  screen shows the greeter is not currently guaranteed.
- **Hotplug during login is ignored, not blanked.** A connector plugged in
  while the greeter is up is left untouched; it is not modeset to black. What
  it displays is whatever the firmware or the previous occupant left there.
- **`start_session` succeeding is not proof the session ran.** greetd
  acknowledges the request before the exec completes, and the only real signal
  of success is this process being killed. Success therefore has to be inferred
  from a timeout, which also means a test daemon that never kills the greeter
  is indistinguishable from a genuinely failed exec.
- **greetd exposes no user list.** Enumerating users, their full names, avatars
  and wallpapers is the greeter's job and requires reading `/etc/passwd` or
  talking to AccountsService.
- **A failed conversation must be cancelled.** greetd will not accept a second
  `create_session` while one is in flight; the greeter sends `cancel_session`
  before returning to the username field.
- **Chatty PAM stacks.** A stack may emit several `info`/`error` messages in a
  row, each needing an acknowledgement. The greeter follows them through with a
  bounded loop so a misbehaving stack cannot spin it forever.

## Rationale

- **greetd rather than PAM in-process.** It keeps Otto out of the privileged
  path entirely: greetd runs as root and owns the VT, the greeter runs as an
  unprivileged `greeter` user, and Otto links no authentication code. It is
  also small, packaged widely, and the de-facto standard in the wlroots
  ecosystem, so Otto interoperates with existing greeters and vice versa.
- **layer-shell rather than xdg-shell fullscreen.** The overlay layer with
  exclusive keyboard interactivity already provides full-output sizing,
  z-order above everything, and input exclusivity, with no changes to Otto's
  window management. It is also what gtkgreet and wlgreet expect, so
  third-party greeters work unmodified.
- **`ext-session-lock-v1` was considered and rejected for login.** It offers
  stronger guarantees — a client crash leaves a blank locked screen rather than
  exposing what is behind — but its semantics are about protecting an existing
  session, and there is nothing behind the greeter to protect. It remains the
  right protocol for screen locking.
- **Chrome hidden rather than removed from the scene.** Keeping the tree shape
  identical avoids scattering login-mode conditionals through layout, damage,
  and plane-assignment code.
- **A mock backend rather than requiring greetd to iterate.** Restarting a
  VT-owning root daemon to adjust padding is a bad development loop.

## Open Questions

- Should the greeter prefer the connector marked `primary` in the display
  config over the first to arrive, and if so, how long should it wait for it?
- Should ignored connectors be modeset to black rather than left alone? That
  costs a modeset and a buffer per connector but avoids showing stale content.
- Where should user enumeration come from — `/etc/passwd`, AccountsService, or
  a small privileged helper (as cosmic-greeter uses) that can also read
  per-user wallpapers?
- Should the greeter honour the compositor's configured theme, accent colour
  and wallpaper, given it runs before any user's config is readable?
