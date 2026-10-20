# otto-term-kit

The terminal interfaces of Otto's command-line tools, so `otto-msg doctor` and `otto-setup` look and behave the same: a header with Otto's mark and the tool's name, a rule, a list to move through, and a help line on the last row. Underneath is the terminal itself, hand-rolled over termios and ANSI escapes: raw mode, the alternate screen, key parsing and the size.

- **Layer:** kit. Used by `otto-msg` (doctor) and `otto-setup`.

## Dependencies
- **Internal:** none.
- **External:** `libc` (termios, `poll`, `ioctl`). No terminal UI crate, on purpose: what the tools draw is a few styled lines, and the whole terminal layer is smaller than one.

## Develop
- Tests: `cargo test -p otto-term-kit`: key parsing and line cutting.
- An interface built on it needs a terminal on stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`), not in your own terminal session.

## Where things are
- `src/term.rs`: `Terminal` (raw mode and the alternate screen, put back on drop and on panic), `Key`, `size`, `is_tty`.
- `src/line.rs`: `Spans`, one screen line built from styled pieces and cut at the width; `printable`.
- `src/lib.rs`: the styles, and the shared look: `header`, `rule`, `compose`.

## Rules
- Colours are the terminal's own palette (bold, dim, reverse video, the standard colours), never RGB, so the tools follow the terminal's light or dark theme.
- Text from outside (files, logs, command output) goes through `printable` before it is drawn: it must not move the cursor or change colours.
- The terminal is always put back, panic included.

## See also
- [otto-msg](../otto-msg/AGENTS.md), [otto-setup](../otto-setup/AGENTS.md).
