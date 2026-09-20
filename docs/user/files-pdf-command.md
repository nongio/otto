# PDF from Pictures

The [Duplicate tutorial](files-custom-commands.md) builds a small command that
copies files. This one goes further: a script that turns a selection of
pictures into a PDF, reports a line per page while it works, and offers a
second command that reads the text in the pictures so the PDF can be searched.

Along the way it uses the three parts of the script protocol the first
tutorial doesn't touch:

- **two commands from one script**, offered only for pictures;
- **`progress`**, so a long run fills the status line, the island and the dock
  icon as it goes;
- **`choices`**, an argument you pick from a list rather than type.

The finished script is in the Otto source tree, at
`components/otto-files/scripts/pdf`. If you'd rather read it than build it:

```sh
cp components/otto-files/scripts/pdf ~/.config/otto/files-scripts/
chmod +x ~/.config/otto/files-scripts/pdf
```

Open a new Files window afterwards, select some pictures and press `Ctrl+P`.

## What you need

Pictures become pages through [img2pdf](https://pypi.org/project/img2pdf/),
the pages are put back together with [pikepdf](https://pypi.org/project/pikepdf/),
and the text layer comes from [tesseract](https://github.com/tesseract-ocr/tesseract).
Only the first two are needed for the plain command.

```sh
# Arch
sudo pacman -S python-img2pdf python-pikepdf tesseract tesseract-data-eng
# Debian and Ubuntu
sudo apt install python3-img2pdf python3-pikepdf tesseract-ocr
```

Each tesseract language is its own package: `tesseract-data-ita`,
`tesseract-ocr-ita` and so on. The script offers exactly the ones you have
installed.

## 1. Two commands, for pictures only

Start `~/.config/otto/files-scripts/pdf` with the plain command. The `when`
block is what keeps it out of the way: it asks for pictures, and for files
rather than folders, so the command is never offered for a selection it
couldn't turn into pages.

```python
#!/usr/bin/env python3
import json
import os
import subprocess
import sys
import tempfile

import img2pdf
import pikepdf

PICTURES = ["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif"]


def describe():
    return {
        "commands": [
            {
                "id": "pdf",
                "title": "PDF from Pictures",
                "keywords": ["pdf", "document", "combine", "pages"],
                "when": {
                    "targets": "some",
                    "extensions": PICTURES,
                    "kinds": "files",
                },
                "arg": {
                    "prompt": "PDF name",
                    "label": "name",
                    "initial": "Scanned.pdf",
                    "preview": True,
                },
                "undo": "Make PDF",
                "progress": True,
            }
        ]
    }
```

A selection has to be *all* pictures for the command to appear. That is on
purpose: a PDF made out of some of what you picked isn't what anyone meant.

`"progress": true` is the new field. It changes how Files reads the script's
output during a run, and section 4 puts it to use.

## 2. Make the pages

One picture at a time, into a temporary folder, then all the pages into one
document. Going page by page costs nothing and gives the run something to
report between the pages.

```python
def page_of(picture, destination):
    """One picture as a one-page PDF, written to `destination`."""
    with open(destination, "wb") as pdf:
        pdf.write(img2pdf.convert(picture))


def run(request):
    directory = request["situation"]["path"]
    name = (request.get("arg") or "").strip() or "Scanned.pdf"
    if not name.lower().endswith(".pdf"):
        name += ".pdf"
    name = unique(directory, name)
    destination = os.path.join(directory, name)
    pictures = request["targets"]

    with tempfile.TemporaryDirectory(prefix="otto-files-pdf-") as workspace:
        pages = []
        for index, picture in enumerate(pictures):
            page = os.path.join(workspace, f"{index:04d}.pdf")
            page_of(picture, page)
            pages.append(page)

        document = pikepdf.Pdf.new()
        for page in pages:
            with pikepdf.Pdf.open(page) as one:
                document.pages.extend(one.pages)
        document.save(destination)

    return {
        "status": f"Made “{name}” from {len(pictures)} pictures",
        "changes": [{"kind": "created", "path": destination}],
        "reload": True,
    }
```

The `created` change is what gives the command an undo step: `Ctrl+Z` moves
the PDF to the Trash.

**A name that is free.** The command asks for a name, not for permission to
replace anything, so it never writes over a PDF that is already there. It
takes the next free name instead:

```python
def unique(directory, name):
    """`name`, or the first "name 2.pdf" that is free."""
    stem, extension = os.path.splitext(name)
    candidate = name
    n = 2
    while os.path.exists(os.path.join(directory, candidate)):
        candidate = f"{stem} {n}{extension}"
        n += 1
    return candidate
```

## 3. A preview of one thing made of many

Duplicate previews one new name per file. This command makes a single PDF out
of the whole selection, so the rows say what goes in and leave out `to`; the
note under the list says where it all ends up.

```python
def preview(request):
    name = (request.get("arg") or "").strip() or "Scanned.pdf"
    if not name.lower().endswith(".pdf"):
        name += ".pdf"
    rows = [{"from": os.path.basename(t)} for t in request["targets"]]
    return {"rows": rows, "note": f"{len(rows)} pictures into “{name}”"}
```

Rows still matter even without a `to`: press `↓` and `Space` on one to leave
that picture out, and both the next preview and the run get the shorter list.
The order of the rows is the order of the pages, which is the order Files
shows the pictures in.

## 4. Say where the run has got to

A dozen scans take a while. With `"progress": true` declared, a run may print
one JSON object per line while it works:

```python
def report(done, total, item):
    """One line saying where the run has got to. Flushed, because the host
    reads it while the script is still working."""
    json.dump({"done": done, "total": total, "item": item}, sys.stdout)
    sys.stdout.write("\n")
    sys.stdout.flush()
```

Call it before each page, and once more when the pages are done and only the
assembly is left:

```python
        for index, picture in enumerate(pictures):
            report(index, len(pictures), os.path.basename(picture))
            ...
        report(len(pictures), len(pictures), "")
```

The last line that isn't one of those objects is the result, so the reply at
the end of `run` needs no changing. Files turns the lines into the window's
status line, the [island](dynamic-island.md) and the progress ring on the
Files icon in the dock. The script doesn't need to know any of those exist.

`flush()` is the part that is easy to forget. Python buffers standard output
when it isn't a terminal, and without the flush every line arrives at once,
when the script finishes.

If a script doesn't know how much there is to do yet, send `"total": 0` and
raise it later: `total` is allowed to grow as the run finds more work.

## 5. A second command, with a list to choose from

Tesseract can put the text it finds in a picture behind the picture, so the
PDF can be searched and its words copied. Which language it reads in matters,
so that is what the second command asks for. This time the argument is a
**choice** rather than a field to type in:

```python
def tesseract_languages():
    """What tesseract can read here, or an empty list if it cannot be asked."""
    try:
        listed = subprocess.run(
            ["tesseract", "--list-langs"],
            capture_output=True, text=True, timeout=2,
        )
    except (OSError, subprocess.SubprocessError):
        return []
    if listed.returncode != 0:
        return []
    # The first line is a heading, the rest are the languages — except "osd",
    # which is tesseract's orientation detector and reads no text at all.
    return [
        line.strip()
        for line in listed.stdout.splitlines()[1:]
        if line.strip() and line.strip() != "osd"
    ]
```

The command is added to the list `describe` returns only when that comes back
with something, so on a machine without tesseract there is no command that
can't work:

```python
    languages = tesseract_languages()
    if languages:
        commands.append({
            "id": "ocr",
            "title": "Searchable PDF from Pictures",
            "keywords": ["pdf", "ocr", "text", "searchable", "scan"],
            "when": {"targets": "some", "extensions": PICTURES, "kinds": "files"},
            "arg": {
                "prompt": "Read the text as",
                "label": "language",
                "initial": languages[0],
                "choices": [{"value": code, "title": NAMES.get(code, code)}
                            for code in languages],
                "preview": True,
            },
            "undo": "Make PDF",
            "progress": True,
        })
```

`NAMES` is a small table of tesseract's codes against the names people read,
`{"eng": "English", "ita": "Italiano", ...}`, falling back to the code for
anything not in it.

`choices` can be a plain list of strings, or objects with a `title` and an
optional `subtitle` where the value on its own isn't much to read. The palette
completes them as you type, so `ita` and `Italiano` both find the same entry.
Put the language the window is in first, so `Return` takes the likely one
without any typing.

Both commands share one `run`; `request["command"]` says which was chosen.
The searchable one names the file itself, since its question went on the
language:

```python
def read_page(picture, destination, language):
    """One picture as a one-page PDF with the text tesseract found in it."""
    base = os.path.splitext(destination)[0]
    result = subprocess.run(
        ["tesseract", picture, base, "-l", language, "pdf"],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        sys.exit((result.stderr or result.stdout).strip() or "tesseract failed")
```

`sys.exit` with a message is how a script fails: Files shows the last line of
standard error and stops there. Tesseract writes its own `.pdf` extension on
to the name it is given, which is why `read_page` hands it the name without
one.

Reading is slower than converting, by a lot, which is what makes the progress
lines from section 4 worth having.

## 6. Say it in more than one language

Every string a person reads can be given once per language, and `describe`
hands the whole table over for Files to pick from:

```python
STRINGS = {
    "en": {"title": "PDF from Pictures", "prompt": "PDF name", ...},
    "it": {"title": "PDF dalle immagini", "prompt": "Nome del PDF", ...},
}


def by_locale(key):
    """One string per language, for `describe` to hand over whole."""
    return {language: table[key] for language, table in STRINGS.items()}
```

Then `"title": by_locale("title")` in the command. For the text the script
writes while it runs, such as the note and the status, the window's language
arrives with each request, as `locale` in the JSON and `OTTO_LOCALE` in the
environment:

```python
def tr(request, key, **kwargs):
    table = STRINGS.get(
        (request.get("locale") or "en").split("-")[0].lower(), STRINGS["en"]
    )
    return table[key].format(**kwargs)
```

The finished script also keeps a plural form beside anything counted, as
`into` and `into_many`, and picks between them on `count`.

## Trying it without Files

Every call is an ordinary process, so a request you write yourself is enough
to exercise the whole thing from a terminal:

```sh
cat > /tmp/request.json <<'EOF'
{ "command": "pdf", "arg": "Holiday.pdf", "locale": "en-GB",
  "targets": ["/home/me/Pictures/a.png", "/home/me/Pictures/b.png"],
  "situation": { "path": "/tmp" } }
EOF
~/.config/otto/files-scripts/pdf describe
~/.config/otto/files-scripts/pdf preview < /tmp/request.json
~/.config/otto/files-scripts/pdf run < /tmp/request.json
```

The `run` prints its progress lines as it goes, which is the quickest way to
check the flush is working. Change `"command"` to `"ocr"` and `"arg"` to a
language code such as `eng` for the searchable one.

## When it doesn't show up

- The command is offered only when **every** target is a picture, and none of
  them is a folder. One stray `.txt` in the selection hides it.
- The searchable command needs `tesseract --list-langs` to answer. If it
  prints nothing but `osd`, install a language pack.
- Scripts are read when a window opens, so open a new one after editing.
- Run `otto-files` from a terminal to see why a script was skipped.

The full list of what a script can say is in the
[reference](files-custom-commands.md#reference).
