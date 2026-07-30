# XWayland fullscreen game focus — findings & implementation plan

**Goal:** fullscreen XWayland games (Cuphead via Proton/Steam) that (a) render without stalling,
(b) survive workspace switches, and (c) **receive keyboard input** — all at once.

**Status:** ALL THREE SOLVED and in the tree (uncommitted), user-confirmed with real keyboard input
("everything works"). (a) renders, (b) survives workspace switches, (c) keyboard reaches the game via
gamescope-style `XSetInputFocus`. Remaining work is cleanup + consolidation + commit (sections 6).
NOTE: the wlrctl *virtual* keyboard is unreliable — it can false-fail on input even when delivery works;
trust real keyboard / the menu actually advancing, not virtual-keyboard negatives.

Full investigation log: project memory `project_xwayland_fullscreen_black_scanout.md` (UPDATEs 1–20)
and `project_cuphead_fullscreen_hang.md`.

---

## 1. The three dynamics (root causes)

Cuphead is a **Unity / Proton (DXVK) game**, ICCCM input model **Globally-Active** (`WM_HINTS.input=false`
+ `WM_TAKE_FOCUS`), default **`Run In Background = false`**. Each dynamic was a separate bug:

| # | Symptom | Root cause | Fix |
|---|---------|-----------|-----|
| 1 | Black screen at launch | Forcing keyboard/X focus (→ `WM_TAKE_FOCUS`) **at map** stalls the render loop | Don't focus-protocol at map; route keys to the `wl_surface` |
| 2 | Freeze on workspace switch | Unity (`Run In Background=false`) **pauses on X11 `FocusOut`**; switching focused another window → XWayland sent the game `FocusOut` | Keep focus on the fullscreen game (focus-retention) |
| 3 | **Keyboard has no effect** (SOLVED) | `wl_keyboard` focus alone does NOT deliver keys — XWayland needs the **WM to set X input focus**. But `WM_TAKE_FOCUS` (the ICCCM way) breaks rendering (dynamic #1, any time) | **gamescope's bare `XSetInputFocus`** (no `WM_TAKE_FOCUS`) — implemented in §4, confirmed working |

Predicate used throughout: **self-managing == `WmInputModel::GloballyActive | None`**
(helper `WindowElement::x11_self_manages_focus()`).

---

## 2. Reference implementations — the key comparison

When the WM gives focus to a Globally-Active X11 window, there are two strategies:

| Compositor | Globally-Active focus | ICCCM-correct? | Works with Cuphead? |
|---|---|---|---|
| **smithay default** (`X11Surface::enter`) | send `WM_TAKE_FOCUS`, no `set_input_focus` | ✅ yes | ❌ **breaks render** |
| **wlroots/sway** (`offer_focus`) | send `WM_TAKE_FOCUS` only ("it gets focus itself") | ✅ yes | ❌ (Hyprland #7155 = same, closed unsolved) |
| **gamescope** (`steamcompmgr.cpp:4857`) | **bare `XSetInputFocus(win, RevertToNone, CurrentTime)`, NO `WM_TAKE_FOCUS`** | ❌ deviates | ✅ **works** |

gamescope code (the validated answer):
```cpp
// steamcompmgr.cpp ~4857, on map / focus apply. WM_HINTS.input only gates XRaiseWindow above.
if ( w == ctx->focus.inputFocusWindow || w->xwayland().id == ctx->currentKeyboardFocusWindow )
    XSetInputFocus(ctx->dpy, w->xwayland().id, RevertToNone, CurrentTime);
```
`grep WM_TAKE_FOCUS steamcompmgr.cpp` → **nothing**. gamescope never uses it.

**Insight:** Wine's `WM_TAKE_FOCUS` *handler* (re-asserts fullscreen / mode-set) is what breaks DXVK; a
plain `FocusIn` from `XSetInputFocus` sets X focus — delivering keys — without invoking that handler.
For fullscreen games we should follow **gamescope, not ICCCM/wlroots**.

Sources: [gamescope steamcompmgr.cpp](https://github.com/ValveSoftware/gamescope/blob/master/src/steamcompmgr.cpp) ·
[wlroots xwm.c](https://github.com/swaywm/wlroots/blob/master/xwayland/xwm.c) ·
[Hyprland #7155](https://github.com/hyprwm/Hyprland/issues/7155) ·
[smithay #1863 `_NET_ACTIVE_WINDOW`](https://github.com/Smithay/smithay/pull/1863)

---

## 3. Current working build (in the tree, uncommitted)

Renders perfectly (harness: 2145+ frames, rock-steady ~38fps) and survives workspace switches (30s, 3
swipe cycles) — but **no keyboard** (dynamics #1 & #2 fixed, #3 open). Files:

- **src/shell/element.rs** — `x11_self_manages_focus()` helper; `set_activate` no-op (never toggle
  `_NET_WM_STATE_FOCUSED`); `output_leave` no-op (never `wl_surface.leave(output)`) — for self-managing X11.
- **src/focus.rs** — `x11_self_managed_surface(s)` helper; `enter`/`key`/`modifiers` for self-managing X11
  route to the **`wl_surface`** (gives `wl_keyboard` focus, NO `WM_TAKE_FOCUS`); `leave` is a no-op.
- **src/state/app_management.rs** — always `keyboard.set_focus`; **focus-retention**: `focus_top_window_or_clear`
  early-returns and `set_keyboard_focus_on_window` skips when a self-managing game is fullscreen & target≠game;
  `set_x11_active_window` won't hand `_NET_ACTIVE_WINDOW` to another window while the game is fullscreen.
- **src/state/xwayland_handler.rs** — `surface_associated` focuses the window at map (routes to wl_surface
  for self-managing) + `apply_x11_fullscreen` replay.

Evidence it works: harness imported 2145 frames at steady 38fps through 8 keypresses with **zero** freeze/
unmap; survived multiple swipes in an earlier run. Keyboard confirmed dead by user + harness (no menu advance).

---

## 4. The fix — add gamescope's `XSetInputFocus` (lever-2)

Keep the render-stable `wl_surface` key routing (gives the game `wl_keyboard` focus so XWayland *has* the
keys) AND additionally set the **X input focus** to the game so XWayland *delivers* them — using
`XSetInputFocus`, never `WM_TAKE_FOCUS`.

### 4a. smithay patch (`../smithay`, a path dep)
smithay exposes no way to force `set_input_focus` (only `X11Wm::set_active_window` / `raise_window`). The
exact call already exists privately in `X11Surface::enter` (surface.rs:1147:
`conn.set_input_focus(InputFocus::NONE, self.window, x11rb::CURRENT_TIME)`). Add a public method:
```rust
// smithay/src/xwayland/xwm/surface.rs, impl X11Surface
/// Set the X11 input focus to this window directly (RevertToNone), WITHOUT sending
/// WM_TAKE_FOCUS. Mirrors gamescope: needed to deliver keyboard to Globally-Active
/// games (Unity/DXVK) whose WM_TAKE_FOCUS handler would otherwise break their render loop.
pub fn set_input_focus(&self) -> Result<(), ConnectionError> {
    if let Some(conn) = self.conn.upgrade() {
        conn.set_input_focus(InputFocus::NONE, self.window, x11rb::CURRENT_TIME)?;
        let _ = conn.flush();
    }
    Ok(())
}
```
Additive, doesn't change existing behavior. (Upstreamable as the smithay analogue of
`wlr_xwayland_surface_offer_focus` but for the gamescope/force-focus policy.)

### 4b. Otto change
When a self-managing X11 game gains keyboard focus, call `set_input_focus()` on it. Cleanest seam:
`src/focus.rs` `KeyboardTarget::enter`, the self-managing branch:
```rust
WindowSurface::X11(s) => match x11_self_managed_surface(s) {
    Some(surface) => {
        let _ = s.set_input_focus();              // gamescope: X focus, no WM_TAKE_FOCUS -> keys flow
        KeyboardTarget::enter(&surface, seat, data, keys, serial)  // wl_keyboard focus -> XWayland has keys
    }
    None => KeyboardTarget::enter(s, seat, data, keys, serial),
},
```
The game is focused at map (xwayland_handler) and held by focus-retention, so `set_input_focus` fires once
when it becomes focused and stays. No `WM_TAKE_FOCUS` is ever sent to it.

### 4c. Verify (harness or real input)
1. Cuphead renders fullscreen continuously (no stall) — must still hold.
2. **Without clicking**, send menu keys (Enter/Z/arrows) → game **advances past the title attract screen**
   to save-slot/main menu = keys reached it. Capture AFTER screenshot within ~5s (the game self-exits if
   left idle — investigate that separately; possibly a key mapping to quit).
3. Swipe away/back several times → still renders, still takes keys.
4. A real click into the game → does NOT break rendering (no `WM_TAKE_FOCUS` anywhere now).

---

## 5. Open questions / risks

- **Does `XSetInputFocus` at map stall?** gamescope does it in its map handler and Cuphead works → expected
  safe. Verify; if it does, defer the `set_input_focus` to the first frame / first interaction.
- **Scope.** Only call `set_input_focus` for self-managing X11 (`GloballyActive|None`). Passive/Local apps
  keep the normal smithay `X11Surface::enter` path (`WM_TAKE_FOCUS` + `set_input_focus`) — don't regress them.
- **Focus stealing.** With focus-retention, X focus stays on the game; confirm a second monitor / dock /
  un-fullscreen path can still move focus when the game stops being fullscreen.
- **Self-exit.** Harness saw Cuphead exit ~20s after idle keypresses (PipeWire "Cuphead.exe removed"). Rule
  out a test key hitting a quit, and confirm Otto's own quit shortcut can't be triggered by game keys.
- **ICCCM deviation.** This intentionally ignores the Globally-Active "don't set focus" convention for
  fullscreen games, matching gamescope. Acceptable for a gaming-capable compositor; document it.

---

## 6. Refactor / consolidation (the "proper solution")

Right now the policy is spread across 4 files and several gates. Consolidate into one concept:
**"a self-managing X11 game is fullscreen → it owns focus + X-input-focus + `_NET_ACTIVE_WINDOW` until it
unfullscreens, and is focused via `XSetInputFocus` not `WM_TAKE_FOCUS`."**

- Consider a single helper, e.g. `Workspaces::fullscreen_focus_lock() -> Option<&WindowElement>`, and route
  all focus/activation/visibility decisions through it.
- Decide the minimal necessary gate set by removing each and testing (focus-retention is THE one for #2;
  set_activate/output_leave/leave no-ops are defense-in-depth — verify each is actually needed).
- Verify normal X11 apps (xterm, Passive/Local) focus and take keys correctly.

### Cleanup checklist (before commit)
- Remove `OTTO_DEBUG_TEX`-gated logs added during debugging: `[set-activate]`, `[x11-active-window]`,
  `[kbd-focus]`, `[ws-switch]`, and `[tex-import]/[tex-render]/[frame-cb]/[scanout]/[sync]`.
- Remove experimental env gates: `OTTO_NO_SWIPE_FOCUS` (gestures.rs, redundant), and the older
  `OTTO_FORCE_PRESENT_FB`, `OTTO_SKIP_FS_ANIM`, `OTTO_ANVIL_FULLSCREEN`, `OTTO_X11_SCANOUT`, `OTTO_BLIT_CCS`,
  `OTTO_NO_X11_DMABUF_FEEDBACK`, `OTTO_XWAYLAND_WL_DEBUG` (per memory revert list); smithay `OTTO_BUF_DROP`
  probe + anvil bisect injections in `../smithay`.
- Keep: the smithay `set_input_focus` method (4a), and `udev/feedback.rs strip_clear_color_modifiers` (a
  separate correct fix — Otto can't sample CCS_CC; stops grim hanging).
- spec-sync, then commit smithay + otto together (smithay is a path dep — bump/commit it too).

---

## 7. Diagnostic method (reuse it)
`RUST_LOG=info OTTO_DEBUG_TEX=1 ./target/release/otto --tty-udev > /tmp/otto.log` (kill the respawning
`xdg-desktop-portal-otto` watchdog in a loop or it pkills busy Otto). Per-surface by `wl_surface@N`, game =
2880x1920 dims. `[tex-import]` gap = freeze instant. `frame-cb throttle=` `0ns→33ms` marks the
workspace-non-current instant. `unmap_window`+`ReparentWindow BadWindow`+dock `running=[...]` = game exit
vs pause. The `otto-remote-control` skill drives a live Otto (Steam+Cuphead, wlrctl pointer/keyboard, grim)
for objective verification. Build PLAIN `cargo build --release` (smithay is a path dep; --features dev
recompiles all of smithay).
