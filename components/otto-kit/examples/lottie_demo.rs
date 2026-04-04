use otto_kit::lottie::LottiePlayer;
use otto_kit::prelude::*;
use smithay_client_toolkit::shell::WaylandSurface;
use std::time::{Duration, Instant};

const SAMPLE_LOTTIE: &[u8] = include_bytes!("../../otto-islands/assets/touch_id.json");

struct LottieApp {
    window: Option<Window>,
    player: Option<LottiePlayer>,
    start: Instant,
}

impl App for LottieApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let player = LottiePlayer::from_json_with_color(SAMPLE_LOTTIE, [1.0, 1.0, 1.0, 1.0])
            .map_err(|e| format!("Failed to load Lottie: {e}"))?;

        let (w, h) = player.size();
        println!(
            "Loaded animation: {}x{}, duration: {:.2}s",
            w,
            h,
            player.duration()
        );

        let mut window = Window::new("Lottie Animation Demo", 400, 400)?;
        window.set_background(Color::from_rgb(30, 30, 30));

        self.player = Some(player);
        self.window = Some(window);
        self.start = Instant::now();

        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let Some(player) = &self.player else { return };
        let Some(window) = &self.window else { return };

        let elapsed = self.start.elapsed().as_secs_f64();
        let duration = player.duration();
        let progress = if duration > 0.0 {
            (elapsed % duration) / duration
        } else {
            0.0
        };

        let toplevel = window.surface().unwrap();
        toplevel.draw(|canvas| {
            canvas.clear(Color::from_rgb(30, 30, 30));
            player.render(canvas, progress, 100.0, 100.0, 200.0, 200.0);

            Label::new(&format!("progress: {:.0}%", progress * 100.0))
                .at(140.0, 340.0)
                .with_style(styles::CAPTION_1)
                .with_color(Color::from_argb(180, 255, 255, 255))
                .render(canvas);
        });
        toplevel.window().commit();
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(Duration::from_millis(16)) // ~60fps
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = LottieApp {
        window: None,
        player: None,
        start: Instant::now(),
    };
    AppRunner::new(app).run()?;
    Ok(())
}
