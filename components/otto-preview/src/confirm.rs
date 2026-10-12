//! The questions a window with unsaved changes asks before anything is
//! lost: on a close, save them, close without them, or stay open; on a save
//! over a file changed on disk since, or a save that failed, save a copy
//! elsewhere instead (or, for the change on disk, save over it anyway).
//!
//! A popup of its own over the middle of the window, rounded, blurred and
//! casting the compositor's shadow, as menus and the colour picker do. Built
//! like otto-kit's colour picker popup (read its module docs): the pointer
//! callback is registered once, with the window, never from inside pointer
//! dispatch, and each [`ConfirmPopup::open`] only makes and destroys the
//! popup surface.

// Rust guideline compliant 2026-02-21

use std::cell::RefCell;
use std::rc::Rc;

use otto_kit::components::label::Label;
use otto_kit::components::push_button::{self, PushButton, Role};
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode};
use otto_kit::skia::{Contains, Point, RRect};
use otto_kit::typography::styles;
use otto_kit::PopupSurface;
use smithay_client_toolkit::reexports::client::backend::ObjectId;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::{
    xdg_positioner, xdg_surface,
};
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::XdgPositioner;

/// The card's size, and the room around its edge.
const CARD_W: f32 = 380.0;
const MARGIN: f32 = 20.0;
const TITLE_H: f32 = 26.0;
const BODY_H: f32 = 36.0;
const CORNER: f32 = 12.0;
/// The evdev code of the primary button.
const BTN_LEFT: u32 = 0x110;

/// What the person picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
    SaveAs,
    Overwrite,
}

/// What the window asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// It is closing with changes not saved.
    Close,
    /// A save would replace a change made to the file on disk since it was
    /// opened or last saved.
    ChangedOnDisk,
    /// The save failed, for the reason given.
    SaveFailed(String),
}

impl Question {
    /// The answer Return gives, on the accent button.
    pub fn default_answer(&self) -> Answer {
        match self {
            Self::Close => Answer::Save,
            Self::ChangedOnDisk | Self::SaveFailed(_) => Answer::SaveAs,
        }
    }

    /// The button on the card's leading edge, away from the other two.
    fn leading(&self) -> Option<Answer> {
        match self {
            Self::Close => Some(Answer::Discard),
            Self::ChangedOnDisk => Some(Answer::Overwrite),
            Self::SaveFailed(_) => None,
        }
    }

    fn title(&self, name: &str) -> String {
        let name = name.to_owned();
        match self {
            Self::Close => otto_kit::t_owned!("studio-close-title", name = name),
            Self::ChangedOnDisk => otto_kit::t_owned!("studio-changed-title", name = name),
            Self::SaveFailed(_) => otto_kit::t_owned!("studio-save-failed-title", name = name),
        }
    }

    fn body(&self) -> String {
        match self {
            Self::Close => otto_kit::t_owned!("studio-close-body"),
            Self::ChangedOnDisk => otto_kit::t_owned!("studio-changed-body"),
            Self::SaveFailed(_) => otto_kit::t_owned!("studio-save-failed-body"),
        }
    }
}

/// The card's size, `CARD_W` wide unless the window is narrower.
fn card_size(window_w: f32) -> (f32, f32) {
    (
        CARD_W.min((window_w - 2.0 * MARGIN).max(340.0)),
        TITLE_H + BODY_H + push_button::HEIGHT + 3.0 * MARGIN,
    )
}

/// Where the buttons are on a card `size` big asking `question`, from its
/// top-left corner: the default on the trailing edge, Cancel before it,
/// and any other on the leading edge.
fn buttons(question: &Question, (width, height): (f32, f32)) -> Vec<(Answer, Rect)> {
    let h = push_button::HEIGHT;
    let y = height - MARGIN - h;
    let wide = |answer: Answer| push_button::width(&label(answer));
    let default = question.default_answer();
    let main = Rect::from_xywh(width - MARGIN - wide(default), y, wide(default), h);
    let cancel_w = wide(Answer::Cancel);
    let cancel = Rect::from_xywh(main.left - 8.0 - cancel_w, y, cancel_w, h);
    let mut buttons = vec![(default, main), (Answer::Cancel, cancel)];
    if let Some(leading) = question.leading() {
        buttons.push((leading, Rect::from_xywh(MARGIN, y, wide(leading), h)));
    }
    buttons
}

/// A button's label.
fn label(answer: Answer) -> String {
    match answer {
        Answer::Save => otto_kit::t_owned!("studio-save"),
        Answer::Discard => otto_kit::t_owned!("studio-close-discard"),
        Answer::Cancel => otto_kit::t_owned!("studio-close-cancel"),
        Answer::SaveAs => otto_kit::t_owned!("studio-save-as"),
        Answer::Overwrite => otto_kit::t_owned!("studio-changed-overwrite"),
    }
}

/// What a press at `at` on a card `size` big answers, if anything.
fn press(question: &Question, size: (f32, f32), at: Point) -> Option<Answer> {
    buttons(question, size)
        .into_iter()
        .find(|(_, rect)| rect.contains(at))
        .map(|(answer, _)| answer)
}

/// Paint the card, `size` big, asking `question` about the file `name`;
/// `pressed` is the button held down with the pointer over it.
fn draw(
    canvas: &Canvas,
    size: (f32, f32),
    question: &Question,
    name: &str,
    pressed: Option<Answer>,
    theme: &Theme,
) {
    let card = Rect::from_wh(size.0, size.1);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    canvas.clear(Color::TRANSPARENT);
    paint.set_color(theme.material_popup);
    canvas.draw_rrect(RRect::new_rect_xy(card, CORNER, CORNER), &paint);
    // The hairline menus carry, to close the shape against what is behind.
    paint.set_style(otto_kit::skia::PaintStyle::Stroke);
    paint.set_stroke_width(Theme::HAIRLINE_WIDTH);
    paint.set_color(theme.hairline());
    canvas.draw_rrect(RRect::new_rect_xy(card, CORNER, CORNER), &paint);

    let text_w = card.width() - 2.0 * MARGIN;
    Label::new(question.title(name))
        .with_style(styles::HEADLINE)
        .with_color(theme.text_primary)
        .with_width(text_w)
        .centered_on(MARGIN, MARGIN + TITLE_H / 2.0)
        .render(canvas);
    let body_top = MARGIN + TITLE_H;
    match question {
        Question::SaveFailed(why) => {
            // What to do, then why, on a line of its own.
            Label::new(question.body())
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .with_width(text_w)
                .centered_on(MARGIN, body_top + BODY_H * 0.28)
                .render(canvas);
            let why = otto_kit::typography::ellipsize(&styles::BODY.font(), why, text_w);
            Label::new(why)
                .with_style(styles::BODY)
                .with_color(theme.text_tertiary)
                .with_width(text_w)
                .centered_on(MARGIN, body_top + BODY_H * 0.78)
                .render(canvas);
        }
        Question::Close | Question::ChangedOnDisk => {
            Label::new(question.body())
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .with_width(text_w)
                .centered_on(MARGIN, body_top + BODY_H / 2.0)
                .render(canvas);
        }
    }

    let default = question.default_answer();
    for (answer, rect) in buttons(question, size) {
        let role = if answer == default {
            Role::Default
        } else {
            Role::Normal
        };
        push_button::draw(
            canvas,
            rect,
            &label(answer),
            PushButton::new(role).pressed(pressed == Some(answer)),
            theme,
        );
    }
}

struct Inner {
    popup: Option<PopupSurface>,
    surface_id: Option<ObjectId>,
    question: Question,
    name: String,
    theme: Theme,
    /// The button a press began on, and whether the pointer is still on it.
    held: Option<(Answer, bool)>,
    on_answer: Option<Rc<dyn Fn(Answer)>>,
}

/// The question popup of one window.
pub struct ConfirmPopup {
    inner: Rc<RefCell<Inner>>,
}

impl ConfirmPopup {
    /// Build it with the window: it registers a pointer callback, which
    /// can't be done from inside pointer dispatch.
    pub fn new() -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            popup: None,
            surface_id: None,
            question: Question::Close,
            name: String::new(),
            theme: Theme::light(),
            held: None,
            on_answer: None,
        }));
        let pointed = Rc::clone(&inner);
        AppContext::register_pointer_callback(move |events| {
            for event in events {
                let ours = pointed.borrow().surface_id.as_ref() == Some(&event.surface.id());
                if !ours {
                    continue;
                }
                let at = Point::new(event.position.0 as f32, event.position.1 as f32);
                let (size, question) = {
                    let state = pointed.borrow();
                    let Some(popup) = state.popup.as_ref() else {
                        continue;
                    };
                    let (w, h) = popup.dimensions();
                    ((w as f32, h as f32), state.question.clone())
                };
                // A button acts when released over the one the press began
                // on; sliding off before letting go takes it back.
                match event.kind {
                    PointerEventKind::Motion { .. } => {
                        let changed = {
                            let mut state = pointed.borrow_mut();
                            match &mut state.held {
                                Some((answer, over)) => {
                                    let now = press(&question, size, at) == Some(*answer);
                                    std::mem::replace(over, now) != now
                                }
                                None => false,
                            }
                        };
                        if changed {
                            Self::render(&pointed);
                        }
                    }
                    PointerEventKind::Press {
                        button: BTN_LEFT, ..
                    } => {
                        let held = press(&question, size, at).map(|answer| (answer, true));
                        pointed.borrow_mut().held = held;
                        Self::render(&pointed);
                    }
                    PointerEventKind::Release {
                        button: BTN_LEFT, ..
                    } => {
                        let held = pointed.borrow_mut().held.take();
                        match held {
                            Some((answer, true)) if press(&question, size, at) == Some(answer) => {
                                Self::answer(&pointed, answer);
                            }
                            _ => Self::render(&pointed),
                        }
                    }
                    _ => {}
                }
            }
        });
        Self { inner }
    }

    pub fn is_open(&self) -> bool {
        self.inner.borrow().popup.is_some()
    }

    /// What the popup asks while it is up.
    pub fn question(&self) -> Option<Question> {
        let state = self.inner.borrow();
        state.popup.as_ref().map(|_| state.question.clone())
    }

    /// The popup's surface, so the window is still found should the
    /// compositor give the popup the keyboard. Keys go to the window while
    /// the popup is up; it asks for no grab.
    pub fn surface_id(&self) -> Option<ObjectId> {
        self.inner.borrow().surface_id.clone()
    }

    /// Take the popup down without an answer.
    pub fn close(&self) {
        let mut state = self.inner.borrow_mut();
        if let Some(mut popup) = state.popup.take() {
            popup.destroy();
        }
        state.surface_id = None;
        state.on_answer = None;
        state.held = None;
    }

    /// Ask `question` about the file `name` over the middle of a window
    /// `window_size` big, parented to `parent`. `on_answer` gets what was
    /// picked.
    pub fn open(
        &self,
        parent: &xdg_surface::XdgSurface,
        window_size: (f32, f32),
        question: Question,
        name: &str,
        theme: Theme,
        on_answer: impl Fn(Answer) + 'static,
    ) {
        if self.is_open() {
            return;
        }
        let (w, h) = card_size(window_size.0);
        let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) else {
            return;
        };
        positioner.set_size(w as i32, h as i32);
        // A point in the middle of the window, the card centred on it.
        positioner.set_anchor_rect(
            (window_size.0 / 2.0) as i32,
            (window_size.1 / 2.0 - h / 2.0) as i32,
            1,
            1,
        );
        positioner.set_anchor(xdg_positioner::Anchor::Top);
        positioner.set_gravity(xdg_positioner::Gravity::Bottom);
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::SlideX
                | xdg_positioner::ConstraintAdjustment::SlideY,
        );
        // No grab: a grab needs the serial of an input the compositor takes
        // for one, and a close can come from a key or from the compositor
        // itself. The window keeps the keyboard (Return saves, Escape stays)
        // and takes a press outside the card as a cancel.
        let popup = match PopupSurface::new_with_grab(parent, &positioner, w as i32, h as i32, None)
        {
            Ok(popup) => popup,
            Err(err) => {
                tracing::warn!(%err, "could not open the question");
                return;
            }
        };
        let surface_id = popup.wl_surface().id();
        {
            let mut state = self.inner.borrow_mut();
            state.question = question;
            state.name = name.to_owned();
            state.theme = theme;
            state.held = None;
            state.on_answer = Some(Rc::new(on_answer));
            state.surface_id = Some(surface_id.clone());
            state.popup = Some(popup);
        }
        let done = Rc::clone(&self.inner);
        AppContext::register_popup_done_callback(surface_id.clone(), move || {
            Self::answer(&done, Answer::Cancel);
        });
        let configured = Rc::clone(&self.inner);
        AppContext::register_popup_configure_callback(surface_id, move |_serial| {
            {
                let mut state = configured.borrow_mut();
                let theme = state.theme.clone();
                if let Some(popup) = state.popup.as_mut() {
                    apply_surface_effects(popup, &theme);
                    popup.mark_configured();
                }
            }
            Self::render(&configured);
        });
    }

    /// Hand `answer` on and take the popup down.
    fn answer(inner: &Rc<RefCell<Inner>>, answer: Answer) {
        let on_answer = inner.borrow_mut().on_answer.take();
        {
            let mut state = inner.borrow_mut();
            if let Some(mut popup) = state.popup.take() {
                popup.destroy();
            }
            state.surface_id = None;
            state.held = None;
        }
        if let Some(on_answer) = on_answer {
            on_answer(answer);
        }
    }

    fn render(inner: &Rc<RefCell<Inner>>) {
        let state = inner.borrow();
        let Some(popup) = state.popup.as_ref() else {
            return;
        };
        let (w, h) = popup.dimensions();
        popup.draw(|canvas| {
            draw(
                canvas,
                (w as f32, h as f32),
                &state.question,
                &state.name,
                state.held.and_then(|(answer, over)| over.then_some(answer)),
                &state.theme,
            );
        });
    }
}

/// Round, shadow and blur the popup's surface, with the values a menu and
/// the colour picker use, so all three cast the same shadow.
fn apply_surface_effects(popup: &PopupSurface, theme: &Theme) {
    let Some(style) = popup.base_surface().surface_style() else {
        return;
    };
    let scale = AppContext::fractional_scale();
    let shadow = theme.shadow;
    style.set_corner_radius(CORNER as f64);
    style.set_masks_to_bounds(ClipMode::Enabled);
    style.set_shadow(
        shadow.a() as f64 / 255.0,
        16.0 * scale,
        0.0,
        4.0 * scale,
        shadow.r() as f64 / 255.0,
        shadow.g() as f64 / 255.0,
        shadow.b() as f64 / 255.0,
    );
    style.set_blend_mode(if otto_kit::frosting::enabled() {
        BlendMode::BackgroundBlur
    } else {
        BlendMode::Normal
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_button_answers_and_the_rest_of_the_card_doesnt() {
        let size = card_size(800.0);
        let middle = |rect: Rect| Point::new(rect.center_x(), rect.center_y());
        let questions = [
            Question::Close,
            Question::ChangedOnDisk,
            Question::SaveFailed("no room".into()),
        ];
        for question in &questions {
            let placed = buttons(question, size);
            assert_eq!(placed[0].0, question.default_answer());
            for (answer, rect) in &placed {
                assert_eq!(press(question, size, middle(*rect)), Some(*answer));
            }
            assert_eq!(press(question, size, Point::new(size.0 / 2.0, 30.0)), None);
            // The buttons fit a narrow window's card without overlapping.
            let narrow = buttons(question, card_size(300.0));
            for (index, (_, a)) in narrow.iter().enumerate() {
                for (_, b) in &narrow[index + 1..] {
                    assert!(a.right <= b.left || b.right <= a.left, "{question:?}");
                }
            }
        }
        assert_eq!(
            buttons(&Question::Close, size)
                .iter()
                .map(|(answer, _)| *answer)
                .collect::<Vec<_>>(),
            vec![Answer::Save, Answer::Cancel, Answer::Discard]
        );
        assert_eq!(buttons(&Question::SaveFailed(String::new()), size).len(), 2);
    }
}
