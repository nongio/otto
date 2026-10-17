//! The preview itself, drawn under the chrome exactly as Peek draws it in its
//! panel: `otto_peek::draw`, the same one Files uses, with this window's PDF
//! form fields and marks laid over the picture.

// Rust guideline compliant 2026-02-21

use otto_kit::prelude::*;

use crate::viewer::Viewer;

/// Paint the preview into the viewer's content box.
pub fn draw(canvas: &Canvas, viewer: &Viewer, theme: &Theme) {
    let session = &viewer.session;
    let content = viewer.content();

    otto_peek::draw::content(canvas, session, content, theme, |canvas| {
        let frames = viewer.frames();
        crate::editing::draw(canvas, viewer, &frames, theme);
        if !viewer.marks_hidden {
            crate::marks::draw(canvas, &frames, &viewer.marks);
        }
    });
    // The recogniser working, or finished with text to select.
    otto_peek::draw::badge(canvas, session, theme, content, 1.0);
}
