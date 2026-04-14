# Testing Otto

Otto has two complementary test systems: **WLCS** for Wayland protocol conformance and **headless integration tests** for compositor-specific behavior.

## Quick Start

```sh
# Headless integration tests (gesture, expose, workspaces, scene graph)
cargo test --features headless --test headless_basic

# WLCS protocol conformance (requires building WLCS first)
./compile_wlcs.sh
cargo build -p wlcs_otto
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='SelfTest*:FrameSubmission*'
```

## Headless Integration Tests

Located in `tests/headless_basic.rs`. These start a real compositor instance (no GPU) on a background thread and exercise it through two channels:

- **Wayland clients** via `otto_kit::testing::TestClient` — connects over the socket, creates surfaces, roundtrips
- **Direct state access** via `HeadlessHandle` — gesture simulation, state queries, scene inspection

### Key Components

| File | Purpose |
|---|---|
| `src/headless.rs` | Headless backend — `HeadlessHandle` starts compositor, provides `with_state()`, `query()`, `settle()` |
| `components/otto-kit/src/testing.rs` | Lightweight Wayland test client with SHM buffers (no EGL/Skia) |
| `tests/headless_basic.rs` | Integration tests |

### HeadlessHandle API

```rust
let handle = HeadlessHandle::start(HeadlessConfig::default());
let mut client = TestClient::connect(&handle.socket_name).unwrap();

// Create a window
let toplevel = client.create_toplevel("my-window", 640, 480);
handle.wait(Duration::from_millis(100));
client.roundtrip().unwrap();

// Simulate gestures
handle.swipe_begin();
handle.swipe_update(0.0, -80.0);
handle.swipe_end();

// Let animations finish (deterministic 60fps stepping, no wall-clock sleep)
handle.settle(300);

// Query state
assert!(handle.is_expose_active());
assert_eq!(handle.window_count(), 1);

// Inspect scene graph
let snapshot = handle.scene_snapshot();
let json = handle.scene_json();

// Run arbitrary closure on compositor state
let titles: Vec<String> = handle.query(|state| {
    state.workspaces.spaces_elements().map(|w| w.xdg_title()).collect()
});

handle.stop();
```

### Writing a New Headless Test

```rust
#[test]
#[serial]  // tests share a compositor, run sequentially
fn my_test() {
    let handle = start_compositor();
    let mut client = connect_client(&handle);

    // ... test logic ...

    handle.stop();
}
```

Tests must be `#[serial]` because each test starts its own compositor on a unique socket, but the `serial_test` crate prevents parallel execution which avoids port/resource conflicts.

### What to Test Here

- Gesture state machines (swipe detection, expose triggers, workspace switching)
- Window lifecycle (map, focus, minimize, close)
- Expose mode (enter, exit, window layout, stacking order preservation)
- Workspace switching (programmatic, gesture-driven)
- Scene graph structure (layer visibility, hierarchy)
- Animation settling (deterministic frame stepping)

## WLCS Protocol Conformance

[WLCS](https://github.com/MirServer/wlcs) (Wayland Layout Conformance Suite) tests protocol-level behavior: surface creation, pointer/touch input routing, xdg_shell compliance.

### Building

```sh
./compile_wlcs.sh          # Build WLCS test runner (one-time)
cargo build -p wlcs_otto   # Build Otto's WLCS adapter (cdylib)
```

### Running

```sh
# Run a specific test group
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='XdgToplevelStableTest.*'

# List all available tests
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_list_tests

# Recommended groups (these all pass)
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='SelfTest*:FrameSubmission*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='XdgOutputV1Test*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='*/SurfacePointerMotionTest*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='XdgToplevelStableTest.*'
```

### Architecture

| File | Purpose |
|---|---|
| `wlcs_otto/src/lib.rs` | FFI entry point, event types |
| `wlcs_otto/src/main_loop.rs` | Headless compositor loop, event handling |
| `wlcs_otto/src/ffi_wrappers.rs` | C FFI callbacks for WLCS |
| `wlcs_otto/src/renderer.rs` | Dummy renderer (no GPU) |
| `compile_wlcs.sh` | Builds the WLCS test runner binary |

### Current Status

| Group | Pass | Fail | Notes |
|---|---|---|---|
| SelfTest + FrameSubmission | 12 | 0 | Core protocol |
| XdgOutputV1 | 1 | 0 | Output properties |
| SurfacePointerMotion | 8 | 0 | Pointer enter/leave/motion |
| XdgToplevelStable | 7 | 5 | 5 fail due to Smithay configure-ack timing |
| XdgSurfaceStable | 2 | 4 | 4 fail due to missing Smithay validation |
| BadBuffer | 1 | 1 | 1 fail due to Smithay shm truncation handling |

### Known Smithay-Level Limitations

- **Configure-ack timing**: Smithay rejects `wl_surface.commit` with a buffer if the client hasn't acked the initial configure. WLCS tests that create+commit in one protocol batch hit this. Even Smithay's own anvil CI skips these tests.
- **Missing `get_xdg_surface` validation**: Smithay doesn't check for existing roles or buffer state when `get_xdg_surface` is called.
- **Truncated SHM**: Smithay catches SIGBUS but doesn't post `WL_SHM_ERROR_INVALID_FD` to the client.

### What to Test Here

- Protocol compliance (surface roles, configure sequences)
- Input routing (pointer/touch enter/leave across surfaces)
- Subsurface ordering and positioning
- Error handling for invalid client requests
