//! The question a window with unsaved changes asks before it closes: save
//! them, close without them, or stay open.
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
const BUTTON_W: f32 = 96.0;
const BUTTON_H: f32 = 28.0;
const CORNER: f32 = 12.0;
/// The evdev code of the primary button.
const BTN_LEFT: u32 = 0x110;

/// What the person picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

/// The card's size, `CARD_W` wide unless the window is narrower.
fn card_size(window_w: f32) -> (f32, f32) {
    (
        CARD_W.min((window_w - 2.0 * MARGIN).max(340.0)),
        TITLE_H + BODY_H + BUTTON_H + 3.0 * MARGIN,
    )
}

/// Where the buttons are on a card `size` big, from its top-left corner.
struct Buttons {
    save: Rect,
    cancel: Rect,
    discard: Rect,
}

fn buttons((width, height): (f32, f32)) -> Buttons {
    let y = height - MARGIN - BUTTON_H;
    let save = Rect::from_xywh(width - MARGIN - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let cancel = Rect::from_xywh(save.left - 8.0 - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let discard = Rect::from_xywh(MARGIN, y, BUTTON_W, BUTTON_H);
    Buttons {
        save,
        cancel,
        discard,
    }
}

/// What a press at `at` on a card `size` big answers, if anything.
fn press(size: (f32, f32), at: Point) -> Option<Answer> {
    let buttons = buttons(size);
    if buttons.save.contains(at) {
        Some(Answer::Save)
    } else if buttons.discard.contains(at) {
        Some(Answer::Discard)
    } else if buttons.cancel.contains(at) {
        Some(Answer::Cancel)
    } else {
        None
    }
}

/// Paint the card, `size` big, asking about the file `name`; `hovered` is
/// the button the pointer is over.
fn draw(canvas: &Canvas, size: (f32, f32), name: &str, hovered: Option<Answer>, theme: &Theme) {
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
    Label::new(otto_kit::t_owned!(
        "studio-close-title",
        name = name.to_owned()
    ))
    .with_style(styles::HEADLINE)
    .with_color(theme.text_primary)
    .with_width(text_w)
    .centered_on(MARGIN, MARGIN + TITLE_H / 2.0)
    .render(canvas);
    Label::new(otto_kit::t_owned!("studio-close-body"))
        .with_style(styles::BODY)
        .with_color(theme.text_secondary)
        .with_width(text_w)
        .centered_on(MARGIN, MARGIN + TITLE_H + BODY_H / 2.0)
        .render(canvas);

    let buttons = buttons(size);
    let button = |rect: Rect, label: String, answer: Answer| {
        let primary = answer == Answer::Save;
        let over = hovered == Some(answer);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        let ground = if primary {
            theme.accent
        } else {
            theme.fill_tertiary
        };
        paint.set_color(if over {
            ground.with_a(ground.a().saturating_add(40).max(ground.a()))
        } else {
            ground
        });
        canvas.draw_rrect(RRect::new_rect_xy(rect, 7.0, 7.0), &paint);
        Label::new(label)
            .with_style(styles::BODY)
            .with_color(if primary {
                Color::WHITE
            } else {
                theme.text_primary
            })
            .centered_at(rect.center_x(), rect.center_y())
            .render(canvas);
    };
    button(
        buttons.discard,
        otto_kit::t_owned!("studio-close-discard"),
        Answer::Discard,
    );
    button(
        buttons.cancel,
        otto_kit::t_owned!("studio-signature-cancel"),
        Answer::Cancel,
    );
    button(
        buttons.save,
        otto_kit::t_owned!("studio-save"),
        Answer::Save,
    );
}

struct Inner {
    popup: Option<PopupSurface>,
    surface_id: Option<ObjectId>,
    name: String,
    theme: Theme,
    hovered: Option<Answer>,
    on_answer: Option<Rc<dyn Fn(Answer)>>,
}

/// The "save changes?" popup of one window.
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
            name: String::new(),
            theme: Theme::light(),
            hovered: None,
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
                let size = {
                    let state = pointed.borrow();
                    let Some(popup) = state.popup.as_ref() else {
                        continue;
                    };
                    let (w, h) = popup.dimensions();
                    (w as f32, h as f32)
                };
                match event.kind {
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        let hovered = press(size, at);
                        let changed = {
                            let mut state = pointed.borrow_mut();
                            std::mem::replace(&mut state.hovered, hovered) != hovered
                        };
                        if changed {
                            Self::render(&pointed);
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        pointed.borrow_mut().hovered = None;
                        Self::render(&pointed);
                    }
                    PointerEventKind::Press {
                        button: BTN_LEFT, ..
                    } => {
                        if let Some(answer) = press(size, at) {
                            Self::answer(&pointed, answer);
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

    /// The popup's surface, which has the keyboard while it is up.
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
        state.hovered = None;
    }

    /// Ask about the file `name` over the middle of a window `window_size`
    /// big, parented to `parent`. `on_answer` gets what was picked; a press
    /// outside or Escape is [`Answer::Cancel`].
    pub fn open(
        &self,
        parent: &xdg_surface::XdgSurface,
        window_size: (f32, f32),
        name: &str,
        theme: Theme,
        serial: u32,
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
        let Ok(popup) =
            PopupSurface::new_with_grab(parent, &positioner, w as i32, h as i32, Some(serial))
        else {
            return;
        };
        let surface_id = popup.wl_surface().id();
        {
            let mut state = self.inner.borrow_mut();
            state.name = name.to_owned();
            state.theme = theme;
            state.hovered = None;
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
            state.hovered = None;
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
                &state.name,
                state.hovered,
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
        let wide = buttons(size);
        let middle = |rect: Rect| Point::new(rect.center_x(), rect.center_y());
        assert_eq!(press(size, middle(wide.save)), Some(Answer::Save));
        assert_eq!(press(size, middle(wide.discard)), Some(Answer::Discard));
        assert_eq!(press(size, middle(wide.cancel)), Some(Answer::Cancel));
        assert_eq!(press(size, Point::new(size.0 / 2.0, 30.0)), None);
        // The buttons fit a narrow window's card without overlapping.
        let narrow = buttons(card_size(300.0));
        assert!(narrow.discard.right < narrow.cancel.left);
    }
}
