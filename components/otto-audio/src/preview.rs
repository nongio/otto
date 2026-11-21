//! `--png`: the window drawn offscreen, light and dark side by side, from
//! the live graph. No display needed, and nothing is changed: the connection
//! only listens.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use otto_audio::graph_view::GraphMode;
use otto_audio::WINDOW_APP;
use otto_audio_graph::Connection;
use otto_kit::skia::{surfaces, EncodedImageFormat};
use otto_kit::theme::Theme;

use crate::ui::{Pane, Ui};

/// How long to listen before drawing: long enough for every node's details
/// and the engine settings to arrive.
const LISTEN: Duration = Duration::from_millis(1500);
/// The drawing's size, in points, and its pixel scale.
const W: f32 = 1100.0;
const H: f32 = 700.0;
const SCALE: f32 = 2.0;

/// Draw the window on `pane`, the Graph section in `mode`, into the PNG at
/// `out`.
pub fn render(out: &str, pane: Pane, mode: GraphMode) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel();
    let connection = Connection::start(WINDOW_APP, move |event| {
        let _ = tx.send(event);
    });
    let mut uis = [Ui::new(connection.clone()), Ui::new(connection)];
    let until = Instant::now() + LISTEN;
    while let Ok(event) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        for ui in &mut uis {
            ui.apply(event.clone());
        }
    }
    let width = (W * 2.0 * SCALE) as i32;
    let height = (H * SCALE) as i32;
    let mut surface = surfaces::raster_n32_premul((width, height)).ok_or("no raster surface")?;
    let canvas = surface.canvas();
    for (index, (ui, theme)) in uis
        .iter_mut()
        .zip([Theme::light(), Theme::dark()])
        .enumerate()
    {
        ui.size = (W, H);
        ui.show(pane);
        ui.show_graph_mode(mode);
        if pane == Pane::VirtualDevices {
            ui.preview_virtual();
        }
        if pane == Pane::Effects {
            ui.preview_effects();
        }
        canvas.save();
        canvas.scale((SCALE, SCALE));
        canvas.translate((index as f32 * W, 0.0));
        canvas.clip_rect(otto_kit::skia::Rect::from_wh(W, H), None, None);
        ui.draw(canvas, &theme);
        canvas.restore();
    }
    let data = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, None)
        .ok_or("PNG encoding failed")?;
    std::fs::write(out, data.as_bytes())?;
    Ok(())
}
