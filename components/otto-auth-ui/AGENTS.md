# otto-auth-ui

The password panel Otto shows when it needs to know who is at the keyboard: the login screen, the lock screen and the polkit dialog all draw it. The drawing knows nothing of greetd or PAM: a client translates its own conversation into a `View`, hands it to `Panel::update`, and asks `Panel::action_at` where a click landed. The PAM conversation that otto-lock and otto-authorize share lives here too, behind a feature.

- **Layer:** kit. Used by `otto-greeter`, `otto-lock` and `otto-authorize`.

## Dependencies
- **Internal:** `otto-kit` for typography, theme colours, components, the Lottie player, i18n and the XDG config paths.
- **External:** `laye-rs` and `taffy` lay out the panel; Skia (with `svg` and `skottie`) draws it. libpam only with the `pam` feature, off by default so the greeter does not link it. logind on the system bus through `zbus` for suspend, restart and shut down (polkit decides). `zeroize` wipes typed secrets. It reads `/etc/passwd`, the usual avatar locations, and Otto's `config.toml` for wallpaper, colours and font.

## Develop
- `cargo run -p otto-auth-ui --example preview -- /tmp/panel` renders every panel state to PNGs, with no fullscreen overlay. `OTTO_PANEL_WALLPAPER=resources/wallpaper.jpg` frosts a real wallpaper instead of the fallback gradient.
- Tests: `cargo test -p otto-auth-ui` and `cargo test -p otto-auth-ui --features pam` (the PAM service tests are behind the feature).
- To see it live, run one of its clients.

## Where things are
- `src/lib.rs` is the public API; `src/panel.rs` lays out, draws and hit-tests the panel in all its states.
- `src/pam.rs` is the PAM conversation on its own thread (`pam` feature); `src/secret.rs` is the field's buffer.
- Around them: logind power calls, the user's name and avatar, the look from Otto's config, frame pacing, and the fingerprint messages and animation.

## Rules
- Check layout changes with the `preview` example rather than a live overlay.
- Keep greetd and PAM out of the drawing. What differs between clients (protocol, session picker) stays in the client.
- Do not make `pam` a default feature, and do not use it from otto-greeter.
- Secrets live only in `SecretInput` and `Zeroizing` buffers. The field's buffer is reserved in full up front and refuses input past it, because a growing `String` frees an unwiped copy.
- PAM and logind calls run off the main thread and are polled, so the panel keeps drawing while a module holds the conversation.
- The config is read directly, not through the compositor's `Config` type, which this crate cannot depend on. Keep the field names in step with Otto's `config.toml`.
- Sizes are logical points on a canvas the caller has already scaled.

## See also
- Specs: [lock screen](../../specs/lock-screen.md), [login mode](../../specs/login-mode.md).
- [Permissions](../../docs/developer/permissions.md) for the polkit side.
- [otto-greeter](../otto-greeter/AGENTS.md), [otto-lock](../otto-lock/AGENTS.md), [otto-authorize](../otto-authorize/AGENTS.md).
