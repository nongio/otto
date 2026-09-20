# Where we stand against the AHP spec

A reading of the whole vendored specification against what this service does,
taken at spec v0.9.0. It is a standing assessment, not a plan: the plans in
[plans/](plans/README.md) say what happens next.

Two questions are kept apart here. **Coverage** is how much of the protocol
surface we serve, and the answer is "a little, deliberately". **Doctrine** is
whether we agree with what the protocol is *for*, and the answer is "yes, bar
one thing" — the handover.

## Coverage

The state model is complete, because it is not ours: wire types come from
`ahp-types` at the pinned version, every mutation goes through `ahp::reducers`,
and `tests/conformance.rs` runs upstream's reducer and round-trip corpora over
all nine channel reducers. What is partial is the command and action surface.

Commands: the spec defines 32 methods, we answer 8 — `ping`, `initialize`, `subscribe`,
`unsubscribe`, `listSessions`, `createSession`, `disposeSession` and
`dispatchAction`. `initialize` negotiates the version properly, closing the
connection when nothing overlaps, honours `initialSubscriptions`, and returns
`serverSeq`, `serverInfo` and `defaultDirectory`.

| Not served | What it costs us |
|---|---|
| `reconnect` | A dropped socket loses the session view; see [Replay](#replay-is-the-one-gap-that-is-not-scope) |
| `fetchTurns` | We emit `chat/turnsLoaded`, but nothing asks for it: a client cannot page history |
| `createChat`, `disposeChat` | One chat per session, reachable as `defaultChat` |
| `authenticate`, `auth/required` | No authentication in the protocol; the socket's owner is the access check |
| the nine `resource*` commands, `createResourceWatch` | No filesystem surface; agents read files with their own tools |
| `resolveSessionConfig`, `sessionConfigCompletions`, `completions` | Configuration is ours, through `_meta` |
| terminals, MCP, customizations, annotations, automations, changesets, comments, telemetry | Eight channel families, seven specification chapters, untouched |

Actions: around 30 of 113. The chat turn lifecycle is whole
(`turnStarted`, `delta`, `reasoning`, `responsePart`, `turnComplete`,
`turnCancelled`, `truncated`), tool calls run start → ready → complete →
confirmed, and elicitation is there
(`chat/inputRequested`, `inputAnswerChanged`, `inputCompleted`). Missing and
worth noting: `chat/toolCallDelta` and `toolCallContentChanged`, so a client
cannot watch a tool call's parameters fill in; `chat/activityChanged`;
`chat/usage`; `chat/draftChanged`; `chat/workingDirectorySet`/`Removed`; and
`root/agentsChanged`, our agent list being fixed at startup.

None of that is a compliance failure. The doctrine asks for exactly this
on-ramp: "a minimal host should be able to expose a useful agent experience
with root and session channels, session creation, basic turns, and state
updates."

## Two methods that are not in the spec

`host.rs` serves `releaseSession` and `setMode`. Neither appears anywhere in
`spec/upstream/`. A stock AHP client does not know them and a stock AHP server
answers `method not found`, so both work only between our own two sides. The
spec's escape hatch is `_meta`, which we already use for `otto.modes` — so
these would sit better as `_meta` state plus a dispatched action, or as a
proposal upstream.

## Doctrine

Where we agree, and it is most of it:

- **State-first.** Every mutation goes through the same reducers clients run,
  under the one lock that assigns `serverSeq`. Truth is the state, not the
  event that caused it.
- **Host-authoritative, client-responsive.** The service owns the one ACP
  connection; clients apply optimistically and reconcile. The launcher stores
  nothing and a session outlives every window that looks at it.
- **Above ACP, not instead of it.** Advertising no `fs/*` or `terminal/*`
  client capability keeps the agent's implementation behind the host boundary,
  which is the line the doctrine draws.
- **Escape hatches stay explicit.** `otto.modes`, `otto.terminal` and
  `otto.defaultOption` live in `_meta` rather than in invented typed fields.
  The doctrine allows this and warns that anything interoperable clients come
  to need should graduate into typed state.

### The handover is a real divergence

`releaseSession` has the host step out. otto-agents stops its own agent once
idle, the terminal's agent becomes the only writer, and this service never sees
those turns.

AHP has no seat for that. It assumes the host is the only thing that ever talks
to the agent — that assumption is what makes its state authoritative and
replayable. Its vocabulary for who is driving is `activeClients`, with
`session/activeClientSet` and `activeClientRemoved`, and it is host-managed:
clients contend for attention, they never take custody.

So this is a difference of axioms, not a missing feature. Otto's is that the
agent's own history is the session, with otto-agents as one of two
interchangeable front-ends; AHP's is that the host's state is the session.
Ours fits terminal-first harnesses that already keep a durable store — every
agent tried so far advertises `loadSession` — but it is off-doctrine, and while
a session is out in a terminal it is invisible to every AHP client until
someone reopens it. Worth either proposing upstream as a lifecycle, with
`activeClients` naming the terminal, or stating plainly as an Otto extension.

### "Terminal" means opposite things

AHP's terminal channel is a pty the host allocated for an agent, surfaced to
clients: its own URI and state, scrollback as content parts, `terminal/data`
out, `terminal/input` in, ownership as a claim that moves between a session and
a client. It pulls a pty into the protocol.

Otto's terminal is the person's emulator opening the agent's own interface and
taking the session over. It pushes a session out of the protocol. We implement
none of the spec's terminal machinery and our handover uses none of it. Both
are coherent; they only share a word.

### Replay is the one gap that is not scope

Replayability is one of the doctrine's load-bearing properties, and `reconnect`
is the command that delivers it. Plan
[0002](plans/0002-core-protocol-loop.md) deferred it and it has not come back.
Replaying an agent's history on `session/load` is us rebuilding state through
the agent because the protocol path for it is not there.
