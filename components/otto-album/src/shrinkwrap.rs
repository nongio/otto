//! Shrink-wrap: an SkSL runtime effect that makes the cover look like it is
//! still sealed in its plastic sleeve — crinkle wrinkles catching the light,
//! a broad diagonal sheen, and a bright rim where the film folds over the edge.

use skia_safe::{runtime_effect::ChildPtr, Data, Image, Matrix, RuntimeEffect, Shader};

const SKSL: &str = r#"
uniform shader cover;
uniform float2 uSize;
uniform float2 uLight;   // spotlight position, in 0..1 sleeve coordinates
uniform float  uGloss;
uniform float  uSeed;    // per-sleeve variation: creases, angle, ripple phase

float hash(float2 p) {
    return fract(sin(dot(p, float2(127.1, 311.7))) * 43758.5453);
}

float noise(float2 p) {
    float2 i = floor(p);
    float2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + float2(1.0, 0.0));
    float c = hash(i + float2(0.0, 1.0));
    float d = hash(i + float2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Height of the film: long creases where the wrap is pulled tight, plus the
// fine ripples that give plastic its shimmering micro-reflections.
float height(float2 uv) {
    // Every sleeve is wrapped by a different pair of hands: the seed shifts the
    // noise field, tilts the direction the film is pulled, and detunes the
    // ripple frequencies.
    float2 off = float2(uSeed * 13.37, uSeed * 7.71);
    // Full sweep: the film can be pulled in any direction, not just diagonally.
    float ang = fract(uSeed * 0.6180339) * 3.14159265;
    float ca = cos(ang);
    float sa = sin(ang);
    float2 r = float2(uv.x * ca + uv.y * sa, -uv.x * sa + uv.y * ca);

    // Creases: stretched noise, folded so ridges read as folds.
    float stretch = 2.9 + fract(uSeed * 0.317) * 1.4;
    float2 q = float2(r.x * stretch, r.y * 1.0) + off;
    float v = 0.0;
    float amp = 0.6;
    for (int i = 0; i < 3; i++) {
        v += amp * noise(q);
        q *= 2.1;
        amp *= 0.5;
    }
    float crease = 1.0 - abs(v - 0.55) * 2.2;
    crease = pow(clamp(crease, 0.0, 1.0), 8.0);

    // Mini waves: high-frequency ripples whose phase wanders with the creases,
    // so they bend around the folds rather than marching in straight lines.
    float ph = fract(uSeed * 0.7548) * 6.2831;
    float detune = 0.85 + fract(uSeed * 0.2236) * 0.4;
    float warp = noise(r * 4.0 + off) * 6.2 + v * 3.0 + ph;
    float ripple =
        sin(r.x * 46.0 * detune + warp) * 0.5 +
        sin(r.x * 91.0 * detune - r.y * 12.0 + warp * 1.7) * 0.28 +
        sin(r.y * 63.0 * detune + warp * 0.6) * 0.22;

    return crease * 0.50 + ripple * 0.042;
}

half4 main(float2 xy) {
    float2 uv = xy / uSize;

    // Normal from the slope of the height field.
    float e = 1.0;
    float h  = height(uv);
    float hx = height((xy + float2(e, 0.0)) / uSize) - h;
    float hy = height((xy + float2(0.0, e)) / uSize) - h;
    float3 n = normalize(float3(-hx * 85.0, -hy * 85.0, 1.0));

    // The art seen through the film: the shift is barely a pixel, the film is
    // thin and taut, not a lens.
    half4 base = cover.eval(xy + float2(n.x, n.y) * 0.4);

    // What the film reflects. Mirror the view direction about the normal and
    // look up a small environment: a lamp above-left of the sleeve, a faint
    // room gradient, and a dim fill from the opposite side.
    float3 V = float3(0.0, 0.0, 1.0);
    float3 R = reflect(-V, n);
    float2 env_p = uv + R.xy * 0.85;

    float2 lampd = (env_p - uLight) * float2(1.0, 1.35);
    float lamp = exp(-pow(length(lampd) / 0.13, 2.0)) * 1.9
               + exp(-pow(length(lampd) / 0.42, 2.0)) * 0.14;
    float fill = exp(-pow(length((env_p - float2(1.05, 0.95))) / 0.55, 2.0)) * 0.05;
    float room = (1.0 - clamp(env_p.y, 0.0, 1.0)) * 0.018;
    float env = lamp + fill + room;

    // Fresnel: the film reflects far more at grazing angles, which is what
    // makes the crease flanks and the edges light up.
    float fres = 0.05 + 0.95 * pow(1.0 - clamp(n.z, 0.0, 1.0), 2.2);

    // The lamp also pools light on the sleeve itself, falling off with distance.
    float spot = exp(-pow(length((uv - uLight) * float2(1.0, 1.2)) / 0.48, 2.0));

    // Rim: the wrap folds and bunches where it wraps around the edges.
    float2 edge = min(uv, 1.0 - uv);
    float rim = 1.0 - smoothstep(0.0, 0.035, min(edge.x, edge.y));

    float gloss = (env * (0.16 + 0.84 * fres) * (0.12 + 0.88 * spot)
                   + rim * 0.14 * (0.3 + spot)) * uGloss;

    // Plastic haze: only where the film is nearly edge-on to the eye.
    half3 film = mix(base.rgb, half3(0.80, 0.85, 0.93), half(0.004 + fres * 0.025 + rim * 0.045));
    half3 lit = film + half3(half(gloss)) * half3(1.0, 0.99, 0.97);

    return half4(clamp(lit, 0.0, 1.0), base.a);
}
"#;

/// A stable per-album seed, so one record always wraps the same way but two
/// different records do not.
pub fn seed_for(key: &str) -> f32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in key.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    (h % 100_000) as f32 / 97.0
}

/// Build a shader that renders `image` inside a `side × side` box as if it were
/// wrapped in plastic. `crop` is the source rect of the image to show.
pub fn wrapped_cover(
    image: &Image,
    crop: skia_safe::Rect,
    side: f32,
    gloss: f32,
    seed: f32,
) -> Option<Shader> {
    // Lamp above and to the left of the sleeve.
    const LIGHT: (f32, f32) = (0.66, 0.40);
    let k = side / crop.width();
    let mut local = Matrix::scale((k, k));
    local.pre_translate((-crop.left(), -crop.top()));

    let cover = image.to_shader(
        (skia_safe::TileMode::Clamp, skia_safe::TileMode::Clamp),
        skia_safe::SamplingOptions::new(
            skia_safe::FilterMode::Linear,
            skia_safe::MipmapMode::Linear,
        ),
        &local,
    )?;

    let effect = match RuntimeEffect::make_for_shader(SKSL, None) {
        Ok(effect) => effect,
        Err(err) => {
            tracing::warn!("shrink-wrap shader failed to compile: {err}");
            return None;
        }
    };

    let mut uniforms = Vec::with_capacity(24);
    uniforms.extend_from_slice(&side.to_ne_bytes());
    uniforms.extend_from_slice(&side.to_ne_bytes());
    uniforms.extend_from_slice(&LIGHT.0.to_ne_bytes());
    uniforms.extend_from_slice(&LIGHT.1.to_ne_bytes());
    uniforms.extend_from_slice(&gloss.to_ne_bytes());
    uniforms.extend_from_slice(&seed.to_ne_bytes());

    effect.make_shader(Data::new_copy(&uniforms), &[ChildPtr::Shader(cover)], None)
}

#[cfg(test)]
mod tests {
    /// Dev helper: `WRAP_OUT=/tmp/wrap.png cargo test -p otto-album wrap`
    /// renders the wrapped cover on its own, large, to judge the effect.
    #[test]
    fn wrap() {
        let Ok(out) = std::env::var("WRAP_OUT") else {
            return;
        };
        let image = crate::cover::bundled_cover().expect("bundled cover");
        let side = 600.0f32;
        let (iw, ih) = (image.width() as f32, image.height() as f32);
        let s = iw.min(ih);
        let crop = skia_safe::Rect::from_xywh((iw - s) / 2.0, (ih - s) / 2.0, s, s);
        let mut surface = skia_safe::surfaces::raster_n32_premul((600, 600)).unwrap();
        let mut paint = skia_safe::Paint::default();
        paint.set_shader(
            super::wrapped_cover(
                &image,
                crop,
                side,
                std::env::var("WRAP_GLOSS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
                std::env::var("WRAP_SEED")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| super::seed_for("Unknown Pleasures")),
            )
            .expect("shader"),
        );
        surface
            .canvas()
            .draw_rect(skia_safe::Rect::from_wh(side, side), &paint);
        let data = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .unwrap();
        std::fs::write(out, data.as_bytes()).unwrap();
    }
}
