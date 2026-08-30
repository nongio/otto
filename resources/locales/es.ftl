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

common-open = Abrir
common-save = Guardar
common-cancel = Cancelar
common-add = Añadir
common-remove = Quitar
common-quit = Salir
common-cut = Cortar
common-copy = Copiar
common-paste = Pegar
common-rename = Cambiar nombre
common-delete = Eliminar
common-move = Mover


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Ocultar automáticamente
dock-auto-hide-on = ✓ Ocultar automáticamente
dock-magnification = Aumento
dock-magnification-on = ✓ Aumento
dock-position-bottom = Abajo
dock-position-bottom-on = ✓ Abajo
dock-position-left = Izquierda
dock-position-left-on = ✓ Izquierda
dock-position-right = Derecha
dock-position-right-on = ✓ Derecha

# Shown on an app's icon when the app is not running.
dock-open = Abrir
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Mantener en el Dock
dock-keep-in-dock-on = ✓ Mantener en el Dock
dock-quit = Salir


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = General
settings-pane-displays = Pantallas
settings-pane-dock = Dock
settings-pane-keyboard = Teclado
settings-pane-pointing = Trackpad y ratón
settings-pane-sound = Sonido
settings-pane-power = Energía
settings-pane-lock-and-login = Bloqueo e inicio de sesión


## Settings — General

settings-group-appearance = Apariencia
settings-colour-scheme = Esquema de color
settings-accent-colour = Color de acento
settings-rounded-corners = Esquinas redondeadas
settings-rounded-corners-detail = Se aplica tras reiniciar
settings-window-controls = Controles de ventana
settings-maximize-button = Botón de maximizar
settings-maximize-button-detail = Muestra el punto de ampliar; un doble clic en la barra de título también amplía
settings-font = Tipo de letra del sistema
settings-gtk-theme = Tema GTK

settings-group-desktop = Escritorio
settings-background-colour = Color de fondo
settings-background-image = Imagen de fondo
settings-background-image-detail = Elegida a través del selector de archivos del portal de escritorio

settings-group-pointer-and-icons = Puntero e iconos
settings-cursor-theme = Tema del cursor
settings-cursor-size = Tamaño del cursor
settings-icon-theme = Tema de iconos

settings-group-window-switcher = Selector de ventanas
settings-follow-cursor = Mostrar en la pantalla del puntero

settings-group-language = Idioma
settings-preferred-languages = Idiomas preferidos

settings-group-configuration = Configuración
settings-configuration-file = Archivo de configuración
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = desconocido: el compositor no responde


## Settings — Displays

settings-display-active = Activa
settings-display-active-detail = Una pantalla inactiva conserva su lugar en la disposición
settings-display-primary = Usar como principal
settings-display-primary-detail = El dock y la barra están en la pantalla principal
settings-display-x-position = Posición X
settings-display-y-position = Posición Y
settings-display-x-position-detail = Esquina superior izquierda en el espacio de coordenadas del escritorio
settings-display-width = Anchura
settings-display-width-detail = Píxeles. Una salida sin cabezal puede tener cualquier tamaño
settings-display-height = Altura
settings-display-refresh = Frecuencia de actualización
settings-display-refresh-detail = Hercios: cada cuánto se envía un fotograma a la transmisión
settings-display-resolution = Resolución
settings-display-scale = Escala de la pantalla
settings-display-scale-detail = Se aplica en el siguiente inicio de sesión. El escritorio no se reajusta en vivo

# Shown when the compositor reports no outputs at all.
settings-display-none = Sin pantallas
settings-display-none-detail = El compositor no controla ninguna salida

settings-virtual-displays = Pantallas virtuales
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } salida sin cabezal, transmitida por PipeWire. Quitar retira la seleccionada
        [many] { $count } salidas sin cabezal, transmitidas por PipeWire. Quitar retira la seleccionada
       *[other] { $count } salidas sin cabezal, transmitidas por PipeWire. Quitar retira la seleccionada
    }


## Settings — Dock

settings-dock-size = Tamaño
settings-dock-position = Posición en la pantalla
settings-dock-autohide = Ocultar automáticamente
settings-dock-magnification = Aumento
settings-group-magnification-and-icons = Aumento e iconos
settings-dock-magnification-amount = Nivel de aumento
settings-dock-tint-icons = Teñir iconos
settings-switcher-colorize-icons = Teñir el selector
settings-dock-icon-tint = Tinte de los iconos
settings-dock-icon-tint-strength = Intensidad del tinte


## Settings — Keyboard

settings-key-repeat-delay = Retardo de repetición de tecla
settings-key-repeat-rate = Velocidad de repetición de tecla
settings-group-input-source = Fuente de entrada
settings-xkb-layout = Distribución
settings-xkb-variant = Variante
settings-xkb-options = Opciones
settings-group-shortcuts = Atajos
settings-key-combination = Combinación de teclas
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift o Logo unidos con +, seguidos de una tecla: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Trackpad
settings-tap-to-click = Tocar para pulsar
settings-tap-and-drag = Tocar y arrastrar
settings-drag-lock = Bloqueo de arrastre
settings-click-method = Método de clic
settings-ignore-while-typing = Ignorar mientras se escribe
settings-natural-scrolling = Desplazamiento natural
settings-left-handed = Zurdo
settings-middle-click-emulation = Emulación de clic central
settings-group-pointer = Puntero
settings-tracking-speed = Velocidad de seguimiento
settings-pointer-acceleration = Aceleración
settings-scrolling-speed = Velocidad de desplazamiento


## Settings — Sound

settings-interface-sounds = Sonidos de la interfaz
settings-sound-theme = Tema de sonido


## Settings — Power

settings-manage-lid-switch = Gestionar el interruptor de la tapa
settings-manage-lid-switch-detail = Otto suspende al cerrar la tapa en lugar de logind
settings-on-lid-close = Al cerrar la tapa
settings-on-power-button = Al pulsar el botón de encendido


## Settings — Lock & Login

settings-group-lock = Bloqueo
settings-lock-after = Bloquear tras
settings-lock-screen = Pantalla de bloqueo
settings-lock-screen-detail = Se aplica la próxima vez que se bloquee la pantalla
settings-lock-screen-arguments = Argumentos de la pantalla de bloqueo
settings-group-login = Inicio de sesión
settings-greeter = Pantalla de bienvenida
settings-greeter-detail = Se aplica en el siguiente inicio de sesión
settings-greeter-arguments = Argumentos de la pantalla de bienvenida


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Claro
settings-choice-dark = Oscuro
settings-choice-controls-left = Izquierda
settings-choice-controls-right = Derecha
settings-choice-position-bottom = Abajo
settings-choice-position-left = Izquierda
settings-choice-position-right = Derecha
settings-choice-clickfinger = Pulsar con los dedos
settings-choice-buttonareas = Pulsar en las esquinas
settings-choice-accel-flat = Velocidad constante
settings-choice-accel-adaptive = Velocidad según el movimiento
settings-choice-lid-auto = Decidir automáticamente
settings-choice-lid-lock = Bloquear la pantalla
settings-choice-lid-disable-internal = Apagar la pantalla integrada
settings-choice-power-ignore = No hacer nada
settings-choice-power-lock = Bloquear la pantalla
settings-choice-power-suspend = Suspender
settings-choice-power-shutdown = Apagar
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

files-window-title = Archivos
# The Get Info panel's own window.
files-info-window-title = Información


## Files — commands

files-get-info = Obtener información
files-new-folder = Nueva carpeta
files-move-to-trash = Mover a la papelera
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Mover { $count } elemento a la papelera
        [many] Mover { $count } elementos a la papelera
       *[other] Mover { $count } elementos a la papelera
    }


## Files — sidebar and columns

files-places = Lugares
files-home = Carpeta personal
files-desktop = Escritorio
files-documents = Documentos
files-downloads = Descargas
files-music = Música
files-pictures = Imágenes
files-videos = Vídeos
files-trash = Papelera

files-column-name = Nombre
files-column-size = Tamaño
files-column-kind = Tipo
files-column-date-modified = Fecha de modificación


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Carpeta
files-kind-image = Imagen
files-kind-movie = Película
files-kind-audio = Audio
files-kind-text = Texto
files-kind-document = Documento
files-kind-archive = Archivo comprimido
files-kind-application = Aplicación


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Cargando…
files-empty = Vacía
# The idle line: what the folder holds.
files-status-no-items = Sin elementos
files-status-items =
    { $count ->
        [one] { $count } elemento
        [many] { $count } elementos
       *[other] { $count } elementos
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, { $hidden } ocultos
files-status-selected = { $count } de { $total } seleccionados
files-status-opening-preview = Abriendo la previsualización…
files-nothing-to-undo = Nada que deshacer
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Se deshizo: { $label }
files-undo-move = Mover
files-undo-copy = Copiar
files-undo-delete = Eliminar
files-undo-rename = Cambiar nombre
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Nombre cambiado a «{ $name }»
files-new-folder-created = Nueva carpeta «{ $name }»
files-gone = «{ $name }» ya no está ahí
files-rename-failed = No se pudo cambiar el nombre: { $error }
files-new-folder-failed = No se pudo crear la carpeta: { $error }
files-open-failed = No se pudo abrir ese archivo: { $error }
files-new-window-failed = No se pudo abrir una nueva ventana: { $error }


## Files — the listing

files-folder-empty = Esta carpeta está vacía.
files-folder-denied = No hay permisos para ver el contenido de esta carpeta.
files-folder-gone = Esta carpeta ya no existe.
files-folder-open-failed = No se pudo abrir esta carpeta: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Ubicación
files-info-kind = Tipo
files-info-modified = Modificado
files-info-created = Creado
files-info-accessed = Último acceso
files-info-owner = Propietario
files-info-links-to = Enlaza a
files-info-permissions = Permisos
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Lectura
files-perm-write = Escritura
files-perm-exec = Ejecución
# Row labels: who each set of permissions applies to.
files-perm-owner = Propietario
files-perm-group = Grupo
files-perm-everyone = Todos

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Abrir
files-picker-save-as = Guardar como
files-picker-save-files = Guardar archivos
files-picker-all-files = Todos los archivos
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Guardar como:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Introduzca un nombre
files-save-name-has-slash = Un nombre no puede contener «/»
files-save-name-reserved = Ese nombre está reservado
files-save-nowhere = No hay dónde guardar
files-save-permission-denied = No hay permisos para guardar aquí

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = «{ $name }» ya existe. ¿Reemplazar?
files-replace-one-detail = Al reemplazarlo se sobrescribe su contenido actual.
files-replace-many = { $count } de estos archivos ya existen. ¿Reemplazarlos?
files-replace-many-detail = Al reemplazarlos se sobrescribe su contenido actual.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } byte
        [many] { $count } bytes
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

files-date-modified = { $day } { $month } { $year }, { $time }

files-month-jan = ene
files-month-feb = feb
files-month-mar = mar
files-month-apr = abr
files-month-may = may
files-month-jun = jun
files-month-jul = jul
files-month-aug = ago
files-month-sep = sep
files-month-oct = oct
files-month-nov = nov
files-month-dec = dic


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %A %-d de %B  %H:%M


## Settings — widgets
##
## The controls themselves, rather than the settings they edit.

# Shown in a text field that has no value yet.
settings-not-set = No establecido
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Elegir…
settings-no-file-chosen = Ningún archivo elegido
settings-choose-background-image = Elegir una imagen de fondo

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Configuración de Otto — { $pane }

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
schema-screen-scale-label = Escala de pantalla
schema-screen-scale-description = Factor de escala global aplicado al escritorio.
schema-theme-scheme-label = Esquema de color
schema-theme-scheme-description = Esquema de colores claro u oscuro.
schema-accent-color-label = Color de acento
schema-accent-color-description = Un nombre de la paleta, que sigue los esquemas claro y oscuro, o un color #RRGGBB.
schema-rounded-corners-label = Esquinas redondeadas
schema-rounded-corners-description = El Dock, la barra superior, las decoraciones de ventana y los paneles del escritorio.
schema-window-controls-side-label = Controles de ventana
schema-window-controls-side-description = En qué extremo de la barra de título están los controles de cerrar, minimizar y ampliar.
schema-show-maximize-button-label = Botón de maximizar
schema-show-maximize-button-description = Muestra el control de ampliar en la barra de título de una ventana. Desactivado por defecto: un doble clic en la barra de título amplía la ventana igualmente.
schema-font-family-label = Tipo de letra de la interfaz
schema-font-family-description = Familia tipográfica usada por la propia interfaz de Otto.
schema-background-color-label = Color de fondo
schema-background-color-description = Color de fondo del escritorio, en formato hexadecimal.
schema-background-image-label = Imagen de fondo
schema-background-image-description = Ruta de la imagen de fondo del escritorio. Vacío para ninguna.
schema-cursor-theme-label = Tema del cursor
schema-cursor-theme-description = Nombre del tema XCursor.
schema-cursor-size-label = Tamaño del cursor
schema-cursor-size-description = Tamaño del cursor en píxeles lógicos.
schema-icon-theme-label = Tema de iconos
schema-icon-theme-description = Nombre del tema de iconos. Vacío para detección automática.
schema-gtk-theme-label = Tema GTK
schema-gtk-theme-description = Nombre del tema GTK proporcionado a los clientes. Vacío para detección automática.
schema-locales-label = Idiomas
schema-locales-description = Idiomas preferidos, en orden de preferencia.

# --- dock ---
schema-dock-size-label = Tamaño
schema-dock-size-description = Multiplicador del tamaño del Dock.
schema-dock-position-label = Posición en la pantalla
schema-dock-position-description = Borde de la pantalla en el que se encuentra el Dock.
schema-dock-autohide-label = Ocultar automáticamente
schema-dock-autohide-description = Oculta el Dock hasta que el puntero alcanza su borde de la pantalla.
schema-dock-magnification-label = Aumento
schema-dock-magnification-description = Aumenta el tamaño de los iconos bajo el puntero.
schema-dock-genie-scale-label = Nivel de aumento
schema-dock-genie-scale-description = Cuánto crecen los iconos bajo el puntero.
schema-dock-genie-span-label = Alcance del aumento
schema-dock-genie-span-description = A cuántos iconos vecinos llega el aumento.
schema-dock-colorize-icons-label = Teñir iconos
schema-dock-colorize-icons-description = Tiñe los iconos del Dock con un único color.
schema-dock-colorize-color-label = Tinte de los iconos
schema-dock-colorize-color-description = Color usado para teñir los iconos del Dock, en formato hexadecimal.
schema-dock-colorize-intensity-label = Intensidad del tinte
schema-dock-colorize-intensity-description = Con qué intensidad se aplica el tinte.

# --- general ---
schema-keyboard-repeat-delay-label = Retardo de repetición de tecla
schema-keyboard-repeat-delay-description = Milisegundos que se mantiene pulsada una tecla antes de empezar a repetirse.
schema-keyboard-repeat-rate-label = Velocidad de repetición de tecla
schema-keyboard-repeat-rate-description = Repeticiones por segundo mientras se mantiene pulsada una tecla.

# --- input ---
schema-input-xkb-layout-label = Distribución del teclado
schema-input-xkb-layout-description = Nombre de la distribución XKB. Vacío para usar la predeterminada del sistema.
schema-input-xkb-variant-label = Variante del teclado
schema-input-xkb-variant-description = Nombre de la variante XKB. Vacío para usar la predeterminada del sistema.
schema-input-xkb-options-label = Opciones del teclado
schema-input-xkb-options-description = Cadenas de opciones XKB.
schema-input-tap-enabled-label = Tocar para pulsar
schema-input-tap-enabled-description = Trata un toque en el trackpad como una pulsación.
schema-input-tap-drag-enabled-label = Tocar y arrastrar
schema-input-tap-drag-enabled-description = Inicia un arrastre a partir de un toque seguido de un contacto mantenido.
schema-input-tap-drag-lock-enabled-label = Bloqueo de arrastre
schema-input-tap-drag-lock-enabled-description = Mantiene el toque-y-arrastre activo durante un breve levantamiento del dedo.
schema-input-touchpad-click-method-label = Método de clic
schema-input-touchpad-click-method-description = Si una pulsación se determina por el número de dedos o por zonas de botón.
schema-input-touchpad-dwt-enabled-label = Ignorar mientras se escribe
schema-input-touchpad-dwt-enabled-description = Ignora el trackpad mientras se usa el teclado.
schema-input-touchpad-natural-scroll-enabled-label = Desplazamiento natural
schema-input-touchpad-natural-scroll-enabled-description = El contenido sigue a los dedos.
schema-input-touchpad-left-handed-label = Zurdo
schema-input-touchpad-left-handed-description = Intercambia el botón principal con el secundario.
schema-input-touchpad-middle-emulation-enabled-label = Emulación de clic central
schema-input-touchpad-middle-emulation-enabled-description = Pulsar ambos botones a la vez equivale a una pulsación central.
schema-input-scroll-speed-label = Velocidad de desplazamiento
schema-input-scroll-speed-description = Multiplicador por software aplicado a los eventos de desplazamiento.
schema-input-pointer-accel-speed-label = Velocidad del puntero
schema-input-pointer-accel-speed-description = Aceleración del puntero, de -1 (más lenta) a 1 (más rápida).
schema-input-pointer-accel-profile-label = Aceleración del puntero
schema-input-pointer-accel-profile-description = Constante es la velocidad sin ajustes; adaptativa sigue la curva de libinput.

# --- audio ---
schema-audio-sound-enabled-label = Sonidos de la interfaz
schema-audio-sound-enabled-description = Reproduce un sonido de confirmación para los eventos de la interfaz.
schema-audio-sound-theme-label = Tema de sonido
schema-audio-sound-theme-description = Nombre del tema de sonido XDG. Vacío para detección automática.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Gestionar el interruptor de la tapa
schema-power-management-manage-lid-switch-description = Permite que sea Otto quien gestione la tapa en lugar de logind.
schema-power-management-on-lid-close-label = Al cerrar la tapa
schema-power-management-on-lid-close-description = Qué ocurre cuando se cierra la tapa del portátil.
schema-power-management-on-power-button-label = Al pulsar el botón de encendido
schema-power-management-on-power-button-description = Qué ocurre cuando se pulsa el botón físico de encendido.

# --- lock ---
schema-lock-locker-command-label = Comando de bloqueo de pantalla
schema-lock-locker-command-description = El programa de bloqueo que se lanza para bloquear la sesión.
schema-lock-locker-args-label = Argumentos de la pantalla de bloqueo
schema-lock-locker-args-description = Argumentos que se pasan al programa de bloqueo.
schema-lock-auto-lock-timeout-label = Bloquear tras
schema-lock-auto-lock-timeout-description = Segundos de inactividad antes de bloquear. 0 no bloquea nunca.

# --- login ---
schema-login-greeter-command-label = Comando de la pantalla de bienvenida
schema-login-greeter-command-description = El programa de bienvenida que se lanza en modo de inicio de sesión.
schema-login-greeter-args-label = Argumentos de la pantalla de bienvenida
schema-login-greeter-args-description = Argumentos que se pasan a la pantalla de bienvenida.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = El selector sigue al puntero
schema-appswitcher-follow-cursor-description = Muestra el selector de aplicaciones en la pantalla donde está el puntero.
schema-appswitcher-colorize-icons-label = Teñir los iconos del selector
schema-appswitcher-colorize-icons-description = Aplica también al selector de aplicaciones el tinte de iconos del Dock. No hace nada mientras el tinte del Dock está desactivado.

## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automático


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Azul
settings-choice-accent-purple = Morado
settings-choice-accent-pink = Rosa
settings-choice-accent-red = Rojo
settings-choice-accent-orange = Naranja
settings-choice-accent-yellow = Amarillo
settings-choice-accent-green = Verde
settings-choice-accent-mint = Menta
settings-choice-accent-teal = Verde azulado
settings-choice-accent-cyan = Cian
settings-choice-accent-indigo = Índigo
settings-choice-accent-brown = Marrón
settings-choice-accent-graphite = Grafito
# The button under the shortcut list that adds another line.
settings-add-shortcut = Añadir atajo
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Escritorio { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Buscar aplicaciones y ventanas…
launcher-search-apps = Buscar aplicaciones…
launcher-search-windows = Buscar ventanas…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Ventana
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
auth-enter-password = Introducir contraseña

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Iniciar sesión

# strftime pattern for the time in the large clock above the login/lock card,
# not prose: only the %-codes and the separators between them are yours.
# Change it where the local convention differs — %-I:%M %p for a 12-hour
# locale. The digits render at 46pt, so keep the result short.
auth-clock-time-format = %H:%M

# strftime pattern for the date under that clock, again not prose. Reorder the
# parts and change the punctuation to suit the locale (German would be
# "%A, %-d. %B"); the weekday and month names are translated by the system, so
# do not spell them out here. Renders at 15pt in a 360pt box.
auth-clock-date-format = %A %-d de %B

### otto-greeter — the login screen shown before any session exists.
### Everything here is drawn on the login card, centred on the wallpaper.
### The card is narrow: prompts sit above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field while the login screen is asking who is logging
# in. One line, above a text field roughly 20 characters wide — keep it to one
# or two words.
greeter-prompt-username = Nombre de usuario

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Contraseña

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Autenticando…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Servicio de inicio de sesión no disponible: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Servicio de inicio de sesión interrumpido

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } no se ha iniciado

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Autenticado

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Coloque el dedo en el lector

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Deslice el dedo por el lector

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Coloque { $finger } en el lector
greeter-status-swipe-named-finger = Deslice { $finger } por el lector

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Huella no reconocida

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Esperando al lector de huellas…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Iniciando la sesión…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Suspensión no permitida

# As above, for the restart request.
greeter-power-restart-denied = Reinicio no permitido

# As above, for the shut down request.
greeter-power-shutdown-denied = Apagado no permitido

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = No se ha podido suspender

# As above, for the restart request.
greeter-power-restart-failed = No se ha podido reiniciar

# As above, for the shut down request.
greeter-power-shutdown-failed = No se ha podido apagar

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Contraseña

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Autenticado

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Coloque el dedo en el lector

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Deslice el dedo por el lector

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Coloque { $finger } en el lector
lock-status-swipe-named-finger = Deslice { $finger } por el lector

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = el pulgar izquierdo
auth-finger-left-index = el índice izquierdo
auth-finger-left-middle = el dedo medio izquierdo
auth-finger-left-ring = el anular izquierdo
auth-finger-left-little = el meñique izquierdo
auth-finger-right-thumb = el pulgar derecho
auth-finger-right-index = el índice derecho
auth-finger-right-middle = el dedo medio derecho
auth-finger-right-ring = el anular derecho
auth-finger-right-little = el meñique derecho

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Huella no reconocida

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Esperando al lector de huellas…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = No hay ningún usuario que autenticar

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Servicio de autenticación interrumpido

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Nombre de usuario no válido

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Autenticación no disponible

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Error de autenticación ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Suspensión no permitida

# As above, for the restart request.
lock-power-restart-denied = Reinicio no permitido

# As above, for the shut down request.
lock-power-shutdown-denied = Apagado no permitido

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = No se ha podido suspender: { $error }

# As above, for the restart request.
lock-power-restart-failed = No se ha podido reiniciar: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = No se ha podido apagar: { $error }


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
quickview-fact-size = Tamaño
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Dimensiones
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Duración
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Píxeles
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Páginas
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Título
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Artista
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Álbum
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Año

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Archivo vacío
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Demasiado grande para previsualizar
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } megapíxeles
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Instalar uno de estos: { $packages } — para ver las páginas


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Carpeta vacía
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } elemento
        [many] { $count } elementos
       *[other] { $count } elementos
    }
quickview-archive-summary = { $items } — { $size }


## Quick View — sizes
##
## Byte units. Quick View counts in powers of 1024, so the symbols are the
## conventional binary-rounded ones. Translate only the spelled-out "bytes".

quickview-size-bytes =
    { $count ->
        [one] { $count } byte
        [many] { $count } bytes
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
quickview-error-not-previewable = no es un archivo del que se pueda ver una previsualización
# The file's metadata could not be read.
quickview-error-stat-file = no se puede leer la información del archivo: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = no se puede leer el archivo: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = el archivo no permite reposicionarse
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = no se puede aislar el visualizador: { $error }

# Image previewer.
quickview-error-read-image = no se puede leer la imagen: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = no es una imagen que esta versión sepa descodificar
quickview-error-image-no-size = la imagen no indica ningún tamaño
quickview-error-image-decode = la imagen no se ha descodificado: { $error }
quickview-error-image-readback = no se puede releer la imagen descodificada

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = no se puede leer el dibujo: { $error }
quickview-error-drawing-parse = no se ha podido analizar el dibujo
quickview-error-drawing-surface = no hay ninguna superficie donde representarlo
quickview-error-drawing-readback = no se puede releer el dibujo

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = este archivo no es texto en ninguna codificación que se lea

# PDF previewer.
quickview-error-read-document = no se puede leer el documento: { $error }
quickview-error-page-readback = no se puede leer la página generada

# Folder listing.
quickview-error-read-folder = no se puede leer la carpeta

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = no se encuentra el visualizador: { $error }
quickview-error-previewer-start = no se puede iniciar el visualizador: { $error }
quickview-error-previewer-no-output = el visualizador no ha producido nada
quickview-error-previewer-unreadable = el visualizador ha producido algo ilegible
quickview-error-previewer-failed = el visualizador se ha interrumpido: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = este archivo ha tardado demasiado en previsualizarse

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
islands-close = Cerrar

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = ahora
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = hace { $count } min
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = hace { $count } h


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Permitir
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Continuar
# Refuses the request.
islands-dialog-deny = Denegar


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Dock
a11y-app-running = En ejecución
a11y-app-not-running = No está en ejecución
a11y-app-switcher = Selector de aplicaciones
a11y-windows = Ventanas
a11y-workspaces = Escritorios
a11y-untitled-window = Ventana sin título
a11y-menu-bar = Barra de menús
a11y-status = Estado
a11y-tray-item = Elemento { $number }
a11y-notifications = Notificaciones
a11y-categories = Categorías
a11y-results = Resultados
a11y-settings = Ajustes
a11y-preview = Vista previa
a11y-preview-page = Vista previa, página { $page } de { $pages }
a11y-preview-shortened = Vista previa, abreviada
