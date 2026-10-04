# Search Language

**Status:** draft  
**Related specs:** [file-browser.md](./file-browser.md) (Find and Recent, the first users), [peek-ocr.md](./peek-ocr.md) (words read from pictures, which search also answers from), [launcher.md](./launcher.md)

## Summary

One way to write a file search, understood everywhere Otto searches for
files: the Files strip, the launcher, the command line, D-Bus and the agents.
Plain words find names; `key:value` filters narrow by kind, place, date, size
and contents. The same text finds the same files wherever it is typed.

## Goals

- A query typed in one place works unchanged in every other.
- Plain words behave as they always have: they find names, forgivingly.
- Every filter a person can type is also something a menu can add, so the
  language can be discovered without reading about it.
- Nothing typed is dropped silently. Text that does not parse is searched as
  words, and the person is told why it was not read as a filter.
- A saved or shared query means the same thing in every locale.

## Non-Goals

- An index of Otto's own. Queries are answered by the desktop's file index
  (LocalSearch), plus the words Otto has read in pictures (see
  [peek-ocr.md](./peek-ocr.md)).
- Boolean expressions (`OR` between terms, parentheses). Commas inside one
  filter give OR where it is useful (`kind:image,video`).
- Regular expressions or wildcards typed by the person.
- Searching metadata the index does not hold (tags, ratings, colours).

## Behavior

### Tokens

A query is split on whitespace. A double quote runs to the next double quote,
spaces included; an unclosed quote runs to the end of the text, so a phrase
still being typed is one token.

Each token is one of:

| Form | Meaning |
|---|---|
| `invoice` | The name contains these letters in this order: `otfl` finds `otto-files.rs`. |
| `"tax return"` | The name contains this exact text. |
| `key:value` | A filter (below). The value may be quoted: `in:"~/My Files"`. |
| `-token` | The token, excluded: `-draft`, `-kind:image`. A lone `-` is a word. |

All terms must hold. Words are independent of each other: `tax invoice` finds
`invoice-tax.pdf`. Matching ignores case.

A token is read as a filter only when the text before its first colon is made
of letters. `12:30`, `C:\` and `http://…` are words and earn no hint.

### Filters

| Filter | Values | Meaning |
|---|---|---|
| `text:` | a word or `"a phrase"` | The contents contain it. Documents are searched through the index's full-text store; pictures through the words Otto has read in them. |
| `kind:` | `document`, `pdf`, `image`, `video`, `audio`, `text`, `archive`, `app`, `folder`; comma for any of several | The kind of file. |
| `in:` | a folder: `~/Documents`, `/mnt/data`, `notes` | Under that folder, at any depth. `~` is home; a relative path is relative to the folder the search started in. Several `in:` terms: under any of them. `-in:` leaves a folder out. |
| `modified:` | `today`, `yesterday`, `week`, `month`, `year`; `<7d`, `>1y` (units `h`, `d`, `w`, `m`, `y`); `2025`, `2025-03`, `2025-03-14`, each optionally with `<` or `>` | When the file was last written. `<7d` is "in the last seven days", `>1y` "longer ago than a year". A date is a calendar period in local time: `2025-03` is all of March, `<2025-03` before it began, `>2025-03` after it ended. A bare duration means "within". |
| `size:` | `>100M`, `<1K`, `1.5G`; units `K`, `M`, `G`, `T` with optional `B` or `iB` | Size. Units are powers of 1000, as Files shows sizes; `KiB`, `MiB`, … are powers of 1024. A bare size means "at least". |
| `sort:` | `relevance`, `modified`, `size`, `name` | How results are ordered: best name match, newest, largest, or A to Z. The last `sort:` wins. |

Aliases are accepted for values (`photos` for `image`, `music` for `audio`,
`date` for `modified`, and so on). Relative months and years are fixed
lengths (30 and 365 days), not calendar ones.

`kind:document` and `kind:app` are what the index files as documents and as
software. `kind:archive` is judged by the file name's extension, because the
index has no type for archives. The other kinds are judged by type.

### Scope

Every search has a default scope given by where it was started: in Files,
*This folder* or *Everywhere*. An `in:` term replaces the default scope; it
does not narrow it.

### Words in pictures

Pictures whose words Otto has read answer `text:`. When a query has no
`text:`, they also answer its plain words, so that typing a word seen in a
screenshot finds the screenshot whatever it is called. Every other filter
still applies to them.

### The command line

`otto-search [--in DIR]... [--limit N] [--json] QUERY...` runs a query and
prints what it found. The words of `QUERY` are joined with spaces, so a query
can be one quoted argument or several. Its default scope is home; `--in` sets
it (repeatable), and an `in:` in the query still replaces it. A relative path,
in `--in` or in `in:`, is relative to the current directory.

- It prints one path per line, best first (A to Z for `sort:name`), at most
  `--limit` (default 20). `--json` prints one object per line instead:
  `path`, `name`, `kind` (`file` or `folder`), `modified` (RFC 3339 in local
  time, or `null`), `size` in bytes (`null` for a folder) and `snippet` (the
  matched passage for `text:`, or `null`). After the files, on every exit
  status, comes one last line that is not a file:
  `{"index":{"state":…,"progress":…,"remaining_seconds":…}}`. `state` is
  `idle`, `indexing`, `paused`, `stopped` (installed, not running) or
  `missing` (not installed); `progress` runs from 0 to 1;
  `remaining_seconds` is the indexer's estimate, or `null`. Without `--json`,
  an index that is `indexing` or `paused` puts a `note:` on stderr instead,
  saying the results may be incomplete. The status is read without starting
  the indexer.
- Each problem below is printed to stderr as a hint; the search still runs.
- Exit status: `0` found something, `1` found nothing, `2` the index could not
  answer (the message says the file indexer is not running and to install or
  start LocalSearch), `3` a usage error.
- It answers from the index alone: words Otto has read in pictures are found
  by Files, not by the command.

`otto-files --search QUERY [--in DIR]` opens the same query in Files; see
[file-browser.md](./file-browser.md#starting-on-a-file-or-a-search).

### Problems

A token that does not parse as a filter is searched as a plain word, whole,
and carries a hint the interface can show:

- an unknown key: `foo:bar` ("no filter is called foo:");
- a key with no value, as while typing: `kind:`;
- a value the key does not understand: `kind:blob`, `size:big`,
  `modified:2025-02-30`;
- an exclusion that means nothing: `-sort:size`.

### Canonical form

Keys and values are written in English in their canonical form in anything
stored or sent: URIs, D-Bus calls, saved searches. Interfaces may accept and
show localised aliases, but they write the canonical form.

## Constraints & Edge Cases

- **The index lags the disk.** A result is checked against the file as it is
  now: one that no longer exists is dropped, and its time and size are read
  from the disk, not the index. An index that remembers many deleted files
  must not starve a listing: more is asked for until there are enough real
  files or a bound is reached.
- **Files the index could not read** have no type in it. They are still found
  by name, date, size and place; only type-based kinds miss them.
- **Nowhere to look** (no `in:` and no default scope) is reported as the index
  being unable to answer, not as an empty result.
- **Every term is sent to the index as a value, never as query text.** Quotes,
  braces or backslashes in what someone typed cannot change the query.
- **Full-text syntax is not exposed.** `text:` values are matched as words;
  operators such as `NEAR` or `*` are searched for literally.

## Rationale

- **Letters in order, not substrings, for names.** It is how every other
  Otto search matches (the launcher, the command palette), and a search that
  answered differently depending on where it was typed would be one people
  stop trusting. The index is asked for names with the letters in order, then
  results are ranked with names that contain each word outright first.
- **Words are independent.** People type the words they remember, not the
  order they appear in the name.
- **Plain words find names, not contents.** Content hits for common words
  would bury the file whose name was typed. Contents are asked for explicitly
  with `text:`. Words read from pictures are the exception, because Find has
  always answered from them and they are only pictures the person has seen.
- **Powers of 1024 for sizes.** A file shown as "1 MB" must be found by
  `size:>1M`.
- **Parsing never fails.** A search box that refuses a query because of a
  typo in a filter is worse than one that searches the typo.

## Open Questions

- Localised aliases for keys and values: which locales, and how chips show
  them.
- Photo and audio metadata filters (`taken:`, `camera:`, `artist:`,
  `album:`). The index holds EXIF and audio tags; place names are rare, since
  the indexer does not turn GPS coordinates into places.
