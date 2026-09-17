# The Files script protocol, field by field

The authority is the module comment at the top of
`components/otto-files/src/scripts.rs`, which is where the host parses all of
this. The user guide is
https://github.com/nongio/otto/blob/main/docs/user/files-custom-commands.md.

## The three calls

| Call | When | Standard input | Deadline |
|---|---|---|---|
| `describe` | Once, as a window opens | nothing | 3 seconds |
| `preview` | After every keystroke, for an `arg` with `preview: true` | the request | 0.6 seconds |
| `run` | When the command is chosen | the request | none |

Each call is a fresh process. `OTTO_LOCALE` holds the window's language on
every call. The working directory is not set — use the absolute paths in the
request. A `run` happens on a worker thread, so the window stays usable; a
`preview` that overruns is killed and nothing is shown.

## `describe`

```json
{ "commands": [ {
    "id": "add-prefix",
    "title": "Add Prefix",
    "keywords": ["rename", "batch"],
    "group": "file",
    "when": { "targets": "some", "extensions": ["jpg", "png"], "kinds": "files" },
    "arg": { "prompt": "Put in front of each name", "label": "prefix",
             "placeholder": "2026-", "initial": "", "preview": true },
    "undo": "Add Prefix"
} ] }
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Identifies the command inside this script, and comes back as `command` in each request. The host namespaces it as `scripts:<file name>.<id>`, so two scripts may use the same id |
| `title` | yes | What the palette and the right-click menu show |
| `keywords` | no | Extra words the palette matches. Never shown |
| `group` | no | `go`, `file` (default), `edit`, `view`. There are no other groups |
| `when` | no | When the command is offered |
| `arg` | no | Ask for text before running |
| `undo` | no | The name of the undo step. Defaults to `title` |

### `when`

Checked by the host against the window alone, never by running the script —
that is what keeps the palette instant.

| Field | Values |
|---|---|
| `targets` | `some` (default) one or more, `one` exactly one, `none` only with nothing to act on, `any` always |
| `extensions` | `["jpg", "png"]` — every target must carry one. Case-insensitive, no dot |
| `kinds` | `any` (default), `files`, `folders` |

`kinds` is decided without touching the disk, from the cursor entry alone. So
`folders` matches only a single folder, and `files` cannot rule out a folder
inside a larger selection. Check again in `run` when it matters.

### `arg`

| Field | Meaning |
|---|---|
| `prompt` | Required. Shown before the field |
| `label` | The short name after the title in the list — `Duplicate › suffix` |
| `placeholder` | Dimmed text while the field is empty |
| `initial` | Text the field starts with. With an extension, only the stem is selected |
| `preview` | `true` to be called with `preview` while it is typed |

A command with no `arg` runs the moment it is chosen.

## The request

```json
{ "command": "add-prefix",
  "arg": "2026-",
  "locale": "en-GB",
  "targets": ["/home/me/Pictures/a.png", "/home/me/Pictures/b.png"],
  "situation": { "path": "/home/me/Pictures", "…": "…" } }
```

`targets` is the selection, or the item under the cursor when nothing is
selected, as absolute paths. Rows skipped in the preview are already gone from
it. `arg` is absent for a command without one.

`situation` is the whole window:

| Field | Meaning |
|---|---|
| `path` | The folder on show |
| `selection` | The selected paths |
| `siblings` | Every name in that listing — enough to tell whether a name is taken without reading the disk |
| `cursor_name`, `cursor_is_dir` | The entry under the cursor |
| `show_hidden` | Whether hidden files are shown |
| `view` | `list`, `grid`, `columns` |
| `sort` | `name`, `size`, `kind`, `modified` |
| `places` | The sidebar, as `{ "label", "path" }` |
| `trash`, `recent` | This window is the Trash, or Recent. Script commands are not offered in either |
| `can_paste`, `can_undo`, `can_go_back`, `can_go_forward`, `can_go_up`, `has_entries` | What the window can do now |

## The preview reply

```json
{ "rows": [ { "from": "a.png", "to": "2026-a.png", "conflict": false } ],
  "note": "1 name already taken" }
```

- One row per target. `from` **must** be the target's file name: that is how
  skipping a row finds it again.
- Leave `to` out when many files become one result, such as an archive. The
  rows then say what goes in and the note says where it ends up.
- `conflict: true` draws the row in red. It does not stop the run — the script
  has to refuse too.
- `note` is the line under the list.
- A non-zero exit puts the last line of stderr in the note. Invalid JSON or an
  overrun shows no preview at all.
- A preview must change nothing. It runs after every keystroke.

## The run reply

```json
{ "status": "Renamed 2 item(s)",
  "changes": [ { "kind": "created", "path": "/home/me/a.zip" },
               { "kind": "moved", "from": "/home/me/a.png", "to": "/home/me/2026-a.png" } ],
  "reload": true }
```

Every field is optional, and so is the reply — printing nothing and exiting 0
is a success.

| Kind | What Undo does |
|---|---|
| `created` | Moves `path` to the Trash; an empty folder is removed |
| `moved` | Moves `to` back to `from`, refusing if something is there now |

A command gets an undo step only if it reports at least one change. A non-zero
exit is a failure and the last line of stderr is the error — and **nothing a
failed run already did is recorded**, so validate every destination before
touching the first file.

`reload` refreshes the listing. `status` is the status line.

## Translations

`title`, `undo`, `prompt`, `label`, `placeholder` and `initial` each take a
string or an object keyed by language:

```json
"title": { "en": "Add Prefix", "it": "Aggiungi prefisso" }
```

The host picks the exact tag (`pt-BR`), then the bare language (`pt`), then
English, then whatever came first. `_` reads as `-` and case does not matter.
`keywords` may be keyed the same way, and every language's words match at once.

For text written at run time — `status`, notes, errors — use `locale` from the
request or `OTTO_LOCALE`.

## Not there yet

- Undo covers new and moved files only. In-place edits and deletions cannot be
  undone.
- Only the four built-in groups.
- A run cannot report progress or be cancelled.
- Scripts are read when a window opens, and not again.
