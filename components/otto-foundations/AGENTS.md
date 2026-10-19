# otto-foundations

The small rules several Otto programs have to agree on: where the XDG directories are, how a path becomes a `file://` URI and back, and how typed text is scored against a name. No UI, no async runtime, no D-Bus.

How it works: [otto-foundations](../../docs/developer/otto-foundations.md).

- **Layer:** foundation. Used by `otto-kit` (re-exported as `otto_kit::xdg`, `otto_kit::uri`, `otto_kit::matching`), and directly by crates that do not link the toolkit: `otto-search`, `otto-agents-client`, `otto-agents`, `xdg-desktop-portal-otto`, `otto-pdf`, `otto-rdp` and `otto-render`.

## Dependencies
- **Internal:** none.
- **External:** `percent-encoding`, and nothing else.

## Develop
- Tests: `cargo test -p otto-foundations`.
- `cargo run -p otto-arch` checks that XDG lookups stay here: the `xdg-dirs-through-foundations` source rule in `architecture.toml` flags an `env::var("HOME")` or `XDG_*_HOME` lookup anywhere else.

## Where things are
- `xdg`: the base directories, `home`, Otto's config paths, `user-dirs.dirs`, and the `~/…` spelling of a path (`tilde`).
- `uri`: `file://` URIs over a path's bytes, with a lenient and a strict decoder.
- `locale`: the locales the environment asks messages in, in gettext's order (`LANGUAGE` first, ignored under `C`). otto-kit and otto-agents map them to catalogues.
- `matching`: subsequence scoring for typed queries, shared by the launcher, every toolkit list and file search.

## Rules
- An XDG variable is used only when it holds an absolute path. A relative value is invalid under the spec, and an empty one is how a shell spells "unset". In either case the usual folder under the home folder is used. `runtime_dir` has no such fallback: it returns `None` instead of making up a directory that nothing cleans up.
- A `user-dirs.dirs` entry is `$HOME`, `$HOME/<rest>` or an absolute path. Any other value is skipped, a relative path included.
- `decode_path` treats a `%` that is not followed by two hex digits as a literal `%`, because file names contain them. `try_decode_path` returns `None` for one; use it on a wire where both ends are Otto's. Neither decoder takes a sign as a hex digit, so `%+f` is not an escape.
- A helper belongs here when a program that does not link otto-kit needs it, and it needs only the standard library. A helper that draws, talks D-Bus or needs a runtime belongs in otto-kit, otto-dbus or the crate that uses it.

## See also
- [XDG specifications](../../docs/developer/xdg-specifications.md), [Architecture](../../docs/developer/architecture.md).
- [otto-kit](../otto-kit/AGENTS.md), [otto-dbus](../otto-dbus/AGENTS.md).
