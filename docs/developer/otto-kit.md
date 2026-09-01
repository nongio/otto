# otto-kit

`components/otto-kit/` is the crate every Otto application is built on, and the
one the compositor draws its own chrome with. It is a Wayland client runtime,
a Skia drawing layer, a widget set and a design system in one crate.

It has **two consumers, and they use it in opposite ways.**

- **Client apps** — otto-bar, otto-islands, otto-launcher, otto-settings,
  otto-files, otto-quickview, otto-lock, otto-greeter, otto-auth-ui. They take
  the runtime (`AppRunner`, `AppContext`, surfaces, protocols) and then either
  draw their own Skia or assemble components.
- **The compositor** — `src/` has no `AppRunner`, no `AppContext` and no
  `wl_surface`. It calls the drawing half only: `Titlebar` and `WindowControl`
  for server-side decorations, `ContextMenuRenderer` for the dock and top-bar
  menus, and `theme`, `typography` and `icons` everywhere.

That split is the crate's main design constraint: **anything in `components/`
must be drawable from a bare `&Canvas`**, with no connection, no event loop and
no ownership of a surface. A component that needs to be told about a click
takes a state struct the caller owns and hands events to; it never subscribes
to anything itself.

## Layout

```
components/otto-kit/src/
├── app_runner/       The event loop: App trait, AppRunner, AppContext
├── surfaces/         One type per Wayland surface role
├── rendering/        EGL + Skia surface, and the lay-rs renderer
├── components/       The widget set
├── theme.rs          Palette, and ColorScheme
├── typography.rs     Named text styles
├── icons.rs          Icon lookup
├── icon_theme.rs     freedesktop icon theme resolution
├── accent.rs         Accent colour, from the settings portal
├── color_scheme.rs   Light/dark, from the settings portal or OTTO_COLOR_SCHEME
├── protocols/        Otto's own Wayland protocols, generated
├── desktop_entry.rs  .desktop parsing
├── filetype/         MIME lookup by glob and content
├── preview/          File thumbnails
├── clipboard.rs      Selection ownership and paste
├── dnd.rs            Drag and drop
├── input.rs          Keyboard/pointer helpers
├── lottie.rs         Lottie animation playback
├── sound.rs          Feedback sounds
└── testing.rs        SHM-only test client (feature `testing`)
```

## The application model

An app implements `App` and hands it to `AppRunner`:

```rust
use otto_kit::{App, AppContext, AppRunner};

struct MyApp { /* … */ }

impl App for MyApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        // Connected; globals bound. Create surfaces here.
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(MyApp::new()).run()
}
```

`AppRunner` owns the connection, the event queue and the seat, and drives the
`App` through a lifecycle: `on_start`, `on_app_ready`, then the callbacks for
what happens next — `on_configure` for a toplevel, `on_configure_layer` for a
layer surface, `on_configure_lock_surface` / `on_session_locked` /
`on_session_lock_finished` for a locker, `on_keyboard_event`, pointer and
gesture callbacks, and `on_close`. Everything except `on_app_ready` has a
default no-op, so an app implements only the roles it plays.

`AppContext` is the handle to everything the runtime bound. Most of it is
**associated functions on globals, not methods** — `AppContext::outputs()`,
`AppContext::fractional_scale()`, `AppContext::wlr_layer_shell()`,
`AppContext::current_theme()`. This is deliberate: a draw closure or a
component deep in a view tree needs the scale or the theme without being handed
a context reference through every layer above it. The `*_ref` methods exist for
the paths that do have a context.

## Surfaces

One type per role, all implementing `BaseWaylandSurface`:

| Type | Role |
|------|------|
| `ToplevelSurface` | An ordinary application window (`xdg_toplevel`) |
| `LayerShellSurface` | A panel or overlay (`zwlr_layer_shell_v1`) |
| `PopupSurface` | A menu or dropdown, anchored to a parent (`xdg_popup`) |
| `SubsurfaceSurface` | A child surface positioned in its parent's coordinates |
| `SessionLockSurface` | A locker's per-output surface (`ext-session-lock-v1`) |
| `DockItem` | A surface the dock hosts, via Otto's `otto-dock-v1` |

`Window` (`components/window/`) sits above `ToplevelSurface` and adds a title
bar, resize affordances and a content area. Note who actually uses it: the
settings app and the examples. Every other app is layer-shell, subsurface or
session-lock, and draws its own frame — which is why `Window` is thinner than
its name suggests.

### Frame pacing

A surface paints when the compositor says the last frame is on screen. The
runtime tracks a frame callback per surface: a paint requested while one is in
flight is held, and `frame()` clears the flag and dispatches the pending paint.
An app that redraws in a tight loop is therefore throttled to the output's
refresh rate without doing anything itself, and one that redraws on a timer
never gets ahead of the compositor.

## Drawing

Two paths, and an app picks one:

**Straight Skia.** Get a canvas for the surface and draw. This is what
otto-islands, otto-lock, otto-greeter and most of otto-bar do — they own a
model and paint a bespoke view of it every frame.

**The lay-rs engine.** Call `AppContext::enable_layer_engine(w, h)` before
creating a surface, and the app gets the same retained scene graph the
compositor uses: layers with positions, opacity, blur, corner radius and spring
animations, updated on a background ticker and drawn with `draw_scene`. Use it
when the UI animates. The order matters — the engine has to exist before the
surface, because a surface builds its root layer node when it is created.

`rendering/` holds the pieces underneath both: `SkiaContext` (the shared
`DirectContext`), `SkiaSurface` and `EglSurfaceResources` (per-surface EGL), and
`LayersRenderer` (the lay-rs engine plus its update thread).

See [The Scene Graph](scene-graph.md) and [Layers](layers.md) for the engine
itself; it is the same one the compositor runs.

## Components

Two shapes, for the reason described at the top:

**Stateless `Renderable` builders** — a value that knows how to paint itself.

```rust
Label::new("Cursor size").with_style(styles::SUBHEADLINE).render(canvas);
```

**Retained state plus an immediate-mode renderer** — for anything interactive.
The caller owns a state struct, calls `render_at(canvas, w, h)` to draw it, and
feeds it `on_pointer_down` / `on_pointer_drag` / `on_pointer_up` / `on_key`,
each returning a response describing what changed. `TextInput` set this
precedent; the form controls, scroll view, dropdown, slider and context menu
all follow it.

| Group | Components |
|-------|-----------|
| Text and images | `Label`, `Icon`, `SvgIcon` |
| Containers | `Frame`, `Stack`, `Toolbar`, `ScrollView` |
| Controls | `Button`, `Toggle`, `Slider`, `TextInput`, `Dropdown`, `ColorPicker` |
| Collections | `List`, `SourceList` |
| Menus | `MenuBar`, `ContextMenu`, `MenuItem` |
| Window chrome | `Titlebar`, `WindowControl`, `Decoration`, `SharingIndicator`, `Window` |

`Titlebar`, `WindowControl` and `ContextMenu` are the ones the compositor draws
directly, so a change to them lands on server-side decorations and the dock's
menus at the same time as on apps.

## Theme, typography and icons

`theme::Theme` is the palette. It is derived, not configured by the app:
`AppContext::current_theme()` is `Theme::for_scheme(current_color_scheme())`,
with the accent folded in.

Both inputs come from the freedesktop settings portal —
`org.freedesktop.appearance`'s `color-scheme` and `accent-color` — read once at
startup and then watched for `SettingChanged`, each kept in an atomic. So every
otto-kit app follows the user's light/dark and accent choice with no code, and
switches live. The portal backend is optional, so light/dark has a second
source: the compositor publishes its configured scheme as `OTTO_COLOR_SCHEME`,
which `color_scheme.rs` falls back to when the portal has answered nothing —
startup-only, and always outranked by the portal. Otto's own backend for that portal is
[`xdg-desktop-portal-otto`](settings-dbus-api.md); see
[Color Scheme](color-scheme-setting.md) for the whole path.

`typography::styles` holds the named text styles (`SUBHEADLINE` and friends);
`icons` and `icon_theme` resolve icon names against the user's icon theme.

## Otto's own protocols

`protocols/` generates client bindings from the XML in `protocols/`:

- **`otto-surface-style-unstable-v1`** — lets a client hand the *compositor*
  a surface's size, position, corner radius, blur, shadow and colour, and have
  them animated server-side with springs. This is what makes the dynamic island
  morph rather than cross-fade: the geometry is animated by Otto, and the
  content is drawn once at the target size. See
  [Surface Style Protocol](surface-style-protocol.md).
- **`otto-dock-v1`** — the dock's client-side contract: publishing a dock item,
  and pushing per-app badge counts and progress. otto-islands uses the badge
  half to put unread notification counts on dock icons.

## Everything else

`desktop_entry` parses `.desktop` files (the dock and launcher's app database);
`filetype` resolves MIME types by glob and by content sniffing, and `preview`
renders file thumbnails against the shared freedesktop cache — both for
otto-files and otto-quickview. `clipboard` and `dnd` cover selections and drag
and drop — `clipboard::set_text` and `clipboard::text` are the plain-text pair
a text field needs, since `TextInput` owns no clipboard itself: it answers a
`Copy` or `Cut` key with `TextInputResponse::Clipboard(text)` for the host to
offer, and takes a paste as `TextInputKey::Paste(text)` already read. `sound`
plays feedback sounds, and `lottie` plays Lottie animations
(the greeter and lock screen use it).

## Building and running

```sh
cargo build -p otto-kit
cargo run -p otto-kit --example simple_app
```

The `examples/` directory is the practical reference — around thirty of them,
one per component or surface pattern: `simple_app` (toplevel + menu),
`window_with_titlebar`, `sidebar_window`, `form_controls_gallery`,
`list_gallery`, `dropdown_gallery`, `titlebar_gallery`, `scroll_ab`,
`blur_window`, `music_notch_layer` and `dock_application_layer` (layer-shell and
dock surfaces), plus probes like `output_probe` and `clip_children_probe`.

Run any of them against a development compositor:

```sh
cargo run -- --winit &
WAYLAND_DISPLAY=wayland-1 cargo run -p otto-kit --example form_controls_gallery
```

## Testing

The `testing` feature exposes `otto_kit::testing::TestClient` — a minimal
Wayland client built on SHM buffers, with no EGL, Skia or `AppRunner`. It
exists so the compositor's end-to-end tests can drive real clients:

```rust
let mut client = TestClient::connect("wayland-1").unwrap();
let toplevel = client.create_toplevel("test-window", 200, 150);
client.roundtrip().unwrap();
assert!(toplevel.lock().unwrap().configured);
```

Those tests live in the compositor's `tests/` and run behind its `headless`
feature — see [Project Structure](project-structure.md).

## Where the gaps are

[otto-kit Roadmap](otto-kit-roadmap.md) is the gap analysis: which parts of the
toolkit an app still has to work around, and the order the remaining pieces are
being built in. This page describes what exists; that one describes what does
not.
