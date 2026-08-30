# Accessibility

**Status:** draft
**Related specs:** [otto-kit-window-focus](otto-kit-window-focus.md), [window-focus-navigation](window-focus-navigation.md), [settings-app](settings-app.md)

## Summary

What Otto and otto-kit offer assistive technologies: a screen reader running in
an Otto session receives the keys it needs, is told what the shell is showing,
and can read and operate otto-kit applications.

## Goals

- A screen reader's own keybindings work. Under Wayland a client cannot read the
  keyboard, so the compositor must hand keys over on request.
- Keys an assistive technology has taken never reach anything else — no
  shortcut, no focused client, no toggle state change — and its releases are
  swallowed with its presses, so no client ever sees half a keystroke.
- The shell — dock, app switcher, workspaces — is announced, by application
  name, and can be operated from an assistive technology.
- An otto-kit application can describe its interface, and everything it
  describes can be reached from the keyboard alone.
- A session with no assistive technology running pays effectively nothing: no
  accessible tree is ever built.
- What is announced and what is drawn come from the same state, so they cannot
  drift apart.

## Non-Goals

- Shipping a screen reader, a magnifier or an on-screen keyboard. Otto provides
  the interfaces; Orca and friends are separate software.
- Accessibility of the lock screen, of a greeter, or of any surface holding an
  exclusive keyboard grab. Those paths deliberately hand every key to the
  surface that owns them.
- Character-level text navigation inside kit text fields. A field reports its
  contents and its role; caret-by-caret review is a later addition.
- A nested Otto (`--winit`, `--x11`) exposing anything. It is a window inside
  another session, which owns the accessibility interfaces.

## Behavior

### Key monitoring and grabs

Otto owns the well-known bus name `org.freedesktop.a11y.Manager` on the session
bus, serving `org.freedesktop.a11y.KeyboardMonitor` at
`/org/freedesktop/a11y/Manager`.

- `WatchKeyboard` / `UnwatchKeyboard` — while watching, the client is sent every
  key through the `KeyEvent` signal, and the session handles the key as usual.
- `GrabKeyboard` / `UngrabKeyboard` — while grabbing, the client is sent every
  key *and* the session does not handle it.
- `SetKeyGrabs(modifiers, keystrokes)` — replaces that client's grabs. Each
  entry of `modifiers` is an XKB keysym that is grabbed, along with anything
  pressed while it is held. Each entry of `keystrokes` is a keysym and the XKB
  modifier mask it must be pressed under.
- `KeyEvent(released, state, keysym, unichar, keycode)` is emitted to each
  client that watches or grabs the key, and to no one else.

The same object serves `org.freedesktop.a11y.PointerLocator`, which is not
optional: at-spi2-core builds one input device from both interfaces, so a
manager offering only the keyboard half is one Orca cannot construct a device
from at all — no keys are ever grabbed, and Orca dies on the reply it did not
get.

- `QueryPointer` answers with `(a{sv} app_data, d rel_x, d rel_y)` — where the
  pointer is, in logical pixels across the whole layout. Otto has nothing to put
  in `app_data` and sends it empty.
- `PointerPositionChanged` carries **no arguments**: it says only that the
  pointer moved, and a listener that cares asks again. It is emitted only after
  something has asked at least once, and no more often than every 50 ms.

Required behaviour beyond the plain reading:

- A grabbed key's **release** is taken whenever its press was, and only then.
- A **lone press of a grabbed modifier** is taken; a second press of the same
  modifier within the key repeat delay is handled normally instead, as is its
  release. This is what lets a screen reader own a modifier without taking the
  key away from the user entirely.
- Everything a client asked for is **dropped when it disconnects**. A screen
  reader that crashes must not leave the session with dead keys.
- VT switching, the power button and the emergency quit/lock shortcuts are
  never grabbable.

### The shell

The compositor publishes its own chrome as one accessible application, holding:

- the **dock**, as a toolbar of buttons — each labelled with the application's
  desktop name, described as running or not, and clickable, where clicking
  focuses that application exactly as a pointer click would. Whether an
  application is described as running is whatever the dock is currently
  drawing: when an application starts or exits, the description changes at the
  moment the dock's own row of icons does, not some fraction of a second later
  and never permanently behind it;
- the **app switcher**, present only while it is open, as a list whose selected
  entry is the accessible focus;
- the **workspaces**, as a list with the current one selected, named as the UI
  names them;
- the **all-windows overview**, present only while it is open, as a list of the
  windows on the current workspace, titled as their windows are titled and with
  the active one selected. Choosing one leaves the overview and focuses that
  window, exactly as clicking its thumbnail does.

Every label the shell and its applications speak comes out of the localisation
catalogue, so a screen reader talks in the same language the desktop is drawn
in — a spoken label that stays English beside a translated interface names
something the user cannot see.

Everything the shell places on screen carries its bounds, in logical pixels
across the whole layout — the same space the pointer is reported in. Without
them an assistive technology can read the shell but cannot find any of it by
pointing, which is what mouse review does.

The shell reports itself as the focused window only while the app switcher or
the overview is open. At any other time the user is working in an application, and the shell
must not take the screen reader's attention away from it.

### otto-kit applications

An application declares which of its surfaces are accessible. For each of those,
while an assistive technology is attached, it is asked to describe the surface
and the description is published.

- Node identity is the same identity the keyboard uses: what Tab moves between
  and what is described are one list, not two. Every control a surface draws is
  described, including one that changes nothing outside the application;
  whether the keyboard stops on it is a separate question, and a control that
  only displays a value is described without being stopped on.
- A row of push buttons is one keyboard stop and one described control per
  button, each carrying the bounds of its own button rather than of the row.
- The focused control reported is whatever the surface's focus ring says, always.
- Tab and Shift+Tab move the focus between the controls of the focused window,
  wrapping at both ends, skipping disabled controls, and leaving them in their
  place in the order. With nothing focused, Tab starts at the first control and
  Shift+Tab at the last.
- A list is **one** keyboard stop, not one per row: Tab enters it and the arrow
  keys move within it. That is what the list role tells an assistive technology
  to expect, and a window whose keyboard stops and whose nodes differ this way
  must report which node the focus is really on — a list that reports only
  itself tells a screen reader nothing about where the user is.
- A window that declares no focusable controls does not consume Tab at all.
- Focus survives a rebuild that keeps the control, and is dropped when the
  control goes away.
- A password field reports that it is one and never reports its contents.
- An assistive technology may ask for an action — click, focus, set value — and
  the application must act on it as though the user had done it by hand.

### Configuration and gating

`accessibility.enabled` (default true) turns the whole of the above off. A
nested Otto never exposes any of it regardless.

## Constraints & Edge Cases

- The grab decision is made synchronously while the key is being processed: a
  round trip to another process or thread in that path would stall every
  keystroke in the session. Where the pointer is is answered the same way: the
  position is published into an atomic as the pointer moves, never fetched from
  the input path.
- Both interfaces must match at-spi2-core's expectations exactly, down to
  argument types. A wrong reply shape is not a degraded feature — libatspi
  parses it with a fixed format string, and Orca dies on the mismatch.
- `KeyEvent` signals must arrive in the order the keys were pressed.
- Two keyboards can press the same key: the second press of a key already held
  by a client is taken as well, and only the first release is.
- If the well-known name is already owned — a stale compositor, a session that
  already has one — Otto logs it and carries on. Only accessibility is affected.
- An assistive technology may attach and detach repeatedly; each attach must be
  answerable with a complete tree.
- A pop-up control opens from the keyboard onto the value it is showing, and
  the open menu then owns every key: the arrows walk it (scrolling a capped
  list to keep the highlight in view), Enter or Space chooses, Escape closes
  with nothing chosen. The open list is described as well as the button, so a
  screen reader is not left reading a control it cannot see past.
- The state a description is built from and the state the shell draws from can
  be resolved at different times. Where they are, the description must be
  rebuilt when the drawing state settles, or it reports the previous state
  indefinitely rather than briefly.

## Rationale

- **AccessKit for the object model, hand-written D-Bus for the keyboard
  monitor.** The AT-SPI object model is large and its details are where screen
  readers are unforgiving; the keyboard monitor is one small interface that has
  to be wired into the compositor's input path and could not be delegated.
- **Trees are declared, not derived.** Kit widgets are draw calls inside coarse
  layers — there is no object graph to walk. Declaring them in the pass that
  draws them keeps the two in step.
- **Nothing is built unless something is listening.** The alternative is paying
  for accessibility on every frame of every session that never uses it.
- **The dock's running state is taken from the dock, not from the window
  state.** Which applications are running is resolved by the dock on its own
  schedule, up to half a second after the windows change. A tree built only
  from the window state was always one dock tick behind, and — because nothing
  rebuilt it when that tick landed — went on calling a freshly started
  application "not running" for as long as nothing else changed.
- **The shell only claims focus while the switcher is open.** A dock that
  claimed to be the focused window would make a screen reader read the desktop
  instead of the user's work.

## Open Questions

- Should the lock screen be accessible? It is deliberately excluded now, but a
  user who cannot see the screen also has to be able to unlock it.
- Caret-level text review in kit fields — worth the AT-SPI `Text` interface, or
  is the value enough?
- Keys synthesised through `zwp_virtual_keyboard` are invisible to assistive
  technologies: they never pass the compositor's input filter, so an on-screen
  keyboard's output is not something a screen reader can echo.
