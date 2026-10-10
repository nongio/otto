# otto-foundations: the UI-free base

`components/otto-foundations/` holds the small rules that several Otto
programs have to agree on: where the XDG directories are, how a path becomes
a `file://` URI and back, and how typed text is scored against a name. It has
no UI, no async runtime and no D-Bus. Its one dependency is
`percent-encoding`.

| Module | What it does |
|--------|--------------|
| `xdg` | The XDG base directories (`config_home`, `data_home`, `state_home`, `cache_home`, `runtime_dir`, `data_dirs`, `config_dirs`), `home`, Otto's config paths, the user's folders from `user-dirs.dirs` (`user_dirs`, `user_dir`, `parse_user_dirs`), and the `~/…` spelling of a path (`tilde`, `tilde_in`) |
| `uri` | `file://` URIs over a path's bytes: `path_to_uri`, `path_to_glib_uri` (the thumbnail standard's form), `uri_to_path`, `encode_path`, and two decoders: a lenient `decode_path` and a strict `try_decode_path` |
| `matching` | Subsequence scoring for typed queries (`score`, `positions`, `word_positions`), shared by the launcher, every toolkit list and file search |

## Who links it

otto-kit re-exports all three modules as `otto_kit::xdg`, `otto_kit::uri`
and `otto_kit::matching`, and toolkit apps import them from there. Crates
that do not link the toolkit depend on otto-foundations directly:

- `otto-search` scores names with `matching`, and makes and reads the
  index's `file:` URLs with `uri`.
- `otto-agents-client` and `otto-agents` take the runtime directory, the
  base directories and `tilde` from `xdg`, and file URIs from `uri`.
- `xdg-desktop-portal-otto` finds the Pictures folder with
  `xdg::user_dir` and hands out screenshot URIs with `uri::path_to_uri`.

Before this crate existed otto-kit depended on otto-search, a crate with
zbus and tokio, only for name matching. The portal and the agents crates
each carried their own copies of these helpers, and the copies did not
always behave the same way.

## The rules

- An XDG variable is used only when it holds an absolute path. A relative
  value is invalid under the spec, and an empty one is how a shell spells
  "unset". In either case the usual folder under the home folder is used.
  `runtime_dir` has no such fallback. It returns `None` instead of making up
  a directory that nothing cleans up.
- A `user-dirs.dirs` entry is `$HOME`, `$HOME/<rest>` or an absolute path.
  Any other value is skipped, a relative path included.
- `decode_path` treats a `%` that is not followed by two hex digits as a
  literal `%`, because file names contain them. `try_decode_path` returns
  `None` for one; use it on a wire where both ends are Otto's. Neither
  decoder takes a sign as a hex digit, so `%+f` is not an escape.

## Adding to it

A helper belongs here when a program that does not link otto-kit needs it,
and it needs only the standard library. A helper that draws, talks D-Bus or
needs a runtime belongs somewhere else: otto-kit, otto-dbus or the crate
that uses it.
