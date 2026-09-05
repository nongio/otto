# Otto — Simplified Chinese
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

common-open = 打开
common-save = 存储
common-cancel = 取消
common-add = 添加
common-remove = 移除
common-quit = 退出
common-cut = 剪切
common-copy = 拷贝
common-paste = 粘贴
common-rename = 重命名
common-delete = 删除
common-replace = 替换
common-move = 移动


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = 自动隐藏
dock-auto-hide-on = ✓ 自动隐藏
dock-magnification = 放大
dock-magnification-on = ✓ 放大
dock-position-bottom = 底部
dock-position-bottom-on = ✓ 底部
dock-position-left = 左侧
dock-position-left-on = ✓ 左侧
dock-position-right = 右侧
dock-position-right-on = ✓ 右侧

# Shown on an app's icon when the app is not running.
dock-open = 打开
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = 在 Dock 中保留
dock-keep-in-dock-on = ✓ 在 Dock 中保留
dock-quit = 退出


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = 通用
settings-pane-displays = 显示器
settings-pane-dock = Dock
settings-pane-keyboard = 键盘
settings-pane-pointing = 触控板与鼠标
settings-pane-sound = 声音
settings-pane-power = 电源
settings-pane-lock-and-login = 锁定与登录


## Settings — General

settings-group-appearance = 外观
settings-colour-scheme = 配色方案
settings-accent-colour = 强调色
settings-rounded-corners = 圆角
settings-window-controls = 窗口控件
settings-maximize-button = 最大化按钮
settings-maximize-button-detail = 显示缩放圆点；双击标题栏同样可以缩放
settings-font = 系统字体
settings-gtk-theme = GTK 主题

settings-group-desktop = 桌面
settings-background-colour = 背景颜色
settings-background-image = 背景图片
settings-background-image-detail = 通过桌面门户的文件选择器选取

settings-group-pointer-and-icons = 指针与图标
settings-cursor-theme = 光标主题
settings-cursor-size = 光标大小
settings-icon-theme = 图标主题

settings-group-window-switcher = 窗口切换器
settings-follow-cursor = 在指针所在的显示器上显示

settings-group-language = 语言
settings-display-language = 显示语言

settings-group-configuration = 配置
settings-configuration-file = 配置文件
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = 未知 —— 合成器没有响应


## Settings — Displays

settings-display-active = 启用
settings-display-active-detail = 停用的显示器仍保留在排列中的位置
settings-display-primary = 用作主显示器
settings-display-primary-detail = Dock 和菜单栏位于主显示器上
settings-display-x-position = X 位置
settings-display-y-position = Y 位置
settings-display-x-position-detail = 桌面坐标空间中的左上角
settings-display-width = 宽度
settings-display-width-detail = 像素。无头输出可以是任意大小
settings-display-height = 高度
settings-display-refresh = 刷新率
settings-display-refresh-detail = Hertz —— 串流每隔多久获得一帧
settings-display-resolution = 分辨率
settings-display-scale = 显示器缩放
settings-display-scale-detail = 下次登录时生效。桌面不会即时重新排布

# Shown when the compositor reports no outputs at all.
settings-display-none = 没有显示器
settings-display-none-detail = 合成器没有驱动任何输出

settings-virtual-displays = 虚拟显示器
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
       *[other] { $count } 个无头输出，通过 PipeWire 串流。移除会去掉选中的那个
    }


## Settings — Dock

settings-dock-size = 大小
settings-dock-position = 屏幕上的位置
settings-dock-autohide = 自动隐藏
settings-dock-magnification = 放大
settings-group-magnification-and-icons = 放大与图标
settings-dock-magnification-amount = 放大幅度
settings-dock-tint-icons = 图标着色
settings-switcher-colorize-icons = 切换器图标着色
settings-dock-icon-tint = 图标色调
settings-dock-icon-tint-strength = 图标色调强度


## Settings — Keyboard

settings-key-repeat-delay = 按键重复延迟
settings-key-repeat-rate = 按键重复速率
settings-group-input-source = 输入源
settings-xkb-layout = 布局
settings-xkb-variant = 变体
settings-xkb-options = 选项
settings-group-shortcuts = 快捷键
settings-key-combination = 组合键
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl、Alt、Shift 或 Logo 以 + 相连，再加一个键：Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = 触控板
settings-tap-to-click = 轻点来点按
settings-tap-and-drag = 轻点拖移
settings-drag-lock = 拖移锁定
settings-click-method = 点按方式
settings-ignore-while-typing = 打字时忽略
settings-natural-scrolling = 自然滚动
settings-left-handed = 左手模式
settings-middle-click-emulation = 模拟中键点按
settings-group-pointer = 指针
settings-tracking-speed = 跟踪速度
settings-pointer-acceleration = 加速
settings-scrolling-speed = 滚动速度


## Settings — Sound

settings-interface-sounds = 界面声音
settings-sound-theme = 声音主题


## Settings — Power

settings-manage-lid-switch = 处理合盖开关
settings-manage-lid-switch-detail = 合盖时由 Otto 而不是 logind 进入睡眠
settings-on-lid-close = 合上盖子时
settings-on-power-button = 按下电源键时


## Settings — Lock & Login

settings-group-lock = 锁定
settings-lock-after = 多久后锁定
settings-lock-screen = 锁屏程序
settings-lock-screen-detail = 下次锁定屏幕时生效
settings-lock-screen-arguments = 锁屏程序参数
settings-group-login = 登录
settings-greeter = 登录界面
settings-greeter-detail = 下次登录时生效
settings-greeter-arguments = 登录界面参数


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = 浅色
settings-choice-dark = 深色
settings-choice-controls-left = 左侧
settings-choice-controls-right = 右侧
settings-choice-position-bottom = 底部
settings-choice-position-left = 左侧
settings-choice-position-right = 右侧
settings-choice-clickfinger = 用手指点按
settings-choice-buttonareas = 在角落点按
settings-choice-accel-flat = 无加速
settings-choice-accel-adaptive = 自适应
settings-choice-lid-auto = 自动决定
settings-choice-lid-lock = 锁定屏幕
settings-choice-lid-disable-internal = 关闭内建显示器
settings-choice-power-ignore = 不执行任何操作
settings-choice-power-lock = 锁定屏幕
settings-choice-power-suspend = 睡眠
settings-choice-power-shutdown = 关机
# The automatic option for a theme that follows the system.
settings-choice-auto = 自动


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value }%
settings-readout-pixels = { $value } px
settings-readout-milliseconds = { $value } 毫秒
settings-readout-seconds = { $value } 秒
# Key repeats per second.
settings-readout-per-second = { $value } 次/秒


## Files — windows

files-window-title = 文件
# The Get Info panel's own window.
files-info-window-title = 简介


## Files — commands

files-get-info = 显示简介
files-new-folder = 新建文件夹
files-move-to-trash = 移到废纸篓
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
       *[other] 将 { $count } 个项目移到废纸篓
    }
files-put-back = 放回原处
files-empty-trash = 清倒废纸篓
files-delete-immediately = 立即删除
# $count 总是两个或更多；单个项目使用 files-delete-immediately。
files-delete-count-immediately =
    { $count ->
       *[other] 立即删除 { $count } 个项目
    }


## Files — sidebar and columns

files-places = 位置
files-home = 个人文件夹
files-desktop = 桌面
files-documents = 文稿
files-downloads = 下载
files-music = 音乐
files-pictures = 图片
files-videos = 影片
files-trash = 废纸篓

files-column-name = 名称
files-column-size = 大小
files-column-kind = 种类
files-column-date-modified = 修改日期
# 废纸篓窗口中“种类”列的标题，在那里文件从哪里来比它是什么类型更重要。
files-column-original-location = 原始位置


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = 文件夹
files-kind-image = 图像
files-kind-movie = 影片
files-kind-audio = 音频
files-kind-text = 文本
files-kind-document = 文稿
files-kind-archive = 归档
files-kind-application = 应用程序


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = 正在载入…
files-empty = 空
# The idle line: what the folder holds.
files-status-no-items = 没有项目
files-status-items =
    { $count ->
       *[other] { $count } 个项目
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }，{ $hidden } 个隐藏
files-status-selected = 已选择 { $total } 个中的 { $count } 个
files-status-opening-preview = 正在打开预览…
files-nothing-to-undo = 没有可撤销的操作
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = 已撤销{ $label }
files-undo-move = 移动
files-undo-copy = 拷贝
files-undo-delete = 删除
files-undo-rename = 重命名
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = 已重命名为“{ $name }”
files-new-folder-created = 新文件夹“{ $name }”
files-gone = “{ $name }”已不在那里
files-no-such-folder = “{ $path }”不存在
files-rename-failed = 无法重命名：{ $error }
files-new-folder-failed = 无法创建文件夹：{ $error }
files-open-failed = 无法打开该文件：{ $error }
files-new-window-failed = 无法打开新窗口：{ $error }


## Files — the listing

files-folder-empty = 此文件夹是空的。
files-trash-empty = 废纸篓是空的。
# 打开废纸篓里的文件，等于对已经扔掉的东西启动应用程序；这里提供的是先把它
# 放回原处。
files-trash-cant-open = 废纸篓中的项目无法打开。请先放回原处。
files-trash-cant-rename = 废纸篓中的项目无法重命名。
files-folder-denied = 没有查看此文件夹内容的权限。
files-folder-gone = 此文件夹已不存在。
files-folder-open-failed = 无法打开此文件夹：{ $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = 位置
files-info-kind = 种类
files-info-modified = 修改时间
files-info-created = 创建时间
files-info-accessed = 访问时间
files-info-owner = 所有者
files-info-links-to = 链接到
files-info-permissions = 权限
# Column headers over the permission checkboxes — narrower still.
files-perm-read = 读
files-perm-write = 写
files-perm-exec = 执行
# Row labels: who each set of permissions applies to.
files-perm-owner = 所有者
files-perm-group = 群组
files-perm-everyone = 所有人


## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = 打开
files-picker-save-as = 存储为
files-picker-save-files = 存储文件
files-picker-all-files = 所有文件
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = 存储为：

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = 名称不能为空
files-save-name-has-slash = 名称不能包含“/”
files-save-name-reserved = 该名称已被保留
files-save-nowhere = 没有可存储的位置
files-save-permission-denied = 没有在此处存储的权限

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = “{ $name }”已存在。要替换吗？
files-replace-one-detail = 替换会覆盖它现有的内容。
files-replace-many = 其中 { $count } 个文件已存在。要替换吗？
files-replace-many-detail = 替换会覆盖它们现有的内容。
files-delete-forever-one = 要永久删除“{ $name }”吗？
files-delete-forever-many = 要永久删除 { $count } 个项目吗？
files-delete-forever-detail = 此操作无法撤销。
files-empty-trash-confirm = 要清倒废纸篓吗？
files-empty-trash-detail =
    { $count ->
       *[other] { $count } 个项目将被永久删除。此操作无法撤销。
    }


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
       *[other] { $count } 字节
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

files-date-modified = { $year }年{ $month }{ $day }日 { $time }

files-month-jan = 1月
files-month-feb = 2月
files-month-mar = 3月
files-month-apr = 4月
files-month-may = 5月
files-month-jun = 6月
files-month-jul = 7月
files-month-aug = 8月
files-month-sep = 9月
files-month-oct = 10月
files-month-nov = 11月
files-month-dec = 12月


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %-m月%-d日 %A  %H:%M


## Settings — widgets
##
## The controls themselves, rather than the settings they edit.

# Shown in a text field that has no value yet.
settings-not-set = 未设置
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = 选取…
settings-no-file-chosen = 未选取文件
settings-choose-background-image = 选取背景图片

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Otto 设置 —— { $pane }


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
schema-screen-scale-label = 显示器缩放
schema-screen-scale-description = 应用于整个桌面的全局缩放系数。
schema-theme-scheme-label = 配色方案
schema-theme-scheme-description = 浅色或深色配色方案。
schema-accent-color-label = 强调色
schema-accent-color-description = 随浅色与深色方案变化的调色板名称，或 #RRGGBB 颜色值。
schema-rounded-corners-label = 圆角
schema-rounded-corners-description = Dock、顶部菜单栏、窗口装饰以及桌面自身的面板。
schema-window-controls-side-label = 窗口控件
schema-window-controls-side-description = 关闭、最小化和缩放控件位于窗口标题栏的哪一端。
schema-show-maximize-button-label = 最大化按钮
schema-show-maximize-button-description = 在窗口标题栏中显示缩放控件。默认关闭：双击标题栏同样可以缩放窗口。
schema-font-family-label = 界面字体
schema-font-family-description = Otto 自身界面使用的字体族。
schema-background-color-label = 背景颜色
schema-background-color-description = 桌面背景颜色，以十六进制字符串表示。
schema-background-image-label = 背景图片
schema-background-image-description = 桌面背景图片的路径。留空表示不使用。
schema-cursor-theme-label = 光标主题
schema-cursor-theme-description = XCursor 主题的名称。
schema-cursor-size-label = 光标大小
schema-cursor-size-description = 光标大小，以逻辑像素计。
schema-icon-theme-label = 图标主题
schema-icon-theme-description = 图标主题的名称。留空则自动检测。
schema-gtk-theme-label = GTK 主题
schema-gtk-theme-description = 交给客户端的 GTK 主题名称。留空则自动检测。
schema-locales-label = 语言环境
schema-locales-description = 首选的语言环境，最优先的在前。

# --- dock ---
schema-dock-size-label = 大小
schema-dock-size-description = Dock 大小的倍数。
schema-dock-position-label = 屏幕上的位置
schema-dock-position-description = Dock 所处的屏幕边缘。
schema-dock-autohide-label = 自动隐藏
schema-dock-autohide-description = 隐藏 Dock，直到指针到达它所在的屏幕边缘。
schema-dock-magnification-label = 放大
schema-dock-magnification-description = 放大指针下方的图标。
schema-dock-genie-scale-label = 放大幅度
schema-dock-genie-scale-description = 指针下方的图标放大多少。
schema-dock-genie-span-label = 放大范围
schema-dock-genie-span-description = 放大效果波及多少个相邻图标。
schema-dock-colorize-icons-label = 图标着色
schema-dock-colorize-icons-description = 用单一颜色为 Dock 图标着色。
schema-dock-colorize-color-label = 图标色调
schema-dock-colorize-color-description = 用于为 Dock 图标着色的颜色，以十六进制字符串表示。
schema-dock-colorize-intensity-label = 图标色调强度
schema-dock-colorize-intensity-description = 色调施加的强度。

# --- general ---
schema-keyboard-repeat-delay-label = 重复延迟
schema-keyboard-repeat-delay-description = 按住一个键多少毫秒后开始重复。
schema-keyboard-repeat-rate-label = 重复速率
schema-keyboard-repeat-rate-description = 按住一个键时每秒重复的次数。

# --- input ---
schema-input-xkb-layout-label = 键盘布局
schema-input-xkb-layout-description = XKB 布局名称。留空则使用系统默认值。
schema-input-xkb-variant-label = 键盘变体
schema-input-xkb-variant-description = XKB 变体名称。留空则使用系统默认值。
schema-input-xkb-options-label = 键盘选项
schema-input-xkb-options-description = XKB 选项字符串。
schema-input-tap-enabled-label = 轻点来点按
schema-input-tap-enabled-description = 将触控板上的轻点视为点按。
schema-input-tap-drag-enabled-label = 轻点拖移
schema-input-tap-drag-enabled-description = 轻点后按住不放即开始拖移。
schema-input-tap-drag-lock-enabled-label = 拖移锁定
schema-input-tap-drag-lock-enabled-description = 手指短暂抬起时轻点拖移继续保持。
schema-input-touchpad-click-method-label = 点按方式
schema-input-touchpad-click-method-description = 点按取决于手指数量还是按钮区域。
schema-input-touchpad-dwt-enabled-label = 打字时停用
schema-input-touchpad-dwt-enabled-description = 使用键盘期间忽略触控板。
schema-input-touchpad-natural-scroll-enabled-label = 自然滚动
schema-input-touchpad-natural-scroll-enabled-description = 内容跟随手指移动。
schema-input-touchpad-left-handed-label = 左手模式
schema-input-touchpad-left-handed-description = 交换主要按钮和次要按钮。
schema-input-touchpad-middle-emulation-enabled-label = 模拟中键点按
schema-input-touchpad-middle-emulation-enabled-description = 同时按下两个按钮即为中键点按。
schema-input-scroll-speed-label = 滚动速度
schema-input-scroll-speed-description = 施加于滚动事件的软件倍数。
schema-input-pointer-accel-speed-label = 指针速度
schema-input-pointer-accel-speed-description = 指针加速，从 -1（最慢）到 1（最快）。
schema-input-pointer-accel-profile-label = 指针加速
schema-input-pointer-accel-profile-description = 恒定为原始速度；自适应遵循 libinput 的曲线。

# --- audio ---
schema-audio-sound-enabled-label = 界面声音
schema-audio-sound-enabled-description = 为界面事件播放声音反馈。
schema-audio-sound-theme-label = 声音主题
schema-audio-sound-theme-description = XDG 声音主题的名称。留空则自动检测。

# --- power_management ---
schema-power-management-manage-lid-switch-label = 处理合盖开关
schema-power-management-manage-lid-switch-description = 由 Otto 处理合盖，而不是交给 logind。
schema-power-management-on-lid-close-label = 合上盖子时
schema-power-management-on-lid-close-description = 合上笔记本盖子时发生的事。
schema-power-management-on-power-button-label = 按下电源键时
schema-power-management-on-power-button-description = 按下硬件电源键时发生的事。

# --- lock ---
schema-lock-locker-command-label = 锁屏命令
schema-lock-locker-command-description = 用于锁定会话的锁屏程序。
schema-lock-locker-args-label = 锁屏程序参数
schema-lock-locker-args-description = 传递给锁屏程序的参数。
schema-lock-auto-lock-timeout-label = 多久后锁定
schema-lock-auto-lock-timeout-description = 锁定前的闲置秒数。0 表示从不锁定。

# --- login ---
schema-login-greeter-command-label = 登录界面命令
schema-login-greeter-command-description = 在登录模式下启动的登录界面程序。
schema-login-greeter-args-label = 登录界面参数
schema-login-greeter-args-description = 传递给登录界面程序的参数。

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = 切换器跟随指针
schema-appswitcher-follow-cursor-description = 在指针所在的输出上显示应用程序切换器。
schema-appswitcher-colorize-icons-label = 切换器图标着色
schema-appswitcher-colorize-icons-description = 让 Dock 的图标色调延伸到应用程序切换器。Dock 的着色关闭时不起作用。


## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = 自动
settings-choice-system-language = 系统语言


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = 蓝色
settings-choice-accent-purple = 紫色
settings-choice-accent-pink = 粉色
settings-choice-accent-red = 红色
settings-choice-accent-orange = 橙色
settings-choice-accent-yellow = 黄色
settings-choice-accent-green = 绿色
settings-choice-accent-mint = 薄荷色
settings-choice-accent-teal = 蓝绿色
settings-choice-accent-cyan = 青色
settings-choice-accent-indigo = 靛蓝色
settings-choice-accent-brown = 棕色
settings-choice-accent-graphite = 石墨色
# The button under the shortcut list that adds another line.
settings-add-shortcut = 添加快捷键
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = 桌面 { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = 搜索应用程序和窗口…
launcher-search-apps = 搜索应用程序…
launcher-search-windows = 搜索窗口…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = 应用
launcher-badge-window = 窗口
launcher-badge-calc = 计算器


## Login, lock and authentication
##
## The greeter, the lock screen, and the panel both of them draw. Text that
## arrives from PAM or greetd at runtime is not here: those localise
## themselves, and restating them would be guessing at another program's words.
# Button under the login/lock card, offered only while the fingerprint reader
# is being waited on: it abandons the finger and asks for a password instead.
# The button sizes itself to the text, but it sits on a card 380pt wide — keep
# it to roughly 20 characters so it does not overhang.
auth-enter-password = 输入密码

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = 登录

# strftime pattern for the time in the large clock above the login/lock card,
# not prose: only the %-codes and the separators between them are yours.
# Change it where the local convention differs — %-I:%M %p for a 12-hour
# locale. The digits render at 46pt, so keep the result short.
auth-clock-time-format = %H:%M

# strftime pattern for the date under that clock, again not prose. Reorder the
# parts and change the punctuation to suit the locale (German would be
# "%A, %-d. %B"); the weekday and month names are translated by the system, so
# do not spell them out here. Renders at 15pt in a 360pt box.
auth-clock-date-format = %-m月%-d日 %A

### otto-greeter — the login screen shown before any session exists.
### Everything here is drawn on the login card, centred on the wallpaper.
### The card is narrow: prompts sit above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field while the login screen is asking who is logging
# in. One line, above a text field roughly 20 characters wide — keep it to one
# or two words.
greeter-prompt-username = 用户名

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = 密码

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = 正在验证…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = 登录服务不可用：{ $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = 登录服务已断开

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } 没有启动

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = 已通过验证

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = 将手指放在读取器上

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = 让手指划过读取器

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = 将{ $finger }放在读取器上
greeter-status-swipe-named-finger = 让{ $finger }划过读取器

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = 未识别出指纹

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = 正在等待指纹读取器…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = 正在启动会话…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = 无权进入睡眠

# As above, for the restart request.
greeter-power-restart-denied = 无权重新启动

# As above, for the shut down request.
greeter-power-shutdown-denied = 无权关机

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = 无法进入睡眠

# As above, for the restart request.
greeter-power-restart-failed = 无法重新启动

# As above, for the shut down request.
greeter-power-shutdown-failed = 无法关机

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = 密码

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = 已通过验证

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = 将手指放在读取器上

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = 让手指划过读取器

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = 将{ $finger }放在读取器上
lock-status-swipe-named-finger = 让{ $finger }划过读取器

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = 左手拇指
auth-finger-left-index = 左手食指
auth-finger-left-middle = 左手中指
auth-finger-left-ring = 左手无名指
auth-finger-left-little = 左手小指
auth-finger-right-thumb = 右手拇指
auth-finger-right-index = 右手食指
auth-finger-right-middle = 右手中指
auth-finger-right-ring = 右手无名指
auth-finger-right-little = 右手小指

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = 未识别出指纹

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = 正在等待指纹读取器…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = 没有可验证的用户

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = 验证服务出错

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = 用户名无效

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = 验证不可用

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = 验证未通过（{ $status }）

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = 无权进入睡眠

# As above, for the restart request.
lock-power-restart-denied = 无权重新启动

# As above, for the shut down request.
lock-power-shutdown-denied = 无权关机

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = 无法进入睡眠：{ $error }

# As above, for the restart request.
lock-power-restart-failed = 无法重新启动：{ $error }

# As above, for the shut down request.
lock-power-shutdown-failed = 无法关机：{ $error }


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
quickview-fact-kind = 种类
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = 大小
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = 尺寸
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = 时长
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = 像素
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = 页数
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = 标题
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = 表演者
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = 专辑
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = 年份

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = 空文件
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = 过大，无法预览
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } 百万像素
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = 安装其中之一即可看到页面：{ $packages }


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = 空文件夹
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
       *[other] { $count } 个项目
    }
# Summary line for an archive, joining the entry count to the archive's own
# size on disk. $items is quickview-item-count, $size is a formatted byte
# count. The dash is an em dash.
quickview-archive-summary = { $items } —— { $size }


## Quick View — sizes
##
## Byte units. Quick View counts in powers of 1024, so the symbols are the
## conventional binary-rounded ones. Translate only the spelled-out "bytes".

quickview-size-bytes =
    { $count ->
       *[other] { $count } 字节
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
quickview-error-not-previewable = 这不是可以预览的文件
# The file's metadata could not be read.
quickview-error-stat-file = 无法读取文件信息：{ $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = 无法读取文件：{ $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = 该文件不支持定位
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = 无法将预览程序放入沙盒：{ $error }

# Image previewer.
quickview-error-read-image = 无法读取图像：{ $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = 此版本无法解码的图像
quickview-error-image-no-size = 图像没有报告尺寸
quickview-error-image-decode = 图像没有解码成功：{ $error }
quickview-error-image-readback = 无法回读解码后的图像

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = 无法读取图形：{ $error }
quickview-error-drawing-parse = 无法解析该图形
quickview-error-drawing-surface = 没有可供绘制的表面
quickview-error-drawing-readback = 无法回读该图形

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = 此文件不是 Otto 能读取的任何编码的文本

# PDF previewer.
quickview-error-read-document = 无法读取文稿：{ $error }
quickview-error-page-readback = 无法读取渲染后的页面

# Folder listing.
quickview-error-read-folder = 无法读取文件夹

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = 找不到预览程序：{ $error }
quickview-error-previewer-start = 无法启动预览程序：{ $error }
quickview-error-previewer-no-output = 预览程序没有产生输出
quickview-error-previewer-unreadable = 预览程序产生了无法读取的内容
quickview-error-previewer-failed = 预览程序出错：{ $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = 此文件预览耗时过长

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
islands-close = 关闭

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = 刚刚
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count } 分钟前
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count } 小时前


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = 允许
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = 继续
# Refuses the request.
islands-dialog-deny = 拒绝


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
a11y-app-running = 正在运行
a11y-app-not-running = 未在运行
# The panel that appears while the switch-application keys are held.
a11y-app-switcher = 应用程序切换器
# The list of open windows shown by the overview.
a11y-windows = 窗口
# The strip of workspaces shown by the overview.
a11y-workspaces = 桌面
# A window that reports no title of its own.
a11y-untitled-window = 无标题窗口
# The bar across the top of the screen.
a11y-menu-bar = 菜单栏
# The right-hand end of the bar, holding the clock and the tray icons.
a11y-status = 状态
# A tray icon whose application gave it no name of its own. $number
# counts from 1, left to right.
a11y-tray-item = 托盘项目 { $number }
# The stack of notification islands.
a11y-notifications = 通知
# The sidebar of Settings, listing its panes.
a11y-categories = 类别
# The launcher's list of matches for what has been typed.
a11y-results = 结果
# Names the Settings pane when no pane is selected.
a11y-settings = 设置
# Quick Look's contents, when it is showing something with no pages.
a11y-preview = 预览
a11y-preview-page = 预览，第 { $page } 页，共 { $pages } 页
# Said of a preview that shows only the beginning of a long file.
a11y-preview-shortened = 预览，已截短
