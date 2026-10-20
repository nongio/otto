# otto-foundations

The small rules several Otto programs have to agree on: where the XDG directories are, how a path becomes a `file://` URI and back, how typed text is scored against a name, how a desktop entry's keys are read and set, and what the login name is. No UI, no async runtime, no D-Bus.

How it works: [otto-foundations](../../docs/developer/otto-foundations.md).

- **Layer:** foundation. Used by `otto-kit` (re-exported as `otto_kit::xdg`, `otto_kit::uri`, `otto_kit::matching`, `otto_kit::desktop_file`, `otto_kit::user`), and directly by crates that do not link the toolkit: `otto-search`, `otto-agents-client`, `otto-agents`, `xdg-desktop-portal-otto`, `otto-pdf`, `otto-rdp`, `otto-msg` and `otto-render`.

## Dependencies
- **Internal:** none.
- **External:** `percent-encoding`, and nothing else.

## Develop
- Tests: `cargo test -p otto-foundations`.
- `cargo run -p otto-arch` checks that these lookups stay here. Source rules in `architecture.toml` flag, anywhere else: an `env::var("HOME")` or `XDG_*_HOME` lookup (`xdg-dirs-through-foundations`), a `"user-dirs.dirs"` path (`user-dirs-through-foundations`), an `env::var("USER")`, `LOGNAME` or locale variable lookup (`user-and-locale-through-foundations`; the locale itself is otto-kit's `i18n`, and the compositor's `src/locale_env.rs`), and a `"[Desktop Entry]"` literal (`desktop-entry-through-foundations`).

## Where things are
- `xdg`: the base directories, `home`, Otto's config paths, `user-dirs.dirs`, and the `~/…` spelling of a path (`tilde`).
- `uri`: `file://` URIs over a path's bytes, with a lenient and a strict decoder.
- `matching`: subsequence scoring for typed queries, shared by the launcher, every toolkit list and file search.
- `desktop_file`: the `[Desktop Entry]` group of a desktop entry's text, read (`value`, `entries`) and edited in place (`with_value`). Looking an app up by id is otto-kit's `desktop_entry`, not this.
- `user`: the login name (`login_name`).

## Rules
- An XDG variable is used only when it holds an absolute path. A relative value is invalid under the spec, and an empty one is how a shell spells "unset". In either case the usual folder under the home folder is used. `runtime_dir` has no such fallback: it returns `None` instead of making up a directory that nothing cleans up.
- A `user-dirs.dirs` entry is `$HOME`, `$HOME/<rest>` or an absolute path. Any other value is skipped, a relative path included.
- `desktop_file` reads only the `[Desktop Entry]` group, takes the first of a duplicated key, and returns values as written (escapes and `Exec` quoting are the caller's). `with_value` keeps every other line, comments included.
- `login_name` is `$USER`, then `$LOGNAME`; an empty value counts as unset.
- `decode_path` treats a `%` that is not followed by two hex digits as a literal `%`, because file names contain them. `try_decode_path` returns `None` for one; use it on a wire where both ends are Otto's. Neither decoder takes a sign as a hex digit, so `%+f` is not an escape.
- A helper belongs here when a program that does not link otto-kit needs it, and it needs only the standard library. A helper that draws, talks D-Bus or needs a runtime belongs in otto-kit, otto-dbus or the crate that uses it.

## See also
- [XDG specifications](../../docs/developer/xdg-specifications.md), [Architecture](../../docs/developer/architecture.md).
- [otto-kit](../otto-kit/AGENTS.md), [otto-dbus](../otto-dbus/AGENTS.md).
