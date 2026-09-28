//! The preview itself, drawn under the chrome exactly as Peek draws it in its
//! panel: the same toolkit renderer, the same picture edges, the same word
//! selection, the same player and the same badges.

// Rust guideline compliant 2026-02-21

use otto_kit::icons;
use otto_kit::prelude::*;

use crate::viewer::Viewer;

/// Paint the preview into the viewer's content box.
pub fn draw(canvas: &Canvas, viewer: &Viewer, theme: &Theme) {
    let session = &viewer.session;
    let content = viewer.content();

    canvas.save();
    canvas.clip_rect(content, None, true);
    if let Some(video) = &session.video {
        // The card's artwork is the poster until the first frame lands.
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
    }

    // The pan's bars, inside the clip of the picture they belong to. Nothing
    // is drawn unless there is something past the edge to scroll to.
    let (horizontal, vertical) = session.pan_bars();
    ScrollRenderer::draw(canvas, horizontal, theme, |_, _| {});
    ScrollRenderer::draw(canvas, vertical, theme, |_, _| {});
    canvas.restore();

    // The recogniser working, or finished with text to select.
    match session.recognising_phase() {
        Some(phase) => {
            otto_files::view::draw_peek_working_badge(canvas, theme, content, 1.0, phase)
        }
        None if !session.words().is_empty() => {
            otto_files::view::draw_peek_text_badge(canvas, theme, content, 1.0);
        }
        None => {}
    }
}
