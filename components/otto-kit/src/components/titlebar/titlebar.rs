use skia_safe::{Canvas, Color, Paint, Rect};

use crate::common::Renderable;

/// A group of items in a titlebar (typically window controls)
pub struct TitlebarGroup {
    items: Vec<Box<dyn Renderable>>,
    spacing: f32,
}

impl TitlebarGroup {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            spacing: 8.0,
        }
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Add an item to the group
    #[allow(clippy::should_implement_trait)]
    pub fn add<T: Renderable + 'static>(mut self, item: T) -> Self {
        self.items.push(Box::new(item));
        self
    }

    pub fn build(self) -> Self {
        self
    }

    fn render_at(&self, canvas: &Canvas, x: f32, y: f32, height: f32) -> f32 {
        let mut current_x = x;

        for (i, item) in self.items.iter().enumerate() {
            let (item_width, item_height) = item.intrinsic_size().unwrap_or((32.0, height));

            // Center vertically within titlebar height
            let item_y = y + (height - item_height) / 2.0;

            // Render the item
            canvas.save();
            canvas.translate((current_x, item_y));
            item.render(canvas);
            canvas.restore();

            current_x += item_width;

            // Add spacing between items (except after last item)
            if i < self.items.len() - 1 {
                current_x += self.spacing;
            }
        }

        current_x - x // Return total width used
    }

    fn measure_width(&self, height: f32) -> f32 {
        let mut total_width = 0.0;
        for (i, item) in self.items.iter().enumerate() {
            let (item_width, _) = item.intrinsic_size().unwrap_or((32.0, height));
            total_width += item_width;

            if i < self.items.len() - 1 {
                total_width += self.spacing;
            }
        }
        total_width
    }
}

impl Default for TitlebarGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// The surface treatment of a titlebar: a translucent tint over whatever is
/// behind it, lifted by a very subtle top-to-bottom gradient, a hairline
/// highlight along the top edge and a hairline shade along the bottom.
///
/// The tint is meant to be translucent and the window surface to carry
/// `background_blur`, so the compositor blurs the desktop behind it. When the
/// component draws onto a plain canvas (offscreen previews, clients without the
/// blur protocol), `backdrop_blur` reproduces the same effect locally through
/// a save-layer backdrop filter.
#[derive(Debug, Clone, Copy)]
pub struct TitlebarMaterial {
    /// Translucent fill drawn over the (blurred) backdrop
    pub tint: Color,
    /// Local backdrop blur sigma. 0 leaves the backdrop untouched — use that
    /// when the compositor already blurs behind the surface.
    pub backdrop_blur: f32,
    /// Strength of the vertical sheen, 0.0..=1.0. Keep it low: this reads as
    /// depth only while it stays below the threshold of being noticed.
    pub gradient: f32,
    /// Hairline along the very top edge (the lit bevel)
    pub top_highlight: Option<Color>,
    /// Hairline along the bottom edge, separating bar from content
    pub bottom_shade: Option<Color>,
}

impl TitlebarMaterial {
    /// Light material for the focused window
    pub fn light_active() -> Self {
        Self {
            tint: Color::from_argb(0xE4, 0xEC, 0xEC, 0xEE),
            backdrop_blur: 0.0,
            gradient: 0.5,
            top_highlight: Some(Color::from_argb(0x99, 0xFF, 0xFF, 0xFF)),
            bottom_shade: Some(Color::from_argb(0x26, 0x00, 0x00, 0x00)),
        }
    }

    /// Light material for an unfocused window: flatter and less saturated,
    /// the depth cue that says "not this one".
    pub fn light_inactive() -> Self {
        Self {
            tint: Color::from_argb(0xDC, 0xF4, 0xF4, 0xF6),
            gradient: 0.2,
            top_highlight: Some(Color::from_argb(0x55, 0xFF, 0xFF, 0xFF)),
            bottom_shade: Some(Color::from_argb(0x14, 0x00, 0x00, 0x00)),
            ..Self::light_active()
        }
    }

    pub fn dark_active() -> Self {
        Self {
            tint: Color::from_argb(0xE6, 0x32, 0x34, 0x3A),
            backdrop_blur: 0.0,
            gradient: 0.5,
            top_highlight: Some(Color::from_argb(0x40, 0xFF, 0xFF, 0xFF)),
            bottom_shade: Some(Color::from_argb(0x66, 0x00, 0x00, 0x00)),
        }
    }

    pub fn dark_inactive() -> Self {
        Self {
            tint: Color::from_argb(0xDE, 0x2A, 0x2C, 0x31),
            gradient: 0.2,
            top_highlight: Some(Color::from_argb(0x22, 0xFF, 0xFF, 0xFF)),
            bottom_shade: Some(Color::from_argb(0x4D, 0x00, 0x00, 0x00)),
            ..Self::dark_active()
        }
    }

    /// A flat, fully opaque material — the pre-material look, for comparison.
    pub fn flat(color: Color) -> Self {
        Self {
            tint: color,
            backdrop_blur: 0.0,
            gradient: 0.0,
            top_highlight: None,
            bottom_shade: None,
        }
    }

    pub fn with_tint(mut self, tint: Color) -> Self {
        self.tint = tint;
        self
    }

    pub fn with_backdrop_blur(mut self, sigma: f32) -> Self {
        self.backdrop_blur = sigma;
        self
    }

    /// The same material with its tint filled in to full opacity.
    ///
    /// The tints are translucent because they are meant to sit over a blurred
    /// backdrop. With nothing blurred behind the bar — an unfocused window,
    /// whose blur has been dropped — translucency is not a softer version of
    /// the same bar: it is the desktop showing through, which costs the title
    /// and the controls their contrast against it.
    pub fn opaque(mut self) -> Self {
        let tint = self.tint;
        self.tint = Color::from_argb(0xFF, tint.r(), tint.g(), tint.b());
        self
    }

    pub fn with_gradient(mut self, gradient: f32) -> Self {
        self.gradient = gradient;
        self
    }
}

/// A horizontal titlebar component with centered text and trailing controls
pub struct Titlebar {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Styling
    background_color: Option<Color>,
    material: Option<TitlebarMaterial>,
    border_color: Option<Color>,
    padding: f32,

    // Content
    title: Option<Box<dyn Renderable>>,
    leading: Option<TitlebarGroup>,
    controls: Option<TitlebarGroup>,

    /// Rounds the two top corners, matching the window frame radius.
    corner_radius: f32,
}

impl Titlebar {
    /// Create a new titlebar
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 28.0, // Compact titlebar height
            background_color: None,
            material: None,
            border_color: None,
            padding: 8.0,
            title: None,
            leading: None,
            controls: None,
            corner_radius: 0.0,
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Set the surface treatment (tint, sheen, bevel hairlines). Takes
    /// precedence over [`with_background`].
    pub fn with_material(mut self, material: TitlebarMaterial) -> Self {
        self.material = Some(material);
        self
    }

    pub fn with_border_bottom(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the centered title
    pub fn with_title<T: Renderable + 'static>(mut self, title: T) -> Self {
        self.title = Some(Box::new(title));
        self
    }

    /// Set the window controls (right-aligned)
    pub fn with_controls(mut self, controls: TitlebarGroup) -> Self {
        self.controls = Some(controls);
        self
    }

    /// Set a leading (left-aligned) group, e.g. macOS-style traffic lights
    pub fn with_leading(mut self, leading: TitlebarGroup) -> Self {
        self.leading = Some(leading);
        self
    }

    /// Round the top corners to match the window frame — or leave them square,
    /// on a desktop configured without rounded corners.
    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = crate::corners::radius(radius);
        self
    }

    pub fn build(self) -> Self {
        self
    }

    /// Paint the material: (optionally blurred) backdrop, translucent tint, a
    /// very subtle vertical sheen, then the two bevel hairlines.
    fn draw_material(&self, canvas: &Canvas, material: &TitlebarMaterial, rect: Rect) {
        // Local backdrop blur. Skipped when the compositor already blurs behind
        // the surface (`backdrop_blur == 0`).
        if material.backdrop_blur > 0.0 {
            if let Some(filter) = skia_safe::image_filters::blur(
                (material.backdrop_blur, material.backdrop_blur),
                None,
                None,
                skia_safe::image_filters::CropRect::from(rect),
            ) {
                let rec = skia_safe::canvas::SaveLayerRec::default()
                    .bounds(&rect)
                    .backdrop(&filter);
                canvas.save_layer(&rec);
                canvas.restore();
            }
        }

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(material.tint);
        canvas.draw_rect(rect, &paint);

        // Vertical sheen: lighter at the top, a touch darker at the bottom.
        // Both ends are scaled by `gradient` so the whole thing stays a hint.
        if material.gradient > 0.0 {
            let g = material.gradient.clamp(0.0, 1.0);
            let top = (0x1C as f32 * g) as u8;
            let bottom = (0x0E as f32 * g) as u8;
            let colors = [
                Color::from_argb(top, 0xFF, 0xFF, 0xFF),
                Color::from_argb(0, 0xFF, 0xFF, 0xFF),
                Color::from_argb(bottom, 0x00, 0x00, 0x00),
            ];
            if let Some(shader) = skia_safe::gradient_shader::linear(
                (
                    skia_safe::Point::new(rect.left, rect.top),
                    skia_safe::Point::new(rect.left, rect.bottom),
                ),
                &colors[..],
                Some(&[0.0, 0.55, 1.0][..]),
                skia_safe::TileMode::Clamp,
                None,
                None,
            ) {
                let mut sheen = Paint::default();
                sheen.set_anti_alias(true);
                sheen.set_shader(shader);
                canvas.draw_rect(rect, &sheen);
            }
        }

        // Bevel hairlines. Drawn at half-pixel offsets so they stay crisp
        // instead of straddling two rows.
        let mut line = Paint::default();
        line.set_anti_alias(true);
        line.set_style(skia_safe::PaintStyle::Stroke);
        line.set_stroke_width(1.0);
        if let Some(highlight) = material.top_highlight {
            line.set_color(highlight);
            let y = rect.top + 0.5;
            // Inset by the corner radius so the highlight doesn't cut across
            // the rounded corners.
            let inset = self.corner_radius * 0.8;
            canvas.draw_line((rect.left + inset, y), (rect.right - inset, y), &line);
        }
        if let Some(shade) = material.bottom_shade {
            line.set_color(shade);
            let y = rect.bottom - 0.5;
            canvas.draw_line((rect.left, y), (rect.right, y), &line);
        }
    }
}

impl Default for Titlebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderable for Titlebar {
    fn render(&self, canvas: &Canvas) {
        // Save canvas state
        canvas.save();

        // Clip to titlebar bounds to prevent overflow. With a corner radius the
        // clip follows the window frame on the top two corners only — the
        // bottom edge butts against the content, so it stays square.
        let clip_rect = Rect::from_xywh(self.x, self.y, self.width, self.height);
        if self.corner_radius > 0.0 {
            let r = self.corner_radius;
            let rrect = skia_safe::RRect::new_rect_radii(
                clip_rect,
                &[
                    (r, r).into(),
                    (r, r).into(),
                    (0.0, 0.0).into(),
                    (0.0, 0.0).into(),
                ],
            );
            canvas.clip_rrect(rrect, None, Some(true));
        } else {
            canvas.clip_rect(clip_rect, None, Some(true));
        }

        // Draw background
        let rect = Rect::from_xywh(self.x, self.y, self.width, self.height);
        if let Some(material) = self.material {
            self.draw_material(canvas, &material, rect);
        } else if let Some(bg_color) = self.background_color {
            let mut paint = Paint::default();
            paint.set_color(bg_color);
            paint.set_anti_alias(true);
            canvas.draw_rect(rect, &paint);
        }

        let content_height = self.height - self.padding * 2.0;

        // Measure both groups first: the title is centered in the space left
        // between them, and it must not slide under either side.
        let leading_width = self
            .leading
            .as_ref()
            .map(|g| g.measure_width(content_height))
            .unwrap_or(0.0);
        let controls_width = self
            .controls
            .as_ref()
            .map(|g| g.measure_width(content_height))
            .unwrap_or(0.0);

        let reserved_left = if leading_width > 0.0 {
            leading_width + self.padding * 2.0
        } else {
            0.0
        };
        let reserved_right = if controls_width > 0.0 {
            controls_width + self.padding * 2.0
        } else {
            0.0
        };

        // Render centered title
        if let Some(ref title) = self.title {
            if let Some((title_width, title_height)) = title.intrinsic_size() {
                // Center on the titlebar, then push it clear of whichever side
                // it would collide with (keeps short titles optically centered
                // while long ones stay readable).
                let mut center_x = self.x + (self.width - title_width) / 2.0;
                center_x = center_x.max(self.x + reserved_left);
                center_x = center_x.min(self.x + self.width - reserved_right - title_width);
                center_x = center_x.max(self.x + reserved_left);
                let center_y = self.y + (self.height - title_height) / 2.0;

                canvas.save();
                canvas.translate((center_x, center_y));
                title.render(canvas);
                canvas.restore();
            }
        }

        // Render the leading group on the left
        if let Some(ref leading) = self.leading {
            leading.render_at(
                canvas,
                self.x + self.padding,
                self.y + self.padding,
                content_height,
            );
        }

        // Render controls on the right
        if let Some(ref controls) = self.controls {
            let controls_x = self.x + self.width - controls_width - self.padding;
            let controls_y = self.y + self.padding;
            controls.render_at(canvas, controls_x, controls_y, content_height);
        }

        // Restore before drawing border (so border isn't clipped)
        canvas.restore();

        // Draw bottom border (outside clip region to ensure full width)
        if let Some(border_color) = self.border_color {
            let mut paint = Paint::default();
            paint.set_color(border_color);
            paint.set_style(skia_safe::PaintStyle::Stroke);
            paint.set_stroke_width(1.0);
            paint.set_anti_alias(true);

            let y = self.y + self.height - 0.5;
            canvas.draw_line((self.x, y), (self.x + self.width, y), &paint);
        }
    }
}
