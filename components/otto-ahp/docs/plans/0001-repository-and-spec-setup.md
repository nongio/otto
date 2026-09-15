# 0001: Repository and spec setup

**Status:** Done

## Goal

A repository where protocol work can start immediately: the spec is on disk at a known
version, one command checks everything, and a minimal server proves the toolchain end
to end.

## Decisions

- **Vendor the spec, don't use a submodule.** `spec/upstream/` is a plain copy, so the
  spec is greppable, readable offline and visible in review diffs when the pin moves.
  `scripts/sync-spec.sh --check` in CI stops hand edits and silent drift.
- **Pin to spec release tags.** Upstream tags `spec/vX.Y.Z` and `rust/vX.Y.Z` on the
  same commit. Pinning the spec tag and the `ahp-types` crate to the same version means
  the prose we read and the types we compile against always agree.
  `tests/spec_pin.rs` enforces this.
- **Vendor the TypeScript types and test corpora, not just the prose.** `types/*.ts` is
  upstream's source of truth, and `types/test-cases/` holds language-agnostic reducer and
  round-trip fixtures we can reuse for conformance tests.
- **Reference docs are optional.** They are generated with Node (`npm run
  generate:docs`) and duplicate `types/`, so they are off by default
  (`AHP_WITH_REFERENCE=0`).
- **Reuse `ahp-types` for wire types.** No hand-written protocol structs. The server
  advertises only the pinned protocol version, not the crate's whole fallback list,
  because the generated types describe exactly one version.
- **WebSocket transport.** The spec doesn't mandate a transport, but WebSocket text
  frames are what VS Code and the official clients use.
- **Test against the official client.** End-to-end tests drive the server with the
  official `ahp` + `ahp-ws` crates rather than a client we wrote ourselves.

## Layout

```
crates/otto-ahp/     server library + `otto-ahp` binary
  src/rpc.rs         JSON-RPC framing
  src/host.rs        host state and per-connection dispatch
  src/server.rs      WebSocket accept loop
  tests/             end-to-end and spec-pin tests
spec/                vendored upstream spec and its pin (upstream.env)
scripts/             sync-spec.sh, spec-status.sh, ci.sh
docs/                plans/ and testing.md
.github/workflows/   ci.yml, spec-watch.yml
```

## Delivered

- [x] Cargo workspace, toolchain file and `.gitignore`
- [x] `scripts/sync-spec.sh` with sync, `--ref`, `--check` and `--with-reference` modes
- [x] `scripts/spec-status.sh` to detect newer upstream releases
- [x] `scripts/ci.sh` as the single local and CI entry point
- [x] Spec vendored at `spec/v0.9.0`
- [x] Minimal server: `initialize` (version negotiation, `-32005` then close), `ping`,
      `subscribe` / `unsubscribe` on `ahp-root://`, empty `listSessions`,
      `MethodNotFound` for everything else
- [x] Unit, end-to-end and spec-pin tests
- [x] GitHub Actions CI and weekly spec watch

## Out of scope

Sessions, chats, action sequencing, reconnect and agents are covered in
[0002](0002-core-protocol-loop.md) and [0003](0003-acp-agent-backend.md).
