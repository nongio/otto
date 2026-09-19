# Custom Commands in Files

You can add your own commands to Files. Write a small script, put it in
`~/.config/otto/files-scripts/`, and its commands show up in the
[command palette](files-command-palette.md) and the right-click menu alongside
the built-in ones. They get the same features:

- live previews while you type;
- skipping files in the preview;
- undo;
- a status line that says the command is running, while the window stays
  responsive.

A script is any executable file, in any language, that reads and writes JSON.
This page walks through building one, then describes everything a script can
say.

## Tutorial: a Duplicate command

We'll build **Duplicate**, a command that copies the selected files next to
themselves, as `photo copy.jpg`. It is about 70 lines of Python and uses only
the standard library.

### 1. Describe the command

Files runs every script in the folder once, when a window opens, with the
single argument `describe`. The script answers with the commands it offers.
Create `~/.config/otto/files-scripts/duplicate`:

```python
#!/usr/bin/env python3
import json
import os
import shutil
import sys


def describe():
    return {
        "commands": [
            {
                "id": "duplicate",
                "title": "Duplicate",
                "keywords": ["copy", "clone"],
                "when": {"targets": "some"},
            }
        ]
    }


verb = sys.argv[1] if len(sys.argv) > 1 else ""
if verb == "describe":
    reply = describe()
else:
    sys.exit("usage: duplicate describe|preview|run")
json.dump(reply, sys.stdout)
```

`"when": {"targets": "some"}` means the command is offered only when there is
something to act on. Make the file executable and check it:

```sh
chmod +x ~/.config/otto/files-scripts/duplicate
~/.config/otto/files-scripts/duplicate describe
```

### 2. Do the work

When you choose the command, Files runs the script with `run` and sends a
request as JSON on standard input. The request's `targets` field lists the
selected paths, or the item under the cursor if nothing is selected. The script
does the work and replies with what it changed. Add a `run` function and route
to it:

```python
def run(request):
    changes = []
    for source in request["targets"]:
        folder, name = os.path.split(source)
        stem, ext = os.path.splitext(name)
        destination = os.path.join(folder, stem + " copy" + ext)
        if os.path.exists(destination):
            sys.exit(f"“{os.path.basename(destination)}” is already there")
        if os.path.isdir(source):
            shutil.copytree(source, destination, symlinks=True)
        else:
            shutil.copy2(source, destination)
        changes.append({"kind": "created", "path": destination})
    return {
        "status": f"Duplicated {len(changes)} item(s)",
        "changes": changes,
        "reload": True,
    }
```

```python
elif verb == "run":
    reply = run(json.load(sys.stdin))
```

Three parts of the reply matter:

- **`changes`** lets **Undo** reverse the command. Undoing a `created` change
  moves that file to the Trash, so undo never deletes anything outright.
- **`reload`** refreshes the listing so the copies appear.
- **`status`** is the message shown in the status line.

If something goes wrong, exit with a non-zero status. Files shows the last
line the script wrote to standard error; in Python, `sys.exit("message")` does
both.

### 3. Try it

Open a new Files window, select a file and press `Ctrl+P`. Type `dup` and
press `Return`, and the copy appears. Press `Ctrl+Z` to move it to the Trash.
You can also right-click the file: **Duplicate** is in the menu.

### 4. Ask for a suffix, and show a preview

It would be better to choose the suffix and see the new names before anything
is copied. Add an `arg` to the description, so the command asks for text first:

```python
                "arg": {
                    "prompt": "Add to each name",
                    "label": "suffix",
                    "initial": " copy",
                    "preview": True,
                },
```

Because `preview` is true, Files runs the script with `preview` after every
keystroke. The request is the same one `run` gets, with what you have typed so
far in `arg`. A preview **must not change anything**. It replies with one row
per file, marking any name that is already taken as a conflict:

```python
def plan(request):
    """One (source, destination) per target, with the suffix added
    before the extension: "photo.jpg" -> "photo copy.jpg"."""
    suffix = request.get("arg") or " copy"
    out = []
    for source in request["targets"]:
        folder, name = os.path.split(source)
        stem, ext = os.path.splitext(name)
        out.append((source, os.path.join(folder, stem + suffix + ext)))
    return out


def preview(request):
    rows = []
    for source, destination in plan(request):
        rows.append({
            "from": os.path.basename(source),
            "to": os.path.basename(destination),
            "conflict": os.path.exists(destination),
        })
    taken = sum(row["conflict"] for row in rows)
    note = f"{taken} name already taken" if taken else None
    return {"rows": rows, "note": note}
```

Change `run` to use the same `plan`, so the preview and the result can't
disagree. The finished script is below.

Open a new window and choose **Duplicate** again. The field opens with ` copy`
in it, and the list shows `photo.jpg → photo copy.jpg` for each file, updating
as you type. Names that are already taken are shown in red.

Press `↓` and then `Space` on a row to leave that file out. Files then asks
your script for a preview of the smaller selection, and runs the command on
that selection too. The script doesn't need any code for this. It only needs
each row's `from` to be the file's name.

### The finished script

```python
#!/usr/bin/env python3
import json
import os
import shutil
import sys


def describe():
    return {
        "commands": [
            {
                "id": "duplicate",
                "title": "Duplicate",
                "keywords": ["copy", "clone"],
                "when": {"targets": "some"},
                "arg": {
                    "prompt": "Add to each name",
                    "label": "suffix",
                    "initial": " copy",
                    "preview": True,
                },
            }
        ]
    }


def plan(request):
    """One (source, destination) per target, with the suffix added
    before the extension: "photo.jpg" -> "photo copy.jpg"."""
    suffix = request.get("arg") or " copy"
    out = []
    for source in request["targets"]:
        folder, name = os.path.split(source)
        stem, ext = os.path.splitext(name)
        out.append((source, os.path.join(folder, stem + suffix + ext)))
    return out


def preview(request):
    rows = []
    for source, destination in plan(request):
        rows.append({
            "from": os.path.basename(source),
            "to": os.path.basename(destination),
            "conflict": os.path.exists(destination),
        })
    taken = sum(row["conflict"] for row in rows)
    note = f"{taken} name already taken" if taken else None
    return {"rows": rows, "note": note}


def run(request):
    changes = []
    for source, destination in plan(request):
        if os.path.exists(destination):
            sys.exit(f"“{os.path.basename(destination)}” is already there")
        if os.path.isdir(source):
            shutil.copytree(source, destination, symlinks=True)
        else:
            shutil.copy2(source, destination)
        changes.append({"kind": "created", "path": destination})
    return {
        "status": f"Duplicated {len(changes)} item(s)",
        "changes": changes,
        "reload": True,
    }


verb = sys.argv[1] if len(sys.argv) > 1 else ""
if verb == "describe":
    reply = describe()
elif verb == "preview":
    reply = preview(json.load(sys.stdin))
elif verb == "run":
    reply = run(json.load(sys.stdin))
else:
    sys.exit("usage: duplicate describe|preview|run")
json.dump(reply, sys.stdout)
```

### Testing it without Files

Each call is an ordinary process, so you can run the script by hand with a
request you write yourself:

```sh
cat > /tmp/request.json <<'EOF'
{ "command": "duplicate", "arg": " backup",
  "targets": ["/home/me/Documents/report.pdf"],
  "situation": { "path": "/home/me/Documents" } }
EOF
~/.config/otto/files-scripts/duplicate preview < /tmp/request.json
```

## Reference

### Where scripts live

Files looks in `$XDG_CONFIG_HOME/otto/files-scripts/`, which is usually
`~/.config/otto/files-scripts/`. To use a different folder, set
`OTTO_FILES_SCRIPTS` to its path before starting `otto-files`.

Files skips anything that isn't executable, and anything whose name starts
with a dot. Scripts are read once, when a window opens, so open a new window
after adding or changing one.

Script commands aren't offered in the Trash or in Recent, because neither is a
real folder.

### The three calls

| Call | When | Standard input | Time limit |
|------|------|----------------|------------|
| `describe` | Once, when a window opens | nothing | 3 seconds |
| `preview` | After each keystroke, for an `arg` with `preview` set | the request | 0.6 seconds |
| `run` | When the command is chosen | the request | none |

A `run` happens in the background. While it is running, the status line says so
and the window stays usable.

Every call also gets the window's language in the `OTTO_LOCALE` environment
variable. Files doesn't set the working directory, so always use the absolute
paths from the request.

### `describe`

Answer with `{ "commands": [ … ] }`, where each command is:

| Field | Required | Meaning |
|-------|----------|---------|
| `id` | yes | Identifies the command within this script. Sent back as `command` in every request |
| `title` | yes | The name shown in the palette and the menu |
| `keywords` | no | Extra words the palette matches. They are never shown |
| `group` | no | `go`, `file` (the default), `edit` or `view` |
| `when` | no | When the command is offered; see below |
| `arg` | no | Makes the command ask for text first; see below |
| `undo` | no | The name of the undo step. Defaults to `title` |

**`when`**. Files checks these conditions itself, without running the script.
That's why opening the palette stays instant.

| Field | Values |
|-------|--------|
| `targets` | `some` (default): one or more items. `one`: exactly one. `none`: only when there is nothing to act on. `any`: always |
| `extensions` | A list such as `["jpg", "png"]`. Every target must have one of them. Case doesn't matter; leave out the dot |
| `kinds` | `any` (default), `files` or `folders` |

To keep the palette fast, Files checks `kinds` without reading the disk. It
only knows whether the item under the cursor is a folder. That means `folders`
matches only a single folder, and `files` can't rule out a folder somewhere in
a larger selection. If this matters, check again in `run`.

**`arg`**

| Field | Meaning |
|-------|---------|
| `prompt` | Shown before the text field, as in `Add to each name:`. Required |
| `label` | The short name shown after the title in the list, as in `Duplicate › suffix`. Defaults to `name` |
| `placeholder` | Dimmed text shown while the field is empty |
| `initial` | Text the field starts with. If it has an extension, only the part before the extension is selected, so typing a new name keeps the extension |
| `preview` | `true` to receive `preview` calls while the user types |

A command without an `arg` runs as soon as it is chosen.

### The request

`preview` and `run` both receive:

```json
{ "command": "duplicate",
  "arg": " copy",
  "locale": "en-GB",
  "targets": ["/home/me/Pictures/a.png", "/home/me/Pictures/b.png"],
  "situation": { "path": "/home/me/Pictures", … } }
```

- `targets` holds absolute paths: the selection, or the item under the cursor
  when nothing is selected. Files you left out of a preview are not included.
- `arg` is the text typed so far. It is absent for a command without an `arg`.

`situation` describes the window:

| Field | Meaning |
|-------|---------|
| `path` | The folder being shown |
| `selection` | The selected paths |
| `siblings` | Every name in that folder's listing. Useful for checking whether a name is taken without reading the disk |
| `cursor_name`, `cursor_is_dir` | The item under the cursor, and whether it is a folder |
| `show_hidden` | Whether hidden files are shown |
| `view` | `list`, `grid` or `columns` |
| `sort` | `name`, `size`, `kind` or `modified` |
| `places` | The sidebar, as `{ "label", "path" }` pairs |
| `can_paste`, `can_undo`, `can_go_back`, `can_go_forward`, `can_go_up`, `has_entries`, `trash`, `recent` | What the window can do right now |

### The preview reply

```json
{ "rows": [ { "from": "a.png", "to": "a copy.png", "conflict": false } ],
  "note": "1 name already taken" }
```

- Give one row per target. `from` must be the target's file name, because
  that's how skipping a file finds its row.
- Leave out `to` when a command makes one result out of many files, such as an
  archive. The rows then list what goes in, and the note says where it ends up.
- `conflict: true` shows the row in red. Your `run` should refuse in the same
  case; Files doesn't stop it for you.
- `note` is the line under the list.

If the preview exits with a non-zero status, the last line of standard error is
shown as the note. If it takes too long or prints something that isn't valid
JSON, no preview is shown.

### The run reply

```json
{ "status": "Duplicated 2 item(s)",
  "changes": [ { "kind": "created", "path": "/home/me/Pictures/a copy.png" },
               { "kind": "moved", "from": "/home/me/a.png", "to": "/home/me/Old/a.png" } ],
  "reload": true }
```

Every field is optional, and so is the reply itself. A script that prints
nothing and exits with status 0 has succeeded.

`changes` is the undo record. A command gets an undo step only if it reports at
least one change:

| Kind | What Undo does |
|------|----------------|
| `created` | Moves `path` to the Trash. An empty folder is simply removed |
| `moved` | Moves `to` back to `from`. It refuses if something is now at `from` |

A non-zero exit means the run failed, and the last line of standard error is
shown as the error. Anything the script did before failing is not recorded for
undo.

### Translations

Every text a person reads can be given once, or once per language:

```json
"title": { "en": "Duplicate", "it": "Duplica", "de": "Duplizieren" }
```

This works for `title`, `undo`, and the `arg` fields `prompt`, `label`,
`placeholder` and `initial`. Files picks the window's language: first the exact
tag (`pt-BR`), then the language on its own (`pt`), then English. `keywords`
can be split by language the same way, and the palette matches keywords in
every language.

For text the script writes at run time, such as `status`, notes and errors, use
`locale` from the request or `OTTO_LOCALE`.

## When a script doesn't show up

Run `otto-files` from a terminal. When a script can't be run, exits with an
error while describing itself, or prints invalid JSON, Files logs a warning
that names the script and the reason.

Also check:

- the file is executable (`chmod +x`), and its name doesn't start with a dot;
- the `#!` line points to an interpreter that exists;
- `describe` finishes in under 3 seconds;
- the `when` conditions match what you have selected;
- you opened a new window after changing the script.

## More examples

The Otto source tree includes two complete scripts in
`components/otto-files/scripts/`:

- **`zip`** — **Compress to Zip**: asks for an archive name and previews what
  goes in. Needs `zip`.
- **`unzip`** — **Extract Archive**: offered only when every target is a
  `.zip`. Extracts each archive into a folder named after it. Needs `unzip`.

Both are translated into Italian and show how to handle conflicts. To install
them:

```sh
mkdir -p ~/.config/otto/files-scripts
cp components/otto-files/scripts/{zip,unzip} ~/.config/otto/files-scripts/
chmod +x ~/.config/otto/files-scripts/{zip,unzip}
```

## Not there yet

- Undo covers only new and moved files. Changes a script makes in place, or
  files it deletes, can't be undone.
- Commands can only use the four built-in groups.
- A running script can't report progress or be cancelled.
- Scripts are read only when a window opens.
