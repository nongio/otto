# otto-md-kit

Otto's Markdown parser. It takes a `&str` and returns the block vocabulary the toolkit's document renderer draws (`otto_kit::preview::document`), so a `.md` file reads as a document rather than as its source. It is a reading parser, not a conforming one, and it never produces HTML.

- **Layer:** kit. Used by `otto-peek`, which parses in its sandboxed decode worker, and `otto-agents-kit`.

## Dependencies
- **Internal:** `otto-kit`, only for the block vocabulary (`Block`, `Span`, `SpanStyle`, re-exported here).
- **External:** none.

## Develop
- Tests: `cargo test -p otto-md-kit`.
- `cargo run -p otto-md-kit --example markdown_window -- README.md` draws a file in a window; with no path it shows a built-in sample. Against a dev compositor: `cargo run -- --winit &`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-md-kit --example markdown_window`.

## Where things are
- `src/lib.rs` is the entry (`parse_capped`, `source_text`), the limits and the re-exported vocabulary.
- `src/block.rs` parses the block structure (headings, lists, quotes, code, rules).
- `src/inline.rs` parses the spans inside a block (emphasis, code, links).

## Rules
- No dependencies beyond the toolkit, and no files, network or runtime. That is what lets it run inside a worker with no descriptors.
- A construct it does not handle degrades to a paragraph. Tables are drawn as code blocks and images as their alt text, on purpose.
- The input is untrusted. Keep the `MAX_BLOCKS`, `MAX_LINE_BYTES` and `MAX_INDENT` caps.

## See also
- [Peek spec](../../specs/peek.md), [File previews](../../docs/developer/file-previews.md).
- [otto-kit](../otto-kit/AGENTS.md), [otto-peek](../otto-peek/AGENTS.md).
