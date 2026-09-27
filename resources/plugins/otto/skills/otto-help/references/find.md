# Finding a file

For "where's that invoice from March", "find the photos from last week", "the
PDF that mentions the lease", "what did I download yesterday": anything that
means finding a file the person can describe but not locate.

**Prefer `otto-search`.** It asks the desktop's file index, which already
knows every name, type, date, size and the words inside documents: it answers
in a moment, ranks the best match first, and runs without asking the person.
`find` walks the disk, is slow over a home folder, and stops to ask first.

Translate, search, judge, then **open Files on the answer**. Do not paste a
list of paths into the chat; the person wants to see the files, and Files
shows them with thumbnails, sizes and dates, ready to open.

## Step 1: write the query

Otto has one search language, the same one the Files search strip reads.
Plain words match file names, forgivingly: the letters in order, so `otfl`
finds `otto-files`. Every word must match, in any order. `key:value` filters
narrow it down.

| Filter | Examples | Means |
|---|---|---|
| a word | `invoice`, `tax invoice` | the name contains it (each word on its own) |
| `"a phrase"` | `"tax return"` | the name contains exactly this |
| `text:` | `text:lease`, `text:"notice period"` | the contents mention it (documents the indexer has read) |
| `kind:` | `kind:pdf`, `kind:image,video` | `document` `pdf` `image` `video` `audio` `text` `archive` `app` `folder`; a comma is "any of" |
| `in:` | `in:~/Downloads`, `in:"~/My Files"` | under that folder, at any depth |
| `modified:` | `modified:today`, `modified:<7d`, `modified:2025-03`, `modified:>1y` | when it was last written |
| `size:` | `size:>100M`, `size:<1K` | how big; `K` `M` `G` `T` |
| `sort:` | `sort:modified`, `sort:size`, `sort:name` | order: newest, largest, A to Z (default: best match) |
| `-` before anything | `-draft`, `-kind:image`, `-in:~/.cache` | leave it out |

How everyday words map:

| They say | Write |
|---|---|
| "last week", "recently" | `modified:<7d` (or `<30d` for "recently", if a week finds nothing) |
| "today", "yesterday" | `modified:today`, `modified:yesterday` |
| "in March", "last year" | `modified:2026-03`, `modified:2025` (work out the year from today's date) |
| "photos", "pictures", "screenshots" | `kind:image` (add `screenshot` as a word for screenshots) |
| "videos", "music", "documents", "PDFs", "zip files" | `kind:video`, `kind:audio`, `kind:document`, `kind:pdf`, `kind:archive` |
| "that mentions X", "about X", "that says X" | `text:X` (quote it if it is several words) |
| "in Downloads", "on my desktop" | `in:~/Downloads`, `in:~/Desktop` |
| "big files", "over a gig" | `size:>100M sort:size`, `size:>1G` |
| "the latest", "newest" | `sort:modified` |
| "not the drafts" | `-draft` |
| a folder | `kind:folder` plus its name |

Keep it short. A name word or two and one or two filters beat a long query:
every term must hold, so each extra one is another way to miss. Put a guess
about the name in `text:` only when the person described what the file says,
not what it is called.

## Step 2: search

Show the query before you run it, in a line of its own, so the person learns
the language and can type it themselves next time:

> Searching for `invoice kind:pdf modified:2026-03`: PDFs named like
> "invoice", changed in March.

```sh
otto-search --json --limit 20 'invoice kind:pdf modified:2026-03'
```

Single-quote the query so the shell leaves `<`, `>` and `"` alone. Run it as
shown, on its own: no pipe to `head` or `grep`, no `2>&1`, no `; echo $?`.
`--limit` already caps the output, the exit status comes back with the
result, and a bare `otto-search` runs without asking the person first. Each
line of output is one file:

```json
{"path":"/home/me/Documents/Bills/invoice-0312.pdf","name":"invoice-0312.pdf","kind":"file","modified":"2026-03-12T09:41:02+01:00","size":84211,"snippet":null}
```

The last line is not a file but what the indexer is doing:

```json
{"index":{"state":"indexing","progress":0.62,"remaining_seconds":340}}
```

`state` is `idle` (caught up), `indexing` or `paused` (the answer may be
missing files it has not reached yet), `stopped` or `missing`. When it is
`indexing` or `paused`, say so alongside the results ("the index is 62% of the
way through, so this may not be everything"), and say it again if nothing
turns up.

`snippet` is the passage that matched, for a `text:` query. The search looks
under the person's home folder unless the query has `in:`.

Words Otto has read in pictures (screenshots, photos of receipts) are not in
`otto-search`'s answer; Files' own search has them. When the person is after
a picture of some text and `otto-search` finds nothing, open Files on the
search anyway (step 4): it may find it there.

The exit status says what happened:

- `0`: it found something.
- `1`: it found nothing.
- `2`: the file indexer is not running, so nothing could look. Tell the person
  that file search needs LocalSearch, the desktop's file indexer, and that it
  should be installed and running (`localsearch` package; it starts with the
  session). Stop there; do not fall back to `find` over the whole disk unless
  they ask.

A line on stderr starting `hint:` means part of the query was not read as a
filter (a typo like `kind:pfd`). Fix it and search again.

## Step 3: judge the results

Read the names, folders, dates and snippets against what the person said.

After each search, say what came back before doing anything else: how many
files, where most of them are, and a name or two that stand out. Keep the
person in the loop between searches rather than going quiet until the end:

> 3 files, all in Documents/Bills; `invoice-0312.pdf` looks like the one.

> Nothing. Widening to the last 30 days: `invoice kind:pdf modified:<30d`.

- **Nothing, or clearly the wrong files**: loosen or rephrase and search
  again, saying what you are changing and why, with the new query. Drop the
  least certain filter first, widen a date (`<7d` to `<30d`), swap a name
  word for `text:`, or the other way round. Try once or twice, not more.
- **Still nothing**: say so plainly, and say what you searched for, with the
  queries ("I looked for `lease kind:pdf modified:2026`, then `text:lease`").
  Say the file may have been moved or made very recently, since the index
  can take a while to notice, and offer to open Files on the search so they
  can look themselves.
- **A whole folder never shows up**: it may be outside the folders the index
  looks in. Read `search.folders` and offer to add it (see
  [configure.md](configure.md)), or open `otto-settings --pane search`.

## Step 4: open Files

Start Files through this skill's `open` script, always, and write the
command exactly in the shape below: no `setsid`, no `&`, no redirections. Put every path and query in single quotes, whole, even with spaces,
`&` or brackets in it: `--select '/home/me/Desktop/Report (1).pdf'`. Never
escape characters with backslashes; an escaped command is not recognised as
safe and stops to ask the person first.

- **One clear match**: open its folder with it selected.

  ```sh
  <this skill>/scripts/open otto-files --select '/home/me/Documents/Bills/invoice-0312.pdf'
  ```

- **Several that could be it**: open Files on the search itself, so the person
  can look through them and refine the query in the search strip. Add
  `--select` with the best guess to have it highlighted.

  ```sh
  <this skill>/scripts/open otto-files --search 'invoice kind:pdf modified:2026-03'
  <this skill>/scripts/open otto-files --search 'invoice kind:pdf' --select '/home/me/Documents/Bills/invoice-0312.pdf'
  ```

  Add `--in DIR` to scope the search to one folder, the way the *This folder*
  button does.

Then say what you opened, with the query it came from: "Files is open on the
March invoice, in Documents/Bills (`invoice kind:pdf modified:2026-03`)." or
"Files is showing the four PDFs from March that match 'invoice'; the one from
the 12th is selected. The query is in the search strip if you want to refine
it."

## Rules

- **Prefer `otto-search`** to `find` or `ls -R`.
- **Show every query you run**, in backticks, exactly as run.
- **Narrate between searches**: a line after each one on what it found and
  what you will try next.
- **Open Files; do not list paths.** Mention one or two names in your sentence
  if it helps, never a list of twenty.
- **Do not open the files themselves** unless the person asked you to. Finding
  is not opening.
- **Read only.** Do not move, rename or delete anything you found unless asked.
- If they asked a question about a file ("how big is it", "when did I last
  change it"), answer it from the JSON and still offer to show it in Files.
