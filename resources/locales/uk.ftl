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

common-open = Відкрити
common-save = Зберегти
common-cancel = Скасувати
common-add = Додати
common-remove = Вилучити
common-quit = Вийти
common-cut = Вирізати
common-copy = Копіювати
common-paste = Вставити
common-rename = Перейменувати
common-delete = Видалити
common-move = Перемістити


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Автоприховування
dock-auto-hide-on = ✓ Автоприховування
dock-magnification = Збільшення
dock-magnification-on = ✓ Збільшення
dock-position-bottom = Знизу
dock-position-bottom-on = ✓ Знизу
dock-position-left = Ліворуч
dock-position-left-on = ✓ Ліворуч
dock-position-right = Праворуч
dock-position-right-on = ✓ Праворуч

# Shown on an app's icon when the app is not running.
dock-open = Відкрити
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Залишити в Dock
dock-keep-in-dock-on = ✓ Залишити в Dock
dock-quit = Завершити


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Загальні
settings-pane-displays = Дисплеї
settings-pane-dock = Dock
settings-pane-keyboard = Клавіатура
settings-pane-pointing = Трекпад і миша
settings-pane-sound = Звук
settings-pane-power = Живлення
settings-pane-lock-and-login = Блокування і вхід


## Settings — General

settings-group-appearance = Вигляд
settings-colour-scheme = Схема кольорів
settings-accent-colour = Колір акценту
settings-rounded-corners = Заокруглені кути
settings-rounded-corners-detail = Застосовується після перезапуску
settings-window-controls = Кнопки вікна
settings-maximize-button = Кнопка розгортання
settings-maximize-button-detail = Показує кружечок масштабування; подвійне клацання на заголовку однаково розгортає вікно
settings-font = Системний шрифт
settings-gtk-theme = Тема GTK

settings-group-desktop = Робочий стіл
settings-background-colour = Колір тла
settings-background-image = Зображення тла
settings-background-image-detail = Обирається через засіб вибору файлів робочого стола

settings-group-pointer-and-icons = Вказівник і піктограми
settings-cursor-theme = Тема курсора
settings-cursor-size = Розмір курсора
settings-icon-theme = Тема піктограм

settings-group-window-switcher = Перемикач вікон
settings-follow-cursor = Показувати на дисплеї з вказівником

settings-group-language = Мова
settings-display-language = Мова інтерфейсу

settings-group-configuration = Налаштування
settings-configuration-file = Файл налаштувань
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = невідомо — композитор не відповідає


## Settings — Displays

settings-display-active = Активний
settings-display-active-detail = Неактивний дисплей зберігає своє місце в розташуванні
settings-display-primary = Використовувати як основний
settings-display-primary-detail = Dock і панель розташовані на основному дисплеї
settings-display-x-position = Позиція X
settings-display-y-position = Позиція Y
settings-display-x-position-detail = Верхній лівий кут у системі координат стільниці
settings-display-width = Ширина
settings-display-width-detail = Пікселі. Автономний вихід може мати будь-який розмір
settings-display-height = Висота
settings-display-refresh = Частота оновлення
settings-display-refresh-detail = Герц — як часто потік отримує кадр
settings-display-resolution = Роздільна здатність
settings-display-scale = Масштаб дисплея
settings-display-scale-detail = Застосовується під час наступного входу. Стільниця не перебудовується одразу

# Shown when the compositor reports no outputs at all.
settings-display-none = Немає дисплеїв
settings-display-none-detail = Композитор не керує жодним виходом

settings-virtual-displays = Віртуальні дисплеї
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } автономний вихід, транслюється через PipeWire. «Вилучити» вилучає вибраний
        [few] { $count } автономні виходи, транслюються через PipeWire. «Вилучити» вилучає вибраний
        [many] { $count } автономних виходів, транслюються через PipeWire. «Вилучити» вилучає вибраний
       *[other] { $count } автономного виходу, транслюється через PipeWire. «Вилучити» вилучає вибраний
    }


## Settings — Dock

settings-dock-size = Розмір
settings-dock-position = Розташування на екрані
settings-dock-autohide = Автоматично приховувати
settings-dock-magnification = Збільшення
settings-group-magnification-and-icons = Збільшення і піктограми
settings-dock-magnification-amount = Ступінь збільшення
settings-dock-tint-icons = Тонувати піктограми
settings-switcher-colorize-icons = Тонувати перемикач
settings-dock-icon-tint = Колір тонування піктограм
settings-dock-icon-tint-strength = Сила тонування піктограм


## Settings — Keyboard

settings-key-repeat-delay = Затримка повтору клавіш
settings-key-repeat-rate = Швидкість повтору клавіш
settings-group-input-source = Джерело введення
settings-xkb-layout = Розкладка
settings-xkb-variant = Варіант
settings-xkb-options = Параметри
settings-group-shortcuts = Комбінації клавіш
settings-key-combination = Комбінація клавіш
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift або Logo, поєднані знаком +, а потім одна клавіша: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Трекпад
settings-tap-to-click = Дотик для натискання
settings-tap-and-drag = Дотик і перетягування
settings-drag-lock = Фіксація перетягування
settings-click-method = Спосіб натискання
settings-ignore-while-typing = Ігнорувати під час набору тексту
settings-natural-scrolling = Природне прокручування
settings-left-handed = Для лівої руки
settings-middle-click-emulation = Емуляція середнього натискання
settings-group-pointer = Вказівник
settings-tracking-speed = Швидкість стеження
settings-pointer-acceleration = Прискорення
settings-scrolling-speed = Швидкість прокручування


## Settings — Sound

settings-interface-sounds = Звуки інтерфейсу
settings-sound-theme = Тема звуків


## Settings — Power

settings-manage-lid-switch = Керувати перемикачем кришки
settings-manage-lid-switch-detail = Otto присипляє систему при закритті кришки замість logind
settings-on-lid-close = Коли кришку закрито
settings-on-power-button = Коли натиснуто кнопку живлення


## Settings — Lock & Login

settings-group-lock = Блокування
settings-lock-after = Блокувати після
settings-lock-screen = Екран блокування
settings-lock-screen-detail = Застосовується під час наступного блокування екрана
settings-lock-screen-arguments = Аргументи екрана блокування
settings-group-login = Вхід
settings-greeter = Вітальний екран
settings-greeter-detail = Застосовується під час наступного входу
settings-greeter-arguments = Аргументи вітального екрана


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Світла
settings-choice-dark = Темна
settings-choice-controls-left = Ліворуч
settings-choice-controls-right = Праворуч
settings-choice-position-bottom = Знизу
settings-choice-position-left = Ліворуч
settings-choice-position-right = Праворуч
settings-choice-clickfinger = Натискання пальцями
settings-choice-buttonareas = Натискання в кутах
settings-choice-accel-flat = Стала швидкість
settings-choice-accel-adaptive = Швидкість залежить від руху
settings-choice-lid-auto = Визначати автоматично
settings-choice-lid-lock = Блокувати екран
settings-choice-lid-disable-internal = Вимикати вбудований дисплей
settings-choice-power-ignore = Нічого не робити
settings-choice-power-lock = Блокувати екран
settings-choice-power-suspend = Присипляти
settings-choice-power-shutdown = Вимикати комп'ютер
# The automatic option for a theme that follows the system.
settings-choice-auto = Авто


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value }%
settings-readout-pixels = { $value } px
settings-readout-milliseconds = { $value } мс
settings-readout-seconds = { $value } с
# Key repeats per second.
settings-readout-per-second = { $value } / с


## Files — windows

files-window-title = Файли
# The Get Info panel's own window.
files-info-window-title = Інформація


## Files — commands

files-get-info = Інформація
files-new-folder = Нова папка
files-move-to-trash = Перемістити в кошик
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Перемістити { $count } елемент до кошика
        [few] Перемістити { $count } елементи до кошика
        [many] Перемістити { $count } елементів до кошика
       *[other] Перемістити { $count } елемента до кошика
    }


## Files — sidebar and columns

files-places = Місця
files-home = Домівка
files-desktop = Стільниця
files-documents = Документи
files-downloads = Завантаження
files-music = Музика
files-pictures = Зображення
files-videos = Відео
files-trash = Кошик

files-column-name = Назва
files-column-size = Розмір
files-column-kind = Тип
files-column-date-modified = Дата зміни


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Папка
files-kind-image = Зображення
files-kind-movie = Відео
files-kind-audio = Аудіо
files-kind-text = Текст
files-kind-document = Документ
files-kind-archive = Архів
files-kind-application = Застосунок


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Завантаження…
files-empty = Порожньо
# The idle line: what the folder holds.
files-status-no-items = Немає елементів
files-status-items =
    { $count ->
        [one] { $count } елемент
        [few] { $count } елементи
        [many] { $count } елементів
       *[other] { $count } елемента
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, прихованих: { $hidden }
files-status-selected = Вибрано { $count } з { $total }
files-status-opening-preview = Відкриття перегляду…
files-nothing-to-undo = Немає що скасовувати
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Скасовано: { $label }
files-undo-move = переміщення
files-undo-copy = копіювання
files-undo-delete = видалення
files-undo-rename = перейменування
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Перейменовано на «{ $name }»
files-new-folder-created = Нова папка «{ $name }»
files-gone = «{ $name }» більше немає
files-no-such-folder = «{ $path }» не існує
files-rename-failed = Не вдалося перейменувати: { $error }
files-new-folder-failed = Не вдалося створити папку: { $error }
files-open-failed = Не вдалося відкрити файл: { $error }
files-new-window-failed = Не вдалося відкрити нове вікно: { $error }


## Files — the listing

files-folder-empty = Ця папка порожня.
files-folder-denied = Немає прав на перегляд вмісту цієї папки.
files-folder-gone = Цієї папки більше не існує.
files-folder-open-failed = Не вдалося відкрити цю папку: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Розташування
files-info-kind = Тип
files-info-modified = Змінено
files-info-created = Створено
files-info-accessed = Відкрито
files-info-owner = Власник
files-info-links-to = Посилається на
files-info-permissions = Права доступу
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Читання
files-perm-write = Запис
files-perm-exec = Виконання
# Row labels: who each set of permissions applies to.
files-perm-owner = Власник
files-perm-group = Група
files-perm-everyone = Усі

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Відкрити
files-picker-save-as = Зберегти як
files-picker-save-files = Зберегти файли
files-picker-all-files = Усі файли
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Зберегти як:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Назву не вказано
files-save-name-has-slash = Назва не може містити «/»
files-save-name-reserved = Ця назва зарезервована
files-save-nowhere = Немає куди зберігати
files-save-permission-denied = Немає прав на збереження тут

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = «{ $name }» вже існує. Замінити?
files-replace-one-detail = Заміна перезапише поточний вміст файла.
files-replace-many = { $count } з цих файлів уже існують. Замінити?
files-replace-many-detail = Заміна перезапише поточний вміст файлів.


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } байт
        [few] { $count } байти
        [many] { $count } байтів
       *[other] { $count } байта
    }
files-size-kb = { $value } КБ
files-size-mb = { $value } МБ
files-size-gb = { $value } ГБ
files-size-tb = { $value } ТБ


## Files — dates
##
## Assembled from the parts below rather than from a format string, because
## the month names have to be translated too.
##
## $day is the day of the month, $month one of the abbreviations below, $year
## the four-digit year, $time the time as HH:MM. Reorder them freely — en-US
## puts the month first.

files-date-modified = { $day } { $month } { $year } о { $time }

files-month-jan = січ.
files-month-feb = лют.
files-month-mar = бер.
files-month-apr = квіт.
files-month-may = трав.
files-month-jun = черв.
files-month-jul = лип.
files-month-aug = серп.
files-month-sep = вер.
files-month-oct = жовт.
files-month-nov = лист.
files-month-dec = груд.


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %A %-d %B %H:%M


## Settings — widgets
##
## The controls themselves, rather than the settings they edit.

# Shown in a text field that has no value yet.
settings-not-set = Не задано
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Обрати…
settings-no-file-chosen = Файл не обрано
settings-choose-background-image = Вибір зображення тла

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Налаштування Otto — { $pane }

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
schema-screen-scale-label = Масштаб дисплея
schema-screen-scale-description = Загальний коефіцієнт масштабування стільниці.
schema-theme-scheme-label = Схема кольорів
schema-theme-scheme-description = Світла або темна кольорова схема.
schema-accent-color-label = Колір акценту
schema-accent-color-description = Назва з палітри, яка відповідає світлій і темній схемам, або колір #RRGGBB.
schema-rounded-corners-label = Заокруглені кути
schema-rounded-corners-description = Dock, верхня панель, оформлення вікон і панелі стільниці.
schema-window-controls-side-label = Кнопки вікна
schema-window-controls-side-description = Біля якого краю смуги заголовка розташовані кнопки закриття, згортання та масштабування.
schema-show-maximize-button-label = Кнопка розгортання
schema-show-maximize-button-description = Показувати кнопку масштабування в заголовку вікна. Типово вимкнено: подвійне клацання на заголовку однаково розгортає вікно.
schema-font-family-label = Шрифт інтерфейсу
schema-font-family-description = Гарнітура шрифту, яку використовує власний інтерфейс Otto.
schema-background-color-label = Колір тла
schema-background-color-description = Колір тла стільниці у вигляді шістнадцяткового рядка.
schema-background-image-label = Зображення тла
schema-background-image-description = Шлях до зображення тла стільниці. Порожньо, якщо тло не задано.
schema-cursor-theme-label = Тема курсора
schema-cursor-theme-description = Назва теми XCursor.
schema-cursor-size-label = Розмір курсора
schema-cursor-size-description = Розмір курсора в логічних пікселях.
schema-icon-theme-label = Тема піктограм
schema-icon-theme-description = Назва теми піктограм. Порожньо — визначається автоматично.
schema-gtk-theme-label = Тема GTK
schema-gtk-theme-description = Назва теми GTK, яку передають клієнтам. Порожньо — визначається автоматично.
schema-locales-label = Локалі
schema-locales-description = Бажані локалі, у порядку спадання пріоритету.

# --- dock ---
schema-dock-size-label = Розмір
schema-dock-size-description = Множник розміру Dock.
schema-dock-position-label = Розташування на екрані
schema-dock-position-description = Край екрана, біля якого розташовано Dock.
schema-dock-autohide-label = Автоматично приховувати
schema-dock-autohide-description = Приховувати Dock, доки вказівник не досягне краю екрана, де він розташований.
schema-dock-magnification-label = Збільшення
schema-dock-magnification-description = Збільшувати піктограми під вказівником.
schema-dock-genie-scale-label = Ступінь збільшення
schema-dock-genie-scale-description = Наскільки збільшуються піктограми під вказівником.
schema-dock-genie-span-label = Радіус збільшення
schema-dock-genie-span-description = Скільки сусідніх піктограм охоплює збільшення.
schema-dock-colorize-icons-label = Тонувати піктограми
schema-dock-colorize-icons-description = Тонувати піктограми Dock одним кольором.
schema-dock-colorize-color-label = Колір тонування піктограм
schema-dock-colorize-color-description = Колір, яким тонують піктограми Dock, у вигляді шістнадцяткового рядка.
schema-dock-colorize-intensity-label = Сила тонування піктограм
schema-dock-colorize-intensity-description = Наскільки сильно застосовується тонування.

# --- general ---
schema-keyboard-repeat-delay-label = Затримка повтору
schema-keyboard-repeat-delay-description = Скільки мілісекунд клавішу потрібно тримати, перш ніж почнеться повтор.
schema-keyboard-repeat-rate-label = Швидкість повтору
schema-keyboard-repeat-rate-description = Кількість повторів за секунду під час утримання клавіші.

# --- input ---
schema-input-xkb-layout-label = Розкладка клавіатури
schema-input-xkb-layout-description = Назва розкладки XKB. Порожньо — використовується системна розкладка за умовчанням.
schema-input-xkb-variant-label = Варіант клавіатури
schema-input-xkb-variant-description = Назва варіанту XKB. Порожньо — використовується системний варіант за умовчанням.
schema-input-xkb-options-label = Параметри клавіатури
schema-input-xkb-options-description = Рядки параметрів XKB.
schema-input-tap-enabled-label = Дотик для натискання
schema-input-tap-enabled-description = Вважати дотик до тачпада натисканням.
schema-input-tap-drag-enabled-label = Дотик і перетягування
schema-input-tap-drag-enabled-description = Починати перетягування з дотику, за яким слідує утримання пальця.
schema-input-tap-drag-lock-enabled-label = Фіксація перетягування
schema-input-tap-drag-lock-enabled-description = Продовжувати перетягування дотиком під час короткого відриву пальця.
schema-input-touchpad-click-method-label = Спосіб натискання
schema-input-touchpad-click-method-description = Чи визначає натискання кількість пальців, чи зону кнопки.
schema-input-touchpad-dwt-enabled-label = Вимикати під час набору тексту
schema-input-touchpad-dwt-enabled-description = Ігнорувати тачпад, поки використовується клавіатура.
schema-input-touchpad-natural-scroll-enabled-label = Природне прокручування
schema-input-touchpad-natural-scroll-enabled-description = Вміст рухається за пальцями.
schema-input-touchpad-left-handed-label = Для лівої руки
schema-input-touchpad-left-handed-description = Поміняти місцями основну і додаткову кнопки.
schema-input-touchpad-middle-emulation-enabled-label = Емуляція середнього натискання
schema-input-touchpad-middle-emulation-enabled-description = Одночасне натискання обох кнопок вважається натисканням середньої кнопки.
schema-input-scroll-speed-label = Швидкість прокручування
schema-input-scroll-speed-description = Програмний множник, який застосовується до подій прокручування.
schema-input-pointer-accel-speed-label = Швидкість вказівника
schema-input-pointer-accel-speed-description = Прискорення вказівника — від -1 (найповільніше) до 1 (найшвидше).
schema-input-pointer-accel-profile-label = Прискорення вказівника
schema-input-pointer-accel-profile-description = Стале — вихідна швидкість без змін; адаптивне слідує кривій libinput.

# --- audio ---
schema-audio-sound-enabled-label = Звуки інтерфейсу
schema-audio-sound-enabled-description = Відтворювати звуковий відгук для подій інтерфейсу.
schema-audio-sound-theme-label = Тема звуків
schema-audio-sound-theme-description = Назва звукової теми XDG. Порожньо — визначається автоматично.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Керувати перемикачем кришки
schema-power-management-manage-lid-switch-description = Дозволити Otto реагувати на кришку, а не залишати це на logind.
schema-power-management-on-lid-close-label = Коли кришку закрито
schema-power-management-on-lid-close-description = Що відбувається, коли закривають кришку ноутбука.
schema-power-management-on-power-button-label = Коли натиснуто кнопку живлення
schema-power-management-on-power-button-description = Що відбувається при натисканні апаратної кнопки живлення.

# --- lock ---
schema-lock-locker-command-label = Команда екрана блокування
schema-lock-locker-command-description = Програма блокування, яку запускають для блокування сеансу.
schema-lock-locker-args-label = Аргументи екрана блокування
schema-lock-locker-args-description = Аргументи, які передають програмі блокування.
schema-lock-auto-lock-timeout-label = Блокувати після
schema-lock-auto-lock-timeout-description = Секунди бездіяльності до блокування. 0 вимикає блокування.

# --- login ---
schema-login-greeter-command-label = Команда вітального екрана
schema-login-greeter-command-description = Вітальний екран, який запускають у режимі входу.
schema-login-greeter-args-label = Аргументи вітального екрана
schema-login-greeter-args-description = Аргументи, які передають вітальному екрану.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = Перемикач слідує за вказівником
schema-appswitcher-follow-cursor-description = Показувати перемикач вікон на виході, де перебуває вказівник.
schema-appswitcher-colorize-icons-label = Тонувати піктограми перемикача
schema-appswitcher-colorize-icons-description = Застосовувати тонування піктограм Dock і до перемикача вікон. Нічого не робить, доки тонування Dock вимкнено.


## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Автоматично
settings-choice-system-language = Системна мова


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Синій
settings-choice-accent-purple = Фіолетовий
settings-choice-accent-pink = Рожевий
settings-choice-accent-red = Червоний
settings-choice-accent-orange = Оранжевий
settings-choice-accent-yellow = Жовтий
settings-choice-accent-green = Зелений
settings-choice-accent-mint = М'ятний
settings-choice-accent-teal = Бірюзовий
settings-choice-accent-cyan = Блакитний
settings-choice-accent-indigo = Індиго
settings-choice-accent-brown = Коричневий
settings-choice-accent-graphite = Графітовий
# The button under the shortcut list that adds another line.
settings-add-shortcut = Додати комбінацію
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Робочий простір { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Пошук застосунків і вікон…
launcher-search-apps = Пошук застосунків…
launcher-search-windows = Пошук вікон…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = Застосунок
launcher-badge-window = Вікно
launcher-badge-calc = Калькулятор


## Login, lock and authentication
##
## The greeter, the lock screen, and the panel both of them draw. Text that
## arrives from PAM or greetd at runtime is not here: those localise
## themselves, and restating them would be guessing at another program's words.
# Button under the login/lock card, offered only while the fingerprint reader
# is being waited on: it abandons the finger and asks for a password instead.
# The button sizes itself to the text, but it sits on a card 380pt wide — keep
# it to roughly 20 characters so it does not overhang.
auth-enter-password = Ввести пароль

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Вхід

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
greeter-prompt-username = Ім'я користувача

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Пароль

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Автентифікація…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Служба входу недоступна: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Служба входу розірвала з'єднання

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = Сеанс { $session } не запустився

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Автентифікацію пройдено

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Прикладіть палець до сканера

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Проведіть пальцем по сканеру

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Торкніться сканера { $finger }
greeter-status-swipe-named-finger = Проведіть по сканеру { $finger }

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Відбиток не розпізнано

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Очікування сканера відбитків…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Запуск сеансу…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Перехід у сплячий режим не дозволено

# As above, for the restart request.
greeter-power-restart-denied = Перезавантаження не дозволено

# As above, for the shut down request.
greeter-power-shutdown-denied = Вимкнення не дозволено

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Не вдалося перейти у сплячий режим

# As above, for the restart request.
greeter-power-restart-failed = Не вдалося перезавантажити

# As above, for the shut down request.
greeter-power-shutdown-failed = Не вдалося вимкнути

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Пароль

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Автентифікацію пройдено

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Прикладіть палець до сканера

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Проведіть пальцем по сканеру

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Торкніться сканера { $finger }
lock-status-swipe-named-finger = Проведіть по сканеру { $finger }

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = лівим великим пальцем
auth-finger-left-index = лівим вказівним пальцем
auth-finger-left-middle = лівим середнім пальцем
auth-finger-left-ring = лівим безіменним пальцем
auth-finger-left-little = лівим мізинцем
auth-finger-right-thumb = правим великим пальцем
auth-finger-right-index = правим вказівним пальцем
auth-finger-right-middle = правим середнім пальцем
auth-finger-right-ring = правим безіменним пальцем
auth-finger-right-little = правим мізинцем

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Відбиток не розпізнано

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Очікування сканера відбитків…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Немає користувача для автентифікації

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Збій служби автентифікації

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Неприпустиме ім'я користувача

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Автентифікація недоступна

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Автентифікацію не пройдено ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Перехід у сплячий режим не дозволено

# As above, for the restart request.
lock-power-restart-denied = Перезавантаження не дозволено

# As above, for the shut down request.
lock-power-shutdown-denied = Вимкнення не дозволено

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Не вдалося перейти у сплячий режим: { $error }

# As above, for the restart request.
lock-power-restart-failed = Не вдалося перезавантажити: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Не вдалося вимкнути: { $error }


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
quickview-fact-kind = Тип
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = Розмір
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Розміри
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Тривалість
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Пікселі
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Сторінки
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Назва
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Виконавець
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Альбом
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Рік

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Порожній файл
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Завелике для перегляду
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels =
    { $count ->
        [one] { $count } мегапіксель
        [few] { $count } мегапікселі
        [many] { $count } мегапікселів
       *[other] { $count } мегапікселя
    }
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Щоб побачити сторінки, встановіть один із пакунків: { $packages }


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Порожня папка
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } елемент
        [few] { $count } елементи
        [many] { $count } елементів
       *[other] { $count } елемента
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
        [one] { $count } байт
        [few] { $count } байти
        [many] { $count } байтів
       *[other] { $count } байта
    }
quickview-size-kb = { $value } КБ
quickview-size-mb = { $value } МБ
quickview-size-gb = { $value } ГБ
quickview-size-tb = { $value } ТБ


## Quick View — nothing to show
##
## Each of these fills the card in place of a preview, so a person reads it
## instead of seeing the file. They state what happened and stop. Lower case,
## no full stop: they are shown as a sentence fragment.
##
## $error is an operating-system message, which arrives in whatever language
## the system libraries produce and is usually English. Keep it at the end.

# The file is a pipe, socket or device — opening it could block forever.
quickview-error-not-previewable = цей файл неможливо переглянути
# The file's metadata could not be read.
quickview-error-stat-file = не вдалося отримати відомості про файл: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = не вдалося прочитати файл: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = у файлі неможливе переміщення
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = не вдалося ізолювати програму перегляду: { $error }

# Image previewer.
quickview-error-read-image = не вдалося прочитати зображення: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = ця збірка не підтримує такий формат зображення
quickview-error-image-no-size = зображення не повідомляє свій розмір
quickview-error-image-decode = зображення не декодовано: { $error }
quickview-error-image-readback = не вдалося прочитати декодоване зображення

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = не вдалося прочитати рисунок: { $error }
quickview-error-drawing-parse = не вдалося розібрати рисунок
quickview-error-drawing-surface = немає поверхні для його відтворення
quickview-error-drawing-readback = не вдалося прочитати готовий рисунок

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = це не текст у жодному з кодувань, які читає Otto

# PDF previewer.
quickview-error-read-document = не вдалося прочитати документ: { $error }
quickview-error-page-readback = не вдалося прочитати відтворену сторінку

# Folder listing.
quickview-error-read-folder = не вдалося прочитати папку

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = не вдалося знайти програму перегляду: { $error }
quickview-error-previewer-start = не вдалося запустити програму перегляду: { $error }
quickview-error-previewer-no-output = програма перегляду нічого не видала
quickview-error-previewer-unreadable = програма перегляду видала щось нечитабельне
quickview-error-previewer-failed = збій програми перегляду: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = перегляд цього файлу тривав задовго

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
islands-close = Закрити

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = щойно
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count } хв тому
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count } год тому


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Дозволити
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Продовжити
# Refuses the request.
islands-dialog-deny = Відмовити


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Док
a11y-app-running = Запущено
a11y-app-not-running = Не запущено
a11y-app-switcher = Перемикач програм
a11y-windows = Вікна
a11y-workspaces = Робочі простори
a11y-untitled-window = Вікно без назви
a11y-menu-bar = Рядок меню
a11y-status = Стан
a11y-tray-item = Елемент { $number }
a11y-notifications = Сповіщення
a11y-categories = Категорії
a11y-results = Результати
a11y-settings = Налаштування
a11y-preview = Перегляд
a11y-preview-page = Перегляд, сторінка { $page } з { $pages }
a11y-preview-shortened = Перегляд, скорочений
