# Lock Screen

**Status:** draft
**Related specs:** [login-mode](./login-mode.md), [multi-output](./multi-output.md),
[lid-power](./lid-power.md)

## Summary

Otto can lock the running session: the desktop is hidden behind an opaque
surface on every output, input goes only to the locking client, and the session
is restored unchanged once the user authenticates. The protocol is
[`ext-session-lock-v1`], which is designed for exactly this and gives the
guarantee layer-shell cannot — if the locker crashes, the screen stays blank
rather than revealing what is behind it.

Otto ships `otto-lock` as its locker. It draws the same panel as the greeter
(`components/otto-auth-ui`) and authenticates against PAM as the session's own
user. Any other `ext-session-lock-v1` client works too.

[`ext-session-lock-v1`]: https://wayland.app/protocols/ext-session-lock-v1

## Goals

- A locked session shows no window, dock, panel, notification or cursor content
  from the session on any output, and none of it is recoverable by input.
- The session outlives the lock: windows, workspaces, focus and running clients
  are exactly as they were once it is unlocked.
- The screen is blank on every output *before* the client is told the session is
  locked, so nothing is visible in the gap between the request and the first
  lock frame.
- A locker that crashes or exits without unlocking leaves the session locked and
  the outputs blank.
- Outputs that appear, disappear or change mode while locked are covered too.
- `Ctrl+Alt+F<n>` still switches VT while locked.
- The locker is an ordinary Wayland client, replaceable via configuration, and
  Otto itself performs no authentication.

## Non-Goals

- **Logging in.** A greeter authenticates a user who has no session yet and is
  bound to greetd's process model; see [login-mode](./login-mode.md).
- **Idle detection.** What decides *when* to lock — an idle timer, the lid, a
  suspend hook, a keybinding — is separate from the lock itself. This spec
  covers only the mechanism and the explicit triggers below.
- **Owning authentication.** The locker calls PAM; the compositor does not.
- **Hiding the session from a privileged attacker.** A lock screen protects
  against someone at the keyboard, not against root or physical memory access.

## Behavior

### Locking

- A locker binds `ext_session_lock_manager_v1` and requests a lock. Otto accepts
  it, hides the whole session — windows, dock, app switcher, expose, workspace
  selector, layer-shell panels, drag surfaces and popups — behind an opaque
  black surface on every output, and cancels any interactive grab in progress.
- Otto sends the `locked` event only after a frame with the session hidden has
  been presented on every output it drives. If it cannot reach that state the
  lock request is refused with `finished` and the session is untouched.
- "Every output it drives" means every output someone can see. A virtual
  (PipeWire) output composites only while something is consuming it, so one
  with no stream attached never presents and is not waited for — its blank goes
  up with the rest, and a stream that starts later finds it there.
- A confirmation that has not arrived within five seconds is given up on: the
  blank comes down, the session comes back, and the request is refused. The
  state being avoided is a session hidden behind a lock no locker can unlock,
  because the client is waiting for `locked` before it authenticates.
- Only one lock may be active. A second lock request while locked is refused.
- Locking is idempotent from the user's side: triggering a lock while already
  locked does nothing.
- The blank comes down from the top of the screen like a shade and springs into
  place. The session is visible under it while it falls but is already
  unreachable: input is cut off when the lock is requested, and no output counts
  as blanked before the shade has landed.
- Locking plays the sound theme's `desktop-screen-lock` event, through the same
  XDG sound-theme lookup as every other UI sound and subject to the same
  `audio.sound_enabled`. Themes that ship no such event — `freedesktop` is one —
  lock silently.

### Lock surfaces

- The locker creates one surface per `wl_output`. Otto configures each with the
  output's size in logical points and the output's scale, and the surface must
  commit a buffer of the configured size before it is shown.
- Lock surfaces are drawn above everything, including fullscreen windows and
  overlay-layer panels, and are never subject to workspace scrolling,
  animations, or expose.
- Until a lock surface commits its first buffer, its output shows the blank.
  An output whose lock surface is destroyed reverts to the blank.
- Keyboard focus goes to the lock surface of the output that has pointer focus,
  and follows the pointer between outputs. Pointer and touch events go only to
  lock surfaces.
- The cursor is drawn from the locker's own cursor surface, or Otto's default if
  it sets none.

### While locked

- Compositor keybindings are inert with two exceptions: VT switching, and
  volume/brightness/media keys, which continue to work.
- Session clients receive no input and no keyboard focus; their frame callbacks
  are throttled as they are for occluded windows, and their surfaces are not
  rendered.
- Screencopy, screenshare and any other capture of a locked output is refused
  for the duration.
- An output hotplugged while locked is blanked immediately and offered to the
  locker, which may create a surface for it. Removing an output destroys its
  lock surface without ending the lock.
- Suspending and resuming, or switching VT away and back, leaves the session
  locked. Coming back from another VT redraws in full: the primary plane has no
  content and nothing in the scene changed while Otto was away, so damage
  tracking alone would present an empty frame over an empty plane.

### Unlocking

- The locker authenticates the user itself and then requests unlock. Otto
  restores the session's rendering and input at once, gives keyboard focus back
  to the window that had it when the lock began, if it still exists, and takes
  the blank back up off the top of the screen.
- The shade rises with no bounce — a rebound would drop it back over a session
  the user already has back — and the lock surfaces are destroyed when it is
  off-screen. The locker exits as soon as it has asked for the session back, so
  its panel rides up on scene layers that outlive the client rather than
  vanishing on the first frame.
- The full-scene composite and the ban on direct scanout hold until the shade is
  gone, not until the unlock request arrives: the plane path has nothing to draw
  it with, and a promoted window would scan out straight through it.
- A locker that exits, crashes or is killed **without** requesting unlock leaves
  the session locked and blank. There is no compositor-side escape hatch; the
  ways out are authenticating through a new locker (which Otto respawns) or
  switching VT.
- If the locker dies while locked, Otto restarts it, rate-limited, so a crash is
  recoverable without a VT switch.

### Triggers

- The `lock` action requests a lock by launching the configured locker. It is
  bound to `Ctrl+Alt+Escape` by default and is rebindable like any other
  shortcut.
- `lock.locker_command` and `lock.locker_args` name that client, defaulting to
  `otto-lock`. `$OTTO_LOCKER_COMMAND` overrides both as a whitespace-separated
  argv, for testing uninstalled builds.
- Launching the locker is the only trigger Otto implements; anything else that
  wants to lock (an idle daemon, a suspend hook) runs the same command.

### The locker

- `otto-lock` presents `components/otto-auth-ui`'s panel: the same frosted card,
  avatar, field and Touch ID mark the greeter shows, with no session picker —
  a lock screen passes no session name.
- Appearance comes from Otto's config, including the user's, so the lock screen
  matches the running session.
- Authentication is a PAM conversation on a dedicated service (`otto-lock`),
  run as the session user. Prompts, info and error messages map onto the panel
  exactly as greetd's `auth_message`s do in the greeter, so a fingerprint reader
  configured through `pam_fprintd` works with no locker-specific code.
- The service file is `components/otto-lock/otto-lock.pam`, installed as
  `/etc/pam.d/otto-lock`. Without it PAM falls through to `other`, which denies
  everything, so a missing file would lock the user out of their own session;
  the locker notices and falls back to `system-auth`, then `login`, saying so.
  `$OTTO_LOCK_PAM_SERVICE` names a different service, for exercising the lock
  against a stack whose answer is known.
- A refused attempt is followed by another, so there is always a field — or a
  reader — to try again with. The delay between them is PAM's; the locker
  imposes only a floor, so a stack that cannot run at all (no service file, a
  broken module) fails fast without spinning.
- The clock keeps time. A session can be locked for hours, and the panel's
  clock draws from a closure the scene engine otherwise records once and
  replays — so the locker damages it when the minute turns.
- PAM's conversation is blocking, so it runs off the main thread; the panel must
  stay animating while a reader waits for a finger.
- A failed attempt returns to the field with the error shown, and repeated
  failures are rate-limited by PAM's own stack rather than by the locker.
- On success the locker requests unlock and only then exits, so the session is
  never left with a destroyed locker and no unlock.

## Constraints & Edge Cases

- **The blank must be real, not "no content".** Damage tracking that skips an
  unchanged output would leave the previous frame — the unlocked desktop — on
  screen. The blank is a surface that is drawn, and every output is damaged in
  full when the lock begins.
- **`locked` is a promise.** Once sent, the client is entitled to assume nothing
  of the session is visible. Sending it before the blank has actually reached
  the screen on every output is the classic lock-screen race and is what the
  present-then-confirm ordering above exists to prevent.
- **Direct scanout must be dropped at lock time.** A promoted client buffer on a
  hardware plane is scanned out independently of the composited frame; a plane
  left active would keep showing the window under the blank. All promotions are
  released when the lock begins, and only lock surfaces may be promoted while
  locked.
- **The plane decomposition has no lock plane, so locking leaves it.** On the
  DRM backend the scene is not composited in one pass: it is split into a fixed
  set of subtrees — background, windows, expose, overlay, switcher, dock — each
  rendered to its own KMS plane. The lock plane is in none of them, so a locked
  session that stayed on that path would draw the blank and the locker nowhere
  while the desktop's planes kept scanning out their last buffers — a lock
  screen showing the whole desktop. Locking therefore forces the full-scene
  composite (the path the minimize genie already uses), whose subtree is the
  output layer and so includes the lock plane. Nested backends composite in one
  pass and never showed this, which is why it has to be said here.
- **A stream is not a screen, and is hidden the same way.** Screencopy is
  refused while locked, but a virtual (PipeWire) output composites its own
  plane stack for a consumer that is still attached — and that stack was built
  from the same subtrees, none of which is the lock plane. While locked a
  virtual output composites the lock plane and nothing else, so no subtree that
  could hold a window is even consulted.
- **Rendering must not be skipped while locked.** A lock surface that animates
  (the Touch ID mark) needs frame callbacks even though no session client does.
- **A lock surface's commit must request a redraw like any other surface's.**
  Nothing else is redrawing while locked — the session is hidden and its
  clients are throttled — so the locker's own commits are the only thing left
  asking for frames. A commit path that updates the scene and returns without
  requesting one leaves the panel frozen on whichever frame some unrelated
  redraw happened to carry, and, because frame callbacks are sent from the
  presentation path, the client never learns its frame arrived: it falls back
  to painting on its own timeout, an order of magnitude slower, into a screen
  nobody is drawing. The tell is that moving the pointer — which requests a
  redraw for its own reasons — makes the lock screen come alive.
- **The lock outlives the client.** Compositor state, not the client's presence,
  is what "locked" means. Every path that tears down a client — crash, kill,
  protocol error — must leave the lock intact.
- **The unlock request must be flushed before the locker exits.** A locker
  unlocks in order to leave, and a request still sitting in its client-side
  buffer when the connection closes is one the compositor never sees. What it
  sees instead is a locker that died while locked — which by design leaves the
  session locked, the screen blank, and a respawned locker as the only way
  back. The request is therefore pushed out where it is made, not left for the
  event loop's next flush.
- **A locker started by hand must not be able to unlock what it did not lock.**
  The protocol enforces this: unlock is a request on the `ext_session_lock_v1`
  object, and Otto ignores it from any other lock.
- **Multi-output timing.** Lockers create their surfaces one output at a time.
  Otto must not wait for all of them before sending `locked`; the blank is what
  the promise is about, not the locker's content.
- **XWayland.** X11 clients can neither see nor grab input while locked, since
  focus and input routing are compositor-side, but an X client that already
  holds a pointer grab must have it broken when the lock begins.
- **PAM in the client, not the compositor.** `otto-lock` runs as the user, so it
  can authenticate that user without privilege — but a PAM stack that needs a
  privileged helper (fingerprint, smartcard) reaches it through the usual
  daemons, not through Otto.

## Rationale

- **`ext-session-lock-v1` rather than an overlay layer surface.** The crash
  guarantee is the whole point: with layer-shell, a locker that dies takes the
  lock with it and exposes the session. The protocol also makes the
  present-before-`locked` ordering explicit, and every serious locker in the
  ecosystem now speaks it.
- **A separate client rather than locking inside the compositor.** A crash in
  the panel, the Lottie renderer or the PAM stack must not take the compositor
  down with the session inside it. It also keeps PAM out of Otto, as login mode
  keeps it out.
- **The panel is shared with the greeter.** `otto-auth-ui` was written for two
  clients; a lock screen that looks like the login screen is the point, and the
  session picker is the only element that differs.
- **Otto respawns a dead locker.** The protocol's guarantee is that the session
  stays hidden, not that the user stays locked out. Without a respawn the only
  recovery is a VT switch, which a laptop lid or a tablet may not offer.
- **Locking is triggered by launching the locker, not by an internal action.**
  It keeps a single path into the locked state, and lets idle daemons, suspend
  hooks and the keybinding all use the same mechanism.

## Open Questions

- Should locking be forced before suspend, and if so, does Otto do it or does a
  systemd sleep hook?
- Should the blank be black, the wallpaper, or a blurred capture of the desktop?
  A blurred capture is what the panel's frost is designed against, but it is
  also a partial disclosure of what was on screen.
- Should the panel offer suspend / restart / shut down from a locked screen, as
  it does from the greeter? It does today, on the grounds that the session
  picker is the only element meant to differ — but a "Restart" button on a lock
  screen is a data-loss button for a session that is still running.
- Should Otto refuse to lock when no locker is installed, or fall back to a
  built-in blank with no way in except a VT switch?
