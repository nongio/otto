# Using Otto Files' commands yourself

Otto Files has a command palette (`Ctrl+P`, the right-click menu), and every
command in it is a script the person can also run without a window: making a
PDF from pictures, a searchable PDF that reads the text in scans (OCR),
compressing to zip, extracting, rotating and flipping pictures, setting a
background, and whatever they have added. When a request is one of these, use
the person's command rather than doing the same job another way: it is the one
they know, it names files the way Files does, and Files can undo it.

`<this skill>` below is the folder this skill's `SKILL.md` is in; write it as
an absolute path.

## 1. See what there is

```sh
<this skill>/scripts/files-command commands
```

One command a line: the script, the command's id, what it does, which files
it takes, and the text it asks for, with its choices and default:

```
pdf pdf: PDF from Pictures [files: some of png,jpg,…] [--arg: PDF name; default Scanned.pdf]
pdf ocr: Searchable PDF from Pictures [files: some of png,jpg,…] [--arg: Read the text as, one of eng (English), deu (Deutsch), …; default eng]
zip compress: Compress to Zip [files: some] [--arg: Archive name; default Archive.zip]
```

The list is the person's own: commands come and go as they add them. Look
before saying one exists, and do not invent one that is not listed.

## 2. Run one

```sh
<this skill>/scripts/files-command run SCRIPT ID [--arg TEXT] FILE...
```

- **`SCRIPT ID`** are the first two words of its line: `pdf ocr`,
  `zip compress`.
- **`--arg`** is the text the command asks for, when its line has `--arg`: a
  name, or one of its choices (`--arg ita`). Leave it out to take the
  default.
- **The files** are absolute paths, of the kinds its line says. They are
  selected in the folder the first one is in, and what the command makes goes
  there too.

It prints the command's progress, then one line of JSON: `status` says what
happened, and `changes` lists every file it made or changed. Tell the person
the status in your own words and the paths of what it made. A command that
fails prints why on its last line.

Running a command changes files, so the person is asked before it runs.

## Finding the files first

The request usually names files rather than paths: "the scans from this
morning", "the photos in Downloads". Find them first
([find.md](find.md) when it is there, or list the folder), and when more than
one set could be meant, ask which with your question tool before running
anything.
