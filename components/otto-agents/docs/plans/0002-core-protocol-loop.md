# 0002: Core protocol loop

**Status:** Draft

## Goal

A host that multiple clients can share: sessions and chats exist, every mutation is
sequenced and broadcast, client actions are validated, and a dropped client can
reconnect without losing state. Agents are still stubbed; real agents come in
[0003](0003-acp-agent-backend.md).

## Scope

Spec references are relative to `spec/upstream/specification/`.

- **Channels:** root, session and chat (`root-channel.md`, `session-channel.md`,
  `chat-channel.md`)
- **Commands:** `createSession`, `disposeSession`, `listSessions` (with pagination),
  `createChat`, `fetchTurns`
- **Notifications out:** `action` envelopes, plus `root/sessionAdded`,
  `root/sessionRemoved` and `root/sessionSummaryChanged`
- **Notifications in:** `dispatchAction`, validated per the "Server Validation of
  Client Actions" tables. Invalid actions are echoed with `rejectionReason`; actions on
  unknown channels are silently ignored.
- **Lifecycle:** `reconnect`, with a replay result when the gap fits the buffer and
  snapshots otherwise, and `missing` for disposed channels (`lifecycle.md`)
- **Pending messages:** queued-message consumption when a turn completes
  (`chat-channel.md`)

Out of scope: terminals, changesets, `resource*`, authentication, automations,
telemetry.

## Design sketch

- **One writer.** A single host task owns all state and applies mutations in order.
  That gives a total order for `serverSeq` without locks across await points.
  Connections send it commands over an mpsc channel and receive envelopes over their
  own outbound channel.
- **Reuse upstream reducers.** Apply actions with `ahp::reducers::apply_action_to_*`,
  the same reducers clients run. Server and client state then match by construction,
  and a `ReduceOutcome::Invalid` result feeds the rejection path.
- **Replay buffer.** A bounded ring of recent envelopes, keyed by `serverSeq`.
  `reconnect` replays from it when `lastSeenServerSeq` is still covered and falls back
  to snapshots otherwise.
- **Atomic snapshots.** Read `fromSeq` and the state under the same critical section,
  so no action is lost or duplicated between snapshot and stream.
- **Stub agent.** An `Agent` trait whose first implementation echoes the user message,
  so turns run end to end (`chat/turnStarted`, then `chat/responsePart` / `chat/delta`,
  then `chat/turnComplete`).

## Testing

- **Schema conformance:** every emitted message validated against
  `spec/upstream/schema`
- **Reducer corpus:** `spec/upstream/types/test-cases/reducers` replayed through the
  host's state
- **Multi-client scenarios:** a turn started by one client is seen by another;
  first-wins tool confirmation; cancellation
- **Reconnect:** both the replay path and the snapshot fallback

## Open questions

- Replay buffer size, and whether it is global or per channel
- Whether sessions persist across restarts (the spec only requires in-memory)
- Whether to honour `delivery.maxLatencyMs` coalescing now or later
