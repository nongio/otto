# otto-search

Otto's file search: one query language over the desktop's file index, so the
same words find the same files in Files, Settings and on the command line. It
is a library that turns a query into SPARQL and asks LocalSearch over D-Bus,
plus the `otto-search` command.

It sits in the **service** layer (see [Architecture Rules](architecture.md))
and is used by `otto-files` and `otto-settings`. How Files drives it (the
worker thread, Recent, words read from pictures) is on
[File Search](file-search.md).

## Dependencies

- **Internal:** [otto-foundations](otto-foundations.md), for name scoring
  (`matching`) and the index's `file:` URLs (`uri`).
- **External:** LocalSearch on the session bus
  (`org.freedesktop.LocalSearch3`, the `org.freedesktop.Tracker3.Endpoint`
  SPARQL endpoint and the files miner). `zbus` for the wire.

## Develop

- `cargo run -p otto-search -- invoice kind:pdf modified:<30d` runs a query.
  Flags: `--in DIR` (repeatable), `--limit N`, `--json`.
- `cargo test -p otto-search`.
- `cargo test -p otto-search --test live_index -- --ignored --nocapture`
  checks every filter against the real index. It needs LocalSearch running,
  and what it finds depends on the machine.

## Code map

- `src/query.rs`: text to terms. It never fails.
- `src/plan.rs`: a query resolved against a place and a time, rendered as
  SPARQL or checked against one file.
- `src/index.rs`: the LocalSearch wire, its status and its errors.
- `src/find.rs`: a plan run to a ranked list of files that are really on
  disk, paging past rows the index remembers wrongly.
- `src/dates.rs`: calendar arithmetic, for anything that shows a date
  without a date crate.
- `src/main.rs`: the `otto-search` command.

## Rules

- No toolkit and no Wayland, so command-line tools and daemons can link it.
- The query syntax is specified in `specs/search-language.md`. A change to
  the language updates the spec in the same change.

## See also

[File Search](file-search.md), [Search plan](search-plan.md),
[Search language spec](../../specs/search-language.md),
[Files user guide](../user/files.md).
