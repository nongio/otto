# Otto — Italian
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

common-open = Apri
common-save = Salva
common-cancel = Annulla
common-add = Aggiungi
common-remove = Rimuovi
common-quit = Esci
common-cut = Taglia
common-copy = Copia
common-paste = Incolla
common-rename = Rinomina
common-delete = Elimina
common-move = Sposta


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Nascondi automaticamente
dock-auto-hide-on = ✓ Nascondi automaticamente
dock-magnification = Ingrandimento
dock-magnification-on = ✓ Ingrandimento
dock-position-bottom = In basso
dock-position-bottom-on = ✓ In basso
dock-position-left = A sinistra
dock-position-left-on = ✓ A sinistra
dock-position-right = A destra
dock-position-right-on = ✓ A destra

# Shown on an app's icon when the app is not running.
dock-open = Apri
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Mantieni nel Dock
dock-keep-in-dock-on = ✓ Mantieni nel Dock
dock-quit = Esci


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Generali
settings-pane-displays = Monitor
settings-pane-dock = Dock
settings-pane-keyboard = Tastiera
settings-pane-pointing = Trackpad e mouse
settings-pane-sound = Suono
settings-pane-power = Energia
settings-pane-lock-and-login = Blocco e accesso


## Settings — General

settings-group-appearance = Aspetto
settings-colour-scheme = Schema colori
settings-accent-colour = Accento
settings-rounded-corners = Angoli arrotondati
settings-rounded-corners-detail = Si applica dopo il riavvio
settings-window-controls = Comandi della finestra
settings-font = Font di sistema
settings-gtk-theme = Tema GTK

settings-group-desktop = Scrivania
settings-background-colour = Colore di sfondo
settings-background-image = Immagine di sfondo
settings-background-image-detail = Scelta tramite il selettore file del portale del desktop

settings-group-pointer-and-icons = Puntatore e icone
settings-cursor-theme = Tema del cursore
settings-cursor-size = Dimensione del cursore
settings-icon-theme = Tema delle icone

settings-group-window-switcher = Cambio finestra
settings-follow-cursor = Mostra sul monitor del puntatore
settings-switcher-colorize-icons = Colora le icone come il Dock

settings-group-language = Lingua
settings-preferred-languages = Lingue preferite

settings-group-configuration = Configurazione
settings-configuration-file = File di configurazione
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = sconosciuto — il compositor non risponde


## Settings — Displays

settings-display-active = Attivo
settings-display-active-detail = Un monitor inattivo mantiene la propria posizione nella disposizione
settings-display-primary = Utilizza come principale
settings-display-primary-detail = Il Dock e la barra si trovano sul monitor principale
settings-display-x-position = Posizione X
settings-display-y-position = Posizione Y
settings-display-x-position-detail = Angolo superiore sinistro nello spazio di coordinate della scrivania
settings-display-width = Larghezza
settings-display-width-detail = Pixel. Un output headless può avere qualsiasi dimensione
settings-display-height = Altezza
settings-display-refresh = Frequenza di aggiornamento
settings-display-refresh-detail = Hertz — la frequenza con cui lo stream riceve un fotogramma
settings-display-resolution = Risoluzione
settings-display-scale = Scala del monitor
settings-display-scale-detail = Si applica al prossimo accesso. La scrivania non si aggiorna dinamicamente

# Shown when the compositor reports no outputs at all.
settings-display-none = Nessun monitor
settings-display-none-detail = Il compositor non sta gestendo alcun output

settings-virtual-displays = Monitor virtuali
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } output headless, trasmesso via PipeWire. Rimuovi elimina quello selezionato
        [many] { $count } output headless, trasmessi via PipeWire. Rimuovi elimina quello selezionato
       *[other] { $count } output headless, trasmessi via PipeWire. Rimuovi elimina quello selezionato
    }


## Settings — Dock

settings-dock-size = Dimensione
settings-dock-position = Posizione sullo schermo
settings-dock-autohide = Nascondi automaticamente
settings-dock-magnification = Ingrandimento
settings-group-magnification-and-icons = Ingrandimento e icone
settings-dock-magnification-amount = Livello di ingrandimento
settings-dock-tint-icons = Colora le icone
settings-dock-icon-tint = Tinta delle icone
settings-dock-icon-tint-strength = Intensità della tinta


## Settings — Keyboard

settings-key-repeat-delay = Ritardo di ripetizione dei tasti
settings-key-repeat-rate = Velocità di ripetizione dei tasti
settings-group-input-source = Sorgente di input
settings-xkb-layout = Disposizione
settings-xkb-variant = Variante
settings-xkb-options = Opzioni
settings-group-shortcuts = Scorciatoie
settings-key-combination = Combinazione di tasti
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift o Logo uniti da +, poi un tasto: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Trackpad
settings-tap-to-click = Tocca per fare clic
settings-tap-and-drag = Tocca e trascina
settings-drag-lock = Blocco trascinamento
settings-click-method = Metodo di clic
settings-ignore-while-typing = Ignora durante la digitazione
settings-natural-scrolling = Scorrimento naturale
settings-left-handed = Mancino
settings-middle-click-emulation = Emulazione del clic centrale
settings-group-pointer = Puntatore
settings-tracking-speed = Velocità di tracciamento
settings-pointer-acceleration = Accelerazione
settings-scrolling-speed = Velocità di scorrimento


## Settings — Sound

settings-interface-sounds = Suoni dell'interfaccia
settings-sound-theme = Tema sonoro


## Settings — Power

settings-manage-lid-switch = Gestisci la chiusura del coperchio
settings-manage-lid-switch-detail = Otto sospende alla chiusura del coperchio al posto di logind
settings-on-lid-close = Alla chiusura del coperchio
settings-on-power-button = Alla pressione del pulsante di accensione


## Settings — Lock & Login

settings-group-lock = Blocco
settings-lock-after = Blocca dopo
settings-lock-screen = Blocco schermo
settings-lock-screen-detail = Si applica al prossimo blocco dello schermo
settings-lock-screen-arguments = Argomenti del blocco schermo
settings-group-login = Accesso
settings-greeter = Schermata di accesso
settings-greeter-detail = Si applica al prossimo accesso
settings-greeter-arguments = Argomenti della schermata di accesso


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Chiaro
settings-choice-dark = Scuro
settings-choice-controls-left = A sinistra
settings-choice-controls-right = A destra
settings-choice-position-bottom = In basso
settings-choice-position-left = A sinistra
settings-choice-position-right = A destra
settings-choice-clickfinger = Clic con le dita
settings-choice-buttonareas = Clic negli angoli
settings-choice-accel-flat = Velocità costante
settings-choice-accel-adaptive = Velocità in base al movimento
settings-choice-lid-auto = Decisione automatica
settings-choice-lid-lock = Blocca lo schermo
settings-choice-lid-disable-internal = Spegni il monitor integrato
settings-choice-power-ignore = Non fare nulla
settings-choice-power-lock = Blocca lo schermo
settings-choice-power-suspend = Sospendi
settings-choice-power-shutdown = Arresta il sistema
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

files-window-title = File
# The Get Info panel's own window.
files-info-window-title = Informazioni


## Files — commands

files-get-info = Ottieni informazioni
files-new-folder = Nuova cartella
files-move-to-trash = Sposta nel Cestino
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Sposta { $count } elemento nel Cestino
        [many] Sposta { $count } elementi nel Cestino
       *[other] Sposta { $count } elementi nel Cestino
    }


## Files — sidebar and columns

files-places = Risorse
files-home = Home
files-desktop = Scrivania
files-documents = Documenti
files-downloads = Download
files-music = Musica
files-pictures = Immagini
files-videos = Video
files-trash = Cestino

files-column-name = Nome
files-column-size = Dimensione
files-column-kind = Tipo
files-column-date-modified = Data di modifica


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Cartella
files-kind-image = Immagine
files-kind-movie = Filmato
files-kind-audio = Audio
files-kind-text = Testo
files-kind-document = Documento
files-kind-archive = Archivio
files-kind-application = Applicazione


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Caricamento…
files-empty = Vuoto
# The idle line: what the folder holds.
files-status-no-items = Nessun elemento
files-status-items =
    { $count ->
        [one] { $count } elemento
        [many] { $count } elementi
       *[other] { $count } elementi
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, { $hidden } nascosti
files-status-selected = { $count } di { $total } selezionati
files-status-opening-preview = Apertura anteprima…
files-nothing-to-undo = Niente da annullare
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Annullato: { $label }
files-undo-move = Spostamento
files-undo-copy = Copia
files-undo-delete = Eliminazione
files-undo-rename = Ridenominazione
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Rinominato in “{ $name }”
files-new-folder-created = Nuova cartella “{ $name }”
files-gone = “{ $name }” non è più presente
files-rename-failed = Impossibile rinominare: { $error }
files-new-folder-failed = Impossibile creare la cartella: { $error }
files-open-failed = Impossibile aprire il file: { $error }
files-new-window-failed = Impossibile aprire una nuova finestra: { $error }


## Files — the listing

files-folder-empty = Questa cartella è vuota.
files-folder-denied = Permessi non sufficienti per vedere il contenuto di questa cartella.
files-folder-gone = Questa cartella non esiste più.
files-folder-open-failed = Impossibile aprire questa cartella: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Dove
files-info-kind = Tipo
files-info-modified = Modificato
files-info-created = Creato
files-info-accessed = Ultimo accesso
files-info-owner = Proprietario
files-info-links-to = Collegamento a
files-info-permissions = Permessi
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Lettura
files-perm-write = Scrittura
files-perm-exec = Esecuzione
# Row labels: who each set of permissions applies to.
files-perm-owner = Proprietario
files-perm-group = Gruppo
files-perm-everyone = Tutti

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Apri
files-picker-save-as = Salva con nome
files-picker-save-files = Salva i file
files-picker-all-files = Tutti i file
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Salva con nome:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Inserisci un nome
files-save-name-has-slash = Un nome non può contenere “/”
files-save-name-reserved = Quel nome è riservato
files-save-nowhere = Nessuna posizione in cui salvare
files-save-permission-denied = Permessi non sufficienti per salvare qui

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = “{ $name }” esiste già. Sostituire?
files-replace-one-detail = Sostituendolo, il contenuto attuale viene sovrascritto.
files-replace-many = { $count } di questi file esistono già. Sostituirli?
files-replace-many-detail = Sostituendoli, i contenuti attuali vengono sovrascritti.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } byte
        [many] { $count } byte
       *[other] { $count } byte
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

files-date-modified = { $day } { $month } { $year } alle { $time }

files-month-jan = Gen
files-month-feb = Feb
files-month-mar = Mar
files-month-apr = Apr
files-month-may = Mag
files-month-jun = Giu
files-month-jul = Lug
files-month-aug = Ago
files-month-sep = Set
files-month-oct = Ott
files-month-nov = Nov
files-month-dec = Dic


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
settings-not-set = Non impostato
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Scegli…
settings-no-file-chosen = Nessun file scelto
settings-choose-background-image = Scegli un'immagine di sfondo

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Impostazioni di Otto — { $pane }

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
schema-screen-scale-label = Scala del display
schema-screen-scale-description = Fattore di scala globale applicato alla scrivania.
schema-theme-scheme-label = Schema colori
schema-theme-scheme-description = Schema colori chiaro o scuro.
schema-accent-color-label = Accento
schema-accent-color-description = Colore di accento usato dall'interfaccia di Otto: un nome della tavolozza, che segue gli schemi colori chiaro e scuro, o un colore #RRGGBB.
schema-rounded-corners-label = Angoli arrotondati
schema-rounded-corners-description = Arrotonda gli angoli del Dock, della barra superiore, delle decorazioni delle finestre e dei pannelli disegnati dalla scrivania stessa.
schema-window-controls-side-label = Comandi della finestra
schema-window-controls-side-description = A quale estremità della barra del titolo si trovano i comandi chiudi, riduci a icona e zoom.
schema-font-family-label = Carattere dell'interfaccia
schema-font-family-description = Famiglia di caratteri usata dall'interfaccia di Otto.
schema-background-color-label = Colore di sfondo
schema-background-color-description = Colore di sfondo della scrivania, come stringa esadecimale.
schema-background-image-label = Immagine di sfondo
schema-background-image-description = Percorso dell'immagine di sfondo della scrivania. Vuoto per nessuna.
schema-cursor-theme-label = Tema del cursore
schema-cursor-theme-description = Nome del tema XCursor.
schema-cursor-size-label = Dimensione del cursore
schema-cursor-size-description = Dimensione del cursore in pixel logici.
schema-icon-theme-label = Tema delle icone
schema-icon-theme-description = Nome del tema delle icone. Vuoto per il rilevamento automatico.
schema-gtk-theme-label = Tema GTK
schema-gtk-theme-description = Nome del tema GTK fornito ai client. Vuoto per il rilevamento automatico.
schema-locales-label = Lingue
schema-locales-description = Lingue preferite, in ordine di preferenza.

# --- dock ---
schema-dock-size-label = Dimensione
schema-dock-size-description = Moltiplicatore delle dimensioni del Dock.
schema-dock-position-label = Posizione sullo schermo
schema-dock-position-description = Bordo dello schermo su cui si trova il Dock.
schema-dock-autohide-label = Nascondi automaticamente
schema-dock-autohide-description = Nasconde il Dock finché il puntatore non raggiunge il bordo dello schermo.
schema-dock-magnification-label = Ingrandimento
schema-dock-magnification-description = Ingrandisce le icone sotto il puntatore.
schema-dock-genie-scale-label = Livello di ingrandimento
schema-dock-genie-scale-description = Di quanto crescono le icone sotto il puntatore.
schema-dock-genie-span-label = Estensione dell'ingrandimento
schema-dock-genie-span-description = A quante icone vicine arriva l'ingrandimento.
schema-dock-colorize-icons-label = Colora le icone
schema-dock-colorize-icons-description = Colora le icone del Dock con un unico colore.
schema-dock-colorize-color-label = Tinta delle icone
schema-dock-colorize-color-description = Colore usato per tingere le icone del Dock, come stringa esadecimale.
schema-dock-colorize-intensity-label = Intensità della tinta
schema-dock-colorize-intensity-description = Con quale intensità viene applicata la tinta.

# --- general ---
schema-keyboard-repeat-delay-label = Ritardo di ripetizione dei tasti
schema-keyboard-repeat-delay-description = Millisecondi di pressione di un tasto prima che inizi a ripetersi.
schema-keyboard-repeat-rate-label = Velocità di ripetizione dei tasti
schema-keyboard-repeat-rate-description = Ripetizioni al secondo mentre un tasto resta premuto.

# --- input ---
schema-input-xkb-layout-label = Disposizione della tastiera
schema-input-xkb-layout-description = Nome della disposizione XKB. Vuoto per usare quella predefinita di sistema.
schema-input-xkb-variant-label = Variante della tastiera
schema-input-xkb-variant-description = Nome della variante XKB. Vuoto per usare quella predefinita di sistema.
schema-input-xkb-options-label = Opzioni della tastiera
schema-input-xkb-options-description = Stringhe di opzioni XKB.
schema-input-tap-enabled-label = Tocca per fare clic
schema-input-tap-enabled-description = Considera un tocco sul trackpad come un clic.
schema-input-tap-drag-enabled-label = Tocca e trascina
schema-input-tap-drag-enabled-description = Avvia un trascinamento da un tocco seguito da un contatto prolungato.
schema-input-tap-drag-lock-enabled-label = Blocco trascinamento
schema-input-tap-drag-lock-enabled-description = Mantiene attivo il tocca-e-trascina anche dopo un breve sollevamento del dito.
schema-input-touchpad-click-method-label = Metodo di clic
schema-input-touchpad-click-method-description = Se un clic è determinato dal numero di dita o dalle aree dei pulsanti.
schema-input-touchpad-dwt-enabled-label = Ignora durante la digitazione
schema-input-touchpad-dwt-enabled-description = Ignora il trackpad mentre la tastiera è in uso.
schema-input-touchpad-natural-scroll-enabled-label = Scorrimento naturale
schema-input-touchpad-natural-scroll-enabled-description = Il contenuto segue le dita.
schema-input-touchpad-left-handed-label = Mancino
schema-input-touchpad-left-handed-description = Scambia il pulsante primario con quello secondario.
schema-input-touchpad-middle-emulation-enabled-label = Emulazione del clic centrale
schema-input-touchpad-middle-emulation-enabled-description = Premere entrambi i pulsanti insieme equivale a un clic centrale.
schema-input-scroll-speed-label = Velocità di scorrimento
schema-input-scroll-speed-description = Moltiplicatore software applicato agli eventi di scorrimento.
schema-input-pointer-accel-speed-label = Velocità del puntatore
schema-input-pointer-accel-speed-description = Accelerazione del puntatore, da -1 (più lenta) a 1 (più veloce).
schema-input-pointer-accel-profile-label = Accelerazione del puntatore
schema-input-pointer-accel-profile-description = Costante corrisponde alla velocità grezza; adattiva segue la curva di libinput.

# --- audio ---
schema-audio-sound-enabled-label = Suoni dell'interfaccia
schema-audio-sound-enabled-description = Riproduce un suono di conferma per gli eventi dell'interfaccia.
schema-audio-sound-theme-label = Tema sonoro
schema-audio-sound-theme-description = Nome del tema sonoro XDG. Vuoto per il rilevamento automatico.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Gestisci la chiusura del coperchio
schema-power-management-manage-lid-switch-description = Lascia che sia Otto a occuparsi del coperchio invece di logind.
schema-power-management-on-lid-close-label = Alla chiusura del coperchio
schema-power-management-on-lid-close-description = Cosa succede quando il coperchio del portatile viene chiuso.
schema-power-management-on-power-button-label = Alla pressione del pulsante di accensione
schema-power-management-on-power-button-description = Cosa succede quando viene premuto il pulsante fisico di accensione.

# --- lock ---
schema-lock-locker-command-label = Comando di blocco schermo
schema-lock-locker-command-description = Il programma di blocco avviato per bloccare la sessione.
schema-lock-locker-args-label = Argomenti del blocco schermo
schema-lock-locker-args-description = Argomenti passati al programma di blocco.
schema-lock-auto-lock-timeout-label = Blocca dopo
schema-lock-auto-lock-timeout-description = Secondi di inattività prima del blocco. 0 non blocca mai.

# --- login ---
schema-login-greeter-command-label = Comando della schermata di accesso
schema-login-greeter-command-description = Il programma di accesso avviato in modalità login.
schema-login-greeter-args-label = Argomenti della schermata di accesso
schema-login-greeter-args-description = Argomenti passati alla schermata di accesso.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = Il selettore segue il puntatore
schema-appswitcher-follow-cursor-description = Mostra il cambio finestra sul monitor in cui si trova il puntatore.
schema-appswitcher-colorize-icons-label = Colora le icone del selettore
schema-appswitcher-colorize-icons-description = Applica anche al cambio finestra la tinta delle icone del Dock. Non fa nulla finché la tinta del Dock è disattivata.

## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automatico


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Blu
settings-choice-accent-purple = Viola
settings-choice-accent-pink = Rosa
settings-choice-accent-red = Rosso
settings-choice-accent-orange = Arancione
settings-choice-accent-yellow = Giallo
settings-choice-accent-green = Verde
settings-choice-accent-mint = Menta
settings-choice-accent-teal = Verde acqua
settings-choice-accent-cyan = Ciano
settings-choice-accent-indigo = Indaco
settings-choice-accent-brown = Marrone
settings-choice-accent-graphite = Grafite
# The button under the shortcut list that adds another line.
settings-add-shortcut = Aggiungi scorciatoia
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Scrivania { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Cerca app e finestre…
launcher-search-apps = Cerca app…
launcher-search-windows = Cerca finestre…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Finestra
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
auth-enter-password = Inserisci password

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Accedi

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
greeter-prompt-username = Nome utente

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Password

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Autenticazione…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Servizio di accesso non disponibile: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Servizio di accesso interrotto

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } non è stato avviato

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Autenticato

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Appoggia il dito sul lettore

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Fai scorrere il dito sul lettore

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Appoggia { $finger } sul lettore
greeter-status-swipe-named-finger = Fai scorrere { $finger } sul lettore

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Impronta non riconosciuta

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = In attesa del lettore di impronte…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Avvio della sessione…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Sospensione non consentita

# As above, for the restart request.
greeter-power-restart-denied = Riavvio non consentito

# As above, for the shut down request.
greeter-power-shutdown-denied = Arresto non consentito

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Sospensione non riuscita

# As above, for the restart request.
greeter-power-restart-failed = Riavvio non riuscito

# As above, for the shut down request.
greeter-power-shutdown-failed = Arresto non riuscito

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
lock-status-authenticated = Autenticato

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Appoggia il dito sul lettore

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Fai scorrere il dito sul lettore

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Appoggia { $finger } sul lettore
lock-status-swipe-named-finger = Fai scorrere { $finger } sul lettore

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = il pollice sinistro
auth-finger-left-index = l'indice sinistro
auth-finger-left-middle = il medio sinistro
auth-finger-left-ring = l'anulare sinistro
auth-finger-left-little = il mignolo sinistro
auth-finger-right-thumb = il pollice destro
auth-finger-right-index = l'indice destro
auth-finger-right-middle = il medio destro
auth-finger-right-ring = l'anulare destro
auth-finger-right-little = il mignolo destro

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Impronta non riconosciuta

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = In attesa del lettore di impronte…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Nessun utente da autenticare

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Servizio di autenticazione interrotto

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Nome utente non valido

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Autenticazione non disponibile

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Autenticazione non riuscita ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Sospensione non consentita

# As above, for the restart request.
lock-power-restart-denied = Riavvio non consentito

# As above, for the shut down request.
lock-power-shutdown-denied = Arresto non consentito

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Sospensione non riuscita: { $error }

# As above, for the restart request.
lock-power-restart-failed = Riavvio non riuscito: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Arresto non riuscito: { $error }


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
quickview-fact-kind = Tipo
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = Dimensione
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Dimensioni
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Durata
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Pixel
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Pagine
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Titolo
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Artista
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Album
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Anno

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = File vuoto
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Troppo grande per l’anteprima
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } megapixel
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Installa uno di questi: { $packages } — per vedere le pagine


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Cartella vuota
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } elemento
        [many] { $count } elementi
       *[other] { $count } elementi
    }
quickview-archive-summary = { $items } — { $size }


## Quick View — sizes
##
## Byte units. Quick View counts in powers of 1024, so the symbols are the
## conventional binary-rounded ones. Translate only the spelled-out "bytes".

quickview-size-bytes =
    { $count ->
        [one] { $count } byte
        [many] { $count } byte
       *[other] { $count } byte
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
quickview-error-not-previewable = non è un file di cui si possa mostrare l’anteprima
# The file's metadata could not be read.
quickview-error-stat-file = impossibile leggere i dati del file: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = impossibile leggere il file: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = il file non consente il riposizionamento
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = impossibile isolare il visualizzatore: { $error }

# Image previewer.
quickview-error-read-image = impossibile leggere l’immagine: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = non è un’immagine che questa build sappia decodificare
quickview-error-image-no-size = l’immagine non dichiara alcuna dimensione
quickview-error-image-decode = l’immagine non si è decodificata: { $error }
quickview-error-image-readback = impossibile rileggere l’immagine decodificata

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = impossibile leggere il disegno: { $error }
quickview-error-drawing-parse = impossibile analizzare il disegno
quickview-error-drawing-surface = nessuna superficie su cui visualizzarlo
quickview-error-drawing-readback = impossibile rileggere il disegno

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = questo file non è testo in nessuna codifica che si sappia leggere

# PDF previewer.
quickview-error-read-document = impossibile leggere il documento: { $error }
quickview-error-page-readback = impossibile leggere la pagina generata

# Folder listing.
quickview-error-read-folder = impossibile leggere la cartella

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = impossibile trovare il visualizzatore: { $error }
quickview-error-previewer-start = impossibile avviare il visualizzatore: { $error }
quickview-error-previewer-no-output = il visualizzatore non ha prodotto nulla
quickview-error-previewer-unreadable = il visualizzatore ha prodotto qualcosa di illeggibile
quickview-error-previewer-failed = il visualizzatore si è interrotto: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = questo file ha richiesto troppo tempo per l’anteprima

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
islands-close = Chiudi

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = adesso
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count } min fa
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count } h fa


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Consenti
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Continua
# Refuses the request.
islands-dialog-deny = Nega


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Dock
a11y-app-running = In esecuzione
a11y-app-not-running = Non in esecuzione
a11y-app-switcher = Selettore di applicazioni
a11y-windows = Finestre
a11y-workspaces = Scrivanie
a11y-untitled-window = Finestra senza titolo
a11y-menu-bar = Barra dei menu
a11y-status = Stato
a11y-tray-item = Elemento { $number }
a11y-notifications = Notifiche
a11y-categories = Categorie
a11y-results = Risultati
a11y-settings = Impostazioni
a11y-preview = Anteprima
a11y-preview-page = Anteprima, pagina { $page } di { $pages }
a11y-preview-shortened = Anteprima, abbreviata
