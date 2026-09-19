//! A Markdown file, in a window.
//!
//! The whole of this crate's use, end to end: read the bytes, [`source_text`]
//! them, [`parse`] them into blocks, and hand those to the toolkit's document
//! renderer. There is no Markdown-aware drawing code here and none in the
//! toolkit either — the blocks *are* the interface between the two.
//!
//! ```sh
//! cargo run -p otto-md-kit --example markdown_window -- README.md
//! ```
//!
//! With no path it shows a document written into this file, so the example
//! runs anywhere. Scroll with the wheel or a touchpad — the toolkit's
//! [`ScrollView`] does the physics, the rubber band and the scrollbar — or
//! with the arrow keys, Page Up/Down and Home/End. Click a link to open it.

// A single-threaded example: the Arc<Mutex<..>> mirrors the shape a real
// otto-kit application uses for state shared with its draw closure.
#![allow(clippy::arc_with_non_send_sync)]

use std::sync::{Arc, Mutex};

use otto_kit::components::scroll::ScrollView;
use otto_kit::prelude::*;
use otto_kit::preview::document::{self, Line};
use otto_md_kit::{parse, source_text, Block};
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use wayland_client::protocol::wl_keyboard;

const WIDTH: i32 = 720;
const HEIGHT: i32 = 640;
/// The margin the text is set in. A document read at arm's length wants air
/// around it far more than it wants the last few characters of line length.
const MARGIN: f32 = 32.0;
/// What an arrow key moves. Not a line of the document — the lines are not
/// all one height — just a comfortable step.
const KEY_STEP: f32 = 48.0;

/// The document, and where the reader is in it.
///
/// The wrap is cached against the width it was made for rather than recomputed
/// per frame: wrapping measures every line in the file, and a window that is
/// not being resized is the same width it was last frame.
struct Document {
    blocks: Vec<Block>,
    lines: Vec<Line>,
    wrapped_for: f32,
    /// Offset, momentum, rubber band and scrollbar, all of it the toolkit's.
    scroll: ScrollView,
    width: f32,
    height: f32,
}

impl Document {
    fn viewport(&self) -> Rect {
        Rect::from_ltrb(
            MARGIN,
            MARGIN,
            (self.width - MARGIN).max(MARGIN + 1.0),
            (self.height - MARGIN).max(MARGIN + 1.0),
        )
    }

    /// Re-wrap if the window changed width, and keep the scroll view told how
    /// long the document is. Called from the draw, which is the one place that
    /// knows the wrap is about to be read.
    fn relayout(&mut self) {
        let viewport = self.viewport();
        self.scroll.set_viewport(viewport);
        if (viewport.width() - self.wrapped_for).abs() > 0.5 {
            self.lines = document::wrap(&self.blocks, viewport.width());
            self.wrapped_for = viewport.width();
        }
        let length = self
            .lines
            .last()
            .map_or(0.0, |line| line.top + line.height)
            // A last line flush against the bottom edge reads as cut off.
            + MARGIN;
        self.scroll.set_content_length(length);
    }

    /// The link under a point of the window, if the point is over one.
    fn link_at(&self, x: f32, y: f32) -> Option<String> {
        let viewport = self.viewport();
        if x < viewport.left || x >= viewport.right || y < viewport.top || y >= viewport.bottom {
            return None;
        }
        // Into the space the lines were laid out in, which is what the scroll
        // view translates by when it draws them.
        let (x, y) = self.scroll.viewport_to_content(x, y);
        let band = Rect::from_xywh(
            0.0,
            self.scroll.offset(),
            viewport.width(),
            viewport.height(),
        );
        document::link_at_scrolled(band, &self.lines, self.scroll.offset(), (x, y))
            .map(str::to_string)
    }

    fn scroll_by(&mut self, delta: f32) -> bool {
        let offset = self.scroll.offset();
        self.scroll.scroll_to(offset + delta)
    }
}

/// Hand a link to the desktop, if it is the sort of link this window is
/// willing to act on.
///
/// The parser keeps destinations exactly as the file wrote them and judges
/// nothing — see [`otto_md_kit`]. Deciding what is safe to open is the host's
/// job, and this host is deliberately narrow: a document off disk is
/// untrusted text, and `xdg-open` will happily launch a handler for a scheme
/// nobody here meant to support.
fn open_link(href: &str) {
    let allowed = ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| href.starts_with(scheme));
    if !allowed {
        println!("not opening {href}: only http, https and mailto links are followed");
        return;
    }
    match std::process::Command::new("xdg-open").arg(href).spawn() {
        Ok(_) => println!("opening {href}"),
        Err(err) => println!("could not open {href}: {err}"),
    }
}

struct MarkdownWindow {
    document: Arc<Mutex<Document>>,
    window: Option<Window>,
    title: String,
}

impl MarkdownWindow {
    /// Repaint, if there is a window yet to repaint.
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_frame();
        }
    }
}

impl App for MarkdownWindow {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new(&self.title, WIDTH, HEIGHT)?;
        window.set_background(otto_kit::preview::background(&AppContext::current_theme()));

        let document = self.document.clone();
        window.on_draw(move |canvas| {
            let theme = AppContext::current_theme();
            canvas.clear(otto_kit::preview::background(&theme));

            let mut document = document.lock().unwrap();
            document.relayout();
            // Two borrows of one lock guard, so the lines and the scroll view
            // have to be split apart by hand.
            let Document { lines, scroll, .. } = &mut *document;
            // The scroll view clips, translates and paints its own scrollbar;
            // the closure paints the document at its own coordinates, and
            // `visible` says which band of it is worth painting.
            scroll.render(canvas, &theme, |canvas, visible| {
                // No copy button: this example does not follow the pointer over
                // the document's code blocks.
                document::draw_scrolled(canvas, visible, lines, visible.top, &theme, None);
            });
        });

        let document = self.document.clone();
        window.on_pointer_event(move |events| {
            let mut document = document.lock().unwrap();
            for event in events {
                let (x, y) = (event.position.0 as f32, event.position.1 as f32);
                match &event.kind {
                    PointerEventKind::Axis { vertical, .. } => {
                        if vertical.stop {
                            document.scroll.on_wheel_end();
                        } else if vertical.discrete != 0 {
                            document.scroll.on_wheel_discrete(vertical.discrete as f32);
                        } else {
                            document.scroll.on_wheel(vertical.absolute as f32);
                        }
                    }
                    // The scrollbar gets first refusal on a press: dragging
                    // its thumb is not a click on the word underneath it.
                    PointerEventKind::Press { .. } => {
                        if !document.scroll.on_pointer_down(x, y) {
                            if let Some(href) = document.link_at(x, y) {
                                open_link(&href);
                            }
                        }
                    }
                    PointerEventKind::Release { .. } => document.scroll.on_pointer_up(),
                    PointerEventKind::Motion { .. } => {
                        document.scroll.on_pointer_drag(x, y);
                        document.scroll.on_pointer_move(x, y);
                    }
                    PointerEventKind::Leave { .. } => document.scroll.on_pointer_leave(),
                    _ => {}
                }
            }
        });

        AppContext::register_window(window.clone());
        self.window = Some(window);
        Ok(())
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        let Some((width, height)) = configure.new_size.0.zip(configure.new_size.1) else {
            return;
        };
        let mut document = self.document.lock().unwrap();
        document.width = width.get() as f32;
        document.height = height.get() as f32;
    }

    fn on_keyboard_event(
        &mut self,
        _ctx: &AppContext,
        key: u32,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        if state != wl_keyboard::KeyState::Pressed {
            return;
        }
        // evdev codes: the raw hook is enough here because none of these keys
        // produce text, so there is nothing a keymap would change.
        const DOWN: u32 = 108;
        const UP: u32 = 103;
        const PAGE_DOWN: u32 = 109;
        const PAGE_UP: u32 = 104;
        const HOME: u32 = 102;
        const END: u32 = 107;
        const SPACE: u32 = 57;
        const Q: u32 = 16;

        let mut document = self.document.lock().unwrap();
        // A page is a screenful less a little, so the line you were reading
        // is still there to pick the next one up from.
        let page = (document.viewport().height() - KEY_STEP).max(KEY_STEP);
        let moved = match key {
            DOWN => document.scroll_by(KEY_STEP),
            UP => document.scroll_by(-KEY_STEP),
            PAGE_DOWN | SPACE => document.scroll_by(page),
            PAGE_UP => document.scroll_by(-page),
            HOME => document.scroll.scroll_to(0.0),
            END => document.scroll.scroll_to(f32::MAX),
            Q => {
                std::process::exit(0);
            }
            _ => false,
        };
        if moved {
            document.scroll.flash_scrollbar();
        }
        drop(document);
        if moved {
            self.redraw();
        }
    }

    /// Momentum, the rubber band and the scrollbar's fade are all things that
    /// keep moving after the input that started them, so the view is ticked
    /// every time round the loop and repainted while it has something to say.
    fn on_update(&mut self, _ctx: &AppContext) {
        let moving = self.document.lock().unwrap().scroll.tick();
        if moving {
            self.redraw();
        }
    }
}

/// Shown when no path is given, so the example has something to render
/// wherever it is run — and so the flattenings the crate documents can be seen
/// rather than only read about.
const SAMPLE: &str = "\
# Markdown, read as a document

This window is drawing blocks, not Markdown. `otto-md-kit` parsed the source
into headings, paragraphs, items, quotes, code and rules, and the toolkit's
`document` renderer wrapped those to the width and painted them. Neither half
knows what the other is for.

## What it does with the awkward parts

- *Emphasis* and **strength** compose, and `snake_case_names` are left alone.
- A [link](https://example.com) keeps its label *and* its destination — click
  one and this window hands it to the desktop.
- 1. Numbered lists keep their own numbers.
- A [reference-style link][ref] is drawn as a link and goes nowhere, because
  nothing resolved it.

> A quote absorbs the lines that follow it, because that is how people write
> them.

```rust
// A fence swallows what would otherwise be markup:
# not a heading
```

| a table | is drawn |
|---------|----------|
| as code | because  |
| columns | need     |
| fonts   | to align |

---

Pass a path to read a real file:
`cargo run -p otto-md-kit --example markdown_window -- README.md`
";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1);
    let (title, text) = match &path {
        Some(path) => {
            let bytes = std::fs::read(path)?;
            let text = source_text(bytes).ok_or("that file is not text")?;
            let name = std::path::Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            (name, text)
        }
        None => ("Markdown".to_string(), SAMPLE.to_string()),
    };

    let document = Document {
        blocks: parse(&text),
        lines: Vec::new(),
        // Nothing is wrapped yet, and no real width is negative.
        wrapped_for: -1.0,
        scroll: ScrollView::new(Rect::from_xywh(
            MARGIN,
            MARGIN,
            WIDTH as f32 - 2.0 * MARGIN,
            HEIGHT as f32 - 2.0 * MARGIN,
        )),
        width: WIDTH as f32,
        height: HEIGHT as f32,
    };

    let app = MarkdownWindow {
        document: Arc::new(Mutex::new(document)),
        window: None,
        title,
    };
    tokio::runtime::Runtime::new()?.block_on(async { AppRunner::new(app).run() })
}
