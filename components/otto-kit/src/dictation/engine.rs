//! A client for whisper.cpp's `whisper-server` API: one clip in, its text
//! out. Otto's Parakeet server answers the same API, and so does CrispASR,
//! whose answer is JSON rather than WebVTT.

// Rust guideline compliant 2026-02-21

use std::time::Duration;

use super::agreement::{is_marker, Word};
use super::capture::SAMPLE_RATE;

/// Where the recogniser lives and what language it listens for.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The server's `/inference` endpoint.
    pub url: String,
    /// An ISO 639-1 code, or `auto` to let Whisper detect it.
    pub language: String,
    /// How much a hotword is favoured, in log-probability per matching
    /// token (CrispASR's `hotwords_boost`). Past about 6 it garbles the
    /// words around a name.
    pub hotwords_boost: f32,
}

/// Why a pass produced no words.
#[derive(Debug)]
pub enum TranscribeError {
    /// The server could not be reached, timed out or answered with an error.
    Http(ureq::Error),
    /// The server answered neither WebVTT nor JSON with timed words.
    Response,
}

impl std::fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(error) => write!(f, "{error}"),
            Self::Response => write!(f, "the server's answer has no timed words"),
        }
    }
}

impl std::error::Error for TranscribeError {}

impl From<ureq::Error> for TranscribeError {
    fn from(error: ureq::Error) -> Self {
        Self::Http(error)
    }
}

/// Longest a single pass may take before it is given up.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Separates the parts of the multipart body; never occurs in a WAV header or
/// the form values.
const BOUNDARY: &str = "otto-dictation-7f3a9c1e5b";

impl Engine {
    /// The engine the environment names, or the local server.
    ///
    /// - `OTTO_DICTATE_URL`: the server's `/inference` endpoint (default
    ///   `http://127.0.0.1:8080/inference`).
    /// - `OTTO_DICTATE_LANGUAGE`: an ISO 639-1 code or `auto` (default: from
    ///   `LANG`).
    /// - `OTTO_DICTATE_HOTWORDS_BOOST`: how much hotwords are favoured
    ///   (default 4).
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("OTTO_DICTATE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080/inference".into()),
            language: std::env::var("OTTO_DICTATE_LANGUAGE").unwrap_or_else(|_| system_language()),
            hotwords_boost: std::env::var("OTTO_DICTATE_HOTWORDS_BOOST")
                .ok()
                .and_then(|boost| boost.parse().ok())
                .unwrap_or(4.0),
        }
    }

    /// Transcribe `samples` (16 kHz mono f32) into timed words. `prompt` is
    /// the text said just before the clip, which keeps a clip that starts
    /// mid-sentence in context. `hotwords`, comma separated, are names the
    /// engine favours while decoding (CrispASR); others ignore them.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::Http`] when the server cannot be reached,
    /// times out or answers with an error status, and
    /// [`TranscribeError::Response`] when the answer is not WebVTT.
    pub fn transcribe(
        &self,
        samples: &[f32],
        prompt: &str,
        hotwords: &str,
    ) -> Result<Vec<Word>, TranscribeError> {
        let boost = self.hotwords_boost.to_string();
        let body = multipart(
            &wav(samples),
            &[
                ("language", &self.language),
                ("prompt", prompt),
                ("hotwords", hotwords),
                ("hotwords_boost", &boost),
            ],
        );
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .new_agent();
        let text = agent
            .post(&self.url)
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .send(&body[..])?
            .into_body()
            .read_to_string()?;
        parse_words(&text)
            .or_else(|| parse_json_words(&text))
            .ok_or(TranscribeError::Response)
    }
}

/// `LANG=it_IT.UTF-8` → `it`; `auto` when it is unset or `C`.
fn system_language() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|lang| lang.get(..2).map(str::to_ascii_lowercase))
        .filter(|code| code.chars().all(|c| c.is_ascii_lowercase()) && code != "c")
        .unwrap_or_else(|| "auto".into())
}

/// The words of a WebVTT answer. Each cue gives a segment's start and end;
/// its words get times spread across the segment in proportion to their
/// length, which is close enough for cutting the audio between segments.
fn parse_words(vtt: &str) -> Option<Vec<Word>> {
    if !vtt.trim_start().starts_with("WEBVTT") {
        return None;
    }
    let mut words = Vec::new();
    let mut lines = vtt.lines();
    while let Some(line) = lines.next() {
        let Some((start, end)) = line.split_once(" --> ") else {
            continue;
        };
        let (Some(start), Some(end)) = (seconds(start), seconds(end)) else {
            continue;
        };
        let text: String = lines
            .by_ref()
            .take_while(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let text = strip_markers(&text);
        let pieces: Vec<&str> = text.split_whitespace().filter(|w| !is_marker(w)).collect();
        let total: usize = pieces.iter().map(|w| w.chars().count()).sum();
        let mut done = 0usize;
        for piece in pieces {
            let len = piece.chars().count();
            let at = |chars: usize| start + (end - start) * chars as f32 / total.max(1) as f32;
            words.push(Word {
                text: format!(" {piece}"),
                start: at(done),
                end: at(done + len),
            });
            done += len;
        }
    }
    Some(words)
}

/// The words of a JSON answer. Either OpenAI's verbose form, with
/// `segments[].words[]` of `word`, `start` and `end` in seconds, or
/// CrispASR's, with `segments[].tokens[]` of `text`, `t0` and `t1` in
/// milliseconds, where a token starting with a space starts a word.
fn parse_json_words(json: &str) -> Option<Vec<Word>> {
    let answer: serde_json::Value = serde_json::from_str(json).ok()?;
    let segments = answer.get("segments")?.as_array()?;
    let mut words: Vec<Word> = Vec::new();
    for segment in segments {
        if let Some(timed) = segment.get("words").and_then(|w| w.as_array()) {
            for word in timed {
                let text = word.get("word")?.as_str()?.trim();
                if text.is_empty() || is_marker(text) {
                    continue;
                }
                words.push(Word {
                    text: format!(" {text}"),
                    start: word.get("start")?.as_f64()? as f32,
                    end: word.get("end")?.as_f64()? as f32,
                });
            }
        } else if let Some(tokens) = segment.get("tokens").and_then(|t| t.as_array()) {
            let mut current: Option<Word> = None;
            for token in tokens {
                let text = token.get("text")?.as_str()?;
                let start = token.get("t0")?.as_f64()? as f32 / 1000.0;
                let end = token.get("t1")?.as_f64()? as f32 / 1000.0;
                match current.as_mut() {
                    Some(word) if !text.starts_with(' ') => {
                        word.text.push_str(text);
                        word.end = end;
                    }
                    _ => {
                        words.extend(current.take());
                        current = Some(Word {
                            text: format!(" {}", text.trim_start()),
                            start,
                            end,
                        });
                    }
                }
            }
            words.extend(current);
        }
    }
    words.retain(|w| !w.text.trim().is_empty() && !is_marker(&w.text));
    Some(words)
}

/// `text` without the spans Whisper uses for sounds that are not speech:
/// `*Sounds of a dead man*`, `[BLANK_AUDIO]`, `(music)`.
fn strip_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut closing: Option<char> = None;
    for c in text.chars() {
        match closing {
            Some(close) if c == close => closing = None,
            Some(_) => {}
            None => match c {
                '*' => closing = Some('*'),
                '[' => closing = Some(']'),
                '(' => closing = Some(')'),
                _ => out.push(c),
            },
        }
    }
    out
}

/// `00:00:04.640` as seconds.
fn seconds(stamp: &str) -> Option<f32> {
    let mut total = 0.0;
    for part in stamp.trim().split(':') {
        total = total * 60.0 + part.parse::<f32>().ok()?;
    }
    Some(total)
}

/// `samples` as a 16-bit PCM WAV file.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn multipart(wav: &[u8], fields: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 1024);
    let fixed = [
        ("response_format", "vtt"),
        ("temperature", "0.0"),
        ("suppress_nst", "true"),
    ];
    for &(name, value) in fixed.iter().chain(fields) {
        if value.is_empty() {
            continue;
        }
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"clip.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wav_header_describes_16k_mono_pcm() {
        let out = wav(&[0.0, 1.0, -1.0]);
        assert_eq!(out.len(), 44 + 6);
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(u32::from_le_bytes(out[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(out[40..44].try_into().unwrap()), 6);
        assert_eq!(i16::from_le_bytes([out[46], out[47]]), i16::MAX);
    }

    #[test]
    fn words_come_from_every_cue_without_markers() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\n Hello *Gunshot* there\n\n00:00:01.500 --> 00:00:02.000\n world.\n\n";
        let words = parse_words(vtt).unwrap();
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, [" Hello", " there", " world."]);
        assert_eq!(words[0].start, 0.0);
        assert!((words[1].end - 1.0).abs() < 1e-6);
        assert_eq!(words[2].start, 1.5);
    }

    #[test]
    fn json_answers_give_timed_words() {
        let crisp = r#"{"segments":[{"t0":240,"t1":1040,"tokens":[
            {"text":" And","t0":240,"t1":560},{"text":" so","t0":560,"t1":880},
            {"text":",","t0":880,"t1":1040},{"text":" Ghost","t0":1100,"t1":1300},
            {"text":"ty","t0":1300,"t1":1500}]}]}"#;
        let words = parse_json_words(crisp).unwrap();
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, [" And", " so,", " Ghostty"]);
        assert!((words[1].end - 1.04).abs() < 1e-6);

        let openai = r#"{"segments":[{"words":[{"word":"Hello","start":0.1,"end":0.4},{"word":"[BLANK_AUDIO]","start":0.4,"end":1.0}]}]}"#;
        let words = parse_json_words(openai).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, " Hello");
        assert!(parse_json_words("not json").is_none());
    }

    #[test]
    fn marker_spans_are_removed_whole() {
        assert_eq!(
            strip_markers("Hi *Sounds of a dead man* there [BLANK_AUDIO]"),
            "Hi  there "
        );
    }

    #[test]
    fn timestamps_parse() {
        assert_eq!(seconds("00:01:04.500"), Some(64.5));
        assert_eq!(seconds("bad"), None);
    }
}
