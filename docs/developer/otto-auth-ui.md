# otto-auth-ui

The password panel Otto shows when it needs to know who is at the keyboard: the login screen, the lock screen and the polkit dialog all draw it. It is a library. The drawing knows nothing of greetd or PAM: a client translates its own conversation into a `View`, hands it to `Panel::update`, and asks `Panel::action_at` where a click landed. The PAM conversation that otto-lock and otto-authorize share lives here too, behind a feature.

It sits in the **kit** layer (see [Architecture Rules](architecture.md)) and is used by [otto-greeter](otto-greeter.md), [otto-lock](otto-lock.md) and [otto-authorize](otto-authorize.md).

## Dependencies

- **Internal:** `otto-kit` for typography, theme colours, components, the Lottie player, i18n and the XDG config paths.
- **External:**
  - `laye-rs` and `taffy` lay out the panel; Skia (with `svg` and `skottie`) draws it.
  - libpam, only with the `pam` feature (pulls in `libc`). It is off by default so the greeter, which talks to greetd, does not link libpam.
  - logind on the system bus, through `zbus`, for the suspend, restart and shut down buttons. polkit decides whether they are allowed.
  - `zeroize` wipes typed secrets.
  - Reads `/etc/passwd` and the usual avatar locations for the user's name and picture, and Otto's `config.toml` for the wallpaper, colours and font.

## Develop

- `cargo run -p otto-auth-ui --example preview -- /tmp/panel` renders every panel state to PNGs, with no fullscreen overlay to take over the screen. `OTTO_PANEL_WALLPAPER=resources/wallpaper.jpg` frosts a real wallpaper instead of the fallback gradient.
- Tests: `cargo test -p otto-auth-ui` and `cargo test -p otto-auth-ui --features pam` (the PAM service tests are behind the feature).
- To see it live, run one of its clients (see their guides).

## Code map

- `src/lib.rs` is the public API.
- `src/panel.rs` lays out and draws the panel, in all its states, and hit-tests it.
- `src/pam.rs` is the PAM conversation on its own thread, and the `Service` stacks with their fallbacks (`pam` feature).
- `src/secret.rs` is the field's buffer (`SecretInput`).
- `src/reader.rs` recognises `pam_fprintd`'s messages so they can be shown in the panel's language.
- `src/power.rs` asks logind to suspend, restart or shut down, off the main thread.
- `src/user.rs` finds the user's name and avatar; `src/appearance.rs` reads the look from Otto's config.
- `src/pacing.rs` bounds frame pacing and keeps the minute clock.
- `assets/touch_id.json` is the fingerprint animation.

## Rules

- Keep greetd and PAM out of the drawing. What differs between clients (protocol, session picker) stays in the client.
- Do not make `pam` a default feature, and do not use it from otto-greeter.
- Secrets live only in `SecretInput` and `Zeroizing` buffers. The field's buffer is reserved in full up front and refuses input past it, because a growing `String` frees an unwiped copy.
- PAM and logind calls run off the main thread and are polled, so the panel keeps drawing while a module holds the conversation.
- The config is read directly, not through the compositor's `Config` type, which this crate cannot depend on. Keep the field names in step with Otto's `config.toml`.
- Sizes are logical points on a canvas the caller has already scaled.

## See also

- Specs: [lock screen](../../specs/lock-screen.md), [login mode](../../specs/login-mode.md).
- [Permissions](permissions.md) for the polkit side.
