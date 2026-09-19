# 0005: Authentication

**Status:** Draft

## Goal

Authentication matters at two boundaries:

1. **Client → otto-agents.** Only trusted clients may connect. Every connection can start
   agents that run with the user's full access, so this boundary matters.
2. **Agents → their providers.** Agents need to be signed in, without credentials
   landing in config, AHP state or logs.

## What AHP defines

Spec references are under `spec/upstream/specification/`.

- **Connection gating** is a transport concern, handled in the WebSocket upgrade before
  `initialize` (`transport.md`).
- **Per-resource bearer tokens** (`authentication.md`, RFC 9728 / RFC 6750):
  - Agents declare `AgentInfo.protectedResources`, and clients push tokens with
    `authenticate { resource, token, scopes? }`.
  - Commands used without a required token fail with `AuthRequired` (`-32007`).
  - The server sends `auth/required` when a token lapses.
  - Auth state is per connection.

## Client → otto-agents

- **Loopback by default.** Bind to `127.0.0.1`.
- **Connection token, even on loopback.** Loopback is not a boundary: every local
  process, and every web page open in a browser, can reach `localhost`.
  - On first start, write a random token to `$XDG_RUNTIME_DIR/otto-agents/token` with mode
    `0600`.
  - Clients send `Authorization: Bearer <token>` on the upgrade, or `?token=` when they
    can't set headers.
  - Otto surfaces run as the same user and read the file.
- **Origin check.** Reject upgrades that carry a browser `Origin` header not on an
  allowlist.
- **Unix socket.** `$XDG_RUNTIME_DIR/otto-agents/ahp.sock` checks peer credentials (same
  UID) and needs no token. This is the preferred transport for Otto's own surfaces.
- **Remote access is opt-in.** A non-loopback `listen` requires TLS (`wss://`) and the
  token.
- **Rejection happens before AHP.** A failed handshake gets HTTP `401` before the
  upgrade completes.

## Agents → providers

- **v1: sign-in happens outside,** as in `../otto/specs/agents.md`.
  - Agents such as Claude Code keep their own login.
  - An agent that isn't signed in fails its first turn, and the reason is surfaced as
    `chat/error`.
  - No `protectedResources` are advertised, and `authenticate` for an unknown resource
    returns `InvalidParams` (`-32602`).
- **Later: sign-in through AHP.**
  - Map the auth methods ACP agents report during `initialize` to `protectedResources`
    (to be verified against the ACP spec).
  - Tokens pushed with `authenticate` go to the agent through ACP's authentication
    flow.
  - Tokens are cached in the Secret Service (`org.freedesktop.secrets`, e.g. via `oo7`)
    so the desktop doesn't ask for them again. Otto itself provides no secret storage.
  - This cache knowingly departs from AHP's per-connection model, on the grounds that
    the desktop has a single user. It is opt-in and can be cleared.
- **Expiry.** An auth failure mid-session emits `auth/required` with
  `reason: "expired"`.
- **Secrets never leak.** No secret appears in config, automation definitions
  ([0006](0006-task-spawning.md)), `_meta`, logs or telemetry.

## Testing

- **Upgrade handshake:** requests without a token, with a wrong token, with the right
  one, and with a disallowed `Origin`
- **Unix socket:** a different UID is refused, where the environment allows it
- **Unsigned agent:** the fake agent fails a turn with an auth reason, and the client
  sees `chat/error`
- **Log scrubbing:** a test asserts tokens never appear in logs

## Open questions

- Token rotation: rotate on every start, or keep the token until it is deleted?
- Is remote access (`wss://` from another device) wanted in v1?
- When sign-in arrives, should the OAuth flow run in the launcher (as an AHP
  client pushing tokens) or in the service?
