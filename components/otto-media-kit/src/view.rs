//! The player as a view: the current frame, fitted, with the transport
//! under it.
//!
//! Canvas-pure like the rest of the drawing half. A host that wants the
//! frame somewhere else, or no transport, uses [`Frame::to_image`] and
//! [`transport`] directly.

use otto_kit::theme::Theme;
use skia_safe::{Canvas, Color, Image, Paint, Rect};

use crate::player::{Frame, Playback, Player, State};
use crate::transport::{self, TransportLayout, TransportState};

/// What the host is doing to the playback, which the drawing reflects.
#[derive(Debug, Clone, Copy, Default)]
pub struct Interaction {
    /// A scrub drag in progress, as a fraction of the duration.
    pub scrubbing: Option<f32>,
    /// The transport's presence, 0 → 1.
    pub transport_opacity: f32,
}

/// Where the picture goes: the box above the transport, and the fitted
/// rect of a frame of `size` inside it.
pub fn picture_rect(bounds: Rect, size: (u32, u32)) -> Rect {
    let stage = stage_rect(bounds);
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let scale = (stage.width() / w).min(stage.height() / h);
    let (fw, fh) = (w * scale, h * scale);
    Rect::from_xywh(
        stage.center_x() - fw / 2.0,
        stage.center_y() - fh / 2.0,
        fw,
        fh,
    )
}

/// The box the picture is fitted into: everything but the transport.
pub fn stage_rect(bounds: Rect) -> Rect {
    Rect::from_ltrb(
        bounds.left,
        bounds.top,
        bounds.right,
        (bounds.bottom - transport::HEIGHT).max(bounds.top),
    )
}

/// The transport's layout for `bounds`, for hit-testing.
pub fn transport_layout(bounds: Rect) -> TransportLayout {
    transport::layout(bounds)
}

/// Paint the player into `bounds`.
///
/// `poster` is drawn until the first frame arrives — a host passes the
/// file's own artwork or nothing, in which case the stage is simply dark.
pub fn draw(
    canvas: &Canvas,
    bounds: Rect,
    player: &Player,
    poster: Option<&Image>,
    interaction: Interaction,
    theme: &Theme,
) {
    let state = player.state();
    let frame = player.frame();
    draw_frame(
        canvas,
        bounds,
        frame.as_ref(),
        poster,
        &state,
        interaction,
        theme,
    );
}

/// [`draw`] from a snapshot rather than the player itself.
///
/// For a host that records its drawing into a picture on another thread, or
/// later: a [`Frame`] and a [`State`] are both `Send` and cheap to clone,
/// while a [`Player`] is neither.
pub fn draw_frame(
    canvas: &Canvas,
    bounds: Rect,
    frame: Option<&Frame>,
    poster: Option<&Image>,
    state: &State,
    interaction: Interaction,
    theme: &Theme,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // The stage: black behind letterboxing, the way every player does it,
    // because a picture over anything else looks cropped.
    paint.set_color(Color::from_argb(0xFF, 0x08, 0x08, 0x0A));
    canvas.draw_rect(bounds, &paint);

    let picture = frame
        .as_ref()
        .and_then(|frame| {
            frame
                .to_image()
                .map(|image| (image, (frame.width, frame.height)))
        })
        .or_else(|| {
            poster.map(|image| (image.clone(), (image.width() as u32, image.height() as u32)))
        });
    if let Some((image, size)) = picture {
        let dest = picture_rect(bounds, size);
        let sampling = skia_safe::SamplingOptions::new(
            skia_safe::FilterMode::Linear,
            skia_safe::MipmapMode::Linear,
        );
        canvas.draw_image_rect_with_sampling_options(&image, None, dest, sampling, &paint);
    }

    if let Some(reason) = failure(state) {
        use otto_kit::common::Renderable;
        let stage = stage_rect(bounds);
        otto_kit::components::label::Label::new(reason)
            .with_style(otto_kit::typography::styles::CALLOUT)
            .with_color(Color::from_argb(0xC0, 0xFF, 0xFF, 0xFF))
            .with_width(stage.width() - 40.0)
            .with_align(otto_kit::components::label::TextAlign::Center)
            .centered_on(stage.left + 20.0, stage.center_y())
            .render(canvas);
    }

    let layout = transport::layout(bounds);
    transport::draw(
        canvas,
        &layout,
        &TransportState {
            playing: state.playback == Playback::Playing,
            position: state.position(),
            duration: state.duration,
            muted: state.volume <= 0.0,
            scrubbing: interaction.scrubbing,
            opacity: interaction.transport_opacity,
        },
        theme,
    );
}

fn failure(state: &State) -> Option<String> {
    (state.playback == Playback::Failed).then(|| {
        state
            .error
            .clone()
            .unwrap_or_else(|| "this video could not be played".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_frame_is_letterboxed_and_a_tall_one_pillarboxed() {
        let bounds = Rect::from_wh(400.0, 300.0 + transport::HEIGHT);
        let wide = picture_rect(bounds, (1600, 400));
        assert!((wide.width() - 400.0).abs() < 0.01);
        assert!((wide.height() - 100.0).abs() < 0.01);
        assert!((wide.center_y() - 150.0).abs() < 0.01);

        let tall = picture_rect(bounds, (300, 600));
        assert!((tall.height() - 300.0).abs() < 0.01);
        assert!((tall.width() - 150.0).abs() < 0.01);
        assert!((tall.center_x() - 200.0).abs() < 0.01);
    }
}
