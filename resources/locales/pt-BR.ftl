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
common-save = Salvar
common-cancel = Cancelar
common-add = Adicionar
common-remove = Remover
common-quit = Sair
common-cut = Recortar
common-copy = Copiar
common-paste = Colar
common-rename = Renomear
common-delete = Excluir
common-move = Mover


## Dock
##
## The dock's context menus. "Dock" is a product noun and stays untranslated.
## A tick prefixes the label when the setting is on; it is part of the string
## so the tick and the text can be reordered together if a language needs it.

dock-auto-hide = Ocultar automaticamente
dock-auto-hide-on = ✓ Ocultar automaticamente
dock-magnification = Ampliação
dock-magnification-on = ✓ Ampliação
dock-position-bottom = Embaixo
dock-position-bottom-on = ✓ Embaixo
dock-position-left = Esquerda
dock-position-left-on = ✓ Esquerda
dock-position-right = Direita
dock-position-right-on = ✓ Direita

# Shown on an app's icon when the app is not running.
dock-open = Abrir
# Pins an application to the dock so it stays after it quits.
dock-keep-in-dock = Manter no Dock
dock-keep-in-dock-on = ✓ Manter no Dock
dock-quit = Sair


## Settings — pane names
##
## The sidebar of the settings application. These are short by necessity: the
## sidebar does not grow to fit them.

settings-pane-general = Geral
settings-pane-displays = Monitores
settings-pane-dock = Dock
settings-pane-keyboard = Teclado
settings-pane-pointing = Trackpad e mouse
settings-pane-sound = Som
settings-pane-power = Energia
settings-pane-lock-and-login = Bloqueio e login


## Settings — General

settings-group-appearance = Aparência
settings-colour-scheme = Esquema de cores
settings-accent-colour = Cor de destaque
settings-rounded-corners = Cantos arredondados
settings-rounded-corners-detail = Aplicado após reiniciar
settings-window-controls = Controles da janela
settings-maximize-button = Botão de maximizar
settings-maximize-button-detail = Mostra o ponto de ampliar; um clique duplo na barra de título amplia de qualquer forma
settings-font = Fonte do sistema
settings-gtk-theme = Tema do GTK

settings-group-desktop = Área de trabalho
settings-background-colour = Cor do plano de fundo
settings-background-image = Imagem do plano de fundo
settings-background-image-detail = Escolhida através do seletor de arquivos do portal da área de trabalho

settings-group-pointer-and-icons = Ponteiro e ícones
settings-cursor-theme = Tema do cursor
settings-cursor-size = Tamanho do cursor
settings-icon-theme = Tema de ícones

settings-group-window-switcher = Alternador de janelas
settings-follow-cursor = Mostrar no monitor do ponteiro

settings-group-language = Idioma
settings-display-language = Idioma da interface

settings-group-configuration = Configuração
settings-configuration-file = Arquivo de configuração
# Shown in place of the file's path when the compositor does not answer.
settings-configuration-file-unknown = desconhecido — o compositor não está respondendo


## Settings — Displays

settings-display-active = Ativo
settings-display-active-detail = Um monitor inativo mantém seu lugar no arranjo
settings-display-primary = Usar como principal
settings-display-primary-detail = O dock e a barra ficam no monitor principal
settings-display-x-position = Posição X
settings-display-y-position = Posição Y
settings-display-x-position-detail = Canto superior esquerdo no espaço de coordenadas da área de trabalho
settings-display-width = Largura
settings-display-width-detail = Pixels. Uma saída headless pode ter qualquer tamanho
settings-display-height = Altura
settings-display-refresh = Taxa de atualização
settings-display-refresh-detail = Hertz — a frequência com que o fluxo recebe um quadro
settings-display-resolution = Resolução
settings-display-scale = Escala do monitor
settings-display-scale-detail = Aplicada no próximo login. A área de trabalho não se reorganiza em tempo real

# Shown when the compositor reports no outputs at all.
settings-display-none = Nenhum monitor
settings-display-none-detail = O compositor não está controlando nenhuma saída

settings-virtual-displays = Monitores virtuais
# $count is how many headless outputs exist. These are streamed to other
# machines rather than shown on a panel.
settings-virtual-displays-detail =
    { $count ->
        [one] { $count } saída headless, transmitida via PipeWire. Remover elimina a selecionada
        [many] { $count } saídas headless, transmitidas via PipeWire. Remover elimina a selecionada
       *[other] { $count } saídas headless, transmitidas via PipeWire. Remover elimina a selecionada
    }


## Settings — Dock

settings-dock-size = Tamanho
settings-dock-position = Posição na tela
settings-dock-autohide = Ocultar automaticamente
settings-dock-magnification = Ampliação
settings-group-magnification-and-icons = Ampliação e ícones
settings-dock-magnification-amount = Nível de ampliação
settings-dock-tint-icons = Colorir ícones
settings-switcher-colorize-icons = Colorir o alternador
settings-dock-icon-tint = Cor dos ícones
settings-dock-icon-tint-strength = Intensidade da cor dos ícones


## Settings — Keyboard

settings-key-repeat-delay = Atraso de repetição de tecla
settings-key-repeat-rate = Taxa de repetição de tecla
settings-group-input-source = Fonte de entrada
settings-xkb-layout = Layout
settings-xkb-variant = Variante
settings-xkb-options = Opções
settings-group-shortcuts = Atalhos
settings-key-combination = Combinação de teclas
# Modifier names and the example are literal syntax. Do not translate Ctrl,
# Alt, Shift, Logo or Ctrl+Shift+Return.
settings-key-combination-detail = Ctrl, Alt, Shift ou Logo unidos por +, seguidos de uma tecla: Ctrl+Shift+Return


## Settings — Trackpad & Mouse

settings-group-trackpad = Trackpad
settings-tap-to-click = Tocar para clicar
settings-tap-and-drag = Tocar e arrastar
settings-drag-lock = Bloqueio de arraste
settings-click-method = Método de clique
settings-ignore-while-typing = Ignorar ao digitar
settings-natural-scrolling = Rolagem natural
settings-left-handed = Canhoto
settings-middle-click-emulation = Emulação de clique do meio
settings-group-pointer = Ponteiro
settings-tracking-speed = Velocidade de rastreamento
settings-pointer-acceleration = Aceleração
settings-scrolling-speed = Velocidade de rolagem


## Settings — Sound

settings-interface-sounds = Sons da interface
settings-sound-theme = Tema de som


## Settings — Power

settings-manage-lid-switch = Controlar o interruptor da tampa
settings-manage-lid-switch-detail = O Otto suspende ao fechar a tampa em vez do logind
settings-on-lid-close = Ao fechar a tampa
settings-on-power-button = Ao pressionar o botão de energia


## Settings — Lock & Login

settings-group-lock = Bloqueio
settings-lock-after = Bloquear após
settings-lock-screen = Tela de bloqueio
settings-lock-screen-detail = Aplicada na próxima vez que a tela bloquear
settings-lock-screen-arguments = Argumentos da tela de bloqueio
settings-group-login = Login
settings-greeter = Tela de login
settings-greeter-detail = Aplicada no próximo login
settings-greeter-arguments = Argumentos da tela de login


## Settings — choices
##
## The options inside pop-up menus. Each belongs to the setting named in its
## key, so the same English word may need different translations in different
## languages depending on what it modifies.

settings-choice-light = Claro
settings-choice-dark = Escuro
settings-choice-controls-left = Esquerda
settings-choice-controls-right = Direita
settings-choice-position-bottom = Embaixo
settings-choice-position-left = Esquerda
settings-choice-position-right = Direita
settings-choice-clickfinger = Clicar com os dedos
settings-choice-buttonareas = Clicar nos cantos
settings-choice-accel-flat = Velocidade constante
settings-choice-accel-adaptive = Velocidade acompanha o movimento
settings-choice-lid-auto = Decidir automaticamente
settings-choice-lid-lock = Bloquear a tela
settings-choice-lid-disable-internal = Desligar o monitor integrado
settings-choice-power-ignore = Não fazer nada
settings-choice-power-lock = Bloquear a tela
settings-choice-power-suspend = Suspender
settings-choice-power-shutdown = Desligar
# The automatic option for a theme that follows the system.
settings-choice-auto = Automático


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

files-window-title = Arquivos
# The Get Info panel's own window.
files-info-window-title = Informações


## Files — commands

files-get-info = Obter informações
files-new-folder = Nova pasta
files-move-to-trash = Mover para o lixo
# $count is always two or more; the single-item case uses files-move-to-trash.
files-move-count-to-trash =
    { $count ->
        [one] Mover { $count } item para o lixo
        [many] Mover { $count } itens para o lixo
       *[other] Mover { $count } itens para o lixo
    }


## Files — sidebar and columns

files-places = Locais
files-home = Início
files-desktop = Área de trabalho
files-documents = Documentos
files-downloads = Downloads
files-music = Música
files-pictures = Imagens
files-videos = Vídeos
files-trash = Lixeira

files-column-name = Nome
files-column-size = Tamanho
files-column-kind = Tipo
files-column-date-modified = Data de modificação


## Files — kinds
##
## The Kind column. These name what a file is, as a user would say it.

files-kind-folder = Pasta
files-kind-image = Imagem
files-kind-movie = Filme
files-kind-audio = Áudio
files-kind-text = Texto
files-kind-document = Documento
files-kind-archive = Arquivo compactado
files-kind-application = Aplicativo


## Files — status
##
## The line under the listing. It reports what just happened; it never
## apologises and never blames.

files-loading = Carregando…
files-empty = Vazio
# The idle line: what the folder holds.
files-status-no-items = Nenhum item
files-status-items =
    { $count ->
        [one] { $count } item
        [many] { $count } itens
       *[other] { $count } itens
    }
# $items is an already-formatted count from files-status-items.
files-status-items-hidden = { $items }, { $hidden } ocultos
files-status-selected = { $count } de { $total } selecionados
files-status-opening-preview = Abrindo a visualização…
files-nothing-to-undo = Nada para desfazer
# $label is a command name — Move, Copy, Delete — from the files-undo-* keys.
files-undid = Desfeito: { $label }
files-undo-move = Mover
files-undo-copy = Copiar
files-undo-delete = Excluir
files-undo-rename = Renomear
# $name is a file or folder name, already wrapped in quotation marks.
files-renamed-to = Renomeado para “{ $name }”
files-new-folder-created = Nova pasta “{ $name }”
files-gone = “{ $name }” não está mais lá
files-no-such-folder = “{ $path }” não existe
files-rename-failed = Não foi possível renomear: { $error }
files-new-folder-failed = Não foi possível criar a pasta: { $error }
files-open-failed = Não foi possível abrir o arquivo: { $error }
files-new-window-failed = Não foi possível abrir uma nova janela: { $error }


## Files — the listing

files-folder-empty = Esta pasta está vazia.
files-folder-denied = Sem permissão para ver o conteúdo desta pasta.
files-folder-gone = Esta pasta não existe mais.
files-folder-open-failed = Não foi possível abrir esta pasta: { $error }


## Files — Get Info
##
## The panel behind Get Info. The left column is a set of field names; keep
## them short, they share a narrow column with the values beside them.

files-info-where = Onde
files-info-kind = Tipo
files-info-modified = Modificado
files-info-created = Criado
files-info-accessed = Último acesso
files-info-owner = Proprietário
files-info-links-to = Link para
files-info-permissions = Permissões
# Column headers over the permission checkboxes — narrower still.
files-perm-read = Leitura
files-perm-write = Escrita
files-perm-exec = Execução
# Row labels: who each set of permissions applies to.
files-perm-owner = Proprietário
files-perm-group = Grupo
files-perm-everyone = Todos

## Files — the file picker
##
## Shown to other applications through the desktop portal, so these are the
## first Otto strings many users see.

files-picker-open = Abrir
files-picker-save-as = Salvar como
files-picker-save-files = Salvar arquivos
files-picker-all-files = Todos os arquivos
# The label beside the name field, so it carries its colon.
files-picker-save-as-field = Salvar como:

# Why the Save button is refusing. Each states the situation, not the mistake.
files-save-enter-a-name = Digite um nome
files-save-name-has-slash = Um nome não pode conter “/”
files-save-name-reserved = Esse nome é reservado
files-save-nowhere = Nenhum local para salvar
files-save-permission-denied = Sem permissão para salvar aqui

# Confirming an overwrite. $name is a file name, already in quotation marks;
# $count is always two or more.
files-replace-one = “{ $name }” já existe. Substituir?
files-replace-one-detail = Substituí-lo sobrescreve o conteúdo atual.
files-replace-many = { $count } destes arquivos já existem. Substituí-los?
files-replace-many-detail = Substituí-los sobrescreve o conteúdo atual.


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

files-date-modified = { $day } de { $month } de { $year } às { $time }

files-month-jan = Jan
files-month-feb = Fev
files-month-mar = Mar
files-month-apr = Abr
files-month-may = Mai
files-month-jun = Jun
files-month-jul = Jul
files-month-aug = Ago
files-month-sep = Set
files-month-oct = Out
files-month-nov = Nov
files-month-dec = Dez


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
settings-not-set = Não definido
# The button that opens the file picker, and the field beside it before a file
# has been chosen.
settings-choose = Escolher…
settings-no-file-chosen = Nenhum arquivo escolhido
settings-choose-background-image = Escolher uma imagem de plano de fundo

# The settings window's title bar. $pane is the selected pane's name.
settings-window-title = Configurações do Otto — { $pane }


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
schema-screen-scale-label = Escala do monitor
schema-screen-scale-description = Fator de escala global aplicado à área de trabalho.
schema-theme-scheme-label = Esquema de cores
schema-theme-scheme-description = Esquema de cores claro ou escuro.
schema-accent-color-label = Cor de destaque
schema-accent-color-description = Um nome da paleta, que acompanha os esquemas claro e escuro, ou uma cor #RRGGBB.
schema-rounded-corners-label = Cantos arredondados
schema-rounded-corners-description = O Dock, a barra superior, as decorações de janela e os painéis da área de trabalho.
schema-window-controls-side-label = Controles da janela
schema-window-controls-side-description = Em que extremidade da barra de título ficam os controles de fechar, minimizar e ampliar.
schema-show-maximize-button-label = Botão de maximizar
schema-show-maximize-button-description = Mostra o controle de ampliar na barra de título de uma janela. Desativado por padrão: um clique duplo na barra de título amplia a janela de qualquer forma.
schema-font-family-label = Fonte da interface
schema-font-family-description = Família de fonte usada pela própria interface do Otto.
schema-background-color-label = Cor do plano de fundo
schema-background-color-description = Cor do plano de fundo da área de trabalho, como uma string hexadecimal.
schema-background-image-label = Imagem do plano de fundo
schema-background-image-description = Caminho da imagem de plano de fundo da área de trabalho. Vazio para nenhuma.
schema-cursor-theme-label = Tema do cursor
schema-cursor-theme-description = Nome do tema XCursor.
schema-cursor-size-label = Tamanho do cursor
schema-cursor-size-description = Tamanho do cursor em pixels lógicos.
schema-icon-theme-label = Tema de ícones
schema-icon-theme-description = Nome do tema de ícones. Vazio para detecção automática.
schema-gtk-theme-label = Tema do GTK
schema-gtk-theme-description = Nome do tema GTK repassado aos clientes. Vazio para detecção automática.
schema-locales-label = Idiomas
schema-locales-description = Idiomas preferidos, do mais preferido ao menos.

# --- dock ---
schema-dock-size-label = Tamanho
schema-dock-size-description = Multiplicador do tamanho do Dock.
schema-dock-position-label = Posição na tela
schema-dock-position-description = Borda da tela em que o Dock fica.
schema-dock-autohide-label = Ocultar automaticamente
schema-dock-autohide-description = Ocultar o Dock até que o ponteiro alcance sua borda da tela.
schema-dock-magnification-label = Ampliação
schema-dock-magnification-description = Aumentar os ícones sob o ponteiro.
schema-dock-genie-scale-label = Nível de ampliação
schema-dock-genie-scale-description = Quanto os ícones sob o ponteiro aumentam.
schema-dock-genie-span-label = Alcance da ampliação
schema-dock-genie-span-description = Quantos ícones vizinhos a ampliação alcança.
schema-dock-colorize-icons-label = Colorir ícones
schema-dock-colorize-icons-description = Colorir os ícones do Dock com uma única cor.
schema-dock-colorize-color-label = Cor dos ícones
schema-dock-colorize-color-description = Cor usada para colorir os ícones do Dock, como uma string hexadecimal.
schema-dock-colorize-intensity-label = Intensidade da cor dos ícones
schema-dock-colorize-intensity-description = Com que intensidade a cor é aplicada.

# --- general ---
schema-keyboard-repeat-delay-label = Atraso de repetição de tecla
schema-keyboard-repeat-delay-description = Milissegundos que uma tecla é mantida pressionada antes de começar a se repetir.
schema-keyboard-repeat-rate-label = Taxa de repetição de tecla
schema-keyboard-repeat-rate-description = Repetições por segundo enquanto uma tecla é mantida pressionada.

# --- input ---
schema-input-xkb-layout-label = Layout do teclado
schema-input-xkb-layout-description = Nome do layout XKB. Vazio usa o padrão do sistema.
schema-input-xkb-variant-label = Variante do teclado
schema-input-xkb-variant-description = Nome da variante XKB. Vazio usa o padrão do sistema.
schema-input-xkb-options-label = Opções do teclado
schema-input-xkb-options-description = Strings de opções XKB.
schema-input-tap-enabled-label = Tocar para clicar
schema-input-tap-enabled-description = Tratar um toque no trackpad como um clique.
schema-input-tap-drag-enabled-label = Tocar e arrastar
schema-input-tap-drag-enabled-description = Iniciar um arraste a partir de um toque seguido de contato mantido.
schema-input-tap-drag-lock-enabled-label = Bloqueio de arraste
schema-input-tap-drag-lock-enabled-description = Manter um toque-arraste durante um breve levantamento do dedo.
schema-input-touchpad-click-method-label = Método de clique
schema-input-touchpad-click-method-description = Se um clique é determinado pela quantidade de dedos ou por áreas de botão.
schema-input-touchpad-dwt-enabled-label = Desativar ao digitar
schema-input-touchpad-dwt-enabled-description = Ignorar o trackpad enquanto o teclado está em uso.
schema-input-touchpad-natural-scroll-enabled-label = Rolagem natural
schema-input-touchpad-natural-scroll-enabled-description = O conteúdo acompanha os dedos.
schema-input-touchpad-left-handed-label = Canhoto
schema-input-touchpad-left-handed-description = Trocar os botões principal e secundário.
schema-input-touchpad-middle-emulation-enabled-label = Emulação de clique do meio
schema-input-touchpad-middle-emulation-enabled-description = Pressionar os dois botões juntos equivale a um clique do meio.
schema-input-scroll-speed-label = Velocidade de rolagem
schema-input-scroll-speed-description = Multiplicador de software aplicado aos eventos de rolagem.
schema-input-pointer-accel-speed-label = Velocidade do ponteiro
schema-input-pointer-accel-speed-description = Aceleração do ponteiro, de -1 (mais lento) a 1 (mais rápido).
schema-input-pointer-accel-profile-label = Aceleração do ponteiro
schema-input-pointer-accel-profile-description = Constante é a velocidade bruta; adaptativa segue a curva do libinput.

# --- audio ---
schema-audio-sound-enabled-label = Sons da interface
schema-audio-sound-enabled-description = Reproduzir um retorno sonoro para eventos da interface.
schema-audio-sound-theme-label = Tema de som
schema-audio-sound-theme-description = Nome do tema de som XDG. Vazio para detecção automática.

# --- power_management ---
schema-power-management-manage-lid-switch-label = Controlar o interruptor da tampa
schema-power-management-manage-lid-switch-description = Deixar o Otto agir sobre a tampa em vez de deixar isso a cargo do logind.
schema-power-management-on-lid-close-label = Ao fechar a tampa
schema-power-management-on-lid-close-description = O que acontece quando a tampa do notebook é fechada.
schema-power-management-on-power-button-label = Ao pressionar o botão de energia
schema-power-management-on-power-button-description = O que acontece quando o botão físico de energia é pressionado.

# --- lock ---
schema-lock-locker-command-label = Comando da tela de bloqueio
schema-lock-locker-command-description = O bloqueador iniciado para bloquear a sessão.
schema-lock-locker-args-label = Argumentos da tela de bloqueio
schema-lock-locker-args-description = Argumentos passados ao bloqueador.
schema-lock-auto-lock-timeout-label = Bloquear após
schema-lock-auto-lock-timeout-description = Segundos de inatividade antes de bloquear. 0 nunca bloqueia.

# --- login ---
schema-login-greeter-command-label = Comando da tela de login
schema-login-greeter-command-description = O programa da tela de login iniciado no modo de login.
schema-login-greeter-args-label = Argumentos da tela de login
schema-login-greeter-args-description = Argumentos passados à tela de login.

# --- appswitcher ---
schema-appswitcher-follow-cursor-label = O alternador segue o ponteiro
schema-appswitcher-follow-cursor-description = Mostrar o alternador de aplicativos na saída em que o ponteiro está.
schema-appswitcher-colorize-icons-label = Colorir ícones do alternador
schema-appswitcher-colorize-icons-description = Aplicar a cor dos ícones do Dock também ao alternador de aplicativos. Não faz nada enquanto a cor do Dock está desligada.


## Late additions

# The auto-detect entry in a theme pop-up, offered when no theme is set.
settings-choice-automatic = Automático
settings-choice-system-language = Idioma do sistema


## Accent colour names
##
## The named accents Otto offers. Colour names, translated the way the
## platform names colours — not invented.

settings-choice-accent-blue = Azul
settings-choice-accent-purple = Roxo
settings-choice-accent-pink = Rosa
settings-choice-accent-red = Vermelho
settings-choice-accent-orange = Laranja
settings-choice-accent-yellow = Amarelo
settings-choice-accent-green = Verde
settings-choice-accent-mint = Menta
settings-choice-accent-teal = Azul-petróleo
settings-choice-accent-cyan = Ciano
settings-choice-accent-indigo = Índigo
settings-choice-accent-brown = Marrom
settings-choice-accent-graphite = Grafite
# The button under the shortcut list that adds another line.
settings-add-shortcut = Adicionar atalho
# A workspace nobody has named, in the switcher and expose. `number` counts
# from 1. Follow the platform's own word for a virtual desktop.
workspace-numbered = Área de Trabalho { $number }


## Launcher

# What the search field says when empty. It names the mode, because the
# launcher has three and the field is the only thing that says which is up.
launcher-search-everything = Pesquisar aplicativos e janelas…
launcher-search-apps = Pesquisar aplicativos…
launcher-search-windows = Pesquisar janelas…

# The badge on a result row, saying what kind of thing it is. Very short —
# it sits in a small pill beside the result.
launcher-badge-app = App
launcher-badge-window = Janela
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
auth-enter-password = Inserir senha

# Stands in for the person's name above the field when nobody has been
# identified yet — the greeter before a username is typed. Drawn 22pt bold and
# centred on a 380pt card; two or three words at most.
auth-sign-in = Fazer login

# strftime pattern for the time in the large clock above the login/lock card,
# not prose: only the %-codes and the separators between them are yours.
# Change it where the local convention differs — %-I:%M %p for a 12-hour
# locale. The digits render at 46pt, so keep the result short.
auth-clock-time-format = %H:%M

# strftime pattern for the date under that clock, again not prose. Reorder the
# parts and change the punctuation to suit the locale (German would be
# "%A, %-d. %B"); the weekday and month names are translated by the system, so
# do not spell them out here. Renders at 15pt in a 360pt box.
auth-clock-date-format = %A, %-d de %B

### otto-greeter — the login screen shown before any session exists.
### Everything here is drawn on the login card, centred on the wallpaper.
### The card is narrow: prompts sit above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field while the login screen is asking who is logging
# in. One line, above a text field roughly 20 characters wide — keep it to one
# or two words.
greeter-prompt-username = Nome de usuário

# Label above the input field once a password is what is being asked for.
# Replaces the username label in the same place, same width.
greeter-prompt-password = Senha

# Label above the field during the pause between the username being submitted
# and the login service asking its first question. It replaces the prompt, so
# it must fit the same one-line slot. Ends in an ellipsis: work is in progress.
greeter-prompt-authenticating = Autenticando…

# Status line under the field: the login service (greetd) could not be reached
# or stopped responding mid-login. { $error } is the operating system's own
# error text and arrives in English. The line is clipped, not wrapped, so keep
# the part before the error short.
greeter-error-service-unavailable = Serviço de login indisponível: { $error }

# Status line under the field: the login service closed the connection while a
# login was in progress. The screen has returned to the username field.
greeter-error-service-gone = O serviço de login encerrou a conexão

# Status line under the field: the session was asked to start and the login
# screen is still here several seconds later, so the session did not launch.
# { $session } is the session's own name from its .desktop file ("Otto",
# "GNOME") and is never translated.
greeter-error-session-did-not-start = A sessão { $session } não iniciou

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the session starts. One short line.
greeter-status-authenticated = Autenticado

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
greeter-status-place-finger = Coloque o dedo no leitor

# As above, for a swipe reader rather than one you rest a finger on.
greeter-status-swipe-finger = Deslize o dedo no leitor

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names, in the middle of the sentence
# — reorder the line freely, but keep it to the same one clipped line. The
# lock screen says the same thing in lock-status-*-named-finger; the two are
# separate keys because the two screens are separate places.
greeter-status-place-named-finger = Coloque { $finger } no leitor
greeter-status-swipe-named-finger = Deslize { $finger } no leitor

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
greeter-status-no-match = Impressão digital não reconhecida

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
greeter-status-waiting-for-reader = Aguardando o leitor de digitais…

# Replaces the input field entirely once the login has succeeded and the
# session is being launched. Centred on the card, one short line.
greeter-status-starting-session = Iniciando a sessão…

# Status line under the field: the system refused the suspend request from the
# login screen (a policy decision, not a failure). One short line.
greeter-power-suspend-denied = Sem permissão para suspender

# As above, for the restart request.
greeter-power-restart-denied = Sem permissão para reiniciar

# As above, for the shut down request.
greeter-power-shutdown-denied = Sem permissão para desligar

# Status line under the field: the suspend request could not be run at all —
# the system tool behind it is missing or failed to launch. One short line.
greeter-power-suspend-failed = Não foi possível suspender

# As above, for the restart request.
greeter-power-restart-failed = Não foi possível reiniciar

# As above, for the shut down request.
greeter-power-shutdown-failed = Não foi possível desligar

### otto-lock — the lock screen shown over a running session.
### Everything here is drawn on the unlock card, centred on each screen.
### The card is narrow: the prompt sits above the input field and status lines
### sit under it, both on a single line that is clipped rather than wrapped.

# Label above the input field on the lock screen. Also the fallback when the
# authentication stack asks a question with no readable text of its own.
# One line, above a field roughly 20 characters wide — one or two words.
lock-prompt-password = Senha

# Status line under the fingerprint mark once a fingerprint has been
# recognised, just before the screen unlocks. One short line.
lock-status-authenticated = Autenticado

# Status line under the fingerprint mark while the reader is waiting for a
# finger, when the module did not say which finger it wants. One line, clipped
# at roughly 40 characters.
lock-status-place-finger = Coloque o dedo no leitor

# As above, for a swipe reader rather than one you rest a finger on.
lock-status-swipe-finger = Deslize o dedo no leitor

# As the two above, but the reader named the finger it has enrolled.
# { $finger } is one of the auth-finger-* names below, in the middle of the
# sentence — reorder the line freely, but keep it to the same one clipped line.
lock-status-place-named-finger = Coloque { $finger } no leitor
lock-status-swipe-named-finger = Deslize { $finger } no leitor

# The ten fingers a fingerprint reader can ask for by name, as they appear
# inside the two lines above and nowhere else. Lower case, no article: the
# sentence supplies it. If the local grammar needs an article or a possessive
# glued to the name, move it out of the sentence and into these instead.
auth-finger-left-thumb = o polegar esquerdo
auth-finger-left-index = o indicador esquerdo
auth-finger-left-middle = o dedo médio esquerdo
auth-finger-left-ring = o anelar esquerdo
auth-finger-left-little = o dedo mínimo esquerdo
auth-finger-right-thumb = o polegar direito
auth-finger-right-index = o indicador direito
auth-finger-right-middle = o dedo médio direito
auth-finger-right-ring = o anelar direito
auth-finger-right-little = o dedo mínimo direito

# Status line under the fingerprint mark when the reader looked at a finger and
# did not recognise it. The reader asks again straight afterwards, so this is a
# statement, not an instruction. One line.
lock-status-no-match = Impressão digital não reconhecida

# Status line under the field when a password has been typed and submitted but
# the fingerprint reader still holds the conversation, so nothing can be sent
# yet. Tells the user the delay is the reader, not a failure. One line.
lock-status-waiting-for-reader = Aguardando o leitor de digitais…

# Status line under the field: the lock screen could not work out whose
# session it is locking, so there is no account to authenticate against.
# Rare, and not recoverable from the lock screen. One line.
lock-error-no-user = Nenhum usuário para autenticar

# Status line under the field: the authentication stack stopped answering
# part-way through an attempt. The card offers another try afterwards.
lock-error-service-failed = Falha no serviço de autenticação

# Status line under the field: the account name of the locked session cannot
# be used for authentication (it contains something the stack rejects).
lock-error-invalid-user = Nome de usuário inválido

# Status line under the field: the authentication stack could not be started
# at all, so no password can be checked.
lock-error-unavailable = A autenticação está indisponível

# Status line under the field: an attempt failed and the authentication stack
# gave no reason. { $status } is its numeric result code, shown so a support
# request has something to quote; do not translate it.
lock-error-auth-failed = Falha na autenticação ({ $status })

# Status line under the field: the system refused the suspend request from the
# lock screen (a policy decision, not a failure). One short line.
lock-power-suspend-denied = Sem permissão para suspender

# As above, for the restart request.
lock-power-restart-denied = Sem permissão para reiniciar

# As above, for the shut down request.
lock-power-shutdown-denied = Sem permissão para desligar

# Status line under the field: the suspend request could not be run at all.
# { $error } is the operating system's own error text and arrives in English.
# The line is clipped, not wrapped, so keep the part before the error short.
lock-power-suspend-failed = Não foi possível suspender: { $error }

# As above, for the restart request.
lock-power-restart-failed = Não foi possível reiniciar: { $error }

# As above, for the shut down request.
lock-power-shutdown-failed = Não foi possível desligar: { $error }


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
quickview-fact-size = Tamanho
# Column heading for an image's or a video's pixel dimensions. Max ~12 characters.
quickview-fact-dimensions = Dimensões
# Column heading for a video's or an audio track's running time. Max ~12 characters.
quickview-fact-duration = Duração
# Column heading for an image's total pixel count, in megapixels. Max ~12 characters.
quickview-fact-pixels = Pixels
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
quickview-fact-year = Ano

# Subtitle of the card for a file that is zero bytes long. Shown under the
# file's name in place of its type.
quickview-empty-file = Arquivo vazio
# Subtitle for an image with too many pixels to decode safely. Its real
# dimensions are still listed below it.
quickview-image-too-large = Muito grande para visualizar
# The value beside "Pixels" on that card. $count is a whole number of
# megapixels.
quickview-megapixels =
    { $count ->
        [one] { $count } megapixel
        [many] { $count } megapixels
       *[other] { $count } megapixels
    }
# Subtitle for a PDF when no page rasteriser is installed. $packages is a
# comma-separated list of package names — pdftoppm's package and so on — and
# is not translated. Wraps to two lines if it has to.
quickview-pdf-install-rasteriser = Para ver as páginas, instale um destes: { $packages }


## Quick View — listings
##
## A folder or an archive is previewed as a list of what is inside, with one
## summary line under it.

# Summary line for a folder with nothing in it.
quickview-empty-folder = Pasta vazia
# Summary line for a folder or archive: how many entries it holds. Hidden
# entries are counted.
quickview-item-count =
    { $count ->
        [one] { $count } item
        [many] { $count } itens
       *[other] { $count } itens
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
quickview-error-not-previewable = este não é um arquivo que possa ser visualizado
# The file's metadata could not be read.
quickview-error-stat-file = não é possível obter os dados do arquivo: { $error }
# The file's bytes could not be read. Also used by the text previewer.
quickview-error-read-file = não é possível ler o arquivo: { $error }
# The file cannot be rewound, so it cannot be identified and then read.
quickview-error-not-seekable = o arquivo não permite reposicionamento
# The worker refused to parse the file because it could not confine itself
# first. Parsing an untrusted file uncontained is not something Otto does.
quickview-error-sandbox = não foi possível isolar o visualizador: { $error }

# Image previewer.
quickview-error-read-image = não é possível ler a imagem: { $error }
# The bytes are an image format this build has no decoder for.
quickview-error-image-unsupported = esta compilação não decodifica este formato de imagem
quickview-error-image-no-size = a imagem não informa seu tamanho
quickview-error-image-decode = a imagem não foi decodificada: { $error }
quickview-error-image-readback = não foi possível reler a imagem decodificada

# SVG previewer. "the drawing" means the SVG, as distinct from a photograph.
quickview-error-read-drawing = não é possível ler o desenho: { $error }
quickview-error-drawing-parse = não foi possível interpretar o desenho
quickview-error-drawing-surface = nenhuma superfície para exibi-lo
quickview-error-drawing-readback = não foi possível reler o desenho

# Text previewer: the bytes are not text in UTF-8 or in Latin-1.
quickview-error-not-text = este arquivo não é texto em nenhuma codificação que o Otto lê

# PDF previewer.
quickview-error-read-document = não é possível ler o documento: { $error }
quickview-error-page-readback = não foi possível ler a página renderizada

# Folder listing.
quickview-error-read-folder = não é possível ler a pasta

## The worker process itself failed. "the previewer" is the separate program
## that parses the file; a person never sees it by name anywhere else, so
## describing it as "the previewer" rather than naming it is deliberate.

quickview-error-previewer-missing = não é possível encontrar o visualizador: { $error }
quickview-error-previewer-start = não é possível iniciar o visualizador: { $error }
quickview-error-previewer-no-output = o visualizador não produziu nada
quickview-error-previewer-unreadable = o visualizador produziu algo ilegível
quickview-error-previewer-failed = o visualizador falhou: { $error }
# The worker was still going after the deadline and was killed.
quickview-error-timeout = este arquivo demorou demais para ser visualizado

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
islands-close = Fechar

# Age of a notification, shown at the bottom right of its card. Under a
# minute old.
islands-elapsed-just-now = agora mesmo
# Age of a notification between one minute and an hour old. $count is whole
# minutes. English abbreviates hard ("5m ago") because there is no room for
# more; keep it to about 7 characters.
islands-elapsed-minutes = há { $count } min
# Age of a notification an hour or more old. $count is whole hours. Same
# width constraint as above.
islands-elapsed-hours = há { $count } h


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
islands-dialog-deny = Negar


## Accessibility
##
## Spoken by a screen reader, never drawn on screen, so these are the only
## strings in the catalogue with no width limit — say the whole thing rather
## than abbreviating. They name parts of the desktop a sighted person
## recognises by shape: read them as answers to "what is this?".

a11y-dock = Dock
a11y-app-running = Em execução
a11y-app-not-running = Não está em execução
a11y-app-switcher = Alternador de aplicativos
a11y-windows = Janelas
a11y-workspaces = Áreas de trabalho
a11y-untitled-window = Janela sem título
a11y-menu-bar = Barra de menus
a11y-status = Estado
a11y-tray-item = Item { $number }
a11y-notifications = Notificações
a11y-categories = Categorias
a11y-results = Resultados
a11y-settings = Ajustes
a11y-preview = Pré-visualização
a11y-preview-page = Pré-visualização, página { $page } de { $pages }
a11y-preview-shortened = Pré-visualização, abreviada
