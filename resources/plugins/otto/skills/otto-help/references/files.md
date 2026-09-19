# Adding a command to Otto Files

A Files command is an executable that answers three questions in JSON:

```
script describe              → { "commands": [ … ] }        once, when a window opens
script preview < request     → { "rows": [ … ], "note": … } after every keystroke
script run     < request     → { "status", "changes", "reload" }
```

Follow the steps in order. **Do not write a script from scratch** — start from
the starter. **Do not copy, move or `chmod` files yourself** — the
`files-command` tool in this skill does all of that:

| Command | Does |
|---|---|
| `<this skill>/scripts/files-command new NAME` | Starts a draft from the starter and prints its path |
| `<this skill>/scripts/files-command edit NAME` | Starts a draft from a command already published |
| `<this skill>/scripts/files-command check NAME` | Runs the draft's `describe` and checks it prints JSON |
| `<this skill>/scripts/files-command review NAME` | Opens the draft in the person's own editor, to read |
| `<this skill>/scripts/files-command publish NAME` | Moves the draft, executable, to where Files reads it |
| `<this skill>/scripts/files-command list` | Shows drafts and published commands |

`<this skill>` is the folder this skill's `SKILL.md` is in; write it as an
absolute path.

`check` and running a draft by hand (Step 5) run a script you wrote, so unlike
the rest they are not pre-approved: the harness will ask the person first. That
is deliberate. Expect the prompt, and say what you are about to run and why
rather than treating it as a failure.

A draft lives in `~/.local/state/otto/files-drafts/`, where Files never looks,
so a half-finished command never appears in a window.

## Step 1 — ask three questions

You cannot write the script without these answers. Ask them before writing
anything.

1. **What should it do to the files?** "Convert to PNG" could mean replace the
   original, put the new file beside it, or put it in a subfolder. The answer
   decides whether the command can be undone.
2. **When should it appear?** Any file, several files, only `.jpg`, only a
   folder?
3. **Does it need something typed in?** A new name, a suffix, a size. If yes,
   the command opens a text field first.

## Step 2 — start a draft

```sh
<this skill>/scripts/files-command new NAME
```

Replace `NAME` with a short lower-case name: letters, digits and `-`, starting
with a letter. It prints the draft's path. To change a command that already
exists, use `edit NAME` instead of `new NAME`.

The starter is a working command called **Add Prefix**: it asks for text, shows
a live preview, refuses when a name is taken, and can be undone.

## Step 3 — check it runs

```sh
<this skill>/scripts/files-command check NAME
```

It must print `ok`. If it prints an error, fix that before going on.

## Step 4 — change these four things

Edit the draft — the path Step 2 printed — in this order:

1. **`STRINGS`** — the English text. `title` is what appears in the palette.
2. **`"id"`** in `describe` — a short name for the command, lower case.
3. **`"when"`** in `describe` — from your Step 1 answer:
   - any number of files: `{"targets": "some"}`
   - exactly one: `{"targets": "one"}`
   - only certain types: `{"targets": "some", "extensions": ["jpg", "png"]}`
   - only a folder: `{"targets": "one", "kinds": "folders"}`
4. **`plan`, `preview` and `run`** — what the command actually does.

If the command needs nothing typed in, delete the `"arg"` block from `describe`
and delete the `preview` function and its `elif` branch.

## Step 5 — test it by hand, on files you do not care about

```sh
mkdir -p /tmp/demo && cd /tmp/demo && touch a.txt b.txt

cat > /tmp/request.json <<'JSON'
{ "command": "add-prefix", "arg": "2026-", "locale": "en-GB",
  "targets": ["/tmp/demo/a.txt", "/tmp/demo/b.txt"],
  "situation": { "path": "/tmp/demo", "siblings": ["a.txt", "b.txt"],
                 "cursor_name": "a.txt", "cursor_is_dir": false } }
JSON

~/.local/state/otto/files-drafts/NAME describe
~/.local/state/otto/files-drafts/NAME preview < /tmp/request.json
~/.local/state/otto/files-drafts/NAME run     < /tmp/request.json
```

Change `"command"` to your own id. Test a conflict too — a file whose new name
already exists — and check the script refuses instead of overwriting.

**Never test on the person's real files.**

## Step 6 — offer a review

Ask: "The command is ready. Do you want to read it before I install it?"

- **Yes** → run this, then wait until they say to go on or ask for changes:

  ```sh
  <this skill>/scripts/files-command review NAME
  ```

  If they ask for changes, make them in the draft and go back to Step 5.
- **No** → go to Step 7.

## Step 7 — publish it

```sh
<this skill>/scripts/files-command publish NAME
```

It replaces a published command of the same name. Then tell them: open a **new** Files window, press `Ctrl+P`, type the first few
letters of the title, press Return. The command is also in the right-click menu,
and `Ctrl+Z` undoes it.

## The rules a script must follow

Breaking one of these is what makes a command misbehave.

1. **A `preview` must not change anything.** It runs after every keystroke.
2. **`preview` and `run` must agree.** Write one `plan` function that says what
   would happen, and call it from both. The starter does this.
3. **Each preview row's `from` must be the file's own name** (`a.txt`, not the
   full path). That is how skipping a file finds its row.
4. **Check every destination before changing the first file.** A run that fails
   half way leaves work that Undo cannot reverse. Refuse up front.
5. **`"conflict": true` does not stop anything.** It only colours the row red.
   The `run` must refuse for itself.
6. **Report changes so Undo works.** `{"kind": "created", "path": "…"}` for a
   new file, `{"kind": "moved", "from": "…", "to": "…"}` for a rename or a move.
   Nothing else can be undone — an edit in place or a deletion cannot.
7. **Fail by exiting non-zero.** The last line on stderr is what the person
   reads. In Python: `sys.exit("That name is already taken")`.
8. **Use the absolute paths from the request.** Files does not set a working
   directory.
9. **Print JSON on stdout and nothing else.** One stray `print` breaks the
   command.
10. **Keep it fast.** `describe` has 3 seconds, `preview` has 0.6. A `run` may
    take as long as it likes.

## When the command does not appear

Check these in order:

1. Was a **new** window opened after the change? Scripts are read once, when a
   window opens.
2. Was it published? `<this skill>/scripts/files-command list` shows drafts
   and published commands; a draft is not in Files until `publish`.
3. Does `files-command check NAME` pass? (For a published command, `edit NAME`
   first.)
4. Does the `#!` line point at an interpreter that exists?
5. Does `"when"` match what is selected right now?
6. Is the window the Trash or Recent? Script commands are not offered there.

To see the reason, start Files from a terminal — it logs a warning naming the
script:

```sh
otto-files
```

## Writing the text people see

- `title`: two or three words, title case — **Add Prefix**, **Compress to Zip**.
  Add `…` at the end if it asks for something first.
- `prompt`: what the field is for — "Put in front of each name". Not "Please
  enter…".
- `status`: what happened, past tense — "Renamed 2 items".
- No exclamation marks, no emoji. British spelling.

## More detail

- [files/protocol.md](files/protocol.md) — every field of every call:
  `describe`, the request, `situation`, both replies, the undo kinds,
  translations.
- [files/starter](files/starter) — the script to copy.
- Otto's own scripts, for bigger examples: `components/otto-files/scripts/zip`
  (one archive from many files), `unzip` (`when.extensions`), `ask` (the
  smallest useful script — no preview).
- https://nongio.github.io/otto/files-custom-commands/
- https://nongio.github.io/otto/files-command-palette/
