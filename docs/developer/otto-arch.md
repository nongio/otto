# otto-arch

`otto-arch` checks the workspace against the rules in `architecture.toml`: crate layers, the compositor's module boundaries, and patterns that belong in exactly one place. It also generates the crate graph in [architecture.md](architecture.md) and, with `affected`, tells CI which crates a change reaches. It is a command-line tool run by developers and by CI's `architecture` and `changes` jobs.

It sits in the **tool** layer (see [Architecture Rules](architecture.md)) and is used by nothing.

## Dependencies

- **Internal:** none. It reads the workspace through `cargo metadata`, not as a dependency.
- **External:** pure Rust with no system libraries, so CI runs it before the compositor is built. `guppy` and `determinator` build and diff the package graphs; `syn` parses `src/` for module paths. `affected` needs `git`, and checks the base revision out in a temporary worktree.

## Develop

- `cargo run -p otto-arch` checks. `cargo run -p otto-arch -- --bless` also rewrites the graph in `docs/developer/architecture.md`; run it after adding a crate or changing a dependency, and commit both.
- `cargo run -q -p otto-arch -- affected --base origin/main [--head REV] --explain` says which crates a change reaches and why. With `--test-args` it prints arguments for `cargo test`: `cargo run -q -p otto-arch -- affected --base origin/main --test-args | xargs -r cargo test`.
- Tests: `cargo test -p otto-arch`. They include a check of the real workspace, so a violation fails the tests too. CI lints it with `cargo clippy -p otto-arch --all-targets -- -D warnings`.
- Third-party policy (who may link Smithay, GStreamer, PipeWire) is not this tool's job. It lives in `deny.toml`: `cargo deny --workspace check bans -D unused-wrapper`. Without `--workspace` cargo-deny judges only the compositor.

## Code map

- [src/main.rs](../../tools/otto-arch/src/main.rs): arguments and the two modes.
- [src/rules.rs](../../tools/otto-arch/src/rules.rs): the shape of `architecture.toml`.
- [src/workspace.rs](../../tools/otto-arch/src/workspace.rs): crates and the resolved graph from `cargo metadata`.
- [src/check.rs](../../tools/otto-arch/src/check.rs): the layer, module, source-pattern and lint checks.
- [src/docs.rs](../../tools/otto-arch/src/docs.rs): the generated graph between the `otto-arch:graph` markers.
- [src/affected.rs](../../tools/otto-arch/src/affected.rs): the affected-crate selection and CI outputs.

## Rules

- An `[[allow]]` entry is debt, not permission: it needs a reason, and a stale one fails the check. Fix code rather than add entries.
- A change here or to `architecture.toml` makes CI check everything, so the selection cannot shrink the checks it is judged by.
- The "everything" outputs in `src/affected.rs` must match the fallback `.github/workflows/ci.yml` writes when the tool cannot run; a test keeps them in step. The list of crates with their own clippy steps there (`OWN_CLIPPY`) follows `ci.yml` too.
- Any failure in `affected` selects everything, never nothing.

## See also

- [Architecture](architecture.md): the layers, module rules, `[[affects]]` rules and how to change them.
- [architecture.toml](../../architecture.toml) and [deny.toml](../../deny.toml)
