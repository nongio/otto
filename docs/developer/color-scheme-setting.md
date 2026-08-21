# Color Scheme

How Otto decides whether the desktop is light or dark, and how applications
find out.

## The setting

One option in `otto_config.toml`:

```toml
theme_scheme = "Light"   # or "Dark"
```

It controls two separate things:

1. **Otto's own UI** — dock, app switcher, window decorations, menus. Read
   from `src/config/mod.rs` and consumed by `src/theme/` and the various view
   modules.
2. **Client applications** — over D-Bus, through the XDG Settings portal.

## Why applications need a portal for this

An application cannot read `otto_config.toml` — it may be sandboxed, and it
should not have to know which compositor it is running under. The desktop-wide
answer is the **Settings portal**
(`org.freedesktop.portal.Settings`), where a toolkit asks for the
`color-scheme` key in the `org.freedesktop.appearance` namespace and gets back
a number:

| Value | Meaning |
|-------|---------|
| `0` | no preference |
| `1` | prefer dark |
| `2` | prefer light |

GTK4 and Qt6 apps query this automatically. So does Firefox, and Chromium via
its portal integration.

## The chain

```
App  →  org.freedesktop.portal.Settings          (xdg-desktop-portal)
     →  org.freedesktop.impl.portal.Settings     (xdg-desktop-portal-otto)
     →  org.otto.Settings                        (the compositor)
     →  theme_scheme in the config
```

**Compositor service** — `src/settings_service.rs` registers `org.otto.Settings`
at `/org/otto/Settings`, exposing `GetColorScheme()` and `GetIconTheme()`
alongside the general settings API. It is started during the compositor's
D-Bus service initialization.

**Portal backend** — `components/xdg-desktop-portal-otto/src/portal/settings.rs`
implements `org.freedesktop.impl.portal.Settings`, handling `ReadAll()` and
`Read()` per spec, including namespace glob filtering. It reaches the
compositor through the D-Bus proxy in
`components/xdg-desktop-portal-otto/src/otto_client/settings.rs`.

The backend is registered as `org.freedesktop.impl.portal.desktop.otto` and
declared in `otto.portal` alongside its other interfaces.

`GetColorScheme` and `GetIconTheme` are a **frozen contract** — the portal
backend depends on them and they must keep working even as the wider settings
API grows. See [settings-dbus-api.md](settings-dbus-api.md).

## Verifying

```sh
# Ask the portal what the desktop's preference is
gdbus call --session \
  --dest org.freedesktop.portal.Desktop \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.Settings.Read \
  org.freedesktop.appearance color-scheme

# Ask the compositor directly, bypassing the portal
busctl --user call org.otto.Settings /org/otto/Settings \
  org.otto.Settings GetColorScheme
```

If the two disagree, the portal backend is stale — a backend from an earlier
login can still own the bus name. See the note in
[screenshare.md](screenshare.md#testing-a-portal-build).

## Changes reach running applications

The portal backend subscribes to `org.otto.Settings`' `Changed` signal and
re-emits the settings it serves as the portal's own `SettingChanged`
(`portal::spawn_change_relay`). Otto's identifiers are not the portal's keys, so
only the ones with a counterpart are forwarded:

| `org.otto.Settings` | `org.freedesktop.appearance` |
| ------------------- | ---------------------------- |
| `theme_scheme`      | `color-scheme`               |
| `accent_color`      | `accent-color`               |
| `icon_theme`        | `icon-theme`                 |

otto-kit apps pick these up through the watchers in `color_scheme.rs`,
`accent.rs` and `icon_theme.rs`, started by `AppRunner`.

## Accent colour

`accent-color` is served as `(ddd)` — sRGB in `0.0..=1.0`, no alpha, as the
spec requires. The compositor stores the accent by name (see
[theming](../user/theming.md)); `GetAccentColor` on `org.otto.Settings` does the
palette lookup and the conversion, so the portal passes the triple straight
through.

Inside the compositor the accent takes a shorter path. Otto draws its window
decorations with otto-kit's `WindowDecoration`, which tints the traffic-light
controls from `accent::current_accent()` — the global the portal watcher fills
in a client. Otto cannot use that watcher: it is what answers the portal call,
so it would be querying itself. `theme::publish_accent()` writes the resolved
colour straight into the same store with `accent::set_accent`, at startup
(`Otto::init`) and again whenever `accent_color` is applied live, and
`theme::accent_color()` reads it back out. One store on both sides, so a
compositor-drawn titlebar and an otto-kit client's own titlebar cannot disagree.

Because the accent is read inside render functions rather than held in view
state, applying it re-renders rather than updates: `rerender_accent_colored_views`
rebuilds the workspace selector, the window selectors, and every window's
decoration layer.

## Gaps

- **`contrast` is not served.** The `org.freedesktop.appearance` namespace
  defines it; Otto has no such setting yet.

## Spec references

- [XDG Desktop Portal — Settings](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html)
- [`org.freedesktop.appearance`](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.appearance.xml)
