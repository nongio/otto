# Configuring Otto

Otto's settings are changed by calling D-Bus. Follow the steps below in order.
Do not improvise commands: every command you need is written out somewhere in
these pages, ready to copy.

## Step 1 — check Otto is running

Run this:

```sh
busctl --user list | grep org.otto.Settings
```

- **A line comes back** → go to Step 2.
- **Nothing comes back** → Otto is not running. Say: "Otto does not seem to be
  running, so I cannot change settings live." Then stop, or read
  [configure/config-file.md](configure/config-file.md) if they want to edit
  the file instead.

## Step 2 — find the identifier

Every setting has an identifier such as `dock.size`. Find the one you need in
the table below, then open that page and read it. **Read one page, not all of
them.**

| The request is about | Open this page |
|---|---|
| Dark mode, accent colour, wallpaper, fonts, cursors, icons, rounded corners, frosting, interface language | [configure/appearance.md](configure/appearance.md) |
| Monitors, resolution, refresh rate, arrangement, scale, panel zones | [configure/displays.md](configure/displays.md) |
| A screen with no monitor behind it, for recording or remote desktop | [configure/virtual-outputs.md](configure/virtual-outputs.md) |
| Keyboard layout, xkb options, key repeat, the Cmd key | [configure/keyboard.md](configure/keyboard.md) |
| Binding or rebinding keys, the list of actions, an i3-style tiling set | [configure/shortcuts.md](configure/shortcuts.md) |
| Tap to click, scrolling, pointer speed, left-handed | [configure/touchpad.md](configure/touchpad.md) |
| Dock size, edge, autohide, magnification, pinned apps | [configure/dock.md](configure/dock.md) |
| Tiled layouts, gaps, tiled window chrome | [configure/tiling.md](configure/tiling.md) |
| Locking, idle lock, fingerprint, which locker runs | [configure/lock-screen.md](configure/lock-screen.md) |
| The login screen, greetd, which greeter runs | [configure/greeter.md](configure/greeter.md) |
| The laptop lid, clamshell, the power button, suspend | [configure/lid-and-power.md](configure/lid-and-power.md) |
| Starting a program with the session, the top bar or islands missing | [configure/autostart-commands.md](configure/autostart-commands.md) |
| `.desktop` autostart entries an application installed | [configure/autostart-xdg.md](configure/autostart-xdg.md) |
| Interface sounds, the app switcher, workspaces, accessibility | [configure/system.md](configure/system.md) |
| Which file gets written, why a change did not stick | [configure/config-file.md](configure/config-file.md) |

If no page matches, list everything and search it:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings GetAll
```

## Offer choices; do not guess

Most requests leave something open. When they do, **ask with the question
tool** — `AskUserQuestion` on Claude, `request_user_input` on Codex, whatever
the harness calls its question form — and give the person something to pick.
Guessing and then telling them what you did is the wrong way round: they
cannot see the settings, so every guess is theirs to undo.

Ask when:

- the request could mean two settings ("make it bigger" — the dock, the
  pointer, the text?);
- it names a setting but no value ("change the wallpaper");
- it is broad ("set up my keyboard", "sort out my second monitor");
- a value is a choice and you would otherwise pick for them.

Do not ask when the request already names one setting and one value — "put the
dock on the left", "Caps Lock should be Escape". Do that, and say what changed.
Do not ask when they only want to know where a setting lives.

## The wizard

For anything broad, walk them through it one question at a time. Keep it
short: three questions is a wizard, ten is an interrogation.

1. **One decision per question.** Two to four choices, and each choice says
   what will happen.
2. **Offer a way out.** Include `Keep it as it is` when a question could
   reasonably be skipped, so nobody has to answer to escape.
3. **Say what costs something.** If a choice needs a restart, may suspend the
   machine, or turns off a protection, say so in the choice itself, not
   afterwards.
4. **Stay in the area they asked about.** An answer about the dock is not
   permission to touch the keyboard.
5. **Collect, then confirm.** Gather the answers, read the current values, then
   show one short list of what you are about to change and ask to go ahead.
   One answer, one setting: just do it.
6. **Common choices first, typing last.** For a layout name, a colour, a path
   or an exact number, offer the usual handful as choices, and only ask for
   free text if none fits.

### Words for the choices

The person reads these, so write them the way you would speak:

- A label is two to five plain words: `Bottom`, `Hide until I reach the edge`,
  `Suspend, and lock first`.
- A description is one short sentence about what they will notice: "The dock
  stays out of the way until the pointer touches the bottom of the screen."
- Never put a setting identifier, a type letter or a command in a label. The
  person does not need `dock.autohide`; they need "hide the dock".
- Say the current value when it helps them choose: "Bottom (where it is now)".

### Wizards worth having ready

Use the topic page for the exact identifiers, limits and commands. These are
good first questions; skip any the person has already answered.

| Topic | Ask | Choices |
|---|---|---|
| Appearance | "What would you like to change?" | Light or dark; accent colour; wallpaper; text and pointer size. |
| Light or dark | "How should the desktop look?" | Light; dark; keep it as it is. |
| Keyboard | "What would you like to set up?" | The layout; move a key somewhere else; how keys repeat; just show me the options. |
| Moving a key | "Which change do you want?" | Caps Lock becomes Escape; Caps Lock becomes Ctrl; swap Caps Lock and Escape; Cmd-style modifiers. |
| Key repeat | "How should a held key repeat?" | Balanced (300 ms, 30 a second); faster (250 ms, 40 a second); slower (500 ms, 20 a second); let me give exact numbers. |
| Dock | "What would you like to adjust?" | Where it sits and how big it is; whether it hides; icon tint; pinned apps. |
| Where the dock sits | "Where should the dock be?" | Bottom; left; right; keep it as it is. |
| How the dock behaves | "How should the dock behave?" | Always visible, icons grow on hover; hides until the pointer reaches the edge; always visible, no growing; keep it as it is. |
| Locking | "When should the screen lock?" | After 5 minutes; after 15 minutes; only when I ask; keep it as it is. |
| Lid and power button | "What would you like to set up?" | Closing the lid; the power button; using it with the lid shut; just show me the options. |
| Closing the lid | "What should closing the lid do?" | Suspend, and lock first; suspend; keep running with the screen off; leave it to the system. |
| The power button | "What should the power button do?" | Lock the screen; suspend; shut down; leave it to the system. |

Two of these need care. **Lid and power** only work if Otto is allowed to
receive the event, which is a system file change: ask about that on its own,
as `Let Otto handle it (needs a system file change you approve)` against
`Leave it to the system`, and never fold it into another question.
**Displays, shortcuts, pinned apps, autostart and virtual outputs** are not
settings and are written to a file, so every one of them needs a log out and
back in — say that in the choice.

## Step 3 — read the current value

Replace `dock.size` with your identifier:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Get s dock.size
```

The answer looks like `v d 1.25`. The letter after `v` is the **type**. You need
it for Step 4.

## Step 4 — set the new value

The command is always this shape:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    Set sv IDENTIFIER TYPE VALUE
```

Pick `TYPE VALUE` from this table. The type must match exactly — the wrong type
is an error, not a conversion.

| Type letter | Write it like this | Full example |
|---|---|---|
| `b` (true/false) | `b true` or `b false` | `Set sv dock.autohide b true` |
| `i` (whole number) | `i 32` | `Set sv cursor_size i 32` |
| `d` (decimal) | `d 1.25` — **always write a decimal point** | `Set sv dock.size d 1.25` |
| `s` (text, choice, colour) | `s "left"` — **always use quotes** | `Set sv dock.position s "left"` |
| `as` (list of text) | `as 1 "caps:escape"` — the number is how many items follow | `Set sv input.xkb_options as 1 "caps:escape"` |

Two rules that catch people out:

- `d 1` is **wrong** for a decimal setting. Write `d 1.0`.
- `i 1.5` is **wrong** for a whole-number setting. Round it.

## Step 5 — read what came back, and say it

The command prints one of two words:

| It printed | What happened | Say this |
|---|---|---|
| `s "applied"` | Changed now, and saved | "Done — the dock is on the left now." |
| `s "pending-restart"` | Saved, but **not** changed yet | "Saved. It takes effect the next time you log in." |

Never say a change has happened when you got `pending-restart`.

If it printed an error instead, nothing was saved. Find it here:

| Error contains | What it means | What to do |
|---|---|---|
| `no such setting` | The identifier is wrong | Check spelling against the page you read |
| `is double, got` / `is int, got` | Wrong type letter | Go back to Step 4's table |
| `must be >=` / `must be <=` | Number out of range | Use a number inside the range on the page |
| `must be one of` | Not a valid choice | Use one of the listed choices, spelled exactly |
| `Unsupported` | Cannot be changed on this machine | Say so; do not retry |

## Undoing a change

To put a setting back to normal, use `Reset` — not `Set` with the old value:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Reset s dock.size
```

## Common requests, ready to copy

Replace the numbers if the person asked for something different.

```sh
# Dark mode / light mode
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv theme_scheme s "Dark"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv theme_scheme s "Light"

# Accent colour (blue purple pink red orange yellow green mint teal cyan indigo brown gray)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv accent_color s "purple"

# Bigger or smaller dock (0.5 to 2.0)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.size d 1.25

# Move the dock (bottom, left, right)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.position s "left"

# Hide the dock until the pointer reaches the edge
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.autohide b true

# Wallpaper (an absolute path)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv background_image s "/home/me/Pictures/wall.jpg"

# Bigger pointer (16 to 96, in steps of 8)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv cursor_size i 32

# Caps Lock becomes Escape
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.xkb_options as 1 "caps:escape"

# Turn off tap-to-click
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.tap_enabled b false

# Reverse the scroll direction
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.touchpad_natural_scroll_enabled b false

# Lock the screen after 5 minutes idle (seconds; 0 never locks)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv lock.auto_lock_timeout i 300

# Turn off interface sounds
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv audio.sound_enabled b false
```

## When there is no identifier

Shortcuts, pinned dock apps, display profiles, autostart entries and virtual
outputs are **not** settings and have no `Set`. They are written in a
configuration file. The page for the topic shows exactly what to write. A file
change always needs a restart — logging out and back in.

## Opening the Settings app

If the person would rather click than be told, run:

```sh
otto-settings
```

It cannot be opened on a particular page, so tell them which one to click:
General, Displays, Dock, Keyboard, Trackpad & Mouse, Sound, Power, or Lock &
Login.

## Rules

Follow these exactly.

1. **Do not change anything the person did not ask for.** A wizard answer
   counts for the setting that question was about, and nothing else. Several
   answers: list them and ask to go ahead before applying any. Unsure which
   setting they mean: ask, with the two or three likely ones as choices.
2. **Ask before editing the configuration file by hand**, and always ask before
   changing `login.greeter_command`, `lock.locker_command` or anything under
   `power_management`. A wrong value there can stop the machine logging in.
3. **Do not run `otto --probe`.** It takes over the session.
4. **Do not suggest `Logo+Q`** to restart unless the person asks how to restart.
   It quits immediately and loses unsaved work.
5. **Do not invent identifiers, values or commands.** If it is not in this
   skill or in `Describe`, it does not exist.
6. **Report the result in one or two sentences.** Do not paste the commands you
   ran unless asked.

## How to write

- Short sentences. British spelling: colour, behaviour, minimise.
- Say what happened, plainly: "The dock is on the left now."
- No exclamation marks. No emoji. Do not congratulate anyone.
- Do not compare Otto to other desktops.
- If something needs a restart, say so in the same sentence.

## Documentation to share

| Topic | Link |
|---|---|
| The Settings app | https://nongio.github.io/otto/settings/ |
| Config files | https://nongio.github.io/otto/configuration/ |
| Good settings to start with | https://nongio.github.io/otto/recommended-settings/ |
| When something does not work | https://nongio.github.io/otto/troubleshooting/ |
| All the user guides | https://nongio.github.io/otto/ |
