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

## Gaps

- **Changing the theme still needs a compositor restart.** The general
  `org.otto.Settings` API does emit a `Changed` signal, but the portal backend
  does not yet translate that into the portal's own `SettingChanged` signal, so
  running applications will not react.
- **Only `color-scheme` is exposed.** The `org.freedesktop.appearance`
  namespace also defines `accent-color` and `contrast`; neither is served yet.

## Spec references

- [XDG Desktop Portal — Settings](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html)
- [`org.freedesktop.appearance`](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.appearance.xml)
