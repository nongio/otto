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
common-replace = Замінити
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
settings-pane-appearance = Вигляд
settings-pane-displays = Дисплеї
settings-pane-dock = Dock
settings-pane-top-bar = Верхня панель
settings-pane-tiling = Мозаїка
settings-pane-keyboard = Клавіатура
settings-pane-pointing = Трекпад і миша
settings-pane-sound = Звук
settings-pane-power = Живлення
settings-pane-lock-and-login = Блокування і вхід
settings-pane-search = Пошук
settings-pane-agents = Агенти
settings-pane-about = Про Otto
settings-sidebar-search = Пошук
settings-sidebar-search-none = Немає налаштувань, що відповідають «{ $query }»
settings-group-about-machine = Цей комп’ютер
settings-about-version-line = Версія { $version }
settings-about-computer-name = Назва комп’ютера
settings-about-os = Операційна система
settings-about-kernel = Ядро
settings-about-processor = Процесор
settings-about-memory = Пам’ять
settings-about-memory-gb = { $size } ГБ


## Settings — General

settings-group-appearance = Вигляд
settings-colour-scheme = Схема кольорів
settings-accent-colour = Колір акценту
settings-rounded-corners = Заокруглені кути
settings-frosting = Матове скло
settings-frosting-detail = Напівпрозорий розмитий матеріал за доком, панеллю та панелями стільниці
settings-window-controls = Кнопки вікна
settings-maximize-button = Кнопка розгортання
settings-maximize-button-detail = Показує кружечок масштабування; подвійне клацання на заголовку однаково розгортає вікно
settings-font = Системний шрифт
settings-gtk-theme = Тема GTK

settings-group-desktop = Робочий стіл
settings-background-colour = Колір тла
settings-background-image = Зображення тла
settings-background-image-detail = Обирається через засіб вибору файлів робочого стола
settings-desktop-widget = Фоновий віджет
settings-desktop-widget-needs-ewwii = Потрібен ewwii, а його не встановлено
# { $folder } is a path such as ~/.config/otto/widgets/ewwii.
settings-desktop-widget-detail = Малюється ewwii. Додавай власні віджети в { $folder }
# Stands in for the wallpaper thumbnail when the file cannot be decoded.
settings-background-image-unavailable = Неможливо показати
settings-show-desk = Показувати файли на робочому столі
settings-show-desk-detail = Файли з папки «Стільниця», позаду вікон

settings-group-desk = Стільниця
settings-desk-folder = Тека
settings-desk-folder-default = Ваша тека «Стільниця»
settings-desk-choose-folder-title = Виберіть теку для стільниці
settings-desk-layout = Розмір і положення
settings-desk-layout-edit = Змінити…
settings-desk-layout-reset = Скинути
settings-desk-layout-fill = На весь екран. «Змінити» показує маркери, щоб перетягнути її на місце
settings-desk-layout-placed = Розміщено вручну. «Скинути» знову розтягне її на весь екран
settings-desk-icon-size = Розмір значків
settings-desk-overflow = Коли значки не вміщаються
settings-desk-overflow-scroll = Прокручувати
settings-desk-overflow-stack = Показувати поверх
settings-desk-overflow-detail = Прокручування зсуває сітку. «Показувати поверх» залишає зайві значки в останній клітинці й відкриває їх над вікнами

settings-group-pointer-and-icons = Вказівник і піктограми
settings-cursor-theme = Тема курсора
settings-cursor-size = Розмір курсора
settings-icon-theme = Тема піктограм

settings-group-app-menu = Меню програм
settings-show-app-menu = Показувати меню програм
settings-show-app-menu-detail = Поруч із назвою активної програми. Програми, відкриті, поки це вимкнено, залишають меню у власному вікні

settings-group-clock = Годинник
settings-show-clock = Показувати дату й час
settings-show-clock-detail = Біля правого краю верхньої панелі
settings-clock-format = Формат
settings-clock-format-detail = Підійде й будь-який формат strftime: clock_format у розділі [topbar] файлу конфігурації
settings-clock-format-automatic = { $preview } (типово для мови)

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
settings-renderer = Рушій рендерингу
settings-renderer-detail = Стосується сеансу, запущеного з вітального екрана. Сеанси у вікні завжди використовують OpenGL. Застосовується під час наступного входу.
settings-renderer-no-vulkan-detail = Vulkan тут недоступний, тож Otto малює через OpenGL.
settings-renderer-windowed-detail = Сеанси у вікні завжди використовують OpenGL. Змінити це можна в сеансі, запущеному з вітального екрана.

# Shown when the compositor reports no outputs at all.
settings-display-none = Немає дисплеїв
settings-display-none-detail = Композитор не керує жодним виходом
# Under the display arrangement canvas, explaining what clicking one does.
settings-arrangement-hint = Натисніть на екран, щоб змінити його налаштування нижче

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
# Pill on a row whose setting the compositor cannot apply until it restarts.
# Kept short: it is drawn inside a badge beside the row's label.
settings-restart-required = Потрібен перезапуск


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


## Settings — Tiling

settings-tiling-intro = Мозаїчний робочий простір заповнює екран своїми вікнами, поруч одне з одним. Це типові значення; окремий робочий простір може задати власні відступи.
settings-tiling-decoration = Оформлення мозаїчних вікон
settings-group-tiling-gaps = Відступи
settings-tiling-inner-gap = Між вікнами
settings-tiling-outer-gap = Навколо вікон
settings-tiling-smart-gaps = Без відступів для одного вікна
settings-group-tiling-keyboard = Клавіатура
settings-tiling-resize-step = Крок зміни розміру
settings-group-tiling-animation = Анімація
settings-tiling-layout-duration = Зміна розташування
settings-tiling-layout-bounce = Відскок розташування
settings-tiling-mode-duration = Перехід у мозаїку
settings-tiling-mode-bounce = Відскок мозаїки


## Settings — Keyboard

settings-key-repeat-delay = Затримка повтору клавіш
settings-key-repeat-rate = Швидкість повтору клавіш
settings-group-input-source = Джерело введення
settings-xkb-layout-nth = Розкладка { $n }
settings-xkb-layout-default = Системна за умовчанням
settings-xkb-layout-remove = Вилучити цю розкладку
settings-xkb-layouts = Розкладки
settings-xkb-layouts-detail = До чотирьох, за порядком. Сеанс починається з першої.
settings-xkb-variant = Варіант
settings-xkb-variant-standard = Стандартний
settings-xkb-switch = Перемикати розкладки клавішами
settings-xkb-switch-detail = Клавіші, що перемикають на наступну розкладку.
settings-xkb-switch-none = Немає
settings-xkb-show-in-bar = Показувати на панелі
settings-xkb-options = Параметри
settings-group-shortcuts = Комбінації клавіш
settings-key-combination = Комбінація клавіш
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift або Logo, поєднані знаком +, а потім одна клавіша: Ctrl+Shift+Return
# Shown in a shortcut's key field when no combination is set.
settings-key-combination-unassigned = Не призначено
# Shown in a shortcut's key field while its record button waits for a
# combination to be pressed.
settings-key-combination-listening = Натисніть клавіші…


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
settings-group-sound-output = Пристрої виведення
settings-group-sound-input = Пристрої введення
settings-sound-output-device = Пристрій виведення
settings-sound-input-device = Пристрій введення
settings-sound-volume = Гучність
settings-sound-mute = Вимкнути звук
settings-sound-no-outputs = Немає пристроїв виведення
settings-sound-no-inputs = Немає пристроїв введення
settings-sound-unavailable = Звуковий сервер не відповідає. Для звуку потрібен PipeWire з pipewire-pulse або PulseAudio, а також pactl.
settings-sound-show = Показати
settings-sound-view-playback = Відтворення
settings-sound-view-recording = Запис
settings-sound-view-configuration = Конфігурація
settings-sound-port = Порт
settings-sound-unplugged = не під'єднано
settings-sound-default = Використовувати типово
settings-sound-profile = Профіль
settings-sound-no-playback = Жодна програма не відтворює звук
settings-sound-no-recording = Жодна програма не записує звук
settings-sound-no-cards = Немає звукових карт


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
settings-lock-on-suspend = Блокувати під час переходу в сон
settings-group-login = Вхід
settings-greeter = Вітальний екран
settings-greeter-detail = Застосовується під час наступного входу
settings-greeter-arguments = Аргументи вітального екрана
settings-lock-never = Ніколи
# Under each Lock & Login row: the compositor asks for the password before it applies a change.
settings-asks-for-password = Для зміни знадобиться пароль
settings-login-background-failed = Не вдалося змінити тло екрана входу
settings-login-background-not-image = Це не зображення PNG, JPEG чи WebP
settings-login-background-too-large = Більше за 20 МБ
settings-images-filter = Зображення
# The auto-lock interval pop-up.
settings-interval-minutes =
    { $count ->
        [one] { $count } хвилина
        [few] { $count } хвилини
        [many] { $count } хвилин
       *[other] { $count } хвилини
    }
settings-interval-hours =
    { $count ->
        [one] { $count } година
        [few] { $count } години
        [many] { $count } годин
       *[other] { $count } години
    }
settings-interval-seconds =
    { $count ->
        [one] { $count } секунда
        [few] { $count } секунди
        [many] { $count } секунд
       *[other] { $count } секунди
    }


## Settings — Search
##
## The file index Files searches, which LocalSearch keeps. These rows show
## what it is doing and change what it looks at, through the [search] section
## of Otto's configuration.

settings-search-intro = Файли шукають в індексі ваших файлів, який LocalSearch оновлює у фоні.
# The row showing what the indexer is doing.
settings-search-index = Індекс файлів
settings-search-checking = Перевірка…
# $files is already written with its digits grouped, such as 48,210.
settings-search-idle = Актуальний · файлів: { $files }
settings-search-idle-uncounted = Актуальний
# $percent is how far through the indexer is, 0 to 99.
settings-search-indexing = Індексування… { $percent }%
# $minutes is the indexer's own estimate, rounded up.
settings-search-indexing-minutes = Індексування… { $percent }% · лишилося близько { $minutes } хв
settings-search-indexing-hours = Індексування… { $percent }% · лишилося близько { $hours } год
settings-search-paused = Призупинено
settings-search-paused-detail = Індексатор призупиняється, коли мало заряду чи місця на диску, а також на прохання застосунку. Потім продовжує сам.
# Under a Start button. A search starts the indexer by itself too.
settings-search-stopped = Не запущено. Пошук запустить його сам, або можна запустити зараз.
settings-search-start = Запустити
settings-search-missing = Не встановлено
# localsearch is the package name; keep it as is.
settings-search-missing-detail = Для пошуку файлів потрібен пакет localsearch. Встановіть його й знову відкрийте цю сторінку.
# What goes between groups of three digits in a count: 48,210. Written as
# a string so a space survives.
settings-search-digit-separator = { "\u00A0" }
# The heading over the folders, switches and Re-index.
settings-search-looks-in = Що індексується
settings-search-folders = Теки
# The home folder, in the list of folders the index looks in.
settings-search-folder-home = Домашня тека
settings-search-folders-none = Немає
# The row under the folders, whose button opens a folder picker.
settings-search-add-folder = Додати теку
settings-search-choose = Вибрати…
settings-search-add-folder-detail = Пошук переглядає кожну теку й усе, що в ній
# $folder is the folder just chosen, named as the list names it.
settings-search-folder-duplicate = { $folder } уже в списку
# $folder is the folder just chosen; $parent is the listed folder it is in,
# such as Home.
settings-search-folder-covered = { $folder } уже входить у пошук як частина { $parent }
# Under Folders when the list is empty.
settings-search-folders-empty = Нічого не індексується, тож пошук не знайде файлів
# The title of the folder picker.
settings-search-choose-folder-title = Виберіть теку для пошуку
settings-search-picker-failed = Не вдалося відкрити вибір теки
# Leaves out folders that hold a .git folder, which is to say code.
settings-search-skip-repos = Пропускати репозиторії коду
settings-search-skip-repos-detail = Не індексує теки, в яких є тека .git
settings-search-removable = Шукати на знімних дисках
settings-search-removable-detail = Індексує флешки та інші диски, поки вони під'єднані
settings-search-reindex = Переіндексувати домашню теку
# The button on the Re-index Home row.
settings-search-reindex-button = Переіндексувати
settings-search-reindex-detail = Коли результати здаються застарілими. Усе в домашній теці буде перевірено знову.
settings-search-reindex-asked = Індексатор знову перевірить домашню теку. Це може зайняти час.
settings-search-reindex-failed = Не вдалося зв'язатися з індексатором. Він запущений?


## Settings — Agents
##
## The agents Ask sends requests to, read from agents.toml. Changes wait for
## Apply, which saves the file and restarts the agent service.

settings-agents-intro = Агенти, яким Ask може передати запит. Дозволи визначають, що відбувається, коли агент хоче щось зробити, а його папка — це все, що він може читати.
settings-agents-default = Типовий агент
settings-agents-default-detail = Кому йде запит, якщо агента не вибрано
# The row holding the Revert and Apply buttons.
settings-agents-changes = Зміни
settings-agents-revert = Скасувати
settings-agents-apply = Застосувати
settings-agents-saved = Усе збережено
settings-agents-unsaved = Ще не збережено. «Застосувати» записує agents.toml і перезапускає службу агентів.
# $error is the reason, as the system gives it.
settings-agents-failed = Не вдалося зберегти: { $error }
# The otto-agents service, which runs the agents; the row shows whether it is
# up, with Start or Restart.
settings-agents-service = Служба агентів
settings-agents-service-checking = Перевірка…
settings-agents-service-running = Працює
settings-agents-service-stopped = Зупинена. Ask не може зв'язатися з жодним агентом, доки вона не запуститься.
settings-agents-service-failed = Зупинена після помилки. Причину покаже journalctl --user -u otto-agents.
settings-agents-service-missing = Не встановлена
# The system has no systemctl, so the app can't tell whether the service runs.
settings-agents-service-unmanaged = Невідомо: у цій системі немає systemctl
settings-agents-start = Запустити
settings-agents-restart = Перезапустити
# The row that opens agents.toml. Its path is shown under it.
settings-agents-file = Файл конфігурації
settings-agents-none = Агентів не налаштовано
settings-agents-none-detail = Додай їх у { $path }. Як це зробити, описано в посібнику з Ask і агентів.
# Titles an agent's section. $id is how agents.toml and the agent's sessions
# know it, such as claude.
settings-agent-id = id: { $id }
# The field that replaces that row while the agent is being renamed.
settings-agent-name = Ім'я
settings-agents-rename = Перейменувати
# The row under the list of agents, holding its Add button.
settings-agents-add = Новий агент
# What an agent is called until it is renamed.
settings-agents-new-name = Новий агент
# The program an agent runs on: Claude Code, Codex, OpenCode and so on.
settings-agent-harness = Програма
# A harness Settings doesn't recognise, set up by its command alone.
settings-agent-harness-custom = Інша
settings-agent-command = Команда
# Which agent file the agent runs as: who it is and how it answers.
settings-agent-instructions = Інструкції
# No instructions from Otto: the harness runs as itself, Claude Code as
# Claude Code and so on.
settings-agent-instructions-default = Типові
settings-agent-instructions-detail = Файли агентів лежать у ~/.local/share/otto/plugins/<plugin>/agents/
# $name is the agent file the agent names, which wasn't found.
settings-agent-instructions-missing = Немає файлу агента з назвою { $name }. Поклади його в ~/.local/share/otto/plugins/<plugin>/agents/
# $name is the agent file; Hermes runs it in a profile of that name, which
# Hermes has to make.
settings-agent-instructions-hermes = Для цього Hermes потрібен профіль: виконай hermes profile create { $name }
# The row that opens the agent file. Its path is shown under it.
settings-agent-instructions-file = Файл інструкцій
settings-agent-permissions = Дозволи
settings-agent-permissions-deny = Завжди відмовляти
settings-agent-permissions-ask = Питати мене
settings-agent-permissions-allow = Завжди дозволяти
settings-agent-model = Модель
settings-agent-folder = Папка
# Under an agent's Folder when none is set. $path is Ask's scratch folder,
# such as ~/.local/state/otto/ask.
settings-agent-folder-unset = Не задано: сеанси починаються в { $path }, тимчасовій папці
settings-agent-folder-detail = Агент може читати все, що в ній є
# The tint of an agent's cards in Ask. None keeps the plain material.
settings-agent-colour = Колір
settings-agent-colour-none = Немає
settings-agent-colour-red = Червоний
settings-agent-colour-orange = Помаранчевий
settings-agent-colour-amber = Бурштиновий
settings-agent-colour-yellow = Жовтий
settings-agent-colour-lime = Лаймовий
settings-agent-colour-green = Зелений
settings-agent-colour-teal = Бірюзовий
settings-agent-colour-cyan = Блакитний
settings-agent-colour-blue = Синій
settings-agent-colour-indigo = Індиго
settings-agent-colour-violet = Фіолетовий
settings-agent-colour-magenta = Пурпуровий


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
settings-choice-widget-none = Немає
settings-choice-widget-calendar = Календар
# The next two name pages whose text is in English, so they stay as written.
settings-choice-widget-cross-pad = Cross pad
settings-choice-widget-grid-pad = Grid pad
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
files-open-with = Відкрити за допомогою…
files-open-with-window-title = Відкрити за допомогою
files-open-with-count =
    { $count ->
        [one] { $count } файл
        [few] { $count } файли
        [many] { $count } файлів
       *[other] { $count } файлу
    }
files-open-with-mixed = Файли різних типів
files-open-with-search = Пошук застосунків
files-open-with-default = Типовий
files-open-with-other-apps = Інші застосунки
files-open-with-no-match = Немає відповідних застосунків
files-open-with-always = Завжди використовувати для типу «{ $kind }»
files-open-with-not-remembered = Відкрито, але вибір не збережено: { $error }
files-new-folder = Нова папка
files-desk-open-in-files = Відкрити у Файлах
files-desk-edit-done = Готово
# The desk's overflow tile: the caption under it, and what a screen reader
# says for it. $count is how many items it holds.
files-desk-overflow-caption = Ще
files-desk-overflow =
    { $count ->
        [one] Ще { $count } об’єкт
        [few] Ще { $count } об’єкти
        [many] Ще { $count } об’єктів
       *[other] Ще { $count } об’єкта
    }
files-move-to-trash = Перемістити в кошик
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Перемістити { $count } елемент до кошика
        [few] Перемістити { $count } елементи до кошика
        [many] Перемістити { $count } елементів до кошика
       *[other] Перемістити { $count } елемента до кошика
    }
files-put-back = Повернути на місце
files-empty-trash = Очистити кошик
files-delete-immediately = Видалити негайно
# $count завжди два або більше; для одного елемента використовується files-delete-immediately.
files-delete-count-immediately =
    { $count ->
        [one] Видалити { $count } елемент негайно
        [few] Видалити { $count } елементи негайно
        [many] Видалити { $count } елементів негайно
       *[other] Видалити { $count } елемента негайно
    }


## Files — sidebar and columns

files-places = Місця
files-recent = Нещодавні
files-home = Домівка
files-desktop = Стільниця
files-documents = Документи
files-downloads = Завантаження
files-music = Музика
files-pictures = Зображення
files-videos = Відео
files-trash = Кошик

# Заголовки днів у списку «Нещодавні», над файлами, збереженими кожного з
# них.
files-recent-today = Сьогодні
files-recent-yesterday = Учора
files-recent-this-week = Раніше цього тижня
files-recent-this-month = Раніше цього місяця
files-recent-earlier = Раніше
# Показується, коли команда, якій потрібна папка, — перейменування, відкриття,
# переміщення в кошик, перегляд — застосовується до списку, у якого папки немає:
# «Нещодавні» або результати пошуку.
files-synthetic-no-action = У цього списку немає папки.
files-recent-grid-only = «Нещодавні» показуються сіткою.
files-recent-not-a-folder = «Нещодавні» — це список, а не папка.
files-recent-no-location = У «Нещодавніх» немає розташування, до якого можна перейти.

# Смуга фільтра між заголовком вікна і списком і те, що вона повідомляє.
files-search-placeholder = Фільтр: { $folder }
files-search-scope-folder = Ця папка
files-search-scope-everywhere = Всюди
files-search-found =
    { $count ->
        [one] { $count } результат
        [few] { $count } результати
        [many] { $count } результатів
       *[other] { $count } результату
    }
files-search-none = Нічого не знайдено
files-search-no-columns = У результатів немає стовпців для показу.
# Показується замість числа результатів, коли індексування файлів у системі не
# працює. Пошук і «Нещодавні» звертаються до нього, тож без нього жоден з них
# не може відповісти — а порожній список читався б як «такого файла немає», а
# не як «нічим було шукати».
files-search-unavailable = Індексування файлів вимкнено
files-search-indexing = Індексування ще триває ({ $percent }%), результати можуть бути неповними
files-search-indexing-paused = Індексування призупинено, результати можуть бути неповними

files-preview-dimensions = { $width } × { $height }
files-preview-animation = { $width } × { $height } · { $duration }

files-column-name = Назва
files-column-size = Розмір
files-column-kind = Тип
files-column-date-modified = Дата зміни
# Заголовок стовпця «Тип» у вікні кошика, де важливіше, звідки файл потрапив
# сюди, ніж якого він типу.
files-column-original-location = Початкове розташування


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Папка
attachments-screen-region = Область екрана
files-add-to-stash = Додати до добірки
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
files-task-copying = Копіювання { $done } з { $total }
files-task-moving = Переміщення { $done } з { $total }
# The status bar's fuller line: $name is the file being handled right now.
files-task-progress = { $name } — { $done } з { $total }
files-task-already-running = По одній дії за раз — ця ще триває
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
files-open-app-broken = команда запуску застосунку пошкоджена
files-open-no-app = немає встановленого застосунку, що відкриває файли цього типу
files-new-window-failed = Не вдалося відкрити нове вікно: { $error }
files-settings-open-failed = Не вдалося відкрити Налаштування: { $error }


## Files — the listing

files-folder-empty = Ця папка порожня.
files-trash-empty = Кошик порожній.
# Відкрити файл із кошика означає запустити програму над тим, що вже викинуто;
# натомість пропонуємо спершу повернути його на місце.
files-trash-cant-open = Елементи в кошику не можна відкривати. Спершу поверніть їх на місце.
files-trash-cant-rename = Елементи в кошику не можна перейменовувати.
files-folder-denied = Немає прав на перегляд вмісту цієї папки.
files-folder-gone = Цієї папки більше не існує.
files-folder-open-failed = Не вдалося відкрити цю папку: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Розташування
files-info-kind = Тип
files-info-text = Текст
files-info-text-reading = Розпізнається…
files-info-text-words =
    { $count ->
        [one] { $count } слово
        [few] { $count } слова
       *[many] { $count } слів
    }
files-info-text-none = Тексту немає
files-info-text-unread = Ще не розпізнано
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
files-delete-forever-one = Видалити «{ $name }» назавжди?
files-delete-forever-many = Видалити { $count } елементів назавжди?
files-delete-forever-detail = Цю дію неможливо скасувати.
files-empty-trash-confirm = Очистити кошик?
files-empty-trash-detail =
    { $count ->
        [one] { $count } елемент буде видалено назавжди. Цю дію неможливо скасувати.
        [few] { $count } елементи буде видалено назавжди. Цю дію неможливо скасувати.
        [many] { $count } елементів буде видалено назавжди. Цю дію неможливо скасувати.
       *[other] { $count } елемента буде видалено назавжди. Цю дію неможливо скасувати.
    }


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

size-bytes =
    { $count ->
        [one] { $count } байт
        [few] { $count } байти
        [many] { $count } байтів
       *[other] { $count } байта
    }
size-kb = { $value } КБ
size-mb = { $value } МБ
size-gb = { $value } ГБ
size-tb = { $value } ТБ


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

## Files — commands

files-new-folder-with-selection = Нова папка з вибраним
# $count завжди два або більше; для одного елемента використовується
# files-new-folder-with-selection.
files-new-folder-with-count = Нова папка з { $count } елементами


## Files — command palette

# Панель, що відкривається через Ctrl+P: кілька літер назви команди — і вона
# виконується.
files-palette-placeholder = Виконати команду
files-palette-no-matches = Немає відповідних команд
# $count — скільки команд пропонує палітра; оголошується під час відкриття.
files-palette-opened =
    { $count ->
        [one] Палітра команд, { $count } команда
        [few] Палітра команд, { $count } команди
        [many] Палітра команд, { $count } команд
       *[other] Палітра команд, { $count } команди
    }

files-command-group-go = Перехід
files-command-group-file = Файл
files-command-group-edit = Редагування
files-command-group-view = Вигляд

files-command-back = Назад
files-command-forward = Вперед
files-command-up = Вгору
files-command-go-to-path = Перейти до шляху
files-command-go-to-place = Перейти до місця
files-command-undo = Скасувати
files-command-select-all = Вибрати все
files-command-select-matching = Вибрати за шаблоном
files-command-move-to = Перемістити до папки
files-command-change-view = Змінити вигляд
files-command-sort-by = Сортувати за
files-command-show-hidden = Показати приховані файли
files-command-hide-hidden = Сховати приховані файли
files-command-quick-look = Швидкий перегляд
files-command-search = Пошук
files-command-recognise-text = Розпізнати текст
files-recognise-no-pictures = Тут немає чого розпізнавати

# Нередагований префікс, який поле палітри носить, поки вводиться аргумент.
# Після нього додаються двокрапка і пробіл.
files-command-go-to-path-prompt = Перейти до шляху
files-command-go-to-place-prompt = Перейти до місця
files-command-rename-prompt = Перейменувати на
files-command-new-folder-prompt = Нова папка з назвою
files-command-change-view-prompt = Вигляд
files-command-sort-by-prompt = Сортувати за
files-command-search-prompt = Шукати
files-command-select-matching-prompt = Вибрати за шаблоном
files-command-move-to-prompt = Перемістити до
files-command-rename-many-prompt = Перейменувати на

# Назва того, що команда просить, одним словом — показується приглушено після
# її заголовка у списку.
files-command-arg-path = шлях
files-command-arg-place = місце
files-command-arg-name = назва
files-command-arg-view = вигляд
files-command-arg-sort = ключ
files-command-arg-query = текст
files-command-arg-pattern = шаблон

files-view-list = Список
files-view-grid = Сітка
files-view-columns = Стовпці
# Відмова, коли назва не могла б належати файлу — порожня або з похилою рискою.
files-name-invalid = Файл не може мати таку назву
files-no-pattern = Введіть шаблон, наприклад *.png
files-nothing-matches = Нічого не відповідає «{ $pattern }»

# Перейменування вибраного за одним шаблоном — сам шаблон див. у `rename.rs`.
files-rename-many =
    { $count ->
        [one] Перейменувати { $count } елемент
        [few] Перейменувати { $count } елементи
        [many] Перейменувати { $count } елементів
       *[other] Перейменувати { $count } елемента
    }
# Підсумковий рядок пробного запуску під рядками палітри.
files-rename-preview = Перейменує { $count } з { $total }
files-rename-preview-unchanged = Нічого не зміниться
files-rename-conflicts = { $count ->
    [one] Одна назва вже зайнята
    [few] { $count } назви вже зайняті
    [many] { $count } назв вже зайняті
   *[other] { $count } назви вже зайняті
}
files-rename-invalid = { $count ->
    [one] Одна назва була б порожня
    [few] { $count } назви були б порожні
    [many] { $count } назв були б порожні
   *[other] { $count } назви були б порожні
}
files-renamed-count =
    { $count ->
        [one] Перейменовано { $count } елемент
        [few] Перейменовано { $count } елементи
        [many] Перейменовано { $count } елементів
       *[other] Перейменовано { $count } елемента
    }

# Скрипти в ~/.config/otto/files-scripts/; див. components/otto-files/src/scripts.rs.
files-script-running = Виконується { $title }…
files-script-failed-quietly = Скрипт завершився без пояснення причини
files-script-timed-out = Скрипт виконувався надто довго
files-nothing-selected = Нічого не вибрано
files-cant-move-into-itself = Папку не можна перемістити в себе саму
# $count — скільки записів вибрав шаблон.
files-selected-count =
    { $count ->
        [one] Вибрано { $count } елемент
        [few] Вибрано { $count } елементи
        [many] Вибрано { $count } елементів
       *[other] Вибрано { $count } елемента
    }
files-name-taken = «{ $name }» тут уже є


## Files — status

files-undo-new-folder-with-selection = створення папки з вибраним


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %A %-d %B %H:%M

## Top bar — battery

bar-battery-percent = Батарея { $percent } %
bar-battery-remaining = Батарея { $percent } % — залишилося { $time }
bar-battery-charging-time = Батарея { $percent } % — до повного заряду { $time }
bar-battery-full = Батарея { $percent } % — повністю заряджена
bar-battery-charging = Батарея { $percent } % — заряджається
bar-battery-plugged = Батарея { $percent } % — підключена, не заряджається
bar-cpu-frequency = ЦП: у середньому { $avg } ГГц, пік { $max } ГГц
bar-cpu-governor = Регулятор: { $governor }
bar-power-saver = Енергозбереження
bar-power-balanced = Збалансований
bar-power-performance = Продуктивність
bar-power-settings = Налаштування живлення…
bar-keyboard-settings = Налаштування клавіатури…
bar-keyboard-layout-label = Розкладка клавіатури: { $layout }
bar-otto-menu = Otto
bar-otto-about = Про Otto
bar-otto-settings = Налаштування…
bar-otto-log-out = Вийти
# The menu under the focused application's name in the top bar.
bar-app-minimize = Згорнути
bar-app-quit = Завершити { $app }
bar-logout-title = Вийти зараз?
bar-logout-body = Спершу застосунки отримають запит на закриття, щоб незбережене можна було зберегти.


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
schema-frosting-label = Матове скло
schema-frosting-description = Напівпрозорий розмитий матеріал за доком, верхньою панеллю, лаунчером і панелями стільниці.
schema-window-controls-side-label = Кнопки вікна
schema-window-controls-side-description = Біля якого краю смуги заголовка розташовані кнопки закриття, згортання та масштабування.
schema-show-maximize-button-label = Кнопка розгортання
schema-show-maximize-button-description = Показувати кнопку масштабування в заголовку вікна. Типово вимкнено: подвійне клацання на заголовку однаково розгортає вікно.
schema-font-family-label = Шрифт інтерфейсу
schema-font-family-description = Гарнітура шрифту, яку використовує власний інтерфейс Otto.
schema-desk-enabled-label = Показувати файли на робочому столі
schema-desk-enabled-description = Файли з папки «Стільниця», позаду вікон.
schema-canvas-width-label = Ширина бічного полотна
schema-canvas-width-description = Ширина бічного полотна в логічних точках. Усе, що на ньому є, малюється такої ширини.
schema-desktop-widget-label = Фоновий віджет
schema-desktop-widget-description = Сторінка на весь екран поверх шпалер, позаду вікон. Потрібен ewwii.
schema-topbar-show-clock-label = Показувати дату й час
schema-topbar-show-clock-description = Годинник біля правого краю верхньої панелі.
schema-topbar-clock-format-label = Формат годинника
schema-topbar-clock-format-description = Як верхня панель пише дату й час, у форматі strftime. Порожньо — за мовою.
schema-topbar-show-app-menu-label = Показувати меню програм
schema-topbar-show-app-menu-description = Меню активної програми поруч із її назвою на верхній панелі.
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

# --- tiling ---
schema-tiling-decoration-label = Оформлення мозаїчних вікон
schema-tiling-decoration-description = Скільки оформлення зберігає вікно при мозаїчному розміщенні: смуга заввишки в один рядок тексту, із заголовком і кнопкою закриття, або жодної смуги, а активне вікно позначене тонкою рамкою.
schema-tiling-inner-gap-label = Відступ між вікнами
schema-tiling-inner-gap-description = Логічні пікселі між двома сусідніми вікнами.
schema-tiling-outer-gap-label = Відступ навколо вікон
schema-tiling-outer-gap-description = Логічні пікселі між вікнами та краєм екрана.
schema-tiling-smart-gaps-label = Без відступів для одного вікна
schema-tiling-smart-gaps-description = Робочий простір з єдиним вікном не залишає відступів, щоб одне вікно не виглядало без причини вдавленим.
schema-tiling-resize-step-label = Крок зміни розміру
schema-tiling-resize-step-description = Наскільки змінюється контейнер за один крок з клавіатури — часткою його ширини або висоти.
schema-tiling-layout-duration-label = Анімація розташування
schema-tiling-layout-duration-description = Секунди, які триває зміна розташування: вікно входить у дерево або залишає його, переміщення, обмін, вирівнювання. Нуль перемикає одразу.
schema-tiling-layout-bounce-label = Відскок розташування
schema-tiling-layout-bounce-description = Наскільки зміна розташування проскакує ціль, перш ніж зупинитися. Нуль зупиняється без відскоку.
schema-tiling-mode-duration-label = Анімація мозаїки
schema-tiling-mode-duration-description = Секунди, за які робочий простір перебудовується при вмиканні або вимиканні мозаїки, коли кожне вікно летить на своє місце. Нуль перемикає одразу.
schema-tiling-mode-bounce-label = Відскок мозаїки
schema-tiling-mode-bounce-description = Наскільки ця перебудова проскакує ціль, перш ніж зупинитися.

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
schema-input-xkb-layout-description = Назви розкладок XKB через кому, перша активна під час запуску. Порожньо — використовується системна розкладка за умовчанням.
schema-input-xkb-variant-label = Варіант клавіатури
schema-input-xkb-variant-description = Назви варіантів XKB через кому, по одному на розкладку. Порожньо — використовується стандартний варіант кожної розкладки.
schema-input-xkb-options-label = Параметри клавіатури
schema-input-xkb-options-description = Рядки параметрів XKB.
schema-input-show-layout-in-bar-label = Показувати розкладку на панелі
schema-input-show-layout-in-bar-description = Показує активну розкладку клавіатури в otto-bar, якщо їх більше однієї.
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
schema-lock-on-suspend-label = Блокувати під час переходу в сон
schema-lock-on-suspend-description = Блокувати екран перед переходом комп’ютера в сон, щоб він прокидався на екрані блокування.

# --- login ---
schema-login-greeter-command-label = Команда вітального екрана
schema-login-greeter-command-description = Вітальний екран, який запускають у режимі входу.
schema-login-greeter-args-label = Аргументи вітального екрана
schema-login-greeter-args-description = Аргументи, які передають вітальному екрану.

# --- search ---
schema-search-folders-label = Індексовані теки
schema-search-folders-description = Теки, які переглядає індекс файлів, з усім вмістом. ~ — ваша домашня тека.
schema-search-skip-code-repositories-label = Пропускати репозиторії коду
schema-search-skip-code-repositories-description = Не індексувати теки, в яких є тека .git.
schema-search-index-removable-drives-label = Шукати на знімних дисках
schema-search-index-removable-drives-description = Індексувати флешки та інші диски, поки вони під'єднані.

# --- rendering ---
schema-rendering-renderer-label = Рушій рендерингу
schema-rendering-renderer-description = GPU API, через який Otto малює в сеансі, запущеному з вітального екрана. Сеанси у вікні завжди використовують OpenGL.

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
# What is typed matched nothing.
launcher-no-results = Немає результатів
# Ask mode: what is typed is a request for an AI agent, not a search.
launcher-search-ask = Запитати агента…
# Ask mode, with an agent picked from the list: the empty field says whose
# request it is. { $agent } is the agent's name.
launcher-search-ask-agent = Запитати @{ $agent }…
# Ask mode, once a request has been sent: the empty field takes the next one.
launcher-search-ask-more = Поставити ще запитання…
# Agents mode: what is typed narrows the list of agent sessions to pick one
# to open.
launcher-search-agents = Пошук сеансів агентів…
# Agents mode: a session in the list. Its title, when it has none yet.
launcher-agents-untitled = Сеанс без назви
# Agents mode: what a session in the list is doing, before its folder.
launcher-agents-idle = Неактивний
launcher-agents-working = Працює
launcher-agents-needs-input = Чекає на відповідь
launcher-agents-error = Помилка
launcher-agents-none = Сеансів агентів ще немає
canvas-sessions-heading = Агенти
stash-drop-invite = Перетягніть файли сюди, щоб додати до добірки
# Ask mode: the files that go with a request, under it in the log, or above
# the field before it is sent. { $files } is their names, comma-separated.
# Ask mode, while an existing session is being opened to continue it.
launcher-ask-opening = Відкриття сеансу…
launcher-ask-loading = Завантаження бесіди…
# The session is going to its terminal window.
launcher-ask-handing-over = Продовження в терміналі…
launcher-ask-handing-over-busy = Після завершення поточної роботи — продовження в терміналі…
# The terminal window's title, as the dock shows it.
launcher-ask-window-title = Ask: @{ $agent }: { $title }
launcher-ask-window-title-agent = Ask: @{ $agent }
launcher-ask-window-title-plain = Ask: { $title }
launcher-ask-window-title-bare = Ask
# Ask mode: the last line of the log above the field, saying what the agent is
# doing now.
launcher-ask-starting = Запуск: { $agent }…
launcher-ask-starting-agent = Запуск агента…
launcher-ask-thinking = Думає…
launcher-ask-writing = Пише…
launcher-ask-running = Виконується: { $tool }…
launcher-ask-working = Працює…
launcher-ask-sending = Надсилання наступного повідомлення…
# The agent asked for permission; its answers are the rows under the field.
launcher-ask-waiting = Чекає на відповідь нижче
# Ask mode: the line under the status naming the agent and the mode it is in.
# The agent is shown as a handle, like "@Claude"; the mode's name follows in a
# pill of its own, and the hint after it, only when the agent has more than one
# mode. Keep the hint's brackets: they hold it apart from the mode.
launcher-ask-agent = @{ $agent }
launcher-ask-mode-hint = (Shift+Tab — перемикання)
# Ask mode: a tool call the agent made, in the log. { $tool } is the command or
# file, as the agent names it.
launcher-ask-step-running = ▸ { $tool }
launcher-ask-step-done = ✓ { $tool }
launcher-ask-step-failed = ✗ { $tool }
launcher-ask-step-denied = Відхилено: { $tool }
# Ask mode: under a request in the log, what became of it.
launcher-ask-queued = У черзі
launcher-ask-cancelled = Скасовано
launcher-ask-failed = Помилка: { $error }
# Ask mode, when the agent service is not running to take a request.
launcher-ask-unreachable = Служба агентів не запущена
# Ask mode: the agent asked the person something (a choice, a value, a link to
# open). The rows under the field are the answers; these are their labels.
launcher-input-yes = Так
launcher-input-no = Ні
launcher-input-continue = Продовжити
launcher-input-skip = Пропустити питання
launcher-input-decline = Не відповідати
launcher-input-open-link = Відкрити посилання
# Sends a request that only asked to open a link.
launcher-input-done = Готово
launcher-input-send = Надіслати відповіді
# Beside an option the agent suggests.
launcher-input-suggested = Рекомендовано
# What the empty field says while it takes the answer.
launcher-input-type-answer = Відповідь…
launcher-input-type-number = Число…
launcher-input-type-other = Або своя відповідь…
# In the log, above a question when the agent asked several.
launcher-input-progress = Питання { $current } з { $total }
# In the log, a question already answered, and its answer.
launcher-input-answer = { $question }: { $answer }
launcher-input-skipped = Пропущено
# In the log, under a request nobody answered the usual way.
launcher-input-declined = Відхилено
launcher-input-dismissed = Закрито
launcher-input-unanswered = Без відповіді
# In the log, under a question, when the answer typed can’t be taken.
launcher-input-error-empty = Потрібна відповідь
launcher-input-error-number = Потрібне число
launcher-input-error-integer = Потрібне ціле число
launcher-input-error-min = Має бути не менше { $min }
launcher-input-error-max = Має бути не більше { $max }
launcher-input-error-short = { $min ->
        [one] Потрібно щонайменше { $min } символ
        [few] Потрібно щонайменше { $min } символи
        [many] Потрібно щонайменше { $min } символів
       *[other] Потрібно щонайменше { $min } символу
    }
launcher-input-error-long = { $max ->
        [one] Не більше { $max } символу
        [few] Не більше { $max } символів
        [many] Не більше { $max } символів
       *[other] Не більше { $max } символу
    }
launcher-input-error-pick-min = Треба вибрати щонайменше { $min }
launcher-input-error-pick-max = Можна вибрати не більше { $max }

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = Застосунок
launcher-badge-window = Вікно
launcher-badge-calc = Калькулятор


## Agent service

## The dialog otto-agents puts up when an agent asks to use a tool. The
## sentence is composed by the service and drawn by the islands.

# { $agent } is the agent's name; { $action } one of the phrases below.
agents-permission-title = { $agent } хоче { $action }
# What the tool does, by kind. Each completes "{ $agent } wants to …".
agents-permission-read = прочитати файл
agents-permission-edit = змінити файл
agents-permission-delete = вилучити файл
agents-permission-move = перемістити файл
agents-permission-search = виконати пошук
agents-permission-execute = виконати команду
agents-permission-fetch = отримати дані з інтернету
agents-permission-switch-mode = змінити принцип роботи
agents-permission-tool = використати інструмент
# The dialog's body: the session's folder, as ~/… when it is under home.
agents-permission-in-folder = в { $folder }
# The dialog's own words for the answer buttons, chosen by the option's kind
# rather than the agent's label, which the agent could word misleadingly.
agents-permission-allow = Дозволити
agents-permission-allow-always = Завжди дозволяти
agents-permission-reject = Відхилити
agents-permission-reject-always = Ніколи не дозволяти
# The button that hands the question to the Ask window instead.
agents-permission-open-in-ask = Відкрити в Ask


## Emoji picker

# What the search field says when empty.
emoji-search = Пошук емодзі…
# Shown in the grid when nothing matches what was typed.
emoji-no-results = Емодзі не знайдено
# The section and tab titles. These are Unicode's own category names, and
# the CLDR translations of them are the reference where one exists.
emoji-group-recent = Нещодавні
emoji-group-smileys = Смайлики та емоції
emoji-group-people = Люди й тіло
emoji-group-nature = Тварини і природа
emoji-group-food = Їжа та напої
emoji-group-travel = Подорожі та місця
emoji-group-activities = Заняття
emoji-group-objects = Предмети
emoji-group-symbols = Символи
emoji-group-flags = Прапори


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


## Peek — card labels
##
## Fact keys: the left-hand column of a card's detail list. One or two words,
## drawn in a narrow column — keep them short. Title case in English.

# Column heading for the file's type, e.g. "JPEG", "PDF". Max ~12 characters.
peek-fact-kind = Тип
# Column heading for the file's size on disk. Max ~12 characters.
peek-fact-size = Розмір
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
peek-fact-dimensions = Розміри
# Column heading for a video's or an audio track's running time. Max ~12 characters.
peek-fact-duration = Тривалість
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
peek-fact-pixels = Пікселі
# Column heading for a PDF's page count. Max ~12 characters.
peek-fact-pages = Сторінки
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
peek-fact-title = Назва
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
peek-fact-artist = Виконавець
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
peek-fact-album = Альбом
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
peek-fact-year = Рік

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
peek-empty-file = Порожній файл
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
peek-image-too-large = Завелике для перегляду
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
peek-megapixels =
    { $count ->
        [one] { $count } мегапіксель
        [few] { $count } мегапікселі
        [many] { $count } мегапікселів
       *[other] { $count } мегапікселя
    }
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
peek-pdf-install-rasteriser = Щоб побачити сторінки, встановіть один із пакунків: { $packages }
# Which page of a PDF the panel is showing, in the corner of its title strip.
# $page and $pages are whole numbers. Very little room — keep it to a few
# characters, and drop the word for "page" if the language can.
peek-page-of = { $page } / { $pages }


## Peek — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
peek-empty-folder = Порожня папка
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
peek-item-count =
    { $count ->
        [one] { $count } елемент
        [few] { $count } елементи
        [many] { $count } елементів
       *[other] { $count } елемента
    }
# Summary line for an archive, joining the entry count to the archive's own
# size on disk. $items is peek-item-count, $size is a formatted byte
# count. The dash is an em dash.
peek-archive-summary = { $items } — { $size }


## Peek — nothing to show
##
## Each of these fills the card in place of a preview, so a person reads it
## instead of seeing the file. They state what happened and stop. Lower case,
## no full stop: they are shown as a sentence fragment.
##
## $error is an operating-system message, which arrives in whatever language
## the system libraries produce and is usually English. Keep it at the end.

# The file is a pipe, socket or device — opening it could block forever.
peek-error-not-previewable = цей файл неможливо переглянути
# The file's metadata could not be read.
peek-error-stat-file = не вдалося отримати відомості про файл: { $error }
# The file's bytes could not be read. Also used by the text previewer.
peek-error-read-file = не вдалося прочитати файл: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
peek-error-not-seekable = у файлі неможливе переміщення
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
peek-error-sandbox = не вдалося ізолювати програму перегляду: { $error }

# Image previewer.
peek-error-read-image = не вдалося прочитати зображення: { $error }
# The bytes are an image format this build has no decoder for.
peek-error-image-unsupported = ця збірка не підтримує такий формат зображення
peek-error-image-no-size = зображення не повідомляє свій розмір
peek-error-image-decode = зображення не декодовано: { $error }
peek-error-image-readback = не вдалося прочитати декодоване зображення

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
peek-error-read-drawing = не вдалося прочитати рисунок: { $error }
peek-error-drawing-parse = не вдалося розібрати рисунок
peek-error-drawing-surface = немає поверхні для його відтворення
peek-error-drawing-readback = не вдалося прочитати готовий рисунок

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
peek-error-not-text = це не текст у жодному з кодувань, які читає Otto

# PDF previewer.
peek-error-read-document = не вдалося прочитати документ: { $error }
peek-error-page-readback = не вдалося прочитати відтворену сторінку

# Folder listing.
peek-error-read-folder = не вдалося прочитати папку

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

peek-error-previewer-missing = не вдалося знайти програму перегляду: { $error }
peek-error-previewer-start = не вдалося запустити програму перегляду: { $error }
peek-error-previewer-no-output = програма перегляду нічого не видала
peek-error-previewer-unreadable = програма перегляду видала щось нечитабельне
peek-error-previewer-failed = збій програми перегляду: { $error }
# The worker was still going after the deadline and was killed.
peek-error-timeout = перегляд цього файлу тривав задовго

## Islands
##
## The dynamic island: the small dark bubble at the top of the screen that
## grows into a notification card, and the permission dialogs the portal
## raises through it. Space is very tight — a card is about 320 px wide and
## 64 px tall, drawn at 9–13 px.


## Islands — notification card

islands-music-elsewhere = Відтворюється на іншому пристрої

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


## Islands — agents' questions
##
## An agent (Claude, say) asking the person something: one question a page,
## with the options as rows beneath it. The dialog owns these words, not the
## agent — only the question and its options come from the agent itself.
## Buttons sit side by side and narrow: one word each.

# Sends the answers back to the agent, on the last (or only) question.
islands-dialog-answer = Відповісти
# Leaves every question unanswered and lets the agent carry on without them.
islands-dialog-skip = Пропустити
# Goes on to the next question, keeping what has been picked so far. Shown in
# the place of "Answer" until the last question.
islands-dialog-next = Далі
# Goes back to the question before. A small button at the top-left corner; the
# chevron pointing back is drawn, so the word alone belongs here.
islands-dialog-back = Назад
# Which question of how many this is: a caption on its own line between the
# options and the buttons, shown whenever there is more than one question.
islands-dialog-page = { $current } з { $total }
# Under a question that takes any number of answers, where the rows are
# toggles rather than a single choice. No full stop: it is a hint, not a
# sentence of instructions.
islands-dialog-multi-hint = Виберіть усі відповідні


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
a11y-search-settings = Пошук налаштувань
a11y-results = Результати
a11y-settings = Налаштування
a11y-preview = Перегляд
a11y-preview-page = Перегляд, сторінка { $page } з { $pages }
a11y-preview-pages = Перегляд, { $pages } сторінок
a11y-preview-shortened = Перегляд, скорочений

## The Photos view

files-view-photos = Фото
files-photos-day = { $weekday }, { $day } { $month }
files-photos-day-year = { $weekday }, { $day } { $month } { $year }
files-photos-undated = Без дати
files-photos-other = Інші файли
files-photos-summary = { $images }, { $folders }
files-photos-images =
    { $count ->
        [one] { $count } зображення
        [few] { $count } зображення
        [many] { $count } зображень
       *[other] { $count } зображення
    }
files-photos-folders =
    { $count ->
        [one] { $count } тека
        [few] { $count } теки
        [many] { $count } тек
       *[other] { $count } теки
    }

files-weekday-sun = Неділя
files-weekday-mon = Понеділок
files-weekday-tue = Вівторок
files-weekday-wed = Середа
files-weekday-thu = Четвер
files-weekday-fri = Пʼятниця
files-weekday-sat = Субота

files-month-long-jan = січня
files-month-long-feb = лютого
files-month-long-mar = березня
files-month-long-apr = квітня
files-month-long-may = травня
files-month-long-jun = червня
files-month-long-jul = липня
files-month-long-aug = серпня
files-month-long-sep = вересня
files-month-long-oct = жовтня
files-month-long-nov = листопада
files-month-long-dec = грудня
files-photos-month =
    { $number ->
        [1] Січень { $year }
        [2] Лютий { $year }
        [3] Березень { $year }
        [4] Квітень { $year }
        [5] Травень { $year }
        [6] Червень { $year }
        [7] Липень { $year }
        [8] Серпень { $year }
        [9] Вересень { $year }
        [10] Жовтень { $year }
        [11] Листопад { $year }
        [12] Грудень { $year }
       *[other] { $month } { $year }
    }
files-photos-folders-title = Теки
files-photos-group-day = За днями
files-photos-group-month = За місяцями
files-photos-group-none = Без групування
files-photos-info-kind = Зображення { $format }
files-photos-info-copied = Скопійовано
files-photos-info-dimensions = Розміри
files-photos-info-modified = Змінено
files-photos-info-where = Де
files-photos-info-many =
    { $count ->
        [one] { $count } об’єкт
        [few] { $count } об’єкти
        [many] { $count } об’єктів
       *[other] { $count } об’єкта
    }
files-photos-one-selected = 1 вибрано · Пробіл — перегляд · ↵ — відкрити


## otto-authorize — the panel that asks for the password before a sensitive setting changes.
## The reason line is composed by Otto from the setting and the value asked for; { $value } is the program or options, quoted.

authorize-cancel = Скасувати
authorize-error-failed = Не вдалося пройти автентифікацію
authorize-path-in = { $name } (у { $dir })
polkit-unknown-program = Невідома програма

## Settings › Privacy: what apps were allowed, read from xdg-permission-store.

privacy-applies-to-unsandboxed = стосується всіх програм поза пісочницею
privacy-app-unsandboxed = Програми поза пісочницею
privacy-decision-allow = Дозволяти
privacy-decision-ask = Питати
privacy-decision-deny = Не дозволяти
privacy-forget = Забути
privacy-group-notifications = Сповіщення
privacy-group-screen = Доступ до екрана
privacy-notifications-none = Жодна програма ще не просила надсилати сповіщення
privacy-reading = Читання…
privacy-remembered-by = запам’ятано в { $desktop }
privacy-remote-desktop = Керує мишею й клавіатурою та бачить екран
privacy-reset = Скинути
privacy-screencast = Записує екран
privacy-screencast-monitor = Записує екран { $screen }
privacy-screencast-window = Записує вікно
privacy-screen-none = Жодній програмі не запам’ятано дозвіл на доступ до екрана
privacy-screenshot = Робить знімки екрана
privacy-screenshot-allowed = Робить знімки екрана без запиту
privacy-screenshot-denied = Не може робити знімки екрана
privacy-store-unavailable = Не вдається прочитати, що дозволено програмам
privacy-store-unavailable-detail = Сховище дозволів (xdg-permission-store з xdg-desktop-portal) недоступне
settings-pane-privacy = Конфіденційність

screencast-picker-remember = Запам’ятати для { $app }

## Users pane

settings-pane-account = Користувачі
settings-account-picture = Зображення
settings-account-picture-detail = Показується на екранах входу та блокування
settings-account-choose-picture = Вибрати зображення
settings-account-picture-unreadable = Otto не може прочитати цей файл як зображення
settings-account-full-name = Повне ім’я
settings-account-name = Назва облікового запису
settings-account-type = Тип облікового запису
settings-account-type-administrator = Адміністратор
settings-account-type-standard = Звичайний
settings-account-no-accountsservice = Тут це змінити не можна: AccountsService не запущено
settings-account-not-permitted = Система не дозволила цю зміну
settings-group-password = Пароль
settings-account-current-password = Поточний пароль
settings-account-new-password = Новий пароль
settings-account-confirm-password = Підтвердьте новий пароль
settings-account-change-password = Змінити пароль
settings-account-change-password-ellipsis = Змінити пароль…
settings-account-password-detail = Потрібен для входу, розблокування екрана та підтвердження змін
settings-account-password-changing = Зміна пароля…
settings-account-password-changed = Пароль змінено
settings-account-password-missing = Введіть поточний пароль і новий
settings-account-password-mismatch = Нові паролі не збігаються
settings-account-password-same = Новий пароль такий самий, як поточний
settings-account-password-wrong-current = Поточний пароль неправильний
settings-account-password-failed = Не вдалося змінити пароль
settings-account-reset-password-ellipsis = Скинути пароль…
settings-account-reset-detail = Задати новий пароль для цього облікового запису
settings-account-working = Очікування системи…
settings-users-you = { $kind } · Ви
settings-users-reset-title = Скидання пароля: { $name }
settings-users-reset-action = Скинути пароль
settings-users-add-title = Додати користувача
settings-users-add-action = Додати користувача
settings-users-delete-title = Видалити { $name }?
settings-users-delete-body = Цей користувач більше не зможе увійти. Його домашню теку буде збережено.
settings-users-delete-action = Видалити користувача
settings-users-invalid-name = Назва облікового запису починається з малої латинської літери й містить лише a–z, 0–9, - та _
settings-users-name-taken = Обліковий запис із такою назвою вже існує
settings-users-password-missing = Введіть пароль для облікового запису

## Налаштування › Диктування

settings-pane-dictation = Диктування
settings-dictation-intro = Мовлення набирається в текстове поле й розпізнається на цьому комп'ютері. Ctrl+D вмикає диктування в лаунчері; в інших застосунках це робить комбінація клавіш, призначена на otto-dictate toggle.
settings-dictation-engine = Рушій
settings-dictation-engine-detail = Вибір рушія запускає його мовний сервер і зупиняє решту
settings-dictation-engine-parakeet = Parakeet
settings-dictation-engine-whisper = Whisper (лише англійська)
settings-dictation-engine-crispasr = CrispASR
settings-dictation-server = Мовний сервер
settings-dictation-server-checking = Перевірка…
settings-dictation-server-running = Працює
settings-dictation-server-stopped = Зупинений. Диктування нічого не чує, доки він не запуститься.
settings-dictation-server-failed = Зупинений після помилки. Причину покаже journalctl --user -u { $unit }.
settings-dictation-server-missing = Не встановлений. Встановити його можна командою components/otto-dictate/engines/install.sh { $engine }.
settings-dictation-server-unmanaged = Невідомо: у цій системі немає systemctl
settings-dictation-start = Запустити
settings-dictation-restart = Перезапустити
settings-dictation-language = Мова
settings-dictation-language-auto = Автоматично
settings-dictation-language-detail = Мова мовлення. Parakeet і CrispASR визначають її самі.
settings-dictation-hotwords-boost = Підсилення назв
settings-dictation-hotwords-boost-detail = Наскільки перевага надається назвам, яких очікує поле. Понад 6 слова навколо назви починають спотворюватися.
settings-dictation-autostart = Запускати диктування під час входу
settings-dictation-autostart-detail = Запускає otto-dictate з ~/.config/autostart — Otto читає цю теку, коли ввімкнено xdg_autostart
