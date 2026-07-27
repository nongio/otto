//! The panel as a `lay-rs` scene.
//!
//! The panel is a tree of layers, not a draw call: the card, the field, the
//! caret and the chrome are each a node, positioned once and then *changed*
//! when state changes. That buys three things immediate-mode drawing could not
//! give us — the frosted card is a real [`BlendMode::BackgroundBlur`] layer
//! rather than a hand-rolled blur of the wallpaper; property changes can carry
//! a transition, so the caret slides and errors fade in; and the engine repaints
//! only what a change actually damaged.
//!
//! Layout is in logical points, absolute rather than flexed, because a login
//! panel's arrangement is fixed: an error appearing must never move the field
//! someone is typing into.
//!
//! Clients own the surface and the authentication conversation. They build the
//! tree with [`Panel::new`], size it with [`Panel::set_size`], push state in
//! with [`Panel::update`], and paint by asking their surface to render the
//! layer node the panel is parented to.

use std::sync::Arc;

use layers::prelude::*;
use layers::types::{BlendMode, Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::lottie::LottiePlayer;
use otto_kit::typography::get_font_with_fallback;
use skia_safe::{
    canvas::SrcRectConstraint, Canvas, Color, Color4f, Data, Font, FontStyle, Image, Paint,
    PaintStyle, Point, RRect, Rect, SamplingOptions,
};

use crate::{Appearance, User};

/// The Touch ID mark, as a Lottie animation. Shared with the fingerprint
/// island in otto-islands, which is where it was drawn.
static TOUCH_ID: &[u8] = include_bytes!("../assets/touch_id.json");

/// How long one pass of the Touch ID animation takes, in seconds. The asset's
/// own duration is a single draw-in; looping it at that rate reads as frantic.
const TOUCH_ID_PERIOD: f64 = 2.4;

/// Size of the mark inside the field. The box is what the *ridges* fill, not
/// the asset's canvas — `render_fit` discards the padding the export carries
/// around them — and they are drawn very nearly square.
const TOUCH_ID_H: f32 = 30.0;
const TOUCH_ID_W: f32 = 30.0;

/// The animation draws itself in from nothing, so a cycle starting at zero
/// blinks out at every loop. Starting a little way in keeps the ridges present
/// and just re-draws them.
const TOUCH_ID_START: f64 = 0.15;

/// How long the mark takes to run from wherever it had got to up to a finished
/// fingerprint, once the finger is recognised. Cutting the animation dead at
/// the moment of success is what it did before, and it read as a glitch; taking
/// the rest of the cycle at its own pace would put up to [`TOUCH_ID_PERIOD`] in
/// front of every login. Finishing early at a fixed cost is the compromise.
const TOUCH_ID_FINISH: f64 = 0.45;

/// How long the finished mark is held in [`ACCEPTED`] before the panel says it
/// has nothing left to show. Without it the result is drawn and replaced in the
/// same breath and nobody sees it.
const TOUCH_ID_HOLD: f64 = 0.4;

/// What a recognised finger turns the mark. Green rather than the accent,
/// because at this point the mark has stopped being a prompt and become an
/// answer — the same reading the fingerprint island gives it.
const ACCEPTED: Color = Color::from_argb(255, 92, 208, 132);

/// Where in the draw-in a point `cycle` of the way through a loop sits. The
/// animation draws itself in from nothing, so replaying its empty first
/// instants blinks the mark out once per loop; the loop starts past them.
fn loop_progress(cycle: f64) -> f64 {
    TOUCH_ID_START + cycle * (1.0 - TOUCH_ID_START)
}

/// Width of the card.
const PANEL_W: f32 = 380.0;
/// Height of the card. Fixed, so the field never moves between states — the
/// room below the field is where a status line appears, and reserving it is
/// what keeps an error from shifting everything above it.
const PANEL_H: f32 = 302.0;
const PANEL_RADIUS: f32 = 28.0;
const AVATAR: f32 = 96.0;
const FIELD_H: f32 = 46.0;
const FIELD_W: f32 = PANEL_W - 2.0 * 40.0;
const POWER_BUTTON: f32 = 40.0;
const SCREEN_MARGIN: f32 = 40.0;
/// How far the card sits above the vertical centre. A panel centred exactly
/// looks low, because the eye reads the clock above it as part of the group.
const PANEL_RISE: f32 = 40.0;
/// Where the caret sits when the field is empty, from the field's left edge.
const FIELD_TEXT_INSET: f32 = 20.0;
/// Spacing between the dots of a masked secret.
const DOT_SPACING: f32 = 14.0;

/// What the panel should show. Neither greetd nor PAM appears here; a client
/// translates its own conversation into this.
pub struct View<'a> {
    /// Who is logging in, if known. Before a username is entered there is no
    /// one to show, and the panel draws a generic silhouette.
    pub user: Option<&'a User>,
    /// Label above the field, as the authentication conversation phrased it.
    pub prompt: &'a str,
    pub field: Field<'a>,
    pub status: Option<Status<'a>>,
    /// Session name for the picker in the bottom-left. A lock screen has no
    /// session to choose, and passes `None`.
    pub session: Option<&'a str>,
    /// Set once the conversation is over and the panel is inert — the field is
    /// replaced by this message.
    pub busy: Option<&'a str>,
    /// Whether to offer suspend / restart / shut down.
    pub power: bool,
}

/// The input field's contents.
pub enum Field<'a> {
    /// Echoed, for a username or a `visible` PAM prompt.
    Text(&'a str),
    /// Masked, drawn as dots. The length is all the panel needs, and taking
    /// only that keeps the password itself out of the drawing code.
    Secret(usize),
}

/// A line below the field.
pub enum Status<'a> {
    Info(&'a str),
    Error(&'a str),
    /// A fingerprint reader is in play, and the mark in the field illustrates
    /// it. What the mark does is [`Finger`].
    Fingerprint(&'a str, Finger),
}

/// How far a fingerprint has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finger {
    /// Waiting for one. The mark loops for as long as this lasts.
    Awaited,
    /// Recognised. The mark finishes its draw-in, settles into the accepted
    /// colour and holds there long enough to be read — a client should let
    /// [`Panel::wants_frames`] fall before it moves on.
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Suspend,
    Restart,
    Shutdown,
}

/// Something the pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CycleSession,
    Power(PowerAction),
}

/// The panel's scene.
pub struct Panel {
    appearance: Appearance,
    engine: Arc<Engine>,

    /// Everything the panel owns hangs off this, so a client can hand over the
    /// layer node of its surface and get the whole panel inside it.
    root: Layer,
    wallpaper: Layer,
    card: Layer,
    avatar: Layer,
    name: Layer,
    prompt: Layer,
    field: Layer,
    caret: Layer,
    /// The Touch ID mark, shown inside the field while a finger is expected.
    fingerprint: Layer,
    status: Layer,
    busy: Layer,
    clock: Layer,
    session: Layer,
    power: Vec<(PowerAction, Layer)>,

    /// Cached decode of the wallpaper and the current avatar, so a state change
    /// never re-reads them from disk.
    wallpaper_image: Option<Image>,
    avatar_image: Option<(std::path::PathBuf, Option<Image>)>,

    /// Parsed once; `None` if the asset will not load, which costs the mark
    /// and nothing else. Untinted — the colour says which state the finger is
    /// in, so it is applied per draw rather than baked in.
    touch_id: Option<Arc<LottiePlayer>>,
    /// What the mark is doing, and when it started doing it. Kept so that an
    /// update which changes something else — a keystroke, an error — does not
    /// restart the animation, and so that finishing it can pick up from
    /// wherever the loop had got to.
    finger: Option<Finger>,
    mark_started: Option<std::time::Instant>,
    /// When an accepted mark has been on show long enough. `None` unless the
    /// finger was accepted.
    mark_settles_at: Option<std::time::Instant>,

    size: (f32, f32),
    /// Where the clickable chrome ended up, in the same space as pointer
    /// events. Kept beside the layers because hit testing wants rectangles.
    session_hitbox: Option<Rect>,
    power_hitboxes: Vec<(PowerAction, Rect)>,
}

impl Panel {
    /// Build the scene under `parent`, which is normally the layer node of the
    /// client's surface. The panel is invisible until [`Panel::set_size`] has
    /// been given the surface's dimensions.
    pub fn new(appearance: Appearance, engine: Arc<Engine>, parent: Option<&Layer>) -> Self {
        let new_layer = |key: &str| {
            let layer = engine.new_layer();
            layer.set_key(key);
            layer.set_layout_style(taffy::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            layer
        };

        let root = new_layer("auth-panel");
        match parent {
            Some(parent) => {
                let _ = parent.add_sublayer(&root);
            }
            None => {
                let _ = engine.add_layer(&root);
            }
        }

        let wallpaper = new_layer("auth-wallpaper");
        let card = new_layer("auth-card");
        let avatar = new_layer("auth-avatar");
        let name = new_layer("auth-name");
        let prompt = new_layer("auth-prompt");
        let field = new_layer("auth-field");
        let caret = new_layer("auth-caret");
        let fingerprint = new_layer("auth-fingerprint");
        let status = new_layer("auth-status");
        let busy = new_layer("auth-busy");
        let clock = new_layer("auth-clock");
        let session = new_layer("auth-session");

        let _ = root.add_sublayer(&wallpaper);
        let _ = root.add_sublayer(&card);
        let _ = root.add_sublayer(&clock);
        let _ = root.add_sublayer(&session);
        let _ = card.add_sublayer(&avatar);
        let _ = card.add_sublayer(&name);
        let _ = card.add_sublayer(&prompt);
        let _ = card.add_sublayer(&field);
        let _ = card.add_sublayer(&status);
        let _ = card.add_sublayer(&busy);
        let _ = field.add_sublayer(&caret);
        let _ = field.add_sublayer(&fingerprint);

        let power = [
            PowerAction::Suspend,
            PowerAction::Restart,
            PowerAction::Shutdown,
        ]
        .into_iter()
        .map(|action| {
            let layer = new_layer("auth-power");
            let _ = root.add_sublayer(&layer);
            (action, layer)
        })
        .collect();

        let mut panel = Self {
            appearance,
            engine,
            root,
            wallpaper,
            card,
            avatar,
            name,
            prompt,
            field,
            caret,
            fingerprint,
            status,
            busy,
            clock,
            session,
            power,
            wallpaper_image: None,
            avatar_image: None,
            touch_id: None,
            finger: None,
            mark_started: None,
            mark_settles_at: None,
            size: (0.0, 0.0),
            session_hitbox: None,
            power_hitboxes: Vec::new(),
        };
        panel.style();
        panel
    }

    /// The layer everything hangs off, for a client that needs to reparent or
    /// hand it to a surface explicitly.
    pub fn layer(&self) -> &Layer {
        &self.root
    }

    pub fn appearance(&self) -> &Appearance {
        &self.appearance
    }

    /// Fixed appearance that never depends on state: colours, radii, the card's
    /// frost. Set once, so `update` only has to touch what actually changes.
    fn style(&mut self) {
        self.wallpaper_image = self.appearance.wallpaper.as_deref().and_then(decode_image);

        // The card is a real backdrop-blur layer. `blur_include_content` opts
        // it into blurring same-plane content — the wallpaper layer painted
        // just before it — rather than only what the compositor put behind the
        // surface, which for a fullscreen greeter is nothing.
        self.card.set_blend_mode(BlendMode::BackgroundBlur);
        self.card.set_blur_include_content(true);
        self.card.set_background_color(
            PaintColor::Solid {
                color: lay_color(Color::from_argb(
                    if self.appearance.dark { 150 } else { 120 },
                    20,
                    20,
                    28,
                )),
            },
            None,
        );
        self.card
            .set_border_corner_radius(BorderRadius::new_single(PANEL_RADIUS), None);
        self.card.set_border_width(1.0, None);
        self.card.set_border_color(
            PaintColor::Solid {
                color: lay_color(Color::from_argb(46, 255, 255, 255)),
            },
            None,
        );
        self.card
            .set_shadow_color(LayerColor::new_rgba(0.0, 0.0, 0.0, 0.35), None);
        self.card
            .set_shadow_offset(LayerPoint { x: 0.0, y: 10.0 }, None);
        self.card.set_shadow_radius(24.0, None);

        self.touch_id = LottiePlayer::from_json(TOUCH_ID)
            .map_err(|err| tracing::warn!(%err, "the Touch ID animation could not be loaded"))
            .ok()
            .map(Arc::new);

        // The mark is redrawn every frame anyway (see `animate`), so recording
        // a whole-layer picture of it as well would only be work thrown away.
        self.fingerprint.set_picture_cached(false);
        self.fingerprint
            .set_size(LayerSize::points(TOUCH_ID_W, TOUCH_ID_H), None);
        self.fingerprint.set_opacity(0.0, None);

        self.avatar
            .set_border_corner_radius(BorderRadius::new_single(AVATAR / 2.0), None);
        self.avatar.set_clip_content(true, None);

        self.field.set_background_color(
            PaintColor::Solid {
                color: lay_color(Color::from_argb(38, 255, 255, 255)),
            },
            None,
        );
        self.field
            .set_border_corner_radius(BorderRadius::new_single(FIELD_H / 2.0), None);
        self.field.set_border_width(1.0, None);
        self.field.set_border_color(
            PaintColor::Solid {
                color: lay_color(with_alpha(self.appearance.accent, 190)),
            },
            None,
        );

        self.caret.set_background_color(
            PaintColor::Solid {
                color: lay_color(with_alpha(self.appearance.accent, 235)),
            },
            None,
        );
        self.caret
            .set_border_corner_radius(BorderRadius::new_single(1.0), None);
        self.caret.set_size(LayerSize::points(2.0, 22.0), None);

        self.session.set_background_color(
            PaintColor::Solid {
                color: lay_color(Color::from_argb(40, 255, 255, 255)),
            },
            None,
        );
        self.session
            .set_border_corner_radius(BorderRadius::new_single(17.0), None);

        for (action, layer) in &self.power {
            layer.set_background_color(
                PaintColor::Solid {
                    color: lay_color(Color::from_argb(40, 255, 255, 255)),
                },
                None,
            );
            layer.set_border_corner_radius(BorderRadius::new_single(POWER_BUTTON / 2.0), None);
            layer.set_size(LayerSize::points(POWER_BUTTON, POWER_BUTTON), None);
            layer.set_draw_content(draw_power_icon(*action));
        }
    }

    /// Place everything for a surface of `width × height` logical points.
    pub fn set_size(&mut self, width: f32, height: f32) {
        if (width, height) == self.size {
            return;
        }
        self.size = (width, height);

        self.engine.scene_set_size(width, height);
        self.root.set_size(LayerSize::points(width, height), None);
        self.wallpaper
            .set_size(LayerSize::points(width, height), None);
        self.wallpaper.set_draw_content(draw_wallpaper(
            self.wallpaper_image.clone(),
            self.appearance.background,
            width,
            height,
        ));

        let card_x = (width - PANEL_W) / 2.0;
        let card_y = (height - PANEL_H) / 2.0 - PANEL_RISE;
        self.card.set_position(
            LayerPoint {
                x: card_x,
                y: card_y,
            },
            None,
        );
        self.card
            .set_size(LayerSize::points(PANEL_W, PANEL_H), None);

        // Inside the card, positions are card-relative.
        let center = PANEL_W / 2.0;
        self.avatar.set_position(
            LayerPoint {
                x: center - AVATAR / 2.0,
                y: 36.0,
            },
            None,
        );
        self.avatar
            .set_size(LayerSize::points(AVATAR, AVATAR), None);

        self.name
            .set_position(LayerPoint { x: 0.0, y: 142.0 }, None);
        self.name.set_size(LayerSize::points(PANEL_W, 30.0), None);

        self.prompt
            .set_position(LayerPoint { x: 0.0, y: 180.0 }, None);
        self.prompt.set_size(LayerSize::points(PANEL_W, 18.0), None);

        self.field.set_position(
            LayerPoint {
                x: center - FIELD_W / 2.0,
                y: 208.0,
            },
            None,
        );
        self.field
            .set_size(LayerSize::points(FIELD_W, FIELD_H), None);
        self.caret.set_position(
            LayerPoint {
                x: FIELD_TEXT_INSET,
                y: (FIELD_H - 22.0) / 2.0,
            },
            None,
        );

        // At the trailing end of the field, where a Touch ID prompt sits on
        // other systems: it reads as "or use this instead", right where the
        // password would otherwise be typed.
        self.fingerprint.set_position(
            LayerPoint {
                x: FIELD_W - TOUCH_ID_W - 14.0,
                y: (FIELD_H - TOUCH_ID_H) / 2.0,
            },
            None,
        );

        self.status
            .set_position(LayerPoint { x: 0.0, y: 266.0 }, None);
        self.status.set_size(LayerSize::points(PANEL_W, 20.0), None);

        self.busy
            .set_position(LayerPoint { x: 0.0, y: 214.0 }, None);
        self.busy.set_size(LayerSize::points(PANEL_W, 60.0), None);

        self.clock.set_position(
            LayerPoint {
                x: SCREEN_MARGIN,
                y: SCREEN_MARGIN,
            },
            None,
        );
        self.clock.set_size(LayerSize::points(360.0, 80.0), None);
        self.clock.set_draw_content(draw_clock(&self.appearance));

        // Chrome along the bottom. The hitboxes are the same rectangles, in
        // surface coordinates, because that is the space pointer events arrive
        // in.
        let session_y = height - SCREEN_MARGIN - 34.0;
        self.session.set_position(
            LayerPoint {
                x: SCREEN_MARGIN,
                y: session_y,
            },
            None,
        );

        let gap = 12.0;
        let count = self.power.len() as f32;
        let total = count * POWER_BUTTON + (count - 1.0).max(0.0) * gap;
        let power_y = height - SCREEN_MARGIN - POWER_BUTTON;
        let mut x = width - SCREEN_MARGIN - total;

        self.power_hitboxes.clear();
        for (action, layer) in &self.power {
            layer.set_position(LayerPoint { x, y: power_y }, None);
            self.power_hitboxes.push((
                *action,
                Rect::from_xywh(x, power_y, POWER_BUTTON, POWER_BUTTON),
            ));
            x += POWER_BUTTON + gap;
        }
    }

    /// Push the current state into the scene.
    pub fn update(&mut self, view: &View) {
        // `Transition` is not `Copy`, and each change consumes one, so make a
        // fresh one where it is needed rather than threading a single value.
        let fade = || Transition::ease_out_quad(0.18);

        self.update_avatar(view.user);

        let name = view
            .user
            .map(|user| user.display_name.clone())
            .unwrap_or_else(|| "Sign in".to_string());
        self.name.set_draw_content(draw_centered_text(
            name,
            self.font(22.0, FontStyle::bold()),
            Color::WHITE,
            22.0,
        ));

        // Busy replaces the field wholesale: the conversation is over and there
        // is nothing left to type into.
        let busy = view.busy.is_some();
        self.field
            .set_opacity(if busy { 0.0 } else { 1.0 }, Some(fade()));
        self.prompt
            .set_opacity(if busy { 0.0 } else { 1.0 }, Some(fade()));
        self.busy
            .set_opacity(if busy { 1.0 } else { 0.0 }, Some(fade()));

        if let Some(message) = view.busy {
            self.busy.set_draw_content(draw_busy(
                message.to_string(),
                self.font(15.0, FontStyle::normal()),
            ));
        }

        self.prompt.set_draw_content(draw_centered_text(
            view.prompt.to_string(),
            self.font(13.0, FontStyle::normal()),
            Color::from_argb(160, 255, 255, 255),
            13.0,
        ));

        self.update_field(&view.field);

        // The mark shows only while a finger is in play. A busy panel is past
        // the conversation, so it takes the mark with the rest of the field.
        let finger = match view.status {
            Some(Status::Fingerprint(_, finger)) if !busy => Some(finger),
            _ => None,
        };
        self.update_fingerprint(finger, fade());

        match &view.status {
            Some(status) => {
                let (text, color, fingerprint) = match status {
                    Status::Error(text) => (*text, Color::from_argb(255, 255, 110, 110), false),
                    Status::Info(text) => (*text, Color::from_argb(205, 255, 255, 255), false),
                    // No glyph beside the text: the animated mark in the field
                    // is the illustration, and two of them would compete.
                    Status::Fingerprint(text, _) => {
                        (*text, Color::from_argb(215, 255, 255, 255), false)
                    }
                };
                self.status.set_draw_content(draw_status(
                    text.to_string(),
                    self.font(13.0, FontStyle::normal()),
                    color,
                    fingerprint,
                ));
                self.status.set_opacity(1.0, Some(fade()));
            }
            None => {
                self.status.set_opacity(0.0, Some(fade()));
            }
        }

        match view.session {
            Some(session) => {
                let font = self.font(13.0, FontStyle::normal());
                let label = format!("{session}  ⌄");
                let width = font.measure_str(&label, None).0 + 32.0;
                self.session.set_size(LayerSize::points(width, 34.0), None);
                self.session
                    .set_draw_content(draw_session_label(label, font));
                self.session.set_opacity(1.0, Some(fade()));
                self.session_hitbox = Some(Rect::from_xywh(
                    SCREEN_MARGIN,
                    self.size.1 - SCREEN_MARGIN - 34.0,
                    width,
                    34.0,
                ));
            }
            None => {
                self.session.set_opacity(0.0, Some(fade()));
                self.session_hitbox = None;
            }
        }

        for (_, layer) in &self.power {
            layer.set_opacity(if view.power { 1.0 } else { 0.0 }, Some(fade()));
        }
    }

    fn update_avatar(&mut self, user: Option<&User>) {
        let path = user.and_then(|user| user.avatar.as_deref());

        if self
            .avatar_image
            .as_ref()
            .map(|(cached, _)| Some(cached.as_path()) != path)
            .unwrap_or(true)
        {
            self.avatar_image = path.map(|path| (path.to_path_buf(), decode_image(path)));
        }

        let image = self
            .avatar_image
            .as_ref()
            .and_then(|(_, image)| image.clone());
        let initials = user.map(User::initials).filter(|i| !i.is_empty());

        self.avatar.set_draw_content(draw_avatar(
            image,
            initials,
            self.appearance.accent,
            self.font(34.0, FontStyle::bold()),
        ));
    }

    fn update_field(&mut self, contents: &Field) {
        let font = self.font(17.0, FontStyle::normal());

        let caret_x = match contents {
            Field::Text(text) => {
                let width = font.measure_str(text, None).0;
                self.field
                    .set_draw_content(draw_field_text(text.to_string(), font));
                FIELD_TEXT_INSET + width
            }
            Field::Secret(len) => {
                // Never let the dots run past the caret's room at the right.
                let max_dots = (((FIELD_W - 2.0 * FIELD_TEXT_INSET) / DOT_SPACING) as usize).max(1);
                let shown = (*len).min(max_dots);
                self.field.set_draw_content(draw_field_dots(shown));
                FIELD_TEXT_INSET + shown as f32 * DOT_SPACING
            }
        };

        // The caret is its own layer, so it can slide to its new home instead
        // of jumping — the one bit of motion typing should have.
        self.caret.set_position(
            LayerPoint {
                x: caret_x + 2.0,
                y: (FIELD_H - 22.0) / 2.0,
            },
            Some(Transition::ease_out_quad(0.08)),
        );
    }

    /// Drive the Touch ID mark to match `finger`, without disturbing it when
    /// nothing about the finger has changed — `update` runs on every keystroke,
    /// and reinstalling the draw closure each time would restart the animation
    /// under whoever is watching it.
    fn update_fingerprint(&mut self, finger: Option<Finger>, fade: Transition) {
        if finger == self.finger {
            return;
        }
        self.finger = finger;
        self.fingerprint
            .set_opacity(if finger.is_some() { 1.0 } else { 0.0 }, Some(fade));

        let Some(player) = self.touch_id.clone() else {
            return;
        };
        let accent = self.appearance.accent;

        match finger {
            Some(Finger::Awaited) => {
                let started = std::time::Instant::now();
                self.mark_started = Some(started);
                self.mark_settles_at = None;
                self.fingerprint
                    .set_draw_content(move |canvas: &Canvas, w, h| {
                        let cycle =
                            (started.elapsed().as_secs_f64() % TOUCH_ID_PERIOD) / TOUCH_ID_PERIOD;
                        player.render_fit_with_color(
                            canvas,
                            loop_progress(cycle),
                            Rect::from_wh(w, h),
                            accent,
                        );
                        Rect::from_wh(w, h)
                    });
            }
            Some(Finger::Accepted) => {
                // Pick up from wherever the loop had got to rather than from
                // the start, so the ridges already on screen stay on screen and
                // simply complete.
                let accepted_at = std::time::Instant::now();
                let from = self
                    .mark_started
                    .map(|started| {
                        let elapsed = accepted_at.duration_since(started).as_secs_f64();
                        loop_progress((elapsed % TOUCH_ID_PERIOD) / TOUCH_ID_PERIOD)
                    })
                    .unwrap_or(TOUCH_ID_START);
                self.mark_settles_at = Some(
                    accepted_at
                        + std::time::Duration::from_secs_f64(TOUCH_ID_FINISH + TOUCH_ID_HOLD),
                );
                self.fingerprint
                    .set_draw_content(move |canvas: &Canvas, w, h| {
                        let elapsed = accepted_at.elapsed().as_secs_f64();
                        let progress = if elapsed >= TOUCH_ID_FINISH {
                            1.0
                        } else {
                            from + (1.0 - from) * (elapsed / TOUCH_ID_FINISH)
                        };
                        player.render_fit_with_color(
                            canvas,
                            progress,
                            Rect::from_wh(w, h),
                            ACCEPTED,
                        );
                        Rect::from_wh(w, h)
                    });
            }
            None => {
                self.mark_started = None;
                self.mark_settles_at = None;
            }
        }
    }

    /// Whether the panel still has something to show that only more frames can
    /// show: the Touch ID mark looping while a finger is awaited, or finishing
    /// and holding its result. Transitions settle on their own and are not
    /// counted here.
    ///
    /// A client waiting to move on after a successful fingerprint should wait
    /// for this to fall, which is what gives the accepted mark its moment.
    pub fn wants_frames(&self) -> bool {
        match self.finger {
            Some(Finger::Awaited) => true,
            Some(Finger::Accepted) => self
                .mark_settles_at
                .is_some_and(|at| std::time::Instant::now() < at),
            None => false,
        }
    }

    /// Advance whatever draws itself differently on every frame. A client that
    /// paints because [`Panel::wants_frames`] said so must call this first.
    ///
    /// The engine records a layer's draw closure into a picture and replays
    /// *that* until something damages the layer, which is exactly right for
    /// content that only changes when state does — and exactly wrong for the
    /// Touch ID mark, whose closure reads the clock. Nothing about the mark's
    /// properties changes as it animates, so it has to declare the damage
    /// itself; without this the mark is painted once and then frozen.
    pub fn animate(&self) {
        if self.wants_frames() {
            self.fingerprint
                .set_damage(Rect::from_wh(TOUCH_ID_W, TOUCH_ID_H));
        }
    }

    /// Which control, if any, is under a pointer at surface coordinates.
    pub fn action_at(&self, x: f32, y: f32) -> Option<Action> {
        let point = Point::new(x, y);
        if self
            .session_hitbox
            .is_some_and(|rect| skia_safe::Contains::contains(&rect, point))
        {
            return Some(Action::CycleSession);
        }
        self.power_hitboxes
            .iter()
            .find(|(_, rect)| skia_safe::Contains::contains(rect, point))
            .map(|(action, _)| Action::Power(*action))
    }

    fn font(&self, size: f32, style: FontStyle) -> Font {
        get_font_with_fallback(&self.appearance.font_family, style, size)
    }
}

// ---------------------------------------------------------------------------
// Content draw functions
//
// Each returns a closure the engine calls with the layer's own size, in the
// layer's own coordinate space. They are built from owned values so the engine
// can hold them past this call.
// ---------------------------------------------------------------------------

fn draw_wallpaper(
    image: Option<Image>,
    background: Color,
    width: f32,
    height: f32,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync + 'static {
    move |canvas, _w, _h| {
        canvas.clear(background);

        match &image {
            Some(image) => {
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    Some((
                        &cover_source(image.width() as f32, image.height() as f32, width, height),
                        SrcRectConstraint::Fast,
                    )),
                    Rect::from_wh(width, height),
                    SamplingOptions::default(),
                    &Paint::default(),
                );
                // Wallpapers are not chosen for text contrast, so darken the
                // whole thing slightly; the card's frost does the rest.
                let mut scrim = Paint::default();
                scrim.set_color(Color::from_argb(80, 0, 0, 0));
                canvas.draw_rect(Rect::from_wh(width, height), &scrim);
            }
            None => {
                // No wallpaper: a vertical wash from the configured background
                // colour, so the card still has something to sit against.
                let mut paint = Paint::default();
                paint.set_shader(skia_safe::gradient_shader::linear(
                    (Point::new(0.0, 0.0), Point::new(0.0, height)),
                    skia_safe::gradient_shader::GradientShaderColors::Colors(&[
                        lighten(background, 0.18),
                        darken(background, 0.45),
                    ]),
                    None,
                    skia_safe::TileMode::Clamp,
                    None,
                    None,
                ));
                canvas.draw_rect(Rect::from_wh(width, height), &paint);
            }
        }
        Rect::from_wh(width, height)
    }
}

fn draw_avatar(
    image: Option<Image>,
    initials: Option<String>,
    accent: Color,
    font: Font,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync + 'static {
    move |canvas, w, h| {
        let bounds = Rect::from_wh(w, h);

        match &image {
            Some(image) => {
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    Some((
                        &cover_source(image.width() as f32, image.height() as f32, w, h),
                        SrcRectConstraint::Fast,
                    )),
                    bounds,
                    SamplingOptions::default(),
                    &Paint::default(),
                );
            }
            None => {
                let mut paint = Paint::default();
                paint.set_shader(skia_safe::gradient_shader::linear(
                    (Point::new(0.0, 0.0), Point::new(w, h)),
                    skia_safe::gradient_shader::GradientShaderColors::Colors(&[
                        lighten(accent, 0.25),
                        darken(accent, 0.25),
                    ]),
                    None,
                    skia_safe::TileMode::Clamp,
                    None,
                    None,
                ));
                canvas.draw_rect(bounds, &paint);

                match &initials {
                    Some(initials) => {
                        let mut text = Paint::new(Color4f::from(Color::WHITE), None);
                        text.set_anti_alias(true);
                        let width = font.measure_str(initials, Some(&text)).0;
                        canvas.draw_str(
                            initials,
                            ((w - width) / 2.0, h / 2.0 + 12.0),
                            &font,
                            &text,
                        );
                    }
                    // Nobody has been named yet, so there are no initials. A
                    // silhouette says "a person, unspecified"; a "?" reads as
                    // something having gone wrong.
                    None => draw_silhouette(canvas, w, h),
                }
            }
        }

        let mut ring = Paint::default();
        ring.set_anti_alias(true);
        ring.set_style(PaintStyle::Stroke);
        ring.set_stroke_width(1.5);
        ring.set_color(Color::from_argb(70, 255, 255, 255));
        canvas.draw_circle(Point::new(w / 2.0, h / 2.0), w / 2.0 - 0.75, &ring);

        bounds
    }
}

/// A head and shoulders, drawn to the avatar's bounds and clipped by its layer.
fn draw_silhouette(canvas: &Canvas, w: f32, h: f32) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(225, 255, 255, 255));

    let unit = w / 96.0;
    canvas.draw_circle(Point::new(w / 2.0, 36.0 * unit), 15.0 * unit, &paint);
    // The shoulders run off the bottom; the layer's corner radius cuts them
    // into the usual rounded shape.
    canvas.draw_rrect(
        RRect::new_rect_xy(
            Rect::from_xywh(w / 2.0 - 27.0 * unit, 60.0 * unit, 54.0 * unit, h),
            27.0 * unit,
            27.0 * unit,
        ),
        &paint,
    );
}

fn draw_field_text(text: String, font: Font) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let mut paint = Paint::new(Color4f::from(Color::WHITE), None);
        paint.set_anti_alias(true);
        canvas.draw_str(&text, (FIELD_TEXT_INSET, h / 2.0 + 6.0), &font, &paint);
        Rect::from_wh(w, h)
    }
}

/// Dots rather than a bullet glyph: the spacing stays even whatever font is
/// installed, and nothing has to be measured.
fn draw_field_dots(count: usize) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let mut paint = Paint::new(Color4f::from(Color::WHITE), None);
        paint.set_anti_alias(true);
        for index in 0..count {
            canvas.draw_circle(
                Point::new(FIELD_TEXT_INSET + 4.0 + index as f32 * DOT_SPACING, h / 2.0),
                4.0,
                &paint,
            );
        }
        Rect::from_wh(w, h)
    }
}

fn draw_centered_text(
    text: String,
    font: Font,
    color: Color,
    baseline: f32,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let mut paint = Paint::new(Color4f::from(color), None);
        paint.set_anti_alias(true);
        let width = font.measure_str(&text, Some(&paint)).0;
        canvas.draw_str(&text, ((w - width) / 2.0, baseline), &font, &paint);
        Rect::from_wh(w, h)
    }
}

fn draw_busy(message: String, font: Font) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let mut paint = Paint::new(Color4f::from(Color::from_argb(215, 255, 255, 255)), None);
        paint.set_anti_alias(true);
        let width = font.measure_str(&message, Some(&paint)).0;
        canvas.draw_str(&message, ((w - width) / 2.0, 22.0), &font, &paint);

        // Three dots that say "working" without needing a frame clock.
        for index in 0..3 {
            paint.set_color(Color::from_argb(90 + index as u8 * 55, 255, 255, 255));
            canvas.draw_circle(
                Point::new(w / 2.0 - 14.0 + index as f32 * 14.0, 46.0),
                3.5,
                &paint,
            );
        }
        Rect::from_wh(w, h)
    }
}

fn draw_status(
    text: String,
    font: Font,
    color: Color,
    fingerprint: bool,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let mut paint = Paint::new(Color4f::from(color), None);
        paint.set_anti_alias(true);
        let text_width = font.measure_str(&text, Some(&paint)).0;

        let icon_room = if fingerprint { 22.0 } else { 0.0 };
        let start = (w - text_width - icon_room) / 2.0;

        if fingerprint {
            draw_fingerprint(canvas, Point::new(start + 7.0, 9.0), color);
        }
        canvas.draw_str(&text, (start + icon_room, 14.0), &font, &paint);
        Rect::from_wh(w, h)
    }
}

/// A fingerprint mark: three nested arcs, shortening outwards so it reads as a
/// fingertip rather than as concentric circles.
fn draw_fingerprint(canvas: &Canvas, center: Point, color: Color) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(1.4);
    paint.set_color(color);

    for (index, radius) in [3.0_f32, 5.5, 8.0].iter().enumerate() {
        let bounds = Rect::from_xywh(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        );
        let sweep = 300.0 - index as f32 * 40.0;
        canvas.draw_arc(bounds, 120.0 + index as f32 * 20.0, sweep, false, &paint);
    }
}

fn draw_session_label(
    label: String,
    font: Font,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let mut paint = Paint::new(Color4f::from(Color::from_argb(220, 255, 255, 255)), None);
        paint.set_anti_alias(true);
        canvas.draw_str(&label, (16.0, h / 2.0 + 4.5), &font, &paint);
        Rect::from_wh(w, h)
    }
}

fn draw_clock(appearance: &Appearance) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    let time_font = get_font_with_fallback(&appearance.font_family, FontStyle::normal(), 46.0);
    let date_font = get_font_with_fallback(&appearance.font_family, FontStyle::normal(), 15.0);

    move |canvas, w, h| {
        let now = chrono::Local::now();
        let mut paint = Paint::new(Color4f::from(Color::WHITE), None);
        paint.set_anti_alias(true);
        canvas.draw_str(
            now.format("%H:%M").to_string(),
            (0.0, 44.0),
            &time_font,
            &paint,
        );

        paint.set_color(Color::from_argb(180, 255, 255, 255));
        canvas.draw_str(
            now.format("%A %-d %B").to_string(),
            (2.0, 70.0),
            &date_font,
            &paint,
        );
        Rect::from_wh(w, h)
    }
}

fn draw_power_icon(action: PowerAction) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, w, h| {
        let center = Point::new(w / 2.0, h / 2.0);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(1.6);
        paint.set_stroke_cap(skia_safe::PaintCap::Round);
        paint.set_color(Color::from_argb(235, 255, 255, 255));

        let radius = 7.5;
        let arc = Rect::from_xywh(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        );

        match action {
            // A power symbol: a ring open at the top, with a stem through it.
            PowerAction::Shutdown => {
                canvas.draw_arc(arc, -60.0, 300.0, false, &paint);
                canvas.draw_line(
                    Point::new(center.x, center.y - radius - 2.5),
                    Point::new(center.x, center.y - 1.0),
                    &paint,
                );
            }
            // A ring open at the top with a solid arrowhead on the loose end,
            // so it reads as rotation rather than as a broken circle.
            PowerAction::Restart => {
                canvas.draw_arc(arc, -70.0, 320.0, false, &paint);
                let tip = Point::new(center.x + 1.0, center.y - radius);
                let head = skia_safe::Path::polygon(
                    &[
                        tip,
                        Point::new(tip.x - 5.5, tip.y - 3.5),
                        Point::new(tip.x - 5.5, tip.y + 3.5),
                    ],
                    true,
                    None,
                    None,
                );
                paint.set_style(PaintStyle::Fill);
                canvas.draw_path(&head, &paint);
            }
            // A crescent: one disc with a second cut out of it.
            PowerAction::Suspend => {
                let moon = skia_safe::Path::circle(center, radius, None);
                let bite = skia_safe::Path::circle(
                    Point::new(center.x + 5.0, center.y - 4.5),
                    radius,
                    None,
                );
                if let Some(crescent) = moon.op(&bite, skia_safe::PathOp::Difference) {
                    paint.set_style(PaintStyle::Fill);
                    canvas.draw_path(&crescent, &paint);
                }
            }
        }
        Rect::from_wh(w, h)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn decode_image(path: &std::path::Path) -> Option<Image> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let image = Image::from_encoded(Data::new_copy(&bytes));
            if image.is_none() {
                tracing::warn!(path = %path.display(), "image could not be decoded");
            }
            image
        }
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "image could not be read");
            None
        }
    }
}

/// The source rectangle that fills `width × height` without distorting the
/// image — the largest centred crop with the destination's aspect ratio.
fn cover_source(image_w: f32, image_h: f32, width: f32, height: f32) -> Rect {
    if width <= 0.0 || height <= 0.0 || image_w <= 0.0 || image_h <= 0.0 {
        return Rect::from_wh(image_w, image_h);
    }

    let target = width / height;
    let source = image_w / image_h;

    if source > target {
        // Image is wider: keep full height, crop the sides.
        let crop_w = image_h * target;
        Rect::from_xywh((image_w - crop_w) / 2.0, 0.0, crop_w, image_h)
    } else {
        let crop_h = image_w / target;
        Rect::from_xywh(0.0, (image_h - crop_h) / 2.0, image_w, crop_h)
    }
}

fn lay_color(color: Color) -> LayerColor {
    LayerColor::new_rgba255(color.r(), color.g(), color.b(), color.a())
}

fn with_alpha(color: Color, alpha: u8) -> Color {
    Color::from_argb(alpha, color.r(), color.g(), color.b())
}

fn lighten(color: Color, amount: f32) -> Color {
    let mix = |channel: u8| (channel as f32 + (255.0 - channel as f32) * amount).round() as u8;
    Color::from_argb(color.a(), mix(color.r()), mix(color.g()), mix(color.b()))
}

fn darken(color: Color, amount: f32) -> Color {
    let mix = |channel: u8| (channel as f32 * (1.0 - amount)).round() as u8;
    Color::from_argb(color.a(), mix(color.r()), mix(color.g()), mix(color.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Panel {
        let engine = Engine::create(1440.0, 960.0);
        let mut panel = Panel::new(Appearance::default(), engine, None);
        panel.set_size(1440.0, 960.0);
        panel
    }

    #[test]
    fn cover_crops_the_long_axis() {
        // A 6016×3614 wallpaper on a 2880×1920 screen: wider than the screen,
        // so the sides are cropped and the full height is kept.
        let wide = cover_source(6016.0, 3614.0, 2880.0, 1920.0);
        assert_eq!(wide.height(), 3614.0);
        assert!(wide.width() < 6016.0);
        assert!((wide.left() - (6016.0 - wide.width()) / 2.0).abs() < 0.01);

        // A tall image on the same screen keeps its width instead.
        let tall = cover_source(1000.0, 3000.0, 2880.0, 1920.0);
        assert_eq!(tall.width(), 1000.0);
        assert!(tall.height() < 3000.0);
    }

    /// Degenerate sizes must not produce NaN rectangles — a zero-sized surface
    /// happens between a configure and the first real size.
    #[test]
    fn cover_survives_zero_sizes() {
        let source = cover_source(100.0, 100.0, 0.0, 0.0);
        assert!(source.width().is_finite() && source.height().is_finite());
    }

    /// The chrome must be clickable where it is drawn: bottom-left for the
    /// session, bottom-right for power, and nothing in between.
    #[test]
    fn hit_testing_follows_the_layout() {
        let mut panel = panel();
        let user = User {
            name: "tester".into(),
            display_name: "Tester".into(),
            avatar: None,
        };
        panel.update(&View {
            user: Some(&user),
            prompt: "Password",
            field: Field::Secret(0),
            status: None,
            session: Some("Otto"),
            busy: None,
            power: true,
        });

        assert_eq!(
            panel.action_at(SCREEN_MARGIN + 10.0, 960.0 - SCREEN_MARGIN - 20.0),
            Some(Action::CycleSession)
        );
        assert_eq!(
            panel.action_at(1440.0 - SCREEN_MARGIN - 20.0, 960.0 - SCREEN_MARGIN - 20.0),
            Some(Action::Power(PowerAction::Shutdown))
        );
        assert_eq!(
            panel.action_at(720.0, 480.0),
            None,
            "the card is not a control"
        );
    }

    /// The Touch ID mark must actually reach the screen and actually move:
    /// it has to be visible in a render of the whole scene, and two frames a
    /// beat apart have to differ. Both have failed before — the mark was drawn
    /// at a third of its box because the asset's canvas is mostly padding, and
    /// it was frozen on its first frame because the engine replays a recorded
    /// picture of a layer's content until something damages the layer.
    #[test]
    fn the_touch_id_mark_animates_while_a_finger_is_expected() {
        let mut panel = panel();
        let view = |status| View {
            user: None,
            prompt: "Password",
            field: Field::Secret(0),
            status,
            session: None,
            busy: None,
            power: false,
        };

        panel.update(&view(None));
        assert!(
            !panel.wants_frames(),
            "a still panel should not ask for frames"
        );

        // The mark's box, from the state that has no mark in it, is the
        // baseline every later frame is compared against.
        let settle = |panel: &Panel| {
            for _ in 0..40 {
                panel.engine.update(0.016);
            }
        };
        settle(&panel);
        let without_mark = mark_pixels(&panel);

        panel.update(&view(Some(Status::Fingerprint(
            "Place your finger on the reader",
            Finger::Awaited,
        ))));
        assert!(panel.wants_frames(), "the mark needs a frame clock to run");

        assert!(
            panel.touch_id.is_some(),
            "the Touch ID asset should parse; without it there is nothing to animate"
        );

        // The mark has to land at the trailing end of the field, not on top of
        // the dots — a layout call that silently fails to apply leaves it at
        // the parent's origin, which is exactly what happened once.
        settle(&panel);
        let mark = panel.fingerprint.render_bounds_transformed();
        let field = panel.field.render_bounds_transformed();
        assert!(
            mark.left() > field.left() + field.width() / 2.0,
            "the mark should sit in the field's trailing half ({mark:?} in {field:?})"
        );
        assert!(
            mark.right() <= field.right(),
            "the mark should stay inside the field ({mark:?} in {field:?})"
        );

        let first = mark_pixels(&panel);

        // The ridges are thin strokes, so they touch about a fifth of the box
        // they span. Scaling the asset's canvas instead of its artwork puts
        // them across a third of each axis — a ninth of the area, well under
        // this — which is the failure this catches.
        let covered = differing(&without_mark, &first) as f32 / (first.len() / 4) as f32;
        assert!(
            covered > 0.10,
            "the mark should fill its box, not a corner of it ({:.0}% covered)",
            covered * 100.0
        );

        std::thread::sleep(std::time::Duration::from_millis(400));
        panel.animate();
        settle(&panel);
        let second = mark_pixels(&panel);

        assert_ne!(first, second, "the mark rendered identically 400ms apart");
    }

    /// A recognised finger must be *seen*: the mark has to finish its draw-in,
    /// change colour, and only then let the panel say it is done — a client
    /// that moves on the instant greetd says yes is what cut the animation off
    /// mid-loop before.
    #[test]
    fn an_accepted_finger_finishes_before_the_panel_goes_quiet() {
        let mut panel = panel();
        let view = |status| View {
            user: None,
            prompt: "Password",
            field: Field::Secret(0),
            status,
            session: None,
            busy: None,
            power: false,
        };
        let frame = |panel: &Panel| {
            panel.animate();
            for _ in 0..4 {
                panel.engine.update(0.016);
            }
            mark_pixels(panel)
        };

        panel.update(&view(Some(Status::Fingerprint("Waiting", Finger::Awaited))));
        for _ in 0..40 {
            panel.engine.update(0.016);
        }
        let awaited = frame(&panel);

        panel.update(&view(Some(Status::Fingerprint(
            "Authenticated",
            Finger::Accepted,
        ))));
        assert!(
            panel.wants_frames(),
            "an accepted mark still has its finish to draw"
        );

        // Part-way through the finish it must differ from the looping mark —
        // both because it is further into the draw-in and because it has
        // changed colour.
        std::thread::sleep(std::time::Duration::from_secs_f64(TOUCH_ID_FINISH / 2.0));
        let finishing = frame(&panel);
        assert_ne!(
            awaited, finishing,
            "the accepted mark should look different"
        );
        assert!(
            panel.wants_frames(),
            "the finish is not over yet ({TOUCH_ID_FINISH}s)"
        );

        // Once finished it holds, so the result is on screen for long enough to
        // register rather than being drawn and replaced in the same breath.
        std::thread::sleep(std::time::Duration::from_secs_f64(
            TOUCH_ID_FINISH / 2.0 + 0.05,
        ));
        assert!(
            panel.wants_frames(),
            "the finished mark should be held, not dropped the moment it completes"
        );

        std::thread::sleep(std::time::Duration::from_secs_f64(TOUCH_ID_HOLD));
        assert!(
            !panel.wants_frames(),
            "the panel should go quiet once the result has been held"
        );
    }

    /// Updating for an unrelated reason — a keystroke, an error appearing —
    /// must not restart the mark. It did, and a mark that starts over every
    /// time a key is pressed never gets anywhere.
    #[test]
    fn typing_does_not_restart_the_mark() {
        let mut panel = panel();
        let view = |field| View {
            user: None,
            prompt: "Password",
            field,
            status: Some(Status::Fingerprint("Waiting", Finger::Awaited)),
            session: None,
            busy: None,
            power: false,
        };

        panel.update(&view(Field::Secret(0)));
        let started = panel.mark_started.expect("the mark should have begun");

        panel.update(&view(Field::Secret(1)));
        assert_eq!(
            panel.mark_started,
            Some(started),
            "the mark's clock should survive a keystroke"
        );
    }

    /// The mark's rectangle, as RGBA, cropped out of a render of the whole
    /// scene — drawing the node on its own would need its transform rebuilt by
    /// hand, and the point is what actually reaches the screen.
    fn mark_pixels(panel: &Panel) -> Vec<u8> {
        let mut scene = skia_safe::surfaces::raster_n32_premul((1440, 960)).unwrap();
        draw_scene(scene.canvas(), panel.engine.scene(), panel.root.id());

        let at = panel.fingerprint.render_bounds_transformed();
        let snapshot = scene.image_snapshot();
        let mut crop = skia_safe::surfaces::raster_n32_premul((
            at.width().ceil() as i32,
            at.height().ceil() as i32,
        ))
        .unwrap();
        crop.canvas().draw_image_rect(
            &snapshot,
            Some((&at, skia_safe::canvas::SrcRectConstraint::Fast)),
            Rect::from_wh(at.width(), at.height()),
            &Paint::default(),
        );

        let image = crop.image_snapshot();
        let pixels = image.peek_pixels().unwrap();
        pixels.bytes().unwrap().to_vec()
    }

    /// How many pixels of two same-sized RGBA buffers are not identical.
    fn differing(a: &[u8], b: &[u8]) -> usize {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
    }

    /// A lock screen passes no session, and must not leave a clickable ghost
    /// where the picker would have been.
    #[test]
    fn without_a_session_there_is_nothing_to_click() {
        let mut panel = panel();
        panel.update(&View {
            user: None,
            prompt: "Password",
            field: Field::Secret(0),
            status: None,
            session: None,
            busy: None,
            power: false,
        });

        assert_eq!(
            panel.action_at(SCREEN_MARGIN + 10.0, 960.0 - SCREEN_MARGIN - 20.0),
            None
        );
    }

    /// The caret tracks what has been typed, so that it lands after the text
    /// rather than at a fixed spot.
    #[test]
    fn the_caret_follows_the_input() {
        let mut panel = panel();
        let view = |field| View {
            user: None,
            prompt: "Password",
            field,
            status: None,
            session: None,
            busy: None,
            power: false,
        };

        // The caret's position is animated, so it only reaches its target once
        // the engine has run — a client that never ticks would see it frozen.
        let settle = |panel: &Panel| {
            for _ in 0..40 {
                panel.engine.update(0.016);
            }
        };

        panel.update(&view(Field::Secret(0)));
        settle(&panel);
        let empty = panel.caret.position().x;

        panel.update(&view(Field::Secret(4)));
        settle(&panel);
        let typed = panel.caret.position().x;

        assert!(
            typed > empty,
            "caret should move right as dots are added ({empty} -> {typed})"
        );
    }
}
