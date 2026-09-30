//! Audio visualiser: a PipeWire spectrum meter, the bar animation it drives
//! and the bars themselves.
//!
//! The music island listens to the player's own stream, never to the whole
//! output: other sounds must not move the bars.

use std::sync::{Arc, Mutex};
use std::thread;

use skia_safe::{Canvas, Color, Paint, RRect, Rect};

/// Number of bars an animator tracks, one per frequency band. Every bar style
/// draws from these.
pub const BAR_COUNT: usize = 12;

/// Centre frequency of each band, evenly spaced on a log scale from bass to
/// treble, about two thirds of an octave apart.
const BAND_HZ: [f32; BAR_COUNT] = [
    50.0, 80.0, 135.0, 225.0, 370.0, 610.0, 1000.0, 1650.0, 2750.0, 4500.0, 7500.0, 12000.0,
];

/// Band-pass Q for bands two thirds of an octave wide: neighbours overlap a little, so a
/// note between two centres lights both rather than neither.
const BAND_Q: f32 = 2.0;

/// The loudness of each band of one application stream, updated from a
/// capture stream on its own thread.
///
/// The stream exists only while the meter listens. A connected capture stream
/// keeps its target running, so a meter left on would stop the sound card
/// from ever suspending.
pub struct LevelMeter {
    /// The loudest reading of each band since the last [`LevelMeter::take_bands`].
    bands: Arc<Mutex<[f32; BAR_COUNT]>>,
    /// The `object.serial` of the stream listened to, and how to stop.
    capture: Option<(u32, pipewire::channel::Sender<()>)>,
}

impl LevelMeter {
    /// A meter, not yet listening.
    pub fn new() -> Self {
        Self {
            bands: Arc::new(Mutex::new([0.0; BAR_COUNT])),
            capture: None,
        }
    }

    /// Listen to the stream with `object.serial` `serial`, on whichever output
    /// it plays, or stop with `None`. Changing the stream replaces the capture
    /// stream. A PipeWire failure is logged and the meter then reads silence.
    pub fn listen(&mut self, serial: Option<u32>) {
        if self.capture.as_ref().map(|(current, _)| *current) == serial {
            return;
        }
        if let Some((_, stop)) = self.capture.take() {
            let _ = stop.send(());
        }
        self.take_bands();
        let Some(serial) = serial else { return };
        let (stop_tx, stop_rx) = pipewire::channel::channel();
        let bands = self.bands.clone();
        thread::spawn(move || {
            if let Err(error) = run_capture(serial, bands, stop_rx) {
                tracing::error!(%error, "PipeWire level meter failed");
            }
        });
        self.capture = Some((serial, stop_tx));
    }

    /// The loudest RMS of each band since the last call, and start over.
    ///
    /// Holding the loudest reading between frames keeps a drum hit that
    /// lands between two redraws from being missed.
    pub fn take_bands(&self) -> [f32; BAR_COUNT] {
        self.bands
            .lock()
            .map(|mut bands| std::mem::take(&mut *bands))
            .unwrap_or_default()
    }
}

impl Drop for LevelMeter {
    fn drop(&mut self) {
        self.listen(None);
    }
}

/// A second-order band-pass filter (the RBJ cookbook's, 0 dB at its peak), in
/// transposed direct form II.
#[derive(Debug, Clone, Copy, Default)]
struct BandPass {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BandPass {
    fn new(rate: f32, centre: f32, q: f32) -> Self {
        // A centre at or past Nyquist can't be filtered; keep it just under.
        let centre = centre.min(rate * 0.45);
        let w0 = std::f32::consts::TAU * centre / rate;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            b2: -alpha / a0,
            a1: -2.0 * w0.cos() / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = -self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Splits a stream into [`BAR_COUNT`] bands and measures each.
///
/// Eight small filters per sample, a few million operations a second at
/// 48 kHz: far cheaper than an FFT and the redraw it feeds. The filters keep
/// their state between buffers, so the stream has to be fed in order.
pub struct BandAnalyser {
    rate: u32,
    filters: [BandPass; BAR_COUNT],
}

impl BandAnalyser {
    /// An analyser for a stream sampled at `rate` Hz.
    pub fn new(rate: u32) -> Self {
        let rate = rate.max(1);
        Self {
            rate,
            filters: BAND_HZ.map(|hz| BandPass::new(rate as f32, hz, BAND_Q)),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The RMS of each band over one buffer of interleaved f32 samples, the
    /// channels mixed to mono. `None` for a buffer with no whole frame.
    pub fn process(&mut self, bytes: &[u8], channels: usize) -> Option<[f32; BAR_COUNT]> {
        const SAMPLE: usize = std::mem::size_of::<f32>();
        let channels = channels.max(1);
        let mut sum_sq = [0.0f32; BAR_COUNT];
        let mut frames = 0usize;
        for frame in bytes.chunks_exact(SAMPLE * channels) {
            let mono = frame
                .chunks_exact(SAMPLE)
                .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
                .sum::<f32>()
                / channels as f32;
            for (filter, sum) in self.filters.iter_mut().zip(sum_sq.iter_mut()) {
                let y = filter.run(mono);
                *sum += y * y;
            }
            frames += 1;
        }
        (frames > 0).then(|| sum_sq.map(|sum| (sum / frames as f32).sqrt()))
    }
}

fn run_capture(
    serial: u32,
    shared_bands: Arc<Mutex<[f32; BAR_COUNT]>>,
    stop: pipewire::channel::Receiver<()>,
) -> Result<(), pipewire::Error> {
    use pipewire as pw;
    use pw::spa;
    use spa::param::format::{MediaSubtype, MediaType};
    use spa::param::format_utils;
    use spa::pod::Pod;

    /// PipeWire's usual graph rate, until the format says otherwise.
    const DEFAULT_RATE: u32 = 48_000;

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
        analyser: BandAnalyser,
        bands: Arc<Mutex<[f32; BAR_COUNT]>>,
    }

    let mut props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
    };
    // Linked to the application's output ports, beside its own link to
    // whichever sink it plays on.
    props.insert("target.object", serial.to_string());
    // When that stream goes the session manager would otherwise relink the
    // capture to the default source, the microphone. The route picks the next
    // stream itself.
    props.insert(*pw::keys::NODE_DONT_RECONNECT, "true");
    props.insert("node.dont-fallback", "true");

    let stream = pw::stream::StreamBox::new(&core, "otto-islands-level-meter", props)?;

    let user_data = UserData {
        channels: 2,
        analyser: BandAnalyser::new(DEFAULT_RATE),
        bands: shared_bands,
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
                if audio_info.rate() > 0 && audio_info.rate() != user_data.analyser.rate() {
                    user_data.analyser = BandAnalyser::new(audio_info.rate());
                }
            }
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
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
            let channels = user_data.channels as usize;
            if let Some(bands) = user_data.analyser.process(&samples[start..end], channels) {
                if let Ok(mut shared) = user_data.bands.lock() {
                    for (held, band) in shared.iter_mut().zip(bands) {
                        *held = held.max(band);
                    }
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

/// Turns band readings into bar heights.
///
/// Each band is read against its own recent peak rather than full scale:
/// music has far more energy in the bass than the treble, and a player at
/// half volume should still fill the bars. Bars jump up at once and fall back
/// quickly, so a beat reads as a beat.
pub struct BarAnimator {
    /// The loudest recent reading of each band, decaying by [`PEAK_DECAY`].
    peaks: [f32; BAR_COUNT],
    levels: [f32; BAR_COUNT],
}

impl Default for BarAnimator {
    fn default() -> Self {
        Self {
            peaks: [SILENCE_RMS; BAR_COUNT],
            levels: [IDLE_LEVEL; BAR_COUNT],
        }
    }
}

impl BarAnimator {
    /// Advance one frame with the latest band readings.
    pub fn step(&mut self, bands: [f32; BAR_COUNT]) -> [f32; BAR_COUNT] {
        for (peak, band) in self.peaks.iter_mut().zip(bands) {
            *peak = (*peak * PEAK_DECAY).max(band);
        }
        // A band that is nearly empty next to the others (no treble in a
        // lo-fi track) is read against the loudest band, so its noise isn't
        // blown up into a full bar.
        let loudest = self.peaks.iter().copied().fold(0.0, f32::max);
        let floor = (loudest * QUIET_BAND_RATIO).max(SILENCE_RMS);
        for ((level, peak), band) in self.levels.iter_mut().zip(self.peaks).zip(bands) {
            let ratio = (band / peak.max(floor)).clamp(0.0, 1.0);
            let target = IDLE_LEVEL + (1.0 - IDLE_LEVEL) * ratio;
            *level += (target - *level) * if target > *level { ATTACK } else { RELEASE };
        }
        self.levels
    }

    pub fn levels(&self) -> [f32; BAR_COUNT] {
        self.levels
    }
}

/// Where a bar rests in silence, so the row stays visible.
const IDLE_LEVEL: f32 = 0.06;

/// Band RMS treated as silence, about -48 dBFS. Peaks never fall below it, so
/// a fade or hiss doesn't get amplified into a full dance.
const SILENCE_RMS: f32 = 0.004;

/// How far below the loudest band another band's peak is floored.
const QUIET_BAND_RATIO: f32 = 0.08;

/// How fast each band's recent peak forgets, per frame. At ~24 fps it halves
/// in about three seconds: long enough that a quiet verse stays quieter than
/// the chorus, short enough that turning the volume down refills the bars.
const PEAK_DECAY: f32 = 0.99;

/// Share of the way to a higher target a bar covers per frame: nearly all
/// of it, so a hit shows on the frame it lands.
const ATTACK: f32 = 0.85;

/// Share of the way down per frame: a bar drops most of the way within a
/// few frames, like a meter's needle.
const RELEASE: f32 = 0.35;

/// Width of one bar beside a compact island's title.
pub const COMPACT_BAR_W: f32 = 2.0;

/// Space between two bars beside a compact island's title.
pub const COMPACT_BAR_GAP: f32 = 2.0;

/// How a row of bars is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarStyle {
    /// Three bars centred in a circle.
    Mini,
    /// A few thin bars growing both ways from the middle, beside a title.
    Compact(usize),
    /// Every bar, standing on the bottom edge, brighter as it rises.
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
            for i in 0..count {
                let level = levels[i * BAR_COUNT / count];
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
            let (bar_w, gap) = (COMPACT_BAR_W, COMPACT_BAR_GAP);
            let total = count as f32 * bar_w + (count as f32 - 1.0) * gap;
            let start_x = rect.left + (rect.width() - total) / 2.0;
            let center_y = rect.top + rect.height() / 2.0;
            paint.set_color(with_alpha(220));
            for i in 0..count {
                let bar_h = levels[i * BAR_COUNT / count].clamp(0.05, 1.0) * rect.height();
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
            let (bar_w, gap) = (4.0f32, 3.0f32);
            let total = BAR_COUNT as f32 * bar_w + (BAR_COUNT - 1) as f32 * gap;
            let start_x = rect.left + (rect.width() - total) / 2.0;
            for (i, level) in levels.iter().enumerate() {
                let level = level.clamp(0.08, 1.0);
                let bar_h = level * rect.height();
                let bx = start_x + i as f32 * (bar_w + gap);
                paint.set_color(with_alpha((190.0 + level * 65.0) as u8));
                canvas.draw_rrect(
                    RRect::new_rect_xy(
                        Rect::from_xywh(bx, rect.bottom - bar_h, bar_w, bar_h),
                        2.0,
                        2.0,
                    ),
                    &paint,
                );
            }
        }
    }
}

/// Draw the "playing on another device" glyph, a screen with waves coming
/// off its corner, as big as fits centred in `rect`.
pub fn draw_elsewhere_glyph(canvas: &Canvas, rect: Rect, colour: Color) {
    use skia_safe::{PaintStyle, PathBuilder, Point};

    // Drawn on a 24-unit grid.
    let size = rect.width().min(rect.height());
    let unit = size / 24.0;
    let origin = Point::new(
        rect.left + (rect.width() - size) / 2.0,
        rect.top + (rect.height() - size) / 2.0,
    );
    let at = |x: f32, y: f32| Point::new(origin.x + x * unit, origin.y + y * unit);
    let around_corner = |radius: f32| {
        let corner = at(2.0, 21.0);
        Rect::from_ltrb(
            corner.x - radius * unit,
            corner.y - radius * unit,
            corner.x + radius * unit,
            corner.y + radius * unit,
        )
    };

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(220, colour.r(), colour.g(), colour.b()));
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(2.0 * unit);
    paint.set_stroke_cap(skia_safe::paint::Cap::Round);
    paint.set_stroke_join(skia_safe::paint::Join::Round);

    // The screen, open at the bottom-left where the waves are.
    let mut frame = PathBuilder::new();
    frame.move_to(at(2.0, 9.0));
    frame.line_to(at(2.0, 5.0));
    frame.quad_to(at(2.0, 3.0), at(4.0, 3.0));
    frame.line_to(at(20.0, 3.0));
    frame.quad_to(at(22.0, 3.0), at(22.0, 5.0));
    frame.line_to(at(22.0, 19.0));
    frame.quad_to(at(22.0, 21.0), at(20.0, 21.0));
    frame.line_to(at(14.0, 21.0));
    canvas.draw_path(&frame.detach(), &paint);

    for radius in [7.5, 11.5] {
        canvas.draw_arc(around_corner(radius), -90.0, 90.0, false, &paint);
    }

    paint.set_style(PaintStyle::Fill);
    let mut dot = PathBuilder::new();
    dot.move_to(at(2.0, 21.0));
    dot.arc_to(around_corner(3.5), -90.0, 90.0, false);
    dot.close();
    canvas.draw_path(&dot.detach(), &paint);
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// `seconds` of a sine at `hz`, amplitude `amp`, as interleaved stereo.
    fn stereo_sine(hz: f32, amp: f32, seconds: f32) -> Vec<u8> {
        let frames = (RATE as f32 * seconds) as usize;
        (0..frames)
            .map(|n| amp * (n as f32 * std::f32::consts::TAU * hz / RATE as f32).sin())
            .flat_map(|v| [v, v])
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    fn loudest_band(bands: [f32; BAR_COUNT]) -> usize {
        (0..BAR_COUNT)
            .max_by(|&a, &b| bands[a].total_cmp(&bands[b]))
            .unwrap()
    }

    #[test]
    fn silence_reads_zero_in_every_band() {
        let mut analyser = BandAnalyser::new(RATE);
        let bands = analyser.process(&vec![0; 4096], 2).unwrap();
        assert_eq!(bands, [0.0; BAR_COUNT]);
    }

    #[test]
    fn a_tone_lights_its_own_band() {
        for (band, hz) in BAND_HZ.iter().enumerate() {
            let mut analyser = BandAnalyser::new(RATE);
            let bands = analyser.process(&stereo_sine(*hz, 0.5, 0.25), 2).unwrap();
            assert_eq!(loudest_band(bands), band, "{hz} Hz: {bands:?}");
        }
    }

    #[test]
    fn a_band_passes_its_centre_at_full_level() {
        let mut analyser = BandAnalyser::new(RATE);
        let bands = analyser.process(&stereo_sine(1000.0, 1.0, 0.5), 2).unwrap();
        // A full-scale sine has an RMS of 1/sqrt(2).
        assert!(
            (bands[6] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.05,
            "{bands:?}"
        );
    }

    #[test]
    fn an_empty_buffer_has_no_reading() {
        assert!(BandAnalyser::new(RATE).process(&[], 2).is_none());
    }

    #[test]
    fn a_low_rate_keeps_the_filters_stable() {
        // The top band sits past Nyquist at 8 kHz.
        let mut analyser = BandAnalyser::new(8_000);
        let bands = analyser
            .process(&stereo_sine(1000.0, 0.5, 0.25), 2)
            .unwrap();
        assert!(bands.iter().all(|b| b.is_finite() && *b < 1.0), "{bands:?}");
    }

    #[test]
    fn a_hit_shows_on_the_frame_it_lands() {
        let mut anim = BarAnimator::default();
        for _ in 0..30 {
            anim.step([0.1; BAR_COUNT]);
        }
        // A quarter of a second between beats.
        for _ in 0..6 {
            anim.step([0.01; BAR_COUNT]);
        }
        let low = anim.levels()[0];
        let hit = anim.step([0.1; BAR_COUNT])[0];
        assert!(hit - low > 0.5, "{low} -> {hit}");
    }

    #[test]
    fn silence_settles_low() {
        let mut anim = BarAnimator::default();
        for _ in 0..30 {
            anim.step([0.2; BAR_COUNT]);
        }
        let mut levels = anim.levels();
        for _ in 0..20 {
            levels = anim.step([0.0; BAR_COUNT]);
        }
        assert!(levels.iter().all(|l| *l < IDLE_LEVEL + 0.01), "{levels:?}");
    }

    #[test]
    fn a_quiet_player_fills_the_bars_like_a_loud_one() {
        let mut quiet = BarAnimator::default();
        let mut loud = BarAnimator::default();
        let (mut q, mut l) = ([0.0; BAR_COUNT], [0.0; BAR_COUNT]);
        for _ in 0..60 {
            q = quiet.step([0.03; BAR_COUNT]);
            l = loud.step([0.3; BAR_COUNT]);
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&q) - mean(&l)).abs() < 0.02, "{q:?} vs {l:?}");
        assert!(mean(&q) > 0.9, "{q:?}");
    }

    #[test]
    fn an_empty_band_beside_loud_ones_stays_low() {
        let mut anim = BarAnimator::default();
        let mut bands = [0.3; BAR_COUNT];
        bands[7] = 0.005;
        let mut levels = [0.0; BAR_COUNT];
        for _ in 0..60 {
            levels = anim.step(bands);
        }
        assert!(levels[7] < 0.3, "{levels:?}");
    }
}
