//! What `AppContext::outputs()` sees, and when.
//!
//! The display probe a client has is `wl_output`: it carries each output's
//! name, its place in the desktop and every mode its connector can be driven
//! at. This prints that list at `on_app_ready` and again on every frame for a
//! moment afterwards, because *when* it is asked matters — the compositor
//! announces its outputs after the app is up, so a list read once at startup
//! is empty and stays empty.
//!
//! ```sh
//! cargo run -p otto-kit --example output_probe
//! ```

use otto_kit::prelude::*;

struct Probe {
    ticks: u32,
}

fn dump(when: &str) {
    let outputs = AppContext::outputs();
    println!("{when}: {} output(s)", outputs.len());
    for info in outputs {
        println!(
            "  {} ({} {}) at {:?} logical {:?} scale {}",
            info.name.clone().unwrap_or_default(),
            info.make,
            info.model,
            info.logical_position.unwrap_or(info.location),
            info.logical_size,
            info.scale_factor,
        );
        for mode in &info.modes {
            println!(
                "    {}x{} @ {:.2} Hz{}{}",
                mode.dimensions.0,
                mode.dimensions.1,
                mode.refresh_rate as f32 / 1000.0,
                if mode.current { " current" } else { "" },
                if mode.preferred { " preferred" } else { "" },
            );
        }
    }
}

impl App for Probe {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        dump("on_app_ready");
        Ok(())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        self.ticks += 1;
        if matches!(self.ticks, 1 | 5 | 30 | 90) {
            dump(&format!("update {}", self.ticks));
        }
        if self.ticks >= 90 {
            std::process::exit(0);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Runtime::new()?.block_on(async { AppRunner::new(Probe { ticks: 0 }).run() })
}
