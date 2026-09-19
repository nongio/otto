# Accessibility

Otto supports screen readers through AT-SPI, the standard Linux accessibility
bus. [Orca] is the screen reader this is built and tested against.

[Orca]: https://orca.gnome.org

## What works

- **Orca's keyboard shortcuts.** Otto hands Orca the keys it asks for, including
  its modifier. This is the part that does not work on most Wayland compositors:
  without it Orca starts but every one of its commands does nothing.
- **The desktop itself.** The dock is announced by application name and says
  which applications are running; the app switcher reads out the entry you are
  moving to; workspaces are read with their names. The top bar reads its menus,
  its clock and each tray icon, and a notification is read as it arrives.
- **Otto's own applications** — Settings, Files, the launcher — describe what
  they are showing, down to a file's kind and size and the contents of a
  preview.
- **The keyboard reaches them.** Every control in an Otto application is a Tab
  stop with a ring drawn around it, and can be operated without a pointer. See
  the next section.
- **Pointing at things works.** Each window tells the compositor where it is, so
  Orca's mouse review, a magnifier following the focus, and braille routing all
  land on the control actually under the pointer rather than on whatever happens
  to sit at the same offset from the corner of the screen.
- **It speaks your language.** Descriptions follow the desktop's locale, not
  Otto's development language: an Italian desktop is read in Italian.
- **Other applications** read as they do anywhere else. GTK and Qt applications
  publish themselves to the accessibility bus on their own.

## Using Otto's applications from the keyboard

This works with or without a screen reader — it is ordinary keyboard operation,
and the ring shows where you are.

| Key | What it does |
|---|---|
| `Tab` / `Shift+Tab` | Move between the controls of the focused window, wrapping at the ends and skipping anything disabled |
| `Space` / `Enter` | Operate the control: flip a switch, press a button, open a pop-up, start editing a text field |
| `←` `→` | Move a slider |
| `↑` `↓` | Move within a list, or open a pop-up |
| `Esc` | Close a pop-up without changing anything |

A **pop-up button** opens rather than cycling. Arrowing along a closed pop-up
would change the setting once per value you passed, and every value in one of
these lists commits the moment it is chosen — so `Space`, `Enter`, `↑` or `↓`
opens the list instead, on the value it is currently showing. Inside it the
arrows move a highlight, `Home` and `End` go to the ends, `Enter` chooses, and
`Esc` closes it with the setting untouched.

## Setting it up

Install the accessibility bus and a screen reader:

```sh
sudo pacman -S at-spi2-core orca      # Arch
sudo dnf install at-spi2-core orca    # Fedora
sudo apt install at-spi2-core orca    # Debian/Ubuntu
```

Then start Orca from a terminal, the launcher, or your autostart:

```sh
orca --replace
```

Nothing else has to be configured — Otto publishes the accessibility interfaces
whenever it is running a real session.

Orca still needs Xwayland for some of its own machinery, so leave Otto's
Xwayland support enabled.

## Turning it off

Add this to your config to stop Otto exposing accessibility altogether:

```toml
[accessibility]
enabled = false
```

The only reason to is if something else on your system is trying to own the same
interfaces.

## Known limitations

- The **lock screen and the greeter are not accessible**. Every key there
  belongs to the locker, screen readers included.
- **Text fields are read as a whole**, not character by character: Otto's own
  applications report what a field contains, but not caret-by-caret review.
- There is **no built-in magnifier**, and no screen curtain.
- In **Files**, the listing and the previews are described but the sidebar, the
  path bar and the toolbar are not yet: you can read a folder, but moving to
  another one needs the pointer.
- In **Settings**, the arrangement diagram at the top of the Displays pane —
  the one you drag screens around in — is not described, so choosing *which*
  display the settings below apply to needs the pointer. Every other control in
  the pane is reachable. The shortcut lines in the Keyboard pane are not
  reachable yet either.
- A **nested Otto** (`otto --winit`, for development) does not publish its own
  desktop — the session hosting it owns the accessibility bus and the key
  grabs. Applications running inside it still describe themselves normally.

## If a screen reader is silent

0. Otto and its applications only publish themselves once something declares
   itself an assistive technology. Orca does that; if you are testing without
   one, nothing will be on the bus until you set it yourself:
   `busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true`

1. Check the accessibility bus is there:
   `busctl --user status org.a11y.Bus`
2. Check Otto is offering key grabs:
   `busctl --user introspect org.freedesktop.a11y.Manager /org/freedesktop/a11y/Manager`
   If the name is unowned, either `accessibility.enabled` is false or another
   process claimed it first — check Otto's log for "Could not own the a11y
   manager name".
3. Check the application itself is on the bus with `accerciser`, which lists
   every accessible application. If it is not there, the problem is that
   application's, not Otto's.
