//! Microphone capture: 16 kHz mono f32 from the default source, the rate and
//! layout Whisper expects, so no resampling happens on this side.

// Rust guideline compliant 2026-02-21

use std::sync::{Arc, Mutex};
use std::thread;

/// Whisper's input sample rate.
pub const SAMPLE_RATE: u32 = 16_000;

/// A running capture stream. Dropping it stops the stream.
pub struct Capture {
    samples: Arc<Mutex<Vec<f32>>>,
    stop: Option<pipewire::channel::Sender<()>>,
}

impl Capture {
    /// Start capturing from the default source on a thread of its own.
    pub fn start() -> Self {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let (stop_tx, stop_rx) = pipewire::channel::channel();
        let thread_samples = samples.clone();
        thread::spawn(move || {
            if let Err(error) = run(thread_samples, stop_rx) {
                tracing::error!(%error, "microphone capture failed");
            }
        });
        Self {
            samples,
            stop: Some(stop_tx),
        }
    }

    /// Play a 16 kHz mono 16-bit WAV file in real time as if it were the
    /// microphone, for testing without one.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the file cannot be read.
    pub fn from_wav(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        // A canonical WAV has a 44-byte header; good enough for test files.
        let clip: Vec<f32> = bytes
            .get(44..)
            .unwrap_or_default()
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
            .collect();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let thread_samples = samples.clone();
        // Stops when the capture is dropped: the Arc is then the thread's alone.
        thread::spawn(move || {
            // 20 ms of audio every 20 ms.
            for chunk in clip.chunks(SAMPLE_RATE as usize / 50) {
                if Arc::strong_count(&thread_samples) == 1 {
                    return;
                }
                if let Ok(mut samples) = thread_samples.lock() {
                    samples.extend_from_slice(chunk);
                }
                thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        Ok(Self {
            samples,
            stop: None,
        })
    }

    /// A capture that never hears anything.
    #[cfg(test)]
    pub(crate) fn silent() -> Self {
        Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            stop: None,
        }
    }

    /// Everything captured so far.
    pub fn samples(&self) -> Vec<f32> {
        self.samples.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// The last `count` samples, or fewer if not that many were captured.
    pub fn tail(&self, count: usize) -> Vec<f32> {
        self.samples
            .lock()
            .map(|s| s[s.len().saturating_sub(count)..].to_vec())
            .unwrap_or_default()
    }

    /// Forget the first `count` samples.
    pub fn trim(&self, count: usize) {
        if let Ok(mut samples) = self.samples.lock() {
            let count = count.min(samples.len());
            samples.drain(..count);
        }
    }

    /// Number of samples captured so far.
    pub fn len(&self) -> usize {
        self.samples.lock().map(|s| s.len()).unwrap_or(0)
    }

    /// Whether nothing was captured yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn run(
    samples: Arc<Mutex<Vec<f32>>>,
    stop: pipewire::channel::Receiver<()>,
) -> Result<(), pipewire::Error> {
    use pipewire as pw;
    use pw::spa;
    use spa::pod::Pod;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let _stop = stop.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });

    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Communication",
        *pw::keys::APP_NAME => "Otto Dictation",
    };
    let stream = pw::stream::StreamBox::new(&core, "otto-dictation", props)?;

    let _listener = stream
        .add_local_listener_with_user_data(samples)
        .process(|stream, samples| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let chunk = data.chunk();
            let start = chunk.offset() as usize;
            let size = chunk.size() as usize;
            let Some(bytes) = data.data() else { return };
            let end = start.saturating_add(size).min(bytes.len());
            if end <= start {
                return;
            }
            let buffer: Vec<f32> = bytes[start..end]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            if let Ok(mut samples) = samples.lock() {
                samples.extend_from_slice(&buffer);
            }
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(SAMPLE_RATE);
    audio_info.set_channels(1);
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
