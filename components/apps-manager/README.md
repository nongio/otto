# Apps — Foreign Toplevel List Debug Tool

**Status: work in progress.** A command-line tool for testing Otto's
`ext_foreign_toplevel_list_v1` implementation — it prints the window list and
exits. It is not the application launcher its name suggests, and has no
interface: nothing here is drawn, so there is nothing for it to expose to
assistive technologies.

## Building

```bash
cargo build -p apps-manager
```

## Usage

### Testing with Otto

1. **Start Otto in one terminal:**
   ```bash
   cargo run -- --winit
   ```

2. **Run the debug tool in another terminal:**
   ```bash
   # Set the Wayland display if needed
   export WAYLAND_DISPLAY=wayland-1  # or wayland-0, check Otto output
   
   # Run the tool
   cargo run -p apps-manager
   ```

3. **Open some windows** in Otto (e.g., with sample clients or other applications)

4. **Observe the output:**
   The tool will print messages when:
   - New toplevels are created
   - Toplevel title/app_id changes are detected
   - Toplevels are closed

### Example Output

```
Foreign Toplevel List Debug Tool
Connecting to Wayland compositor...

Found ext_foreign_toplevel_list_v1 v1
Waiting for events... (Press Ctrl+C to exit)

New toplevel #0
Toplevel #0: title='Terminal' app_id='org.gnome.Terminal' identifier='<none>'
New toplevel #1
Toplevel #1: title='Firefox' app_id='firefox' identifier='<none>'
Toplevel #1 closed: Firefox (firefox)
```

## Protocol Details

This tool implements a client for the `ext_foreign_toplevel_list_v1` Wayland protocol, which allows applications to:
- Get a list of all open windows (toplevels)
- Receive notifications when windows are created or destroyed
- Get window properties (title, app_id, identifier)
- Receive updates when window properties change

This is the modern replacement for `wlr_foreign_toplevel_management_unstable_v1` and is used by task bars, docks, and window switchers.
