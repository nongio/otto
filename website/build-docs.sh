#!/bin/bash
# Build both user guide and developer guide

SCRIPT_DIR="$(dirname "$0")"
DOCS_DIR="$SCRIPT_DIR/../docs"
OUTPUT_DIR="$SCRIPT_DIR/content"

# content/ only holds generated, gitignored files, so a fresh clone
# (e.g. CI) has no content/ directory at all.
mkdir -p "$OUTPUT_DIR"

# ============================================
# PAGE METADATA
# ============================================
# Search engines see two things per page: the <title> and the meta
# description. Neither can come from the document itself — an H1 like
# "Audio" is meaningless in a result list, and the first paragraph is
# written to be read after you have arrived, not before.
#
# So both are written here, keyed by output slug (developer pages are
# prefixed "dev/" because the two guides share slug names). Keep titles
# under ~60 characters and descriptions under ~160, or they get cut off
# mid-word in the result. A page missing an entry still builds; it just
# falls back to the generic site title and description.
declare -A PAGE_TITLE=(
    [readme]="Otto User Guide - Wayland Compositor Documentation"
    [getting-started]="Getting Started with Otto - Install and First Run"
    [desktop-tour]="Otto Desktop Tour - What Every Element Does"
    [window-management]="Window Management in Otto - Move, Resize, Tile"
    [workspaces]="Workspaces in Otto - Multiple Desktops on Wayland"
    [expose-and-switcher]="Expose and App Switcher - Otto Window Overview"
    [dock]="The Otto Dock - Running Apps, Bookmarks, Autohide"
    [topbar]="Otto Top Bar - Clock, Tray, and Global Menus"
    [dynamic-island]="Dynamic Island - Notifications and Live Activities"
    [side-canvas]="Side Canvas - A Column One Swipe Away"
    [keyboard-shortcuts]="Otto Keyboard Shortcuts - Syntax and Actions"
    [gestures]="Touchpad Gestures in Otto - Swipe and Pinch"
    [files]="Otto Files - File Manager, Thumbnails, Peek"
    [settings]="Otto Settings - Live Configuration Editor"
    [launcher]="Otto Launcher - Start Apps and Switch Windows"
    [emoji]="Otto Emoji Picker - Find and Type Any Emoji"
    [agents]="Ask and Agents in Otto - Run a Coding Agent from the Desktop"
    [configuration]="Configuring Otto - Config Files and How They Merge"
    [display]="Display Setup in Otto - Scaling and Multi-Monitor"
    [theming]="Theming Otto - Dark Mode, Accent, Fonts, Wallpaper"
    [customization]="Customizing Otto - Appearance Settings and What They Do"
    [input]="Input Settings in Otto - Keyboard, Touchpad, Pointer"
    [audio]="Audio in Otto - UI Sounds and Sound Themes"
    [power-management]="Power Management in Otto - Lid, Suspend, Clamshell"
    [night-shift]="Night Shift in Otto - Color Temperature at Night"
    [autostart]="Autostart in Otto - exec_once, XDG, and systemd"
    [clipboard]="Wayland Clipboard Persistence and History in Otto"
    [desktop-widgets]="How to Set Up eww Desktop Widgets on Wayland"
    [lock-screen]="Otto Lock Screen - Idle Lock and Fingerprint Unlock"
    [login-greeter]="Otto as a Login Greeter - greetd Setup"
    [screen-sharing]="Screen Sharing in Otto - Portal, OBS, Browsers"
    [privacy]="Privacy in Otto - What Apps Were Allowed"
    [remote-desktop]="Remote Desktop for Otto - RDP and Virtual Outputs"
    [troubleshooting]="Troubleshooting Otto - Logs and Common Failures"
    [bug-reports]="Reporting a Bug in Otto - otto-msg doctor"
    [credits]="Credits - Wallpapers, Icons and Fonts in Otto's Screenshots"
    [dev/readme]="Otto Developer Guide - Architecture Overview"
    [dev/project-structure]="Otto Project Structure - Crates and Building"
    [dev/architecture]="Otto Architecture Rules - Layers and Boundaries"
    [dev/rendering]="Otto Rendering Pipeline - Scene Graph to Skia"
    [dev/render_loop]="Otto Render Loop - Scheduling and Damage Tracking"
    [dev/wayland]="Wayland Protocols in Otto - Globals, Versions, Handlers"
    [dev/scene-graph]="The Otto Scene Graph - Layer Tree and KMS Planes"
    [dev/layers]="Layers in Otto - Properties, Transactions, Caching"
    [dev/drm_plane]="DRM Planes in Otto - Hardware Scanout"
    [dev/dock-design]="Otto Dock Internals - Data Flow and Magnification"
    [dev/expose]="Expose Internals in Otto - Layout and Mirrors"
    [dev/side-canvas]="Side Canvas Internals - Protocol, Scene and Edge Swipe"
    [dev/window-move]="Interactive Window Moves in Otto"
    [dev/foreign-toplevel]="Foreign Toplevel - Exposing Otto's Window List"
    [dev/screenshare]="Screen Capture Internals - Portal and PipeWire"
    [dev/color-scheme-setting]="Color Scheme - Telling Apps Light or Dark"
    [dev/settings-dbus-api]="org.otto.Settings D-Bus API Reference"
    [dev/rdp-virtual-output]="RDP Bridge and Virtual Outputs in Otto"
    [dev/remote-desktop-indicator]="Remote Desktop Indicator in Otto"
    [dev/scroll-pane]="Otto Scroll Panes - Scrolling by Moving Surfaces"
    [dev/tiling-plan]="Otto Tiling - What Is Built and What Is Planned"
    [dev/otto-kit-roadmap]="otto-kit Roadmap - UI Toolkit Gap Analysis"
    [dev/otto-kit-library]="otto-kit Library - Every Component, Pictured"
    [dev/surface-style-protocol]="Surface Style Protocol"
    [dev/screenshot-plan]="Screenshot Portal Plan (Not Implemented)"
    [dev/search-next-plan]="Search Next - Requirements for Otto's File Search"
    [dev/airplay-screenshare]="AirPlay Screencast Exploration in Otto"
    [tiling]="Tiling in Otto - Tiled Workspaces on Wayland"
    [files-command-palette]="Otto Files Command Palette - Run Any Command"
    [files-custom-commands]="Custom Commands in Otto Files - Script Its Menus"
    [files-pdf-command]="PDF from Pictures - An Otto Files Script"
    [emoji]="Otto Emoji Picker - Find and Type Any Emoji"
    [agents]="Ask and Agents in Otto - Run a Coding Agent from the Desktop"
    [accessibility]="Accessibility in Otto - Screen Readers and Keyboard Use"
    [desktop-standards]="Otto Open Standards - XDG Specs and Wayland Protocols"
    [scripting]="Scripting Otto - otto-msg and i3 Commands"
    [dev/file-previews]="Otto File Previews - Thumbnails and Peek"
    [dev/file-icons]="Otto File Icons - Types, Themes, Thumbnails"
    [dev/peek-ocr]="Text in Pictures - OCR in Otto Files"
    [dev/file-search]="Otto File Search - Find, Recent, and the Index"
    [dev/otto-media-kit]="otto-media-kit - Video Playback for Otto Apps"
    [dev/shell-dbus-api]="org.otto.Shell1 D-Bus API Reference"
    [dev/xdg-specifications]="XDG Standards in Otto - What Is Implemented"
    [dev/text-rendering]="Text Rendering in Otto - Fontconfig, Subpixel, Toolkits"
    [dev/agents]="Otto Agents - Coding Agents Behind the Launcher"
    [dev/accessibility]="Otto Accessibility Internals - AT-SPI and Key Grabs"
    [dev/debug-action-hook]="Otto Debug Action Hook - Drive Actions From a Script"
    [dev/components]="Otto Components - Every Crate by Layer"
    [dev/otto-msg]="otto-msg - Drive the Compositor from a Script"
    [dev/otto-rdp]="otto-rdp - The RDP Bridge for an Output"
    [dev/otto-auth-ui]="otto-auth-ui - The Shared Password Panel"
    [dev/otto-authorize]="otto-authorize - The Session's polkit Agent"
    [dev/otto-lock]="otto-lock - The Screen Locker"
    [dev/otto-greeter]="otto-greeter - The Login Screen"
    [dev/otto-agents]="otto-agents - The Agent Service"
    [dev/otto-agents-client]="otto-agents-client - The Wire to the Agent Service"
    [dev/otto-agents-kit]="otto-agents-kit - Shared Parts of the Agent UIs"
    [dev/otto-canvas]="otto-canvas - The Agents Panel in the Side Canvas"
    [dev/otto-stash]="otto-stash - Collecting Things to Ask About"
    [dev/otto-bar]="otto-bar - The Top Menu Bar"
    [dev/otto-settings]="otto-settings - The Settings App"
    [dev/otto-emoji]="otto-emoji - The Emoji Picker"
    [dev/otto-dictate]="otto-dictate - Dictation as an Input Method"
    [dev/otto-md-kit]="otto-md-kit - Markdown Read as a Document"
    [dev/otto-pdf]="otto-pdf - Editing PDFs in Place Through PDFium"
    [dev/otto-peek]="otto-peek - The Space-Bar Preview, Embedded"
    [dev/otto-preview]="otto-preview - Studio, One File in a Window"
    [dev/otto-search]="otto-search - One Query Language Over the File Index"
)

declare -A PAGE_DESC=(
    [readme]="How to use Otto, a Wayland compositor with a Skia-rendered desktop: windows, workspaces, expose, the dock, and every configuration option."
    [getting-started]="Install Otto, choose a backend (DRM, winit or X11), start your first session, and work through the first-run checklist."
    [desktop-tour]="A guided tour of the Otto desktop: the top bar, dock, dynamic island, workspace selector and window decorations, and what each one does."
    [window-management]="Move, resize, maximize, minimize, tile and fullscreen windows in Otto, and control where newly opened windows are placed."
    [workspaces]="Create and switch workspaces, move windows between them, and run independent per-monitor workspaces in Otto."
    [expose-and-switcher]="See every open window at once with Expose, and switch between applications from the keyboard with Otto's app switcher."
    [dock]="Otto's compositor-drawn dock: pinned bookmarks, running apps, minimized windows, magnification, autohide and screen position."
    [topbar]="Configure Otto's top bar: the clock, system tray icons, and the global application menu bar that apps export."
    [side-canvas]="The side canvas slides in from the right edge of the screen with a two-finger swipe and holds panels that applications place in it."
    [dynamic-island]="Otto's dynamic island collects notifications, media controls, system HUDs and permission dialogs into one adaptive surface."
    [keyboard-shortcuts]="Every keyboard action Otto can bind, the modifier syntax it accepts, and how to override or remove the defaults."
    [gestures]="Three-finger swipes to change workspaces and a four-finger pinch for Expose, plus how to tune touchpad gestures in Otto."
    [files]="Browse and manage files with Otto Files: column and grid views, thumbnails, file operations, Peek and the file picker."
    [settings]="Change Otto's configuration while it runs: display arrangement, keyboard shortcuts, appearance and input, all applied live."
    [launcher]="Start applications and jump to open windows from the keyboard with Otto's launcher, including search and result ordering."
    [emoji]="Search or browse every emoji by category with Otto's picker and have it typed into the focused window, with a skin-tone setting."
    [agents]="Run a coding agent from the Otto desktop with Ask: what to install, what each agent can do, permission modes and questions."
    [configuration]="Where Otto's TOML configuration files live, the order they merge in, and how per-backend overrides work."
    [display]="Set resolution, refresh rate and fractional scaling, arrange multiple monitors, and create virtual outputs in Otto."
    [theming]="Set Otto's light or dark scheme, accent color, fonts, wallpaper, cursor and icon themes, and how those reach GTK and Qt apps."
    [customization]="What Otto's appearance settings do, shown on six configurations: accent colour, dock position and tint, corner radius, icon theme and window controls."
    [input]="Configure keyboard layout and repeat rate, touchpad tap and scroll behaviour, and pointer acceleration in Otto."
    [audio]="Enable or replace Otto's interface sound effects, and point the compositor at a different freedesktop sound theme."
    [power-management]="Control what Otto does on lid close and power button, configure idle suspend, and run reliably in clamshell mode."
    [night-shift]="Warm the display's color temperature on a schedule and control screen brightness with Otto's built-in night shift."
    [autostart]="Start programs with your Otto session using exec_once, XDG autostart entries, or systemd's graphical-session target."
    [clipboard]="Why Wayland clipboard contents vanish when an app closes, and which clipboard manager to run with Otto so they persist."
    [desktop-widgets]="Set up eww desktop widgets on Otto step by step: install it, build a first widget, copy a complete system HUD config, and fix the common problems."
    [lock-screen]="Lock your Otto session, set idle auto-lock, and configure PAM for password or fingerprint unlock."
    [login-greeter]="Use Otto as your login screen with greetd, including session selection, autologin and appearance."
    [screen-sharing]="Share your screen from Otto: xdg-desktop-portal setup, capture in browsers and OBS, AirPlay, and taking screenshots."
    [privacy]="See and take back what apps were allowed to do in Otto, remember a screen share, and when Otto asks for your password."
    [remote-desktop]="Serve an Otto session over RDP with otto-rdp, create virtual outputs, and connect from Windows, macOS or mobile clients."
    [troubleshooting]="Find Otto's logs, diagnose the most common startup and rendering failures, and gather what a useful bug report needs."
    [bug-reports]="Write a bug report with otto-msg doctor: gather the system, graphics, screens, configuration and session logs, choose and preview what to share, redact personal details, and attach one file."
    [credits]="Who made the wallpapers, icons and fonts you see in Otto's screenshots, and the licences they are used under."
    [dev/readme]="How Otto is built: Smithay for Wayland, Skia for drawing, and a retained lay-rs scene graph - plus where to start reading the source."
    [dev/project-structure]="Where everything lives in the Otto repository, what each Cargo feature flag turns on, and how to build the workspace."
    [dev/architecture]="The crate layers and compositor module boundaries Otto enforces, where each rule lives and how CI checks it."
    [dev/rendering]="How Otto turns its scene graph into render elements, draws them with Skia, and submits finished frames to the display."
    [dev/render_loop]="When Otto wakes up, when it decides to render, and how damage tracking limits each frame to what actually changed."
    [dev/wayland]="Every Wayland global Otto serves, with its version, who may bind it and the code that handles it, plus how protocol dispatch is wired."
    [dev/scene-graph]="How Otto's layer tree is shaped, how Wayland surfaces enter it, and when a subtree is promoted to a KMS hardware plane."
    [dev/layers]="The unit Otto's scene tree is built from: layer properties, content closures, animation transactions, caching and damage."
    [dev/drm_plane]="Handing parts of the scene to display hardware instead of the GPU: plane selection, format filtering and per-frame validation."
    [dev/dock-design]="How Otto's compositor-drawn dock is built: where its data comes from, how its layers are arranged, and the magnification animation."
    [dev/side-canvas]="How Otto's side canvas is built: the otto-canvas-v1 protocol, its place in the overlay plane, the slide animation and the touchpad edge swipe."
    [dev/expose]="How Otto's all-windows overview works: layout maths, window mirrors, drag-and-drop, and behaviour across multiple outputs."
    [dev/window-move]="How Otto implements interactive window drags, from the pointer grab through to the animation that settles the window."
    [dev/foreign-toplevel]="How Otto publishes its window list to taskbars, docks and launchers through the foreign-toplevel Wayland protocols."
    [dev/screenshare]="Otto's screen capture architecture: the xdg-desktop-portal backend, PipeWire streams, wlr-screencopy and single-window capture."
    [dev/color-scheme-setting]="How Otto tells GTK and Qt applications whether the desktop is currently in light or dark mode."
    [dev/settings-dbus-api]="The wire contract for Otto's settings service: the interfaces, properties and signals on org.otto.Settings."
    [dev/rdp-virtual-output]="How otto-rdp serves a virtual output over RDP: creating the output, encoding frames, and mapping input back into the session."
    [dev/remote-desktop-indicator]="The sharing indicator otto-rdp publishes while a remote client is watching the session, and how the compositor draws it."
    [dev/scroll-pane]="Scrolling an otto-kit application at display rate by moving subsurfaces instead of repainting, and the layouts that go with it."
    [dev/tiling-plan]="Otto's tiling window management: the tree, the command language, decorations and drag behaviour — what is built, and what is still planned."
    [dev/otto-kit-library]="Every UI component in otto-kit with a picture of it, what it is for, how it is driven and which crates use it."
    [dev/otto-kit-roadmap]="What Otto's UI toolkit provides today, what is still missing, and the planned direction. Partially built - read it as a plan."
    [dev/surface-style-protocol]="otto-surface-style-unstable-v1: how a client styles and animates its own surface through the compositor's scene graph, and why the protocol looks the way it does."
    [dev/search-next-plan]="Requirements for reworking Otto's file search into one local service that finds files by meaning, words and filters.. A draft."
    [dev/screenshot-plan]="A design for screenshot support through the desktop portal. Not implemented - a proposal rather than documentation."
    [dev/airplay-screenshare]="Notes from validating AirPlay as a screencast target for Otto. An exploration, not a shipped feature."
    [tiling]="Switch any Otto workspace between stacking and tiling: build a layout from the keyboard or the pointer, set gaps and tile decorations, and drive it from scripts."
    [files-command-palette]="Press Ctrl+P in Otto Files to run any command by name, select files by pattern, and rename many files at once from one pattern with a live dry run."
    [files-custom-commands]="Add your own commands to Otto Files with a script: a tutorial, the JSON a script speaks, previews, undo and translations."
    [files-pdf-command]="Build a Files command that turns pictures into a PDF, reads their text with tesseract, and reports a page at a time while it works."
    [emoji]="Search or browse every emoji by category with Otto's picker and have it typed into the focused window, with a skin-tone setting."
    [agents]="Run a coding agent from the Otto desktop with Ask: what to install, what each agent can do, permission modes and questions."
    [accessibility]="Use Otto with a screen reader or the keyboard alone: what to install, what the desktop announces, and how to drive its applications."
    [desktop-standards]="Every freedesktop.org standard and Wayland protocol Otto supports: desktop entries, icon themes, the trash, portals, fonts, panels, clipboard and screen capture."
    [scripting]="Drive Otto from a script with otto-msg: run i3-style window and tiling commands, read the window tree as JSON, and follow what the compositor is doing."
    [dev/file-previews]="How Otto Files builds previews: the thumbnail store, the shared cache, the preview column, Peek's sandboxed decode worker, and video playback."
    [dev/file-icons]="What goes in the icon box in Otto Files: MIME detection without a cache file, the icon-name chain, theme resolution, and animated previews."
    [dev/peek-ocr]="How Otto reads the words in a picture: an external recogniser in the decode worker's sandbox, the word cache, selection and copy, and searching what it found."
    [dev/file-search]="How Otto Files finds a file: the Find strip, the Recent listing, and the one index both of them ask over D-Bus."
    [dev/otto-media-kit]="The crate behind video in Otto Files: an embeddable player, its GStreamer worker, the frame ring they share, and how it is contained."
    [dev/shell-dbus-api]="The wire contract for org.otto.Shell1: running window and tiling commands, reading the tree, and the events a status bar can subscribe to."
    [dev/xdg-specifications]="The freedesktop and XDG file and D-Bus standards Otto implements, the services it uses, and where in the source each one lives."
    [dev/text-rendering]="How Otto rasterises its own text with fontconfig, where subpixel text applies, and the font settings it hands to GTK, Qt and X11 apps."
    [dev/agents]="How Otto runs coding agents: the otto-agents service, the Agent Host and Agent Client protocols, the launcher as a client, and where a permission question goes."
    [dev/accessibility]="How Otto exposes an AT-SPI tree through AccessKit, and the key grabs a screen reader needs from the compositor."
    [dev/debug-action-hook]="Trigger any builtin Otto shortcut action from a script through $OTTO_ACTION_FILE, for testing and automation."
    [dev/components]="Every crate in the Otto workspace, grouped by architecture layer, each linked to a page that explains how it is built."
    [dev/otto-msg]="How otto-msg drives Otto from a script: i3-syntax commands, the window tree and workspaces as JSON, and events, over the compositor's org.otto.Shell1 D-Bus service."
    [dev/otto-rdp]="How otto-rdp serves an Otto output over RDP: PipeWire capture, GPU H.264 through GStreamer, ironrdp-server, and remote input through virtual pointer and keyboard."
    [dev/otto-auth-ui]="How Otto draws one password panel for the login screen, the lock screen and the polkit dialog, with PAM and logind calls kept off the drawing thread."
    [dev/otto-authorize]="How Otto's polkit agent works: polkitd's requests on a GLib thread, the password dialog on a layer-shell overlay, and polkit's own helper checking the answer."
    [dev/otto-lock]="How Otto's screen locker works: started by the compositor, it takes the lock with ext-session-lock-v1, draws a panel per output and unlocks only on PAM's word."
    [dev/otto-greeter]="How Otto's login screen works: greetd runs Otto in login mode, the greeter relays greetd's PAM conversation over its socket, and greetd starts the session."
    [dev/otto-agents]="How Otto runs coding agents: otto-agents serves ACP agent sessions to every client over the Agent Host Protocol on a Unix socket, with its modules and commands."
    [dev/otto-agents-client]="How Otto's agent UIs reach otto-agents: the socket and its URLs, the AHP transport over a Unix socket, session URIs and file URIs, shared by service and clients."
    [dev/otto-agents-kit]="How Otto's agent UIs share code: the session feed, the Ask chat and its log, list keys and dictation, run in each host on a connection thread that wakes its loop."
    [dev/otto-canvas]="How Otto's side canvas lists agent sessions: otto-canvas places one item through otto-canvas-v1, connects only while shown and hands work to the launcher."
    [dev/otto-stash]="How Otto's stash works: one process gathers selections, files and screen regions into a card, serves org.otto.Stash1, and hands everything to Ask."
    [dev/otto-bar]="How Otto's top bar works: three layer surfaces, the tray watcher and app-menu registrar it serves, and the watchers that follow the compositor, settings and power."
    [dev/otto-settings]="How Otto's Settings app works: a settings store filled from org.otto.Settings, and panes that reach AccountsService, pactl, the permission store, LocalSearch and TOML files."
    [dev/otto-emoji]="How Otto's emoji picker works: a short-lived card on an overlay layer surface, then the pick typed through a virtual keyboard, pasted or copied to the window that had focus."
    [dev/otto-dictate]="How Otto's dictation proof of concept works: a Wayland input method fed by a toggle socket and PipeWire capture, with recognition from a local speech server."
    [dev/otto-md-kit]="How Otto reads Markdown as a document: a small parser that turns untrusted text into the toolkit's blocks, for Peek's sandboxed worker and the agents' chat."
    [dev/otto-pdf]="How Otto fills PDF forms and marks them up: PDFium loaded at run time, one thread that owns it, page coordinates, and atomic saves over the file."
    [dev/otto-peek]="How Otto's Peek preview is built: a session hosts embed, a sandboxed decode worker that is the host re-executed, external rasterisers, and shared thumbnails."
    [dev/otto-preview]="How Studio (otto-preview) works: one instance on org.otto.Preview1, a Peek session in a window, PDF editing via otto-pdf, and an agent chat with MCP tools."
    [dev/otto-search]="How Otto's file search library works: a query language parsed into a plan, SPARQL sent to LocalSearch over D-Bus, and results rechecked against the disk."
)

# Pages with a screenshot worth using as the social-card image.
declare -A PAGE_IMAGE=(
    [desktop-widgets]="images/desktop-widgets.jpg"
    [customization]="images/rice-deep-field.jpg"
    [desktop-tour]="images/desktop-tour.jpg"
    [expose-and-switcher]="images/expose.jpg"
    [workspaces]="images/workspaces.jpg"
    [tiling]="images/tiling.jpg"
    [files]="images/files-icon-view.jpg"
    [dock]="images/dock.jpg"
    [lock-screen]="images/lock-screen.jpg"
    [login-greeter]="images/login-greeter.jpg"
)

# Old URLs for pages that have since been renamed, so existing links and search
# results keep working. Hugo serves each alias as a redirect to the new page.
declare -A PAGE_ALIAS=(
    [dev/surface-style-protocol]="/developer/sc-layer-protocol-design/"
)

# Emit the metadata front-matter lines for one page, given its metadata key.
emit_meta() {
    local key="$1"
    [ -n "${PAGE_TITLE[$key]:-}" ] && echo "page_title: \"${PAGE_TITLE[$key]}\""
    [ -n "${PAGE_DESC[$key]:-}" ]  && echo "description: \"${PAGE_DESC[$key]}\""
    [ -n "${PAGE_IMAGE[$key]:-}" ] && echo "image: \"${PAGE_IMAGE[$key]}\""
    [ -n "${PAGE_ALIAS[$key]:-}" ] && printf 'aliases:\n  - "%s"\n' "${PAGE_ALIAS[$key]}"
    return 0
}

# ============================================
# USER GUIDE (one Hugo page per doc, not concatenated)
# ============================================
rm -f "$OUTPUT_DIR"/*.md

USER_FILES=(
    "user/README.md"
    "user/getting-started.md"
    "user/desktop-tour.md"
    "user/window-management.md"
    "user/tiling.md"
    "user/workspaces.md"
    "user/expose-and-switcher.md"
    "user/dock.md"
    "user/topbar.md"
    "user/dynamic-island.md"
    "user/side-canvas.md"
    "user/keyboard-shortcuts.md"
    "user/gestures.md"
    "user/files.md"
    "user/files-command-palette.md"
    "user/files-custom-commands.md"
    "user/files-pdf-command.md"
    "user/settings.md"
    "user/launcher.md"
    "user/emoji.md"
    "user/agents.md"
    "user/configuration.md"
    "user/recommended-settings.md"
    "user/display.md"
    "user/theming.md"
    "user/customization.md"
    "user/input.md"
    "user/audio.md"
    "user/power-management.md"
    "user/night-shift.md"
    "user/autostart.md"
    "user/clipboard.md"
    "user/desktop-widgets.md"
    "user/desktop-standards.md"
    "user/lock-screen.md"
    "user/login-greeter.md"
    "user/screen-sharing.md"
    "user/privacy.md"
    "user/remote-desktop.md"
    "user/accessibility.md"
    "user/scripting.md"
    "user/troubleshooting.md"
    "user/bug-reports.md"
    "user/credits.md"
)

echo "Building User Guide..."
for file in "${USER_FILES[@]}"; do
    filepath="$DOCS_DIR/$file"
    if [ ! -f "$filepath" ]; then
        echo "⚠ Warning: $file not found"
        continue
    fi

    base="$(basename "$file" .md)"
    slug="$(echo "$base" | tr '[:upper:]' '[:lower:]')"
    title="$(sed -n '1s/^# //p' "$filepath")"
    # README.md is the guide's landing page, served at "/"; everything
    # else gets its own top-level page at "/<slug>/".
    if [ "$slug" = "readme" ]; then
        outfile="$OUTPUT_DIR/_index.md"
    else
        outfile="$OUTPUT_DIR/$slug.md"
    fi

    {
        echo "---"
        echo "title: \"$title\""
        emit_meta "$slug"
        if [ "$slug" != "readme" ]; then
            echo 'layout: "doc"'
        fi
        echo "---"
        echo ""
        tail -n +2 "$filepath"
    } > "$outfile"
done

# docs/user/*.md link to sibling files (for GitHub rendering); each
# page is its own Hugo page here, so rewrite those into real page
# links instead of leaving the raw ".md" filename in the href.
perl -i -pe '
    s{\]\(README\.md\)}{](/)}gi;
    s{\]\(images/([A-Za-z0-9_.-]+)\)}{](/images/$1)}g;
    s{\]\(\.\./developer/README\.md\)}{](/developer/)}gi;
    s{\]\(\.\./developer/([A-Za-z0-9_-]+)\.md#([^)]+)\)}{"](/developer/" . lc($1) . "/#$2)"}ge;
    s{\]\(\.\./developer/([A-Za-z0-9_-]+)\.md\)}{"](/developer/" . lc($1) . "/)"}ge;
    s{\]\(([a-zA-Z0-9_-]+)\.md#([a-zA-Z0-9_-]+)\)}{](/$1/#$2)}g;
    s{\]\(([a-zA-Z0-9_-]+)\.md\)}{](/$1/)}g;
' "$OUTPUT_DIR"/*.md

# Screenshots referenced as images/<name>.png above are served from the
# static dir, the same way the developer guide's diagrams are.
IMAGE_SRC="$DOCS_DIR/user/images"
if [ -d "$IMAGE_SRC" ]; then
    mkdir -p "$SCRIPT_DIR/assets/images"
    # Screenshots are JPEG: as PNGs these were 1-2 MB each, which is the
    # page's largest paint and its slowest one.
    # .gif is here for the short screen recordings the guide embeds: an <img>
    # plays them on the site and on GitHub, where a <video> plays on neither.
    cp "$IMAGE_SRC"/*.jpg "$IMAGE_SRC"/*.png "$IMAGE_SRC"/*.gif "$SCRIPT_DIR/assets/images/" 2>/dev/null
fi

# ============================================
# DEVELOPER GUIDE
# ============================================
# Same shape as the user guide: one Hugo page per doc, so each page gets its
# own table of contents. Concatenating them into a single page produced one
# enormous TOC that was unusable once you started scrolling.
# These live under /developer/ rather than at the top level, so the guides
# keep separate URL namespaces and /developer/ stays a valid landing page.
DEV_OUT="$OUTPUT_DIR/developer"
rm -rf "$DEV_OUT"
mkdir -p "$DEV_OUT"

DEVELOPER_FILES=(
    "developer/README.md"
    "developer/project-structure.md"
    "developer/architecture.md"
    "developer/rendering.md"
    "developer/render_loop.md"
    "developer/wayland.md"
    "developer/xdg-specifications.md"
    "developer/text-rendering.md"
    "developer/scene-graph.md"
    "developer/layers.md"
    "developer/otto-kit.md"
    "developer/otto-kit-library.md"
    "developer/scroll-pane.md"
    "developer/file-previews.md"
    "developer/file-icons.md"
    "developer/peek-ocr.md"
    "developer/file-search.md"
    "developer/otto-media-kit.md"
    "developer/drm_plane.md"
    "developer/vulkan-renderer.md"
    "developer/dock-design.md"
    "developer/expose.md"
    "developer/side-canvas.md"
    "developer/window-move.md"
    "developer/foreign-toplevel.md"
    "developer/screenshare.md"
    "developer/color-scheme-setting.md"
    "developer/settings-dbus-api.md"
    "developer/shell-dbus-api.md"
    "developer/compositor-dbus-api.md"
    "developer/rdp-virtual-output.md"
    "developer/remote-desktop-indicator.md"
    "developer/agents.md"
    "developer/accessibility.md"
    "developer/debug-action-hook.md"
    "developer/versioning.md"
    "developer/otto-kit-roadmap.md"
    "developer/surface-style-protocol.md"
    "developer/screenshot-plan.md"
    "developer/tiling-plan.md"
    "developer/search-next-plan.md"
    "developer/airplay-screenshare.md"
    "developer/permissions.md"
    "developer/vulkan-renderer-plan.md"
    "developer/otto-render.md"
    "developer/otto-foundations.md"
    "developer/otto-dbus.md"
    "developer/otto-files.md"
    "developer/otto-launcher.md"
    "developer/otto-islands.md"
    "developer/xdg-desktop-portal-otto.md"
    "developer/components.md"
    "developer/otto-msg.md"
    "developer/otto-rdp.md"
    "developer/otto-auth-ui.md"
    "developer/otto-authorize.md"
    "developer/otto-lock.md"
    "developer/otto-greeter.md"
    "developer/otto-agents.md"
    "developer/otto-agents-client.md"
    "developer/otto-agents-kit.md"
    "developer/otto-canvas.md"
    "developer/otto-stash.md"
    "developer/otto-bar.md"
    "developer/otto-settings.md"
    "developer/otto-emoji.md"
    "developer/otto-dictate.md"
    "developer/otto-md-kit.md"
    "developer/otto-pdf.md"
    "developer/otto-peek.md"
    "developer/otto-preview.md"
    "developer/otto-search.md"
)

echo "Building Developer Guide..."
for file in "${DEVELOPER_FILES[@]}"; do
    filepath="$DOCS_DIR/$file"
    if [ ! -f "$filepath" ]; then
        echo "⚠ Warning: $file not found"
        continue
    fi

    base="$(basename "$file" .md)"
    slug="$(echo "$base" | tr '[:upper:]' '[:lower:]')"
    title="$(sed -n '1s/^# //p' "$filepath")"
    # README.md is the guide's landing page at "/developer/"; every other
    # doc gets its own page at "/developer/<slug>/".
    if [ "$slug" = "readme" ]; then
        outfile="$DEV_OUT/_index.md"
    else
        outfile="$DEV_OUT/$slug.md"
    fi

    {
        echo "---"
        echo "title: \"$title\""
        emit_meta "dev/$slug"
        echo 'layout: "doc"'
        echo "---"
        echo ""
        tail -n +2 "$filepath"
    } > "$outfile"
done

# docs/developer/*.md link to sibling files, to diagrams, and out into the
# repo (../../specs, ../../protocols) so they render on GitHub. Rewrite each
# kind for the site: siblings become real page links, diagrams point at the
# static dir, and repo paths become GitHub links since they have no page here.
perl -i -pe '
    s{\]\(README\.md\)}{](/developer/)}gi;
    s{\]\(diagrams/([A-Za-z0-9_-]+\.svg)\)}{](/diagrams/$1)}g;
    s{\]\(images/([A-Za-z0-9_./-]+\.png)\)}{](/developer-images/$1)}g;
    s{\]\(\.\./user/README\.md\)}{](/)}gi;
    s{\]\(\.\./user/([A-Za-z0-9_-]+)\.md#([^)]+)\)}{"](/" . lc($1) . "/#$2)"}ge;
    s{\]\(\.\./user/([A-Za-z0-9_-]+)\.md\)}{"](/" . lc($1) . "/)"}ge;
    s{\]\(\.\./\.\./([^)]*/)\)}{](https://github.com/nongio/otto/tree/main/$1)}g;
    s{\]\(\.\./\.\./([^)]+)\)}{](https://github.com/nongio/otto/blob/main/$1)}g;
    s{\]\(([A-Za-z0-9_-]+)\.md#([^)]+)\)}{"](/developer/" . lc($1) . "/#$2)"}ge;
    s{\]\(([A-Za-z0-9_-]+)\.md\)}{"](/developer/" . lc($1) . "/)"}ge;
' "$DEV_OUT"/*.md

# The diagrams are referenced as /diagrams/<name>.svg above; Hugo serves
# assets/ as the static dir, and canonifyURLs resolves the root-relative
# path against the real baseURL.
DIAGRAM_SRC="$DOCS_DIR/developer/diagrams"
if [ -d "$DIAGRAM_SRC" ]; then
    mkdir -p "$SCRIPT_DIR/assets/diagrams"
    cp "$DIAGRAM_SRC"/*.svg "$SCRIPT_DIR/assets/diagrams/" 2>/dev/null
fi

# Pictures in docs/developer/images/ (the otto-kit library's, one folder per
# set) are referenced as /developer-images/<path>.png above, kept apart from
# the user guide's /images/ so the two sets cannot collide.
DEV_IMAGE_SRC="$DOCS_DIR/developer/images"
if [ -d "$DEV_IMAGE_SRC" ]; then
    DEV_IMAGE_OUT="$SCRIPT_DIR/assets/developer-images"
    rm -rf "$DEV_IMAGE_OUT"
    find "$DEV_IMAGE_SRC" -name '*.png' | while read -r src; do
        rel="${src#"$DEV_IMAGE_SRC"/}"
        mkdir -p "$DEV_IMAGE_OUT/$(dirname "$rel")"
        cp "$src" "$DEV_IMAGE_OUT/$rel"
    done
fi

echo "✓ Built User Guide: $OUTPUT_DIR/_index.md"
echo "✓ Built Developer Guide: $DEV_OUT/ ($(ls "$DEV_OUT" | wc -l) pages)"

# The pages exist now, so the search index can be built from them.
"$SCRIPT_DIR/build-search.sh"
echo "✓ Built search index: $SCRIPT_DIR/assets/pagefind/"
