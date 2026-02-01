# Otto ScreenCast portal

`xdg-desktop-portal-otto` is the canonical Otto
implementation of the `org.freedesktop.impl.portal.ScreenCast` **backend**
D-Bus interface. The upstream `xdg-desktop-portal` process exposes
`org.freedesktop.portal.ScreenCast` to applications and delegates each request
to this service. We in turn translate those requests into Otto’s
private API (`org.otto.ScreenCast`), bridging the frontend portal to
the compositor. The current implementation focuses exclusively on the ScreenCast
portal as defined in the upstream specifications. See
[`ScreenCast-backend-spec.md`](./ScreenCast-backend-spec.md)
for the contract this crate implements and
[`ScreenCast-frontend-spec.md`](./ScreenCast-frontend-spec.md)
for the corresponding frontend behaviour we must satisfy.

It exposes the well-known portal service
`org.freedesktop.portal.Desktop` on the session bus and provides
responses that allow PipeWire-capable clients (Chromium, OBS Studio, etc.) to
exercise the standard screencast flow.

The implementation runs entirely in-process using [`zbus`](https://docs.rs/zbus)
for the D-Bus bindings and logs every method invocation and emitted signal via
`tracing`.

## Architecture

The portal backend acts as a translator: it receives standard portal requests and
translates them into Otto-specific D-Bus calls to create and manage
PipeWire streams.

## Running

```bash
cargo run -p xdg-desktop-portal-otto
```

On startup the binary prints `Otto portal running` once it has
successfully registered on the session bus. The process continues to service
requests until it receives `Ctrl+C`.

## Quick test

You can verify that the portal responds using `gdbus`:

```
gdbus call --session \
  --dest org.freedesktop.portal.Desktop \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.ScreenCast.CreateSession \
  "{'handle_token':<'t1'>,'session_handle_token':<'s1'>}"
```

The command returns immediately because the portal emits the corresponding
`org.freedesktop.portal.Request::Response` signal with a success payload as soon
as the method is invoked.

## Specification Compliance

This implementation follows the
[XDG Desktop Portal ScreenCast specification](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html)
and currently implements only the ScreenCast interface.

## Implementation Status

Current functionality:
- ✅ ScreenCast portal coverage per the upstream specification
- ✅ Integration with compositor's internal interface
- ✅ PipeWire stream creation and node ID tracking
- ✅ Cursor mode support (Hidden, Embedded, Metadata)
- ✅ Monitor (output) selection


## Debugging

Logs are written to `/tmp/portal-otto.log`. To monitor in real-time:

```bash
tail -f /tmp/portal-otto.log
```

The portal logs:
- All incoming D-Bus method calls
- Interactions with the compositor's internal interface  
- PipeWire node ID tracking and polling
- Session and stream lifecycle events

## TODO

Not yet implemented:
- ⚠️ Window selection (RecordWindow)
- ⚠️ Restore tokens (session persistence)
- ⚠️ User permission dialogs (currently auto-grants)
