//! Incremental transcription on top of a synchronous engine.
//!
//! Every pass transcribes the audio buffer from its start. A word is settled
//! once two passes in a row agree on it (local agreement): settled words are
//! committed into the field, the rest stay as preedit. The caller trims the
//! audio up to the last settled word now and then, so a pass only ever covers
//! the tail of what was said.

// Rust guideline compliant 2026-02-21

/// One recognised word, with times in seconds from the start of the buffer
/// the pass covered.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    /// As the engine wrote it, leading space included.
    pub text: String,
    pub start: f32,
    pub end: f32,
}

/// What a pass changed.
#[derive(Debug, Default, PartialEq)]
pub struct Update {
    /// Words settled by this pass, to commit.
    pub settled: Vec<Word>,
    /// Words not settled yet, to show as preedit.
    pub tentative: Vec<Word>,
}

/// Slack when comparing a word's start with the end of the settled audio.
const TIME_SLACK: f32 = 0.1;
/// Longest run of settled words a new pass may repeat at its start.
const MAX_OVERLAP: usize = 5;

/// Settles words across passes over one buffer.
#[derive(Debug, Default)]
pub struct Agreement {
    /// The unsettled words of the previous pass.
    previous: Vec<Word>,
    /// End of the last settled word, in buffer time.
    settled_end: f32,
    /// The last few settled words, to spot a pass repeating them.
    recent: Vec<String>,
}

impl Agreement {
    /// Fold in a pass over the buffer.
    pub fn step(&mut self, words: Vec<Word>) -> Update {
        let fresh = self.drop_settled(words);
        let agreed = fresh
            .iter()
            .zip(&self.previous)
            .take_while(|(new, old)| normalise(&new.text) == normalise(&old.text))
            .count();
        let settled = fresh[..agreed].to_vec();
        let tentative = fresh[agreed..].to_vec();
        if let Some(last) = settled.last() {
            self.settled_end = last.end;
        }
        self.recent.extend(settled.iter().map(|w| normalise(&w.text)));
        let excess = self.recent.len().saturating_sub(MAX_OVERLAP);
        self.recent.drain(..excess);
        self.previous = tentative.clone();
        Update { settled, tentative }
    }

    /// The last pass, when the audio has ended: everything left is settled.
    pub fn finish(&mut self, words: Vec<Word>) -> Vec<Word> {
        let fresh = self.drop_settled(words);
        self.previous.clear();
        fresh
    }

    /// Where the buffer can be cut: the end of the last settled word.
    pub fn settled_end(&self) -> f32 {
        self.settled_end
    }

    /// The buffer lost its first `seconds`; move every time back by as much.
    pub fn trimmed(&mut self, seconds: f32) {
        self.settled_end = (self.settled_end - seconds).max(0.0);
        for word in &mut self.previous {
            word.start -= seconds;
            word.end -= seconds;
        }
    }

    /// Words of a pass that were settled before: those that start inside the
    /// settled audio, then any repeat of the last settled words (a word that
    /// straddles the cut can come back).
    fn drop_settled(&self, words: Vec<Word>) -> Vec<Word> {
        let mut fresh: Vec<Word> = words
            .into_iter()
            .filter(|w| w.start >= self.settled_end - TIME_SLACK)
            .collect();
        let limit = self.recent.len().min(fresh.len());
        for n in (1..=limit).rev() {
            let tail = &self.recent[self.recent.len() - n..];
            let head = fresh[..n].iter().map(|w| normalise(&w.text));
            if head.eq(tail.iter().cloned()) {
                fresh.drain(..n);
                break;
            }
        }
        fresh
    }
}

/// A word reduced to what matters for agreement: letters and digits, in lower
/// case, so "Hello," and "hello" agree.
fn normalise(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether the engine's word is a marker for something that is not speech:
/// `[BLANK_AUDIO]`, `*Gunshot*`, `(music)`.
pub fn is_marker(word: &str) -> bool {
    word.contains(['[', ']', '*', '(', ')', '♪'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &[(&str, f32)]) -> Vec<Word> {
        spec.iter()
            .map(|(text, start)| Word {
                text: format!(" {text}"),
                start: *start,
                end: start + 0.3,
            })
            .collect()
    }

    fn texts(words: &[Word]) -> Vec<&str> {
        words.iter().map(|w| w.text.trim()).collect()
    }

    #[test]
    fn a_word_settles_when_two_passes_agree() {
        let mut agreement = Agreement::default();
        let first = agreement.step(words(&[("hello", 0.0), ("word", 0.4)]));
        assert!(first.settled.is_empty());
        assert_eq!(texts(&first.tentative), ["hello", "word"]);

        let second = agreement.step(words(&[("Hello,", 0.0), ("world", 0.4), ("again", 0.8)]));
        assert_eq!(texts(&second.settled), ["Hello,"]);
        assert_eq!(texts(&second.tentative), ["world", "again"]);
    }

    #[test]
    fn settled_words_are_not_settled_twice() {
        let mut agreement = Agreement::default();
        agreement.step(words(&[("one", 0.0), ("two", 0.4)]));
        agreement.step(words(&[("one", 0.0), ("two", 0.4)]));
        let third = agreement.step(words(&[("one", 0.0), ("two", 0.4), ("three", 0.8)]));
        assert!(third.settled.is_empty());
        assert_eq!(texts(&third.tentative), ["three"]);
    }

    #[test]
    fn trimming_keeps_agreement_on_the_tail() {
        let mut agreement = Agreement::default();
        agreement.step(words(&[("one", 0.0), ("two", 0.4)]));
        agreement.step(words(&[("one", 0.0), ("two", 0.4), ("three", 0.8)]));
        agreement.trimmed(agreement.settled_end());
        // The buffer now starts at 0.7 s; "three" is at 0.1.
        let next = agreement.step(words(&[("three", 0.1), ("four", 0.5)]));
        assert_eq!(texts(&next.settled), ["three"]);
        assert_eq!(texts(&next.tentative), ["four"]);
    }

    #[test]
    fn a_repeated_word_at_the_cut_is_dropped() {
        let mut agreement = Agreement::default();
        agreement.step(words(&[("one", 0.0), ("two", 0.4)]));
        agreement.step(words(&[("one", 0.0), ("two", 0.4)]));
        agreement.trimmed(agreement.settled_end());
        let next = agreement.step(words(&[("two", 0.0), ("three", 0.3)]));
        assert!(next.settled.is_empty());
        assert_eq!(texts(&next.tentative), ["three"]);
    }

    #[test]
    fn finish_settles_the_rest() {
        let mut agreement = Agreement::default();
        agreement.step(words(&[("one", 0.0)]));
        agreement.step(words(&[("one", 0.0), ("two", 0.4)]));
        let rest = agreement.finish(words(&[("one", 0.0), ("two", 0.4), ("three", 0.8)]));
        assert_eq!(texts(&rest), ["two", "three"]);
    }

    #[test]
    fn markers_are_recognised() {
        assert!(is_marker(" *Gunshot*"));
        assert!(is_marker(" [BLANK_AUDIO]"));
        assert!(!is_marker(" hello,"));
    }
}
