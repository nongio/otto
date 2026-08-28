# Otto — Russian
#
# Mirrors resources/locales/en-GB.ftl (the source catalogue). Keep keys,
# order and comments in sync with it.
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

common-open = Открыть
common-save = Сохранить
common-cancel = Отмена
common-add = Добавить
common-remove = Убрать
common-quit = Завершить
common-cut = Вырезать
common-copy = Копировать
common-paste = Вставить
common-rename = Переименовать
common-delete = Удалить
common-move = Переместить


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Автоскрытие
dock-auto-hide-on = ✓ Автоскрытие
dock-magnification = Увеличение
dock-magnification-on = ✓ Увеличение
dock-position-bottom = Внизу
dock-position-bottom-on = ✓ Внизу
dock-position-left = Слева
dock-position-left-on = ✓ Слева
dock-position-right = Справа
dock-position-right-on = ✓ Справа

# Shown on an app's icon when the app is not running.
dock-open = Открыть
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Оставить в Dock
dock-keep-in-dock-on = ✓ Оставить в Dock
dock-quit = Завершить


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Основные
settings-pane-displays = Дисплеи
settings-pane-dock = Dock
settings-pane-keyboard = Клавиатура
settings-pane-pointing = Трекпад и мышь
settings-pane-sound = Звук
settings-pane-power = Питание
settings-pane-lock-and-login = Блокировка и вход


## Settings — General

settings-group-appearance = Оформление
settings-appearance = Оформление
settings-accent-colour = Цвет акцента
settings-font = Шрифт
settings-gtk-theme = Тема GTK

settings-group-desktop = Рабочий стол
settings-background-colour = Цвет фона
settings-background-image = Изображение фона
settings-background-image-detail = Выбирается через диалог выбора файлов портала рабочего стола

settings-group-pointer-and-icons = Указатель и значки
settings-cursor-theme = Тема курсора
settings-cursor-size = Размер курсора
settings-icon-theme = Тема значков

settings-group-window-switcher = Переключатель окон
settings-follow-cursor = Показывать на дисплее с указателем

settings-group-language = Язык
settings-preferred-languages = Предпочитаемые языки

settings-group-configuration = Конфигурация
settings-configuration-file = Файл конфигурации
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = неизвестно — композитор не отвечает


## Settings — Displays

settings-display-active = Активен
settings-display-active-detail = Неактивный дисплей сохраняет своё место в расположении
settings-display-primary = Использовать как основной
settings-display-primary-detail = Dock и панель находятся на основном дисплее
settings-display-x-position = Позиция X
settings-display-y-position = Позиция Y
settings-display-x-position-detail = Верхний левый угол в системе координат рабочего стола
settings-display-width = Ширина
settings-display-width-detail = Пиксели. Автономный вывод может быть любого размера
settings-display-height = Высота
settings-display-refresh = Частота обновления
settings-display-refresh-detail = Герц — как часто в поток подаётся кадр
settings-display-resolution = Разрешение
settings-display-scale = Масштаб дисплея
settings-display-scale-detail = Применяется при следующем входе. Рабочий стол не перестраивается на лету

# Shown when the compositor reports no outputs at all.
settings-display-none = Нет дисплеев
settings-display-none-detail = Композитор не управляет ни одним выводом

settings-virtual-displays = Виртуальные дисплеи
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } автономный вывод, транслируется через PipeWire. «Убрать» удаляет выбранный
        [few] { $count } автономных вывода, транслируются через PipeWire. «Убрать» удаляет выбранный
        [many] { $count } автономных выводов, транслируются через PipeWire. «Убрать» удаляет выбранный
       *[other] { $count } автономного вывода, транслируется через PipeWire. «Убрать» удаляет выбранный
    }


## Settings — Dock

settings-dock-size = Размер
settings-dock-position = Положение на экране
settings-dock-autohide = Автоматически скрывать
settings-dock-magnification = Увеличение
settings-group-magnification-and-icons = Увеличение и значки
settings-dock-magnification-amount = Степень увеличения
settings-dock-tint-icons = Тонировать значки
settings-dock-icon-tint = Оттенок значков
settings-dock-icon-tint-strength = Сила оттенка значков


## Settings — Keyboard

settings-key-repeat-delay = Задержка перед повтором
settings-key-repeat-rate = Скорость повтора
settings-group-input-source = Источник ввода
settings-xkb-layout = Раскладка
settings-xkb-variant = Вариант
settings-xkb-options = Параметры
settings-group-shortcuts = Сочетания клавиш
settings-key-combination = Сочетание клавиш
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift или Logo, соединённые «+», затем одна клавиша: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Трекпад
settings-tap-to-click = Нажатие касанием
settings-tap-and-drag = Касание и перетаскивание
settings-drag-lock = Блокировка перетаскивания
settings-click-method = Способ нажатия
settings-ignore-while-typing = Игнорировать при наборе текста
settings-natural-scrolling = Естественная прокрутка
settings-left-handed = Для левши
settings-middle-click-emulation = Эмуляция средней кнопки
settings-group-pointer = Указатель
settings-tracking-speed = Скорость отслеживания
settings-pointer-acceleration = Ускорение
settings-scrolling-speed = Скорость прокрутки


## Settings — Sound

settings-interface-sounds = Звуки интерфейса
settings-sound-theme = Звуковая тема


## Settings — Power

settings-manage-lid-switch = Обрабатывать закрытие крышки
settings-manage-lid-switch-detail = Otto переводит систему в спящий режим при закрытии крышки вместо logind
settings-on-lid-close = При закрытии крышки
settings-on-power-button = При нажатии кнопки питания


## Settings — Lock & Login

settings-group-lock = Блокировка
settings-lock-after = Блокировать через
settings-lock-screen = Экран блокировки
settings-lock-screen-detail = Применяется при следующей блокировке экрана
settings-lock-screen-arguments = Аргументы экрана блокировки
settings-group-login = Вход
settings-greeter = Экран приветствия
settings-greeter-detail = Применяется при следующем входе
settings-greeter-arguments = Аргументы экрана приветствия


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Светлая
settings-choice-dark = Тёмная
settings-choice-position-bottom = Внизу
settings-choice-position-left = Слева
settings-choice-position-right = Справа
settings-choice-clickfinger = Нажатие пальцами
settings-choice-buttonareas = Нажатие в углах
settings-choice-accel-flat = Постоянная скорость
settings-choice-accel-adaptive = Скорость зависит от движения
settings-choice-lid-auto = Решать автоматически
settings-choice-lid-lock = Заблокировать экран
settings-choice-lid-disable-internal = Отключить встроенный дисплей
settings-choice-power-ignore = Ничего не делать
settings-choice-power-lock = Заблокировать экран
settings-choice-power-suspend = Перейти в спящий режим
settings-choice-power-shutdown = Выключить
# The automatic option for a theme that follows the system.
settings-choice-auto = Авто


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value }%
settings-readout-pixels = { $value } пикс.
settings-readout-milliseconds = { $value } мс
settings-readout-seconds = { $value } с
# Key repeats per second.
settings-readout-per-second = { $value } / с


## Files — commands

files-get-info = Свойства
files-new-folder = Новая папка
files-move-to-trash = Переместить в корзину
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Переместить { $count } элемент в корзину
        [few] Переместить { $count } элемента в корзину
        [many] Переместить { $count } элементов в корзину
       *[other] Переместить { $count } элемента в корзину
    }


## Files — sidebar and columns

files-places = Места
files-home = Домашняя папка
files-desktop = Рабочий стол
files-documents = Документы
files-downloads = Загрузки
files-music = Музыка
files-pictures = Изображения
files-videos = Видео
files-trash = Корзина

files-column-name = Имя
files-column-size = Размер
files-column-kind = Тип
files-column-date-modified = Дата изменения


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Папка
files-kind-image = Изображение
files-kind-movie = Видео
files-kind-audio = Аудио
files-kind-text = Текст
files-kind-document = Документ
files-kind-archive = Архив
files-kind-application = Приложение


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Загрузка…
files-empty = Пусто
files-nothing-to-undo = Нечего отменять
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Действие отменено: { $label }
files-undo-move = Перемещение
files-undo-copy = Копирование
files-undo-delete = Удаление
files-undo-rename = Переименование
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Переименовано в «{ $name }»
files-new-folder-created = Новая папка «{ $name }»
files-gone = «{ $name }» больше не существует
files-rename-failed = Не удалось переименовать: { $error }
files-new-folder-failed = Не удалось создать папку: { $error }
files-open-failed = Не удалось открыть файл: { $error }
files-new-window-failed = Не удалось открыть новое окно: { $error }


## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Открыть
files-picker-save-as = Сохранить как
files-picker-save-files = Сохранить файлы
files-picker-all-files = Все файлы

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Введите имя
files-save-name-has-slash = Имя не может содержать «/»
files-save-name-reserved = Это имя зарезервировано


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
        [one] { $count } байт
        [few] { $count } байта
        [many] { $count } байт
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

files-date-modified = { $day } { $month } { $year }, { $time }

files-month-jan = янв.
files-month-feb = февр.
files-month-mar = мар.
files-month-apr = апр.
files-month-may = мая
files-month-jun = июн.
files-month-jul = июл.
files-month-aug = авг.
files-month-sep = сент.
files-month-oct = окт.
files-month-nov = нояб.
files-month-dec = дек.


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
settings-not-set = Не задано
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Выбрать…
settings-no-file-chosen = Файл не выбран
settings-choose-background-image = Выбор изображения фона

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Настройки Otto — { $pane }

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
schema-screen-scale-description = Общий коэффициент масштабирования рабочего стола.
schema-theme-scheme-label = Оформление
schema-theme-scheme-description = Светлая или тёмная цветовая схема.
schema-accent-color-label = Цвет акцента
schema-accent-color-description = Именованный цвет акцента, используемый собственным интерфейсом Otto.
schema-font-family-label = Шрифт интерфейса
schema-font-family-description = Семейство шрифтов, используемое собственным интерфейсом Otto.
schema-background-color-label = Цвет фона
schema-background-color-description = Цвет фона рабочего стола в виде шестнадцатеричной строки.
schema-background-image-label = Изображение фона
schema-background-image-description = Путь к изображению фона рабочего стола. Пусто, если фон не задан.
schema-cursor-theme-label = Тема курсора
schema-cursor-theme-description = Название темы XCursor.
schema-cursor-size-label = Размер курсора
schema-cursor-size-description = Размер курсора в логических пикселях.
schema-icon-theme-label = Тема значков
schema-icon-theme-description = Название темы значков. Пусто — определяется автоматически.
schema-gtk-theme-label = Тема GTK
schema-gtk-theme-description = Название темы GTK, передаваемое клиентам. Пусто — определяется автоматически.
schema-locales-label = Локали
schema-locales-description = Предпочитаемые локали, в порядке убывания предпочтения.

# --- dock ---
schema-dock-size-label = Размер
schema-dock-size-description = Множитель размера Dock.
schema-dock-position-label = Положение на экране
schema-dock-position-description = Край экрана, у которого расположен Dock.
schema-dock-autohide-label = Автоматически скрывать
schema-dock-autohide-description = Скрывать Dock, пока указатель не достигнет края экрана, где он находится.
schema-dock-magnification-label = Увеличение
schema-dock-magnification-description = Увеличивать значки под указателем.
schema-dock-genie-scale-label = Степень увеличения
schema-dock-genie-scale-description = Насколько увеличиваются значки под указателем.
schema-dock-genie-span-label = Радиус увеличения
schema-dock-genie-span-description = Сколько соседних значков затрагивает увеличение.
schema-dock-colorize-icons-label = Тонировать значки
schema-dock-colorize-icons-description = Тонировать значки Dock одним цветом.
schema-dock-colorize-color-label = Оттенок значков
schema-dock-colorize-color-description = Цвет, которым тонируются значки Dock, в виде шестнадцатеричной строки.
schema-dock-colorize-intensity-label = Сила оттенка значков
schema-dock-colorize-intensity-description = Насколько сильно применяется тонирование.

# --- general ---
schema-keyboard-repeat-delay-label = Задержка повтора
schema-keyboard-repeat-delay-description = Сколько миллисекунд клавиша должна быть зажата, прежде чем начнётся повтор.
schema-keyboard-repeat-rate-label = Скорость повтора
schema-keyboard-repeat-rate-description = Количество повторов в секунду при удержании клавиши.

# --- input ---
schema-input-xkb-layout-label = Раскладка клавиатуры
schema-input-xkb-layout-description = Название раскладки XKB. Пусто — используется системная раскладка по умолчанию.
schema-input-xkb-variant-label = Вариант клавиатуры
schema-input-xkb-variant-description = Название варианта XKB. Пусто — используется системный вариант по умолчанию.
schema-input-xkb-options-label = Параметры клавиатуры
schema-input-xkb-options-description = Строки параметров XKB.
schema-input-tap-enabled-label = Нажатие касанием
schema-input-tap-enabled-description = Считать касание тачпада нажатием.
schema-input-tap-drag-enabled-label = Касание и перетаскивание
schema-input-tap-drag-enabled-description = Начинать перетаскивание с касания, за которым следует удержание пальца.
schema-input-tap-drag-lock-enabled-label = Блокировка перетаскивания
schema-input-tap-drag-lock-enabled-description = Продолжать перетаскивание через касание при кратком отрыве пальца.
schema-input-touchpad-click-method-label = Способ нажатия
schema-input-touchpad-click-method-description = Определяет ли нажатие количество пальцев или зона кнопки.
schema-input-touchpad-dwt-enabled-label = Отключать при наборе текста
schema-input-touchpad-dwt-enabled-description = Игнорировать тачпад, пока используется клавиатура.
schema-input-touchpad-natural-scroll-enabled-label = Естественная прокрутка
schema-input-touchpad-natural-scroll-enabled-description = Содержимое следует за пальцами.
schema-input-touchpad-left-handed-label = Для левши
schema-input-touchpad-left-handed-description = Поменять местами основную и дополнительную кнопки.
schema-input-touchpad-middle-emulation-enabled-label = Эмуляция средней кнопки
schema-input-touchpad-middle-emulation-enabled-description = Одновременное нажатие обеих кнопок считается нажатием средней кнопки.
schema-input-scroll-speed-label = Скорость прокрутки
schema-input-scroll-speed-description = Программный множитель, применяемый к событиям прокрутки.
schema-input-pointer-accel-speed-label = Скорость указателя
schema-input-pointer-accel-speed-description = Ускорение указателя — от -1 (самое медленное) до 1 (самое быстрое).
schema-input-pointer-accel-profile-label = Ускорение указателя
schema-input-pointer-accel-profile-description = Постоянное — исходная скорость без изменений; адаптивное следует кривой libinput.

# --- audio ---
schema-audio-sound-enabled-label = Звуки интерфейса
schema-audio-sound-enabled-description = Воспроизводить звуковую обратную связь для событий интерфейса.
schema-audio-sound-theme-label = Звуковая тема
schema-audio-sound-theme-description = Название звуковой темы XDG. Пусто — определяется автоматически.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Обрабатывать закрытие крышки
schema-power-management-manage-lid-switch-description = Позволить Otto реагировать на крышку вместо того, чтобы оставлять это logind.
schema-power-management-on-lid-close-label = При закрытии крышки
schema-power-management-on-lid-close-description = Что происходит при закрытии крышки ноутбука.
schema-power-management-on-power-button-label = При нажатии кнопки питания
schema-power-management-on-power-button-description = Что происходит при нажатии аппаратной кнопки питания.

# --- lock ---
schema-lock-locker-command-label = Команда экрана блокировки
schema-lock-locker-command-description = Программа блокировки, запускаемая для блокировки сеанса.
schema-lock-locker-args-label = Аргументы экрана блокировки
schema-lock-locker-args-description = Аргументы, передаваемые программе блокировки.
schema-lock-auto-lock-timeout-label = Блокировать через
schema-lock-auto-lock-timeout-description = Секунды бездействия до блокировки. 0 отключает блокировку.

# --- login ---
schema-login-greeter-command-label = Команда экрана приветствия
schema-login-greeter-command-description = Приветственный экран, запускаемый в режиме входа.
schema-login-greeter-args-label = Аргументы экрана приветствия
schema-login-greeter-args-description = Аргументы, передаваемые экрану приветствия.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = Переключатель следует за указателем
schema-appswitcher-follow-cursor-description = Показывать переключатель окон на выводе, где находится указатель.


## Late additions

# Shown in the middle of a listing with nothing in it.
files-folder-empty = Эта папка пуста.
# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Автоматически


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Синий
settings-choice-accent-purple = Фиолетовый
settings-choice-accent-pink = Розовый
settings-choice-accent-red = Красный
settings-choice-accent-orange = Оранжевый
settings-choice-accent-yellow = Жёлтый
settings-choice-accent-green = Зелёный
settings-choice-accent-mint = Мятный
settings-choice-accent-teal = Бирюзовый
settings-choice-accent-cyan = Голубой
settings-choice-accent-indigo = Индиго
settings-choice-accent-brown = Коричневый
settings-choice-accent-graphite = Графитовый
# The button under the shortcut list that adds another line.
settings-add-shortcut = Добавить сочетание
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Рабочий стол { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Поиск приложений и окон…
launcher-search-apps = Поиск приложений…
launcher-search-windows = Поиск окон…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = Приложение
launcher-badge-window = Окно
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
auth-sign-in = Вход

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
greeter-prompt-username = Имя пользователя

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Пароль

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Аутентификация…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Служба входа недоступна: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = Служба входа разорвала соединение

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = Сеанс { $session } не запустился

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Аутентификация пройдена

# Status line under the fingerprint mark while the reader is waiting for a
# finger. Shown only when the fingerprint module gave no message of its own.
# One line, clipped at roughly 40 characters.
greeter-status-place-finger = Приложите палец к сканеру

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Ожидание сканера отпечатков…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Запуск сеанса…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Переход в спящий режим не разрешён

# As above, for the restart request.
greeter-power-restart-denied = Перезагрузка не разрешена

# As above, for the shut down request.
greeter-power-shutdown-denied = Выключение не разрешено

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Не удалось перейти в спящий режим

# As above, for the restart request.
greeter-power-restart-failed = Не удалось перезагрузить

# As above, for the shut down request.
greeter-power-shutdown-failed = Не удалось выключить

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
lock-status-authenticated = Аутентификация пройдена

# Status line under the fingerprint mark while the reader is waiting for a
# finger. Shown only when the fingerprint module gave no message of its own.
# One line, clipped at roughly 40 characters.
lock-status-place-finger = Приложите палец к сканеру

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Ожидание сканера отпечатков…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Нет пользователя для аутентификации

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Сбой службы аутентификации

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Недопустимое имя пользователя

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = Аутентификация недоступна

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Аутентификация не пройдена ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Переход в спящий режим не разрешён

# As above, for the restart request.
lock-power-restart-denied = Перезагрузка не разрешена

# As above, for the shut down request.
lock-power-shutdown-denied = Выключение не разрешено

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Не удалось перейти в спящий режим: { $error }

# As above, for the restart request.
lock-power-restart-failed = Не удалось перезагрузить: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Не удалось выключить: { $error }


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
quickview-fact-size = Размер
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Размеры
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Длительность
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Пиксели
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = Страницы
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = Название
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = Исполнитель
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = Альбом
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = Год

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Пустой файл
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Слишком большое для просмотра
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels =
    { $count ->
        [one] { $count } мегапиксель
        [few] { $count } мегапикселя
        [many] { $count } мегапикселей
       *[other] { $count } мегапикселя
    }
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Чтобы увидеть страницы, установите один из пакетов: { $packages }


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Пустая папка
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } элемент
        [few] { $count } элемента
        [many] { $count } элементов
       *[other] { $count } элемента
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
        [few] { $count } байта
        [many] { $count } байт
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
quickview-error-not-previewable = этот файл нельзя просмотреть
# The file's metadata could not be read.
quickview-error-stat-file = не удалось получить сведения о файле: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = не удалось прочитать файл: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = в файле невозможно перемещение
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = не удалось изолировать программу просмотра: { $error }

# Image previewer.
quickview-error-read-image = не удалось прочитать изображение: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = эта сборка не поддерживает такой формат изображения
quickview-error-image-no-size = изображение не сообщает свой размер
quickview-error-image-decode = изображение не декодировано: { $error }
quickview-error-image-readback = не удалось прочитать декодированное изображение

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = не удалось прочитать рисунок: { $error }
quickview-error-drawing-parse = не удалось разобрать рисунок
quickview-error-drawing-surface = нет поверхности для его отрисовки
quickview-error-drawing-readback = не удалось прочитать готовый рисунок

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = это не текст ни в одной из читаемых кодировок

# PDF previewer.
quickview-error-read-document = не удалось прочитать документ: { $error }
quickview-error-page-readback = не удалось прочитать отрисованную страницу

# Folder listing.
quickview-error-read-folder = не удалось прочитать папку

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = не удалось найти программу просмотра: { $error }
quickview-error-previewer-start = не удалось запустить программу просмотра: { $error }
quickview-error-previewer-no-output = программа просмотра ничего не выдала
quickview-error-previewer-unreadable = программа просмотра выдала неразборчивый результат
quickview-error-previewer-failed = сбой программы просмотра: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = просмотр этого файла занял слишком много времени

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
islands-close = Закрыть

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = только что
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count } мин назад
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count } ч назад


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = Разрешить
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = Продолжить
# Refuses the request.
islands-dialog-deny = Запретить
