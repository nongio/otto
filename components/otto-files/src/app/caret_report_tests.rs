
use super::*;
use otto_kit::components::text_input::{KeyMods, TextInputKey};

fn field(value: &str) -> TextInput {
    TextInput::editing(
        value,
        otto_kit::components::text_input::TextInputStyle::default(),
    )
    .with_size(200.0, 24.0)
}

/// What the compositor is told is the caret's box moved to wherever the
/// field was painted — a caret reported at the field's own origin would
/// put an input method in the window's top-left corner.
#[test]
fn the_reported_caret_is_offset_to_where_the_field_was_drawn() {
    let mut input = field("Documents");
    input.on_key(TextInputKey::Home, KeyMods::default());
    let local = input.caret_rect();

    let (x, y, w, h) = caret_in_window(&input, (120.0, 96.0)).expect("a focused field reports");
    assert_eq!((x, y), (120.0 + local.left, 96.0 + local.top));
    assert_eq!((w, h), (local.width(), local.height()));
    assert!(h > 0.0, "and a caret you could stand something next to");
}

/// A field that does not hold the keyboard has no caret on screen, and
/// reporting its position would move an input method to a field the user
/// is not typing in.
#[test]
fn an_unfocused_field_reports_nothing() {
    let mut input = field("Documents");
    input.state.set_focused(false);
    assert!(caret_in_window(&input, (120.0, 96.0)).is_none());
}
