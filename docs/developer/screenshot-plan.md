## Screenshot Portal Implementation Plan

> **Status**: Planning doc — not implemented. For the related but separate
> `wlr-screencopy-v1` Wayland protocol (used by `grim`, `wf-recorder`,
> `wl-mirror`, OBS via wlrobs) see `src/state/screencopy.rs` and the
> "Related: wlr-screencopy-v1" section of [screenshare.md](./screenshare.md).
> wlr-screencopy is already in production and reuses the screenshare
> dmabuf blit path; this doc covers the D-Bus portal interface that
> GTK/Qt screenshot apps use.

### Overview

Implement `org.freedesktop.impl.portal.Screenshot` D-Bus interface to allow third-party screenshot tools (GNOME Screenshot, Spectacle, Flameshot, etc.) to capture screen content.

**Key Difference from ScreenCast**: Screenshots use file-based capture (one-shot PNG) rather than PipeWire streaming.

### Architecture

```
┌────────────────────────────────────────────────────────────────┐
│  Screenshot App (gnome-screenshot, spectacle, flameshot)       │
│       │                                                        │
│       ▼ Portal D-Bus API (org.freedesktop.portal.Screenshot)   │
│                                                                │
│  xdg-desktop-portal (system service)                           │
│       │                                                        │
│       ▼ Backend D-Bus (org.freedesktop.impl.portal.Screenshot) │
│                                                                │
│  xdg-desktop-portal-otto                                       │
│       │                                                        │
│       ▼ Compositor D-Bus (org.otto.Screenshot)                 │
│                                                                │
│  Otto Compositor                                               │
│       │                                                        │
│       ├─► Capture single frame (reuse FrameTapManager)         │
│       ├─► Encode to PNG (image crate)                          │
│       ├─► Save to temp file (/tmp/screenshot-XXX.png)          │
│       └─► Return file:// URI                                   │
│                                                                │
│  Screenshot App reads file and processes                       │
└────────────────────────────────────────────────────────────────┘
```

### Data Flow

**Screenshot Request**:
1. App calls `org.freedesktop.portal.Screenshot.Screenshot()`
2. xdg-desktop-portal forwards to portal-otto backend
3. Portal-otto sends D-Bus request to compositor
4. Compositor captures current frame (dmabuf or RGBA)
5. Convert to CPU memory if needed (dmabuf → RGBA via existing lazy-loading)
6. Encode to PNG using `image` crate
7. Save to `/tmp/screenshot-XXXXXX.png` or `$XDG_RUNTIME_DIR`
8. Return `file:///tmp/screenshot-XXXXXX.png` URI
9. App handles file (display, save elsewhere, clipboard, etc.)

### Implementation

#### Phase 1: Basic Screenshot (Minimum Viable)

**Portal Backend** (`components/xdg-desktop-portal-otto/src/screenshot.rs`):
```rust
// New D-Bus interface implementation
impl Screenshot for PortalBackend {
    async fn screenshot(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        options: HashMap<String, Value<'_>>,
    ) -> Result<(u32, HashMap<String, Value>)> {
        // Forward to compositor via existing D-Bus connection
        // Return (response_code, {"uri": "file:///tmp/screenshot-XXX.png"})
    }
}
```

**Compositor Handler** (`src/screenshare/screenshot.rs`):
```rust
pub enum CompositorCommand {
    // ... existing commands
    Screenshot {
        output_name: String,
        response_tx: oneshot::Sender<Result<String>>, // URI
    },
}

pub async fn handle_screenshot(
    state: &mut Otto,
    output_name: &str,
) -> Result<String> {
    // 1. Capture single frame using FrameTapManager
    // 2. Get RGBA data from dmabuf
    // 3. Encode to PNG
    // 4. Save to temp file
    // 5. Return file:// URI
}
```

**Frame Capture**:
- Reuse existing `FrameTapManager` for one-shot capture
- Use `ExportMem` trait to get RGBA data from current framebuffer
- No need for streaming infrastructure

**PNG Encoding**:
```rust
use image::{ImageBuffer, Rgba};

fn encode_png(rgba_data: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>> {
    let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba_data)
        .ok_or("Failed to create image")?;
    
    let mut png_data = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_data), image::ImageFormat::Png)?;
    Ok(png_data)
}
```

**File Management**:
```rust
use tempfile::NamedTempFile;

fn save_screenshot(png_data: &[u8]) -> Result<String> {
    let temp_file = NamedTempFile::new_in("/tmp")?
        .with_prefix("screenshot-")
        .with_suffix(".png");
    
    temp_file.write_all(png_data)?;
    let path = temp_file.path().to_string_lossy();
    
    Ok(format!("file://{}", path))
}
```

### Phase 2: Color Picker (Optional)

**Portal Method**:
```rust
async fn pick_color(
    &self,
    handle: ObjectPath<'_>,
    app_id: &str,
    parent_window: &str,
    options: HashMap<String, Value<'_>>,
) -> Result<(u32, HashMap<String, Value>)> {
    // Return (response_code, {"color": (r, g, b)})
}
```

### Dependencies

Add to `Cargo.toml`:
```toml
[dependencies]
image = { version = "0.25", default-features = false, features = ["png"] }
tempfile = "3.0"
```

### File Structure

```
components/xdg-desktop-portal-otto/src/
├── screenshot.rs          # New: Screenshot D-Bus interface
└── main.rs               # Register Screenshot interface

src/screenshare/
├── screenshot.rs         # New: Screenshot capture & encoding
└── mod.rs               # Export screenshot module
```

### D-Bus Interface Specification

#### Screenshot Method

```xml
<method name="Screenshot">
  <arg type="o" name="handle" direction="in"/>
  <arg type="s" name="app_id" direction="in"/>
  <arg type="s" name="parent_window" direction="in"/>
  <arg type="a{sv}" name="options" direction="in"/>
  <arg type="u" name="response" direction="out"/>
  <arg type="a{sv}" name="results" direction="out"/>
</method>
```

**Options**:
- `modal` (b): Whether dialog should be modal (ignored - apps handle UI)
- `interactive` (b): Whether to show dialog (ignored - apps handle UI)

**Results**:
- `uri` (s): `file://` URI to saved screenshot PNG

### Testing

#### Manual Testing

```bash
# Install screenshot tools
sudo pacman -S gnome-screenshot spectacle flameshot

# Test basic screenshot
gnome-screenshot

# Test with flameshot
flameshot gui

# Test color picker
# (Use app that supports color picking via portal)
```

#### D-Bus Direct Testing

```bash
# Call Screenshot method directly
gdbus call --session \
  --dest org.freedesktop.impl.portal.ScreenCast.otto \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.impl.portal.Screenshot.Screenshot \
  "/org/freedesktop/portal/desktop/request/1_1" \
  "test.app" \
  "" \
  "{}"
```

### Implementation Checklist

#### Phase 1: Basic Screenshot
- [ ] Add `image` and `tempfile` dependencies
- [ ] Create `src/screenshare/screenshot.rs`
- [ ] Implement Screenshot D-Bus command in compositor
- [ ] Add one-shot frame capture using FrameTapManager
- [ ] Implement PNG encoding
- [ ] Implement temp file creation with proper URI format
- [ ] Create `components/xdg-desktop-portal-otto/src/screenshot.rs`
- [ ] Implement Screenshot D-Bus interface in portal backend
- [ ] Register Screenshot interface in portal backend main.rs
- [ ] Test with gnome-screenshot
- [ ] Test with other screenshot tools

#### Phase 2: Color Picker (Optional)
- [ ] Implement PickColor D-Bus command
- [ ] Add pixel reading from framebuffer
- [ ] Implement BGRA → RGB conversion
- [ ] Test with color picker apps

### Notes

- **No PipeWire**: Screenshots use simple file-based capture, not streaming
- **No UI**: Third-party apps provide their own selection/annotation UI
- **Reuse infrastructure**: Leverage existing FrameTapManager and frame capture
- **Temporary files**: Use `/tmp` or `$XDG_RUNTIME_DIR` for screenshot storage
- **Clean up**: Apps responsible for deleting files after use
- **Single output**: Initial implementation captures full primary output
- **Format**: PNG only (most compatible, lossless)

### Future Enhancements

- Support for specific output selection
- Support for window-specific screenshots (via window ID)
- JPEG format support (with quality parameter)
- Custom save location (from app-provided parameters)
- Screenshot delay/timer support
- Specific area capture (from coordinates)
