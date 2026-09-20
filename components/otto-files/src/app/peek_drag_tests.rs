use super::typeahead_tests::browser_over;
use super::*;

use skia_safe::Point;

/// A browser with a panel up, placed where the test says, and the display it
/// may be dragged around. Both are what the render path would have written
/// back after placing the surface.
fn with_panel(panel: Rect, display: Rect) -> (Browser, impl Sized) {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    browser.select(0, 0);
    browser.begin_peek().expect("a file to preview");
    browser.peek_panel = Some(panel);
    browser.peek_placed_offset = Some((0.0, 0.0));
    browser.peek_display = Some(display);
    (browser, dir)
}

fn panel() -> Rect {
    Rect::from_xywh(100.0, 100.0, 400.0, 300.0)
}

fn display() -> Rect {
    Rect::from_ltrb(-200.0, -100.0, 1000.0, 800.0)
}

/// What the render path writes back after placing the card: where it put it,
/// and the offset it used. A drag must not need this — see
/// [`super::Browser::drag_peek_to`] — but the clamp does.
fn frame(browser: &mut Browser) {
    let offset = offset(browser);
    browser.peek_panel = Some(panel().with_offset(offset));
    browser.peek_placed_offset = Some(offset);
}

fn offset(browser: &Browser) -> (f32, f32) {
    browser.peek.as_ref().expect("a panel").offset
}

#[test]
fn the_title_strip_takes_hold_of_the_panel() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());

    assert!(
        browser.peek_grip(Point::new(strip.center_x(), strip.center_y()), panel()),
        "a press on the strip starts a drag"
    );
    assert!(browser.peek_dragging());
}

/// The content is for reading, scrolling and zooming: a press there means one
/// of those, not a move.
#[test]
fn the_content_does_not() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let content = view::peek_content_rect(panel());

    assert!(!browser.peek_grip(Point::new(content.center_x(), content.center_y()), panel()));
    assert!(!browser.peek_dragging());
}

/// The buttons sit in the strip, so the band a drag starts in has to stop
/// short of them — otherwise expanding or closing would move the card first.
#[test]
fn the_buttons_are_not_a_handle() {
    let (mut browser, _dir) = with_panel(panel(), display());
    for button in [
        view::peek_close_rect(panel()),
        view::peek_expand_rect(panel()),
    ] {
        assert!(
            !browser.peek_grip(Point::new(button.center_x(), button.center_y()), panel()),
            "a button is not a handle"
        );
    }
}

#[test]
fn a_drag_moves_the_panel_by_what_the_pointer_did() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let from = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(from, panel());

    browser.drag_peek_to(Point::new(from.x + 40.0, from.y + 25.0));

    assert_eq!(offset(&browser), (40.0, 25.0));
}

/// A pointer holding a button keeps the focus it was grabbed with, so its
/// positions go on being measured against where the panel's surface was when
/// the press landed — they do not shift as the card moves. A drag that
/// measured against where the card is *now* would move it again for every
/// frame that had gone by.
#[test]
fn a_drag_over_the_panels_own_surface_does_not_run_away() {
    let (mut browser, _dir) = with_panel(panel(), display());
    // Surface-local: the card starts at the surface's own margin, and stays
    // there as far as this drag is concerned.
    let local = Rect::from_xywh(
        peek::SURFACE_MARGIN,
        peek::SURFACE_MARGIN,
        panel().width(),
        panel().height(),
    );
    let strip = view::peek_grip_rect(local);
    let grab = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(grab, local);

    for (travelled, expected) in [(10.0, 10.0), (20.0, 20.0), (30.0, 30.0)] {
        browser.drag_peek_to(Point::new(grab.x + travelled, grab.y));
        // A frame goes by between each: the card has moved, the pointer's own
        // frame has not.
        frame(&mut browser);
        assert_eq!(offset(&browser), (expected, 0.0));
    }
}

#[test]
fn a_panel_cannot_be_dragged_off_the_display() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let from = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(from, panel());

    browser.drag_peek_to(Point::new(from.x + 5000.0, from.y + 5000.0));

    let card = panel().with_offset(offset(&browser));
    assert!(
        card.right <= display().right + 0.5,
        "not past the right edge"
    );
    assert!(
        card.top <= display().bottom - peek::TITLEBAR_H + 0.5,
        "and the title strip is still on the display"
    );
}

/// A window that shrinks, or a display answer that arrives late, can leave a
/// card that was on screen when it was put there out of reach.
#[test]
fn the_panel_is_clamped_back_onto_a_display_that_changed() {
    let (mut browser, _dir) = with_panel(panel(), display());
    browser.peek.as_mut().expect("a panel").offset = (600.0, 0.0);
    browser.peek_panel = Some(panel().with_offset((600.0, 0.0)));

    browser.peek_display = Some(Rect::from_ltrb(0.0, 0.0, 700.0, 600.0));
    browser.clamp_peek();

    let card = panel().with_offset(offset(&browser));
    assert!(card.right <= 700.5, "back inside the smaller display");
}

/// Arrow-keying to the next file keeps the panel where it was put; closing and
/// opening again starts it at rest.
#[test]
fn the_position_outlives_the_file_but_not_the_panel() {
    let (mut browser, _dir) = with_panel(panel(), display());
    browser.peek.as_mut().expect("a panel").offset = (40.0, 25.0);

    browser.begin_peek().expect("the same file again");
    assert_eq!(offset(&browser), (40.0, 25.0), "a new file, the same place");

    browser.close_peek();
    browser.begin_peek().expect("a file to preview");
    assert_eq!(offset(&browser), (0.0, 0.0), "a new panel rests");
}

/// Filling the display leaves nowhere to be dragged aside to, and coming back
/// out to a remembered corner would read as the button having moved the panel.
#[test]
fn expanding_puts_the_panel_back_in_the_middle() {
    let (mut browser, _dir) = with_panel(panel(), display());
    browser.peek.as_mut().expect("a panel").offset = (40.0, 25.0);

    browser.toggle_peek_expand();

    assert_eq!(offset(&browser), (0.0, 0.0));
}

#[test]
fn letting_go_ends_the_drag() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    browser.peek_grip(Point::new(strip.center_x(), strip.center_y()), panel());

    assert!(browser.end_peek_drag(), "it was being dragged");
    assert!(!browser.peek_dragging());
    assert!(!browser.end_peek_drag(), "and only the once");
}

/// The pointer reports far more often than the window paints, so several
/// motions arrive against one placement of the card. Each says where the card
/// should be, not how much further to move it.
#[test]
fn motions_between_two_frames_do_not_stack_up() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let from = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(from, panel());

    // Three motions, one frame: the card ends up where the *last* one asked.
    for travelled in [10.0, 20.0, 30.0] {
        browser.drag_peek_to(Point::new(from.x + travelled, from.y));
    }

    assert_eq!(offset(&browser), (30.0, 0.0), "and not 60");
}

/// The same over the panel's own surface, where the card is the surface and
/// the pointer's own origin is the thing being moved.
#[test]
fn motions_between_two_frames_do_not_stack_up_on_the_panels_surface() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let local = Rect::from_xywh(
        peek::SURFACE_MARGIN,
        peek::SURFACE_MARGIN,
        panel().width(),
        panel().height(),
    );
    let strip = view::peek_grip_rect(local);
    let grab = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(grab, local);

    for travelled in [10.0, 20.0, 30.0] {
        browser.drag_peek_to(Point::new(grab.x + travelled, grab.y));
    }

    assert_eq!(offset(&browser), (30.0, 0.0), "and not 60");
}

/// A clamp between two frames trims what the drag is asking for; it does not
/// put the card back where it was last painted.
#[test]
fn clamping_mid_drag_keeps_what_the_drag_asked_for() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let from = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(from, panel());
    browser.drag_peek_to(Point::new(from.x + 30.0, from.y));

    browser.clamp_peek();

    assert_eq!(offset(&browser), (30.0, 0.0), "the drag stands");
}

/// The strip is the panel's titlebar, and a titlebar answers a double-click
/// by filling the display — the same thing the expand button does.
#[test]
fn a_double_click_on_the_title_strip_expands_the_panel() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let at = Point::new(strip.center_x(), strip.center_y());

    browser.peek_grip(at, panel());
    browser.end_peek_drag();
    assert!(!browser.peek.as_ref().expect("a panel").expanded);

    assert!(browser.peek_grip(at, panel()), "the press is the panel's");
    assert!(
        browser.peek.as_ref().expect("a panel").expanded,
        "the second press fills the display"
    );
    assert!(!browser.peek_dragging(), "and starts no drag");

    browser.peek_grip(at, panel());
    browser.end_peek_drag();
    browser.peek_grip(at, panel());
    assert!(
        !browser.peek.as_ref().expect("a panel").expanded,
        "and a second double-click brings it back"
    );
}

/// A press on the strip long after the last one is a drag, not half of a
/// double-click.
#[test]
fn a_slow_second_press_is_a_drag() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let at = Point::new(strip.center_x(), strip.center_y());

    browser.peek_grip(at, panel());
    browser.end_peek_drag();
    browser.last_peek_title_click = Some(std::time::Instant::now() - DOUBLE_CLICK_WINDOW * 2);

    browser.peek_grip(at, panel());

    assert!(browser.peek_dragging());
    assert!(!browser.peek.as_ref().expect("a panel").expanded);
}

/// The same over the toplevel, where the pointer's frame is the window and the
/// card moves inside it: a frame between two motions must not move it twice.
#[test]
fn frames_between_motions_do_not_move_the_panel_twice() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let from = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(from, panel());

    for travelled in [10.0, 20.0, 30.0] {
        browser.drag_peek_to(Point::new(from.x + travelled, from.y));
        frame(&mut browser);
        assert_eq!(offset(&browser), (travelled, 0.0));
    }
}

/// A clamp runs between every pair of frames, so it must not feed the card's
/// own movement back into the next one either.
#[test]
fn clamping_every_frame_does_not_walk_the_panel() {
    let (mut browser, _dir) = with_panel(panel(), display());
    let strip = view::peek_grip_rect(panel());
    let from = Point::new(strip.center_x(), strip.center_y());
    browser.peek_grip(from, panel());
    browser.drag_peek_to(Point::new(from.x + 30.0, from.y));

    for _ in 0..5 {
        browser.clamp_peek();
        frame(&mut browser);
    }

    assert_eq!(offset(&browser), (30.0, 0.0), "where the pointer left it");
}
