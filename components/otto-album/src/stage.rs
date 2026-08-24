//! The scene, seen from where you would actually be sitting: looking down at a
//! turntable, its far edge receding, the record turning on the platter and the
//! album sleeve lying on the deck beside it.
//!
//! Everything on the deck is drawn in one flat "deck space" and pushed through
//! a single perspective matrix, so the sleeve, the platter and the arm all sit
//! on the same plane and share one light.

use crate::motion::Motion;
use crate::track::Track;
use crate::{disc, shrinkwrap, tonearm::Arm, turntable};
use otto_kit::prelude::*;
use otto_kit::utils::extract_accent_color;
use skia_safe::{BlurStyle, ClipOp, Color4f, MaskFilter, Matrix, Paint, Point, RRect, Rect};

/// The scene is authored at a fixed size and scaled on the way out, so every
/// constant below stays in one readable coordinate system.
const SCENE_W: f32 = 780.0;
const SCENE_H: f32 = 548.0;
/// How big the widget actually sits on screen.
pub const SCALE: f32 = 0.72;

pub const W: f32 = SCENE_W * SCALE;
pub const H: f32 = SCENE_H * SCALE;

/// The deck's top surface, in deck space.
/// A square deck, sized so that its far edge — the edge the sleeve stands on —
/// comes out the same width as the sleeve once the perspective is applied.
const DECK: Rect = Rect {
    left: 334.0,
    top: 190.0,
    right: 630.0,
    bottom: 486.0,
};

const PLATTER_C: Point = Point { x: 482.0, y: 338.0 };
const PLATTER_R: f32 = 118.0;
const DISC_R: f32 = 104.0;

/// Where the deck's far edge lands on screen — the line the standing sleeve
/// meets, which is what makes the L.
/// The sleeve's foot sits a little *below* the deck's far edge, so the deck —
/// drawn after it — laps over the bottom of the sleeve and the two read as
/// touching rather than as two floating panels.
const HORIZON: f32 = 278.0;

/// The sleeve stands upright behind the deck, propped against the wall.
const SLEEVE: Rect = Rect {
    left: 369.0,
    top: HORIZON - 226.0,
    right: 369.0 + 226.0,
    bottom: HORIZON,
};
/// Leaning against the wall, so barely off square.
const SLEEVE_TILT: f32 = -1.2;

/// The lamp, in deck space: over your right shoulder, so everything throws
/// its shadow to the left, into the column the type sits in.
pub const LIGHT: (f32, f32) = (SCENE_W - 40.0, -60.0);

pub const PLAY_C: Point = Point { x: 600.0, y: 462.0 };
pub const PLAY_R: f32 = 15.0;

/// How hard the far edge recedes.
const PERSP: f32 = -0.00105;

/// Deck space → screen. The near edge of the deck stays put and everything
/// beyond it shrinks away.
fn perspective() -> Matrix {
    let (px, py) = (DECK.center_x(), DECK.bottom);
    let mut m = Matrix::translate((px, py));
    m.pre_concat(&Matrix::new_all(
        1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, PERSP, 1.0,
    ));
    m.pre_translate((-px, -py));
    m
}

/// `backdrop` paints the room behind the objects. A desktop widget leaves it
/// off so the wallpaper shows through and only the record, the deck and their
/// shadows sit on the screen.
pub fn draw_with(canvas: &Canvas, track: &Track, motion: &Motion, backdrop: bool) {
    canvas.save();
    canvas.scale((SCALE, SCALE));
    draw_scene(canvas, track, motion, backdrop);
    canvas.restore();
}

fn draw_scene(canvas: &Canvas, track: &Track, motion: &Motion, backdrop: bool) {
    if backdrop {
        room(canvas);
    } else {
        deck_shadow(canvas);
    }

    // The sleeve stands at the back; the deck lies in front of it, and the two
    // planes meet in an L at the horizon line.
    sleeve(canvas, track);

    canvas.save();
    canvas.concat(&perspective());

    turntable::deck(canvas, DECK, PLATTER_C, PLATTER_R);
    disc::draw(
        canvas,
        PLATTER_C,
        DISC_R,
        track,
        label_color(track),
        motion.angle,
        LIGHT,
    );
    Arm::beside(PLATTER_C, DISC_R).draw(canvas, PLATTER_C, DISC_R, track.progress(), motion.lift);

    turntable::play_button(canvas, PLAY_C, PLAY_R, motion.playing, motion.hovering_play);
    turntable::badge(
        canvas,
        Point::new(DECK.left + 26.0, DECK.bottom - 40.0),
        1.0,
    );
    turntable::speed_plate(canvas, Point::new(PLAY_C.x - 62.0, PLAY_C.y + 4.0), false);

    canvas.restore();

    text_shade(canvas, if backdrop { 0.75 } else { 1.0 });
    details(canvas, track, !backdrop);
}

/// Whether a pointer at `(x, y)` is over the play/pause control. The pointer
/// arrives in screen coordinates, so it is mapped back onto the deck.
pub fn play_hit(x: f32, y: f32) -> bool {
    // Screen → scene, then scene → deck.
    let (x, y) = (x / SCALE, y / SCALE);
    let local = perspective()
        .invert()
        .map(|inv| inv.map_point((x, y)))
        .unwrap_or(Point::new(x, y));
    let (dx, dy) = ((local.x - PLAY_C.x).abs(), (local.y - PLAY_C.y).abs());
    dx <= PLAY_R + 4.0 && dy <= PLAY_R + 4.0
}

/// The shadow the deck throws on the desktop. Built from the deck's actual
/// screen quad — the plinth is a trapezoid once the perspective is applied, so
/// a rectangle here reads as a slab of haze poking out from under it.
fn deck_shadow(canvas: &Canvas) {
    let m = perspective();
    let corner = |x: f32, y: f32| m.map_point((x, y));
    let (tl, tr, br, bl) = (
        corner(DECK.left, DECK.top),
        corner(DECK.right, DECK.top),
        corner(DECK.right, DECK.bottom),
        corner(DECK.left, DECK.bottom),
    );

    // Lit from the upper right, so the shadow falls down and to the left.
    let offset = |p: Point, dx: f32, dy: f32| Point::new(p.x + dx, p.y + dy);
    let mut quad = skia_safe::PathBuilder::new();
    quad.move_to(offset(tl, -10.0, 6.0));
    quad.line_to(offset(tr, -6.0, 6.0));
    quad.line_to(offset(br, -14.0, 14.0));
    quad.line_to(offset(bl, -22.0, 14.0));
    quad.close();
    let quad = quad.detach();

    let mut ambient = Paint::default();
    ambient.set_anti_alias(true);
    ambient.set_color(Color::from_argb(58, 10, 10, 12));
    ambient.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 26.0, false));
    canvas.draw_path(&quad, &ambient);

    let mut mid = Paint::default();
    mid.set_anti_alias(true);
    mid.set_color(Color::from_argb(80, 8, 8, 10));
    mid.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 10.0, false));
    canvas.draw_path(&quad, &mid);

    // Contact: a tight dark line along the near edge, where the plinth meets
    // the surface and no light gets in.
    let mut contact = Paint::default();
    contact.set_anti_alias(true);
    contact.set_color(Color::from_argb(150, 6, 6, 8));
    contact.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 4.0, false));
    canvas.draw_rrect(
        RRect::new_rect_xy(
            Rect::from_ltrb(bl.x + 8.0, bl.y - 6.0, br.x - 8.0, br.y + 5.0),
            6.0,
            6.0,
        ),
        &contact,
    );
}

/// What is behind and beside the deck: a wall falling into shadow away from
/// the lamp.
fn room(canvas: &Canvas) {
    let mut wall = Paint::default();
    wall.set_shader(skia_safe::gradient_shader::radial(
        Point::new(LIGHT.0, 0.0),
        SCENE_W * 1.15,
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_rgb(0xEE, 0xEC, 0xE9),
            Color::from_rgb(0xC8, 0xC5, 0xC1),
        ]),
        None,
        skia_safe::TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_rect(Rect::from_wh(SCENE_W, SCENE_H), &wall);

    // The wall meets the surface at the horizon: a soft dark seam, and the
    // shadow the deck throws back against it.
    let mut seam = Paint::default();
    seam.set_color(Color::from_argb(40, 40, 34, 30));
    seam.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 6.0, false));
    canvas.draw_rect(
        Rect::from_ltrb(0.0, HORIZON - 4.0, SCENE_W, HORIZON + 8.0),
        &seam,
    );
}

/// The sleeve lying on the deck: tilted a few degrees, with a shadow that
/// says it is a card object resting on a surface, not a printed panel.
fn sleeve(canvas: &Canvas, track: &Track) {
    canvas.save();
    canvas.rotate(
        SLEEVE_TILT,
        Some(Point::new(SLEEVE.center_x(), SLEEVE.center_y())),
    );

    // Cast on the wall behind, offset away from the lamp.
    let mut ambient = Paint::default();
    ambient.set_anti_alias(true);
    ambient.set_color(Color::from_argb(52, 26, 22, 18));
    ambient.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 26.0, false));
    canvas.draw_rrect(
        RRect::new_rect_xy(SLEEVE.with_offset((-22.0, 16.0)), 4.0, 4.0),
        &ambient,
    );

    let mut wall_shadow = Paint::default();
    wall_shadow.set_anti_alias(true);
    wall_shadow.set_color(Color::from_argb(86, 30, 26, 22));
    wall_shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 11.0, false));
    canvas.draw_rrect(
        RRect::new_rect_xy(SLEEVE.with_offset((-13.0, 7.0)), 3.0, 3.0),
        &wall_shadow,
    );

    // Contact shadow where the sleeve stands on the surface.
    let mut foot = Paint::default();
    foot.set_anti_alias(true);
    foot.set_color(Color::from_argb(120, 14, 12, 10));
    foot.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 12.0, false));
    canvas.draw_oval(
        Rect::from_xywh(
            SLEEVE.left - 10.0,
            SLEEVE.bottom - 12.0,
            SLEEVE.width() + 20.0,
            28.0,
        ),
        &foot,
    );
    let mut tight = Paint::default();
    tight.set_anti_alias(true);
    tight.set_color(Color::from_argb(175, 8, 7, 6));
    tight.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 3.5, false));
    canvas.draw_rect(
        Rect::from_xywh(
            SLEEVE.left + 2.0,
            SLEEVE.bottom - 3.0,
            SLEEVE.width() - 4.0,
            6.0,
        ),
        &tight,
    );

    canvas.save();
    canvas.clip_rrect(
        RRect::new_rect_xy(SLEEVE, 2.0, 2.0),
        ClipOp::Intersect,
        true,
    );
    match &track.cover {
        Some(image) => {
            let (iw, ih) = (image.width() as f32, image.height() as f32);
            let side = iw.min(ih);
            let src = Rect::from_xywh((iw - side) / 2.0, (ih - side) / 2.0, side, side);

            canvas.save();
            canvas.translate((SLEEVE.left, SLEEVE.top));
            let seed = shrinkwrap::seed_for(&track.album);
            match shrinkwrap::wrapped_cover(image, src, SLEEVE.width(), 1.0, seed) {
                Some(shader) => {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true);
                    paint.set_shader(shader);
                    canvas.draw_rect(Rect::from_wh(SLEEVE.width(), SLEEVE.height()), &paint);
                }
                None => {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true);
                    canvas.draw_image_rect(
                        image,
                        Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                        Rect::from_wh(SLEEVE.width(), SLEEVE.height()),
                        &paint,
                    );
                }
            }
            canvas.restore();
        }
        None => {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgb(0x2A, 0x2A, 0x2E));
            canvas.draw_rect(SLEEVE, &paint);
        }
    }
    canvas.restore();

    // The card edge of the sleeve, catching the lamp.
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::paint::Style::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(Color::from_argb(64, 255, 255, 255));
    canvas.draw_rrect(RRect::new_rect_xy(SLEEVE, 2.0, 2.0), &edge);

    canvas.restore();
}

/// The shade the text sits in: the sleeve's own cast shadow, continued past
/// its edge and fanning away from the lamp. It is the same event as the
/// shadows on the deck, so it belongs in the picture rather than sitting on
/// top of it as a panel.
fn text_shade(canvas: &Canvas, strength: f32) {
    // Two separate jobs, and they were being done by one shape before, which
    // is why it read as a cloud hanging in mid-air:
    //
    //  1. the shadow the sleeve throws to its left, which must start hard
    //     against the sleeve's edge and die within a sleeve-width;
    //  2. a quiet pool under the type so it stays legible on any wallpaper,
    //     which belongs to the text block, not to the object.

    // 1. Cast: densest at the edge, gone by 130px out, and no taller than the
    // sleeve that casts it.
    let mut cast = Paint::default();
    cast.set_anti_alias(true);
    cast.set_shader(skia_safe::gradient_shader::linear(
        ((SLEEVE.left + 4.0, 0.0), (SLEEVE.left - 130.0, 0.0)),
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_argb((104.0 * strength) as u8, 10, 9, 11),
            Color::from_argb(0, 10, 9, 11),
        ]),
        None,
        skia_safe::TileMode::Clamp,
        None,
        None,
    ));
    cast.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 18.0, false));
    canvas.draw_rrect(
        RRect::new_rect_xy(
            Rect::from_ltrb(
                SLEEVE.left - 130.0,
                SLEEVE.top + 26.0,
                SLEEVE.left + 4.0,
                SLEEVE.bottom - 6.0,
            ),
            34.0,
            34.0,
        ),
        &cast,
    );

    // 2. Pool: sized to the type, centred on it, and soft all round.
    let cx = SLEEVE.left - 150.0;
    let cy = SLEEVE.top + 74.0;
    canvas.save();
    canvas.translate((cx, cy));
    canvas.scale((1.5, 1.0));
    let mut pool = Paint::default();
    pool.set_anti_alias(true);
    pool.set_shader(skia_safe::gradient_shader::radial(
        Point::new(0.0, 0.0),
        120.0,
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_argb((74.0 * strength) as u8, 12, 11, 13),
            Color::from_argb((40.0 * strength) as u8, 12, 11, 13),
            Color::from_argb(0, 12, 11, 13),
        ]),
        Some(&[0.0f32, 0.5, 1.0][..]),
        skia_safe::TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_circle(Point::new(0.0, 0.0), 120.0, &pool);
    canvas.restore();
}

/// Track details, set in a column to the right of the objects and aligned to
/// the top of the album.
fn details(canvas: &Canvas, track: &Track, on_desktop: bool) {
    // The column sits to the left of the album and is right-aligned to it, so
    // the two share one edge instead of floating apart.
    let right = SLEEVE.left - 34.0;
    let top = SLEEVE.top + 26.0;

    let title_font = styles::TITLE_2_EMPHASIZED.font();
    let artist_font = styles::BODY.font();
    let album_font = styles::SUBHEADLINE.font();
    let time_font = styles::CAPTION_1.font();

    // On the desktop the type carries its own legibility with a soft shadow;
    // in the room it is ink on a lit wall.
    let (fg, sub) = if on_desktop {
        (
            Color4f::new(0.99, 0.98, 0.97, 1.0),
            Color4f::new(0.99, 0.98, 0.97, 0.70),
        )
    } else {
        (
            Color4f::new(0.11, 0.10, 0.10, 1.0),
            Color4f::new(0.11, 0.10, 0.10, 0.58),
        )
    };

    // The column runs from the widget's edge to the album, and a long title
    // has to give way rather than run off the side.
    let available = right - 14.0;

    let line = |canvas: &Canvas, text: &str, font: &Font, y: f32, color: Color4f| {
        let mut paint = Paint::new(color, None);
        paint.set_anti_alias(true);
        let text = &elide(text, font, &paint, available);
        let x = right - font.measure_str(text, Some(&paint)).0;
        if on_desktop {
            let mut shadow = Paint::new(Color4f::new(0.0, 0.0, 0.0, 0.55), None);
            shadow.set_anti_alias(true);
            shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 5.0, false));
            canvas.draw_str(text, (x + 1.0, y + 2.0), font, &shadow);
        }
        canvas.draw_str(text, (x, y), font, &paint);
    };

    line(canvas, &track.title, &title_font, top, fg);
    line(canvas, &track.artist, &artist_font, top + 30.0, fg);
    line(canvas, &track.album, &album_font, top + 54.0, sub);
    line(
        canvas,
        &format!(
            "{} / {}",
            format_time(track.position),
            format_time(track.length)
        ),
        &time_font,
        top + 86.0,
        sub,
    );
}

/// Trim `text` with an ellipsis until it fits `max_w`.
fn elide(text: &str, font: &Font, paint: &Paint, max_w: f32) -> String {
    if font.measure_str(text, Some(paint)).0 <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        candidate.push('…');
        if font.measure_str(&candidate, Some(paint)).0 > max_w {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

fn label_color(track: &Track) -> Color {
    track
        .cover
        .as_ref()
        .map(|image| {
            let c = extract_accent_color(image);
            let f = |v: u8| ((v as f32) * 0.78) as u8;
            Color::from_rgb(f(c.r()).max(40), f(c.g()).max(30), f(c.b()).max(30))
        })
        .unwrap_or(Color::from_rgb(0xC6, 0x4A, 0x1E))
}

fn format_time(micros: u64) -> String {
    let secs = micros / 1_000_000;
    format!("{}:{:02}", secs / 60, secs % 60)
}
