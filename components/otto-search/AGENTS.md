# otto-search

Otto's file search: one query language over the desktop's file index, so the same words find the same files in Files, Settings and on the command line. It is a library that turns a query into SPARQL and asks LocalSearch over D-Bus, plus the `otto-search` command.

How it works: [File Search](../../docs/developer/file-search.md).

- **Layer:** service. Used by `otto-files` and `otto-settings`.

## Dependencies
- **Internal:** `otto-foundations`, for name scoring (`matching`) and the index's `file:` URLs (`uri`).
- **External:** LocalSearch on the session bus (`org.freedesktop.LocalSearch3`, the `org.freedesktop.Tracker3.Endpoint` SPARQL endpoint and the files miner). `zbus` for the wire.

## Develop
- `cargo run -p otto-search -- invoice kind:pdf modified:<30d` runs a query. Flags: `--in DIR` (repeatable), `--limit N`, `--json`.
- Tests: `cargo test -p otto-search`.
- `cargo test -p otto-search --test live_index -- --ignored --nocapture` checks every filter against the real index. It needs LocalSearch running, and what it finds depends on the machine.

## Where things are
- `src/query.rs` turns text into terms and never fails; `src/plan.rs` resolves them against a place and a time, as SPARQL or as a check on one file.
- The LocalSearch wire, its status and its errors are in `src/index.rs`.
- `src/find.rs` runs a plan to a ranked list of files that are really on disk, paging past rows the index remembers wrongly.
- `src/dates.rs` is calendar arithmetic without a date crate; `src/main.rs` is the command.

## Rules
- No toolkit and no Wayland, so command-line tools and daemons can link it.
- The query syntax is specified in `specs/search-language.md`. A change to the language updates the spec in the same change.

## See also
- [File Search](../../docs/developer/file-search.md), [Search plan](../../docs/developer/search-plan.md).
- [Search language spec](../../specs/search-language.md), [Files user guide](../../docs/user/files.md).
- [otto-foundations](../otto-foundations/AGENTS.md).
