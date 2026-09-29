# Agent permissions — review

**Date:** 2026-09-29  
**Scope:** branch `feat/agent-cursor` at `cb0318f3`, read-only review of how
Otto controls what agents (and other local clients) can do.  
**Related specs:** agent-seats.md

Nothing enforces grants yet; agent-seats.md says so for Phases 1–3. This
review lists what that means in practice, including gaps the spec does not
admit. File references are to the reviewed commit. Items marked
**verified** were re-checked against the code after the review.

## Findings

### 1. Critical — sandboxed clients reach every sensitive global

security-context-v1 is advertised, but the tag it puts on a client is read
only to forbid nested contexts (`src/state/mod.rs:876`). Session lock,
data-control (both), input method, virtual keyboard are created with
`|_| true` (`mod.rs:836–860`, **verified**); screencopy, virtual pointer,
wlr-foreign-toplevel (activate/close) and gamma have no filter.

A Flatpak app can type into a terminal through a virtual keyboard on the
user's seat, read the clipboard, log keys through the input-method
keyboard grab, capture the screen, or lock the session with its own locker
to phish the password.

*Fix:* `can_view` filters hiding all of these from clients with a security
context; for the rest, the Phase 5 authorization.

### 2. High — an agent's launch token takes over the user's windows

`request_activation` moves any window that activates with an agent token
to the agent's workspace and gives it the agent's keyboard
(`src/state/xdg_activation_handler.rs:70–79`, **verified**). Launching
`firefox <url>` or `code <file>` while the user has it open makes the
running instance activate an *existing* window — the user's logged-in
browser moves into the agent's grant. The own workspace no longer "holds
nothing of the user's".

*Fix:* place only windows that have never been mapped; tokens single-use
and expiring (~10 s), removed from `agent_launch_tokens` and Smithay's
known tokens once used.

### 3. High — `CaptureWorkspace`: no caller check, works while locked

Any session-bus client can capture any workspace, hidden ones included,
and also while the session is locked — screencopy and screencast refuse
then, this does not (`src/state/workspace_capture.rs`, **verified**). No
sharing indicator is shown.

*Fix:* refuse while locked; scope to the caller's granted workspace (the
owner is known from the message header); show an indicator.

### 4. High — Shell1 `RunCommand` and `FocusApp` move windows around grants

`[app_id=…] focus` then `move container to workspace <agent's>` pulls any
user window into an agent's grant; `kill` closes windows; renaming
workspaces redirects name-based captures. No caller check, no lock check
(`src/shell/commands.rs:57–167`).

*Fix:* identify the caller; refuse window-moving commands from agent
owners; refuse state changes while locked; log them.

### 5. High (for sandboxed callers) — D-Bus services are code execution

`LaunchOnOwnWorkspace` runs any argv in Otto's environment; Settings can
set `lock.locker_command` (run on every lock) and turn auto-lock off;
`AddVirtualOutput` makes an unseen screen. For same-user processes this
adds nothing; for a sandboxed app with session-bus access it is an escape.

*Fix:* read the caller's credentials (`GetConnectionCredentials`, pidfd)
and refuse Flatpak/Snap callers; ask the user before launching programs
and before changing locker, power and lock settings.

### 6. Medium — seats and grants follow names, not identities

Any client can create a virtual pointer on `agent-2` and act inside its
grant (`src/state/virtual_pointer.rs`); after an agent disconnects, any
process asking for a seat under its name inherits seat, colour, grant and
launch tokens (`src/state/agent_seats.rs`). A pointer tied to a removed
seat's name works again when the name comes back.

*Fix:* bind the seat to the owner's credentials, or to a secret returned by
`RequestAgentSeat` and presented over Wayland; key virtual pointers by the
seat, not its name.

### 7. Medium — Stop is weak

With a window fullscreen on the agent's workspace there is no chip, so no
Stop, and the pause shortcut (Phase 4) does not exist. After Stop the
agent can ask for a new workspace at once, without consent; its seat and
launch tokens stay. The chip does not clamp a long name, so Stop could be
pushed off-screen. The static `agent` seat has no border and no Stop.

*Fix:* Stop reachable over fullscreen (or ship the pause shortcut first);
Stop ends the seat and refuses that owner/name until the user allows it
again; ellipsize the chip text; frame everything while the static seat is
in use.

### 8. Medium — per-seat clipboard isolation does not hold

data-control reads and writes every seat's selection. Fix as in 1.

### 9. Medium — no quotas

Unlimited seats per connection (each compiles a keymap, is advertised to
every client and is never freed by Smithay); a new workspace per
release/request cycle; `agent_history`, `agent_launch_tokens` and
Smithay's known tokens only grow; each capture renders and encodes on the
main thread and its file is never deleted (and falls back to `/tmp` when
`XDG_RUNTIME_DIR` is unset); launches are not rate-limited.

*Fix:* one seat per connection, a global cap (~8), a workspace cap, rate
limits on capture and launch, capture returned as a file descriptor, refuse
without `XDG_RUNTIME_DIR`.

### 10. Medium — the user-seat virtual pointer drives Otto's own UI

A virtual pointer on the user's seat hit-tests Otto's chrome like real
input. The Phase 4 consent prompt, as specified, could be answered by any
client. Only Stop is limited to real input.

*Fix:* Phase 5 input authorization before (or with) Phase 4 prompts;
prompts accept only real input events.

### 11. Medium (a11y on) — any bus client can watch or grab keys

`WatchKeyboard`/`GrabKeyboard` on the a11y keyboard monitor have no caller
check (`src/a11y/keyboard_monitor.rs`). The lock screen is excluded.

*Fix:* allow-list assistive technologies by executable.

### 12. Low — confinement gaps

- A button with no motion goes to the stale pointer focus, even if that
  window has since left the grant. Fix: re-hit-test every frame.
- An agent-seat drag clears the user's pending raise and takes the shared
  drag icon; the spec says it must not interfere.
- The launch token can be planted in another process's environment, but
  only if leaked (32 random characters).
- Agent names are not checked for bidi or zero-width characters.
- Six palette colours: the seventh agent repeats one.

Confirmed working: agent keys and modifiers are gated by lock and grant;
agent pointer input is blocked while locked; layer surfaces and X11
windows are out of reach of a workspace grant.

### 13. Low, by design — user-seat virtual input bypasses everything

A virtual pointer or keyboard on the user's seat (or with no seat) from any
client, and the static `agent` seat, reach everything. The spec states this
until Phase 5.

## Architecture recommendations

1. **One identity model for Wayland and D-Bus.** Launch agents' programs
   through a security-context listener tagged with the agent; filter
   globals per client. On D-Bus, read caller credentials (pidfd, uid,
   label); refuse sandboxed callers and allow-list executables for
   privileged methods.
2. **Capabilities follow credentials, not names** — including seats and
   grants.
3. **Sensitive globals off by default** (virtual input, screencopy,
   data-control, input method, foreign-toplevel, session lock, gamma,
   shortcut inhibit), allowed per client: the RDP bridge, the locker, the
   bar.
4. **Consent only by real input**; Phase 5 before Phase 4.
5. **Scoped capture:** the caller's grant only, never while locked, with an
   indicator, returned by file descriptor.
6. **Sticky Stop:** ends the seat and refuses the agent until the user
   allows it; reachable over fullscreen; a pause shortcut exempt from
   shortcut inhibition.
7. **Quotas and rate limits** on seats, workspaces, launches, captures and
   tokens; tokens single-use with a lifetime.
8. **Gate Shell1 and Settings:** mutating commands need an authorized
   caller and an unlocked session; locker, power and lock settings need
   consent.

## Not verified

Keyboard popup grabs surviving Stop; exposé drawing over the lock screen;
the chip overflowing with wide glyphs.
