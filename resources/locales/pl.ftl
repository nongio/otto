# Otto — Polish
#
# Mirrors en-GB.ftl, the source catalogue: every key from en-GB must appear
# here, in the same order.
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

common-open = Otwórz
common-save = Zapisz
common-cancel = Anuluj
common-add = Dodaj
common-remove = Usuń
common-quit = Zakończ
common-cut = Wytnij
common-copy = Kopiuj
common-paste = Wklej
common-rename = Zmień nazwę
common-delete = Usuń
common-move = Przenieś


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Autoukrywanie
dock-auto-hide-on = ✓ Autoukrywanie
dock-magnification = Powiększenie
dock-magnification-on = ✓ Powiększenie
dock-position-bottom = Na dole
dock-position-bottom-on = ✓ Na dole
dock-position-left = Po lewej
dock-position-left-on = ✓ Po lewej
dock-position-right = Po prawej
dock-position-right-on = ✓ Po prawej

# Shown on an app's icon when the app is not running.
dock-open = Otwórz
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Zachowaj w Docku
dock-keep-in-dock-on = ✓ Zachowaj w Docku
dock-quit = Zakończ


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Ogólne
settings-pane-displays = Ekrany
settings-pane-dock = Dock
settings-pane-keyboard = Klawiatura
settings-pane-pointing = Gładzik i mysz
settings-pane-sound = Dźwięk
settings-pane-power = Zasilanie
settings-pane-lock-and-login = Blokada i logowanie


## Settings — General

settings-group-appearance = Wygląd
settings-colour-scheme = Schemat kolorów
settings-accent-colour = Kolor akcentu
settings-rounded-corners = Zaokrąglone rogi
settings-rounded-corners-detail = Zacznie obowiązywać po ponownym uruchomieniu
settings-window-controls = Przyciski okna
settings-maximize-button = Przycisk maksymalizacji
settings-maximize-button-detail = Pokazuje kropkę powiększania; dwukrotne kliknięcie paska tytułu i tak maksymalizuje
settings-font = Czcionka systemowa
settings-gtk-theme = Motyw GTK

settings-group-desktop = Pulpit
settings-background-colour = Kolor tła
settings-background-image = Obraz tła
settings-background-image-detail = Wybierany w oknie wyboru plików portalu pulpitu

settings-group-pointer-and-icons = Wskaźnik i ikony
settings-cursor-theme = Motyw kursora
settings-cursor-size = Rozmiar kursora
settings-icon-theme = Motyw ikon

settings-group-window-switcher = Przełącznik okien
settings-follow-cursor = Pokazuj na ekranie ze wskaźnikiem

settings-group-language = Język
settings-preferred-languages = Preferowane języki

settings-group-configuration = Konfiguracja
settings-configuration-file = Plik konfiguracyjny
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = nieznana — kompozytor nie odpowiada


## Settings — Displays

settings-display-active = Aktywny
settings-display-active-detail = Nieaktywny ekran zachowuje swoje miejsce w układzie
settings-display-primary = Ustaw jako główny
settings-display-primary-detail = Dock i pasek znajdują się na głównym ekranie
settings-display-x-position = Pozycja X
settings-display-y-position = Pozycja Y
settings-display-x-position-detail = Lewy górny róg w układzie współrzędnych pulpitu
settings-display-width = Szerokość
settings-display-width-detail = Piksele. Wyjście bez ekranu może mieć dowolny rozmiar
settings-display-height = Wysokość
settings-display-refresh = Częstotliwość odświeżania
settings-display-refresh-detail = Herce — jak często strumień otrzymuje klatkę
settings-display-resolution = Rozdzielczość
settings-display-scale = Skalowanie ekranu
settings-display-scale-detail = Zacznie obowiązywać po następnym zalogowaniu. Pulpit nie dostosowuje się na bieżąco

# Shown when the compositor reports no outputs at all.
settings-display-none = Brak ekranów
settings-display-none-detail = Kompozytor nie obsługuje żadnego wyjścia

settings-virtual-displays = Ekrany wirtualne
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } wyjście bez ekranu, przesyłane przez PipeWire. Usuń usuwa zaznaczone
        [few] { $count } wyjścia bez ekranu, przesyłane przez PipeWire. Usuń usuwa zaznaczone
        [many] { $count } wyjść bez ekranu, przesyłanych przez PipeWire. Usuń usuwa zaznaczone
       *[other] { $count } wyjścia bez ekranu, przesyłane przez PipeWire. Usuń usuwa zaznaczone
    }


## Settings — Dock

settings-dock-size = Rozmiar
settings-dock-position = Pozycja na ekranie
settings-dock-autohide = Ukrywaj automatycznie
settings-dock-magnification = Powiększenie
settings-group-magnification-and-icons = Powiększenie i ikony
settings-dock-magnification-amount = Stopień powiększenia
settings-dock-tint-icons = Zabarwiaj ikony
settings-switcher-colorize-icons = Zabarwiaj przełącznik
settings-dock-icon-tint = Zabarwienie ikon
settings-dock-icon-tint-strength = Siła zabarwienia ikon


## Settings — Keyboard

settings-key-repeat-delay = Opóźnienie powtarzania klawiszy
settings-key-repeat-rate = Szybkość powtarzania klawiszy
settings-group-input-source = Źródło wprowadzania
settings-xkb-layout = Układ
settings-xkb-variant = Wariant
settings-xkb-options = Opcje
settings-group-shortcuts = Skróty
settings-key-combination = Kombinacja klawiszy
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift lub Logo połączone znakiem +, a potem jeden klawisz: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Gładzik
settings-tap-to-click = Stuknięcie jako kliknięcie
settings-tap-and-drag = Stuknięcie i przeciąganie
settings-drag-lock = Blokada przeciągania
settings-click-method = Metoda klikania
settings-ignore-while-typing = Ignoruj podczas pisania
settings-natural-scrolling = Naturalne przewijanie
settings-left-handed = Dla leworęcznych
settings-middle-click-emulation = Emulacja środkowego przycisku
settings-group-pointer = Wskaźnik
settings-tracking-speed = Szybkość śledzenia
settings-pointer-acceleration = Przyspieszenie
settings-scrolling-speed = Szybkość przewijania


## Settings — Sound

settings-interface-sounds = Dźwięki interfejsu
settings-sound-theme = Motyw dźwiękowy


## Settings — Power

settings-manage-lid-switch = Obsługuj przełącznik pokrywy
settings-manage-lid-switch-detail = Otto usypia po zamknięciu pokrywy zamiast logind
settings-on-lid-close = Po zamknięciu pokrywy
settings-on-power-button = Po naciśnięciu przycisku zasilania


## Settings — Lock & Login

settings-group-lock = Blokada
settings-lock-after = Blokuj po
settings-lock-screen = Ekran blokady
settings-lock-screen-detail = Zacznie obowiązywać przy następnym zablokowaniu ekranu
settings-lock-screen-arguments = Parametry ekranu blokady
settings-group-login = Logowanie
settings-greeter = Ekran logowania
settings-greeter-detail = Zacznie obowiązywać po następnym zalogowaniu
settings-greeter-arguments = Parametry ekranu logowania


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Jasny
settings-choice-dark = Ciemny
settings-choice-controls-left = Po lewej
settings-choice-controls-right = Po prawej
settings-choice-position-bottom = Na dole
settings-choice-position-left = Po lewej
settings-choice-position-right = Po prawej
settings-choice-clickfinger = Klikanie palcami
settings-choice-buttonareas = Klikanie w rogach
settings-choice-accel-flat = Stała prędkość
settings-choice-accel-adaptive = Prędkość zależna od ruchu
settings-choice-lid-auto = Decyduj automatycznie
settings-choice-lid-lock = Zablokuj ekran
settings-choice-lid-disable-internal = Wyłącz wbudowany ekran
settings-choice-power-ignore = Nic nie rób
settings-choice-power-lock = Zablokuj ekran
settings-choice-power-suspend = Uśpij
settings-choice-power-shutdown = Wyłącz komputer
# The automatic option for a theme that follows the system.
settings-choice-auto = Automatycznie


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

files-window-title = Pliki
# The Get Info panel's own window.
files-info-window-title = Informacje


## Files — commands

files-get-info = Zobacz informacje
files-new-folder = Nowy folder
files-move-to-trash = Przenieś do kosza
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Przenieś { $count } element do kosza
        [few] Przenieś { $count } elementy do kosza
        [many] Przenieś { $count } elementów do kosza
       *[other] Przenieś { $count } elementu do kosza
    }


## Files — sidebar and columns

files-places = Miejsca
files-home = Dom
files-desktop = Pulpit
files-documents = Dokumenty
files-downloads = Pobrane
files-music = Muzyka
files-pictures = Obrazy
files-videos = Filmy
files-trash = Kosz

files-column-name = Nazwa
files-column-size = Rozmiar
files-column-kind = Rodzaj
files-column-date-modified = Data modyfikacji


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Folder
files-kind-image = Obraz
files-kind-movie = Film
files-kind-audio = Dźwięk
files-kind-text = Tekst
files-kind-document = Dokument
files-kind-archive = Archiwum
files-kind-application = Aplikacja


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Wczytywanie…
files-empty = Puste
# The idle line: what the folder holds.
files-status-no-items = Brak elementów
files-status-items =
    { $count ->
        [one] { $count } element
        [few] { $count } elementy
        [many] { $count } elementów
       *[other] { $count } elementu
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, ukryte: { $hidden }
files-status-selected = Zaznaczono { $count } z { $total }
files-status-opening-preview = Otwieranie podglądu…
files-nothing-to-undo = Nie ma nic do cofnięcia
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Cofnięto { $label }
files-undo-move = Przeniesienie
files-undo-copy = Kopiowanie
files-undo-delete = Usunięcie
files-undo-rename = Zmianę nazwy
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Zmieniono nazwę na „{ $name }”
files-new-folder-created = Nowy folder „{ $name }”
files-gone = „{ $name }” już tam nie ma
files-rename-failed = Nie można zmienić nazwy: { $error }
files-new-folder-failed = Nie można utworzyć folderu: { $error }
files-open-failed = Nie można otworzyć tego pliku: { $error }
files-new-window-failed = Nie można otworzyć nowego okna: { $error }


## Files — the listing

files-folder-empty = Ten folder jest pusty.
files-folder-denied = Brak uprawnień do wyświetlenia zawartości tego folderu.
files-folder-gone = Ten folder już nie istnieje.
files-folder-open-failed = Nie można otworzyć tego folderu: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Położenie
files-info-kind = Rodzaj
files-info-modified = Zmodyfikowano
files-info-created = Utworzono
files-info-accessed = Ostatni dostęp
files-info-owner = Właściciel
files-info-links-to = Dowiązanie do
files-info-permissions = Uprawnienia
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Odczyt
files-perm-write = Zapis
files-perm-exec = Wykonanie
# Row labels: who each set of permissions applies to.
files-perm-owner = Właściciel
files-perm-group = Grupa
files-perm-everyone = Wszyscy

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Otwórz
files-picker-save-as = Zapisz jako
files-picker-save-files = Zapisz pliki
files-picker-all-files = Wszystkie pliki
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Zapisz jako:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Wpisz nazwę
files-save-name-has-slash = Nazwa nie może zawierać znaku „/”
files-save-name-reserved = Ta nazwa jest zastrzeżona
files-save-nowhere = Brak miejsca do zapisu
files-save-permission-denied = Brak uprawnień do zapisu w tym miejscu

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = „{ $name }” już istnieje. Zastąpić?
files-replace-one-detail = Zastąpienie nadpisuje obecną zawartość.
files-replace-many =
    { $count ->
        [one] { $count } z tych plików już istnieje. Zastąpić?
        [few] { $count } z tych plików już istnieją. Zastąpić?
        [many] { $count } z tych plików już istnieje. Zastąpić?
       *[other] { $count } z tych plików już istnieje. Zastąpić?
    }
files-replace-many-detail = Zastąpienie nadpisuje ich obecną zawartość.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } bajt
        [few] { $count } bajty
        [many] { $count } bajtów
       *[other] { $count } bajta
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

files-date-modified = { $day } { $month } { $year } o { $time }

files-month-jan = sty
files-month-feb = lut
files-month-mar = mar
files-month-apr = kwi
files-month-may = maj
files-month-jun = cze
files-month-jul = lip
files-month-aug = sie
files-month-sep = wrz
files-month-oct = paź
files-month-nov = lis
files-month-dec = gru


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
settings-not-set = Nie ustawiono
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Wybierz…
settings-no-file-chosen = Nie wybrano pliku
settings-choose-background-image = Wybór obrazu tła

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Ustawienia Otto — { $pane }


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
schema-screen-scale-label = Skalowanie ekranu
schema-screen-scale-description = Globalny współczynnik skalowania stosowany do pulpitu.
schema-theme-scheme-label = Schemat kolorów
schema-theme-scheme-description = Jasny lub ciemny schemat kolorów.
schema-accent-color-label = Kolor akcentu
schema-accent-color-description = Nazwa z palety, która podąża za jasnym i ciemnym schematem, albo kolor #RRGGBB.
schema-rounded-corners-label = Zaokrąglone rogi
schema-rounded-corners-description = Dock, górny pasek, dekoracje okien i panele środowiska.
schema-window-controls-side-label = Przyciski okna
schema-window-controls-side-description = Przy którym końcu paska tytułu znajdują się przyciski zamykania, minimalizowania i powiększania.
schema-show-maximize-button-label = Przycisk maksymalizacji
schema-show-maximize-button-description = Pokazuje przycisk powiększania na pasku tytułu okna. Domyślnie wyłączone: dwukrotne kliknięcie paska tytułu i tak maksymalizuje okno.
schema-font-family-label = Czcionka interfejsu
schema-font-family-description = Rodzina czcionek używana przez własny interfejs Otto.
schema-background-color-label = Kolor tła
schema-background-color-description = Kolor tła pulpitu, jako ciąg szesnastkowy.
schema-background-image-label = Obraz tła
schema-background-image-description = Ścieżka do obrazu tła pulpitu. Puste oznacza brak.
schema-cursor-theme-label = Motyw kursora
schema-cursor-theme-description = Nazwa motywu XCursor.
schema-cursor-size-label = Rozmiar kursora
schema-cursor-size-description = Rozmiar kursora w pikselach logicznych.
schema-icon-theme-label = Motyw ikon
schema-icon-theme-description = Nazwa motywu ikon. Puste wykrywa automatycznie.
schema-gtk-theme-label = Motyw GTK
schema-gtk-theme-description = Nazwa motywu GTK przekazywana klientom. Puste wykrywa automatycznie.
schema-locales-label = Ustawienia regionalne
schema-locales-description = Preferowane ustawienia regionalne, w kolejności preferencji.

# --- dock ---
schema-dock-size-label = Rozmiar
schema-dock-size-description = Mnożnik rozmiaru Docka.
schema-dock-position-label = Pozycja na ekranie
schema-dock-position-description = Krawędź ekranu, na której znajduje się Dock.
schema-dock-autohide-label = Ukrywaj automatycznie
schema-dock-autohide-description = Ukrywaj Dock, dopóki wskaźnik nie dotrze do jego krawędzi ekranu.
schema-dock-magnification-label = Powiększenie
schema-dock-magnification-description = Powiększaj ikony pod wskaźnikiem.
schema-dock-genie-scale-label = Stopień powiększenia
schema-dock-genie-scale-description = Jak bardzo powiększają się ikony pod wskaźnikiem.
schema-dock-genie-span-label = Zasięg powiększenia
schema-dock-genie-span-description = Ile sąsiednich ikon obejmuje powiększenie.
schema-dock-colorize-icons-label = Zabarwiaj ikony
schema-dock-colorize-icons-description = Zabarwiaj ikony Docka jednym kolorem.
schema-dock-colorize-color-label = Zabarwienie ikon
schema-dock-colorize-color-description = Kolor używany do zabarwiania ikon Docka, jako ciąg szesnastkowy.
schema-dock-colorize-intensity-label = Siła zabarwienia ikon
schema-dock-colorize-intensity-description = Jak mocno stosowane jest zabarwienie.

# --- general ---
schema-keyboard-repeat-delay-label = Opóźnienie powtarzania klawiszy
schema-keyboard-repeat-delay-description = Milisekundy przytrzymania klawisza przed rozpoczęciem powtarzania.
schema-keyboard-repeat-rate-label = Szybkość powtarzania klawiszy
schema-keyboard-repeat-rate-description = Liczba powtórzeń na sekundę przy przytrzymanym klawiszu.

# --- input ---
schema-input-xkb-layout-label = Układ klawiatury
schema-input-xkb-layout-description = Nazwa układu XKB. Puste używa domyślnego ustawienia systemu.
schema-input-xkb-variant-label = Wariant klawiatury
schema-input-xkb-variant-description = Nazwa wariantu XKB. Puste używa domyślnego ustawienia systemu.
schema-input-xkb-options-label = Opcje klawiatury
schema-input-xkb-options-description = Ciągi opcji XKB.
schema-input-tap-enabled-label = Stuknięcie jako kliknięcie
schema-input-tap-enabled-description = Traktuj stuknięcie w gładzik jako kliknięcie.
schema-input-tap-drag-enabled-label = Stuknięcie i przeciąganie
schema-input-tap-drag-enabled-description = Rozpoczynaj przeciąganie stuknięciem, po którym następuje przytrzymane dotknięcie.
schema-input-tap-drag-lock-enabled-label = Blokada przeciągania
schema-input-tap-drag-lock-enabled-description = Kontynuuj przeciąganie po stuknięciu mimo krótkiego oderwania palca.
schema-input-touchpad-click-method-label = Metoda klikania
schema-input-touchpad-click-method-description = Czy kliknięcie zależy od liczby palców, czy od obszarów przycisków.
schema-input-touchpad-dwt-enabled-label = Ignoruj podczas pisania
schema-input-touchpad-dwt-enabled-description = Ignoruj gładzik podczas korzystania z klawiatury.
schema-input-touchpad-natural-scroll-enabled-label = Naturalne przewijanie
schema-input-touchpad-natural-scroll-enabled-description = Treść podąża za palcami.
schema-input-touchpad-left-handed-label = Dla leworęcznych
schema-input-touchpad-left-handed-description = Zamień przycisk główny i pomocniczy.
schema-input-touchpad-middle-emulation-enabled-label = Emulacja środkowego przycisku
schema-input-touchpad-middle-emulation-enabled-description = Jednoczesne naciśnięcie obu przycisków to kliknięcie środkowe.
schema-input-scroll-speed-label = Szybkość przewijania
schema-input-scroll-speed-description = Programowy mnożnik zastosowany do zdarzeń przewijania.
schema-input-pointer-accel-speed-label = Szybkość śledzenia
schema-input-pointer-accel-speed-description = Przyspieszenie wskaźnika, od -1 (najwolniej) do 1 (najszybciej).
schema-input-pointer-accel-profile-label = Przyspieszenie
schema-input-pointer-accel-profile-description = „Stała prędkość” to surowa szybkość, „prędkość zależna od ruchu” podąża za krzywą libinput.

# --- audio ---
schema-audio-sound-enabled-label = Dźwięki interfejsu
schema-audio-sound-enabled-description = Odtwarzaj dźwiękową informację zwrotną dla zdarzeń interfejsu.
schema-audio-sound-theme-label = Motyw dźwiękowy
schema-audio-sound-theme-description = Nazwa motywu dźwiękowego XDG. Puste wykrywa automatycznie.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Obsługuj przełącznik pokrywy
schema-power-management-manage-lid-switch-description = Otto reaguje na pokrywę zamiast pozostawiać to logind.
schema-power-management-on-lid-close-label = Po zamknięciu pokrywy
schema-power-management-on-lid-close-description = Co się dzieje po zamknięciu pokrywy laptopa.
schema-power-management-on-power-button-label = Po naciśnięciu przycisku zasilania
schema-power-management-on-power-button-description = Co się dzieje po naciśnięciu sprzętowego przycisku zasilania.

# --- lock ---
schema-lock-locker-command-label = Polecenie ekranu blokady
schema-lock-locker-command-description = Program blokujący uruchamiany do zablokowania sesji.
schema-lock-locker-args-label = Parametry ekranu blokady
schema-lock-locker-args-description = Parametry przekazywane programowi blokującemu.
schema-lock-auto-lock-timeout-label = Blokuj po
schema-lock-auto-lock-timeout-description = Sekundy bezczynności przed zablokowaniem. 0 oznacza brak blokady.

# --- login ---
schema-login-greeter-command-label = Polecenie ekranu logowania
schema-login-greeter-command-description = Ekran logowania uruchamiany w trybie logowania.
schema-login-greeter-args-label = Parametry ekranu logowania
schema-login-greeter-args-description = Parametry przekazywane ekranowi logowania.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = Przełącznik podąża za wskaźnikiem
schema-appswitcher-follow-cursor-description = Pokazuj przełącznik aplikacji na ekranie, na którym znajduje się wskaźnik.
schema-appswitcher-colorize-icons-label = Zabarwiaj ikony przełącznika
schema-appswitcher-colorize-icons-description = Zastosuj zabarwienie ikon Docka także w przełączniku aplikacji. Nic nie robi, dopóki zabarwienie Docka jest wyłączone.


## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automatyczny


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Niebieski
settings-choice-accent-purple = Fioletowy
settings-choice-accent-pink = Różowy
settings-choice-accent-red = Czerwony
settings-choice-accent-orange = Pomarańczowy
settings-choice-accent-yellow = Żółty
settings-choice-accent-green = Zielony
settings-choice-accent-mint = Miętowy
settings-choice-accent-teal = Morski
settings-choice-accent-cyan = Błękitny
settings-choice-accent-indigo = Indygo
settings-choice-accent-brown = Brązowy
settings-choice-accent-graphite = Grafitowy
# The button under the shortcut list that adds another line.
settings-add-shortcut = Dodaj skrót
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Biurko { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Szukaj aplikacji i okien…
launcher-search-apps = Szukaj aplikacji…
launcher-search-windows = Szukaj okien…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Okno
launcher-badge-calc = Kalkulator


## Login, lock and authentication
##
## The greeter, the lock screen, and the panel both of them draw. Text that
## arrives from PAM or greetd at runtime is not here: those localise
## themselves, and restating them would be guessing at another program's words.
# Button under the login/lock card, offered only while the fingerprint reader
# is being waited on: it abandons the finger and asks for a password instead.
# The button sizes itself to the text, but it sits on a card 380pt wide — keep
# it to roughly 20 characters so it does not overhang.
auth-enter-password = Wprowadź hasło

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Zaloguj się

# strftime pattern for the time in the large clock above the login/lock card,
# not prose: only the %-codes and the separators between them are yours.
# Change it where the local convention differs — %-I:%M %p for a 12-hour
# locale. The digits render at 46pt, so keep the result short.
auth-clock-time-format = %H:%M

# strftime pattern for the date under that clock, again not prose. Reorder the
# parts and change the punctuation to suit the locale (German would be
# "%A, %-d. %B"); the weekday and month names are translated by the system, so
# do not spell them out here. Renders at 15pt in a 360pt box.
auth-clock-date-format = %A, %-d %B

### otto-greeter — the login screen shown before any session exists.
### Everything here is drawn on the login card, centred on the wallpaper.
### The card is narrow: prompts sit above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field while the login screen is asking who is logging
# in. One line, above a text field roughly 20 characters wide — keep it to one
# or two words.
greeter-prompt-username = Nazwa użytkownika

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Hasło

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Uwierzytelnianie…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Usługa logowania niedostępna: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Usługa logowania przerwała połączenie

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = Sesja { $session } nie uruchomiła się

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Uwierzytelniono

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Połóż palec na czytniku

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Przesuń palec po czytniku

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Połóż { $finger } na czytniku
greeter-status-swipe-named-finger = Przesuń { $finger } po czytniku

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Nie rozpoznano odcisku palca

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Oczekiwanie na czytnik linii papilarnych…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Uruchamianie sesji…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Brak uprawnień do uśpienia komputera

# As above, for the restart request.
greeter-power-restart-denied = Brak uprawnień do ponownego uruchomienia

# As above, for the shut down request.
greeter-power-shutdown-denied = Brak uprawnień do wyłączenia komputera

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Nie można uśpić komputera

# As above, for the restart request.
greeter-power-restart-failed = Nie można uruchomić ponownie

# As above, for the shut down request.
greeter-power-shutdown-failed = Nie można wyłączyć komputera

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Hasło

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Uwierzytelniono

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Połóż palec na czytniku

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Przesuń palec po czytniku

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Połóż { $finger } na czytniku
lock-status-swipe-named-finger = Przesuń { $finger } po czytniku

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = lewy kciuk
auth-finger-left-index = lewy palec wskazujący
auth-finger-left-middle = lewy palec środkowy
auth-finger-left-ring = lewy palec serdeczny
auth-finger-left-little = lewy mały palec
auth-finger-right-thumb = prawy kciuk
auth-finger-right-index = prawy palec wskazujący
auth-finger-right-middle = prawy palec środkowy
auth-finger-right-ring = prawy palec serdeczny
auth-finger-right-little = prawy mały palec

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Nie rozpoznano odcisku palca

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Oczekiwanie na czytnik linii papilarnych…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Brak użytkownika do uwierzytelnienia

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Błąd usługi uwierzytelniania

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Nieprawidłowa nazwa użytkownika

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Uwierzytelnianie jest niedostępne

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Uwierzytelnianie nie powiodło się ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Brak uprawnień do uśpienia komputera

# As above, for the restart request.
lock-power-restart-denied = Brak uprawnień do ponownego uruchomienia

# As above, for the shut down request.
lock-power-shutdown-denied = Brak uprawnień do wyłączenia komputera

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Nie można uśpić komputera: { $error }

# As above, for the restart request.
lock-power-restart-failed = Nie można uruchomić ponownie: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Nie można wyłączyć komputera: { $error }


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
quickview-fact-kind = Rodzaj
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = Rozmiar
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Wymiary
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Czas trwania
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Piksele
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Strony
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Tytuł
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Wykonawca
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Album
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Rok

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Pusty plik
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Zbyt duży do podglądu
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels =
    { $count ->
        [one] { $count } megapiksel
        [few] { $count } megapiksele
        [many] { $count } megapikseli
       *[other] { $count } megapiksela
    }
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Aby zobaczyć strony, zainstaluj jeden z pakietów: { $packages }


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Pusty folder
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } element
        [few] { $count } elementy
        [many] { $count } elementów
       *[other] { $count } elementu
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
        [one] { $count } bajt
        [few] { $count } bajty
        [many] { $count } bajtów
       *[other] { $count } bajta
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
quickview-error-not-previewable = tego pliku nie można wyświetlić w podglądzie
# The file's metadata could not be read.
quickview-error-stat-file = nie można odczytać danych pliku: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = nie można odczytać pliku: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = w pliku nie można się przemieszczać
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = nie można uruchomić podglądu w piaskownicy: { $error }

# Image previewer.
quickview-error-read-image = nie można odczytać obrazu: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = ta wersja nie obsługuje tego formatu obrazu
quickview-error-image-no-size = obraz nie podaje swojego rozmiaru
quickview-error-image-decode = nie udało się zdekodować obrazu: { $error }
quickview-error-image-readback = nie można odczytać zdekodowanego obrazu

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = nie można odczytać rysunku: { $error }
quickview-error-drawing-parse = nie udało się przetworzyć rysunku
quickview-error-drawing-surface = brak powierzchni do jego wyświetlenia
quickview-error-drawing-readback = nie można odczytać narysowanego rysunku

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = ten plik nie jest tekstem w żadnym z odczytywanych kodowań

# PDF previewer.
quickview-error-read-document = nie można odczytać dokumentu: { $error }
quickview-error-page-readback = nie można odczytać wyrenderowanej strony

# Folder listing.
quickview-error-read-folder = nie można odczytać folderu

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = nie można znaleźć programu podglądu: { $error }
quickview-error-previewer-start = nie można uruchomić programu podglądu: { $error }
quickview-error-previewer-no-output = program podglądu nic nie zwrócił
quickview-error-previewer-unreadable = program podglądu zwrócił coś nieczytelnego
quickview-error-previewer-failed = program podglądu zakończył się błędem: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = podgląd tego pliku trwał zbyt długo

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
islands-close = Zamknij

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = przed chwilą
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count } min temu
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count } godz. temu


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Zezwól
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Kontynuuj
# Refuses the request.
islands-dialog-deny = Odmów


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Dok
a11y-app-running = Uruchomiony
a11y-app-not-running = Nieuruchomiony
a11y-app-switcher = Przełącznik programów
a11y-windows = Okna
a11y-workspaces = Biurka
a11y-untitled-window = Okno bez tytułu
a11y-menu-bar = Pasek menu
a11y-status = Stan
a11y-tray-item = Element { $number }
a11y-notifications = Powiadomienia
a11y-categories = Kategorie
a11y-results = Wyniki
a11y-settings = Ustawienia
a11y-preview = Podgląd
a11y-preview-page = Podgląd, strona { $page } z { $pages }
a11y-preview-shortened = Podgląd, skrócony
