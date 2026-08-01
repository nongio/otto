# ScreenCast Backend Specification

> Based on the upstream `org.freedesktop.impl.portal.ScreenCast` interface spec: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html>

## Scope

This document captures the contract that `xdg-desktop-portal-otto`
implements as the **portal backend** for Otto. The upstream
`xdg-desktop-portal` frontend exports `org.freedesktop.portal.ScreenCast` to
clients and delegates each request to this service via
`org.freedesktop.impl.portal.ScreenCast`. We in turn call Otto’s
private API to satisfy those requests. All types, defaults, and behavioural
notes below mirror version **5** of the upstream spec so both sides remain
aligned.

## Interface Summary

| Item | Value |
| --- | --- |
| Bus name | `org.freedesktop.impl.portal.desktop.otto` (recommended) |
| Object path | `/org/freedesktop/portal/desktop` |
| Interface | `org.freedesktop.impl.portal.ScreenCast` |
| Helper objects | `Request` and `Session` objects are created by the frontend and passed by object path |
| Async model | Every method returns `(response, results)` where `response` matches `org.freedesktop.portal.Request.Response` codes |

## Properties

### `AvailableSourceTypes` (readable `u`)

Bitmask describing which capture targets Otto can satisfy:

| Bit | Name | Meaning |
| --- | --- | --- |
| `1` | `MONITOR` | Share existing physical outputs |
| `2` | `WINDOW` | Share individual application surfaces |
| `4` | `VIRTUAL` | Create and share a compositor-provided virtual monitor |

### `AvailableCursorModes` (readable `u`)

Bitmask of supported cursor presentation modes:

| Bit | Name | Behaviour |
| --- | --- | --- |
| `1` | `Hidden` | Pointer excluded from the stream |
| `2` | `Embedded` | Pointer composited into the video buffers |
| `4` | `Metadata` | Pointer sent via PipeWire metadata, not rendered |

### `version` (readable `u`)

Must return `5` to indicate compliance with the latest upstream definition. The property
name is lowercase — xdg-desktop-portal reads `version` and, when it comes back as 0, stops
propagating `AvailableCursorModes` to the client-facing portal, which then rejects every
`cursor_mode` with `Unavailable cursor mode`.

## Methods

All three backend methods carry the same leading arguments:

| Field | Type | Direction | Notes |
| --- | --- | --- | --- |
| `handle` | `o` | IN | Object path of the `org.freedesktop.impl.portal.Request` representing this invocation |
| `session_handle` | `o` | IN | Object path of the `org.freedesktop.impl.portal.Session` this request applies to |
| `app_id` | `s` | IN | Desktop application ID making the request |
| `options` | `a{sv}` | IN | Method-specific vardict of options |
| `response` | `u` | OUT | `0` success, `1` cancelled, `2` denied, `3` failure (higher values reserved) |
| `results` | `a{sv}` | OUT | Method-specific results |

### `CreateSession(handle, session_handle, app_id, options)`

Purpose: allocate a backend session that can later be configured and started.

Additional semantics:
- `options` currently has no mandatory keys; implementations may accept
  compositor-specific hints but must ignore unknown keys from the frontend.
- On success the backend must fill the `results` vardict with:

| Key | Type | Meaning |
| --- | --- | --- |
| `session_id` | `s` | Backend-defined identifier that the frontend passes back during later calls |

The frontend turns `(response, results)` into a `Request::Response` signal.

### `SelectSources(handle, session_handle, app_id, options)`

Purpose: configure what the eventual screencast should include. The backend must
validate inputs immediately (cancel the session if unsupported cursor modes are
requested).

Supported option keys:

| Key | Type | Default | Since | Notes |
| --- | --- | --- | --- | --- |
| `types` | `u` | `MONITOR` | 1 | Bitmask of desired capture targets; must be limited to advertised `AvailableSourceTypes` |
| `multiple` | `b` | `false` | 1 | Allow the user to pick more than one source |
| `cursor_mode` | `u` | `Hidden` | 2 | Must be one of the advertised cursor modes; invalid values require closing the session |
| `restore_data` | `(suv)` | – | 4 | Vendor, version, opaque bytes describing a previously persisted selection |
| `persist_mode` | `u` | `0` | 4 | `0` no persist, `1` persist while app runs, `2` persist until revoked |

`results` is typically empty; any backend-specific confirmation can be exposed
through implementation-specific keys if needed.

### `Start(handle, session_handle, app_id, parent_window, options)`

Purpose: trigger UI, finalize the source list, and publish PipeWire streams.
`parent_window` is passed through from the frontend and must be used when
presenting UI.

`options` currently has no standard keys. The backend must return the following
entries in `results`:

| Key | Type | Since | Meaning |
| --- | --- | --- | --- |
| `streams` | `a(ua{sv})` | 1 | Array of `(node_id, properties)` tuples for every PipeWire node exported |
| `persist_mode` | `u` | 4 | Effective persistence level granted to the app (may be reduced versus the request) |
| `restore_data` | `(suv)` | 4 | Data blob that lets the frontend and backend re-create the same selection later |

#### Stream property schema

Each tuple inside `streams` uses the following property keys in the per-stream
vardict:

| Key | Type | Since | Description |
| --- | --- | --- | --- |
| `position` | `(ii)` | 1 | Logical x/y within the compositor space (monitors only) |
| `size` | `(ii)` | 1 | Logical width/height of the captured region |
| `source_type` | `u` | 3 | Captured content type (matches `AvailableSourceTypes`) |
| `mapping_id` | `s` | 5 | Stable identifier to correlate with libei regions or other compositor data |

Backends may include additional keys but must not omit the properties above when
they are known for the stream type in question.

## Response Codes

`response` values mirror `org.freedesktop.portal.Request.Response`:

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Cancelled by the user or compositor |
| `2` | Permission denied |
| `3` | Other failure |

Any non-zero code should be accompanied by diagnostic logging.

## PipeWire Remote Handoff

The backend spec intentionally lacks an `OpenPipeWireRemote` method. Clients call
`org.freedesktop.portal.ScreenCast.OpenPipeWireRemote`, and the frontend must
request a PipeWire FD from the backend synchronously (commonly via an internal
IPC such as Otto’s private API) before handing it to the client. This
file descriptor must stay valid for the duration of the session.

## References

1. `org.freedesktop.impl.portal.ScreenCast` (v5): <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html>
2. `org.freedesktop.portal.ScreenCast` (frontend methods for context): <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html>
3. Shared helper interfaces: `Request` and `Session` definitions under the same documentation set.
