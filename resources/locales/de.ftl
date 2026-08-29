# Otto — German
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

common-open = Öffnen
common-save = Speichern
common-cancel = Abbrechen
common-add = Hinzufügen
common-remove = Entfernen
common-quit = Beenden
common-cut = Ausschneiden
common-copy = Kopieren
common-paste = Einfügen
common-rename = Umbenennen
common-delete = Löschen
common-move = Verschieben


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Automatisch ausblenden
dock-auto-hide-on = ✓ Automatisch ausblenden
dock-magnification = Vergrößerung
dock-magnification-on = ✓ Vergrößerung
dock-position-bottom = Unten
dock-position-bottom-on = ✓ Unten
dock-position-left = Links
dock-position-left-on = ✓ Links
dock-position-right = Rechts
dock-position-right-on = ✓ Rechts

# Shown on an app's icon when the app is not running.
dock-open = Öffnen
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Im Dock behalten
dock-keep-in-dock-on = ✓ Im Dock behalten
dock-quit = Beenden


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Allgemein
settings-pane-displays = Monitore
settings-pane-dock = Dock
settings-pane-keyboard = Tastatur
settings-pane-pointing = Trackpad & Maus
settings-pane-sound = Ton
settings-pane-power = Energie
settings-pane-lock-and-login = Sperren & Anmelden


## Settings — General

settings-group-appearance = Erscheinungsbild
settings-colour-scheme = Farbschema
settings-accent-colour = Akzentfarbe
settings-rounded-corners = Abgerundete Ecken
settings-rounded-corners-detail = Gilt nach einem Neustart
settings-window-controls = Fenstersteuerelemente
settings-font = Systemschriftart
settings-gtk-theme = GTK-Thema

settings-group-desktop = Schreibtisch
settings-background-colour = Hintergrundfarbe
settings-background-image = Hintergrundbild
settings-background-image-detail = Über die Dateiauswahl des Desktop-Portals gewählt

settings-group-pointer-and-icons = Zeiger & Symbole
settings-cursor-theme = Zeigerdesign
settings-cursor-size = Zeigergröße
settings-icon-theme = Symboldesign

settings-group-window-switcher = Fensterumschalter
settings-follow-cursor = Auf dem Bildschirm des Zeigers anzeigen

settings-group-language = Sprache
settings-preferred-languages = Bevorzugte Sprachen

settings-group-configuration = Konfiguration
settings-configuration-file = Konfigurationsdatei
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = unbekannt – der Compositor antwortet nicht


## Settings — Displays

settings-display-active = Aktiv
settings-display-active-detail = Ein inaktiver Monitor behält seinen Platz in der Anordnung
settings-display-primary = Als primär verwenden
settings-display-primary-detail = Dock und Leiste befinden sich auf dem primären Monitor
settings-display-x-position = X-Position
settings-display-y-position = Y-Position
settings-display-x-position-detail = Obere linke Ecke im Koordinatensystem des Schreibtischs
settings-display-width = Breite
settings-display-width-detail = Pixel. Ein Headless-Ausgang kann beliebig groß sein
settings-display-height = Höhe
settings-display-refresh = Bildwiederholrate
settings-display-refresh-detail = Hertz – wie oft der Stream ein Bild erhält
settings-display-resolution = Auflösung
settings-display-scale = Bildschirmskalierung
settings-display-scale-detail = Gilt ab der nächsten Anmeldung. Der Schreibtisch passt sich nicht sofort an

# Shown when the compositor reports no outputs at all.
settings-display-none = Keine Monitore
settings-display-none-detail = Der Compositor steuert keine Ausgabe an

settings-virtual-displays = Virtuelle Monitore
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } Headless-Ausgang, über PipeWire gestreamt. „Entfernen“ nimmt den ausgewählten weg
       *[other] { $count } Headless-Ausgänge, über PipeWire gestreamt. „Entfernen“ nimmt den ausgewählten weg
    }


## Settings — Dock

settings-dock-size = Größe
settings-dock-position = Position auf dem Bildschirm
settings-dock-autohide = Automatisch ausblenden
settings-dock-magnification = Vergrößerung
settings-group-magnification-and-icons = Vergrößerung & Symbole
settings-dock-magnification-amount = Vergrößerungsgrad
settings-dock-tint-icons = Symbole einfärben
settings-dock-icon-tint = Farbton
settings-dock-icon-tint-strength = Farbtonstärke


## Settings — Keyboard

settings-key-repeat-delay = Verzögerung bis zur Wiederholung
settings-key-repeat-rate = Wiederholungsrate
settings-group-input-source = Eingabequelle
settings-xkb-layout = Layout
settings-xkb-variant = Variante
settings-xkb-options = Optionen
settings-group-shortcuts = Tastenkombinationen
settings-key-combination = Tastenkombination
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift oder Logo verbunden mit +, dann eine Taste: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Trackpad
settings-tap-to-click = Tippen zum Klicken
settings-tap-and-drag = Tippen und Ziehen
settings-drag-lock = Ziehsperre
settings-click-method = Klickmethode
settings-ignore-while-typing = Beim Tippen ignorieren
settings-natural-scrolling = Natürliches Scrollen
settings-left-handed = Linkshändig
settings-middle-click-emulation = Mittelklick-Emulation
settings-group-pointer = Zeiger
settings-tracking-speed = Verfolgungsgeschwindigkeit
settings-pointer-acceleration = Zeigerbeschleunigung
settings-scrolling-speed = Scrollgeschwindigkeit


## Settings — Sound

settings-interface-sounds = Oberflächenklänge
settings-sound-theme = Klangschema


## Settings — Power

settings-manage-lid-switch = Deckelschalter verwalten
settings-manage-lid-switch-detail = Otto versetzt in den Ruhezustand statt logind
settings-on-lid-close = Beim Schließen des Deckels
settings-on-power-button = Beim Drücken der Einschalttaste


## Settings — Lock & Login

settings-group-lock = Sperren
settings-lock-after = Sperren nach
settings-lock-screen = Sperrbildschirm
settings-lock-screen-detail = Gilt ab der nächsten Bildschirmsperre
settings-lock-screen-arguments = Argumente für den Sperrbildschirm
settings-group-login = Anmeldung
settings-greeter = Anmeldebildschirm
settings-greeter-detail = Gilt ab der nächsten Anmeldung
settings-greeter-arguments = Argumente für den Anmeldebildschirm


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Hell
settings-choice-dark = Dunkel
settings-choice-controls-left = Links
settings-choice-controls-right = Rechts
settings-choice-position-bottom = Unten
settings-choice-position-left = Links
settings-choice-position-right = Rechts
settings-choice-clickfinger = Mit Fingern klicken
settings-choice-buttonareas = In Ecken klicken
settings-choice-accel-flat = Konstante Geschwindigkeit
settings-choice-accel-adaptive = Geschwindigkeit folgt der Bewegung
settings-choice-lid-auto = Automatisch entscheiden
settings-choice-lid-lock = Bildschirm sperren
settings-choice-lid-disable-internal = Internen Monitor ausschalten
settings-choice-power-ignore = Nichts tun
settings-choice-power-lock = Bildschirm sperren
settings-choice-power-suspend = Ruhezustand
settings-choice-power-shutdown = Herunterfahren
# The automatic option for a theme that follows the system.
settings-choice-auto = Auto


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value } %
settings-readout-pixels = { $value } px
settings-readout-milliseconds = { $value } ms
settings-readout-seconds = { $value } s
# Key repeats per second.
settings-readout-per-second = { $value } / s


## Files — windows

files-window-title = Dateien
# The Get Info panel's own window.
files-info-window-title = Informationen


## Files — commands

files-get-info = Informationen
files-new-folder = Neuer Ordner
files-move-to-trash = In den Papierkorb legen
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] { $count } Objekt in den Papierkorb legen
       *[other] { $count } Objekte in den Papierkorb legen
    }


## Files — sidebar and columns

files-places = Orte
files-home = Persönlicher Ordner
files-desktop = Schreibtisch
files-documents = Dokumente
files-downloads = Downloads
files-music = Musik
files-pictures = Bilder
files-videos = Videos
files-trash = Papierkorb

files-column-name = Name
files-column-size = Größe
files-column-kind = Art
files-column-date-modified = Änderungsdatum


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Ordner
files-kind-image = Bild
files-kind-movie = Film
files-kind-audio = Audio
files-kind-text = Text
files-kind-document = Dokument
files-kind-archive = Archiv
files-kind-application = Programm


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Wird geladen…
files-empty = Leer
# The idle line: what the folder holds.
files-status-no-items = Keine Objekte
files-status-items =
    { $count ->
        [one] { $count } Objekt
       *[other] { $count } Objekte
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, { $hidden } ausgeblendet
files-status-selected = { $count } von { $total } ausgewählt
files-status-opening-preview = Vorschau wird geöffnet…
files-nothing-to-undo = Nichts rückgängig zu machen
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = { $label } rückgängig gemacht
files-undo-move = Verschieben
files-undo-copy = Kopieren
files-undo-delete = Löschen
files-undo-rename = Umbenennen
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = In „{ $name }“ umbenannt
files-new-folder-created = Neuer Ordner „{ $name }“
files-gone = „{ $name }“ ist nicht mehr vorhanden
files-rename-failed = Umbenennen nicht möglich: { $error }
files-new-folder-failed = Ordner konnte nicht erstellt werden: { $error }
files-open-failed = Datei konnte nicht geöffnet werden: { $error }
files-new-window-failed = Neues Fenster konnte nicht geöffnet werden: { $error }


## Files — the listing

files-folder-empty = Dieser Ordner ist leer.
files-folder-denied = Keine Berechtigung, den Inhalt dieses Ordners anzuzeigen.
files-folder-gone = Dieser Ordner existiert nicht mehr.
files-folder-open-failed = Dieser Ordner konnte nicht geöffnet werden: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Ort
files-info-kind = Art
files-info-modified = Geändert
files-info-created = Erstellt
files-info-accessed = Zuletzt geöffnet
files-info-owner = Eigentümer
files-info-links-to = Verweist auf
files-info-permissions = Zugriffsrechte
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Lesen
files-perm-write = Schreiben
files-perm-exec = Ausführen
# Row labels: who each set of permissions applies to.
files-perm-owner = Eigentümer
files-perm-group = Gruppe
files-perm-everyone = Alle

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Öffnen
files-picker-save-as = Sichern unter
files-picker-save-files = Dateien sichern
files-picker-all-files = Alle Dateien
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Sichern unter:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Namen eingeben
files-save-name-has-slash = Ein Name darf kein „/“ enthalten
files-save-name-reserved = Dieser Name ist reserviert
files-save-nowhere = Kein Speicherort
files-save-permission-denied = Keine Berechtigung, hier zu sichern

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = „{ $name }“ existiert bereits. Ersetzen?
files-replace-one-detail = Beim Ersetzen wird der aktuelle Inhalt überschrieben.
files-replace-many = { $count } dieser Dateien existieren bereits. Ersetzen?
files-replace-many-detail = Beim Ersetzen werden die aktuellen Inhalte überschrieben.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } Byte
       *[other] { $count } Bytes
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

files-date-modified = { $day }. { $month } { $year } um { $time }

files-month-jan = Jan.
files-month-feb = Feb.
files-month-mar = Mär.
files-month-apr = Apr.
files-month-may = Mai
files-month-jun = Jun.
files-month-jul = Jul.
files-month-aug = Aug.
files-month-sep = Sep.
files-month-oct = Okt.
files-month-nov = Nov.
files-month-dec = Dez.


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %A, %-d. %B  %H:%M


## Settings — widgets
##
## The controls themselves, rather than the settings they edit.

# Shown in a text field that has no value yet.
settings-not-set = Nicht festgelegt
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Wählen…
settings-no-file-chosen = Keine Datei gewählt
settings-choose-background-image = Hintergrundbild wählen

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Otto-Einstellungen — { $pane }


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
schema-screen-scale-label = Bildschirmskalierung
schema-screen-scale-description = Globaler Skalierungsfaktor für den Schreibtisch.
schema-theme-scheme-label = Farbschema
schema-theme-scheme-description = Helles oder dunkles Farbschema.
schema-accent-color-label = Akzentfarbe
schema-accent-color-description = Benannte Akzentfarbe für Ottos eigene Oberfläche.
schema-rounded-corners-label = Abgerundete Ecken
schema-rounded-corners-description = Rundet die Ecken des Docks, der oberen Leiste, der Fensterdekorationen und der Panels ab, die der Desktop selbst zeichnet.
schema-window-controls-side-label = Fenstersteuerelemente
schema-window-controls-side-description = An welchem Ende der Titelleiste die Knöpfe zum Schließen, Minimieren und Zoomen sitzen.
schema-font-family-label = Oberflächenschrift
schema-font-family-description = Schriftfamilie für Ottos eigene Oberfläche.
schema-background-color-label = Hintergrundfarbe
schema-background-color-description = Hintergrundfarbe des Schreibtischs, als Hex-Zeichenfolge.
schema-background-image-label = Hintergrundbild
schema-background-image-description = Pfad zum Hintergrundbild des Schreibtischs. Leer für keins.
schema-cursor-theme-label = Zeigerdesign
schema-cursor-theme-description = Name des XCursor-Designs.
schema-cursor-size-label = Zeigergröße
schema-cursor-size-description = Zeigergröße in logischen Pixeln.
schema-icon-theme-label = Symboldesign
schema-icon-theme-description = Name des Symboldesigns. Leer erkennt automatisch.
schema-gtk-theme-label = GTK-Thema
schema-gtk-theme-description = GTK-Themenname, der an Clients weitergegeben wird. Leer erkennt automatisch.
schema-locales-label = Gebietsschemas
schema-locales-description = Bevorzugte Gebietsschemas, in Reihenfolge der Präferenz.

# --- dock ---
schema-dock-size-label = Größe
schema-dock-size-description = Größenfaktor des Docks.
schema-dock-position-label = Position auf dem Bildschirm
schema-dock-position-description = Bildschirmkante, an der sich das Dock befindet.
schema-dock-autohide-label = Automatisch ausblenden
schema-dock-autohide-description = Dock ausblenden, bis der Zeiger seine Bildschirmkante erreicht.
schema-dock-magnification-label = Vergrößerung
schema-dock-magnification-description = Symbole unter dem Zeiger vergrößern.
schema-dock-genie-scale-label = Vergrößerungsgrad
schema-dock-genie-scale-description = Wie stark die Symbole unter dem Zeiger wachsen.
schema-dock-genie-span-label = Vergrößerungsreichweite
schema-dock-genie-span-description = Wie viele benachbarte Symbole die Vergrößerung erreicht.
schema-dock-colorize-icons-label = Symbole einfärben
schema-dock-colorize-icons-description = Dock-Symbole mit einer einzelnen Farbe einfärben.
schema-dock-colorize-color-label = Farbton
schema-dock-colorize-color-description = Farbe zum Einfärben der Dock-Symbole, als Hex-Zeichenfolge.
schema-dock-colorize-intensity-label = Farbtonstärke
schema-dock-colorize-intensity-description = Wie stark der Farbton angewendet wird.

# --- general ---
schema-keyboard-repeat-delay-label = Verzögerung bis zur Wiederholung
schema-keyboard-repeat-delay-description = Millisekunden, die eine Taste gehalten wird, bevor sie zu wiederholen beginnt.
schema-keyboard-repeat-rate-label = Wiederholungsrate
schema-keyboard-repeat-rate-description = Wiederholungen pro Sekunde, während eine Taste gehalten wird.

# --- input ---
schema-input-xkb-layout-label = Tastaturlayout
schema-input-xkb-layout-description = XKB-Layoutname. Leer verwendet die Systemvorgabe.
schema-input-xkb-variant-label = Tastaturvariante
schema-input-xkb-variant-description = XKB-Variantenname. Leer verwendet die Systemvorgabe.
schema-input-xkb-options-label = Tastaturoptionen
schema-input-xkb-options-description = XKB-Optionszeichenfolgen.
schema-input-tap-enabled-label = Tippen zum Klicken
schema-input-tap-enabled-description = Ein Tippen auf dem Trackpad als Klick behandeln.
schema-input-tap-drag-enabled-label = Tippen und Ziehen
schema-input-tap-drag-enabled-description = Ein Ziehen mit einem Tippen, gefolgt von einer gehaltenen Berührung, beginnen.
schema-input-tap-drag-lock-enabled-label = Ziehsperre
schema-input-tap-drag-lock-enabled-description = Ein Tipp-Ziehen über ein kurzes Abheben des Fingers hinweg fortsetzen.
schema-input-touchpad-click-method-label = Klickmethode
schema-input-touchpad-click-method-description = Ob ein Klick über die Fingerzahl oder über Ecken bestimmt wird.
schema-input-touchpad-dwt-enabled-label = Beim Tippen ignorieren
schema-input-touchpad-dwt-enabled-description = Das Trackpad ignorieren, während die Tastatur benutzt wird.
schema-input-touchpad-natural-scroll-enabled-label = Natürliches Scrollen
schema-input-touchpad-natural-scroll-enabled-description = Der Inhalt folgt den Fingern.
schema-input-touchpad-left-handed-label = Linkshändig
schema-input-touchpad-left-handed-description = Primäre und sekundäre Taste vertauschen.
schema-input-touchpad-middle-emulation-enabled-label = Mittelklick-Emulation
schema-input-touchpad-middle-emulation-enabled-description = Beide Tasten gleichzeitig gedrückt ergibt einen Mittelklick.
schema-input-scroll-speed-label = Scrollgeschwindigkeit
schema-input-scroll-speed-description = Software-Multiplikator für Scrollereignisse.
schema-input-pointer-accel-speed-label = Verfolgungsgeschwindigkeit
schema-input-pointer-accel-speed-description = Zeigerbeschleunigung, von -1 (langsamste) bis 1 (schnellste).
schema-input-pointer-accel-profile-label = Zeigerbeschleunigung
schema-input-pointer-accel-profile-description = „Konstante Geschwindigkeit“ ist die reine Geschwindigkeit, „Geschwindigkeit folgt der Bewegung“ folgt der Kurve von libinput.

# --- audio ---
schema-audio-sound-enabled-label = Oberflächenklänge
schema-audio-sound-enabled-description = Klangrückmeldung für Oberflächenereignisse abspielen.
schema-audio-sound-theme-label = Klangschema
schema-audio-sound-theme-description = Name des XDG-Klangschemas. Leer erkennt automatisch.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Deckelschalter verwalten
schema-power-management-manage-lid-switch-description = Otto reagiert auf den Deckel, statt es logind zu überlassen.
schema-power-management-on-lid-close-label = Beim Schließen des Deckels
schema-power-management-on-lid-close-description = Was geschieht, wenn der Laptop-Deckel geschlossen wird.
schema-power-management-on-power-button-label = Beim Drücken der Einschalttaste
schema-power-management-on-power-button-description = Was geschieht, wenn die Hardware-Einschalttaste gedrückt wird.

# --- lock ---
schema-lock-locker-command-label = Sperrbildschirm-Befehl
schema-lock-locker-command-description = Der Sperrbildschirm, der zum Sperren der Sitzung gestartet wird.
schema-lock-locker-args-label = Argumente für den Sperrbildschirm
schema-lock-locker-args-description = An den Sperrbildschirm übergebene Argumente.
schema-lock-auto-lock-timeout-label = Sperren nach
schema-lock-auto-lock-timeout-description = Sekunden der Inaktivität vor dem Sperren. 0 sperrt nie.

# --- login ---
schema-login-greeter-command-label = Anmeldebildschirm-Befehl
schema-login-greeter-command-description = Der im Anmeldemodus gestartete Anmeldebildschirm.
schema-login-greeter-args-label = Argumente für den Anmeldebildschirm
schema-login-greeter-args-description = An den Anmeldebildschirm übergebene Argumente.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = Umschalter folgt dem Zeiger
schema-appswitcher-follow-cursor-description = Fensterumschalter auf dem Bildschirm anzeigen, auf dem sich der Zeiger befindet.

## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automatisch


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Blau
settings-choice-accent-purple = Lila
settings-choice-accent-pink = Pink
settings-choice-accent-red = Rot
settings-choice-accent-orange = Orange
settings-choice-accent-yellow = Gelb
settings-choice-accent-green = Grün
settings-choice-accent-mint = Mint
settings-choice-accent-teal = Petrol
settings-choice-accent-cyan = Cyan
settings-choice-accent-indigo = Indigo
settings-choice-accent-brown = Braun
settings-choice-accent-graphite = Graphit
# The button under the shortcut list that adds another line.
settings-add-shortcut = Tastenkombination hinzufügen
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Schreibtisch { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Apps und Fenster durchsuchen…
launcher-search-apps = Apps durchsuchen…
launcher-search-windows = Fenster durchsuchen…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Fenster
launcher-badge-calc = Rechner


## Login, lock and authentication
##
## The greeter, the lock screen, and the panel both of them draw. Text that
## arrives from PAM or greetd at runtime is not here: those localise
## themselves, and restating them would be guessing at another program's words.
# Button under the login/lock card, offered only while the fingerprint reader
# is being waited on: it abandons the finger and asks for a password instead.
# The button sizes itself to the text, but it sits on a card 380pt wide — keep
# it to roughly 20 characters so it does not overhang.
auth-enter-password = Passwort eingeben

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Anmelden

# strftime pattern for the time in the large clock above the login/lock card,
# not prose: only the %-codes and the separators between them are yours.
# Change it where the local convention differs — %-I:%M %p for a 12-hour
# locale. The digits render at 46pt, so keep the result short.
auth-clock-time-format = %H:%M

# strftime pattern for the date under that clock, again not prose. Reorder the
# parts and change the punctuation to suit the locale (German would be
# "%A, %-d. %B"); the weekday and month names are translated by the system, so
# do not spell them out here. Renders at 15pt in a 360pt box.
auth-clock-date-format = %A, %-d. %B

### otto-greeter — the login screen shown before any session exists.
### Everything here is drawn on the login card, centred on the wallpaper.
### The card is narrow: prompts sit above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field while the login screen is asking who is logging
# in. One line, above a text field roughly 20 characters wide — keep it to one
# or two words.
greeter-prompt-username = Benutzername

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Passwort

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Authentifizierung…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Anmeldedienst nicht erreichbar: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Anmeldedienst nicht mehr verfügbar

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } wurde nicht gestartet

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Authentifiziert

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Finger auf den Sensor legen

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Finger über den Sensor ziehen

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = { $finger } auf den Sensor legen
greeter-status-swipe-named-finger = { $finger } über den Sensor ziehen

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Fingerabdruck nicht erkannt

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Warten auf den Fingerabdrucksensor…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Sitzung wird gestartet…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Ruhezustand nicht erlaubt

# As above, for the restart request.
greeter-power-restart-denied = Neustart nicht erlaubt

# As above, for the shut down request.
greeter-power-shutdown-denied = Herunterfahren nicht erlaubt

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Ruhezustand nicht möglich

# As above, for the restart request.
greeter-power-restart-failed = Neustart nicht möglich

# As above, for the shut down request.
greeter-power-shutdown-failed = Herunterfahren nicht möglich

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Passwort

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Authentifiziert

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Finger auf den Sensor legen

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Finger über den Sensor ziehen

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = { $finger } auf den Sensor legen
lock-status-swipe-named-finger = { $finger } über den Sensor ziehen

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = Linken Daumen
auth-finger-left-index = Linken Zeigefinger
auth-finger-left-middle = Linken Mittelfinger
auth-finger-left-ring = Linken Ringfinger
auth-finger-left-little = Linken kleinen Finger
auth-finger-right-thumb = Rechten Daumen
auth-finger-right-index = Rechten Zeigefinger
auth-finger-right-middle = Rechten Mittelfinger
auth-finger-right-ring = Rechten Ringfinger
auth-finger-right-little = Rechten kleinen Finger

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Fingerabdruck nicht erkannt

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Warten auf den Fingerabdrucksensor…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Kein Benutzer zur Authentifizierung

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Authentifizierungsdienst antwortet nicht

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Ungültiger Benutzername

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Authentifizierung nicht verfügbar

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Authentifizierung fehlgeschlagen ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Ruhezustand nicht erlaubt

# As above, for the restart request.
lock-power-restart-denied = Neustart nicht erlaubt

# As above, for the shut down request.
lock-power-shutdown-denied = Herunterfahren nicht erlaubt

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Ruhezustand nicht möglich: { $error }

# As above, for the restart request.
lock-power-restart-failed = Neustart nicht möglich: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Herunterfahren nicht möglich: { $error }


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
quickview-fact-kind = Art
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = Größe
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Abmessungen
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Dauer
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Pixel
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Seiten
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Titel
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Interpret
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Album
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Jahr

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Leere Datei
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Zu groß für eine Vorschau
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } Megapixel
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Eines davon installieren: { $packages } — dann erscheinen die Seiten


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Leerer Ordner
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } Objekt
       *[other] { $count } Objekte
    }
quickview-archive-summary = { $items } — { $size }


## Quick View — sizes
##
## Byte units. Quick View counts in powers of 1024, so the symbols are the
## conventional binary-rounded ones. Translate only the spelled-out "bytes".

quickview-size-bytes =
    { $count ->
        [one] { $count } Byte
       *[other] { $count } Bytes
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
quickview-error-not-previewable = dies ist keine Datei, die sich in der Vorschau zeigen lässt
# The file's metadata could not be read.
quickview-error-stat-file = die Dateiinformationen sind nicht lesbar: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = die Datei ist nicht lesbar: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = in der Datei lässt sich nicht zurückspringen
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = das Vorschauprogramm ließ sich nicht isolieren: { $error }

# Image previewer.
quickview-error-read-image = das Bild ist nicht lesbar: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = kein Bildformat, das dieser Build decodieren kann
quickview-error-image-no-size = das Bild gibt keine Größe an
quickview-error-image-decode = das Bild ließ sich nicht decodieren: { $error }
quickview-error-image-readback = das decodierte Bild ließ sich nicht zurücklesen

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = die Zeichnung ist nicht lesbar: { $error }
quickview-error-drawing-parse = die Zeichnung ließ sich nicht auswerten
quickview-error-drawing-surface = keine Fläche, um sie darzustellen
quickview-error-drawing-readback = die Zeichnung ließ sich nicht zurücklesen

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = diese Datei ist in keiner gelesenen Codierung Text

# PDF previewer.
quickview-error-read-document = das Dokument ist nicht lesbar: { $error }
quickview-error-page-readback = die gerenderte Seite ließ sich nicht lesen

# Folder listing.
quickview-error-read-folder = der Ordner ist nicht lesbar

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = das Vorschauprogramm ist nicht auffindbar: { $error }
quickview-error-previewer-start = das Vorschauprogramm ließ sich nicht starten: { $error }
quickview-error-previewer-no-output = das Vorschauprogramm lieferte keine Ausgabe
quickview-error-previewer-unreadable = das Vorschauprogramm lieferte etwas Unlesbares
quickview-error-previewer-failed = das Vorschauprogramm brach ab: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = diese Datei brauchte zu lange für eine Vorschau

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
islands-close = Schließen

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = gerade eben
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = vor { $count } Min.
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = vor { $count } Std.


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Erlauben
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Fortfahren
# Refuses the request.
islands-dialog-deny = Ablehnen


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Dock
a11y-app-running = Läuft
a11y-app-not-running = Läuft nicht
a11y-app-switcher = Programmumschalter
a11y-windows = Fenster
a11y-workspaces = Schreibtische
a11y-untitled-window = Fenster ohne Titel
a11y-menu-bar = Menüleiste
a11y-status = Status
a11y-tray-item = Symbol { $number }
a11y-notifications = Mitteilungen
a11y-categories = Kategorien
a11y-results = Ergebnisse
a11y-settings = Einstellungen
a11y-preview = Vorschau
a11y-preview-page = Vorschau, Seite { $page } von { $pages }
a11y-preview-shortened = Vorschau, gekürzt
