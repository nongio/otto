//! The record surface, as an SkSL runtime effect.
//!
//! A record's shine is anisotropic: the grooves are concentric ridges, so each
//! point reflects the lamp only where its groove wall happens to face it. That
//! is what produces the two opposed arcs of light sweeping across a record, and
//! it is why a photo of vinyl never looks like a flat black disc.
//!
//! The grooves themselves are radially symmetric, so spinning them changes
//! nothing — the rotation is carried by what is *not* symmetric: the pressing
//! swirl, dust and hairline scuffs, which ride in the disc's own frame.

use skia_safe::{Data, RuntimeEffect, Shader};

const SKSL: &str = r#"
uniform float2 uCenter;
uniform float2 uLight;   // lamp position, in the same pixel space
uniform float  uR;       // disc radius
uniform float  uLabelR;  // label radius
uniform float  uAngle;   // rotation, radians

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

half4 main(float2 xy) {
    float2 p = xy - uCenter;
    float d = length(p);
    if (d > uR) {
        return half4(0.0);
    }

    float2 dir = p / max(d, 0.001);
    float theta = atan(p.y, p.x);

    // Coordinates in the disc's own frame: these turn with the record.
    float ca = cos(-uAngle);
    float sa = sin(-uAngle);
    float2 q = float2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);
    float spin_theta = theta - uAngle;

    // --- groove relief -----------------------------------------------------
    // Pitch is kept a few pixels wide: any tighter and the rings alias into
    // moire instead of reading as grooves.
    float pitch = 3.2 + 1.3 * (d / uR);
    float phase = d / pitch * 6.2831853;
    float fine = 0.5 + 0.5 * cos(phase);

    // Band gaps between tracks: a few wider, shallower rings.
    float band = noise(float2(d * 0.055, 3.7));
    float gap = smoothstep(0.62, 0.70, band) * (1.0 - smoothstep(0.78, 0.88, band));

    // The lead-out and the land around the label are almost mirror smooth.
    float land = smoothstep(uLabelR, uLabelR * 1.08, d) * (1.0 - smoothstep(uR * 0.96, uR, d));

    // --- lighting ----------------------------------------------------------
    // Grooves are concentric, so the sheen is anisotropic: it collects into two
    // opposed arcs on the axis through the lamp, and the groove ripple only
    // textures them.
    float2 l2 = normalize(uLight - uCenter);
    float axis = abs(dot(dir, l2));
    float arcs = pow(axis, 3.5);
    float sheen = pow(axis, 1.4) * 0.06;

    // Distance falloff of the lamp across the disc.
    float falloff = exp(-pow(length(xy - uLight) / (uR * 2.6), 2.0));

    float texture = mix(0.55, 1.0, fine) * mix(1.0, 0.55, gap);
    float gleam = (arcs * 0.42 * texture + sheen) * land;
    float ndh = arcs;

    // --- the record's own imperfections, which turn with it ----------------
    // Pressing swirl: faint radial streaks from the press.
    float swirl = noise(float2(spin_theta * 7.0, d * 0.09)) * 0.5 + 0.5;

    // Hairline scuffs: thin arcs at a handful of angles.
    float scuff_n = noise(float2(spin_theta * 22.0, d * 0.30));
    float scuff = smoothstep(0.86, 0.995, scuff_n) * pow(ndh, 3.0) * 0.30;

    // Dust: sparse bright specks.
    float dust = step(0.9985, hash(floor(q * 1.7))) * 0.20;

    // --- composite ---------------------------------------------------------
    half3 base = half3(0.043, 0.040, 0.050) + half3(half(swirl * 0.012));
    float light = (gleam * (0.30 + 0.70 * falloff) + scuff + dust) * (0.55 + 0.45 * falloff);

    // Rim: the edge bevel catches a bright line.
    float rim = smoothstep(uR * 0.965, uR, d) * 0.26
              + smoothstep(uLabelR * 1.02, uLabelR, d) * 0.10;

    half3 col = base + half3(half(light + rim * falloff)) * half3(1.0, 0.98, 0.94);

    // Soften the outer edge so the disc is not a hard-aliased circle.
    float alpha = 1.0 - smoothstep(uR - 1.0, uR, d);
    return half4(clamp(col, 0.0, 1.0) * half(alpha), half(alpha));
}
"#;

/// Shader for a record of radius `r` centred at `center`, rotated by `angle`
/// radians, lit from `light`. All coordinates are in the canvas' pixel space.
pub fn surface(
    center: (f32, f32),
    r: f32,
    label_r: f32,
    angle: f32,
    light: (f32, f32),
) -> Option<Shader> {
    let effect = match RuntimeEffect::make_for_shader(SKSL, None) {
        Ok(effect) => effect,
        Err(err) => {
            tracing::warn!("vinyl shader failed to compile: {err}");
            return None;
        }
    };

    let mut uniforms = Vec::with_capacity(28);
    for v in [center.0, center.1, light.0, light.1, r, label_r, angle] {
        uniforms.extend_from_slice(&v.to_ne_bytes());
    }

    effect.make_shader(Data::new_copy(&uniforms), &[], None)
}
