# Permissions

How Otto decides what each client may do, where that lives in the code, and
how to keep it right when adding something. The requirements are in
[specs/security-model.md](../../specs/security-model.md) and
[specs/agent-seats.md](../../specs/agent-seats.md); this page is the map.

## The idea in one paragraph

A program running as the user outside a sandbox already has the user's
rights, so Otto does not try to contain it. Otto makes sure that a sandboxed
app gets nothing from the compositor that would let it out, that the
interfaces which watch the screen or act as the user can be narrowed to an
allowlist (`[privacy] strict`), and that an AI agent works through the
standard protocols on a connection of its own, scoped to one seat and one
workspace, visible on screen and stoppable.

## Who is who

Every Wayland client carries a `ClientState` (`src/state/mod.rs`, `pub
struct ClientState`). The fields that matter here:

| Field | Set when | Means |
|---|---|---|
| `component` | Otto spawned the client on a socketpair (the locker, otto-authorize) | One of Otto's own, beyond doubt |
| `peer` | A client connects to the public socket; read with `SO_PEERPIDFD` | The executable on the other end, for `strict` and component checks |
| `security_context` | The client came through a `wp_security_context_v1` listener | Sandboxed |
| `agent_seat` | The client came through `ConnectAgent` or a listener made on such a connection | The agent seat it belongs to |
| `agent_session` | Same | Which seat instance, so the right user-seat global is shown to it |
| `handed_over` | The agent's seat ended (`remove_agent_seat`) | No longer the agent's; reaches nothing through what it bound |

The questions code asks of a client, all small functions:

- `ClientState::agent_seat_of(client)` (`src/state/mod.rs`): the agent seat
  it acts for, or `None`, also `None` once handed over.
- `ClientState::was_agent_client` and `ClientState::is_handed_over`: it was
  an agent's / it was and the agent is gone.
- `sandbox::is_sandboxed_client`, `sandbox::is_confined_client` (sandboxed,
  or ever an agent's), `sandbox::is_privileged_client` (not confined, and a
  component, or `strict` is off, or the executable is in `allow`),
  `sandbox::may_drive_seat` (`src/sandbox.rs`).
- `otto_kit::trust::is_component` (`components/otto-kit/src/trust.rs`): the
  executable has a name in `COMPONENTS` and root owns it and its directory,
  or it sits beside the running Otto in a directory the user owns.

`[privacy]` is parsed into `PrivacyConfig` (`src/config/mod.rs`) and read
once by `sandbox::init_policy` at startup; a config reload does not change
it.

XWayland has no `ClientState` and is treated as privileged.

## Globals and their filters

Every privileged global is created with a filter, so a client that may not
use it never sees it in the registry. Where each is decided:

| Global | Filter | Where |
|---|---|---|
| `ext_session_lock_manager_v1` | Otto's locker only | `src/state/mod.rs` |
| `zwlr_screencopy_manager_v1` | privileged; frames refused while locked | `src/state/screencopy.rs` |
| `ext_image_copy_capture_manager_v1` and its sources | privileged or agent; agent sessions scoped, handed-over sessions stopped | `src/state/image_capture.rs` |
| `zwlr_virtual_pointer_manager_v1` | privileged or agent; seat checked per pointer (`permitted_seat`) | `src/state/virtual_pointer.rs` |
| `zwp_virtual_keyboard_manager_v1` | privileged or agent; wrong seat is `unauthorized` | `src/state/virtual_keyboard.rs` |
| `zwlr_data_control_manager_v1`, `ext_data_control_manager_v1` | privileged | `src/state/mod.rs` |
| `zwlr_foreign_toplevel_manager_v1` | privileged or agent; agent scoped to its workspace, handed over sees nothing | `src/state/wlr_foreign_toplevel.rs`, the per-window filter in `src/shell/xdg.rs` |
| `ext_foreign_toplevel_list_v1` | not sandboxed; per-window filter by `ToplevelOwner` | `src/state/mod.rs` |
| input method, layer shell, gamma, shortcut inhibit | not confined (`UnsandboxedOnly` wraps smithay globals without a filter) | `src/state/mod.rs`, `src/state/gamma_control.rs` |
| otto_dock, text_cursor | not confined | their `handlers` modules |
| `wp_security_context_manager_v1` | not sandboxed | `src/state/mod.rs` |
| the user's `wl_seat` | not an agent client, ever | `src/state/mod.rs`; agents get it through a per-session global |

A filter decides what a client is offered, not what an object it already
holds may do. Wherever a client's kind can change after it binds (agents
are handed over), the request and event paths check again: see the
`is_handed_over` checks in the window list, window control and capture code.

## Agents

The D-Bus side is `org.otto.Compositor` in `src/screenshare/dbus_service.rs`;
it forwards to the compositor thread as `CompositorCommand`s
(`src/screenshare/mod.rs`), handled in `src/state/agent_seats.rs`.

- **Consent** (`src/agent_consent.rs`). A program is named by
  `otto_kit::process_app` (Flatpak id, desktop id, else executable name).
  The answer lives in xdg-permission-store, table `otto-agents`, entry
  `seat`. Dialogs go to islands as `org.otto.Dialog1.PresentAccess`, and the
  answer counts only if islands sent it. Stop suspends the program in
  memory. A permission-store change ends the seats of programs no longer
  allowed.
- **A seat** (`add_agent_seat`). A smithay seat whose global is filtered to
  the agent's own clients, plus a second global for the user's seat,
  created right after it with smithay's `create_global_with_filter` and
  filtered to that session's clients, so the agent's seat is announced
  first.
- **A connection** (`connect_agent_client`, `insert_agent_client`). A
  socketpair, its server end inserted with `agent_seat` and `agent_session`
  set. Security-context listeners made on it insert their clients the same
  way (`src/state/security_context_handler.rs`).
- **Scope** (`agent_scope_window_ids`, `Grant::Workspace`). The windows of
  the seat's one workspace. Pointer hit-testing, keyboard focus, the window
  lists and capture all ask it.
- **A lent workspace** (`sync_lent_seat_globals`, run every frame from
  `sync_agent_frames`). While the grant is `shared`, one more global of the
  agent's seat is shown to the user's unsandboxed programs.
- **Ending** (`remove_agent_seat`). Ends the grant (an own workspace loses
  its name), removes the seat's globals and listeners, marks every
  connection `handed_over` and keeps them, with the user-seat global, in
  `Otto::handed_over`. `prune_handed_over` removes that global once those
  clients have all gone; removing it earlier would make GTK drop the seat.
- **The frame and chip** (`src/workspaces/agent_frame.rs`, driven by
  `sync_agent_frames`). Stop answers the user's real pointer only
  (`src/input/pointer.rs`).
- **The secure attention key.** Otto listens for logind's
  `SecureAttentionKey` signal (`src/lock.rs`) and calls `stop_all_agents`.

## Otto's bus interfaces

Two guards, both by the executable behind the caller's bus connection:

- `otto_kit::trust::require_component(connection, header, names, what)`:
  only the named components. Used by `org.otto.ScreenCast` (portal backend,
  otto-rdp), `org.otto.Dialog1` (islands' own dialog clients) and
  `FocusApp` (Otto's interface components).
- `sandbox::require_trusted_caller`: only under `strict`, components and
  `allow`. Used by `org.otto.Shell1` methods and the a11y keyboard monitor.

A Flatpak app needs nothing from Otto here: its bus proxy already limits it
to the portals and the names it declares, so it cannot reach `org.otto.*`.

Screen sharing through the portal: a screencast session answers only the
bus connection that created it and ends when that connection leaves; the
portal refuses when it cannot show a picker rather than share a default
output; `otto-rdp` listens on localhost unless told otherwise.

Protected settings (`settings::schema::PROTECTED`) ask polkit for
`org.otto.settings.lock` before `Set` or `Reset` applies them
(`src/settings/polkit.rs`).

## The lock screen

- Otto starts the locker on a socketpair and offers
  `ext_session_lock_manager_v1` to it alone. `loginctl lock-session` asks
  Otto to start it.
- The screen blanks as soon as a lock begins, before the locker is up. A
  locker that dies is started again under the standing lock, and an output
  added while locked starts blank.
- Every suspend locks first, behind a logind delay inhibitor
  (`[lock] on_suspend`).
- The lock shortcut and VT switching survive a keyboard-shortcuts inhibitor.
- Popup and input-method grabs are dropped while locked, and a popup grab
  needs the serial of the latest press on that client.
- No agent input reaches anything while locked, and locking drops every
  agent seat's focus.
- An idle inhibitor counts only while its surface is on screen.

## The password panel

Otto is the session's polkit agent: the compositor starts `otto-authorize
--polkit-agent` on a socketpair marked `OttoComponent::Authorize` and
restarts it if it dies (`src/polkit_agent.rs`; `polkit_agent = false` turns
it off). polkit's own agent library registers it and runs polkit's helper,
which runs PAM; Otto adds only the panel (`otto-auth-ui`). While the panel
is up it holds the keyboard, nothing mapped later draws over it, and popup
and input-method grabs are released.

## Settings › Privacy

`components/otto-settings/src/panes/privacy.rs` reads and writes
xdg-permission-store through `otto_kit::permission_store` and follows its
`Changed` signal:

- `screencast` and `remote-desktop`: remembered shares, with Forget.
- `screenshot`: Ask, Allow or Don't Allow per app.
- `notifications`: a switch per app.
- `otto-agents`: a switch and Forget per program that asked for an agent
  seat. Turning one off or forgetting it ends its seats at once.

Any program running as the user can write the store too; it records
answers, it does not enforce them against same-user programs.

## Adding a privileged interface

1. Create its global with a filter: `is_privileged_client` for interfaces
   the user's programs get by default, or `!is_confined_client` for those
   only sandboxes and agents are kept off.
2. If agents should have a scoped version, add `|| agent_seat_of(..)
   .is_some()` to the filter and scope every request and event to
   `agent_scope_window_ids`.
3. Check every request and event path for `is_handed_over`, so an object
   bound by an agent reaches nothing once the agent is gone.
4. Refuse it while the session is locked if it shows or changes anything.
5. Add it to the table in `specs/security-model.md` and above, and a test
   in `tests/sandboxed_globals.rs`, `tests/privacy_strict.rs` or
   `tests/agent_seat.rs`.

## Tests

| File | Covers |
|---|---|
| `tests/sandboxed_globals.rs` | what a sandboxed client is offered |
| `tests/privacy_strict.rs` | `strict` and `allow` |
| `tests/agent_seat.rs` | seats, scope, connections, consent, the frame, lending, handover |
| `tests/image_capture.rs` | ext-image-copy-capture for the user and agents |
| `tests/session_lock.rs`, `tests/idle_inhibit.rs` | the lock screen |

Run them with `cargo test --features headless --test <name>`.

## Forks this depends on

- **smithay**, branch `feat/seat-filter` of github.com/nongio/smithay:
  `Seat::create_global_with_filter`, one more filtered global for a seat.
- **wayland-backend** and **wayland-sys**, branch
  `fix/filter-global-under-construction` of github.com/nongio/wayland-rs:
  libwayland announces a global from inside `wl_global_create`, before
  wayland-backend knew of it, so its filter was skipped and a new agent's
  seat reached every client. Both crates are patched, or the build gets two
  copies of wayland-sys.

## Not covered

- **Snap apps** do not use security contexts, so they are ordinary
  programs.
- **XWayland**: X clients can read and type into each other.
- **AT-SPI** exposes every app's widgets on the session bus.
- What [specs/security-model.md](../../specs/security-model.md) lists under
  "Not yet as specified".
