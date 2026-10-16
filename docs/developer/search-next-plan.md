# Search Next Plan

> **Status: draft, not built.** The requirements for reworking Otto's file
> search into one local service that finds files by what is in them.

## Status Quo

Today `otto-search` is a library and a command. Every program that uses it
asks LocalSearch, the desktop's file index, on its own.

- **It finds** files by name, full text and filters (kind, place, date, size),
  using the [search language](../../specs/search-language.md).
- **It is used by** Files (Find and Recent), Settings (index status) and the
  `otto-search` command. Agents call the command.
- **Words in pictures** come from Peek's OCR cache, which only Files reads.
- **It cannot** find files by meaning, search speech or video, point to a
  moment in a recording, or answer the launcher and other desktops.

What the rework means for each part:

- **Kept:** the search language.
- **Replaced:** the LocalSearch connection, the OCR cache as a search source,
  and how agents search. Whether LocalSearch itself goes is open.
- **Connected to the new service:** Files, Settings, the `otto-search`
  command, and the launcher, which gains file results.
- **New:** the service itself, its D-Bus and agent interfaces, opening media at
  a time, and idle and power signals the service can follow.

## Functional Requirements

### What is searched

- Every file under the folders the person chooses, on local disks and on
  mounted network shares. No limit on the number or size of files.
- Kinds understood by content: pictures, video, audio (music, recorded
  speech, other sounds), documents and PDFs, Markdown, plain text.
- Every other file is still found by name, place, date, size and kind.

### Queries

- **By meaning:** a description in everyday language finds matching files of
  any kind, even when none of its words appear in the file's name or text.
- **Languages:** queries and contents in at least Italian, Spanish, English
  and German find each other, whichever language each is in.
- **Moments in media:** a hit in a video or audio file names the time ranges
  where it was found, not just the file.
- **Words and filters:** full-text search and filters on date, place, kind,
  size and metadata, as in [search-language.md](../../specs/search-language.md).
  Everything that language finds today it still finds, the same way.
- **One ranking:** meaning, words and filters combine in one query and return
  one list. Filters narrow what meaning finds; they never widen it.
- **By example:** a picture, clip or recording finds similar files. Offered to
  programs through the service, not in Otto's interfaces.

### Results

- Each result carries path, name, kind, modification time and size.
- Each result says why it matched: name, a text passage, words read in it, or
  its content by meaning. A text match carries the passage.
- A media result lists its matching time ranges, best first. Opening it in a
  player starts at the first range.

### Interfaces

- Command line (`otto-search`).
- D-Bus on the session bus, including the standard desktop search-provider
  interface.
- A programmatic interface and an MCP server for agents.
- Files and the launcher.
- All of them are answered by one long-running service, the single source of
  truth. None keeps an index of its own.

### Change tracking

- Creating, changing, deleting, moving and renaming a file are all detected.
- Moving or renaming a file never makes its contents be understood again:
  identity follows the contents, the path is an attribute. Identical contents
  are understood once.
- Local disks: changes are picked up as they happen.
- Network shares: changes are picked up by periodic checks, since shares do
  not report changes made by other machines.
- A reconcile pass catches anything missed: changes made while the service was
  not running or the machine slept, and changes that arrived faster than they
  could be followed.

### Offline sources

- When a network share or removable disk is unreachable, its files stay
  searchable and are marked offline. They are not treated as deleted.
- Enough is kept on the machine for an offline result to be useful: path,
  metadata, a preview image or keyframe, and matching text or transcript
  passages.
- Opening an offline result says the source is not reachable.

### Status

- The service reports its state: idle, catching up, doing expensive work,
  waiting for mains power or for the person to be away, and how much work
  remains.
- Results say when expensive work is incomplete.

## Non-Functional Requirements

### Freshness

| Tier | Work | Target |
|---|---|---|
| 1, cheap | name, place, metadata, full text | seconds on a local disk, one check interval on a share |
| 2, expensive | understanding by meaning, transcription, reading words in pictures and video | when the resource policy allows |

A file is searchable by tier 1 before tier 2 reaches it.

### Resource policy (tier 2)

- Runs only on mains power and while the person is not using the machine.
- Always at low processor and disk priority.
- Stops at once on battery or user activity, and resumes where it stopped.

### Compute

- Runs on an ordinary laptop's processor and integrated graphics.
- The first full index of a large collection may take days; throughput is the
  main constraint, and progress survives sleeps and reboots.

### Robustness

- Resumable and crash-safe: a crash, power cut or reboot loses at most the
  file in progress, and the remaining work resumes by itself.
- Idempotent per content: the same contents are never worked on twice.
- What understood each file (model and version) is recorded with it. Changing
  it is a planned re-index; old and new are never mixed in one answer, and
  search keeps answering from the old until the new has caught up.
- Unreadable files are still found by name and metadata, and are not retried
  on every pass.

### Privacy

- Everything runs on the machine by default.
- Sending contents to an outside service is opt-in, per source or per kind of
  file, never implicit.
- The index stays on the machine, never on a network share.

### Footprint

- A lightweight service with an embedded store. No database server or storage
  service to install or run.
- The space taken by preview images and keyframes has a visible bound.

### Platform

- Linux, Wayland. Otto first; other desktops through D-Bus and the search
  provider.

## References

- [File Search](file-search.md): how search works today
- [Search plan](search-plan.md): the earlier plan for `otto-search`
- [Text in Pictures](peek-ocr.md): OCR and its word cache
- [otto-search](../../components/otto-search/AGENTS.md): the crate
- Specs: [search language](../../specs/search-language.md),
  [file browser](../../specs/file-browser.md),
  [Peek OCR](../../specs/peek-ocr.md),
  [launcher](../../specs/launcher.md)
