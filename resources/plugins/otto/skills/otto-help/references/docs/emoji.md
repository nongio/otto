# Emoji Picker

`otto-emoji` is a card for finding an emoji and typing it: press a key, type
a few letters or browse a category, press Enter, and the emoji lands in
whichever window had the keyboard.

> **First version.** It searches by Unicode's names, browses by category,
> remembers what you picked and which skin tone you use. Keywords in other
> languages and custom shortcodes are not there yet.

## Opening it

The default binding is `Ctrl+.` — an ordinary shortcut in `otto_config.toml`
running `otto-emoji`, so you can rebind it like any other; see
[Keyboard Shortcuts](keyboard-shortcuts.md):

```toml
[keyboard_shortcuts]
"Ctrl+period" = { run = { cmd = "otto-emoji", args = [] } }
```

The card appears beside the text cursor when the application you are typing in
says where it is, with a point aimed at it, and in the middle of the screen
when it doesn't. It picks the side with room, above or below and left or
right. Reporting
the cursor is optional and plenty of programs never do, terminals especially,
so expect both.

While the picker is up it takes the keyboard exclusively: every keystroke goes
to it, not to the window underneath. Anything given on the command line is the
query it starts with, so a binding can open it already narrowed.

## Using it

Type to search. Every word you type has to begin a word of the emoji's name —
`smil cat` finds the smiling cats — and a name that starts with what you typed
ranks above one that merely contains it. Category names work too: `flags`
lists every flag.

With nothing typed, each category is a panel of its own, side by side. Scroll
sideways with two fingers to move between categories and scroll up and down
inside one, the same way the file manager's column view works. Let go mid-swipe
and it keeps going, and settles onto whichever category it lands on. Your
recent picks are the first panel, to the left of Smileys.

The tab strip under the field jumps straight to a category, and its marker
follows as you swipe.

| Keys | Effect |
|------|--------|
| Letters | Search |
| `Left` / `Right` | Move the selection, crossing into the next category at either end (edits the query once something is typed) |
| `Up` / `Down` | Move a row within the category |
| `Page Up` / `Page Down` | Move by a screenful |
| `Tab` / `Shift+Tab` | Next or previous category |
| `Home` / `End` | First or last emoji of the category |
| `Enter` or a click | Type the selected emoji and close |
| `Escape` or a click outside | Close without typing |

The footer names the emoji under the selection, and the six dots on its
right are the skin tone: click one and every emoji that takes a tone changes
to it. The choice is kept between runs.

The query field takes the usual editing keys: select-all with `Ctrl+A`, delete
the previous word with `Ctrl+W`, clear with `Ctrl+U`, and copy, cut and paste
with `Ctrl+C`, `Ctrl+X` and `Ctrl+V`.

## How the emoji gets there

Wayland gives one application no way to write into another, so the picker does
not try. Once its card is gone and the keyboard has returned to the window it
came from, the picker acts as a keyboard for a moment: it presses one key per
character of the emoji on a keymap of its own, and the window receives the
characters exactly as it would from a physical keyboard. This works in every
toolkit and terminal that takes keyboard input, native or X11.

If you would rather have the emoji on the clipboard, run `otto-emoji --copy`:
the picker then copies the pick and waits, invisibly, until you paste it or
copy something else.

## Recently used

The last thirty picks are kept in `~/.local/state/otto/emoji-recent`, one per
line, and the skin tone in `emoji-tone` beside it. Delete either file to start
afresh.

## Not there yet

- Keywords beyond Unicode's names: `:thumbsup:`, translations, synonyms.
- A scrollbar on the panels: they scroll, but nothing yet shows how far.
- Following the text cursor in applications that never report one, which is
  most terminals and everything running under Xwayland.
- Choosing a tone for one emoji without changing the setting.
- Skin tones for emoji showing two people with different tones.
- Emoji the installed font cannot draw are left out rather than shown as
  boxes; the palette is only as complete as the font.
