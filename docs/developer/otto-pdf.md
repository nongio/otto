# otto-pdf

Editing a PDF in place: reading a document's form fields and annotations in
the page's own coordinates, applying edits (a field's value, an annotation
added, moved, recoloured or removed), and saving them over the file. A
viewer that has already drawn a page uses it to put controls over the page
and write back what the person did with them. The work is PDFium's.

It sits in the **service** layer (see [Architecture Rules](architecture.md))
and is used by `otto-preview`.

## Dependencies

- **Internal:** [otto-foundations](otto-foundations.md), for the home folder
  in the library search.
- **External:** PDFium (`libpdfium.so`), loaded with `libloading` the first
  time it is asked for and never linked. A system without it runs everything
  except the editing; `otto_pdf::available()` says which. The places looked
  in, in order: `$OTTO_PDFIUM`, beside the executable,
  `<exe dir>/../lib/otto/`, `/usr/lib/otto/`, `/usr/local/lib/otto/`,
  `~/.local/lib/otto/`, then the system loader.
  `scripts/packaging/fetch-pdfium.sh` fetches the pinned prebuilt library.

## Develop

- `cargo test -p otto-pdf`. The tests write their own PDFs
  (`src/tests/writer.rs`) and skip, saying so, when PDFium cannot be loaded.
  Point `OTTO_PDFIUM` at a library to run them.
- `OTTO_PDF_SAMPLES=DIR` keeps a copy of each test output for looking at.
- An ignored test fills and marks up a real document:
  `OTTO_PDF_REAL=FILE OTTO_PDF_SAMPLES=DIR cargo test -p otto-pdf -- --ignored`.

## Code map

- `src/lib.rs`: the public types (`Rect`, `Field`, `Annotation`, `Edit`,
  `Snapshot`, …).
- `src/worker.rs`: the thread that owns PDFium, the `Document` handle other
  threads use, and the atomic save.
- `src/engine.rs`: reading and editing a document through PDFium.
- `src/geometry.rs`: shown-page coordinates to PDFium's and back.
- `src/library.rs`: finding and loading `libpdfium.so`.
- `src/bindings.rs`: the FFI table.

## Rules

- PDFium keeps global state and is not thread-safe. Only the worker thread
  calls it; everything else sends work through a `Document`.
- `src/bindings.rs` is hand-written against the chromium/8086 headers and
  holds only the functions the engine uses. A new call adds its binding
  there and keeps the safety contract written at the top of that file.
- Coordinates are PDF points from the top-left of the page as shown (its crop
  box, turned by `/Rotate`). Only the engine converts to PDFium's bottom-up
  space.
- A save writes a new file beside the target (`O_NOFOLLOW | O_CLOEXEC`),
  syncs it and renames it over the old one.

## See also

[Preview app spec](../../specs/preview-app.md),
[File previews](file-previews.md).
