# otto-arch

The checker for `architecture.toml` and the affected-crate selection CI runs on.

Read the developer guide first: [docs/developer/otto-arch.md](../../docs/developer/otto-arch.md).

- After adding a crate or a workspace dependency, run `cargo run -p otto-arch -- --bless` and commit the regenerated graph.
