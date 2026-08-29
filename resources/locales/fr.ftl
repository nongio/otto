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

common-open = Ouvrir
common-save = Enregistrer
common-cancel = Annuler
common-add = Ajouter
common-remove = Retirer
common-quit = Quitter
common-cut = Couper
common-copy = Copier
common-paste = Coller
common-rename = Renommer
common-delete = Supprimer
common-move = Déplacer


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Masquage automatique
dock-auto-hide-on = ✓ Masquage automatique
dock-magnification = Grossissement
dock-magnification-on = ✓ Grossissement
dock-position-bottom = Bas
dock-position-bottom-on = ✓ Bas
dock-position-left = Gauche
dock-position-left-on = ✓ Gauche
dock-position-right = Droite
dock-position-right-on = ✓ Droite

# Shown on an app's icon when the app is not running.
dock-open = Ouvrir
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Garder dans le Dock
dock-keep-in-dock-on = ✓ Garder dans le Dock
dock-quit = Quitter


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Général
settings-pane-displays = Écrans
settings-pane-dock = Dock
settings-pane-keyboard = Clavier
settings-pane-pointing = Pavé tactile et souris
settings-pane-sound = Son
settings-pane-power = Énergie
settings-pane-lock-and-login = Verrouillage et connexion


## Settings — General

settings-group-appearance = Apparence
settings-colour-scheme = Schéma de couleurs
settings-accent-colour = Couleur d’accentuation
settings-rounded-corners = Coins arrondis
settings-rounded-corners-detail = Applicable après un redémarrage
settings-window-controls = Boutons de fenêtre
settings-font = Police système
settings-gtk-theme = Thème GTK

settings-group-desktop = Bureau
settings-background-colour = Couleur d’arrière-plan
settings-background-image = Image d’arrière-plan
settings-background-image-detail = Choisie via le sélecteur de fichiers du portail de bureau

settings-group-pointer-and-icons = Pointeur et icônes
settings-cursor-theme = Thème du curseur
settings-cursor-size = Taille du curseur
settings-icon-theme = Thème d’icônes

settings-group-window-switcher = Alternateur de fenêtres
settings-follow-cursor = Afficher sur l’écran du pointeur
settings-switcher-colorize-icons = Teinter les icônes comme le Dock

settings-group-language = Langue
settings-preferred-languages = Langues préférées

settings-group-configuration = Configuration
settings-configuration-file = Fichier de configuration
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = inconnu — le compositeur ne répond pas


## Settings — Displays

settings-display-active = Actif
settings-display-active-detail = Un écran inactif garde sa place dans l’agencement
settings-display-primary = Utiliser comme écran principal
settings-display-primary-detail = Le Dock et la barre se trouvent sur l’écran principal
settings-display-x-position = Position X
settings-display-y-position = Position Y
settings-display-x-position-detail = Coin supérieur gauche dans l’espace de coordonnées du bureau
settings-display-width = Largeur
settings-display-width-detail = Pixels. Une sortie headless peut avoir n’importe quelle taille
settings-display-height = Hauteur
settings-display-refresh = Fréquence de rafraîchissement
settings-display-refresh-detail = Hertz — la fréquence à laquelle une image est envoyée au flux
settings-display-resolution = Résolution
settings-display-scale = Échelle d’affichage
settings-display-scale-detail = Applicable à la prochaine connexion. Le bureau ne se réorganise pas en direct

# Shown when the compositor reports no outputs at all.
settings-display-none = Aucun écran
settings-display-none-detail = Le compositeur ne pilote aucune sortie

settings-virtual-displays = Écrans virtuels
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } sortie headless, diffusée via PipeWire. Retirer supprime celle sélectionnée
        [many] { $count } sorties headless, diffusées via PipeWire. Retirer supprime celle sélectionnée
       *[other] { $count } sorties headless, diffusées via PipeWire. Retirer supprime celle sélectionnée
    }


## Settings — Dock

settings-dock-size = Taille
settings-dock-position = Position à l’écran
settings-dock-autohide = Masquer automatiquement
settings-dock-magnification = Grossissement
settings-group-magnification-and-icons = Grossissement et icônes
settings-dock-magnification-amount = Niveau de grossissement
settings-dock-tint-icons = Teinter les icônes
settings-dock-icon-tint = Teinte des icônes
settings-dock-icon-tint-strength = Intensité de la teinte


## Settings — Keyboard

settings-key-repeat-delay = Délai de répétition des touches
settings-key-repeat-rate = Vitesse de répétition des touches
settings-group-input-source = Source de saisie
settings-xkb-layout = Disposition
settings-xkb-variant = Variante
settings-xkb-options = Options
settings-group-shortcuts = Raccourcis
settings-key-combination = Combinaison de touches
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift ou Logo reliés par +, suivis d’une touche : Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Pavé tactile
settings-tap-to-click = Toucher pour cliquer
settings-tap-and-drag = Toucher-glisser
settings-drag-lock = Verrouillage du glissement
settings-click-method = Méthode de clic
settings-ignore-while-typing = Ignorer pendant la frappe
settings-natural-scrolling = Défilement naturel
settings-left-handed = Gaucher
settings-middle-click-emulation = Émulation du clic central
settings-group-pointer = Pointeur
settings-tracking-speed = Vitesse de suivi
settings-pointer-acceleration = Accélération
settings-scrolling-speed = Vitesse de défilement


## Settings — Sound

settings-interface-sounds = Sons de l’interface
settings-sound-theme = Thème sonore


## Settings — Power

settings-manage-lid-switch = Gérer le capot
settings-manage-lid-switch-detail = Otto met en veille à la fermeture du capot au lieu de logind
settings-on-lid-close = À la fermeture du capot
settings-on-power-button = À l’appui sur le bouton d’alimentation


## Settings — Lock & Login

settings-group-lock = Verrouillage
settings-lock-after = Verrouiller après
settings-lock-screen = Écran de verrouillage
settings-lock-screen-detail = S’applique au prochain verrouillage de l’écran
settings-lock-screen-arguments = Arguments de l’écran de verrouillage
settings-group-login = Connexion
settings-greeter = Écran de connexion
settings-greeter-detail = Applicable à la prochaine connexion
settings-greeter-arguments = Arguments de l’écran de connexion


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Clair
settings-choice-dark = Sombre
settings-choice-controls-left = Gauche
settings-choice-controls-right = Droite
settings-choice-position-bottom = Bas
settings-choice-position-left = Gauche
settings-choice-position-right = Droite
settings-choice-clickfinger = Cliquer avec les doigts
settings-choice-buttonareas = Cliquer dans les coins
settings-choice-accel-flat = Vitesse constante
settings-choice-accel-adaptive = Vitesse selon le mouvement
settings-choice-lid-auto = Décider automatiquement
settings-choice-lid-lock = Verrouiller l’écran
settings-choice-lid-disable-internal = Éteindre l’écran intégré
settings-choice-power-ignore = Ne rien faire
settings-choice-power-lock = Verrouiller l’écran
settings-choice-power-suspend = Mettre en veille
settings-choice-power-shutdown = Éteindre
# The automatic option for a theme that follows the system.
settings-choice-auto = Auto


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value } %
settings-readout-pixels = { $value } px
settings-readout-milliseconds = { $value } ms
settings-readout-seconds = { $value } s
# Key repeats per second.
settings-readout-per-second = { $value } / s


## Files — windows

files-window-title = Fichiers
# The Get Info panel's own window.
files-info-window-title = Informations


## Files — commands

files-get-info = Obtenir des informations
files-new-folder = Nouveau dossier
files-move-to-trash = Mettre à la corbeille
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Mettre { $count } élément à la corbeille
        [many] Mettre { $count } éléments à la corbeille
       *[other] Mettre { $count } éléments à la corbeille
    }


## Files — sidebar and columns

files-places = Emplacements
files-home = Dossier personnel
files-desktop = Bureau
files-documents = Documents
files-downloads = Téléchargements
files-music = Musique
files-pictures = Images
files-videos = Vidéos
files-trash = Corbeille

files-column-name = Nom
files-column-size = Taille
files-column-kind = Genre
files-column-date-modified = Date de modification


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Dossier
files-kind-image = Image
files-kind-movie = Film
files-kind-audio = Audio
files-kind-text = Texte
files-kind-document = Document
files-kind-archive = Archive
files-kind-application = Application


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Chargement…
files-empty = Vide
# The idle line: what the folder holds.
files-status-no-items = Aucun élément
files-status-items =
    { $count ->
        [one] { $count } élément
        [many] { $count } éléments
       *[other] { $count } éléments
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, { $hidden } masqués
files-status-selected = { $count } sur { $total } sélectionnés
files-status-opening-preview = Ouverture de l’aperçu…
files-nothing-to-undo = Rien à annuler
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Annulé : { $label }
files-undo-move = Déplacer
files-undo-copy = Copier
files-undo-delete = Supprimer
files-undo-rename = Renommer
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Renommé en « { $name } »
files-new-folder-created = Nouveau dossier « { $name } »
files-gone = « { $name } » n’existe plus
files-rename-failed = Impossible de renommer : { $error }
files-new-folder-failed = Impossible de créer le dossier : { $error }
files-open-failed = Impossible d’ouvrir ce fichier : { $error }
files-new-window-failed = Impossible d’ouvrir une nouvelle fenêtre : { $error }


## Files — the listing

files-folder-empty = Ce dossier est vide.
files-folder-denied = Aucune autorisation pour afficher le contenu de ce dossier.
files-folder-gone = Ce dossier n’existe plus.
files-folder-open-failed = Impossible d’ouvrir ce dossier : { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Emplacement
files-info-kind = Genre
files-info-modified = Modifié
files-info-created = Créé
files-info-accessed = Dernier accès
files-info-owner = Propriétaire
files-info-links-to = Lien vers
files-info-permissions = Autorisations
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Lecture
files-perm-write = Écriture
files-perm-exec = Exécution
# Row labels: who each set of permissions applies to.
files-perm-owner = Propriétaire
files-perm-group = Groupe
files-perm-everyone = Tous

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Ouvrir
files-picker-save-as = Enregistrer sous
files-picker-save-files = Enregistrer les fichiers
files-picker-all-files = Tous les fichiers
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Enregistrer sous :

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Saisir un nom
files-save-name-has-slash = Un nom ne peut pas contenir « / »
files-save-name-reserved = Ce nom est réservé
files-save-nowhere = Aucun emplacement où enregistrer
files-save-permission-denied = Aucune autorisation pour enregistrer ici

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = « { $name } » existe déjà. Remplacer ?
files-replace-one-detail = Le remplacer écrase son contenu actuel.
files-replace-many = { $count } de ces fichiers existent déjà. Les remplacer ?
files-replace-many-detail = Les remplacer écrase leur contenu actuel.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } octet
        [many] { $count } octets
       *[other] { $count } octets
    }
files-size-kb = { $value } Ko
files-size-mb = { $value } Mo
files-size-gb = { $value } Go
files-size-tb = { $value } To


## Files — dates
##
## Assembled from the parts below rather than from a format string, because
## the month names have to be translated too.
##
## $day is the day of the month, $month one of the abbreviations below, $year
## the four-digit year, $time the time as HH:MM. Reorder them freely — en-US
## puts the month first.

files-date-modified = { $day } { $month } { $year } à { $time }

files-month-jan = janv.
files-month-feb = févr.
files-month-mar = mars
files-month-apr = avr.
files-month-may = mai
files-month-jun = juin
files-month-jul = juil.
files-month-aug = août
files-month-sep = sept.
files-month-oct = oct.
files-month-nov = nov.
files-month-dec = déc.


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
settings-not-set = Non défini
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Choisir…
settings-no-file-chosen = Aucun fichier choisi
settings-choose-background-image = Choisir une image d’arrière-plan

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Réglages d’Otto — { $pane }


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
schema-screen-scale-label = Échelle d’affichage
schema-screen-scale-description = Facteur d’échelle global appliqué au bureau.
schema-theme-scheme-label = Schéma de couleurs
schema-theme-scheme-description = Jeu de couleurs clair ou sombre.
schema-accent-color-label = Couleur d’accentuation
schema-accent-color-description = Un nom de la palette, qui suit les jeux de couleurs clair et sombre, ou une couleur #RRGGBB.
schema-rounded-corners-label = Coins arrondis
schema-rounded-corners-description = Le Dock, la barre supérieure, les décorations de fenêtre et les panneaux du bureau.
schema-window-controls-side-label = Boutons de fenêtre
schema-window-controls-side-description = À quelle extrémité de la barre de titre se trouvent les boutons de fermeture, de réduction et d’agrandissement.
schema-font-family-label = Police de l’interface
schema-font-family-description = Famille de police utilisée par l’interface propre d’Otto.
schema-background-color-label = Couleur d’arrière-plan
schema-background-color-description = Couleur d’arrière-plan du bureau, sous forme de chaîne hexadécimale.
schema-background-image-label = Image d’arrière-plan
schema-background-image-description = Chemin de l’image d’arrière-plan du bureau. Vide pour aucune.
schema-cursor-theme-label = Thème du curseur
schema-cursor-theme-description = Nom du thème XCursor.
schema-cursor-size-label = Taille du curseur
schema-cursor-size-description = Taille du curseur en pixels logiques.
schema-icon-theme-label = Thème d’icônes
schema-icon-theme-description = Nom du thème d’icônes. Vide pour une détection automatique.
schema-gtk-theme-label = Thème GTK
schema-gtk-theme-description = Nom du thème GTK transmis aux clients. Vide pour une détection automatique.
schema-locales-label = Langues
schema-locales-description = Langues préférées, la plus préférée en premier.

# --- dock ---
schema-dock-size-label = Taille
schema-dock-size-description = Multiplicateur de taille du Dock.
schema-dock-position-label = Position à l’écran
schema-dock-position-description = Bord de l’écran sur lequel se trouve le Dock.
schema-dock-autohide-label = Masquer automatiquement
schema-dock-autohide-description = Masquer le Dock jusqu’à ce que le pointeur atteigne son bord d’écran.
schema-dock-magnification-label = Grossissement
schema-dock-magnification-description = Agrandir les icônes sous le pointeur.
schema-dock-genie-scale-label = Niveau de grossissement
schema-dock-genie-scale-description = Ampleur du grossissement des icônes sous le pointeur.
schema-dock-genie-span-label = Étendue du grossissement
schema-dock-genie-span-description = Nombre d’icônes voisines atteintes par le grossissement.
schema-dock-colorize-icons-label = Teinter les icônes
schema-dock-colorize-icons-description = Teinter les icônes du Dock d’une seule couleur.
schema-dock-colorize-color-label = Teinte des icônes
schema-dock-colorize-color-description = Couleur utilisée pour teinter les icônes du Dock, sous forme de chaîne hexadécimale.
schema-dock-colorize-intensity-label = Intensité de la teinte
schema-dock-colorize-intensity-description = Intensité avec laquelle la teinte est appliquée.

# --- general ---
schema-keyboard-repeat-delay-label = Délai de répétition des touches
schema-keyboard-repeat-delay-description = Millisecondes pendant lesquelles une touche est maintenue avant qu’elle ne commence à se répéter.
schema-keyboard-repeat-rate-label = Vitesse de répétition des touches
schema-keyboard-repeat-rate-description = Répétitions par seconde pendant qu’une touche est maintenue.

# --- input ---
schema-input-xkb-layout-label = Disposition du clavier
schema-input-xkb-layout-description = Nom de la disposition XKB. Vide utilise la valeur par défaut du système.
schema-input-xkb-variant-label = Variante du clavier
schema-input-xkb-variant-description = Nom de la variante XKB. Vide utilise la valeur par défaut du système.
schema-input-xkb-options-label = Options du clavier
schema-input-xkb-options-description = Chaînes d’options XKB.
schema-input-tap-enabled-label = Toucher pour cliquer
schema-input-tap-enabled-description = Traiter un toucher sur le pavé tactile comme un clic.
schema-input-tap-drag-enabled-label = Toucher-glisser
schema-input-tap-drag-enabled-description = Démarrer un glissement à partir d’un toucher suivi d’un contact maintenu.
schema-input-tap-drag-lock-enabled-label = Verrouillage du glissement
schema-input-tap-drag-lock-enabled-description = Maintenir un toucher-glisser malgré une brève levée du doigt.
schema-input-touchpad-click-method-label = Méthode de clic
schema-input-touchpad-click-method-description = Détermine si un clic se base sur le nombre de doigts ou sur des zones de bouton.
schema-input-touchpad-dwt-enabled-label = Désactiver pendant la frappe
schema-input-touchpad-dwt-enabled-description = Ignorer le pavé tactile pendant l’utilisation du clavier.
schema-input-touchpad-natural-scroll-enabled-label = Défilement naturel
schema-input-touchpad-natural-scroll-enabled-description = Le contenu suit les doigts.
schema-input-touchpad-left-handed-label = Gaucher
schema-input-touchpad-left-handed-description = Inverser les boutons principal et secondaire.
schema-input-touchpad-middle-emulation-enabled-label = Émulation du clic central
schema-input-touchpad-middle-emulation-enabled-description = Appuyer sur les deux boutons ensemble équivaut à un clic central.
schema-input-scroll-speed-label = Vitesse de défilement
schema-input-scroll-speed-description = Multiplicateur logiciel appliqué aux événements de défilement.
schema-input-pointer-accel-speed-label = Vitesse du pointeur
schema-input-pointer-accel-speed-description = Accélération du pointeur, de -1 (le plus lent) à 1 (le plus rapide).
schema-input-pointer-accel-profile-label = Accélération du pointeur
schema-input-pointer-accel-profile-description = Constante correspond à la vitesse brute ; adaptative suit la courbe de libinput.

# --- audio ---
schema-audio-sound-enabled-label = Sons de l’interface
schema-audio-sound-enabled-description = Jouer un retour sonore pour les événements de l’interface.
schema-audio-sound-theme-label = Thème sonore
schema-audio-sound-theme-description = Nom du thème sonore XDG. Vide pour une détection automatique.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Gérer le capot
schema-power-management-manage-lid-switch-description = Laisser Otto agir sur le capot plutôt que de laisser logind s’en charger.
schema-power-management-on-lid-close-label = À la fermeture du capot
schema-power-management-on-lid-close-description = Ce qui se passe à la fermeture du capot de l’ordinateur portable.
schema-power-management-on-power-button-label = À l’appui sur le bouton d’alimentation
schema-power-management-on-power-button-description = Ce qui se passe à l’appui sur le bouton d’alimentation physique.

# --- lock ---
schema-lock-locker-command-label = Commande de l’écran de verrouillage
schema-lock-locker-command-description = Le verrouilleur lancé pour verrouiller la session.
schema-lock-locker-args-label = Arguments de l’écran de verrouillage
schema-lock-locker-args-description = Arguments transmis au verrouilleur.
schema-lock-auto-lock-timeout-label = Verrouiller après
schema-lock-auto-lock-timeout-description = Secondes d’inactivité avant le verrouillage. 0 ne verrouille jamais.

# --- login ---
schema-login-greeter-command-label = Commande de l’écran de connexion
schema-login-greeter-command-description = Le programme d’écran de connexion lancé en mode connexion.
schema-login-greeter-args-label = Arguments de l’écran de connexion
schema-login-greeter-args-description = Arguments transmis à l’écran de connexion.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = L’alternateur suit le pointeur
schema-appswitcher-follow-cursor-description = Afficher l’alternateur d’applications sur la sortie où se trouve le pointeur.
schema-appswitcher-colorize-icons-label = Teinter les icônes de l’alternateur
schema-appswitcher-colorize-icons-description = Appliquer la teinte des icônes du Dock à l’alternateur d’applications. Sans effet tant que la teinte du Dock est désactivée.

## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automatique


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Bleu
settings-choice-accent-purple = Violet
settings-choice-accent-pink = Rose
settings-choice-accent-red = Rouge
settings-choice-accent-orange = Orange
settings-choice-accent-yellow = Jaune
settings-choice-accent-green = Vert
settings-choice-accent-mint = Menthe
settings-choice-accent-teal = Sarcelle
settings-choice-accent-cyan = Cyan
settings-choice-accent-indigo = Indigo
settings-choice-accent-brown = Marron
settings-choice-accent-graphite = Graphite
# The button under the shortcut list that adds another line.
settings-add-shortcut = Ajouter un raccourci
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Bureau { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Rechercher des applications et des fenêtres…
launcher-search-apps = Rechercher des applications…
launcher-search-windows = Rechercher des fenêtres…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Fenêtre
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
auth-enter-password = Saisir le mot de passe

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Connexion

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
greeter-prompt-username = Nom d’utilisateur

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Mot de passe

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Authentification…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Service de connexion indisponible : { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Service de connexion interrompu

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } n’a pas démarré

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Authentifié

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Poser le doigt sur le lecteur

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Faire glisser le doigt sur le lecteur

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Poser { $finger } sur le lecteur
greeter-status-swipe-named-finger = Faire glisser { $finger } sur le lecteur

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Empreinte non reconnue

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = En attente du lecteur d’empreintes…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Démarrage de la session…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Mise en veille non autorisée

# As above, for the restart request.
greeter-power-restart-denied = Redémarrage non autorisé

# As above, for the shut down request.
greeter-power-shutdown-denied = Arrêt non autorisé

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Mise en veille impossible

# As above, for the restart request.
greeter-power-restart-failed = Redémarrage impossible

# As above, for the shut down request.
greeter-power-shutdown-failed = Arrêt impossible

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Mot de passe

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Authentifié

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Poser le doigt sur le lecteur

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Faire glisser le doigt sur le lecteur

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Poser { $finger } sur le lecteur
lock-status-swipe-named-finger = Faire glisser { $finger } sur le lecteur

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = le pouce gauche
auth-finger-left-index = l’index gauche
auth-finger-left-middle = le majeur gauche
auth-finger-left-ring = l’annulaire gauche
auth-finger-left-little = l’auriculaire gauche
auth-finger-right-thumb = le pouce droit
auth-finger-right-index = l’index droit
auth-finger-right-middle = le majeur droit
auth-finger-right-ring = l’annulaire droit
auth-finger-right-little = l’auriculaire droit

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Empreinte non reconnue

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = En attente du lecteur d’empreintes…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Aucun utilisateur à authentifier

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Service d’authentification interrompu

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Nom d’utilisateur non valide

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Authentification indisponible

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Échec de l’authentification ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Mise en veille non autorisée

# As above, for the restart request.
lock-power-restart-denied = Redémarrage non autorisé

# As above, for the shut down request.
lock-power-shutdown-denied = Arrêt non autorisé

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Mise en veille impossible : { $error }

# As above, for the restart request.
lock-power-restart-failed = Redémarrage impossible : { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Arrêt impossible : { $error }


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
quickview-fact-kind = Genre
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = Taille
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Dimensions
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Durée
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Pixels
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Pages
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Titre
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Artiste
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Album
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Année

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Fichier vide
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Trop grande pour un aperçu
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } mégapixels
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Installer l’un de ces paquets : { $packages } — pour voir les pages


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Dossier vide
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } élément
        [many] { $count } éléments
       *[other] { $count } éléments
    }
quickview-archive-summary = { $items } — { $size }


## Quick View — sizes
##
## Byte units. Quick View counts in powers of 1024, so the symbols are the
## conventional binary-rounded ones. Translate only the spelled-out "bytes".

quickview-size-bytes =
    { $count ->
        [one] { $count } octet
        [many] { $count } octets
       *[other] { $count } octets
    }
quickview-size-kb = { $value } Ko
quickview-size-mb = { $value } Mo
quickview-size-gb = { $value } Go
quickview-size-tb = { $value } To


## Quick View — nothing to show
##
## Each of these fills the card in place of a preview, so a person reads it
## instead of seeing the file. They state what happened and stop. Lower case,
## no full stop: they are shown as a sentence fragment.
##
## $error is an operating-system message, which arrives in whatever language
## the system libraries produce and is usually English. Keep it at the end.

# The file is a pipe, socket or device — opening it could block forever.
quickview-error-not-previewable = ce n’est pas un fichier dont on puisse avoir un aperçu
# The file's metadata could not be read.
quickview-error-stat-file = impossible de lire les informations du fichier : { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = impossible de lire le fichier : { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = le fichier ne permet pas le repositionnement
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = impossible d’isoler le visualiseur : { $error }

# Image previewer.
quickview-error-read-image = impossible de lire l’image : { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = pas une image que cette version sait décoder
quickview-error-image-no-size = l’image n’indique aucune taille
quickview-error-image-decode = l’image ne s’est pas décodée : { $error }
quickview-error-image-readback = impossible de relire l’image décodée

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = impossible de lire le dessin : { $error }
quickview-error-drawing-parse = le dessin n’a pas pu être analysé
quickview-error-drawing-surface = aucune surface pour l’afficher
quickview-error-drawing-readback = impossible de relire le dessin

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = ce fichier n’est du texte dans aucun encodage que l’on lise

# PDF previewer.
quickview-error-read-document = impossible de lire le document : { $error }
quickview-error-page-readback = impossible de lire la page rendue

# Folder listing.
quickview-error-read-folder = impossible de lire le dossier

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = visualiseur introuvable : { $error }
quickview-error-previewer-start = impossible de démarrer le visualiseur : { $error }
quickview-error-previewer-no-output = le visualiseur n’a rien produit
quickview-error-previewer-unreadable = le visualiseur a produit quelque chose d’illisible
quickview-error-previewer-failed = le visualiseur s’est interrompu : { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = l’aperçu de ce fichier a pris trop de temps

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
islands-close = Fermer

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = à l’instant
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = il y a { $count } min
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = il y a { $count } h


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Autoriser
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Continuer
# Refuses the request.
islands-dialog-deny = Refuser


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Dock
a11y-app-running = En cours d’exécution
a11y-app-not-running = Pas en cours d’exécution
a11y-app-switcher = Sélecteur d’applications
a11y-windows = Fenêtres
a11y-workspaces = Bureaux
a11y-untitled-window = Fenêtre sans titre
a11y-menu-bar = Barre des menus
a11y-status = État
a11y-tray-item = Élément { $number }
a11y-notifications = Notifications
a11y-categories = Catégories
a11y-results = Résultats
a11y-settings = Réglages
a11y-preview = Aperçu
a11y-preview-page = Aperçu, page { $page } sur { $pages }
a11y-preview-shortened = Aperçu, abrégé
