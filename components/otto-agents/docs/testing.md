# Testing

## Layers

| Layer | Lives in | What it proves | Run just this |
|---|---|---|---|
| Unit | `#[cfg(test)]` modules in `src` | JSON-RPC framing and dispatch rules, with no I/O | `cargo test --lib` |
| Sessions | `tests/sessions.rs` | The official client creates sessions against the echo backend, runs turns (started directly and queued), and reduces every envelope. It also covers rejections, `createSession` validation and root notifications | `cargo test --test sessions` |
| ACP backend | `tests/acp.rs` | `run_session` against an in-memory fake ACP agent: streaming, cancellation, and a missing agent binary. No credentials needed | `cargo test --test acp` |
| Real agent (manual) | `examples/ask.rs` | `otto-agents serve` with Claude, then `cargo run --example ask -- "…"`. Uses your Claude login, so it stays out of CI | — |
| End-to-end | `tests/handshake.rs` | A real server on an ephemeral port, driven over WebSocket by the official `ahp` + `ahp-ws` client. That client is an independent implementation, so passing means real interoperability, not agreement with ourselves | `cargo test --test handshake` |
| Spec pin | `tests/spec_pin.rs` | The vendored spec version equals the `ahp-types` version, and the vendored JSON Schemas parse | `cargo test --test spec_pin` |
| Spec drift | `scripts/sync-spec.sh --check` | `spec/upstream/` is byte-identical to the pinned upstream ref | `scripts/sync-spec.sh --check` |

Integration tests bind `127.0.0.1:0`, so they run in parallel without port clashes
and need no external services.

## Where they run

- **Locally:** `scripts/ci.sh` runs this crate's checks in one go: rustfmt, clippy
  with `-D warnings`, every test target, then the spec drift check. Set
  `SKIP_SPEC_CHECK=1` to work offline. It is a superset of what CI runs, so a
  clean run here means a clean run there.
- **GitHub Actions, [`ci.yml`](../../../.github/workflows/ci.yml):** the repository's
  one workflow. It does not call `scripts/ci.sh`; it runs the same checks as
  workspace-wide steps — `cargo fmt --all`, `cargo clippy --workspace
  --all-targets`, `cargo test --lib --workspace` for the unit tests, and a
  `cargo test -p otto-agents` step for the integration tests above. All of them
  are gated on the `ui` path filter, which any change under `components/` turns on.
- **The spec drift check runs nowhere automatically.** `scripts/sync-spec.sh --check`
  needs network access to compare against the pinned ref, so for now it is a local
  and release-time step. The [spec pin](#layers) test is what guards the version
  in CI.

The tests must pass in any locale: the service composes its dialog wording through
`i18n`, so anything asserting English calls `i18n::pin_source_locale()` first — the
shared test harnesses already do. Check with `LC_ALL=de_DE.UTF-8 cargo test -p
otto-agents`.

## Planned layers

These arrive with the milestones that need them:

- **Schema conformance** ([plan 0002](plans/0002-core-protocol-loop.md)): validate
  every message the server emits in tests against `spec/upstream/schema/*.schema.json`.
- **Reducer corpus** ([plan 0002](plans/0002-core-protocol-loop.md)): replay
  `spec/upstream/types/test-cases/reducers/*.json` through the host's authoritative
  state, so server-side state transitions match what every client computes.
- **Multi-client** ([plan 0002](plans/0002-core-protocol-loop.md)): two or more
  clients on one session, covering ordering, rejection echoes and reconnect replay.
- **ACP backend** ([plan 0003](plans/0003-acp-agent-backend.md)): a scripted fake ACP
  agent on stdio, so backend tests are deterministic and need no model credentials.
