//! A second seat for automation, with a cursor of its own colour.
//!
//! Agents drive Otto through `zwlr_virtual_pointer_v1` and
//! `zwp_virtual_keyboard_v1`. Bound to the user's seat, every synthesized move
//! drags the user's cursor along and every synthesized click takes their
//! keyboard focus. With `[agent_cursor] enabled = true` Otto advertises a
//! second `wl_seat` (named [`AGENT_SEAT_NAME`]); a virtual device created on
//! that seat drives its own pointer and keyboard instead, and its pointer is
//! drawn as the default arrow tinted in the configured colour, beside the
//! user's.
//!
//! The seat is advertised after the user's, so clients that take the first
//! `wl_seat` (toolkits picking their default seat, `otto-rdp`) keep the user's,
//! and tools that take the last one (`wlrctl`) land on the agent's.
//!
//! What the agent seat does not do, on purpose:
//! - raise windows or activate them — the user's window stays focused;
//! - hover or click Otto's own lay-rs chrome (dock, exposé): the scene engine
//!   has a single pointer, and it belongs to the user;
//! - reach XWayland windows, which only ever see the first seat.

use std::{cell::RefCell, collections::HashMap};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            ImportMem, Renderer,
        },
    },
    input::{pointer::PointerHandle, Seat},
    utils::{Logical, Physical, Point, Transform},
};

use crate::{cursor::CursorManager, state::Backend, state::Otto};

/// The name the agent's `wl_seat` advertises.
pub const AGENT_SEAT_NAME: &str = "agent";

/// The colour used when the configured one does not parse.
const FALLBACK_COLOR: [u8; 3] = [0xff, 0x95, 0x00];

/// The automation seat: its own pointer and keyboard, and the cursor drawn
/// for it.
pub struct AgentSeat<B: Backend + 'static> {
    pub seat: Seat<Otto<B>>,
    pub pointer: PointerHandle<Otto<B>>,
    pub cursor: AgentCursor,
}

/// The agent pointer's picture: the theme's default arrow, recoloured.
pub struct AgentCursor {
    color: [u8; 3],
    /// Hidden until the agent first moves: before that the pointer sits at
    /// the origin, which is not somewhere the agent chose to be.
    pub visible: bool,
    /// Tinted buffers by output scale, for the arrow they were made from —
    /// a theme reload hands out a new arrow, and the cache starts over.
    cache: RefCell<(usize, HashMap<i32, TintedArrow>)>,
}

/// A tinted arrow and its hotspot, in buffer pixels.
type TintedArrow = (MemoryRenderBuffer, Point<i32, Physical>);

impl AgentCursor {
    pub fn new(color: &str) -> Self {
        let color = parse_rgb(color).unwrap_or_else(|| {
            tracing::warn!("agent_cursor.color {color:?} is not a #RRGGBB colour");
            FALLBACK_COLOR
        });
        Self {
            color,
            visible: false,
            cache: RefCell::new((0, HashMap::new())),
        }
    }

    /// The element drawing the agent cursor at `location`, in the output's
    /// logical coordinates, or `None` while it is hidden.
    pub fn render_element<R>(
        &self,
        renderer: &mut R,
        cursor_manager: &CursorManager,
        location: Point<f64, Logical>,
        output_scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if !self.visible {
            return None;
        }
        let scale = output_scale.round() as i32;
        let arrow = cursor_manager.get_default_cursor(scale);
        let arrow_key = std::rc::Rc::as_ptr(&arrow) as usize;

        let (buffer, hotspot) = {
            let mut cache = self.cache.borrow_mut();
            if cache.0 != arrow_key {
                *cache = (arrow_key, HashMap::new());
            }
            cache
                .1
                .entry(scale)
                .or_insert_with(|| {
                    let (_, image) = arrow.frame(0);
                    let buffer = MemoryRenderBuffer::from_slice(
                        &tint(&image.pixels_rgba, self.color),
                        Fourcc::Argb8888,
                        (image.width as i32, image.height as i32),
                        scale,
                        Transform::Normal,
                        None,
                    );
                    (buffer, Point::from((image.xhot as i32, image.yhot as i32)))
                })
                .clone()
        };

        let position = location.to_physical(output_scale) - hotspot.to_f64();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            position.to_i32_round::<i32>().to_f64(),
            &buffer,
            None,
            None,
            None,
            // Not `Kind::Cursor`: the hardware cursor plane holds one cursor,
            // and it is the user's.
            Kind::Unspecified,
        )
        .ok()
    }
}

/// Read a `#RRGGBB` literal.
fn parse_rgb(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Recolour an xcursor image: dark pixels take `color`, light ones stay light.
///
/// Pixels are premultiplied, in the file's byte order — B, G, R, A — which is
/// what `Fourcc::Argb8888` reads. A black arrow with a white rim (most themes)
/// becomes a coloured arrow with the same rim; a white arrow keeps its body
/// and gets a coloured rim.
pub fn tint(pixels: &[u8], color: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = color.map(|c| c as f32 / 255.0);
    let mut out = Vec::with_capacity(pixels.len());
    for px in pixels.chunks_exact(4) {
        let alpha = px[3] as f32 / 255.0;
        if alpha == 0.0 {
            out.extend_from_slice(px);
            continue;
        }
        let unpremul = |c: u8| (c as f32 / 255.0 / alpha).min(1.0);
        let (pb, pg, pr) = (unpremul(px[0]), unpremul(px[1]), unpremul(px[2]));
        let light = 0.2126 * pr + 0.7152 * pg + 0.0722 * pb;
        let mix = |c: f32| (((c * (1.0 - light) + light) * alpha) * 255.0).round() as u8;
        out.extend_from_slice(&[mix(b), mix(g), mix(r), px[3]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_rgb, tint};

    const ORANGE: [u8; 3] = [0xff, 0x80, 0x00];

    #[test]
    fn colours_parse_from_hex() {
        assert_eq!(parse_rgb("#ff8000"), Some(ORANGE));
        assert_eq!(parse_rgb("#FF8000"), Some(ORANGE));
        assert_eq!(parse_rgb("ff8000"), None);
        assert_eq!(parse_rgb("#f80"), None);
    }

    #[test]
    fn black_takes_the_colour() {
        assert_eq!(tint(&[0, 0, 0, 255], ORANGE), vec![0x00, 0x80, 0xff, 255]);
    }

    #[test]
    fn white_stays_white() {
        assert_eq!(
            tint(&[255, 255, 255, 255], ORANGE),
            vec![255, 255, 255, 255]
        );
    }

    #[test]
    fn transparent_pixels_are_untouched() {
        assert_eq!(tint(&[0, 0, 0, 0], ORANGE), vec![0, 0, 0, 0]);
    }

    #[test]
    fn translucent_black_stays_premultiplied() {
        // Half-transparent black: the colour at half strength.
        assert_eq!(tint(&[0, 0, 0, 128], ORANGE), vec![0x00, 0x40, 0x80, 128]);
    }
}
