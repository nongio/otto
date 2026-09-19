# Vendored AHP specification

`upstream/` is a verbatim copy of parts of
[microsoft/agent-host-protocol](https://github.com/microsoft/agent-host-protocol),
pinned in [`upstream.env`](upstream.env). **Do not edit it by hand.**
`scripts/sync-spec.sh` overwrites it; `scripts/sync-spec.sh --check` fails if it has
drifted from the pinned ref. That check needs network access, so it is a local and
release-time step rather than a CI one.

What CI guards, in the `ahp-conformance` job and never gated on changed paths:

- **the version**, through `tests/spec_pin.rs` — the vendored spec and the
  `ahp-types` crate the server compiles against name the same protocol version;
- **the behaviour**, through `tests/conformance.rs` — every case in
  `upstream/types/test-cases/` runs against the types and reducers this service
  is built on. The reducer corpus is compared on the state a run of actions
  lands on; the round-trip corpus is compared exactly, `null` and absent being
  different, bar whole-number floats, which upstream's own Rust harness
  normalises because Rust holds the spec's `number` as `f64`.

| Path | Upstream source | Use it for |
|---|---|---|
| `upstream/specification/` | `docs/specification/` | The normative spec (MUST / SHOULD rules) |
| `upstream/guide/` | `docs/guide/` | Explanations: state model, actions, reconciliation, AHP vs ACP |
| `upstream/schema/` | `schema/*.schema.json` | JSON Schema (2020-12) for validating wire messages in tests |
| `upstream/types/` | `types/` | TypeScript source of truth for every type and reducer, plus the language-agnostic corpora in `types/test-cases/` (`reducers/`, `round-trips/`) |
| `upstream/reference/` | generated `docs/reference/` | Per-type reference pages. Present only when `AHP_WITH_REFERENCE=1` |
| `upstream/LICENSE` | `LICENSE` | Upstream MIT license |

## Updating the spec

```sh
scripts/spec-status.sh                      # is a newer spec/vX.Y.Z release out?
scripts/sync-spec.sh --ref spec/v0.10.0     # vendor it and move the pin
# bump ahp-types / ahp / ahp-ws in Cargo.toml to the same version
scripts/ci.sh
```

The AHP Rust crates are released from the same commit as each spec tag
(`rust/vX.Y.Z` = `spec/vX.Y.Z`), so the crate version and `AHP_PROTOCOL_VERSION`
always match. `crates/otto-agents/tests/spec_pin.rs` enforces this.
