# Otto — British English
#
# This is the source catalogue: the only file guaranteed to carry every key.
# Every other locale mirrors it, except en-US, which is a sparse overlay
# holding only the spellings and formats that differ.
#
# Conventions the translations follow:
#   - Menu items and commands are in macOS-style title case in English. Most
#     other languages use sentence case; follow local practice, not English's.
#   - Settings rows and group headings are in sentence case.
#   - Short labels and `detail` lines carry no terminal full stop.
#   - Otto states facts. It does not instruct, apologise or exclaim.


## Shared
##
## Buttons and commands that appear in more than one place. Keep them short —
## several sit in fixed-width buttons.

common-open = Open
common-save = Save
common-cancel = Cancel
common-add = Add
common-remove = Remove
common-quit = Quit
common-cut = Cut
common-copy = Copy
common-paste = Paste
common-rename = Rename
common-delete = Delete
common-move = Move


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Auto-hide
dock-auto-hide-on = ✓ Auto-hide
dock-magnification = Magnification
dock-magnification-on = ✓ Magnification
dock-position-bottom = Bottom
dock-position-bottom-on = ✓ Bottom
dock-position-left = Left
dock-position-left-on = ✓ Left
dock-position-right = Right
dock-position-right-on = ✓ Right

# Shown on an app's icon when the app is not running.
dock-open = Open
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Keep in Dock
dock-keep-in-dock-on = ✓ Keep in Dock
dock-quit = Quit


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = General
settings-pane-displays = Displays
settings-pane-dock = Dock
settings-pane-keyboard = Keyboard
settings-pane-pointing = Trackpad & Mouse
settings-pane-sound = Sound
settings-pane-power = Power
settings-pane-lock-and-login = Lock & Login


## Settings — General

settings-group-appearance = Appearance
settings-colour-scheme = Colour scheme
settings-accent-colour = Accent colour
settings-font = System font
settings-gtk-theme = GTK theme

settings-group-desktop = Desktop
settings-background-colour = Background colour
settings-background-image = Background image
settings-background-image-detail = Chosen through the desktop portal's file picker

settings-group-pointer-and-icons = Pointer & icons
settings-cursor-theme = Cursor theme
settings-cursor-size = Cursor size
settings-icon-theme = Icon theme

settings-group-window-switcher = Window switcher
settings-follow-cursor = Show on the pointer's display

settings-group-language = Language
settings-preferred-languages = Preferred languages

settings-group-configuration = Configuration
settings-configuration-file = Configuration file
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = not known — the compositor is not answering


## Settings — Displays

settings-display-active = Active
settings-display-active-detail = An inactive display keeps its place in the arrangement
settings-display-primary = Use as primary
settings-display-primary-detail = The dock and the bar live on the primary display
settings-display-x-position = X position
settings-display-y-position = Y position
settings-display-x-position-detail = Top-left corner in the desktop's coordinate space
settings-display-width = Width
settings-display-width-detail = Pixels. A headless output can be any size
settings-display-height = Height
settings-display-refresh = Refresh rate
settings-display-refresh-detail = Hertz — how often the stream is fed a frame
settings-display-resolution = Resolution
settings-display-scale = Display scale
settings-display-scale-detail = Applies at the next login. The desktop does not reflow live

# Shown when the compositor reports no outputs at all.
settings-display-none = No displays
settings-display-none-detail = The compositor is not driving any output

settings-virtual-displays = Virtual displays
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } headless output, streamed over PipeWire. Remove takes away the selected one
       *[other] { $count } headless outputs, streamed over PipeWire. Remove takes away the selected one
    }


## Settings — Dock

settings-dock-size = Size
settings-dock-position = Position on screen
settings-dock-autohide = Automatically hide
settings-dock-magnification = Magnification
settings-group-magnification-and-icons = Magnification & icons
settings-dock-magnification-amount = Magnification amount
settings-dock-tint-icons = Tint icons
settings-dock-icon-tint = Icon tint
settings-dock-icon-tint-strength = Icon tint strength


## Settings — Keyboard

settings-key-repeat-delay = Key repeat delay
settings-key-repeat-rate = Key repeat rate
settings-group-input-source = Input source
settings-xkb-layout = Layout
settings-xkb-variant = Variant
settings-xkb-options = Options
settings-group-shortcuts = Shortcuts
settings-key-combination = Key combination
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift or Logo joined by +, then one key: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Trackpad
settings-tap-to-click = Tap to click
settings-tap-and-drag = Tap and drag
settings-drag-lock = Drag lock
settings-click-method = Click method
settings-ignore-while-typing = Ignore while typing
settings-natural-scrolling = Natural scrolling
settings-left-handed = Left-handed
settings-middle-click-emulation = Middle-click emulation
settings-group-pointer = Pointer
settings-tracking-speed = Tracking speed
settings-pointer-acceleration = Acceleration
settings-scrolling-speed = Scrolling speed


## Settings — Sound

settings-interface-sounds = Interface sounds
settings-sound-theme = Sound theme


## Settings — Power

settings-manage-lid-switch = Handle the lid switch
settings-manage-lid-switch-detail = Otto suspends on lid close instead of logind
settings-on-lid-close = When the lid closes
settings-on-power-button = When the power button is pressed


## Settings — Lock & Login

settings-group-lock = Lock
settings-lock-after = Lock after
settings-lock-screen = Lock screen
settings-lock-screen-detail = Applies the next time the screen locks
settings-lock-screen-arguments = Lock screen arguments
settings-group-login = Login
settings-greeter = Greeter
settings-greeter-detail = Applies at the next login
settings-greeter-arguments = Greeter arguments


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Light
settings-choice-dark = Dark
settings-choice-position-bottom = Bottom
settings-choice-position-left = Left
settings-choice-position-right = Right
settings-choice-clickfinger = Click with fingers
settings-choice-buttonareas = Click in corners
settings-choice-accel-flat = Constant speed
settings-choice-accel-adaptive = Speed follows movement
settings-choice-lid-auto = Decide automatically
settings-choice-lid-lock = Lock the screen
settings-choice-lid-disable-internal = Turn off the built-in display
settings-choice-power-ignore = Do nothing
settings-choice-power-lock = Lock the screen
settings-choice-power-suspend = Suspend
settings-choice-power-shutdown = Shut down
# The automatic option for a theme that follows the system.
settings-choice-auto = Auto


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value }%
settings-readout-pixels = { $value } px
settings-readout-milliseconds = { $value } ms
settings-readout-seconds = { $value } s
# Key repeats per second.
settings-readout-per-second = { $value } / s


## Files — windows

files-window-title = Files
# The Get Info panel's own window.
files-info-window-title = Info


## Files — commands

files-get-info = Get Info
files-new-folder = New Folder
files-move-to-trash = Move to Trash
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Move { $count } Item to Trash
       *[other] Move { $count } Items to Trash
    }


## Files — sidebar and columns

files-places = Places
files-home = Home
files-desktop = Desktop
files-documents = Documents
files-downloads = Downloads
files-music = Music
files-pictures = Pictures
files-videos = Videos
files-trash = Trash

files-column-name = Name
files-column-size = Size
files-column-kind = Kind
files-column-date-modified = Date Modified


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Folder
files-kind-image = Image
files-kind-movie = Movie
files-kind-audio = Audio
files-kind-text = Text
files-kind-document = Document
files-kind-archive = Archive
files-kind-application = Application


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Loading…
files-empty = Empty
# The idle line: what the folder holds.
files-status-no-items = No items
files-status-items =
    { $count ->
        [one] 1 item
       *[other] { $count } items
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, { $hidden } hidden
files-status-selected = { $count } of { $total } selected
files-status-opening-preview = Opening preview…
files-nothing-to-undo = Nothing to undo
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Undid { $label }
files-undo-move = Move
files-undo-copy = Copy
files-undo-delete = Delete
files-undo-rename = Rename
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Renamed to “{ $name }”
files-new-folder-created = New folder “{ $name }”
files-gone = “{ $name }” is no longer there
files-rename-failed = Couldn’t rename: { $error }
files-new-folder-failed = Couldn’t create folder: { $error }
files-open-failed = Couldn’t open that file: { $error }
files-new-window-failed = Couldn’t open a new window: { $error }


## Files — the listing

files-folder-empty = This folder is empty.
files-folder-denied = You do not have permission to see this folder's contents.
files-folder-gone = This folder no longer exists.
files-folder-open-failed = This folder could not be opened: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Where
files-info-kind = Kind
files-info-modified = Modified
files-info-created = Created
files-info-accessed = Accessed
files-info-owner = Owner
files-info-links-to = Links to
files-info-permissions = Permissions
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Read
files-perm-write = Write
files-perm-exec = Exec
# Row labels: who each set of permissions applies to.
files-perm-owner = Owner
files-perm-group = Group
files-perm-everyone = Everyone


## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Open
files-picker-save-as = Save As
files-picker-save-files = Save Files
files-picker-all-files = All Files
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Save As:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Enter a name
files-save-name-has-slash = A name cannot contain “/”
files-save-name-reserved = That name is reserved
files-save-nowhere = Nowhere to save
files-save-permission-denied = You do not have permission to save here

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = “{ $name }” already exists. Replace it?
files-replace-one-detail = Replacing it overwrites its current contents.
files-replace-many = { $count } of these files already exist. Replace them?
files-replace-many-detail = Replacing them overwrites their current contents.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } byte
       *[other] { $count } bytes
    }
files-size-kb = { $value } KB
files-size-mb = { $value } MB
files-size-gb = { $value } GB
files-size-tb = { $value } TB


## Files — dates
##
## Assembled from the parts below rather than from a format string, because
## the month names have to be translated too.
##
## $day is the day of the month, $month one of the abbreviations below, $year
## the four-digit year, $time the time as HH:MM. Reorder them freely — en-US
## puts the month first.

files-date-modified = { $day } { $month } { $year } at { $time }

files-month-jan = Jan
files-month-feb = Feb
files-month-mar = Mar
files-month-apr = Apr
files-month-may = May
files-month-jun = Jun
files-month-jul = Jul
files-month-aug = Aug
files-month-sep = Sep
files-month-oct = Oct
files-month-nov = Nov
files-month-dec = Dec


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %A %-d %B  %H:%M


## Settings — widgets
##
## The controls themselves, rather than the settings they edit.

# Shown in a text field that has no value yet.
settings-not-set = Not set
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Choose…
settings-no-file-chosen = No file chosen
settings-choose-background-image = Choose a Background Image

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Otto Settings — { $pane }


## Settings schema
##
## The labels and descriptions the compositor serves for each setting. The
## label names the row; the description is the smaller line beneath it.
##
## Keys are derived from the setting's own identifier, so they are not written
## by hand and must not be renamed. A setting with no entry here falls back to
## the English in the compositor's schema, so a gap is untranslated rather
## than broken.
##
## Descriptions are full sentences and end in a full stop — unlike the short
## `detail` lines elsewhere, which do not.

# --- general ---
schema-screen-scale-label = Display scale
schema-screen-scale-description = Global scale factor applied to the desktop.
schema-theme-scheme-label = Colour scheme
schema-theme-scheme-description = Light or dark colour scheme.
schema-accent-color-label = Accent colour
schema-accent-color-description = Named accent colour used by Otto's own interface.
schema-font-family-label = Interface font
schema-font-family-description = Font family used by Otto's own interface.
schema-background-color-label = Background colour
schema-background-color-description = Desktop background colour, as a hex string.
schema-background-image-label = Background image
schema-background-image-description = Path to the desktop background image. Empty for none.
schema-cursor-theme-label = Cursor theme
schema-cursor-theme-description = Name of the XCursor theme.
schema-cursor-size-label = Cursor size
schema-cursor-size-description = Cursor size in logical pixels.
schema-icon-theme-label = Icon theme
schema-icon-theme-description = Name of the icon theme. Empty auto-detects.
schema-gtk-theme-label = GTK theme
schema-gtk-theme-description = GTK theme name handed to clients. Empty auto-detects.
schema-locales-label = Locales
schema-locales-description = Preferred locales, most preferred first.

# --- dock ---
schema-dock-size-label = Size
schema-dock-size-description = Dock size multiplier.
schema-dock-position-label = Position on screen
schema-dock-position-description = Screen edge the dock lives on.
schema-dock-autohide-label = Automatically hide
schema-dock-autohide-description = Hide the dock until the pointer reaches its screen edge.
schema-dock-magnification-label = Magnification
schema-dock-magnification-description = Grow the icons under the pointer.
schema-dock-genie-scale-label = Magnification amount
schema-dock-genie-scale-description = How much the icons under the pointer grow.
schema-dock-genie-span-label = Magnification spread
schema-dock-genie-span-description = How many neighbouring icons the magnification reaches.
schema-dock-colorize-icons-label = Tint icons
schema-dock-colorize-icons-description = Tint dock icons with a single colour.
schema-dock-colorize-color-label = Icon tint
schema-dock-colorize-color-description = Colour used to tint dock icons, as a hex string.
schema-dock-colorize-intensity-label = Icon tint strength
schema-dock-colorize-intensity-description = How strongly the tint is applied.

# --- general ---
schema-keyboard-repeat-delay-label = Repeat delay
schema-keyboard-repeat-delay-description = Milliseconds a key is held before it starts repeating.
schema-keyboard-repeat-rate-label = Repeat rate
schema-keyboard-repeat-rate-description = Repeats per second while a key is held.

# --- input ---
schema-input-xkb-layout-label = Keyboard layout
schema-input-xkb-layout-description = XKB layout name. Empty uses the system default.
schema-input-xkb-variant-label = Keyboard variant
schema-input-xkb-variant-description = XKB variant name. Empty uses the system default.
schema-input-xkb-options-label = Keyboard options
schema-input-xkb-options-description = XKB option strings.
schema-input-tap-enabled-label = Tap to click
schema-input-tap-enabled-description = Treat a tap on the touchpad as a click.
schema-input-tap-drag-enabled-label = Tap and drag
schema-input-tap-drag-enabled-description = Start a drag from a tap followed by a held touch.
schema-input-tap-drag-lock-enabled-label = Drag lock
schema-input-tap-drag-lock-enabled-description = Keep a tap-drag going through a brief lift of the finger.
schema-input-touchpad-click-method-label = Click method
schema-input-touchpad-click-method-description = Whether a click means finger count or button areas.
schema-input-touchpad-dwt-enabled-label = Disable while typing
schema-input-touchpad-dwt-enabled-description = Ignore the touchpad while the keyboard is in use.
schema-input-touchpad-natural-scroll-enabled-label = Natural scrolling
schema-input-touchpad-natural-scroll-enabled-description = Content follows the fingers.
schema-input-touchpad-left-handed-label = Left-handed
schema-input-touchpad-left-handed-description = Swap the primary and secondary buttons.
schema-input-touchpad-middle-emulation-enabled-label = Middle-click emulation
schema-input-touchpad-middle-emulation-enabled-description = Pressing both buttons together is a middle click.
schema-input-scroll-speed-label = Scroll speed
schema-input-scroll-speed-description = Software multiplier applied to scroll events.
schema-input-pointer-accel-speed-label = Pointer speed
schema-input-pointer-accel-speed-description = Pointer acceleration, from -1 (slowest) to 1 (fastest).
schema-input-pointer-accel-profile-label = Pointer acceleration
schema-input-pointer-accel-profile-description = Flat is raw speed; adaptive follows libinput's curve.

# --- audio ---
schema-audio-sound-enabled-label = Interface sounds
schema-audio-sound-enabled-description = Play sound feedback for interface events.
schema-audio-sound-theme-label = Sound theme
schema-audio-sound-theme-description = XDG sound theme name. Empty auto-detects.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Handle the lid switch
schema-power-management-manage-lid-switch-description = Let Otto act on the lid rather than leaving it to logind.
schema-power-management-on-lid-close-label = When the lid closes
schema-power-management-on-lid-close-description = What happens when the laptop lid is closed.
schema-power-management-on-power-button-label = When the power button is pressed
schema-power-management-on-power-button-description = What happens when the hardware power button is pressed.

# --- lock ---
schema-lock-locker-command-label = Lock screen command
schema-lock-locker-command-description = The locker launched to lock the session.
schema-lock-locker-args-label = Lock screen arguments
schema-lock-locker-args-description = Arguments passed to the locker.
schema-lock-auto-lock-timeout-label = Lock after
schema-lock-auto-lock-timeout-description = Seconds of inactivity before locking. 0 never locks.

# --- login ---
schema-login-greeter-command-label = Greeter command
schema-login-greeter-command-description = The greeter launched in login mode.
schema-login-greeter-args-label = Greeter arguments
schema-login-greeter-args-description = Arguments passed to the greeter.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = Switcher follows the pointer
schema-appswitcher-follow-cursor-description = Show the app switcher on the output the pointer is on.


## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automatic


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Blue
settings-choice-accent-purple = Purple
settings-choice-accent-pink = Pink
settings-choice-accent-red = Red
settings-choice-accent-orange = Orange
settings-choice-accent-yellow = Yellow
settings-choice-accent-green = Green
settings-choice-accent-mint = Mint
settings-choice-accent-teal = Teal
settings-choice-accent-cyan = Cyan
settings-choice-accent-indigo = Indigo
settings-choice-accent-brown = Brown
settings-choice-accent-graphite = Graphite
# The button under the shortcut list that adds another line.
settings-add-shortcut = Add shortcut
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Workspace { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Search for apps and windows…
launcher-search-apps = Search for apps…
launcher-search-windows = Search for windows…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Window
launcher-badge-calc = Calc


## Login, lock and authentication
##
## The greeter, the lock screen, and the panel both of them draw. Text that
## arrives from PAM or greetd at runtime is not here: those localise
## themselves, and restating them would be guessing at another program's words.
# Button under the login/lock card, offered only while the fingerprint reader
# is being waited on: it abandons the finger and asks for a password instead.
# The button sizes itself to the text, but it sits on a card 380pt wide — keep
# it to roughly 20 characters so it does not overhang.
auth-enter-password = Enter Password

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Sign in

# strftime pattern for the time in the large clock above the login/lock card,
# not prose: only the %-codes and the separators between them are yours.
# Change it where the local convention differs — %-I:%M %p for a 12-hour
# locale. The digits render at 46pt, so keep the result short.
auth-clock-time-format = %H:%M

# strftime pattern for the date under that clock, again not prose. Reorder the
# parts and change the punctuation to suit the locale (German would be
# "%A, %-d. %B"); the weekday and month names are translated by the system, so
# do not spell them out here. Renders at 15pt in a 360pt box.
auth-clock-date-format = %A %-d %B

### otto-greeter — the login screen shown before any session exists.
### Everything here is drawn on the login card, centred on the wallpaper.
### The card is narrow: prompts sit above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field while the login screen is asking who is logging
# in. One line, above a text field roughly 20 characters wide — keep it to one
# or two words.
greeter-prompt-username = Username

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Password

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Authenticating…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Login service unavailable: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Login service went away

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } did not start

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Authenticated

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Place your finger on the reader

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Swipe your finger across the reader

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Place your { $finger } on the reader
greeter-status-swipe-named-finger = Swipe your { $finger } across the reader

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Fingerprint not recognised

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Waiting for the fingerprint reader…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Starting session…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Not permitted to suspend

# As above, for the restart request.
greeter-power-restart-denied = Not permitted to restart

# As above, for the shut down request.
greeter-power-shutdown-denied = Not permitted to shut down

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Could not suspend

# As above, for the restart request.
greeter-power-restart-failed = Could not restart

# As above, for the shut down request.
greeter-power-shutdown-failed = Could not shut down

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Password

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Authenticated

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Place your finger on the reader

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Swipe your finger across the reader

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Place your { $finger } on the reader
lock-status-swipe-named-finger = Swipe your { $finger } across the reader

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = left thumb
auth-finger-left-index = left index finger
auth-finger-left-middle = left middle finger
auth-finger-left-ring = left ring finger
auth-finger-left-little = left little finger
auth-finger-right-thumb = right thumb
auth-finger-right-index = right index finger
auth-finger-right-middle = right middle finger
auth-finger-right-ring = right ring finger
auth-finger-right-little = right little finger

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Fingerprint not recognised

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Waiting for the fingerprint reader…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = No user to authenticate

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Authentication service failed

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Invalid user name

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Authentication is unavailable

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Authentication failed ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Not permitted to suspend

# As above, for the restart request.
lock-power-restart-denied = Not permitted to restart

# As above, for the shut down request.
lock-power-shutdown-denied = Not permitted to shut down

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Could not suspend: { $error }

# As above, for the restart request.
lock-power-restart-failed = Could not restart: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Could not shut down: { $error }


## Quick Look and the islands
##
## The previewer: press space on a file and see it. Everything here is drawn
## inside a small card floating over the file list, so nothing has much room.
## The card is roughly 300–600 px wide.
##
## Most of these strings are produced by a sandboxed worker process that
## parses the file. When it cannot show anything, the reason below *is* the
## preview — it fills the card. Those reasons are lower-case and start
## mid-sentence on purpose: they read as a continuation of "no preview".


## Quick View — card labels
##
## Fact keys: the left-hand column of a card's detail list. One or two words,
## drawn in a narrow column — keep them short. Title case in English.

# Column heading for the file's type, e.g. "JPEG", "PDF". Max ~12 characters.
quickview-fact-kind = Kind
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = Size
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Dimensions
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Duration
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Pixels
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Pages
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Title
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Artist
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Album
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Year

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Empty file
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Too large to preview
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } megapixels
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Install one of: { $packages } — to see the pages


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Empty folder
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } item
       *[other] { $count } items
    }
# Summary line for an archive, joining the entry count to the archive's own
# size on disk. $items is quickview-item-count, $size is a formatted byte
# count. The dash is an em dash.
quickview-archive-summary = { $items } — { $size }


## Quick View — sizes
##
## Byte units. Quick View counts in powers of 1024, so the symbols are the
## conventional binary-rounded ones. Translate only the spelled-out "bytes".

quickview-size-bytes =
    { $count ->
        [one] { $count } byte
       *[other] { $count } bytes
    }
quickview-size-kb = { $value } KB
quickview-size-mb = { $value } MB
quickview-size-gb = { $value } GB
quickview-size-tb = { $value } TB


## Quick View — nothing to show
##
## Each of these fills the card in place of a preview, so a person reads it
## instead of seeing the file. They state what happened and stop. Lower case,
## no full stop: they are shown as a sentence fragment.
##
## $error is an operating-system message, which arrives in whatever language
## the system libraries produce and is usually English. Keep it at the end.

# The file is a pipe, socket or device — opening it could block forever.
quickview-error-not-previewable = this is not a file that can be previewed
# The file's metadata could not be read.
quickview-error-stat-file = cannot stat the file: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = cannot read the file: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = the file is not seekable
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = could not sandbox the previewer: { $error }

# Image previewer.
quickview-error-read-image = cannot read the image: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = not an image this build can decode
quickview-error-image-no-size = the image reports no size
quickview-error-image-decode = the image did not decode: { $error }
quickview-error-image-readback = the decoded image could not be read back

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = cannot read the drawing: { $error }
quickview-error-drawing-parse = the drawing could not be parsed
quickview-error-drawing-surface = no surface to render it on
quickview-error-drawing-readback = the drawing could not be read back

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = this file is not text in any encoding Otto reads

# PDF previewer.
quickview-error-read-document = cannot read the document: { $error }
quickview-error-page-readback = the rendered page could not be read

# Folder listing.
quickview-error-read-folder = cannot read the folder

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = cannot find the previewer: { $error }
quickview-error-previewer-start = cannot start the previewer: { $error }
quickview-error-previewer-no-output = the previewer produced no output
quickview-error-previewer-unreadable = the previewer produced something unreadable
quickview-error-previewer-failed = the previewer failed: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = this file took too long to preview

## Islands
##
## The dynamic island: the small dark bubble at the top of the screen that
## grows into a notification card, and the permission dialogs the portal
## raises through it. Space is very tight — a card is about 320 px wide and
## 64 px tall, drawn at 9–13 px.


## Islands — notification card

# The button that dismisses a notification card. Drawn inside a fixed 40 px
# column at 9 px, so it must fit in roughly 7 characters — a shorter word is
# better than a truer one here.
islands-close = Close

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = just now
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count }m ago
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count }h ago


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Allow
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Continue
# Refuses the request.
islands-dialog-deny = Deny


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

# The dock itself, as one object; the icons inside it are named
# individually by the application they launch.
a11y-dock = Dock
# Said after an application's name in the dock, for the dot under the icon.
a11y-app-running = Running
a11y-app-not-running = Not running
# The panel that appears while the switch-application keys are held.
a11y-app-switcher = Application switcher
# The list of open windows shown by the overview.
a11y-windows = Windows
# The strip of workspaces shown by the overview.
a11y-workspaces = Workspaces
# A window that reports no title of its own.
a11y-untitled-window = Untitled window
# The bar across the top of the screen.
a11y-menu-bar = Menu bar
# The right-hand end of the bar, holding the clock and the tray icons.
a11y-status = Status
# A tray icon whose application gave it no name of its own. $number
# counts from 1, left to right.
a11y-tray-item = Tray item { $number }
# The stack of notification islands.
a11y-notifications = Notifications
# The sidebar of Settings, listing its panes.
a11y-categories = Categories
# The launcher's list of matches for what has been typed.
a11y-results = Results
# Names the Settings pane when no pane is selected.
a11y-settings = Settings
# Quick Look's contents, when it is showing something with no pages.
a11y-preview = Preview
a11y-preview-page = Preview, page { $page } of { $pages }
# Said of a preview that shows only the beginning of a long file.
a11y-preview-shortened = Preview, shortened
