//! Audio visualiser: a PipeWire level meter, the bar animation it drives and
//! the bars themselves.
//!
//! The music island listens to whatever the default output plays.

use std::sync::{Arc, Mutex};
use std::thread;

use skia_safe::{Canvas, Color, Paint, RRect, Rect};

/// Number of bars an animator tracks. Every bar style draws from these.
pub const BAR_COUNT: usize = 8;

/// The loudness of the default output, 0.0 to 1.0, updated from a capture stream
/// on its own thread.
///
/// The stream exists only while the meter is active. A connected capture
/// stream keeps its target running, so a meter left on would stop the sound
/// card from ever suspending.
pub struct LevelMeter {
    level: Arc<Mutex<f32>>,
    capture: Option<pipewire::channel::Sender<()>>,
}

impl LevelMeter {
    /// A meter, not yet listening.
    pub fn new() -> Self {
        Self {
            level: Arc::new(Mutex::new(0.0)),
            capture: None,
        }
    }

    /// Open or close the capture stream. A PipeWire failure is logged and the
    /// meter then reads silence.
    pub fn set_active(&mut self, active: bool) {
        match (active, self.capture.is_some()) {
            (true, false) => {
                let (stop_tx, stop_rx) = pipewire::channel::channel();
                let level = self.level.clone();
                thread::spawn(move || {
                    if let Err(error) = run_capture(level, stop_rx) {
                        tracing::error!(%error, "PipeWire level meter failed");
                    }
                });
                self.capture = Some(stop_tx);
            }
            (false, true) => {
                if let Some(stop) = self.capture.take() {
                    let _ = stop.send(());
                }
                if let Ok(mut level) = self.level.lock() {
                    *level = 0.0;
                }
            }
            _ => {}
        }
    }

    pub fn level(&self) -> f32 {
        self.level.lock().map(|v| *v).unwrap_or(0.0)
    }
}

impl Drop for LevelMeter {
    fn drop(&mut self) {
        self.set_active(false);
    }
}

/// Loudness of one buffer of interleaved f32 samples, from the first channel.
/// Weighted towards the peak so the bars jump on a beat, with enough RMS in it
/// that a sustained note doesn't read as silence.
pub fn buffer_level(bytes: &[u8], channels: usize) -> Option<f32> {
    const SAMPLE: usize = std::mem::size_of::<f32>();
    let frame = SAMPLE * channels.max(1);
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f32;
    let mut seen = 0usize;
    for chunk in bytes.chunks_exact(frame) {
        let val = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]).abs();
        peak = peak.max(val);
        sum_sq += val * val;
        seen += 1;
    }
    if seen == 0 {
        return None;
    }
    let rms = (sum_sq / seen as f32).sqrt();
    Some((peak * 1.35 + rms * 0.65).clamp(0.0, 1.0))
}

fn run_capture(
    shared_level: Arc<Mutex<f32>>,
    stop: pipewire::channel::Receiver<()>,
) -> Result<(), pipewire::Error> {
    use pipewire as pw;
    use pw::spa;
    use spa::param::format::{MediaSubtype, MediaType};
    use spa::param::format_utils;
    use spa::pod::Pod;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let _stop = stop.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });

    struct UserData {
        channels: u32,
        level: Arc<Mutex<f32>>,
        skip_count: u32,
    }

    let mut props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
    };
    // Capturing a sink means its monitor. With no target the session manager
    // links the default sink, and moves the stream when the default changes.
    props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");

    let stream = pw::stream::StreamBox::new(&core, "otto-islands-level-meter", props)?;

    let user_data = UserData {
        channels: 2,
        level: shared_level,
        skip_count: 0,
    };

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else { return };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut audio_info = spa::param::audio::AudioInfoRaw::default();
            if audio_info.parse(param).is_ok() {
                user_data.channels = audio_info.channels().max(1);
            }
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            // The bars redraw at ~24 fps; measuring every 6th buffer is plenty.
            user_data.skip_count += 1;
            if user_data.skip_count < 6 {
                return;
            }
            user_data.skip_count = 0;

            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                return;
            };
            let chunk = data.chunk();
            let start = chunk.offset() as usize;
            let size = chunk.size() as usize;
            let Some(samples) = data.data() else { return };
            let end = start.saturating_add(size).min(samples.len());
            if end <= start {
                return;
            }
            if let Some(level) = buffer_level(&samples[start..end], user_data.channels as usize) {
                if let Ok(mut shared) = user_data.level.lock() {
                    *shared = level;
                }
            }
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .expect("serialising a fixed format pod")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).expect("a pod we just serialised")];

    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    mainloop.run();
    Ok(())
}

/// Turns a single loudness reading into bars that each move on their own.
///
/// Every bar follows its own sine at its own frequency, scaled by the level,
/// and eases towards it, so the row looks like a spectrum without computing
/// one. The seed (a track title, say) shifts the phases, so two tracks don't
/// dance identically.
pub struct BarAnimator {
    phase: f32,
    levels: [f32; BAR_COUNT],
    offsets: [f32; BAR_COUNT],
    seed: String,
}

impl Default for BarAnimator {
    fn default() -> Self {
        Self {
            phase: 0.0,
            levels: [0.12; BAR_COUNT],
            offsets: [0.0; BAR_COUNT],
            seed: String::new(),
        }
    }
}

impl BarAnimator {
    /// Advance one frame.
    pub fn step(&mut self, level: f32, seed: &str) -> [f32; BAR_COUNT] {
        if seed != self.seed {
            self.offsets = seed_offsets(seed);
            self.seed = seed.to_string();
        }

        self.phase += 0.35;
        let envelope = (level.clamp(0.0, 1.0) * 1.4).clamp(0.0, 1.0);

        const FREQ: [f32; BAR_COUNT] = [0.6, 1.1, 0.8, 1.4, 0.5, 1.25, 0.7, 1.0];
        for ((level, freq), offset) in self.levels.iter_mut().zip(FREQ).zip(self.offsets) {
            let wave = ((self.phase * freq + offset).sin() * 0.5) + 0.5;
            let idle = 0.08 + wave * 0.07;
            let driven = envelope * (0.4 + wave * 0.5);
            let target = (idle + driven).clamp(0.0, 1.0);
            // Rise fast, fall slower, like a VU needle.
            *level += (target - *level) * if target > *level { 0.45 } else { 0.30 };
        }
        self.levels
    }

    pub fn levels(&self) -> [f32; BAR_COUNT] {
        self.levels
    }
}

fn seed_offsets(seed: &str) -> [f32; BAR_COUNT] {
    // FNV-1a, then an LCG to spread it across the bars.
    let mut hash: u32 = 2166136261;
    for b in seed.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    let mut out = [0.0f32; BAR_COUNT];
    for item in out.iter_mut() {
        let normalized = (hash & 0xFFFF) as f32 / 65535.0;
        hash = hash.wrapping_mul(1664525).wrapping_add(1013904223);
        *item = normalized * std::f32::consts::TAU;
    }
    out
}

/// How a row of bars is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarStyle {
    /// Three bars centred in a circle.
    Mini,
    /// A few thin bars growing both ways from the middle, beside a title.
    Compact(usize),
    /// Every bar, standing on the bottom edge, brighter towards the top.
    Large,
}

/// Draw `levels` into `rect` in `colour`.
pub fn draw_bars(
    canvas: &Canvas,
    rect: Rect,
    levels: &[f32; BAR_COUNT],
    colour: Color,
    style: BarStyle,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let with_alpha = |a: u8| Color::from_argb(a, colour.r(), colour.g(), colour.b());

    match style {
        BarStyle::Mini => {
            let (count, bar_w, gap) = (3, 3.0f32, 2.5f32);
            let total = count as f32 * bar_w + (count - 1) as f32 * gap;
            let start_x = rect.left + (rect.width() - total) / 2.0;
            let center_y = rect.top + rect.height() / 2.0;
            let max_h = rect.height() * 0.5;
            paint.set_color(with_alpha(220));
            for (i, level) in levels.iter().take(count).enumerate() {
                let bar_h = level.clamp(0.1, 1.0) * max_h;
                let bx = start_x + i as f32 * (bar_w + gap);
                canvas.draw_rrect(
                    RRect::new_rect_xy(
                        Rect::from_xywh(bx, center_y - bar_h / 2.0, bar_w, bar_h),
                        1.0,
                        1.0,
                    ),
                    &paint,
                );
            }
        }
        BarStyle::Compact(count) => {
            let (bar_w, gap) = (3.0f32, 2.0f32);
            let total = count as f32 * bar_w + (count as f32 - 1.0) * gap;
            let start_x = rect.left + (rect.width() - total) / 2.0;
            let center_y = rect.top + rect.height() / 2.0;
            paint.set_color(with_alpha(220));
            for i in 0..count {
                let bar_h = levels[i % BAR_COUNT].clamp(0.05, 1.0) * rect.height();
                let bx = start_x + i as f32 * (bar_w + gap);
                canvas.draw_rrect(
                    RRect::new_rect_xy(
                        Rect::from_xywh(bx, center_y - bar_h / 2.0, bar_w, bar_h),
                        1.0,
                        1.0,
                    ),
                    &paint,
                );
            }
        }
        BarStyle::Large => {
            let (bar_w, gap) = (6.0f32, 4.0f32);
            let total = BAR_COUNT as f32 * bar_w + (BAR_COUNT - 1) as f32 * gap;
            let start_x = rect.left + (rect.width() - total) / 2.0;
            let bottom = rect.bottom;
            for (i, level) in levels.iter().enumerate() {
                let level = level.clamp(0.08, 1.0);
                let bar_h = level * rect.height();
                let bx = start_x + i as f32 * (bar_w + gap);

                paint.set_color(with_alpha((80.0 + level * 40.0) as u8));
                let bot_h = bar_h * 0.6;
                canvas.draw_rrect(
                    RRect::new_rect_xy(Rect::from_xywh(bx, bottom - bot_h, bar_w, bot_h), 2.0, 2.0),
                    &paint,
                );

                paint.set_color(with_alpha((180.0 + level * 75.0) as u8));
                canvas.draw_rrect(
                    RRect::new_rect_xy(
                        Rect::from_xywh(bx, bottom - bar_h, bar_w, bar_h * 0.5),
                        2.0,
                        2.0,
                    ),
                    &paint,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
        values.into_iter().flat_map(f32::to_le_bytes).collect()
    }

    #[test]
    fn silence_reads_zero() {
        let bytes = samples(std::iter::repeat_n(0.0, 512));
        assert_eq!(buffer_level(&bytes, 2), Some(0.0));
    }

    #[test]
    fn a_full_scale_sine_saturates() {
        let sine = (0..480).map(|n| (n as f32 * std::f32::consts::TAU / 48.0).sin());
        let level = buffer_level(&samples(sine), 1).unwrap();
        assert!(level > 0.99, "level {level}");
    }

    #[test]
    fn a_quiet_sine_reads_quiet() {
        let sine = (0..480).map(|n| 0.05 * (n as f32 * std::f32::consts::TAU / 48.0).sin());
        let level = buffer_level(&samples(sine), 1).unwrap();
        assert!(level > 0.05 && level < 0.15, "level {level}");
    }

    #[test]
    fn only_the_first_channel_is_measured() {
        // Left silent, right at full scale.
        let stereo = (0..256).flat_map(|_| [0.0, 1.0]);
        assert_eq!(buffer_level(&samples(stereo), 2), Some(0.0));
    }

    #[test]
    fn an_empty_buffer_has_no_level() {
        assert_eq!(buffer_level(&[], 2), None);
    }

    #[test]
    fn the_animator_is_deterministic_for_a_seed() {
        let mut a = BarAnimator::default();
        let mut b = BarAnimator::default();
        for _ in 0..20 {
            assert_eq!(a.step(0.6, "track"), b.step(0.6, "track"));
        }
    }

    #[test]
    fn different_seeds_move_differently() {
        let mut a = BarAnimator::default();
        let mut b = BarAnimator::default();
        let (mut la, mut lb) = ([0.0; BAR_COUNT], [0.0; BAR_COUNT]);
        for _ in 0..10 {
            la = a.step(0.6, "one");
            lb = b.step(0.6, "two");
        }
        assert_ne!(la, lb);
    }

    #[test]
    fn silence_settles_to_a_low_shimmer() {
        let mut anim = BarAnimator::default();
        for _ in 0..30 {
            anim.step(1.0, "loud");
        }
        let mut levels = anim.levels();
        for _ in 0..200 {
            levels = anim.step(0.0, "loud");
        }
        assert!(levels.iter().all(|l| *l <= 0.16), "levels {levels:?}");
    }

    #[test]
    fn loud_audio_raises_the_bars() {
        let mut quiet = BarAnimator::default();
        let mut loud = BarAnimator::default();
        let (mut q, mut l) = ([0.0; BAR_COUNT], [0.0; BAR_COUNT]);
        for _ in 0..30 {
            q = quiet.step(0.0, "t");
            l = loud.step(0.9, "t");
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean(&l) > mean(&q) * 2.0);
    }
}
