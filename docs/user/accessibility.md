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
  moving to; workspaces are read with their names.
- **Otto's own applications** — Settings, Files, the launcher — describe
  themselves, and can be moved through with Tab and Shift+Tab.
- **Other applications** read as they do anywhere else. GTK and Qt applications
  publish themselves to the accessibility bus on their own.

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
- A **nested Otto** (`otto --winit`, for development) exposes none of this — the
  session hosting it owns the accessibility bus.

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
