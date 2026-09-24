//! The balloon's surfaces: a card that can be dragged anywhere on the output.
//!
//! As the launcher's card is: a transparent overlay surface covers the
//! output and takes the pointer only where the card is, and the card is a
//! subsurface on it. The parent never moves, so pointer positions stay put
//! while the card is dragged; over the card itself they would shift under
//! the pointer with every step it moved.

// Rust guideline compliant 2026-02-21

use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_shm, wl_subcompositor::WlSubcompositor,
    wl_subsurface::WlSubsurface, wl_surface::WlSurface,
};
use wayland_client::QueueHandle;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use otto_kit::protocols::{
    otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1,
    otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity, OttoSurfaceStyleV1},
};
use otto_kit::theme::Theme;

use crate::State;

/// Where the card first sits: this far in from the output's top-right
/// corner, in logical pixels.
const RESTING_MARGIN: i32 = 12;
/// Otto's bar height (`otto-bar`'s `BAR_HEIGHT`). The overlay covers the
/// whole output, bar included, so the card starts below it.
const BAR_HEIGHT: i32 = 30;
/// The tallest the card grows; past it the items scroll.
const MAX_CARD_HEIGHT: i32 = 640;
/// How solid the card's frost is at least: the launcher's floors, with the
/// desktop's frosting on and off.
const FROSTED_MIN_ALPHA: u8 = 0xD8;
const UNFROSTED_MIN_ALPHA: u8 = 0xF6;
/// How the card's changes of size spring: the launcher's, for its card.
const RESIZE_SECONDS: f64 = 0.34;
const RESIZE_BOUNCE: f64 = 0.35;

/// The compositor globals the balloon is made from.
#[derive(Debug)]
pub struct Shell {
    pub compositor: WlCompositor,
    pub subcompositor: WlSubcompositor,
    pub layer_shell: ZwlrLayerShellV1,
    pub style: Option<OttoSurfaceStyleManagerV1>,
}

/// The balloon on screen, from the first add until it is sent or cancelled.
#[derive(Debug)]
pub struct Panel {
    compositor: WlCompositor,
    qh: QueueHandle<State>,
    parent: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    /// Transparent, the size of the output. A real buffer rather than one
    /// pixel stretched by a viewport: Otto hit-tests layer surfaces by their
    /// buffer's size, so a stretched pixel would never take the pointer.
    backing: Option<Buffer>,
    card: WlSurface,
    subsurface: WlSubsurface,
    /// The card's material, drawn by the compositor: the launcher's frost,
    /// corners and shadow. Where it is set, it also decides where the card
    /// is drawn.
    style: Option<OttoSurfaceStyleV1>,
    /// Makes the transactions the card's changes of size spring in.
    styles: Option<OttoSurfaceStyleManagerV1>,
    /// Pixels per logical pixel, for the style's physical-pixel geometry.
    scale: f64,
    /// The output's size in logical pixels; zero until configured.
    output: (i32, i32),
    /// The card's size and top-left corner on the output, in logical pixels.
    card_size: (i32, i32),
    card_at: Option<(i32, i32)>,
    /// Where the pointer is on the output, and, while dragging, where on the
    /// card it took hold.
    pointer: (f64, f64),
    grab: Option<(f64, f64)>,
}

impl Panel {
    /// Maps nothing yet: the card appears once the compositor has sized the
    /// overlay and the card has been drawn.
    pub fn new(shell: &Shell, qh: &QueueHandle<State>, scale: i32) -> Self {
        let parent = shell.compositor.create_surface(qh, ());
        let layer = shell.layer_shell.get_layer_surface(
            &parent,
            None,
            Layer::Overlay,
            "otto-gather".into(),
            qh,
            (),
        );
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        // Never takes the keyboard: the app keeps its focus and selection.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // No input until the card has a place.
        let empty = shell.compositor.create_region(qh, ());
        parent.set_input_region(Some(&empty));
        parent.commit();

        let card = shell.compositor.create_surface(qh, ());
        card.set_buffer_scale(scale);
        // The pointer arrives on the parent, under the card.
        card.set_input_region(Some(&empty));
        empty.destroy();
        let subsurface = shell.subcompositor.get_subsurface(&card, &parent, qh, ());
        // The card redraws on its own; only its position waits for the parent.
        subsurface.set_desync();
        let style = shell.style.as_ref().map(|manager| {
            let style = manager.get_surface_style(&card, qh, ());
            apply_material(&style);
            style
        });

        Self {
            compositor: shell.compositor.clone(),
            qh: qh.clone(),
            parent,
            layer,
            backing: None,
            card,
            subsurface,
            style,
            styles: shell.style.clone(),
            scale: f64::from(scale),
            output: (0, 0),
            card_size: (0, 0),
            card_at: None,
            pointer: (0.0, 0.0),
            grab: None,
        }
    }

    /// The surface the card is drawn on.
    pub fn card(&self) -> &WlSurface {
        &self.card
    }

    /// Whether `surface` is the one that takes the balloon's pointer.
    pub fn takes_pointer(&self, surface: &WlSurface) -> bool {
        *surface == self.parent
    }

    /// Acknowledge the overlay's size and back it with a transparent buffer.
    pub fn configure(&mut self, serial: u32, (width, height): (u32, u32), pool: &mut SlotPool) {
        self.layer.ack_configure(serial);
        let output = (
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        );
        if output != self.output || self.backing.is_none() {
            self.output = output;
            let (w, h) = output;
            match pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) {
                Ok((buffer, pixels)) => {
                    pixels.fill(0);
                    if buffer.attach_to(&self.parent).is_ok() {
                        self.parent.damage_buffer(0, 0, w, h);
                    }
                    self.backing = Some(buffer);
                }
                Err(error) => tracing::warn!(%error, "no buffer behind the balloon"),
            }
        }
        self.place();
    }

    /// Whether the compositor draws the card's material.
    pub fn frosted(&self) -> bool {
        self.style.is_some()
    }

    /// The tallest the card may be on this output, in logical pixels.
    pub fn max_card_height(&self) -> f32 {
        let room = self.output.1 - BAR_HEIGHT - 2 * RESTING_MARGIN;
        room.clamp(200, MAX_CARD_HEIGHT) as f32
    }

    /// How tall the card's buffer is drawn, in logical pixels.
    ///
    /// Where the compositor draws the material, as tall as the card ever
    /// gets: the material clips it, and springing to a new height uncovers
    /// what is already drawn rather than waiting for a new buffer.
    pub fn buffer_height(&self, card_height: f32) -> f32 {
        if self.style.is_some() {
            self.max_card_height().max(card_height)
        } else {
            card_height
        }
    }

    /// Whether the overlay has been sized, so the card can be drawn.
    pub fn configured(&self) -> bool {
        self.output != (0, 0)
    }

    /// The card is about to be drawn at `size`, in logical pixels. Once it
    /// is on screen, its material springs to the new size, unless `follow`
    /// says the size is already moving frame by frame.
    pub fn set_card_size(&mut self, size: (i32, i32), follow: bool) {
        if self.card_size == size {
            return;
        }
        let shown = self.card_size != (0, 0);
        self.card_size = size;
        match self.styles.as_ref().filter(|_| shown && !follow) {
            Some(styles) => {
                let timing = styles.create_timing_function(&self.qh, ());
                timing.set_spring(RESIZE_BOUNCE, 0.0);
                let transaction = styles.begin_transaction(&self.qh, ());
                transaction.set_duration(RESIZE_SECONDS);
                transaction.set_timing_function(&timing);
                self.place();
                transaction.commit();
            }
            None => self.place(),
        }
    }

    /// Track the pointer, and move the card with it while it is held.
    ///
    /// Returns whether the card moved. Otto shows a subsurface's new place
    /// only once the subsurface itself commits, so the caller redraws it.
    pub fn pointer_moved(&mut self, x: f64, y: f64) -> bool {
        self.pointer = (x, y);
        let (Some((grab_x, grab_y)), Some(_)) = (self.grab, self.card_at) else {
            return false;
        };
        // Absolute, from where the pointer took hold: adding up steps would
        // count one again for every event that arrives before the card moved.
        self.card_at = Some(((x - grab_x).round() as i32, (y - grab_y).round() as i32));
        self.place();
        true
    }

    /// Where the card's top-left corner is on the output.
    pub fn card_origin(&self) -> Option<(i32, i32)> {
        self.card_at
    }

    /// Where the pointer is on the card, if it is over it.
    pub fn pointer_on_card(&self) -> Option<(f64, f64)> {
        let (card_x, card_y) = self.card_at?;
        let (x, y) = (
            self.pointer.0 - f64::from(card_x),
            self.pointer.1 - f64::from(card_y),
        );
        let (w, h) = self.card_size;
        let inside = (0.0..f64::from(w)).contains(&x) && (0.0..f64::from(h)).contains(&y);
        inside.then_some((x, y))
    }

    /// A left press on the card takes hold of it.
    pub fn pointer_pressed(&mut self) {
        let Some((card_x, card_y)) = self.card_at else {
            return;
        };
        let (x, y) = self.pointer;
        self.grab = Some((x - f64::from(card_x), y - f64::from(card_y)));
    }

    pub fn pointer_released(&mut self) {
        self.grab = None;
    }

    /// Keep the card on the output, move it there, and take the pointer
    /// only over it.
    fn place(&mut self) {
        let (out_w, out_h) = self.output;
        let (card_w, card_h) = self.card_size;
        if out_w == 0 || card_w == 0 {
            return;
        }
        let (x, y) = self
            .card_at
            .unwrap_or((out_w - card_w - RESTING_MARGIN, BAR_HEIGHT + RESTING_MARGIN));
        let at = (
            x.clamp(0, (out_w - card_w).max(0)),
            y.clamp(0, (out_h - card_h).max(0)),
        );
        self.card_at = Some(at);
        self.subsurface.set_position(at.0, at.1);
        if let Some(style) = self.style.as_ref() {
            let s = self.scale;
            style.set_position(f64::from(at.0) * s, f64::from(at.1) * s);
            style.set_size(f64::from(card_w) * s, f64::from(card_h) * s);
        }
        let region = self.compositor.create_region(&self.qh, ());
        region.add(at.0, at.1, card_w, card_h);
        self.parent.set_input_region(Some(&region));
        region.destroy();
        // The position and the region are the parent's state.
        self.parent.commit();
    }

    pub fn destroy(self) {
        if let Some(style) = self.style {
            style.destroy();
        }
        self.subsurface.destroy();
        self.card.destroy();
        self.layer.destroy();
        self.parent.destroy();
    }
}

/// The launcher's card material: the popup frost, blurred when the desktop's
/// frosting is on, rounded, clipped and shadowed.
fn apply_material(style: &OttoSurfaceStyleV1) {
    let theme = Theme::for_scheme(otto_kit::color_scheme::current_color_scheme());
    let frosting = otto_kit::frosting::enabled();
    let floor = if frosting {
        FROSTED_MIN_ALPHA
    } else {
        UNFROSTED_MIN_ALPHA
    };
    let material = theme.material_popup;
    let channel = |c: u8| f64::from(c) / 255.0;
    style.set_background_color(
        channel(material.r()),
        channel(material.g()),
        channel(material.b()),
        channel(material.a().max(floor)),
    );
    style.set_blend_mode(if frosting {
        BlendMode::BackgroundBlur
    } else {
        BlendMode::Normal
    });
    // In points: the compositor scales it.
    let radius = otto_kit::corners::radius(crate::balloon::RADIUS);
    style.set_corner_radius(f64::from(radius));
    style.set_masks_to_bounds(ClipMode::Enabled);
    // The buffer is as tall as the card gets and the material shows its top:
    // stretched to the material instead, it would squash while it springs.
    style.set_contents_gravity(ContentsGravity::TopLeft);
    // Stroked by the compositor centred on the material's edge, half of it
    // clipped: doubled, the inner half is the hairline.
    let hairline = theme.hairline;
    style.set_border(
        f64::from(Theme::HAIRLINE_WIDTH * 2.0),
        channel(hairline.r()),
        channel(hairline.g()),
        channel(hairline.b()),
        channel(hairline.a()),
    );
    style.set_shadow(0.32, 32.0, 0.0, 12.0, 0.0, 0.0, 0.0);
    // Positions are the card's top-left corner.
    style.set_anchor_point(0.0, 0.0);
}
