# Testing

## Layers

| Layer | Lives in | What it proves | Run just this |
|---|---|---|---|
| Unit | `#[cfg(test)]` modules in `crates/*/src` | JSON-RPC framing and dispatch rules, with no I/O | `cargo test --lib` |
| Sessions | `crates/otto-ahp/tests/sessions.rs` | The official client creates sessions against the echo backend, runs turns (started directly and queued), and reduces every envelope. It also covers rejections, `createSession` validation and root notifications | `cargo test --test sessions` |
| ACP backend | `crates/otto-ahp/tests/acp.rs` | `run_session` against an in-memory fake ACP agent: streaming, cancellation, and a missing agent binary. No credentials needed | `cargo test --test acp` |
| Real agent (manual) | `examples/ask.rs` | `otto-ahp serve` with Claude, then `cargo run --example ask -- "…"`. Uses your Claude login, so it stays out of CI | — |
| End-to-end | `crates/otto-ahp/tests/handshake.rs` | A real server on an ephemeral port, driven over WebSocket by the official `ahp` + `ahp-ws` client. That client is an independent implementation, so passing means real interoperability, not agreement with ourselves | `cargo test --test handshake` |
| Spec pin | `crates/otto-ahp/tests/spec_pin.rs` | The vendored spec version equals the `ahp-types` version, and the vendored JSON Schemas parse | `cargo test --test spec_pin` |
| Spec drift | `scripts/sync-spec.sh --check` | `spec/upstream/` is byte-identical to the pinned upstream ref | `scripts/sync-spec.sh --check` |

Integration tests bind `127.0.0.1:0`, so they run in parallel without port clashes
and need no external services.

## Where they run

- **Locally:** `scripts/ci.sh` runs the same steps as CI: rustfmt, clippy with
  `-D warnings`, all tests, then the spec drift check. Set `SKIP_SPEC_CHECK=1` to
  work offline.
- **GitHub Actions, [`ci.yml`](../.github/workflows/ci.yml):** runs `scripts/ci.sh`
  on every push to `main` and on every pull request.
- **GitHub Actions, [`spec-watch.yml`](../.github/workflows/spec-watch.yml):** runs
  weekly (or manually) and fails when upstream publishes a newer `spec/vX.Y.Z` tag.

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
