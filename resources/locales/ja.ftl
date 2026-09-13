# Otto — Japanese
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

common-open = 開く
common-save = 保存
common-cancel = キャンセル
common-add = 追加
common-remove = 取り除く
common-quit = 終了
common-cut = カット
common-copy = コピー
common-paste = ペースト
common-rename = 名称変更
common-delete = 削除
common-replace = 置き換える
common-move = 移動


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = 自動的に隠す
dock-auto-hide-on = ✓ 自動的に隠す
dock-magnification = 拡大
dock-magnification-on = ✓ 拡大
dock-position-bottom = 下
dock-position-bottom-on = ✓ 下
dock-position-left = 左
dock-position-left-on = ✓ 左
dock-position-right = 右
dock-position-right-on = ✓ 右

# Shown on an app's icon when the app is not running.
dock-open = 開く
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Dockに保持
dock-keep-in-dock-on = ✓ Dockに保持
dock-quit = 終了


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = 一般
settings-pane-displays = ディスプレイ
settings-pane-dock = Dock
settings-pane-tiling = タイル表示
settings-pane-keyboard = キーボード
settings-pane-pointing = トラックパッドとマウス
settings-pane-sound = サウンド
settings-pane-power = 電源
settings-pane-lock-and-login = ロックとログイン


## Settings — General

settings-group-appearance = 外観
settings-colour-scheme = 配色
settings-accent-colour = アクセントカラー
settings-rounded-corners = 角丸
settings-frosting = すりガラス効果
settings-frosting-detail = Dock、バー、デスクトップのパネルの背後にある半透明でぼかした素材
settings-window-controls = ウインドウボタン
settings-maximize-button = ズームボタン
settings-maximize-button-detail = ズームドットを表示します。タイトルバーをダブルクリックするとどちらでもズームします
settings-font = システムフォント
settings-gtk-theme = GTKテーマ

settings-group-desktop = デスクトップ
settings-background-colour = 背景色
settings-background-image = 背景画像
settings-background-image-detail = デスクトップポータルのファイル選択画面から選びます
settings-background-image-unavailable = 表示できません

settings-group-pointer-and-icons = ポインタとアイコン
settings-cursor-theme = カーソルテーマ
settings-cursor-size = カーソルサイズ
settings-icon-theme = アイコンテーマ

settings-group-window-switcher = ウインドウスイッチャー
settings-follow-cursor = ポインタのあるディスプレイに表示

settings-group-language = 言語
settings-display-language = 表示言語

settings-group-configuration = 構成
settings-configuration-file = 設定ファイル
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = 不明 — コンポジタが応答していません


## Settings — Displays

settings-display-active = 有効
settings-display-active-detail = 無効なディスプレイも配置内の位置は保たれます
settings-display-primary = 主ディスプレイとして使用
settings-display-primary-detail = Dockとバーは主ディスプレイに表示されます
settings-display-x-position = X位置
settings-display-y-position = Y位置
settings-display-x-position-detail = デスクトップ座標空間での左上隅
settings-display-width = 幅
settings-display-width-detail = ピクセル。ヘッドレス出力は任意のサイズにできます
settings-display-height = 高さ
settings-display-refresh = リフレッシュレート
settings-display-refresh-detail = Hertz — ストリームにフレームが送られる頻度
settings-display-resolution = 解像度
settings-display-scale = ディスプレイスケール
settings-display-scale-detail = 次回のログインから適用されます。デスクトップはその場では組み直されません

# Shown when the compositor reports no outputs at all.
settings-display-none = ディスプレイなし
settings-display-none-detail = コンポジタはどの出力も駆動していません
# Under the display arrangement canvas, explaining what clicking one does.
settings-arrangement-hint = ディスプレイを選ぶと、その設定を下で変更できます

settings-virtual-displays = 仮想ディスプレイ
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
       *[other] PipeWireでストリーミングされるヘッドレス出力が { $count } 個。取り除くと選択中のものが外れます
    }
# Pill on a row whose setting the compositor cannot apply until it restarts.
# Kept short: it is drawn inside a badge beside the row's label.
settings-restart-required = 再起動が必要


## Settings — Dock

settings-dock-size = サイズ
settings-dock-position = 画面上の位置
settings-dock-autohide = 自動的に隠す
settings-dock-magnification = 拡大
settings-group-magnification-and-icons = 拡大とアイコン
settings-dock-magnification-amount = 拡大率
settings-dock-tint-icons = アイコンに色を付ける
settings-switcher-colorize-icons = スイッチャーのアイコンに色を付ける
settings-dock-icon-tint = アイコンの色
settings-dock-icon-tint-strength = 色の強さ


## Settings — Tiling

settings-tiling-intro = タイル表示のワークスペースは、ウインドウを並べて画面いっぱいに敷き詰めます。ここに示すのは初期値で、間隔はワークスペースごとに上書きできます。
settings-tiling-decoration = タイル表示中のウインドウの装飾
settings-group-tiling-gaps = 間隔
settings-tiling-inner-gap = タイルの間
settings-tiling-outer-gap = タイルの周囲
settings-tiling-smart-gaps = タイルが1つのときは間隔をなくす
settings-group-tiling-keyboard = キーボード
settings-tiling-resize-step = サイズ変更の刻み
settings-group-tiling-animation = アニメーション
settings-tiling-layout-duration = レイアウトの変化
settings-tiling-layout-bounce = レイアウトのバウンス
settings-tiling-mode-duration = タイル表示への切り替え
settings-tiling-mode-bounce = タイル表示のバウンス


## Settings — Keyboard

settings-key-repeat-delay = リピート開始までの時間
settings-key-repeat-rate = リピート速度
settings-group-input-source = 入力ソース
settings-xkb-layout = レイアウト
settings-xkb-variant = バリアント
settings-xkb-options = オプション
settings-group-shortcuts = ショートカット
settings-key-combination = キーの組み合わせ
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl、Alt、Shift、Logo を + でつなぎ、最後にキーを1つ：Ctrl+Shift+Return
settings-key-combination-unassigned = 未設定


## Settings — Trackpad & Mouse

settings-group-trackpad = トラックパッド
settings-tap-to-click = タップでクリック
settings-tap-and-drag = タップでドラッグ
settings-drag-lock = ドラッグロック
settings-click-method = クリック方法
settings-ignore-while-typing = 入力中は無視
settings-natural-scrolling = ナチュラルなスクロール
settings-left-handed = 左利き用
settings-middle-click-emulation = 中クリックのエミュレーション
settings-group-pointer = ポインタ
settings-tracking-speed = 軌跡の速さ
settings-pointer-acceleration = 加速
settings-scrolling-speed = スクロールの速さ


## Settings — Sound

settings-interface-sounds = インターフェイスのサウンド
settings-sound-theme = サウンドテーマ


## Settings — Power

settings-manage-lid-switch = ふたの開閉を管理
settings-manage-lid-switch-detail = ふたを閉じたとき、logindではなくOttoがスリープさせます
settings-on-lid-close = ふたを閉じたとき
settings-on-power-button = 電源ボタンを押したとき


## Settings — Lock & Login

settings-group-lock = ロック
settings-lock-after = ロックするまでの時間
settings-lock-screen = ロック画面
settings-lock-screen-detail = 次に画面がロックされるときから適用されます
settings-lock-screen-arguments = ロック画面の引数
settings-group-login = ログイン
settings-greeter = グリーター
settings-greeter-detail = 次回のログインから適用されます
settings-greeter-arguments = グリーターの引数


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = ライト
settings-choice-dark = ダーク
settings-choice-controls-left = 左
settings-choice-controls-right = 右
settings-choice-position-bottom = 下
settings-choice-position-left = 左
settings-choice-position-right = 右
settings-choice-clickfinger = 指の本数でクリック
settings-choice-buttonareas = 隅の領域でクリック
settings-choice-accel-flat = 一定の速度
settings-choice-accel-adaptive = 動きに応じた速度
settings-choice-lid-auto = 自動的に判断
settings-choice-lid-lock = 画面をロック
settings-choice-lid-disable-internal = 内蔵ディスプレイをオフ
settings-choice-power-ignore = 何もしない
settings-choice-power-lock = 画面をロック
settings-choice-power-suspend = スリープ
settings-choice-power-shutdown = システム終了
# The automatic option for a theme that follows the system.
settings-choice-auto = 自動


## Settings — readouts
##
## Units shown beside a slider. $value is already formatted as a number.

settings-readout-percent = { $value }%
settings-readout-pixels = { $value } px
settings-readout-milliseconds = { $value } ミリ秒
settings-readout-seconds = { $value } 秒
# Key repeats per second.
settings-readout-per-second = { $value } 回/秒


## Files — windows

files-window-title = ファイル
# The Get Info panel's own window.
files-info-window-title = 情報


## Files — commands

files-get-info = 情報を見る
files-new-folder = 新規フォルダ
files-move-to-trash = ゴミ箱に入れる
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
       *[other] { $count } 項目をゴミ箱に入れる
    }
files-put-back = 戻す
files-empty-trash = ゴミ箱を空にする
files-delete-immediately = すぐに削除
# $count is always two or more; the single-item case uses files-delete-immediately.
files-delete-count-immediately =
    { $count ->
       *[other] { $count } 項目をすぐに削除
    }


## Files — sidebar and columns

files-places = 場所
files-recent = 最近の項目
files-home = ホーム
files-desktop = デスクトップ
files-documents = 書類
files-downloads = ダウンロード
files-music = ミュージック
files-pictures = ピクチャ
files-videos = ビデオ
files-trash = ゴミ箱

# The Recent listing's day headings, over the run of files saved in each.
files-recent-today = 今日
files-recent-yesterday = 昨日
files-recent-this-week = 今週
files-recent-this-month = 今月
files-recent-earlier = それ以前
# フォルダを必要とするコマンド（名称変更、開く、ゴミ箱に入れる、プレビュー）を、
# 対応するフォルダのないリスト（「最近の項目」や検索結果）に使ったときに表示されます。
files-synthetic-no-action = このリストに対応するフォルダはありません。
files-recent-grid-only = 「最近の項目」はグリッドで表示されます。
files-recent-not-a-folder = 「最近の項目」はリストであり、フォルダではありません。
files-recent-no-location = 「最近の項目」には移動先の場所がありません。

# The filter strip between the header and the listing, and what it reports.
files-search-placeholder = 「{ $folder }」を絞り込む
files-search-scope-folder = このフォルダ
files-search-scope-everywhere = すべて
files-search-found =
    { $count ->
       *[other] { $count } 件の結果
    }
files-search-none = 見つかりません
files-search-no-columns = 結果に表示できる列はありません。
# Shown in place of a result count when the desktop's file indexer is not
# running. Search and Recent both go through it, so neither can answer without
# it — and an empty listing would read as "no such file" rather than as
# "nothing was able to look".
files-search-unavailable = ファイルのインデックス作成はオフです

files-column-name = 名前
files-column-size = サイズ
files-column-kind = 種類
files-column-date-modified = 変更日
# The Kind column's heading in the Trash window, where where a file came from
# matters more than what kind of file it is.
files-column-original-location = 元の場所


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = フォルダ
files-kind-image = イメージ
files-kind-movie = ムービー
files-kind-audio = オーディオ
files-kind-text = テキスト
files-kind-document = 書類
files-kind-archive = アーカイブ
files-kind-application = アプリケーション


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = 読み込み中…
files-empty = 空
# The idle line: what the folder holds.
files-status-no-items = 項目なし
files-status-items =
    { $count ->
       *[other] { $count } 項目
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }、非表示 { $hidden } 項目
files-status-selected = { $total } 項目中 { $count } 項目を選択
files-status-opening-preview = プレビューを開いています…
files-nothing-to-undo = 取り消せる操作はありません
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = { $label }を取り消しました
files-undo-move = 移動
files-undo-copy = コピー
files-undo-delete = 削除
files-undo-rename = 名称変更
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = 「{ $name }」に名称変更しました
files-new-folder-created = 新規フォルダ「{ $name }」
files-gone = 「{ $name }」はもうありません
files-no-such-folder = 「{ $path }」はありません
files-rename-failed = 名称変更できません：{ $error }
files-new-folder-failed = フォルダを作成できません：{ $error }
files-open-failed = そのファイルを開けません：{ $error }
files-new-window-failed = 新しいウインドウを開けません：{ $error }


## Files — the listing

files-folder-empty = このフォルダは空です。
files-trash-empty = ゴミ箱は空です。
# Opening a trashed file would launch an application on a file the user has
# thrown away; the offer is to put it back first.
files-trash-cant-open = ゴミ箱の中の項目は開けません。先に戻す必要があります。
files-trash-cant-rename = ゴミ箱の中の項目は名称変更できません。
files-folder-denied = このフォルダの中身を見る権限がありません。
files-folder-gone = このフォルダはもう存在しません。
files-folder-open-failed = このフォルダを開けませんでした：{ $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = 場所
files-info-kind = 種類
files-info-modified = 変更日
files-info-created = 作成日
files-info-accessed = アクセス日
files-info-owner = 所有者
files-info-links-to = リンク先
files-info-permissions = アクセス権
# Column headers over the permission checkboxes — narrower still.
files-perm-read = 読み
files-perm-write = 書き
files-perm-exec = 実行
# Row labels: who each set of permissions applies to.
files-perm-owner = 所有者
files-perm-group = グループ
files-perm-everyone = 全員


## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = 開く
files-picker-save-as = 別名で保存
files-picker-save-files = ファイルを保存
files-picker-all-files = すべてのファイル
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = 名前：

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = 名前を入力
files-save-name-has-slash = 名前に「/」は使えません
files-save-name-reserved = その名前は予約されています
files-save-nowhere = 保存先がありません
files-save-permission-denied = ここに保存する権限がありません

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = 「{ $name }」はすでに存在します。置き換えますか？
files-replace-one-detail = 置き換えると現在の内容は上書きされます。
files-replace-many = これらのファイルのうち { $count } 個はすでに存在します。置き換えますか？
files-replace-many-detail = 置き換えると現在の内容は上書きされます。
files-delete-forever-one = 「{ $name }」を完全に削除しますか？
files-delete-forever-many = { $count } 項目を完全に削除しますか？
files-delete-forever-detail = この操作は取り消せません。
files-empty-trash-confirm = ゴミ箱を空にしますか？
files-empty-trash-detail =
    { $count ->
       *[other] { $count } 項目が完全に削除されます。この操作は取り消せません。
    }


## Files — sizes
##
## Byte units. Otto counts in powers of 1000, so these are the SI units — KB,
## not KiB. Most languages keep the symbols as they are; translate only the
## spelled-out "bytes".

files-size-bytes =
    { $count ->
       *[other] { $count } バイト
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

## Files — commands

files-new-folder-with-selection = 選択項目から新規フォルダ
# $count is always two or more; the single-item case uses
# files-new-folder-with-selection.
files-new-folder-with-count =
    { $count ->
       *[other] { $count } 項目から新規フォルダ
    }
# The note under New Folder with Selection's list of items; $name is the
# folder they would go into.
files-new-folder-with-preview =
    { $count ->
       *[other] { $count } 項目を「{ $name }」に移動
    }


## Files — command palette

# The panel opened with Ctrl+P: type a few letters of a command's name to run it.
files-palette-placeholder = コマンドを実行
files-palette-no-matches = 一致するコマンドはありません
# $count is how many commands the palette is offering; announced when it opens.
files-palette-opened =
    { $count ->
       *[other] コマンドパレット、{ $count } 個のコマンド
    }

files-command-group-go = 移動
files-command-group-file = ファイル
files-command-group-edit = 編集
files-command-group-view = 表示

files-command-back = 戻る
files-command-forward = 進む
files-command-up = 上へ
files-command-go-to-path = パスへ移動
files-command-go-to-place = 場所へ移動
files-command-undo = 取り消す
files-command-select-all = すべてを選択
files-command-select-matching = 一致する項目を選択
files-command-move-to = フォルダへ移動
files-command-change-view = 表示を変更
files-command-sort-by = 並べ替え
files-command-show-hidden = 隠しファイルを表示
files-command-hide-hidden = 隠しファイルを非表示
files-command-quick-look = クイックルック
files-command-search = 検索

# The non-editable prefix the palette's field wears while an argument is being
# typed. A colon and a space are added after it.
files-command-go-to-path-prompt = パスへ移動
files-command-go-to-place-prompt = 場所へ移動
files-command-rename-prompt = 新しい名前
files-command-new-folder-prompt = 新規フォルダ名
files-command-change-view-prompt = 表示
files-command-sort-by-prompt = 並べ替え
files-command-search-prompt = 検索
files-command-select-matching-prompt = 一致する項目
files-command-move-to-prompt = 移動先
files-command-rename-many-prompt = 新しい名前

# The one-word name of what a command is asking for, shown dimmed after its
# title in the list.
files-command-arg-path = パス
files-command-arg-place = 場所
files-command-arg-name = 名前
files-command-arg-view = 表示
files-command-arg-sort = 基準
files-command-arg-query = テキスト
files-command-arg-pattern = パターン

files-view-list = リスト
files-view-grid = グリッド
files-view-columns = カラム
# Refused when a name could not belong to a file — empty, or with a slash in it.
files-name-invalid = ファイルに使える名前ではありません
files-no-pattern = パターンを入力（例：*.png）
files-nothing-matches = 「{ $pattern }」に一致する項目はありません

# Renaming a selection from one pattern — see `rename.rs` for the pattern.
files-rename-many = { $count } 項目を名称変更
# The dry run's summary line under the palette's rows.
files-rename-preview = { $total } 項目中 { $count } 項目を名称変更
files-rename-preview-unchanged = 変更はありません
files-rename-conflicts = { $count ->
   *[other] { $count } 個の名前はすでに使われています
}
files-rename-invalid = { $count ->
   *[other] { $count } 個の名前が空になります
}
files-renamed-count = { $count } 項目を名称変更しました

# Scripts in ~/.config/otto/files-scripts/; see components/otto-files/src/scripts.rs.
files-script-running = { $title }を実行中…
files-script-failed-quietly = スクリプトは理由を示さずに失敗しました
files-script-timed-out = スクリプトに時間がかかりすぎました
files-nothing-selected = 何も選択されていません
files-cant-move-into-itself = フォルダを自分自身の中には移動できません
# $count is how many entries a pattern selected.
files-selected-count =
    { $count ->
       *[other] { $count } 項目を選択
    }
files-name-taken = 「{ $name }」はすでにあります


## Files — status

files-undo-new-folder-with-selection = 選択項目から新規フォルダ


## Bar
##
## The menu bar across the top of the screen.

# The clock's format, as chrono specifiers — NOT prose. Rewrite it to the
# locale's own convention: 24-hour here, 12-hour with %p for en-US, and the
# day before the month everywhere except en-US. Do not add or remove %S:
# whether seconds show is a user setting, and it changes how often the bar
# redraws.
bar-clock-format = %-m月%-d日(%a)  %H:%M


## Settings — widgets
##
## The controls themselves, rather than the settings they edit.

# Shown in a text field that has no value yet.
settings-not-set = 未設定
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = 選択…
settings-no-file-chosen = ファイル未選択
settings-choose-background-image = 背景画像を選択

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Otto 設定 — { $pane }


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
schema-screen-scale-label = ディスプレイスケール
schema-screen-scale-description = デスクトップ全体に適用される拡大率。
schema-theme-scheme-label = 配色
schema-theme-scheme-description = ライトまたはダークの配色。
schema-accent-color-label = アクセントカラー
schema-accent-color-description = ライトとダークの配色に追従するパレット名、または #RRGGBB のカラー。
schema-rounded-corners-label = 角丸
schema-rounded-corners-description = Dock、上部バー、ウインドウの装飾、デスクトップ自身のパネルに適用されます。
schema-frosting-label = すりガラス効果
schema-frosting-description = Dock、トップバー、ランチャー、デスクトップのパネルの背後にある半透明でぼかした素材。
schema-window-controls-side-label = ウインドウボタン
schema-window-controls-side-description = 閉じる・しまう・ズームのボタンをタイトルバーのどちら側に置くか。
schema-show-maximize-button-label = ズームボタン
schema-show-maximize-button-description = ウインドウのタイトルバーにズームボタンを表示します。デフォルトはオフで、タイトルバーをダブルクリックすればどちらでもズームします。
schema-font-family-label = インターフェイスフォント
schema-font-family-description = Otto自身のインターフェイスが使うフォントファミリー。
schema-background-color-label = 背景色
schema-background-color-description = デスクトップの背景色。16進文字列で指定します。
schema-background-image-label = 背景画像
schema-background-image-description = デスクトップの背景画像のパス。空なら画像なし。
schema-cursor-theme-label = カーソルテーマ
schema-cursor-theme-description = XCursorテーマの名前。
schema-cursor-size-label = カーソルサイズ
schema-cursor-size-description = 論理ピクセル単位のカーソルサイズ。
schema-icon-theme-label = アイコンテーマ
schema-icon-theme-description = アイコンテーマの名前。空なら自動検出します。
schema-gtk-theme-label = GTKテーマ
schema-gtk-theme-description = クライアントに渡すGTKテーマの名前。空なら自動検出します。
schema-locales-label = ロケール
schema-locales-description = 優先するロケール。優先度の高い順に並べます。

# --- tiling ---
schema-tiling-decoration-label = タイル表示中のウインドウの装飾
schema-tiling-decoration-description = タイル表示中のウインドウが保つ装飾の量。フロート時と同じタイトルバー、タイトルと閉じるボタンだけの文字1行分の高さのバー、あるいはバーをなくし、選択中のタイルを細い枠線で示すかを選べます。
schema-tiling-inner-gap-label = タイルの間の間隔
schema-tiling-inner-gap-description = 隣り合うタイルの間に空ける論理ピクセル。
schema-tiling-outer-gap-label = タイルの周囲の間隔
schema-tiling-outer-gap-description = タイルと画面の端の間に空ける論理ピクセル。
schema-tiling-smart-gaps-label = タイルが1つのときは間隔をなくす
schema-tiling-smart-gaps-description = タイルが1つだけのワークスペースでは間隔をまったく空けず、ウインドウが理由もなく内側に寄って見えないようにします。
schema-tiling-resize-step-label = サイズ変更の刻み
schema-tiling-resize-step-description = キーボードでの1回のサイズ変更でコンテナがどれだけ動くか。幅または高さに対する割合です。
schema-tiling-layout-duration-label = レイアウトのアニメーション
schema-tiling-layout-duration-description = レイアウトの変化にかかる秒数。ウインドウのツリーへの追加や離脱、移動、入れ替え、均等化が対象です。0なら瞬時に切り替わります。
schema-tiling-layout-bounce-label = レイアウトのバウンス
schema-tiling-layout-bounce-description = レイアウトの変化が落ち着くまでにどれだけ行き過ぎるか。0なら行き過ぎずに落ち着きます。
schema-tiling-mode-duration-label = タイル表示のアニメーション
schema-tiling-mode-duration-description = タイル表示を切り替えて各ウインドウがそれぞれのセルへ飛ぶとき、ワークスペースの並べ直しにかかる秒数。0なら瞬時に切り替わります。
schema-tiling-mode-bounce-label = タイル表示のバウンス
schema-tiling-mode-bounce-description = その並べ直しが落ち着くまでにどれだけ行き過ぎるか。

# --- dock ---
schema-dock-size-label = サイズ
schema-dock-size-description = Dockのサイズの倍率。
schema-dock-position-label = 画面上の位置
schema-dock-position-description = Dockを置く画面の端。
schema-dock-autohide-label = 自動的に隠す
schema-dock-autohide-description = ポインタが画面の端に届くまでDockを隠します。
schema-dock-magnification-label = 拡大
schema-dock-magnification-description = ポインタの下のアイコンを大きくします。
schema-dock-genie-scale-label = 拡大率
schema-dock-genie-scale-description = ポインタの下のアイコンをどれだけ大きくするか。
schema-dock-genie-span-label = 拡大の広がり
schema-dock-genie-span-description = 拡大が隣のアイコンいくつまで及ぶか。
schema-dock-colorize-icons-label = アイコンに色を付ける
schema-dock-colorize-icons-description = Dockのアイコンを単一の色で染めます。
schema-dock-colorize-color-label = アイコンの色
schema-dock-colorize-color-description = Dockのアイコンを染める色。16進文字列で指定します。
schema-dock-colorize-intensity-label = 色の強さ
schema-dock-colorize-intensity-description = 色をどれだけ強く適用するか。

# --- general ---
schema-keyboard-repeat-delay-label = リピート開始までの時間
schema-keyboard-repeat-delay-description = キーを押し続けてからリピートが始まるまでのミリ秒。
schema-keyboard-repeat-rate-label = リピート速度
schema-keyboard-repeat-rate-description = キーを押し続けている間の1秒あたりのリピート回数。

# --- input ---
schema-input-xkb-layout-label = キーボードレイアウト
schema-input-xkb-layout-description = XKBレイアウトの名前。空ならシステムのデフォルトを使います。
schema-input-xkb-variant-label = キーボードバリアント
schema-input-xkb-variant-description = XKBバリアントの名前。空ならシステムのデフォルトを使います。
schema-input-xkb-options-label = キーボードオプション
schema-input-xkb-options-description = XKBのオプション文字列。
schema-input-tap-enabled-label = タップでクリック
schema-input-tap-enabled-description = トラックパッドのタップをクリックとして扱います。
schema-input-tap-drag-enabled-label = タップでドラッグ
schema-input-tap-drag-enabled-description = タップに続けて指を置いたままにするとドラッグを始めます。
schema-input-tap-drag-lock-enabled-label = ドラッグロック
schema-input-tap-drag-lock-enabled-description = 指を一瞬離してもタップドラッグを続けます。
schema-input-touchpad-click-method-label = クリック方法
schema-input-touchpad-click-method-description = クリックを指の本数で決めるか、ボタンの領域で決めるか。
schema-input-touchpad-dwt-enabled-label = 入力中は無効
schema-input-touchpad-dwt-enabled-description = キーボードを使っている間はトラックパッドを無視します。
schema-input-touchpad-natural-scroll-enabled-label = ナチュラルなスクロール
schema-input-touchpad-natural-scroll-enabled-description = 内容が指に追従します。
schema-input-touchpad-left-handed-label = 左利き用
schema-input-touchpad-left-handed-description = 主ボタンと副ボタンを入れ替えます。
schema-input-touchpad-middle-emulation-enabled-label = 中クリックのエミュレーション
schema-input-touchpad-middle-emulation-enabled-description = 両方のボタンを同時に押すと中クリックになります。
schema-input-scroll-speed-label = スクロールの速さ
schema-input-scroll-speed-description = スクロールイベントに適用されるソフトウェア側の倍率。
schema-input-pointer-accel-speed-label = ポインタの速さ
schema-input-pointer-accel-speed-description = ポインタの加速。-1（最も遅い）から1（最も速い）まで。
schema-input-pointer-accel-profile-label = ポインタの加速
schema-input-pointer-accel-profile-description = 一定は生の速度、適応型はlibinputのカーブに従います。

# --- audio ---
schema-audio-sound-enabled-label = インターフェイスのサウンド
schema-audio-sound-enabled-description = インターフェイスの操作に音のフィードバックを鳴らします。
schema-audio-sound-theme-label = サウンドテーマ
schema-audio-sound-theme-description = XDGサウンドテーマの名前。空なら自動検出します。

# --- power_management ---
schema-power-management-manage-lid-switch-label = ふたの開閉を管理
schema-power-management-manage-lid-switch-description = ふたの扱いをlogindに任せず、Ottoが受け持ちます。
schema-power-management-on-lid-close-label = ふたを閉じたとき
schema-power-management-on-lid-close-description = ノートブックのふたを閉じたときの動作。
schema-power-management-on-power-button-label = 電源ボタンを押したとき
schema-power-management-on-power-button-description = ハードウェアの電源ボタンを押したときの動作。

# --- lock ---
schema-lock-locker-command-label = ロック画面のコマンド
schema-lock-locker-command-description = セッションをロックするために起動するロッカー。
schema-lock-locker-args-label = ロック画面の引数
schema-lock-locker-args-description = ロッカーに渡す引数。
schema-lock-auto-lock-timeout-label = ロックするまでの時間
schema-lock-auto-lock-timeout-description = ロックするまでの無操作の秒数。0ならロックしません。

# --- login ---
schema-login-greeter-command-label = グリーターのコマンド
schema-login-greeter-command-description = ログインモードで起動するグリーター。
schema-login-greeter-args-label = グリーターの引数
schema-login-greeter-args-description = グリーターに渡す引数。

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = スイッチャーがポインタに追従
schema-appswitcher-follow-cursor-description = ポインタのある出力にAppスイッチャーを表示します。
schema-appswitcher-colorize-icons-label = スイッチャーのアイコンに色を付ける
schema-appswitcher-colorize-icons-description = Dockのアイコンの色をAppスイッチャーにも及ぼします。Dockの色付けがオフのあいだは何もしません。


## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = 自動
settings-choice-system-language = システムの言語


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = ブルー
settings-choice-accent-purple = パープル
settings-choice-accent-pink = ピンク
settings-choice-accent-red = レッド
settings-choice-accent-orange = オレンジ
settings-choice-accent-yellow = イエロー
settings-choice-accent-green = グリーン
settings-choice-accent-mint = ミント
settings-choice-accent-teal = ティール
settings-choice-accent-cyan = シアン
settings-choice-accent-indigo = インディゴ
settings-choice-accent-brown = ブラウン
settings-choice-accent-graphite = グラファイト
# The button under the shortcut list that adds another line.
settings-add-shortcut = ショートカットを追加
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = ワークスペース { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Appとウインドウを検索…
launcher-search-apps = Appを検索…
launcher-search-windows = ウインドウを検索…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = ウインドウ
launcher-badge-calc = 計算


## Emoji picker

# What the search field says when empty.
emoji-search = 絵文字を検索…
# Shown in the grid when nothing matches what was typed.
emoji-no-results = 絵文字が見つかりません
# The section and tab titles. These are Unicode's own category names, and
# the CLDR translations of them are the reference where one exists.
emoji-group-recent = 最近使った項目
emoji-group-smileys = スマイリーと感情
emoji-group-people = 人と身体
emoji-group-nature = 動物と自然
emoji-group-food = 食べ物と飲み物
emoji-group-travel = 旅行と場所
emoji-group-activities = アクティビティ
emoji-group-objects = オブジェクト
emoji-group-symbols = 記号
emoji-group-flags = 旗


## Login, lock and authentication
##
## The greeter, the lock screen, and the panel both of them draw. Text that
## arrives from PAM or greetd at runtime is not here: those localise
## themselves, and restating them would be guessing at another program's words.
# Button under the login/lock card, offered only while the fingerprint reader
# is being waited on: it abandons the finger and asks for a password instead.
# The button sizes itself to the text, but it sits on a card 380pt wide — keep
# it to roughly 20 characters so it does not overhang.
auth-enter-password = パスワードを入力

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = サインイン

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
greeter-prompt-username = ユーザ名

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = パスワード

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = 認証中…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = ログインサービスを利用できません：{ $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = ログインサービスがいなくなりました

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = { $session } は起動しませんでした

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = 認証されました

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = リーダーに指を置いてください

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = リーダーで指をスライドしてください

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = リーダーに{ $finger }を置いてください
greeter-status-swipe-named-finger = リーダーで{ $finger }をスライドしてください

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = 指紋を認識できませんでした

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = 指紋リーダーを待っています…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = セッションを開始しています…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = スリープは許可されていません

# As above, for the restart request.
greeter-power-restart-denied = 再起動は許可されていません

# As above, for the shut down request.
greeter-power-shutdown-denied = システム終了は許可されていません

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = スリープできませんでした

# As above, for the restart request.
greeter-power-restart-failed = 再起動できませんでした

# As above, for the shut down request.
greeter-power-shutdown-failed = システム終了できませんでした

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = パスワード

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = 認証されました

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = リーダーに指を置いてください

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = リーダーで指をスライドしてください

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = リーダーに{ $finger }を置いてください
lock-status-swipe-named-finger = リーダーで{ $finger }をスライドしてください

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = 左の親指
auth-finger-left-index = 左の人差し指
auth-finger-left-middle = 左の中指
auth-finger-left-ring = 左の薬指
auth-finger-left-little = 左の小指
auth-finger-right-thumb = 右の親指
auth-finger-right-index = 右の人差し指
auth-finger-right-middle = 右の中指
auth-finger-right-ring = 右の薬指
auth-finger-right-little = 右の小指

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = 指紋を認識できませんでした

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = 指紋リーダーを待っています…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = 認証するユーザがいません

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = 認証サービスが応答しませんでした

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = ユーザ名が無効です

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = 認証を利用できません

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = 認証されませんでした（{ $status }）

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = スリープは許可されていません

# As above, for the restart request.
lock-power-restart-denied = 再起動は許可されていません

# As above, for the shut down request.
lock-power-shutdown-denied = システム終了は許可されていません

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = スリープできません：{ $error }

# As above, for the restart request.
lock-power-restart-failed = 再起動できません：{ $error }

# As above, for the shut down request.
lock-power-shutdown-failed = システム終了できません：{ $error }


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
quickview-fact-kind = 種類
# Column heading for the file's size on disk. Max ~12 characters.
quickview-fact-size = サイズ
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = 寸法
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = 長さ
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = 画素数
# Column heading for a PDF's page count. Max ~12 characters.
quickview-fact-pages = ページ数
# Column heading for a PDF's document title, taken from the document itself.
# Max ~12 characters.
quickview-fact-title = タイトル
# Column heading for a song's performer, from its ID3 tags. Max ~12 characters.
quickview-fact-artist = アーティスト
# Column heading for a song's album, from its ID3 tags. Max ~12 characters.
quickview-fact-album = アルバム
# Column heading for a song's year of release, from its ID3 tags. Max ~12 characters.
quickview-fact-year = 年

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = 空のファイル
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = 大きすぎてプレビューできません
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels = { $count } メガピクセル
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = ページを表示するには次のいずれかをインストール：{ $packages }


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = 空のフォルダ
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
       *[other] { $count } 項目
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
       *[other] { $count } バイト
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
quickview-error-not-previewable = これはプレビューできる種類のファイルではありません
# The file's metadata could not be read.
quickview-error-stat-file = ファイルの情報を取得できません：{ $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = ファイルを読み込めません：{ $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = このファイルはシークできません
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = プレビューアをサンドボックスに入れられません：{ $error }

# Image previewer.
quickview-error-read-image = イメージを読み込めません：{ $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = このビルドではデコードできないイメージです
quickview-error-image-no-size = イメージがサイズを報告しません
quickview-error-image-decode = イメージをデコードできませんでした：{ $error }
quickview-error-image-readback = デコードしたイメージを読み戻せませんでした

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = ベクター画像を読み込めません：{ $error }
quickview-error-drawing-parse = ベクター画像を解析できませんでした
quickview-error-drawing-surface = 描画先のサーフェスがありません
quickview-error-drawing-readback = ベクター画像を読み戻せませんでした

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = このファイルはOttoが読めるどの符号化のテキストでもありません

# PDF previewer.
quickview-error-read-document = 書類を読み込めません：{ $error }
quickview-error-page-readback = レンダリングしたページを読み取れませんでした

# Folder listing.
quickview-error-read-folder = フォルダを読み込めません

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = プレビューアが見つかりません：{ $error }
quickview-error-previewer-start = プレビューアを起動できません：{ $error }
quickview-error-previewer-no-output = プレビューアは何も出力しませんでした
quickview-error-previewer-unreadable = プレビューアが読み取れないものを出力しました
quickview-error-previewer-failed = プレビューアが止まりました：{ $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = このファイルはプレビューに時間がかかりすぎました

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
islands-close = 閉じる

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = たった今
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = { $count }分前
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = { $count }時間前


## Islands — permission dialogs
##
## Default button labels for a dialog raised by the desktop portal — screen
## sharing, file access. An application may supply its own labels instead, in
## which case these are not used. Buttons are side by side and narrow: one
## word each.

# Grants the request outright, when the dialog asks nothing else.
islands-dialog-allow = 許可
# Grants the request when the dialog also asks the person to choose something
# — which screen to share, for instance — so it carries them onward rather
# than simply consenting.
islands-dialog-continue = 続ける
# Refuses the request.
islands-dialog-deny = 拒否


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
a11y-app-running = 起動中
a11y-app-not-running = 起動していません
# The panel that appears while the switch-application keys are held.
a11y-app-switcher = Appスイッチャー
# The list of open windows shown by the overview.
a11y-windows = ウインドウ
# The strip of workspaces shown by the overview.
a11y-workspaces = ワークスペース
# A window that reports no title of its own.
a11y-untitled-window = 名称未設定のウインドウ
# The bar across the top of the screen.
a11y-menu-bar = メニューバー
# The right-hand end of the bar, holding the clock and the tray icons.
a11y-status = ステータス
# A tray icon whose application gave it no name of its own. $number
# counts from 1, left to right.
a11y-tray-item = トレイ項目 { $number }
# The stack of notification islands.
a11y-notifications = 通知
# The sidebar of Settings, listing its panes.
a11y-categories = カテゴリ
# The launcher's list of matches for what has been typed.
a11y-results = 結果
# Names the Settings pane when no pane is selected.
a11y-settings = 設定
# Quick Look's contents, when it is showing something with no pages.
a11y-preview = プレビュー
a11y-preview-page = プレビュー、{ $pages } ページ中 { $page } ページ
# Said of a preview that shows only the beginning of a long file.
a11y-preview-shortened = プレビュー、短縮表示
