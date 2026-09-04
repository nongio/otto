# Changelog

All notable changes to this project will be documented in this file.

## [1.0.0] - 2026-09-04

### 🚀 Features

- *(otto-kit)* Close the focused window on Cmd+W (#171)
- *(files)* Go to a path with Ctrl+L, and Alt navigation (#172)
- *(l10n)* Simplified Chinese (#174)
- *(files)* Trash window and dock places (#176)

### 🐛 Bug Fixes

- Titlebar of a promoted window, and exposé preview sampling (#170)
- *(workspaces)* Keep an unminimized window in its workspace exposé (#175)
- *(packaging)* Make every package installable, and test that it is
- *(packaging)* Name the gbm package as Fedora has it

### 📚 Documentation

- Correct the island docs, add a customization page
- *(user)* Fix claims that no longer match the code
- *(dev)* Correct stale architecture claims
- Fix README prerequisites, backends and features
- *(dev)* Rename the sc-layer doc to the style protocol

### ⚡ Performance

- Throttle layer-shell surfaces hidden behind windows (#173)

### ⚙️ Miscellaneous Tasks

- *(release)* Changelog for 1.0.0-rc.3

## [1.0.0-rc.3] - 2026-08-30

### 🚀 Features

- *(shell)* Implement ext-background-effect-v1 blur (#157)
- Feat(l10n) + fix(render): localise the shell, and snap layer sizes to the pixel grid (#168)

* feat(otto-kit): add the localisation catalogue and lookup

Fluent catalogues for nine locales, with en-GB as the source of truth and
en-US a sparse overlay of only the keys that differ. The locale comes from
Otto's own setting over the portal, falling back to the environment, so
"Preferred languages" moves the whole desktop rather than just the parts
that happen to read LANG.

The portal read runs on a thread of its own. Three components have an
async main, and building a runtime inside a runtime panics outright rather
than failing, so doing it on the caller's thread works in five components
and kills three at startup.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto): serve the settings schema and chrome translated

Each setting's label and description is keyed off its own identifier, so
adding a setting needs no catalogue entry to keep working: the English
beside it in the schema is the fallback. GetLocales lets a component ask
what language the desktop is in rather than guessing from the environment.

Two tests guard the derivation, because a mismatch fails silently — the
row simply stays English, which nobody notices until they read the pane in
their own language.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(portal): expose the desktop locales

Components read the locale over the portal, the same route the colour
scheme and accent already take.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-bar): localise the clock and menus

The clock format is a catalogue key rather than prose, so a locale
reorders the parts and chooses its own 12- or 24-hour convention. Needs
chrono's unstable-locales: without it %A and %B emit English names
whatever the locale.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-files): localise the browser

Sidebar, column headings, context menus, the Get Info panel and the file
kinds. Sizes and counts go through Fluent selectors so languages with more
than two plural forms get them right.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* fix(otto-settings): send a list setting as a list

Committing a text field always sent a string, so locales — declared
StrList — was refused by the compositor on its type and the edit did
nothing. It failed silently from the user's side: the field kept showing
what was typed while nothing was saved, because the display path already
rendered a list into one comma-separated field and only the write path
was missing its half.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-settings): localise the panes

All eight panes, their group headings and the pop-up choices. Row labels
that the compositor serves are translated there; the pane's own strings
are translated here.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-launcher): localise the search field and badges

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-greeter,otto-lock): localise the login and lock screens

Text that arrives from PAM or greetd at runtime is left as it comes:
those localise themselves, and restating them would be guessing at
another program's words. The keyed fingerprint hints are used only when
the module supplies nothing.

The power messages were built by interpolating the systemctl verb, which
put "Not permitted to poweroff" in front of users and could not be
translated at all. The verb still goes on the command line; the panel now
gets a keyed message.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-auth-ui): localise the panel and let a status line wrap

The clock formatted with chrono's plain format(), so the login and lock
screens showed English weekday and month names in every language.

Status messages carry an OS error appended to a translated prefix, and
the line neither wrapped nor clipped — it centred on a computed offset
that goes negative, so a long message ran off both edges of the card,
losing the error text. It is two lines tall now, and a single-line
message draws exactly where it did before.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(otto-islands,otto-quickview): localise

Quick View decodes in a sandboxed worker re-exec'd with env_clear() and
no bus, so neither the portal nor LANG reaches it. It is handed LANGUAGE
and initialises from the environment; without that its strings stay
English in every locale, and nothing looks broken.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* doc: spec the localisation system

Records the parts that were got wrong during implementation, since those
are what a reader will get wrong again: where the locale comes from, why
the portal read needs its own thread, why language cannot hot-reload, the
sandboxed worker, and what is deliberately left untranslated.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* chore: add the l10n-translator agent

Otto uses the informal register in every language with a T/V
distinction. A deliberate choice, not a default: Apple itself differs
between Italian and Spanish, and a desktop that is intimate in one
language and distant in another is not one product.

Claude-Session: https://claude.ai/code/session_01VJ1L2gacFdRHusyJFnu4MK

* feat(l10n): apply the locales setting to apps and entries

Otto's chrome follows the `locales` setting, but everything around it
still read `LANG`: desktop entries were parsed against `en` alone, so an
Italian desktop labelled every dock icon in English, and apps Otto
started inherited the session locale rather than the configured one.

`locale_env` publishes LANGUAGE always, and LANG/LC_MESSAGES only for a
locale the machine has actually generated — naming an ungenerated one
makes setlocale fall back to C and loses the translation outright. The
assignments ride along with WAYLAND_DISPLAY into the systemd and D-Bus
activation environments, since bus-activated services inherit nothing.

Claude-Session: https://claude.ai/code/session_01HFNdvF58KVhXREHnQS7SwT

* fix(settings): name the colour scheme and system font rows

The Appearance group had a row also called "Appearance", which said
nothing about what it set, and the General pane pointed its first row at
the group's own key so both lines read the same. The scheme row now has
its own key and offers Light and Dark as labelled choices; "Font" is
"System font", which is what it changes.

Claude-Session: https://claude.ai/code/session_01HFNdvF58KVhXREHnQS7SwT

* feat(otto-files): ship an app icon

Files borrowed the generic system-file-manager icon from whatever theme
happened to be installed. It now carries its own hicolor set, installed
by every packaging path. Also bumps the otto-git pkgver.

Claude-Session: https://claude.ai/code/session_01HFNdvF58KVhXREHnQS7SwT

* style(otto-auth-ui): rustfmt the panel

Claude-Session: https://claude.ai/code/session_01HFNdvF58KVhXREHnQS7SwT

* feat(otto-files): localise the listing, Get Info and the picker

Files was still speaking English wherever the string was built rather
than looked up: the item and hidden counts under the listing, every
field name in Get Info and its permissions grid, the folder errors, and
the save sheet down to its window titles.

SaveAction::Blocked now carries the key for its reason rather than the
prose. The picker suppresses the empty-name block — that one is the
placeholder's job to explain — by comparing what Blocked carries, and
comparing translated text held only in English, so every other locale
grew a warning under a field the user had not typed in yet.

Claude-Session: https://claude.ai/code/session_01HFNdvF58KVhXREHnQS7SwT

* test(otto-files): stop asserting user-facing strings in English

A test binary resolves its locale from the environment, so the suite ran
against the Italian catalogue on an Italian developer's machine and five
tests failed on prose they had no business pinning. Each now compares
against the same lookup the code performs, which still asserts what the
test is about: the unit a size crosses into, the operation the undo
stack recorded, the month civil_from_days landed on.

Claude-Session: https://claude.ai/code/session_01HFNdvF58KVhXREHnQS7SwT

* fix(otto-lock): say the reader's finger prompt in Otto's words

pam_fprintd phrases its own request for a finger, in its own process
locale and with the reader's model name in it — "Place your right index
finger on Elan Fingerprint Sensor" under an Italian card. What it asks is
a choice from a fixed table, so read the finger out of it and say it
again from the catalogues. A missed finger goes the same way; the rest of
what the module says is guidance, and keeps its own words.

Claude-Session: https://claude.ai/code/session_01SkrMExyGxVWLVUTaqVUkJG

* fix(otto-greeter): say the reader's finger prompt in Otto's words

greetd relays pam_fprintd's message unchanged, and greetd's environment
is barer than a session's, so the module's English reached the card
however the greeter was set. Same fix as the lock screen, and the parsing
moves to otto-auth-ui, which both clients already share for the panel —
the greeter had a second copy of mentions_fingerprint of its own.

Claude-Session: https://claude.ai/code/session_01SkrMExyGxVWLVUTaqVUkJG

* build: lock the localisation dependencies

The catalogue work adds fluent-bundle, unic-langid and intl-memoizer, which
were not in the lockfile main carries. CI runs with `--locked`, so the branch
would not build there without this.

Resolved minimally rather than regenerated: nothing already pinned moves.

Claude-Session: https://claude.ai/code/session_018GdzP2cU2w1SyKVppu4SgX

* wip(shell): fixed-size windows and popups across a geometry change

RECOVERED WORK — committed unverified to preserve it. The session that
wrote this exited without committing; the files sat orphaned in the shared
tree with no process holding them. Not built or tested by the committer,
and not run on hardware. Test before trusting it.

Three pieces, all apparently complete and spec'd:

Fixed-size windows are no longer maximized. `WindowElement::is_resizable()`
reads the toplevel's cached min/max size — equal non-zero values on both
axes mean the client asked for one size and no other, as otto-files' Get
Info panel does. maximize_request and tile_focused_window bail for such a
window, and its zoom control is drawn gray with no hover glyph and no
press state, via a new `fixed_size` flag on WindowDecorationModel.
Maximizing one configured the client to a size it will not draw, stranding
its layout in the corner of an empty surface.

Popups are repositioned when their parent's geometry changes.
`reposition_popups_for_window` re-runs the unconstrain pass and configures
the clients on maximize, tile and restore, so an open menu does not ride
off the screen edge when the window moves out from under it. A client
whose positioner is not reactive keeps the placement it committed.

Test support: otto-kit gains `roundtrip_timeout` and a `poll_readable`
helper so a test roundtrip cannot block forever, and otto-files gains two
layout tests asserting the permissions grid and the Get Info field names
still fit the panel once translated.

Based on feat/localisation, not main: the otto-files tests read the
localisation catalogues.

Claude-Session: https://claude.ai/code/session_018GdzP2cU2w1SyKVppu4SgX

* fix(render): snap layer sizes to the pixel grid

#161 snapped layer positions onto whole physical pixels but left sizes
alone, and a size reaches a layer the same way a position does — a
logical integer multiplied by the output scale. So every box still had
a fractional FAR edge: the titlebar is 34 logical points, and at scale
1.75 that is 59.5px, painting its bottom hairline across three physical
rows with no fully covered one.

The content layer's offset below the bar came from the same 59.5, so a
decorated window's entire client subtree started on a half pixel and was
resampled — including a client that had painted an exact 1:1 buffer.

Add snap_extent_px, which snaps the far EDGE rather than the extent on
its own; rounding an origin and an extent independently moves the edge a
whole pixel when the two round in opposite directions. Apply it to the
window box in both update_window_view paths, the decoration layer, the
content layer's offset (rounded from the same value as the bar height,
so the client still starts exactly where the bar ends), and the surface
layer's own size.

Claude-Session: https://claude.ai/code/session_01CsyEHLhe8QnQ6BdL2QmWUS

* chore(deps): bump rand from 0.8.5 to 0.10.2

Bumps [rand](https://github.com/rust-random/rand) from 0.8.5 to 0.10.2.
- [Release notes](https://github.com/rust-random/rand/releases)
- [Changelog](https://github.com/rust-random/rand/blob/master/CHANGELOG.md)
- [Commits](https://github.com/rust-random/rand/compare/0.8.5...0.10.2)

---
updated-dependencies:
- dependency-name: rand
  dependency-version: 0.10.2
  dependency-type: direct:production
...

Signed-off-by: dependabot[bot] <support@github.com>

* chore(deps): bump bytes from 1.11.0 to 1.11.1

Bumps [bytes](https://github.com/tokio-rs/bytes) from 1.11.0 to 1.11.1.
- [Release notes](https://github.com/tokio-rs/bytes/releases)
- [Changelog](https://github.com/tokio-rs/bytes/blob/master/CHANGELOG.md)
- [Commits](https://github.com/tokio-rs/bytes/compare/v1.11.0...v1.11.1)

---
updated-dependencies:
- dependency-name: bytes
  dependency-version: 1.11.1
  dependency-type: direct:production
...

Signed-off-by: dependabot[bot] <support@github.com>

* fix(otto-auth-ui): assert the status box height at compile time

Both sides of the assertion are consts, so clippy's
`assertions_on_constants` rejects it as a runtime `assert!` and the
components lint job fails under `-D warnings`. A `const` block checks
the same thing where it belongs.

Claude-Session: https://claude.ai/code/session_01CsyEHLhe8QnQ6BdL2QmWUS

---------

Signed-off-by: dependabot[bot] <support@github.com>
Co-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>
- *(a11y)* AT-SPI support for the shell and otto-kit (#160)

### 🐛 Bug Fixes

- *(otto-kit)* Titlebar material opacity and init-time theme delivery (#158)
- *(render)* Snap surfaces to the pixel grid on fractional scales (#161)
- *(workspaces)* An autohidden dock reserves no space (#162)
- *(shell)* Blur only the region a client asks for (#166)
- *(planes)* Keep a demoted window in its place in the stack (#167)
- *(expose)* Make show desktop work, and animate its exit (#169)

### ⚙️ Miscellaneous Tasks

- Run each area's checks only when it changes (#159)
- *(release)* 1.0.0-rc.3

## [1.0.0-rc.2] - 2026-08-26

### 1.0.0-rc.2

- Usable defaults, island scaling, tiling and input polish (#156)

### 🚀 Features

- *(expose)* Desktop widget layer in exposé, docs screenshots, gesture harness (#152)

### 🐛 Bug Fixes

- *(shell)* SSD titlebar under scanout, and resize borders (#151)
- *(shell)* Double click a titlebar to zoom the window (#154)
- *(ci)* Name the Arch tarball after the workspace version

### 📚 Documentation

- Desktop widgets guide, page titles and descriptions (#153)

### ⚙️ Miscellaneous Tasks

- *(release)* Point the binary PKGBUILD at v1.0.0-rc.2
- *(release)* Regenerate the changelog through v1.0.0-rc.2

## [1.0.0-rc1] - 2026-08-23

### 🚀 Features

- Fuzzy font finder (#65)
- Wlr layer background support (#68)
- Autostart (#69)
- Add otto topbar component (#84)
- Otto-islands — experimental notification manager (#86)
- *(perf)* Per-window frame callback throttling (#92)
- *(test)* Headless integration testing framework and expose bug fixes (#93)
- *(dock)* Add dedicated dot_layer for running app indicator (#95)
- Implement wlr-screencopy-v1 protocol (#94)
- *(input)* Implement wlr-virtual-pointer-unstable-v1 (#96)
- *(screencopy)* Unify with screenshare dmabuf blit path (#98)
- *(dock)* Bounce app icon while a launch is in progress (#102)
- *(tiling)* Snap windows to left/right halves and maximize (#104)
- *(placement)* Place new windows by least overlap (#105)
- *(kms)* Plane scanout and cross-plane backdrop blur (#110)
- *(output)* Per-output rendering and virtual outputs (#111)
- *(remote)* RDP bridge, AirPlay screenshare, XWayland fullscreen games (#112)
- *(shell)* Popup overlay blur, portal access dialog, lid power (#109)
- *(lock)* Session lock and login greeter (#113)
- *(lock)* Idle lock, packaging, dock/expose fixes (#114)
- *(screenshare)* Share a single window (#121)
- *(dock)* Drag the handle to resize the dock
- *(kit)* Shared toolkit for Otto's apps
- *(shell)* Dock, window decorations, exposé and plane scanout
- *(config)* Bind the launcher to Ctrl+Space in the example config

### 🐛 Bug Fixes

- Various otto bug fixes and refactors (#83)
- Send output_enter in wlr-foreign-toplevel protocol (#89)
- Popup crash and subsurface rendering (#91)
- *(scale)* Correct otto-bar sizing on fractional-scale outputs (#119)
- *(planes)* Map plane damage without the workspace scroll (#122)
- *(portal)* Lowercase impl portal version property (screencast cursor mode) (#123)
- *(blur)* Tone-map the backdrop blur so panels stay readable (#126)
- *(portal)* Restore an approved screencast source instead of asking twice (#125)
- *(tiling)* Widen drag side snap zones (#135)
- *(config)* Keep dock defaults and hand-edited keys intact (#132)
- *(lock)* Lock screen buttons are not clickable (#128)
- *(expose)* Keep the top bar hidden during a workspace swipe (#129)
- *(config)* Drop dock keys older builds copied into the user config (#133)
- *(website)* Resolve static asset links against baseURL
- *(website)* Create content/ dir before build, drop stale README merge hack
- *(website)* Canonify root-relative cross-links against baseURL
- *(ci)* Drop otto-settings from deb/rpm assets
- *(packaging)* Ship the desktop apps in every package format
- *(packaging)* Keep user config across Arch upgrades
- *(packaging)* Keep user config across Arch upgrades
- *(files)* Hide Quick View chrome while the card is tiny
- *(render)* Unstick virtual output streaming

### 🚜 Refactor

- Update otto-kit with new features and components (#81)
- Migrate otto to use otto-kit features (#82)

### 📚 Documentation

- Add AGENTS instructions and specs (#80)
- Refresh README and drop stale plan notes (#116)
- Add user guide covering the desktop and its features (#117)
- Recommend region-qualified locale identifiers in config (#118)
- Refresh the developer guide and add architecture diagrams
- *(website)* One page per developer doc, like the user guide
- Add a scene graph page

### ⚡ Performance

- *(renderer)* Fix GL/Skia format mapping and defer GPU fence wait
- *(planes)* Sync once per frame, and fix expose gesture flicker (#139)

### 🧪 Testing

- *(screenshare)* Headless e2e tests for the screencast control plane
- *(rdp)* Cover the bridge's logic and the compositor contract it runs on
- *(shell)* Cover expose previews, click-to-raise and app switcher order
- End-to-end coverage for screenshare, the RDP bridge and shell interactions

### ⚙️ Miscellaneous Tasks

- Upgrade skia-safe 0.88 → 0.93, layers 1.7 → 1.8 (#85)
- Bump pinned toolchain to 1.97.0 (#130)
- Install libpam0g-dev so otto-lock links against libpam
- Build the release binaries once, not once per package job
- Cache the packaging tools again
- Ignore makepkg's in-tree srcdir extraction
- *(release)* 1.0.0-rc.1

### Apps

- File browser, app launcher and settings (#141)

### Docs

- Layers page, app guides, and a sidebar fix (#149)

### Islands

- One island per notification, growing row that pushes its neighbours (#147)

### Toolkit

- The shared foundation Otto's apps are built on (#143)

## [0.15.0] - 2026-03-20

### 🚀 Features

- Configurable accent color! (#44)
- Add spacing between workspaces (#45)
- Dynamic window shadows based on window focus (#47)
- Audio and brightness osd attached to fn keys (#48)
- *(power-management)* Lid close detection (#49)
- *(audio)* Add configurable volume sound feedback with XDG Sound Th… (#50)
- Virtual outputs streamed via PipeWire (#53)
- *(multi-monitor)* Per-output independent workspace sets (wip) (#56)

### 🐛 Bug Fixes

- Wlr layer popup focus (#46)
- *(pkgbuild)* Fix nightly prepare() failing on re-run and broaden gitignore
- Popup grabs not released
- Minimise window on auto-hide dock (#73)
- *(packaging)* Fix Arch PKGBUILD and remove it from release tarball

### 📚 Documentation

- Document new features

### ⚙️ Miscellaneous Tasks

- Update unreleased updates in changelog
- Cargo fmt and clippy
- *(pkgbuild)* Reset PKGBUILD-git pkgver placeholder
- *(release)* V0.15.0

### Fix

- Arch packaging (#60)

### Wip

- Otto-kit frontend ui components (#54)

## [0.14.0] - 2026-02-06

### 🚀 Features

- Add keyboard shortcuts for brightness control (#35)
- Implement wlr-gamma-control-v1 protocol for night shift (#36)
- Media controls with Mpris (#37)
- Add XDG config directory support (#38)
- Add multi-distro packaging and login manager support (#42)

### 🚜 Refactor

- Split input_handler into modules (#30)
- Split skia_renderer in multiple modules (#31)
- Split udev into modules (#32)

### 📚 Documentation

- Separate user / developer documentation
- Separate user / developer documentation
- Update config example

### ⚙️ Miscellaneous Tasks

- Release v0.14.0

## [0.12.0] - 2026-01-20

### 🚀 Features

- Implement cursor_shape protocol with new CursorManager

### 🐛 Bug Fixes

- Resolve all clippy warnings
- Remove duplicate delegate imports after merge
- Cap screenshare framerate at 60fps for Chrome/WebRTC compatibility

### ⚙️ Miscellaneous Tasks

- Bump minimum Rust version to 1.85.0
- Update Rust toolchain to 1.85.0 in GitHub Actions
- Add libpipewire-0.3-dev to system dependencies
- Use ubuntu-24.04 for clippy to match pipewire 0.9 requirements
- Release v0.12.0

### Cargo

- Pin smithay

## [0.11.0] - 2026-01-20

### 🚀 Features

- Bump up smithay
- Initial support for foreign toplevel protocol
- Apps-manager component init
- Initial protocol clients sample clients and system design
- Add window-specific popup visibility control
- Improve application info loading and icon fallback
- Update sc-layer protocol implementation
- Add session startup scripts
- *(portal)* Add compositor watchdog for health monitoring
- *(compositor)* Track and apply layer shell exclusive zones
- Add configurable icon_theme option
- Add wlr-foreign-toplevel-management protocol support
- Support monitor resolution and refresh rate from config
- Animated window size and position
- Smart window placement for fullscreen workspaces
- Improve natural layout with grid-based initial positioning
- Add touchpad configuration options
- *(compositor)* Track and apply layer shell exclusive zones
- Add configurable icon_theme option
- Add wlr-foreign-toplevel-management protocol support
- Support monitor resolution and refresh rate from config

### 🐛 Bug Fixes

- Buffer exaaustion for slow clients for screenshare
- Upgrade smitahy, chrome viewport crash
- Skip dock/workspace selector animations for non-current workspaces
- Prevent window jump when dragging maximized windows
- Reposition window during top/left edge resize
- Use requested size for touch resize positioning
- Dock rendering
- Better AGENT.md
- Workspace + sclayer early init
- Dock scaling + config
- Ux style + ux improvement
- Update puffin_http to 0.16 for compatibility with puffin 0.19
- Lighten window shadows to prevent excessive darkening when overlapping
- Layers visibility
- Set WAYLAND_DISPLAY env variable
- Account for reserved areas when calculating new window position
- Session script start gtk portal
- Fullscreen direct scanout timing and workspace naming
- Dock show/hide
- Prevent crash on window unmaximize
- Fullscreen
- Update puffin_http to 0.16 for compatibility with puffin 0.19

### 🚜 Refactor

- Improve expose gesture handling and API

### 📚 Documentation

- Review doc files
- Add profiling section to README
- Add foreign toplevel management documentation
- Add dock migration strategy to foreign-toplevel
- Add profiling section to README
- Add foreign toplevel management documentation
- Add dock migration strategy to foreign-toplevel

### 🎨 Styling

- UI refinements for dock, expose mode, and app switcher

### ⚙️ Miscellaneous Tasks

- Initial protocol implementation layer protocol
- Rendering metrics calculation
- Rendering metrics calculation

### Fmt

- Suppress dead_code warnings for text style functions

## [0.10.0] - 2025-12-16

### 🚀 Features

- Xdg-desktop-portal for screencomposer
- Screenshare fullscreen
- Session script for dbus and keyring
- Script for aumated testing

### 🐛 Bug Fixes

- Agent instructions + CLAUDE.md symlink
- Agents.md

### 📚 Documentation

- Update screenshare

### ⚙️ Miscellaneous Tasks

- *(release)* V0.10.0

### Review

- Remove unused deps

## [0.9.0] - 2025-12-08

### 🚀 Features

- Theme colors, text styles + config
- Multiple workspaces
- Gate perf counters behind feature flag
- Enable debugger feature in default build
- Add scene snapshot functionality
- Make keyboard shortcuts configurable
- Allow remapping modifiers and keysyms
- Toggle maximize window
- Display config
- Sample-clients for submenus
- First implementation of wlr layers
- Enable swipe workspace gesture
- Direct scanout for fullscreen windows in udev backend

### 🐛 Bug Fixes

- Texture loading
- Improve workspace layout and sizing
- Add allow unsafe_code attribute for font initialization
- Workspace rendering
- Dock + app switch theme
- Keyboard mappings
- Dock rendering colors
- Interaction bugs in dock
- Expose show all
- Prevent dragging fullscreen surfaces
- Workspace selector preview size
- Minimize windows
- Delete fullscreen workspace
- Reset focus on minimize window
- Genie effect glitches
- On undo window drag/drop restore expose window sorting
- When moving windows between workspaces ensure the expose is uptodate
- Workspace move indexing
- Clean logs
- Opening appswitch should exist expose mode
- Popup surface commit / update
- Popups rendering
- Keyboard focus when switching workspaces
- Crash on wlr delete
- Expose overlay opacity on first open
- Fmt
- Clippy

### 🚜 Refactor

- Split state in multiple files
- Refactor and consolidate workspaces
- Handle all workspace elements in rendering pipeline

### 📚 Documentation

- Docs
- AGENTS docs for expose feature
- Wlr layer shell 1.0
- README + docs file update

### ⚡ Performance

- Enable image caching for better performance

### ⚙️ Miscellaneous Tasks

- Use rust 1.82.0
- Fps_ticker as custom feature
- [**breaking**] Multiple workspaces
- Simplify renderer code
- Refactor workspaces data flow, dock, app_switcher
- Run rustfmt on workspace modules
- Cleanup inative gpu logs
- Ci fix cargo cache

### Update

- Refactor transitions

## [0.2.0] - 2024-10-26

### 🐛 Bug Fixes

- Fix linter warnings
- Fix app switcher view
- Fix compile issues for xwayland
- Fix binpacking window size
- Fix window position rendering
- Fix clippy warning
- Fix compilation skia_renderer
- Fix udev
- Fix state
- Fix x11
- Fix xdg shell
- Fix grabs
- Fix input_handler
- Fix compilation errors
- Fix warnings
- Fix skia version
- Fix smithay version and clippy warnings
- Fix raise multiple windows order

### 🚜 Refactor

- Refactor input handling
- Refactor scene_element
- Refactor and optmisation of update loop
- Refactor workspace views + interactive views
- Refactor quit appswitcher app logic
- Refactor workspace views name and pointer events
- Refactor workspace, dock, add minimize windows stub
- Refactor app switcher
- Refactor window selector
- Refactor windows positioning
- Refactor scene damage tracking
- Refactor dock + animations

### 📚 Documentation

- Dock view stub
- Dock minimize animation fix

### ⚙️ Miscellaneous Tasks

- Fix build
- Remove msrv job

<!-- generated by git-cliff -->
