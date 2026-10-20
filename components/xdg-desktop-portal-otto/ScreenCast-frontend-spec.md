# ScreenCast Frontend Specification

> Based on the upstream `org.freedesktop.portal.ScreenCast` interface (v5): <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html>

## Scope

The upstream `xdg-desktop-portal` project exports the ScreenCast portal
interface (`org.freedesktop.portal.ScreenCast`) on behalf of applications. This
crate implements the backend that those requests are delegated to, but we keep a
local copy of the frontend specification for reference. The contract below
mirrors the upstream documentation so we can validate that our backend responses
match what the frontend expects.

## Interface Summary

| Item | Value |
| --- | --- |
| Bus name | `org.freedesktop.portal.Desktop` |
| Object path | `/org/freedesktop/portal/desktop` |
| Interface | `org.freedesktop.portal.ScreenCast` |
| Helper objects | `org.freedesktop.portal.Request` and `org.freedesktop.portal.Session` |
| Interface version | 5 |
| Async model | Portal methods return a `Request` object path; completion arrives via `Request::Response(u response, a{sv} results)` |

## Properties

### `AvailableSourceTypes` (readable `u`)

Bitmask reported to clients describing capture targets:

| Bit | Name | Meaning |
| --- | --- | --- |
| `1` | `MONITOR` | Share existing monitors |
| `2` | `WINDOW` | Share individual application windows |
| `4` | `VIRTUAL` | Create a compositor-provided virtual monitor |

### `AvailableCursorModes` (readable `u`)

Bitmask of supported cursor presentation modes (added in v2):

| Bit | Name | Behaviour |
| --- | --- | --- |
| `1` | `Hidden` | Pointer omitted entirely |
| `2` | `Embedded` | Pointer composited into the video buffers |
| `4` | `Metadata` | Pointer information sent as PipeWire metadata |

### `version` (readable `u`)

Must be set to `5` to advertise the interface revision that this document
captures.

## Method Patterns

- Every portal call that takes `handle_token` returns immediately with a
  `Request` object path.
- Results are delivered asynchronously via
  `org.freedesktop.portal.Request::Response`.
- `response` codes: `0` success, `1` cancelled, `2` ended in any other
  way (denied or failed). No other value is defined.

## Methods

### `CreateSession(options) → handle`

Creates a new ScreenCast session. The caller must later invoke
`SelectSources` and `Start`.

| Option | Type | Required | Notes |
| --- | --- | --- | --- |
| `handle_token` | `s` | yes | Last element of the `Request` object path. Must be a valid path token. |
| `session_handle_token` | `s` | yes | Last element of the session object path the frontend will export. |

**Response results** (via `Request::Response`):

| Key | Type | Notes |
| --- | --- | --- |
| `session_handle` | `s` | Object-path string for the new `org.freedesktop.portal.Session`. Historically typed as `s`, so it remains a string. |

A client may close the session at any time using `Session.Close`. The portal may
also close the session unilaterally, emitting `Session::Closed`.

### `SelectSources(session_handle, options) → handle`

Configures which sources the upcoming ScreenCast should include. Passing invalid
input closes the session, and clients only get one attempt per session.

| Option | Type | Default | Since | Notes |
| --- | --- | --- | --- | --- |
| `handle_token` | `s` | – | 1 | Request identifier suffix. |
| `types` | `u` | `MONITOR` | 1 | Bitmask limited to advertised `AvailableSourceTypes`. |
| `multiple` | `b` | `false` | 1 | Allow selecting more than one source. |
| `cursor_mode` | `u` | `Hidden` | 2 | Must be within `AvailableCursorModes`; unsupported values force the session to close. |
| `restore_token` | `s` | – | 4 | Single-use token describing a previously granted session. Ignored if invalid or stale. |
| `persist_mode` | `u` | `0` | 4 | `0` no persist, `1` persist while the app runs, `2` persist until revoked. Only valid for ScreenCast (not Remote Desktop). |

No standard result keys are defined for this method; implementations simply
complete the request with a response code.

### `Start(session_handle, parent_window, options) → handle`

Begins the ScreenCast session after sources were selected. The frontend typically
shows UI (using `parent_window` for modality) and, on success, returns stream
metadata to the caller.

| Option | Type | Required | Notes |
| --- | --- | --- | --- |
| `handle_token` | `s` | yes | Request identifier suffix. |

**Response results:**

| Key | Type | Since | Notes |
| --- | --- | --- | --- |
| `streams` | `a(ua{sv})` | 1 | Array of `(pipewire_node_id, properties)` tuples describing every published stream. |
| `restore_token` | `s` | 4 | New single-use token to recreate this selection later (only returned if persistence was granted). |

**Per-stream property schema:**

| Property | Type | Since | Description |
| --- | --- | --- | --- |
| `id` | `s` | 4 | Stable, opaque identifier unique within the session; helps correlate restored sessions. Optional. |
| `position` | `(ii)` | 1 | Logical x/y in compositor space (monitors only). Optional. |
| `size` | `(ii)` | 1 | Logical width/height in compositor space. Optional. |
| `source_type` | `u` | 3 | Matches `AvailableSourceTypes` to describe the content type. |
| `mapping_id` | `s` | 5 | Identifier to correlate with libei regions or other compositor resources. Optional. |

Clients must treat any omitted optional keys as “unknown”.

### `OpenPipeWireRemote(session_handle, options) → fd`

Returns a connected PipeWire remote file descriptor for the session. Clients use
it with `pw_context_connect_fd()` to access the published nodes.

- `options` currently has no standardized keys and should be empty.
- The returned FD stays valid for the lifetime of the ScreenCast session.

## Session Lifecycle Notes

1. `CreateSession` allocates the session and yields handles.
2. `SelectSources` defines the content and may hand back errors immediately when
   options are invalid.
3. `Start` prompts the user if needed, publishes streams, and returns metadata.
4. Clients may call `OpenPipeWireRemote` once a session exists (typically after
   `Start` succeeds) to obtain the PipeWire connection.
5. Either side can terminate the session via `Session.Close`, emitting
   `Session::Closed` to notify the peer.

Maintaining fidelity to these semantics ensures Otto remains a drop-in
replacement for other portal backends.
