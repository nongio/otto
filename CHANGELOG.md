# Changelog

## [v0.13.0] - 2026-02-01


- Rename project from screen-composer to otto

Major changes:
- Rename main package: screen-composer → otto
- Rename ScreenComposer struct to Otto throughout codebase
- Rename portal package: xdg-desktop-portal-screencomposer → xdg-desktop-portal-otto
- Rename config files: sc_config.* → otto_config.*
- Rename directories: xdg-desktop-portal-sc → xdg-desktop-portal-otto, wlcs_screencomposer → wlcs_otto
- Update D-Bus services: org.screencomposer → org.otto
- Update all documentation, scripts, and source files
- Keep sc-layer-v1.xml protocol name unchanged

All main components build successfully.

- Merge pull request #23 from nongio/rename-to-otto

Rename project from screen-composer to otto

- Fix/keymap refactor (#25)

* Refactor keyboard remapping to use XKB options

Replace custom keysym-level remapping with XKB's native options system,
aligning with how Sway, Hyprland, and other compositors handle keyboard
customization.

Changes:
- Add xkb_layout, xkb_variant, xkb_options fields to InputConfig
- Update keyboard initialization to use XKB options from config
- Remove custom modifier_remap and key_remap tables
- Remove ModifierKind enum and related helper functions
- Remove keycode remapping logic from input handler
- Remove modifier remapping from keyboard focus targets
- Update config example with XKB options documentation
- Remove old remap tests

Breaking change: Users with existing [modifier_remap] or [key_remap]
configs need to migrate to XKB options. Common migrations:
- modifier_remap: logo = "ctrl" → xkb_options = ["altwin:ctrl_win"]
- key_remap: Caps_Lock = "Escape" → xkb_options = ["caps:escape"]

Benefits:
- Removes ~200+ lines of custom remap logic
- Better performance - XKB compiles options into keymap
- Standards compliance - same approach as other Wayland compositors
- More options - access to all XKB's built-in remapping options

* Migrate otto_config.toml to use XKB options

Replace old modifier_remap config with XKB options.
- Old: modifier_remap: logo = "ctrl", ctrl = "logo"
- New: xkb_options = ["altwin:ctrl_win"]

* Add comprehensive XKB configuration examples

Expand keyboard configuration examples with:
- Caps Lock remapping options (Escape, Ctrl, Backspace, etc.)
- Ctrl key remapping (swap with Caps, use Alt as Ctrl)
- Alt/Win key remapping (Mac-like, swap Alt/Win)
- International layout support (Compose key, Euro sign)
- Multiple layout switching (Alt+Shift, Win+Space, etc.)
- Numpad configuration options
- Advanced options (Shift keys for Caps Lock, compositor termination)
- Pre-configured examples for common use cases:
  * Vim-optimized setup
  * Mac-like setup
  * Emacs-optimized setup
  * Multiple language layouts

Also fixed duplicate layer_shell section in example config.

* Add keyboard debugging tools and fix ISO_Left_Tab shortcut

New debugging tools:
- scripts/show-keys.sh: Real-time keyboard event viewer
- scripts/check-xkb-config.sh: XKB configuration inspector
- scripts/README.md: Documentation for keyboard scripts

Keyboard shortcut fixes:
- Fix Ctrl+Shift+Tab: Use Ctrl+ISO_Left_Tab (XKB behavior)
- Add debug logging for keyboard shortcuts (RUST_LOG=debug)

Config updates:
- Enhanced otto_config.example.toml with XKB tool references
- Fixed otto_config.toml: xkb_options under [input] section

* Update documentation for XKB-based keyboard system

* Update documentation for XKB-based keyboard system

* fmt+clips

* Add cargo test to CI workflow

* add tests in CI

* test: async app info in CI

* disable running tests in CI

- Feat/show desktop (#28)

* Add Settings portal for color scheme (Light/Dark)

Implements org.freedesktop.impl.portal.Settings to expose Otto's theme
configuration to applications via XDG Desktop Portal.

Compositor (Otto):
- Add org.otto.Settings D-Bus interface exposing GetColorScheme()
- Reads theme_scheme from config (Light/Dark -> 1=dark, 2=light)
- Add optional gtk_theme config field for reference

Portal Backend (xdg-desktop-portal-otto):
- Implement org.freedesktop.impl.portal.Settings interface
- Proxy to compositor via org.otto.Settings
- Methods: ReadAll(), Read() (deprecated but required)
- Register alongside ScreenCast at /org/freedesktop/portal/desktop

Configuration:
- Add Settings interface to otto.portal file
- Document theme integration in docs/theme-integration.md
- Manual sync guide for legacy apps (gsettings, kdeglobals, qt5ct)

Modern GTK4/Qt6 apps automatically detect theme via portal.
Legacy apps may need manual sync (documented).

Tested: gedit (light theme works), portal responds to gdbus queries.
Single source of truth: Otto config file.

* fix: window unminimize bugs and improve animation

- Fix focus issue: raise window to top of stack when unminimizing
- Fix inactive appearance: clear color filter from dock press interaction
- Fix inactive title bar: send pending configure after activation
- Reduce unminimize animation duration from 1.5s to 0.8s

When clicking a minimized window in the dock:
1. Window now properly raised to top (focus_app_with_window)
2. Dock's darken filter cleared from window layers
3. Activation state sent to client (send_pending_configure)
4. Faster, snappier animation

* portal: complete ScreenComposer to Otto renaming

- Rename screencomposer_client module to otto_client
- Update D-Bus path from /org/screencomposer to /org/otto
- Update process name in watchdog from screen-composer to otto
- Fix typo in start_session.sh script
- Note minimize bug in README

* feat: implement show desktop with 4-finger pinch gesture

- Add 4-finger pinch gesture to trigger show desktop mode
- Windows slide off screen in radial pattern from center
- Track pinch scale delta for smooth gesture updates
- Prevent conflicts between expose and show desktop modes
- Exit show desktop when entering expose mode via F2
- Disable 3-finger workspace swipes when show desktop is active
- Use spring transition for smooth animations
- Add pinch_last_scale field to track gesture progress

* cargo fmt

* iterate on dock colors

* undo show all on switch

* Update src/theme/colors_light.rs

Co-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>

* update README

* clippy

* bump layers lib

* fix(screenshare): Fix frame timing with proper PTS timestamps

- Add frame_sequence and start_time_ns atomics to track actual rendered frames
- Set PTS (presentation timestamp) metadata when queueing PipeWire buffers
- Calculate PTS based on framerate and frame sequence for smooth playback
- Only increment frame sequence on successful buffer blit
- Fixes non-linear timeline issues in OBS recordings

* feat(screenshare): Render cursor in screenshare on DRM backend

- Hardware cursor plane not included in framebuffer on DRM
- After blitting framebuffer, bind to dmabuf and render cursor elements
- Build cursor elements for screenshare from cursor manager
- Add depth buffer to dmabuf framebuffer for complete FBO
- Cursor now visible in screenshare recordings on udev/DRM backend

Note: Cursor element building is duplicated between normal render and
screenshare to avoid complex lifetime issues with a helper function.

* feat(screenshare): implement cursor mode support

- Add cursor_mode parameter to CreateSession and StartRecording commands
- Conditionally render cursor in screenshare based on CURSOR_MODE_EMBEDDED
- Change default cursor mode from HIDDEN to EMBEDDED
- Update supported cursor modes to exclude METADATA (not implemented)
- Pass cursor mode through D-Bus interface to compositor

* update readme

* fmt clippy

* readme videos

---------

Co-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>

- Fix video links and update disclaimer wording

Updated video links and corrected wording in the README.

- Update README for clarity and consistency

- doc: separate user / developer documentation

- doc: separate user / developer documentation

- doc: update config example

- fix(example): root level configs are before tables (#29)

- refactor: split input_handler into modules (#30)

* refactor: split input_handler into modules

* chore(config): remove default app config in favour of runcmd

* cargo fmt

- refactor: split skia_renderer in multiple modules (#31)

* refactor: split skia_renderer in multiple modules

- refactor: split udev into modules (#32)

* refactor: split udev into modules

- doc: website wip

## [v0.12.0] - 2026-01-20


- feat: implement cursor_shape protocol with new CursorManager

- Add cursor_shape protocol support via CursorShapeManagerState
- Replace old Cursor with new CursorManager for modern cursor handling
- Implement direct cursor rendering in all backends (udev, winit, x11)
- Support hardware cursor planes in udev backend
- Handle client cursor surfaces and named cursor icons
- Add cursor texture caching for performance
- Remove old PointerElement in favor of direct rendering
- Update all backends to use CursorManager API
- Remove GNOME cursor settings configuration from config init

- fix: resolve all clippy warnings

- Fix lifetime elision in FrameTimer
- Remove useless .into() conversions
- Use .values() iterator instead of destructuring tuples
- Replace single-arm match with if let
- Use strip_suffix instead of manual slicing
- Add allow attributes for mutable_key_type and private_bounds

- Merge pull request #21 from nongio/cursor_shape

Cursor shape protocol

- Merge remote-tracking branch 'github/main' into fixes-ux-ui

- fix: remove duplicate delegate imports after merge

- fix: cap screenshare framerate at 60fps for Chrome/WebRTC compatibility

- Cap PipeWire stream framerate at 60fps regardless of display refresh rate
- Fixes 'no more input formats' error with Chrome on high-refresh displays
- Improve display mode refresh rate fallback logic in udev backend
- Document framerate compatibility issue and fix in screenshare.md

- cargo: pin smithay

- chore: bump minimum Rust version to 1.85.0

Required for edition2024 support in dependencies (cfg-expr 0.20.5)

- ci: update Rust toolchain to 1.85.0 in GitHub Actions

Required for edition2024 support in dependencies

- ci: add libpipewire-0.3-dev to system dependencies

Required for screenshare PipeWire integration

- ci: use ubuntu-24.04 for clippy to match pipewire 0.9 requirements

- Merge pull request #22 from nongio/fixes-ux-ui

Fixes ux UI

- chore: release v0.12.0

## [v0.11.0] - 2026-01-20


- clippy & fmt

- fix: buffer exaaustion for slow clients for screenshare

- fix: upgrade smitahy, chrome viewport crash

- fix: skip dock/workspace selector animations for non-current workspaces

The dock and workspace selector are global UI elements that were being
animated by every workspace in the expose show all loop. This caused
them to be updated instantly by non-current workspaces (no transition),
then animated smoothly by the current workspace, resulting in a jarring
fast/jumpy visual effect.

Now only the current workspace animates these global elements, while
non-current workspaces only update their windows. This ensures smooth,
consistent animations across all UI elements.

Also added comprehensive logging for expose animations including:
- Velocity tracking when gestures end
- Delta values for each workspace
- Transition state for dock and workspace selector updates

- refactor: improve expose gesture handling and API

- Refactored expose API: expose_set_visible, expose_update, expose_end_with_velocity
- Added SwipeGestureState state machine for cleaner 3-finger gesture handling
- Improved gesture direction detection with proper state tracking
- Fixed expose_dragging_window -> expose_dragged_window rename for consistency
- Updated Smithay to rev 5186cf7 (fixes Chrome viewporter data race)
- Updated README roadmap items
- Added gesture direction detection docs to expose.md
- Removed deprecated expose_show_all(-1.0) calls in favor of expose_set_visible(false)

- fix: prevent window jump when dragging maximized windows

When dragging a maximized window, calculate the relative grab point
and maintain it during transition to normal size.

Calculate grab ratio (click position relative to maximized dimensions)
and apply it to restored window size, keeping the grab point under cursor.

Fixes both XDG shell and X11 windows, for pointer and touch input.

- fix: reposition window during top/left edge resize

Move window repositioning logic from button release to motion handler
for top and left edge resizes. This provides smooth, real-time window
repositioning as the user drags the edge, instead of jumping on release.

Changes:
- Pointer resize: reposition during motion (src/shell/grabs.rs)
- Touch resize: reposition during motion (src/shell/grabs.rs)
- Remove duplicate repositioning from release handlers
- Fix clippy warning in workspaces (remove unnecessary clone)

- fix: use requested size for touch resize positioning

Use last_window_size instead of geometry for calculating window position
during touch-based top/left edge resize to prevent positioning drift.

- style: UI refinements for dock, expose mode, and app switcher

- Expose mode: increased window border thickness from 10px to 20px for better visibility
- Dock: reduced icon size to 95px, smaller running indicators (5px radius, 90% opacity)
- Dock: increased handle spacing to 50px, reduced padding from 30px to 20px
- Dock: reduced lateral padding from 20px to 10px for better space utilization
- Dock: lighter border color (0.9, 0.9, 0.9, 0.5) for improved visibility
- Dock: fixed hide/show animations during fullscreen workspace transitions
- Dock: tooltip background changed to gray (RGB 157, 157, 157, full opacity)
- App switcher: increased icon size from 170px to 190px
- App switcher: increased selection rect by 7px for better visual feedback
- App switcher: made background more transparent (85% of original opacity)
- Fixed expose_update to be instant when not in expose mode to prevent animation conflicts

- feat: bump up smithay

- Revert "feat: bump up smithay"

This reverts commit 461b5357d508e01ea6d6af020238543ea470e391.

- doc: review doc files

- fix:  dock rendering

- cargo fmt

- feat: initial support for foreign toplevel protocol

- fix: better AGENT.md

- feat: apps-manager component init

- chore: initial protocol implementation layer protocol

- feat: initial protocol clients sample clients and system design

- fix: workspace + sclayer early init

- Merge pull request #13 from nongio/pr/batch-0-sc-layer-foundation

Pr/batch 0 sc layer foundation

- fix: dock scaling + config

- fix: ux style + ux improvement

- Add FontCache with make_font() and get_typeface() helper methods
- Enable subpixel rendering and antialiasing for all fonts
- Change window selector text from BOLD to SEMI_BOLD for better readability
- Update dock corner radius (to 1/3.5 height) and shadow styling
- Make bar_layer public in DockView
- Update workspace selector with improved corner radius (20px), caching, and shadows
- Change workspace label from 'Bench' to 'Workspace'
- Add background blur to workspace close button
- Adjust light theme colors: brighter blue accent and more visible primary fills
- Use theme shadow_color for consistent styling across all components
- Clean up app_switcher debug logs

- feat: add window-specific popup visibility control

- Add hide_popups_for_window() to hide all popups for a specific window
- Add show_popups_for_window() to show all popups for a specific window
- Integrate popup visibility with keyboard focus changes
- Hide popups when window loses focus, show when window gains focus
- Enables proper popup management during window switching

- feat: improve application info loading and icon fallback

- Fix desktop entry lookup to use exact filename matching (case-insensitive)
- Make desktop_entry optional in Application struct to prevent crashes
- Add comprehensive fallback icon support (application-default-icon or application-x-executable)
- Create fallback Application for apps without desktop entries with formatted display names
- Add PID-based app_id resolution for windows without XDG app_id
- Search desktop entries by examining Exec and TryExec fields
- Use display_app_id in workspaces for better app identification
- Add get_dock_height() and get_dock_geometry() helper methods
- Manage popup visibility on window activation
- Add extensive logging for debugging app icon and PID resolution
- Add unit tests for desktop entry matching logic
- Fix expose gesture swipe direction (invert delta for natural scrolling)

- feat: update sc-layer protocol implementation

- Add SetMasksToBounds request handler to control content clipping
- Fix SetCornerRadius to use BorderRadius::new_single for consistency
- Use actual dock geometry for maximize calculations instead of hardcoded values
- Pass DisplayHandle to Workspaces for PID-based app_id resolution
- Fix tracing env_filter to respect RUST_LOG environment variable

- Merge pull request #14 from nongio/fixes/improvements

Fixes/improvements

- feat: add session startup scripts

- Add modular session setup scripts (dbus, pipewire, portal, wifi, kwallet stub)
- Update start_session.sh to source individual setup scripts
- Add --debug flag support for detailed logging
- Add wifi auto-connect functionality
- Improve D-Bus session handling
- Add PipeWire service verification
- Add portal backend validation and startup
- Improve logging with colored output
- Use actual RUST_LOG environment variable

- feat(portal): Add compositor watchdog for health monitoring

Implements a watchdog service that periodically pings the compositor via
D-Bus to ensure it remains responsive. If the compositor becomes unresponsive
(exceeds max consecutive ping failures/timeouts), the watchdog will
terminate it using pkill.

Features:
- Configurable startup delay, ping interval, and timeout
- Configurable max consecutive failures before termination
- Automatic service wait with timeout on startup
- Structured logging of watchdog events
- Graceful compositor termination via pkill -9

Components:
- Watchdog module with async ping loop
- CompositorHealthInterface D-Bus service for ping/pong
- Integration into portal backend main loop

This helps prevent hung compositor states and ensures system reliability.

- feat(compositor): Track and apply layer shell exclusive zones

Implements proper exclusive zone tracking for layer shell surfaces to reserve
screen space on each output edge. This ensures that maximized windows and
other compositor features respect the space reserved by panels, docks, and
other layer shell surfaces.

Features:
- ExclusiveZones struct tracking reserved space per output edge
- recalculate_exclusive_zones() method to compute zones from layer surfaces
- Automatic recalculation on layer surface creation and destruction
- Configurable max limits per edge (top, bottom, left, right)
- Respects layer shell anchor and exclusive zone values
- Integration with window maximize to use available space

Changes:
- Added exclusive_zones HashMap to ScreenComposer state
- Layer surfaces trigger zone recalculation on map/unmap
- Maximize uses tracked zones instead of hardcoded top bar
- New config options for max exclusive zone limits
- Proper cloning to avoid borrow conflicts

This fixes the issue where maximized windows would overlap layer shell
surfaces like top bars and docks.

- feat: add configurable icon_theme option

- Add icon_theme field to Config struct (Option<String>)
- Implement find_icon_with_theme() helper in utils/mod.rs
  - Uses xdgkit to find icons with specified theme
  - Falls back to auto-detection when None
  - Auto-detects from system config (kdeglobals, gtk-3.0/4.0)
- Update apps_info.rs to use find_icon_with_theme()
- Update config example with commented icon_theme option

When icon_theme is omitted or None, system theme is auto-detected.
When set, uses the specified icon theme (e.g., 'WhiteSur', 'Papirus').

- fix: update puffin_http to 0.16 for compatibility with puffin 0.19

- Upgrade puffin_http from 0.13 to 0.16 to match puffin version used by profiling crate
- Uncomment .unwrap() on puffin server initialization to catch startup errors
- Fixes version mismatch causing puffin_viewer connection resets

- docs: add profiling section to README

- Add instructions for using puffin profiler
- Document how to install and connect puffin_viewer
- Note version compatibility requirements

- feat: add wlr-foreign-toplevel-management protocol support

Implement wlr-foreign-toplevel-management-v1 alongside existing
ext-foreign-toplevel-list-v1 for compatibility with rofi, waybar,
and other wlroots-based tools.

- Add unified ForeignToplevelHandles abstraction for both protocols
- Create wlr_foreign_toplevel handler module
- Update window creation to register with both protocols

- docs: add foreign toplevel management documentation

- docs: add dock migration strategy to foreign-toplevel

Add section describing migration path from built-in dock to external
standalone application using wlr-foreign-toplevel protocol

- feat: support monitor resolution and refresh rate from config

- chore: rendering metrics calculation

- fix: lighten window shadows to prevent excessive darkening when overlapping

- fix: layers visibility

- feat: animated window size and position

- fix: set WAYLAND_DISPLAY env variable

- fix: account for reserved areas when calculating new window position

- fix: session script start gtk portal

- fix: fullscreen direct scanout timing and workspace naming

- Add is_fullscreen_animating flag to track window animation state
- Check app_switcher visibility in is_fullscreen_and_stable()
- Clear animating flag when fullscreen animation completes
- Set workspace name to app display name during fullscreen
- Export ApplicationsInfo for consistent app metadata usage

- feat: smart window placement for fullscreen workspaces

- Same-app windows stay in fullscreen workspace (e.g., dialogs)
- Different-app windows redirect to previous workspace
- Direct scanout only when workspace has single window
- Add detailed logging for workspace redirection behavior

- fix: dock show/hide

- feat: improve natural layout with grid-based initial positioning

- fmt: suppress dead_code warnings for text style functions

- feat: add touchpad configuration options

- Add [input] config section with touchpad settings
- Support tap-to-click, tap-and-drag, tap-drag-lock
- Add touchpad click method (clickfinger/buttonareas) for 2-finger right-click
- Configure disable-while-typing, natural scroll, left-handed mode
- Add middle button emulation option
- Apply libinput device configuration on startup
- Update sc_config.example.toml with documented options

Settings map directly to libinput configuration API for compatibility.
Default: clickfinger mode (1-finger=left, 2-finger=right, 3-finger=middle)

- fix: prevent crash on window unmaximize

Use unwrap_or fallback when element_geometry returns None during
window unmaximization to prevent panic.

Also apply rustfmt to natural_layout.rs

- fix: fullscreen

- fmt

- Cargo + fmt

- Merge pull request #15 from nongio/services

Services

- feat(compositor): Track and apply layer shell exclusive zones

Implements proper exclusive zone tracking for layer shell surfaces to reserve
screen space on each output edge. This ensures that maximized windows and
other compositor features respect the space reserved by panels, docks, and
other layer shell surfaces.

Features:
- ExclusiveZones struct tracking reserved space per output edge
- recalculate_exclusive_zones() method to compute zones from layer surfaces
- Automatic recalculation on layer surface creation and destruction
- Configurable max limits per edge (top, bottom, left, right)
- Respects layer shell anchor and exclusive zone values
- Integration with window maximize to use available space

Changes:
- Added exclusive_zones HashMap to ScreenComposer state
- Layer surfaces trigger zone recalculation on map/unmap
- Maximize uses tracked zones instead of hardcoded top bar
- New config options for max exclusive zone limits
- Proper cloning to avoid borrow conflicts

This fixes the issue where maximized windows would overlap layer shell
surfaces like top bars and docks.

- Merge pull request #16 from nongio/feat/exclusive_zones

feat(compositor): Track and apply layer shell exclusive zones

- feat: add configurable icon_theme option

- Add icon_theme field to Config struct (Option<String>)
- Implement find_icon_with_theme() helper in utils/mod.rs
  - Uses xdgkit to find icons with specified theme
  - Falls back to auto-detection when None
  - Auto-detects from system config (kdeglobals, gtk-3.0/4.0)
- Update apps_info.rs to use find_icon_with_theme()
- Update config example with commented icon_theme option

When icon_theme is omitted or None, system theme is auto-detected.
When set, uses the specified icon theme (e.g., 'WhiteSur', 'Papirus').

- Merge pull request #17 from nongio/icon_theme

feat: add configurable icon_theme option

- fix: update puffin_http to 0.16 for compatibility with puffin 0.19

- Upgrade puffin_http from 0.13 to 0.16 to match puffin version used by profiling crate
- Uncomment .unwrap() on puffin server initialization to catch startup errors
- Fixes version mismatch causing puffin_viewer connection resets

- Merge pull request #18 from nongio/puffin_upgrade

fix: update puffin_http to 0.16 for compatibility with puffin 0.19

- docs: add profiling section to README

- Add instructions for using puffin profiler
- Document how to install and connect puffin_viewer
- Note version compatibility requirements

- feat: add wlr-foreign-toplevel-management protocol support

Implement wlr-foreign-toplevel-management-v1 alongside existing
ext-foreign-toplevel-list-v1 for compatibility with rofi, waybar,
and other wlroots-based tools.

- Add unified ForeignToplevelHandles abstraction for both protocols
- Create wlr_foreign_toplevel handler module
- Update window creation to register with both protocols

- docs: add foreign toplevel management documentation

- docs: add dock migration strategy to foreign-toplevel

Add section describing migration path from built-in dock to external
standalone application using wlr-foreign-toplevel protocol

- feat: support monitor resolution and refresh rate from config

- Merge pull request #19 from nongio/feat/foreign-protocol

Feat/foreign protocol

- chore: rendering metrics calculation

- Merge pull request #20 from nongio/metrics

chore: rendering metrics calculation

## [v0.10.0] - 2025-12-16


- Merge pull request #9 from nongio/bump_version

Bump version 0.9.0

- fix: agent instructions + CLAUDE.md symlink

- feat: xdg-desktop-portal for screencomposer

- fix: agents.md

- review: remove unused deps

- Update AGENTS.md

Co-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>

- Update components/xdg-desktop-portal-sc/src/portal/interface.rs

Co-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>

- Merge pull request #11 from nongio/xdg-desktop-portal-sc

Xdg desktop portal sc

- feat: screenshare fullscreen

- feat: session script for dbus and keyring

- feat: script for aumated testing

- Merge pull request #12 from nongio/screenshare-stacked

Screenshare stacked

- docs: update screenshare

- chore(release): v0.10.0

## [v0.9.0] - 2025-12-08


- update: refactor transitions

- wlcs wip

- chore: use rust 1.82.0

- feat: theme colors, text styles + config

- chore: fps_ticker as custom feature

- chore!: multiple workspaces

- refactor: split state in multiple files

- feat: multiple workspaces

- refactor and consolidate workspaces

- simplify dock minimise logic

- workspace selector draw fix

- chore: simplify renderer code

- chore: refactor workspaces data flow, dock, app_switcher

- adjust window selector style

- cargo fmt

- cargo fmt

- wip layer api upgrade

- restore functionality

- disable workspace change shortcut

- fix: texture loading

- tmp disable workspace switch keybinding

- feat: gate perf counters behind feature flag

- chore: run rustfmt on workspace modules

- cargo fmt

- update layers

- feat: enable debugger feature in default build

- Add debugger to default debug features
- This allows for better debugging capabilities during development

- feat: add scene snapshot functionality

- Add Alt+J keyboard shortcut to capture scene snapshots
- Serialize scene state to scene.json file
- Add SceneSnapshot action to input handling
- Import fs module for file operations
- Useful for debugging and development

- refactor: handle all workspace elements in rendering pipeline

- Update render functions to accept window_elements iterator instead of single space
- Modify output_elements to process all workspace windows
- Update post_repaint and take_presentation_feedback functions
- Apply changes across all backends (udev, winit, x11)
- Remove Space dependency from rendering pipeline
- Use spaces_elements() to get all windows from workspaces
- This enables proper multi-workspace rendering

- fix: improve workspace layout and sizing

- Add update_layout method to WorkspaceView for dynamic sizing
- Fix workspace positioning and dimensions based on screen size
- Add update_workspaces_layout to handle layout updates
- Set workspace layers to auto size initially
- Add proper scroll offset handling with apply_scroll_offset
- Make expose_layer private as it's only used internally
- Enable image caching on workspace layers
- Update layout on screen dimension changes and workspace operations

- perf: enable image caching for better performance

- Enable image caching for miniwindow icons in dock rendering
- Enable image caching for window view layers
- Comment out background image caching temporarily
- Add image_cached flag to window layer initialization
- These optimizations should improve rendering performance

- fix: add allow unsafe_code attribute for font initialization

- Add #[allow(unsafe_code)] attribute to get_paragraph_for_text function
- This suppresses warnings for necessary unsafe code in Skia text handling

- add AGENTS + docs

- docs

- Rename sample client to client-rainbow

- fix: workspace rendering

- feat: make keyboard shortcuts configurable

- Support backend-specific config overrides

- Add dock bookmark configuration and launcher plumbing

- clippy + fmt

- restore layers dep

- Merge pull request #5 from nongio/feat/dock-v2

Add dock bookmark configuration and launcher plumbing

- Merge branch 'main' into feat/keymap

- Merge pull request #4 from nongio/feat/keymap

feat: allow configuring keyboard shortcuts

- Add close-window shortcut action

- Merge pull request #6 from nongio/feat/close-window

Add configurable shortcut to close only the focused windowAdd close-window shortcut action

- feat: allow remapping modifiers and keysyms

- update example config

- key remap

- Merge pull request #7 from nongio/feat/key-remap

feat: allow remapping modifiers and keysyms

- feat: toggle maximize window

- feat: display config

- fix: dock + app switch theme

- fix: keyboard mappings

- fix: dock rendering colors

- fix: interaction bugs in dock

- cargo fmt

- add drag drop for window selector windows

- implement droptarget for window selector drag

- hide workspace remove button on current

- fix: expose show all

- disabled pinch gesture

- fix: prevent dragging fullscreen surfaces

- fix: workspace selector preview size

- fix: minimize windows

- fix: delete fullscreen workspace

- fix: reset focus on minimize window

- fix: genie effect glitches

- fix: on undo window drag/drop restore expose window sorting

- fix: when moving windows between workspaces ensure the expose is uptodate

- fix: workspace move indexing

- doc: AGENTS docs for expose feature

- fix: clean logs

- fix: opening appswitch should exist expose mode

- fix: popup surface commit / update

- feat: sample-clients for submenus

- fix: popups rendering

- feat: first implementation of wlr layers

- log: cleanup inative gpu logs

- doc: wlr layer shell 1.0

- feat: enable swipe workspace gesture

- feat: direct scanout for fullscreen windows in udev backend

- Skip scene rendering for fullscreen windows, render directly to output
- Add is_animating tracking for workspace transitions
- Add is_fullscreen_and_stable() and get_fullscreen_window() methods
- Track mode transitions with was_direct_scanout to reset buffers
- Filter post_repaint to only fullscreen window in direct scanout mode
- Reset buffers on fullscreen entry in xdg handler
- Add get_top_window_of_workspace() for focus management
- workspace_swipe_end() now returns target workspace index
- Add animation completion callback to clear is_animating flag

- fix: keyboard focus when switching workspaces

- Add set_current_workspace_index() method to ScreenComposer that handles focus
- Focus top window of target workspace, or clear focus if empty
- Update workspace switching in input_handler.rs to use new method
- Handle focus after workspace swipe gesture ends
- Update workspace_selector.rs to use centralized focus handling

- fix: crash on wlr delete

- fix: expose overlay opacity on first open

- 1.0.0 release

- fix: fmt

- fix: clippy

- pin layers version

- cargo fmt

- bump rust version

- ci fix cargo cache

- doc: README + docs file update

- pin version to 0.9.0 waiting until screenshare is ready

## [v0.2.0] - 2024-10-26


- init fork from anvil project

- update smithay/anvil

- init project to smallvil

- wlcs test suite

- refactor input handling

- udev backend skeleton

- fix linter warnings

- layers_renderer using skia on gles backend

- re-init from anvil-skia

- SceneElement using LayersEngine

- refactor scene_element

- app_switcher inital impplementation

- scene element draw refactor using library functions

- app switcher drawing

- app switcher keyboard focus and quit application

- fix app switcher view

- window/workspace view stub

- add debug text for background view

- window expose gestures stub

- skia renderer sync/fence

- window view draw / parenting fix

- output scale and window positioning fix

- add readme and version bump

- appswitcher supports multiple windows per app + async desktopentry load

- cargo fixes

- use bin pack to show all windows

- prevent click for being forwarded when show all windows

- cargo fix

- fix compile issues for xwayland

- refactor and optmisation of update loop

- fix binpacking window size

- support layers draw_content returning damage rect

- skia renderer import dmabuf

- fix window position rendering

- upgrade skia to 0.70.0

- fix clippy warning

- enable renderdoc on winit backend

- image util update

- refactor workspace views + interactive views

- change cursor

- update skia

- winit set named cursor

- set cursor udev

- remove bin pack dependency

- desktop show all

- re-enable app-switcher

- layers repo

- remove dead variables

- app switcher raise app windows

- x11 update

- update winit

- fix compilation skia_renderer

- fix udev

- fix state

- fix x11

- fix xdg shell

- fix grabs

- fix input_handler

- fix compilation errors

- add basic ci steps

- ci: fix build

- cargo fmt

- ci: remove msrv job

- cargo clippy --fix

- Implement skia fence logic

- cleanup vulkan code in udev

- restore pointerfocus on views

- restore interactive view

- restore on_frame event

- fix warnings

- winit refactor

- window surface rendering fix

- enforce client side decorations

- window maximize/unmaximize animation

- window_selector drawing scale

- is_resizing state pause window updates

- cursor scaling handling

- cargo update

- cargo fix

- config invert scroll

- command with args

- enable dnd composer icons

- println clean

- import dnd on udev

- Merge branch 'feat/dnd'

- rework on raise window and app_switcher state

- Cargo layers dependency refactor

- refactor quit appswitcher app logic

- throttle appswitcher events

- adjust appswitcher layout gap

- scaled appswitcher layout

- Merge branch 'fix/app_switcher_layout'

- appswitch use current screen size

- upgrade layers

- dock view stub

- fix skia version

- clippy

- Merge branch 'feat/dock'

- update README

- add MIT LICENSE

- cargo fmt

- fix smithay version and clippy warnings

- add credits and config info

- refactor workspace views name and pointer events

- layers debug shortcut

- fps counter only on debug

- refactor workspace, dock, add minimize windows stub

- use skia from layers

- windowview effect genie + image cache

- add minimezed windows to dock and animation

- refactor app switcher

- refactor window selector

- refactor windows positioning

- fix raise multiple windows order

- compositor mode config

- refactor scene damage tracking

- restore background round corners

- skiatexture image send/sync

- refactor dock + animations

- dock minimize animation fix

- cargo update deps and version bump

- config default, serializer and example

- apply configs
