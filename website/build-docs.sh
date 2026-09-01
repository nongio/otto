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
    [keyboard-shortcuts]="Otto Keyboard Shortcuts - Syntax and Actions"
    [gestures]="Touchpad Gestures in Otto - Swipe and Pinch"
    [files]="Otto Files - File Manager, Thumbnails, Quick View"
    [settings]="Otto Settings - Live Configuration Editor"
    [launcher]="Otto Launcher - Start Apps and Switch Windows"
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
    [remote-desktop]="Remote Desktop for Otto - RDP and Virtual Outputs"
    [troubleshooting]="Troubleshooting Otto - Logs and Common Failures"
    [dev/readme]="Otto Developer Guide - Architecture Overview"
    [dev/project-structure]="Otto Project Structure - Crates and Building"
    [dev/rendering]="Otto Rendering Pipeline - Scene Graph to Skia"
    [dev/render_loop]="Otto Render Loop - Scheduling and Damage Tracking"
    [dev/wayland]="Wayland Protocols in Otto - Handlers and State"
    [dev/scene-graph]="The Otto Scene Graph - Layer Tree and KMS Planes"
    [dev/layers]="Layers in Otto - Properties, Transactions, Caching"
    [dev/drm_plane]="DRM Planes in Otto - Hardware Scanout"
    [dev/dock-design]="Otto Dock Internals - Data Flow and Magnification"
    [dev/expose]="Expose Internals in Otto - Layout and Mirrors"
    [dev/window-move]="Interactive Window Moves in Otto"
    [dev/foreign-toplevel]="Foreign Toplevel - Exposing Otto's Window List"
    [dev/screenshare]="Screen Capture Internals - Portal and PipeWire"
    [dev/color-scheme-setting]="Color Scheme - Telling Apps Light or Dark"
    [dev/settings-dbus-api]="org.otto.Settings D-Bus API Reference"
    [dev/rdp-virtual-output]="RDP Bridge and Virtual Outputs in Otto"
    [dev/remote-desktop-indicator]="Remote Desktop Indicator in Otto"
    [dev/otto-kit-roadmap]="otto-kit Roadmap - UI Toolkit Gap Analysis"
    [dev/surface-style-protocol]="Surface Style Protocol"
    [dev/screenshot-plan]="Screenshot Portal Plan (Not Implemented)"
    [dev/airplay-screenshare]="AirPlay Screencast Exploration in Otto"
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
    [dynamic-island]="Otto's dynamic island collects notifications, media controls, system HUDs and permission dialogs into one adaptive surface."
    [keyboard-shortcuts]="Every keyboard action Otto can bind, the modifier syntax it accepts, and how to override or remove the defaults."
    [gestures]="Three-finger swipes to change workspaces and a four-finger pinch for Expose, plus how to tune touchpad gestures in Otto."
    [files]="Browse and manage files with Otto Files: column and grid views, thumbnails, file operations, quick view and the file picker."
    [settings]="Change Otto's configuration while it runs: display arrangement, keyboard shortcuts, appearance and input, all applied live."
    [launcher]="Start applications and jump to open windows from the keyboard with Otto's launcher, including search and result ordering."
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
    [remote-desktop]="Serve an Otto session over RDP with otto-rdp, create virtual outputs, and connect from Windows, macOS or mobile clients."
    [troubleshooting]="Find Otto's logs, diagnose the most common startup and rendering failures, and gather what a useful bug report needs."
    [dev/readme]="How Otto is built: Smithay for Wayland, Skia for drawing, and a retained lay-rs scene graph - plus where to start reading the source."
    [dev/project-structure]="Where everything lives in the Otto repository, what each Cargo feature flag turns on, and how to build the workspace."
    [dev/rendering]="How Otto turns its scene graph into render elements, draws them with Skia, and submits finished frames to the display."
    [dev/render_loop]="When Otto wakes up, when it decides to render, and how damage tracking limits each frame to what actually changed."
    [dev/wayland]="The one-big-state pattern behind Otto's Wayland protocol handlers, and how to find the code implementing any protocol."
    [dev/scene-graph]="How Otto's layer tree is shaped, how Wayland surfaces enter it, and when a subtree is promoted to a KMS hardware plane."
    [dev/layers]="The unit Otto's scene tree is built from: layer properties, content closures, animation transactions, caching and damage."
    [dev/drm_plane]="Handing parts of the scene to display hardware instead of the GPU: plane selection, format filtering and per-frame validation."
    [dev/dock-design]="How Otto's compositor-drawn dock is built: where its data comes from, how its layers are arranged, and the magnification animation."
    [dev/expose]="How Otto's all-windows overview works: layout maths, window mirrors, drag-and-drop, and behaviour across multiple outputs."
    [dev/window-move]="How Otto implements interactive window drags, from the pointer grab through to the animation that settles the window."
    [dev/foreign-toplevel]="How Otto publishes its window list to taskbars, docks and launchers through the foreign-toplevel Wayland protocols."
    [dev/screenshare]="Otto's screen capture architecture: the xdg-desktop-portal backend, PipeWire streams, wlr-screencopy and single-window capture."
    [dev/color-scheme-setting]="How Otto tells GTK and Qt applications whether the desktop is currently in light or dark mode."
    [dev/settings-dbus-api]="The wire contract for Otto's settings service: the interfaces, properties and signals on org.otto.Settings."
    [dev/rdp-virtual-output]="How otto-rdp serves a virtual output over RDP: creating the output, encoding frames, and mapping input back into the session."
    [dev/remote-desktop-indicator]="The sharing indicator otto-rdp publishes while a remote client is watching the session, and how the compositor draws it."
    [dev/otto-kit-roadmap]="What Otto's UI toolkit provides today, what is still missing, and the planned direction. Partially built - read it as a plan."
    [dev/surface-style-protocol]="otto-surface-style-unstable-v1: how a client styles and animates its own surface through the compositor's scene graph, and why the protocol looks the way it does."
    [dev/screenshot-plan]="A design for screenshot support through the desktop portal. Not implemented - a proposal rather than documentation."
    [dev/airplay-screenshare]="Notes from validating AirPlay as a screencast target for Otto. An exploration, not a shipped feature."
)

# Pages with a screenshot worth using as the social-card image.
declare -A PAGE_IMAGE=(
    [desktop-widgets]="images/desktop-widgets.jpg"
    [customization]="images/rice-deep-field.jpg"
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
    "user/workspaces.md"
    "user/expose-and-switcher.md"
    "user/dock.md"
    "user/topbar.md"
    "user/dynamic-island.md"
    "user/keyboard-shortcuts.md"
    "user/gestures.md"
    "user/files.md"
    "user/settings.md"
    "user/launcher.md"
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
    "user/lock-screen.md"
    "user/login-greeter.md"
    "user/screen-sharing.md"
    "user/remote-desktop.md"
    "user/troubleshooting.md"
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
    cp "$IMAGE_SRC"/*.jpg "$IMAGE_SRC"/*.png "$SCRIPT_DIR/assets/images/" 2>/dev/null
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
    "developer/rendering.md"
    "developer/render_loop.md"
    "developer/wayland.md"
    "developer/scene-graph.md"
    "developer/layers.md"
    "developer/otto-kit.md"
    "developer/drm_plane.md"
    "developer/dock-design.md"
    "developer/expose.md"
    "developer/window-move.md"
    "developer/foreign-toplevel.md"
    "developer/screenshare.md"
    "developer/color-scheme-setting.md"
    "developer/settings-dbus-api.md"
    "developer/rdp-virtual-output.md"
    "developer/remote-desktop-indicator.md"
    "developer/versioning.md"
    "developer/otto-kit-roadmap.md"
    "developer/surface-style-protocol.md"
    "developer/screenshot-plan.md"
    "developer/airplay-screenshare.md"
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

echo "✓ Built User Guide: $OUTPUT_DIR/_index.md"
echo "✓ Built Developer Guide: $DEV_OUT/ ($(ls "$DEV_OUT" | wc -l) pages)"
