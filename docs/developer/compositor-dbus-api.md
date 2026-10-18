# `org.otto.Compositor`: small requests to the compositor

The D-Bus interface for requests about the compositor itself: a health check,
bringing an application forward, and the details a bug report needs.

> **Status**: implemented on both sides. The compositor serves it
> (`src/screenshare/dbus_service.rs`, answered on the main loop through
> `CompositorCommand`), and the client proxy is
> `otto_dbus::compositor::CompositorProxy`. The watchdog calls `Ping`,
> notification clicks call `FocusApp`, and `otto-doctor`
> (`components/otto-doctor/`) calls `DebugInfo`.

Bus name `org.otto.Compositor`, object path `/org/otto/Compositor`, interface
`org.otto.Compositor`. The compositor owns the name on the same connection as
`org.otto.ScreenCast`.

## Methods

| Method | Signature | Answer |
| --- | --- | --- |
| `Ping` | `() → s` | `"pong"` while the D-Bus thread is alive |
| `FocusApp` | `(s) → b` | raises the window of the given app id; `true` once the request is queued, not whether a window was found |
| `DebugInfo` | `() → s` | a JSON object: version, backend, renderer, configuration files and outputs |

`Ping` answers on the D-Bus thread, so it proves the bus side is up, not that
the main loop is. `DebugInfo` goes through the main loop, so a frozen
compositor does not answer it; a caller should set a short timeout
(`otto-doctor` waits three seconds).

### `DebugInfo() → s`

A JSON string, like the `org.otto.Shell1` getters, so it can be read with
`jq` and grow fields without a signature change. A client ignores fields it
does not know and treats a missing field as unknown.

```json
{
  "version": "1.8.0",
  "git_commit": null,
  "backend": "udev",
  "renderer": {
    "api": "gl",
    "vendor": "Intel",
    "renderer": "Mesa Intel(R) Xe Graphics (TGL GT2)",
    "version": "OpenGL ES 3.2 Mesa 25.1.4"
  },
  "vulkan_fallback": null,
  "config_files": ["/etc/otto/config.toml", "/home/me/.config/otto/config.toml"],
  "outputs": [
    {
      "name": "eDP-1",
      "make": "BOE",
      "model": "NE135A1M-NY1",
      "width": 2880,
      "height": 1920,
      "refresh_mhz": 120000,
      "scale": 2.0,
      "x": 0,
      "y": 0,
      "virtual": false
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `version` | The compositor's crate version |
| `git_commit` | The commit, when the build set `OTTO_GIT_COMMIT` (`PKGBUILD-git` does); otherwise `null` |
| `backend` | `udev`, `winit`, `x11` or `headless` |
| `renderer` | The first renderer the session created, or `null` before one exists. `api` is `gl` or `vulkan`. For GL, `vendor`, `renderer` and `version` are `GL_VENDOR`, `GL_RENDERER` and `GL_VERSION`; for Vulkan they are the driver name, the device name, and the API version with the driver info |
| `vulkan_fallback` | Why the session draws with OpenGL although Vulkan was asked for, or `null` |
| `config_files` | The configuration files on disk, lowest priority first, the layers the compositor merges |
| `outputs` | Every output, sorted by name. `width` and `height` are the current mode in physical pixels, `refresh_mhz` its refresh in millihertz, `scale` the fractional scale, `x` and `y` the position in the layout in logical points, `virtual` whether it is a PipeWire-backed output |

The answer holds paths with the user's home folder in them. `otto-doctor`
redacts them before showing or writing anything; another caller that passes
the answer on should do the same.

## See also

- [otto-dbus](otto-dbus.md): the client proxies.
- [Screen sharing](screenshare.md): the `CompositorCommand` bridge the methods
  are answered through.
- [Reporting a bug with otto-doctor](../user/bug-reports.md).
