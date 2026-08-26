# Changelog

All notable changes to this project will be documented in this file.

## [1.0.0-rc.2] - 2026-08-26

### 🚀 Features

- *(expose)* Desktop widget layer in exposé, docs screenshots, gesture harness (#152)

### 🐛 Bug Fixes

- *(shell)* SSD titlebar under scanout, and resize borders (#151)
- *(shell)* Double click a titlebar to zoom the window (#154)

### 📚 Documentation

- Desktop widgets guide, page titles and descriptions (#153)

### ⚙️ Miscellaneous Tasks

- Usable defaults, island scaling, tiling and input polish (#156)
- *(release)* Point the binary PKGBUILD at v1.0.0-rc.2

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
