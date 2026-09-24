//! What the field expects to hear: names the app is offering, such as the
//! apps a launcher lists.
//!
//! Engines spell a name they don't know the way it sounds: "Ghostty" comes
//! back as "ghost tea", "Firefox" as "fire fox". A [`Vocabulary`] puts the
//! name back when what was heard comes close to it, whatever the engine. For
//! an engine that takes a prompt (Whisper), the names also go in it, which
//! makes them likelier to be heard right in the first place.

// Rust guideline compliant 2026-02-21

/// Most words a name may span, and so the longest run of heard words
/// compared with one.
const MAX_WORDS: usize = 4;
/// Shortest name worth correcting to, in letters and digits: shorter ones
/// match too much of ordinary speech.
const MIN_KEY: usize = 4;

/// Names to recognise, as they are written.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    /// Each name, and its key: letters and digits only, in lower case.
    entries: Vec<(String, String)>,
}

impl Vocabulary {
    /// A vocabulary of `names`. Duplicates and names too short to correct to
    /// are left out.
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut entries: Vec<(String, String)> = Vec::new();
        for name in names {
            let name: String = name.into();
            let key = key(&name);
            let words = name.split_whitespace().count();
            if key.chars().count() >= MIN_KEY
                && words <= MAX_WORDS
                && !entries.iter().any(|(_, k)| *k == key)
            {
                entries.push((name, key));
            }
        }
        Self { entries }
    }

    /// Whether there is nothing to recognise.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The names, comma separated, cut to about `max_chars`: context for an
    /// engine that takes a prompt.
    pub fn prompt(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for (name, _) in &self.entries {
            if out.len() + name.len() + 2 > max_chars {
                break;
            }
            if !out.is_empty() {
                out.push_str(", ");
            }
            out.push_str(name);
        }
        out
    }

    /// The names, comma separated, at most `max` of them: hotwords for an
    /// engine that favours them while decoding.
    pub fn hotwords(&self, max: usize) -> String {
        self.entries
            .iter()
            .take(max)
            .map(|(name, _)| name.replace(',', " "))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// `text` with every run of words that sounds like a name replaced by
    /// the name. Words that already spell one are left as they are, case and
    /// all: only what was misheard changes.
    pub fn correct(&self, text: &str) -> String {
        if self.entries.is_empty() {
            return text.to_string();
        }
        let leading = text.len() - text.trim_start().len();
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut out: Vec<String> = Vec::with_capacity(words.len());
        let mut i = 0;
        while i < words.len() {
            match self.best_at(&words[i..]) {
                Some((n, name)) => {
                    let first = words[i];
                    let last = words[i + n - 1];
                    let before: String =
                        first.chars().take_while(|c| !c.is_alphanumeric()).collect();
                    let after: String = {
                        let tail: Vec<char> = last
                            .chars()
                            .rev()
                            .take_while(|c| !c.is_alphanumeric())
                            .collect();
                        tail.into_iter().rev().collect()
                    };
                    out.push(format!("{before}{name}{after}"));
                    i += n;
                }
                None => {
                    out.push(words[i].to_string());
                    i += 1;
                }
            }
        }
        let mut corrected = " ".repeat(leading.min(1));
        corrected.push_str(&out.join(" "));
        if text.ends_with(char::is_whitespace) && !out.is_empty() {
            corrected.push(' ');
        }
        if corrected != text {
            tracing::info!(heard = %text, corrected = %corrected, "vocabulary");
        }
        corrected
    }

    /// The name the words at the start of `words` come closest to, and how
    /// many words it takes, when close enough and not already spelled right.
    fn best_at(&self, words: &[&str]) -> Option<(usize, &str)> {
        let mut best: Option<(usize, usize, &str)> = None;
        for n in (1..=MAX_WORDS.min(words.len())).rev() {
            let heard = key(&words[..n].concat());
            let len = heard.chars().count();
            if len < MIN_KEY {
                continue;
            }
            for (name, name_key) in &self.entries {
                // One word that already spells the name is right as it is.
                if n == 1 && heard == *name_key {
                    return None;
                }
                let distance = levenshtein(&heard, name_key);
                if distance <= allowed(len.max(name_key.chars().count()))
                    && best.is_none_or(|(d, _, _)| distance < d)
                {
                    best = Some((distance, n, name));
                }
            }
        }
        best.map(|(_, n, name)| (n, name))
    }
}

/// Edits allowed between a heard run and a name `len` letters long: none
/// for short names, which ordinary words come close to too easily.
fn allowed(len: usize) -> usize {
    match len {
        0..=4 => 0,
        5..=6 => 1,
        _ => 2,
    }
}

/// Letters and digits only, in lower case: "Fire fox," and "Firefox" share it.
fn key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vocabulary {
        Vocabulary::new([
            "Firefox",
            "Ghostty",
            "Files",
            "Visual Studio Code",
            "Settings",
            "vi",
        ])
    }

    #[test]
    fn misheard_names_are_put_back() {
        let v = apps();
        assert_eq!(v.correct("open fire fox"), "open Firefox");
        assert_eq!(v.correct("ghost tea"), "Ghostty");
        assert_eq!(v.correct(" visual studio cold."), " Visual Studio Code.");
    }

    #[test]
    fn words_that_are_right_or_far_stay_as_they_are() {
        let v = apps();
        assert_eq!(v.correct("summarise these files"), "summarise these files");
        assert_eq!(v.correct("open the settings"), "open the settings");
        assert_eq!(v.correct("what time is it"), "what time is it");
    }

    #[test]
    fn short_names_are_not_corrected_to() {
        assert!(Vocabulary::new(["vi", "Zed"]).is_empty());
        assert_eq!(apps().correct("we"), "we");
    }

    #[test]
    fn the_prompt_lists_names_up_to_a_length() {
        let v = apps();
        assert_eq!(v.prompt(18), "Firefox, Ghostty");
        assert_eq!(Vocabulary::default().prompt(100), "");
    }

    #[test]
    fn hotwords_are_the_names_comma_separated() {
        assert_eq!(apps().hotwords(2), "Firefox,Ghostty");
        assert_eq!(
            Vocabulary::new(["Files, Folders"]).hotwords(10),
            "Files  Folders"
        );
    }

    #[test]
    fn edit_distance() {
        assert_eq!(levenshtein("ghosttea", "ghostty"), 2);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }
}
