//! The tinted frosted materials, side by side.
//!
//! The panels and launchers that belong to agents — the Ask field, the agent
//! islands, the sessions list — should read as one family, and the thing that
//! makes a family is the material. This lays [`otto_kit::frosted`] out as a
//! grid of layer surfaces over the desktop so the palette can be judged where
//! it will live: over a wallpaper, with the real blur behind it, every hue
//! next to the others.
//!
//! Each column is a material; the top row is the dark scheme and the bottom
//! one the light, so both are read at once whichever scheme the desktop is in.
//! The first column is the untinted popup material, for the weight every tint
//! is meant to match.
//!
//! ```sh
//! cargo run -p otto-kit --example agent_materials
//! ```

use otto_kit::frosted::Frosted;
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode};
use otto_kit::surfaces::LayerShellSurface;
use otto_kit::theme::Theme;
use otto_kit::typography::TextStyle;
use skia_safe::{Color, Color4f, Paint};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
};

const CARD_W: u32 = 92;
const CARD_H: u32 = 86;
const GAP: i32 = 8;
const MARGIN_LEFT: i32 = 12;
const MARGIN_TOP: i32 = 56;
const RADIUS: f32 = 12.0;

fn font(size: f32, weight: i32) -> skia_safe::Font {
    TextStyle {
        family: "Inter",
        weight,
        size,
    }
    .font()
}

/// One card: a layer surface wearing the material, with its name and recipe.
struct Swatch {
    surface: LayerShellSurface,
    title: String,
    detail: String,
    /// The palette the labels are drawn from — the row's scheme, not the
    /// desktop's, so the light row stays readable on a dark desktop.
    theme: Theme,
}

impl Swatch {
    fn new(
        title: String,
        colour: Color,
        col: i32,
        row: i32,
        theme: Theme,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let surface = LayerShellSurface::new(Layer::Overlay, "agent-materials", CARD_W, CARD_H)?;
        surface.set_anchor(Anchor::Top | Anchor::Left);
        surface.set_margin(
            MARGIN_TOP + row * (CARD_H as i32 + GAP),
            0,
            0,
            MARGIN_LEFT + col * (CARD_W as i32 + GAP),
        );
        surface.set_exclusive_zone(0);

        if let Some(style) = surface.base_surface().surface_style() {
            // The frost, the tint and the shape are all the compositor's: the
            // pixels behind the surface are the only ones that can be blurred,
            // and this process never sees them. The card declares the material
            // and paints its label on top of the result.
            let c = Color4f::from(colour);
            style.set_background_color(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
            style.set_blend_mode(if otto_kit::frosting::enabled() {
                BlendMode::BackgroundBlur
            } else {
                BlendMode::Normal
            });
            style.set_corner_radius(otto_kit::corners::radius(RADIUS) as f64);
            style.set_masks_to_bounds(ClipMode::Enabled);
            style.set_shadow(0.28, 24.0, 0.0, 10.0, 0.0, 0.0, 0.0);
        }

        let detail = format!("#{:02X}{:02X}{:02X}", colour.r(), colour.g(), colour.b());
        Ok(Self {
            surface,
            title,
            detail,
            theme,
        })
    }

    fn draw(&self) {
        let theme = self.theme.clone();
        let title = self.title.clone();
        let detail = self.detail.clone();
        self.surface.draw(move |canvas| {
            canvas.clear(Color::TRANSPARENT);

            let mut paint = Paint::default();
            paint.set_anti_alias(true);

            paint.set_color(theme.text_primary);
            canvas.draw_str(&title, (12.0, 26.0), &font(13.0, 600), &paint);

            paint.set_color(theme.text_secondary);
            canvas.draw_str(&detail, (12.0, 43.0), &font(10.0, 400), &paint);

            // A line of body text at the size the agent panels actually use:
            // a tint that only works without words on it is no use here.
            paint.set_color(theme.text_primary);
            canvas.draw_str("Summarise", (12.0, 66.0), &font(12.0, 400), &paint);

            paint.set_color(theme.text_tertiary);
            canvas.draw_str("waiting", (12.0, 79.0), &font(10.0, 400), &paint);
        });
        self.surface.base_surface().wl_surface().commit();
    }
}

struct MaterialsApp {
    swatches: Vec<Swatch>,
}

impl App for MaterialsApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        for (row, dark) in [true, false].into_iter().enumerate() {
            let row = row as i32;
            let theme = if dark {
                Theme::dark_palette()
            } else {
                Theme::light_palette()
            };

            // Column 0 is the untinted material, so each tint is read against
            // the surface it is meant to sit beside.
            self.swatches.push(Swatch::new(
                if dark { "Dark".into() } else { "Light".into() },
                theme.material_popup,
                0,
                row,
                theme.clone(),
            )?);

            for (col, frosted) in Frosted::ALL.into_iter().enumerate() {
                self.swatches.push(Swatch::new(
                    format!("{frosted:?}"),
                    frosted.material(dark),
                    col as i32 + 1,
                    row,
                    theme.clone(),
                )?);
            }
        }
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, _w: i32, _h: i32, _serial: u32) {
        for swatch in &self.swatches {
            swatch.draw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Runtime::new()?.block_on(async {
        AppRunner::new(MaterialsApp {
            swatches: Vec::new(),
        })
        .run()
    })
}
