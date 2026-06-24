# Testing

Otto has two test systems: headless integration tests for compositor behavior and WLCS for Wayland protocol conformance.

## Headless integration tests

Tests compositor behavior (gestures, expose, workspaces, window lifecycle, pointer interactions, scene graph) using a real compositor instance without GPU.

```bash
cargo test --features headless --test headless_basic
```

### Architecture

- **Backend:** `src/headless.rs` — `HeadlessHandle` starts the compositor on a background thread
- **Test client:** `components/otto-kit/src/testing.rs` — lightweight Wayland client with SHM buffers
- **Tests:** `tests/headless_basic.rs`
- **CI:** runs in the `check` job via `cargo test --features headless --test headless_basic`

### HeadlessHandle API

Start a compositor and interact with it:

```rust
let handle = HeadlessHandle::start(HeadlessConfig::default());
```

#### State access

| Method | Returns | Description |
|--------|---------|-------------|
| `with_state(\|state\| { ... })` | — | Run a closure on the compositor thread (blocking) |
| `query(\|state\| value)` | `R` | Query a value from compositor state |
| `window_count()` | `usize` | Number of windows across all workspaces |
| `current_workspace_index()` | `usize` | Active workspace index |
| `is_expose_active()` | `bool` | Whether expose mode is open |
| `is_show_desktop_active()` | `bool` | Whether show-desktop mode is active |

#### Gesture simulation

| Method | Description |
|--------|-------------|
| `swipe_begin()` | Start a 3-finger swipe gesture |
| `swipe_update(dx, dy)` | Send swipe delta (pixels) |
| `swipe_end()` | End swipe gesture |
| `swipe(&[(dx, dy)])` | Complete swipe in one call |
| `pinch_begin()` | Start a 4-finger pinch |
| `pinch_update(scale)` | Send pinch scale |
| `pinch_end()` | End pinch gesture |

#### Pointer simulation

| Method | Description |
|--------|-------------|
| `pointer_move(x, y)` | Move pointer to absolute logical coordinates |
| `pointer_click()` | Left-button press + release at current position |

**Note:** The first `pointer_move` into a new focus area triggers smithay's `enter` event (not `motion`). Selection updates only happen on `motion`. Establish focus with a priming move before testing hover behavior:

```rust
// Prime focus on the window selector area
handle.pointer_move(5.0, 300.0);
handle.settle(2);
// Now this move triggers motion → selection update
handle.pointer_move(target_x, target_y);
handle.settle(10);
```

#### Expose queries

| Method | Returns | Description |
|--------|---------|-------------|
| `expose_window_rects()` | `Vec<(title, x, y, w, h)>` | Expose layout rects in physical pixels |
| `expose_selected_title()` | `Option<String>` | Currently hovered window title in expose |

To convert expose rects (physical) to pointer coordinates (logical), divide by the output scale:

```rust
let scale = handle.query(|state| {
    state.workspaces.outputs().next()
        .map(|o| o.current_scale().fractional_scale())
        .unwrap_or(1.0)
});
let cx = (rect_x + rect_w / 2.0) as f64 / scale;
let cy = (rect_y + rect_h / 2.0) as f64 / scale;
```

#### Scene graph

| Method | Returns | Description |
|--------|---------|-------------|
| `scene_snapshot()` | `SceneSnapshot` | Full scene graph snapshot |
| `scene_json()` | `String` | Scene graph as JSON |
| `scene_has_damage()` | `bool` | Whether scene has pending damage |
| `is_layer_hidden(key)` | `Option<bool>` | Hidden state of a named layer |

#### Animation control

| Method | Description |
|--------|-------------|
| `settle(max_frames)` | Advance scene at 60fps until animations finish or limit reached. Returns frames with damage. |
| `tick(dt)` | Advance one frame by `dt` seconds. Returns true if damage produced. |
| `wait(duration)` | Sleep the test thread (lets compositor event loop run). |

`settle` is deterministic — no wall-clock sleeps. Use it after gestures/pointer events to let animations complete.

### TestClient API

Connect to the compositor and create windows:

```rust
let mut client = TestClient::connect(&handle.socket_name)?;
let toplevel = client.create_toplevel("my-window", 640, 480);
handle.wait(Duration::from_millis(200));
let _ = client.roundtrip();
```

`create_toplevel` creates a toplevel surface with a SHM buffer, sets the title, and commits. The returned `Arc<Mutex<TestToplevel>>` tracks configure events.

### Writing tests

Tests must use `#[serial]` (from `serial_test` crate) since they share a global compositor:

```rust
#[test]
#[serial]
fn my_test() {
    let handle = start_compositor();
    let mut client = connect_client(&handle);
    // ... test logic ...
}
```

## WLCS protocol conformance

Tests Wayland protocol compliance (surface roles, pointer/touch routing, xdg_shell).

```bash
# One-time: build the WLCS test runner
./compile_wlcs.sh

# Build the Otto WLCS adapter
cargo build -p wlcs_otto

# Run specific test groups
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='SelfTest*:FrameSubmission*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='*/SurfacePointerMotionTest*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='XdgToplevelStableTest.*'

# List all tests
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_list_tests
```

- **Adapter:** `wlcs_otto/` — cdylib that WLCS loads, runs a headless Otto instance
- **Key files:** `wlcs_otto/src/main_loop.rs` (event handling), `wlcs_otto/src/ffi_wrappers.rs` (C FFI)
