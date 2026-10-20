//! A [`Session`]'s content, drawn the same in every host.
//!
//! Files draws it inside the Peek panel and Studio (otto-preview) under its
//! window's chrome. Both paint the same layers in the same order: the video or
//! the preview, the picture edges, the word selection, the pan's bars, then
//! the text-recognition badge. The chrome around it stays with each host.

use otto_kit::components::scroll::ScrollRenderer;
use otto_kit::icons;
use otto_kit::skia::{Canvas, Rect};
use otto_kit::theme::Theme;

use crate::session::Session;

/// Paint the session into `content`, clipped to it.
///
/// `overlay` runs over a still preview (not over a playing video), inside the
/// clip and under the pan's bars: Studio draws its PDF form fields and marks
/// there. A host with nothing to add passes `|_| {}`.
pub fn content(
    canvas: &Canvas,
    session: &Session,
    content: Rect,
    theme: &Theme,
    overlay: impl FnOnce(&Canvas),
) {
    canvas.save();
    canvas.clip_rect(content, None, true);
    if let Some(video) = &session.video {
        // A playing video takes the card's place. The card's artwork is the
        // poster until the first frame lands; with none, the stage is dark.
        let poster = video
            .poster
            .as_ref()
            .and_then(otto_kit::preview::Pixels::to_image);
        otto_media_kit::view::draw(
            canvas,
            content,
            &video.player,
            poster.as_ref(),
            otto_media_kit::view::Interaction {
                scrubbing: video.scrubbing,
                transport_opacity: 1.0,
            },
            theme,
        );
    } else {
        otto_kit::preview::draw(
            canvas,
            content,
            &session.preview,
            theme,
            session.first_row,
            session.zoom,
            &|name: &str, size: i32| {
                icons::cached_icon_chain_at(&[name], size, icons::FULL_COLOUR_SIZE)
            },
        );
        // The same hairline every picture in the browser wears, on the
        // picture's own edges — or each page's.
        otto_kit::preview::draw_picture_edges(
            canvas,
            content,
            &session.preview,
            session.first_row,
            session.zoom,
        );
        if let Some(selection) = session.selection {
            otto_kit::preview::draw_selection(
                canvas,
                content,
                &session.preview,
                theme,
                session.zoom,
                selection,
            );
        }
        overlay(canvas);
    }

    // The pan's bars, inside the same clip as the picture they belong to.
    // Nothing is drawn unless there is a zoomed picture with something under
    // the fold: a state with no content past its viewport has no thumb.
    let (horizontal, vertical) = session.pan_bars();
    ScrollRenderer::draw(canvas, horizontal, theme, |_, _| {});
    ScrollRenderer::draw(canvas, vertical, theme, |_, _| {});
    canvas.restore();
}

/// The text-recognition badge in `area`'s bottom-right corner, at `opacity`.
///
/// One badge, two things to say: the recogniser is working, or it has
/// finished and there is something to select. Draw it after [`content`] and
/// outside its clip.
pub fn badge(canvas: &Canvas, session: &Session, theme: &Theme, area: Rect, opacity: f32) {
    match session.recognising_phase() {
        Some(phase) => crate::badge::draw_working(canvas, theme, area, opacity, phase),
        None if !session.words().is_empty() => {
            crate::badge::draw_text(canvas, theme, area, opacity);
        }
        None => {}
    }
}
