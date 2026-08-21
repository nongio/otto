//! RGB <-> HSV conversion, in both directions.
//!
//! Pulled out on its own because it is pure arithmetic with no drawing in
//! it — the easiest thing in this component to get subtly wrong (see the
//! module docs on [`super::panel`] for the saturation/value gradient this
//! feeds), and the easiest to pin down with a table of known colours.

use skia_safe::Color;

/// `color` decomposed as hue in `0.0..360.0`, saturation in `0.0..=1.0`,
/// value in `0.0..=1.0`. Alpha is dropped — the picker only ever produces
/// opaque colours.
pub fn rgb_to_hsv(color: Color) -> (f32, f32, f32) {
    let r = color.r() as f32 / 255.0;
    let g = color.g() as f32 / 255.0;
    let b = color.b() as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    let s = if max <= 0.0 { 0.0 } else { delta / max };

    let h = if delta <= 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };

    (h.rem_euclid(360.0), s.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// Inverse of [`rgb_to_hsv`]. `h` wraps to `0.0..360.0`; `s` and `v` clamp to
/// `0.0..=1.0`. Always fully opaque.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::from_argb(255, to_u8(r1), to_u8(g1), to_u8(b1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn known_primaries_to_hsv() {
        let (h, s, v) = rgb_to_hsv(Color::from_rgb(255, 0, 0));
        assert!(approx(h, 0.0, 0.5) && approx(s, 1.0, 0.01) && approx(v, 1.0, 0.01));

        let (h, s, v) = rgb_to_hsv(Color::from_rgb(0, 255, 0));
        assert!(approx(h, 120.0, 0.5) && approx(s, 1.0, 0.01) && approx(v, 1.0, 0.01));

        let (h, s, v) = rgb_to_hsv(Color::from_rgb(0, 0, 255));
        assert!(approx(h, 240.0, 0.5) && approx(s, 1.0, 0.01) && approx(v, 1.0, 0.01));
    }

    #[test]
    fn black_and_white_are_achromatic() {
        let (_, s, v) = rgb_to_hsv(Color::from_rgb(0, 0, 0));
        assert_eq!(v, 0.0);
        assert_eq!(s, 0.0);

        let (_, s, v) = rgb_to_hsv(Color::from_rgb(255, 255, 255));
        assert!(approx(s, 0.0, 0.01));
        assert!(approx(v, 1.0, 0.01));
    }

    #[test]
    fn known_hsv_to_rgb() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), Color::from_rgb(255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), Color::from_rgb(0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), Color::from_rgb(0, 0, 255));
        assert_eq!(hsv_to_rgb(0.0, 0.0, 0.0), Color::from_rgb(0, 0, 0));
        assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), Color::from_rgb(255, 255, 255));
    }

    #[test]
    fn round_trips_a_spread_of_colours() {
        for color in [
            Color::from_rgb(200, 120, 40),
            Color::from_rgb(10, 200, 190),
            Color::from_rgb(128, 128, 128),
            Color::from_rgb(1, 254, 60),
            Color::from_rgb(90, 30, 200),
        ] {
            let (h, s, v) = rgb_to_hsv(color);
            let back = hsv_to_rgb(h, s, v);
            // Round-trip through floats loses at most a rounding step per
            // channel.
            assert!((back.r() as i32 - color.r() as i32).abs() <= 1);
            assert!((back.g() as i32 - color.g() as i32).abs() <= 1);
            assert!((back.b() as i32 - color.b() as i32).abs() <= 1);
        }
    }

    #[test]
    fn hue_wraps_at_the_ends() {
        assert_eq!(hsv_to_rgb(360.0, 1.0, 1.0), hsv_to_rgb(0.0, 1.0, 1.0));
        assert_eq!(hsv_to_rgb(-60.0, 1.0, 1.0), hsv_to_rgb(300.0, 1.0, 1.0));
    }
}
