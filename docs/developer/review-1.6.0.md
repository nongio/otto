# Post-1.6.0 codebase review

A sweep of the workspace at `v1.6.0` (847cbe2b) for duplication, hacks and
shortcuts, reinvented wheels, dependency hygiene, and whether the kits hold
the shared logic. Paths are relative to the repo root; line numbers are as of
the tag. Effort: **S** an afternoon, **M** a few days, **L** a week or more.
Items marked *(verified)* were confirmed by reading the code, the rest come
from the review sweep and should be confirmed before acting on them.

Overall: literal copy-paste is low (≈3% of statement windows). The cost is in
helpers re-written per crate, parallel code paths that have drifted, a few
god-objects, and kit widgets the apps never adopted.

## 1. Bugs found along the way

- [x] **x11 backend does not compile** *(verified)*. `src/x11.rs:475` calls
  `classify_windows` with 4 arguments; `src/state/window_throttle.rs:196`
  takes 6. CI never builds `--features x11`. Delete the backend (CLAUDE.md
  already calls it unmaintained) or add a clippy step for it. **S** Fixed in #259.
- [x] **otto-files shows modification times in UTC** *(verified)*.
  `components/otto-files/src/model.rs:901` turns epoch seconds into a date with
  no timezone offset. `otto-search/src/dates.rs` already has `local_offset`;
  or use `chrono` (already in the workspace). **S** Fixed in #258.
- [ ] **Same hex colour, different colours** *(verified)*. otto-bar reads
  8 digits as `#AARRGGBB` (`components/otto-bar/src/config.rs:180`),
  otto-input-overlay as `#RRGGBBAA` (`components/otto-input-overlay/src/main.rs:252`),
  otto-auth-ui likewise. Six parsers in total, see §3. **S**
  - *Partly done in #261: one `otto_kit::color::parse_hex` (alpha last); otto-bar keeps `#AARRGGBB` via `parse_hex_argb` because its README documents it. Switching it is a breaking change to decide.*
- [x] **otto-bar logs nothing by default** *(verified)*. Fallback filter is
  `otto_topbar=info` (`components/otto-bar/src/main.rs:21`); the crate is
  `otto_bar`. **S** Fixed in #261.
- [x] **Screencopy writes through a read-only pointer** *(verified)*.
  `src/state/screencopy.rs:489` casts the `*const u8` from
  `shm::with_buffer_contents` to `*mut`. Use `with_buffer_contents_mut`. **S** Fixed in #259.
- [x] **Passwords are not consistently wiped.** otto-greeter never wipes
  (`otto-greeter/src/main.rs:595`, greetd payload at `greetd.rs:188`);
  otto-lock uses plain `.clear()` (`otto-lock/src/main.rs:154,394,423,860`).
  Adopt `zeroize` and a `SecretInput` in otto-auth-ui outside the `pam`
  feature. **S** Fixed in #260.
- [x] **Greeter and lock run `systemctl` on the UI thread**
  (`otto-greeter/src/main.rs:780`, `otto-lock/src/main.rs:580`). One async
  logind proxy in otto-auth-ui. **S** Fixed in #260.
- [x] **XWayland errors panic the compositor**: 32 `unwrap`/`expect` in
  `src/shell/x11.rs` on `ConnectionError`-returning calls. Log and return. **S** Fixed in #259.
- [x] **Launcher corrupts quoted Exec arguments**: `split_whitespace().join(" ")`
  before shell-splitting (`otto-launcher/src/apps.rs:229-290`). Launch via
  `otto_kit::mime_apps` instead. **S** Fixed in #258.
- [x] **otto-files text fields lack word movement / shift-selection**: four
  hand-written Keysym maps (`otto-files/src/app/keys.rs:197,263,360,431`)
  instead of `otto_kit::components::text_input::keymap::key_for`. **S** Fixed in #258.
- [ ] **Trash is incomplete vs the freedesktop spec** (`otto-kit/src/trash.rs`,
  `otto-files/src/model.rs:1667-1900`): no `$topdir/.Trash-$uid` (cross-fs
  trash copies the tree home), `.trashinfo` written after the move instead of
  reserved first with `O_EXCL`, no `directorysizes`. Fix, or adopt the `trash`
  crate. **M**
- [x] **Size formatting differs between Files and Peek**: four formatters,
  base 1000 vs 1024 (`otto-kit/src/components/attachments.rs:317`,
  `otto-kit/src/preview/mod.rs:1430`, `otto-peek/src/decode/mod.rs:370`,
  `otto-files/src/model.rs:867`). One localized helper in otto-kit. **S** Fixed in #261.
- [x] **Compositor default app can differ from the apps'**:
  `src/config/default_apps.rs:146-217` re-parses mimeapps.list and ignores
  `otto-mimeapps.list` and `[Added/Removed Associations]`. Call
  `otto_kit::mime_apps`. **S** Fixed in #259.

## 2. Dependencies

- [x] Remove unused `gl-rs` and `paste` (otto); add `cargo machete` to CI.
  *(this branch; otto-kit's `wayland-backend` looked unused but is needed by
  `wayland_scanner` macro output, so it is on machete's ignore list)*
- [x] `once_cell::sync::Lazy` → `std::sync::LazyLock` (4 uses:
  `src/theme/mod.rs`, `src/winit.rs`, `src/config/default_apps.rs`). **S** Fixed in #259.
- [ ] Hoist shared deps into `[workspace.dependencies]` (today only laye-rs):
  tracing ×21, tokio ×18, wayland-client ×18, smithay-client-toolkit ×16,
  tracing-subscriber ×16, zbus ×13, wayland-protocols-wlr ×12,
  skia-safe ×11 (`=0.93`), serde_json ×11, … **S**
- [ ] Settle zbus features: five crates use default (async-io), eight use
  `tokio`; unification builds both runtimes into otto. **S**
- [ ] xkbcommon 0.6 → 0.9 (otto, otto-emoji, otto-rdp). **S**
- [ ] toml 0.8 → 1.x with matching toml_edit (otto, auth-ui, bar, files;
  otto-agents already on 1.1). **S–M**
- [ ] image 0.24 → 0.25; thiserror 1 → 2 (ours only; v1 stays transitively). **S**
- [ ] smithay-client-toolkit 0.19 → 0.21 (drops calloop 0.13, xkbcommon 0.7). **M**
- [ ] zbus 4 → 5 (accesskit_unix and brightness already pull 5; drops zbus 4
  and nix 0.29). **M**
- [ ] bitflags 1 via laye-rs: fix upstream in layers. **S**

## 3. Kits: shared logic that lives in the apps

Proposed new otto-kit modules (or a UI-free `otto-core` re-exported by
otto-kit, so the portal and otto-agents need not link Skia): `xdg`, `dbus`,
`uri`, `logging`, `toplevels`, `tray`.

- [ ] **`xdg`**: ~14 private `config_home`/`data_home`/`runtime_dir` helpers
  and ~37 files reading `XDG_*`/`HOME` directly; `otto-agents/src/xdg.rs`
  is already complete — promote it. `user-dirs.dirs` parsed twice with
  different validation (`otto-files/src/model.rs:810`,
  `otto-settings/src/panes/search.rs:444`). **M**
  - *Partly done in #261: `otto_kit::xdg` added and the listed helpers migrated; other ad-hoc `HOME`/`XDG_*` readers remain (listed in the PR).*
- [ ] **`dbus`**: one client proxy per `org.otto.*` interface.
  `org.otto.Dialog1` ×3, `org.otto.Settings` ×3 plus raw calls,
  `org.otto.ScreenCast` raw in otto-rdp, `org.otto.Shell1` raw in otto-msg,
  `org.otto.Island` raw in otto-files. The portal `Settings` proxy is declared
  6× inside otto-kit itself, each opening its own connection. **S–M**
- [ ] **`uri`**: percent-encoding hand-written ~9× (otto-kit trash/clipboard,
  otto-peek, otto-agents-client, otto-search, `src/desktop_widget.rs`); use
  `percent-encoding`/`url` (in Cargo.lock). **S**
  - *Partly done in #261: `otto_kit::uri` added; thumbnail URIs now match GLib. otto-search `file_url` and otto-agents-client `uri.rs` remain (no otto-kit dependency).*
- [x] **hex colours**: one `otto_kit::theme::parse_hex` (see §1). **S** Fixed in #261.
- [x] **`logging`**: the same `tracing_subscriber` block in 14–15 `main.rs`,
  with inconsistent default filters; `otto_kit::init()` doing logging + i18n. **S** Fixed in #261.
- [ ] **Colour palette exists twice**: `src/theme/colors_{light,dark}.rs` and
  `otto-kit/src/theme.rs:169-230`. Make otto-kit the only table. **S**
  - *Not done: the values differ (table in #261), and the compositor light/dark menu colours look swapped. Decide the palette first.*
- [ ] **`toplevels`**: three zwlr-foreign-toplevel trackers
  (`otto-kit/src/utils/focus_watcher.rs`, `otto-launcher/src/windows.rs`,
  `otto-emoji/src/target.rs`). **S–M**
- [ ] **`tray`**: StatusNotifierItem + dbusmenu written twice
  (`otto-rdp/src/indicator.rs`, `otto-input-overlay/src/indicator.rs`, 161
  matching lines). Feature-gated module, or `ksni`. **S**
- [ ] **Desktop entries / Exec expansion**: four implementations
  (`src/shell/element.rs:515`, launcher `apps.rs`, kit `mime_apps.rs:636-764`,
  fde's `parse_exec`); greeter hand-parses sessions (`otto-greeter/src/session.rs:114`). **S–M**
- [ ] **Overlay card shell**: launcher and emoji share ~300 lines of
  animate/material/open-close/input-region code (`otto-launcher/src/main.rs:1834-1972`,
  `otto-emoji/src/main.rs`). `otto_kit::OverlayCard`. **M**
- [ ] **Auth session**: greeter/lock/authorize re-implement input,
  fingerprint→password switch, queued submit and clock tick (~300 lines, tests
  copied too). `AuthSession` in otto-auth-ui. **M**
  - *Partly done in #260: `pump`, `prompt_label`, frame and clock helpers live in otto-auth-ui; the `AuthSession` state machine remains.*
- [ ] **Chrono locale** static copied (`otto-bar/src/clock.rs:72`,
  `otto-auth-ui/src/panel.rs:42`) → `otto_kit::i18n`. **S**
- [ ] **Text elide/wrap** ×5 next to `typography::ellipsize`. **S**
- [ ] **PipeWire main loop** set up 6× (islands ×2, rdp ×2, compositor);
  compositor volume still shells out to `wpctl` (`src/audio/volume.rs`). **M**
- [ ] **Two MPRIS clients on two D-Bus stacks**: `mpris` crate (libdbus) in
  the compositor, hand-written zbus client polling position every second in
  `otto-islands/src/mpris.rs`. **M**

### Layering

- [ ] otto-kit depends on otto-search (a binary with zbus+tokio) only for the
  103-line `matching.rs`. Move it into otto-kit. **S**
  - *Not done: otto-search is deliberately toolkit-free (the agents daemon and CLI link it); only a tiny `otto-matching` crate would fix the direction.*
- [ ] otto-preview depends on the otto-files *app* and reaches into its
  internals. Extract the shared browser model/view into a library. **M**
- [ ] otto-peek is both app and library (4 consumers of thumbnailer,
  thumbcache, decode). Split `otto-peek-kit` or fold into otto-media-kit. **M**
- [ ] Fold otto-md-kit into `otto_kit::preview::markdown` (2 consumers, no
  extra deps). **S**
- [ ] otto-stash is the only app not on `AppRunner`, hence its own DnD
  receiver (`otto-stash/src/drop.rs`). **M**

### Dead kit API

- [ ] Widgets used only by otto-kit's examples: `components/{button, list,
  source_list, container, toggle, toolbar}` (~2.3k lines) while settings,
  files, islands, launcher draw their own. Converge or delete. **L**
- [ ] `otto_kit::input::keycodes`, `foreign`, `desktop_appearance` →
  `pub(crate)` or remove.
- [ ] `protocols/sc-layer-v1.xml`, `protocols/wlr-foreign-toplevel-management-unstable-v1.xml`
  are not generated by anything.

## 4. Compositor (`src/`)

- [ ] **Per-frame repaint/throttle step copied into every backend and
  drifted** (`src/winit.rs:705-780`, `src/x11.rs:465-510`,
  `src/udev/render.rs:955-995,3146-3180`, `src/headless.rs:2184`). One
  `Otto::frame_done(output, states, opts)`. **M**
- [ ] **Giant functions**: `render_surface` 1,544 lines
  (`src/udev/render.rs:361`), `render_output_frame` 772, `run_udev` 700,
  `update_backdrop_and_upper_planes` 678, `run_winit` 632, surface-style
  `request` 613, `Otto::init` 506, `expose_show_all_apply` 482. **L**
- [ ] **God-objects**: `src/workspaces/mod.rs` 7.6k (one ~200-method impl),
  `dock/view.rs` 4.6k, `state/mod.rs` 3.4k (+ ~60 lines commented-out code at
  2765-2887, 3317-3337), `config/mod.rs` 3.3k, `udev/render.rs` 3.2k. **L**
- [ ] xdg/X11 window lifecycle duplicated (destroy, unfullscreen, move; the
  1.4 s fullscreen transition ×5). Move onto `WindowElement`. **M**
- [ ] Key-action dispatch duplicated in `src/input_handler.rs` (windowed vs
  udev); swipe gesture copied as the synthetic variant
  (`src/input/gestures.rs:46-126` ≈ `:285-355`). **S**
- [x] `PointerGrab` forwarding boilerplate ×3 in `src/shell/grabs.rs` (~250
  lines); a macro. **S** Fixed in #259.
- [ ] `Arc<RwLock<…>>` around plain scalars in views (`dock/view.rs:121-157`)
  because of `tokio::spawn` timers; use calloop timers / lay-rs callbacks,
  then `Rc<RefCell>` or fields. Drives most of the unwrap counts. **M**
- [ ] Global state behind `try_lock` that silently drops writes
  (`src/textures_storage.rs:9`, `surface_config_cache.rs:55`,
  `config/mod.rs:138,455`, `settings/mod.rs:125`). **M**
- [ ] 41 of 116 `unsafe` blocks lack SAFETY comments (21 in
  `src/skia_renderer.rs`); unjustified `unsafe impl Send`
  (`renderer/textures.rs:67`, `screenshare/pipewire_stream.rs:46,154`). **S**
- [ ] Five overlapping instrumentation systems (render_metrics,
  render_phase_stats, fps_ticker, `FrameLogState`, a static FPS counter) plus
  ~25 `/tmp/otto-*` debug toggles stat'ed per frame under `dev`, one of which
  executes command files from a world-writable path. **M**
- [ ] ~120 inline animation durations/curves; a `theme::motion` module. **S**
- [ ] `clippy::mutable_key_type` allowed 47× for `ObjectId` keys; one
  newtype. 15 local `too_many_arguments` allows already covered workspace-wide. **S**
- [ ] 29 dead `pub fn`s (e.g. `state/mod.rs:1675 get_render_elements`,
  `inject_surface_layers_into_view`, `skia_renderer.rs:1130 blit_fbo_to_fbo`). **S**
  - *Partly done in #259: the 11 named ones are removed; the other ~18 are unchecked.*
- [ ] Config: 49 `default_*` fns duplicating `impl Default`; `#[serde(default)]`
  on the structs. Stringly `match id` in `src/settings/apply.rs:117`. **S**
- [ ] Open stubs: `GetPipeWireFd` always errors (`src/screenshare/mod.rs:709`);
  hard-coded selector height (`src/workspaces/mod.rs:1907`); crop ignored
  (`src/drawing.rs:127`); unbounded reschedule (`src/udev/render.rs:1934`). **S each**

## 5. Apps

- [ ] otto-files `view.rs` is 10.5k lines, with UI state in statics, and three
  rendering paths side by side (canvas, lay-rs `scene.rs`, `pane_surfaces.rs`
  — the last a workaround for otto-kit damaging the whole buffer on commit).
  Fix kit damage, finish the scene migration, split the view. **L**
- [ ] otto-files copy progress is per top-level item; a single large file shows
  none. **M**
- [ ] otto-settings: control geometry recomputed in 4 places with the same
  magic offsets (`view.rs:1257,1669,2172,2289`); 11 `OnceLock<Vec<&'static str>>`
  and 6 `.leak()` to satisfy `&'static str` IDs → `Cow`/`Arc<str>`; 29
  `Arc<Mutex>` for UI-thread-only state. **M**
- [ ] otto-settings polls `systemctl` every 5 s forever
  (`panes/agents.rs:536`), spawns `fc-list` (Skia `FontMgr` is linked) and
  `xdg-open` (use `mime_apps::open`). **S–M**
  - *Partly done in #258: polls only while the pane is visible, `xdg-open` and `fc-list` are gone; a systemd `PropertiesChanged` subscription remains.*
- [ ] otto-peek: hand-rolled MD5 (`thumbcache.rs:361`, `md-5` is in the lock),
  PNG text-chunk parser, 3 XML-unescape copies; `imagesize.rs` (367 lines)
  duplicates the `imagesize` crate already in the lock. **S**
- [ ] otto-greeter: hand-written greetd IPC (567 lines) → `greetd_ipc`. **S**
- [ ] Portal hand-writes its D-Bus interfaces (`Request` twice); screenshots
  shell out to `grim` instead of the compositor's capture (also otto-stash). **M**
- [ ] otto-emoji search matches names only — no CLDR keywords/shortcodes. **S–M**
- [ ] otto-islands `main.rs` 2.5k and `dialog.rs` 2.7k: split. **M**

## Follow-ups found while fixing

- [ ] `otto_kit::mime_apps::tokenize` keeps `%%` inside a quoted Exec
  argument as two characters instead of one `%` (found in #258). **S**
- [ ] `otto-search/src/query.rs:472` says size queries use powers of 1024
  "matching how Files shows sizes"; Files now uses 1000 everywhere (#261). **S**
- [ ] otto-files `paste_tests` put_back tests and one `scripts` test share
  environment variables and fail when run in parallel. **S**
- [ ] otto-files' own `open_in_default_app` still spawns `xdg-open`. **S**
- [ ] Clippy without XWayland and the x11 backend at runtime were not
  exercised after #259; the x11 CI step builds `default,x11`, since
  `--no-default-features --features x11` needs `udev` code paths. **S**

## Checked and fine

Protocol bindings (one XML source, server + client generated once); fuzzy
matching (one scorer shared via `otto_kit::matching`); markdown (otto-md-kit
only); clipboard (otto-kit); battery (UPower proxy + signals); polkit
(`polkit-agent-rs`); shared-mime-info and inotify code (deliberate, tested);
otto-media-kit and the otto-agents-client/kit split.
