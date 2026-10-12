# otto-agents-client

What `otto-agents` and its clients must agree on: how to reach the service's socket, what a session URI looks like, and how a `file://` URI maps to a path. A small library over the Agent Host Protocol crates and a socket, with nothing of the service itself in it.

How it works: [Agents](../../docs/developer/agents.md).

- **Layer:** service. Used by `otto-agents`, `otto-agents-kit` and `otto-preview`.

## Dependencies
- **Internal:** `otto-foundations`, for `runtime_dir` and file URIs.
- **External:** `ahp`, `ahp-types` and `ahp-ws` (the Agent Host Protocol), and `tokio-tungstenite` for WebSocket framing over the Unix socket at `$XDG_RUNTIME_DIR/otto-agents/agents.sock`.

## Develop
- Tests: `cargo test -p otto-agents-client`.
- Clients connect to `default_url()` unless `OTTO_AGENTS_URL` names another service. A URL is `unix:///path/to/agents.sock`, a bare absolute path, or `ws://` (loopback `127.0.0.1:4800` when there is no runtime directory).

## Where things are
- `src/transport.rs`: the socket path, `default_url`, and `connect`, which returns an AHP transport.
- `src/session.rs`: `ahp-session:/` URIs, and picking one session out of a list by a typed prefix.
- `src/uri.rs`: file URIs to paths and back.

## Rules
- The AHP pins (`ahp`, `ahp-types`, `ahp-ws` at `=0.9.0`) match `otto-agents` and the workspace exactly: this crate is the wire between them. Bump them together.
- It stays a leaf: no dependency on `otto-agents` or on any UI crate.
- The socket's directory, readable only by the user, is the access check. A `ws://` loopback URL is for development and tests only.

## See also
- [Agents](../../docs/developer/agents.md).
- [otto-agents](../otto-agents/AGENTS.md), [otto-agents-kit](../otto-agents-kit/AGENTS.md), [otto-foundations](../otto-foundations/AGENTS.md).
