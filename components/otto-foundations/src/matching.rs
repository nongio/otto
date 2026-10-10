//! Scoring a typed query against a name.
//!
//! One implementation, shared: the launcher ranks applications with it and file
//! search ranks names with it, so "what does typing `fire dev` find" has the
//! same answer everywhere in the desktop. `otto_kit::matching` re-exports it.
//!
//! This is subsequence matching, not fuzzy edit distance. A query matches only
//! if every character of it appears in order; what the score decides is which
//! of the matches comes first.

/// Score `query` as a subsequence of `text`, or `None` if it is not one.
///
/// The shape of the score is what makes typing feel right: a match at the start
/// of a word beats one in the middle, a run of adjacent characters beats the
/// same characters scattered, and a short name beats a long one that happens to
/// contain the same letters.
///
/// An empty query matches everything with a score of zero, which lets a caller
/// pass the query straight through without special-casing the empty field.
pub fn score(text: &str, query: &str) -> Option<i32> {
    let hay: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }

    let mut score = 0i32;
    let mut cursor = 0usize;
    let mut previous: Option<usize> = None;

    for &wanted in &needle {
        // Spaces in the query separate words rather than having to be matched:
        // "fire dev" should find "Firefox Developer Edition".
        if wanted == ' ' {
            previous = None;
            continue;
        }
        let found = hay[cursor..].iter().position(|&c| c == wanted)? + cursor;

        score += 8;
        let boundary = found == 0
            || matches!(
                hay[found - 1],
                ' ' | '-' | '_' | '.' | '/' | ':' | '(' | '['
            );
        if boundary {
            score += 14;
        }
        if found == 0 {
            score += 20;
        }
        match previous {
            Some(last) if found == last + 1 => score += 12,
            Some(last) => score -= (found - last - 1).min(10) as i32,
            None => {}
        }

        previous = Some(found);
        cursor = found + 1;
    }

    // Prefer the shorter of two names that both match: "Files" over
    // "Files (Nautilus) Preferences".
    score -= (hay.len() / 6) as i32;
    Some(score)
}

/// Which characters of `text` a match of `query` lands on, as indices into
/// `text.chars()`, or `None` if `query` is not a subsequence of it.
///
/// The same walk [`score`] takes — the first place each character can go,
/// spaces crossing words — so what a list marks as matched is exactly what was
/// scored. An empty query marks nothing.
pub fn positions(text: &str, query: &str) -> Option<Vec<usize>> {
    // Lowered a character at a time so each lowered one still knows where it
    // came from: a capital that lowers to two characters marks once.
    let hay: Vec<(char, usize)> = text
        .chars()
        .enumerate()
        .flat_map(|(at, c)| c.to_lowercase().map(move |lower| (lower, at)))
        .collect();

    let mut marked = Vec::new();
    let mut cursor = 0usize;
    for wanted in query.to_lowercase().chars() {
        if wanted == ' ' {
            continue;
        }
        let found = hay[cursor..].iter().position(|&(c, _)| c == wanted)? + cursor;
        let at = hay[found].1;
        if marked.last() != Some(&at) {
            marked.push(at);
        }
        cursor = found + 1;
    }
    Some(marked)
}

/// Which characters of `text` start its words with the words of `query`, as
/// indices into `text.chars()`, or `None` unless every word typed starts one
/// of `text`'s words.
///
/// What a list marks when it matched words by their starts: "rate" on
/// "Repeat rate" marks the second word, where [`positions`] would mark the
/// first `r`, `a`, `t` and `e` it walks past.
pub fn word_positions(text: &str, query: &str) -> Option<Vec<usize>> {
    // Each word of `text`, lowered a character at a time so a lowered
    // character still knows where it came from.
    let mut words: Vec<Vec<(char, usize)>> = Vec::new();
    let mut word = Vec::new();
    for (at, c) in text.chars().enumerate() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase().map(|lower| (lower, at)));
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }

    let wanted: Vec<Vec<char>> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.chars().collect())
        .collect();
    if wanted.is_empty() {
        return None;
    }

    let starts = |word: &[(char, usize)], wanted: &[char]| {
        word.len() >= wanted.len() && word.iter().zip(wanted).all(|(&(c, _), &w)| c == w)
    };
    let mut used = vec![false; words.len()];
    let mut marked = Vec::new();
    for wanted in &wanted {
        // A word not already taken by an earlier one, if there is one.
        let found = (0..words.len())
            .find(|&i| !used[i] && starts(&words[i], wanted))
            .or_else(|| (0..words.len()).find(|&i| starts(&words[i], wanted)))?;
        used[found] = true;
        marked.extend(words[found][..wanted.len()].iter().map(|&(_, at)| at));
    }
    marked.sort_unstable();
    marked.dedup();
    Some(marked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_positions_mark_the_starts_of_the_words_typed() {
        assert_eq!(
            word_positions("Repeat rate", "rate"),
            Some(vec![7, 8, 9, 10])
        );
        assert_eq!(word_positions("Tap to click", "to"), Some(vec![4, 5]));
        assert_eq!(
            word_positions("Tap to click", "cl ta"),
            Some(vec![0, 1, 7, 8])
        );
        assert_eq!(word_positions("Dock size", "sz"), None);
        assert_eq!(word_positions("Dock size", " "), None);
    }

    #[test]
    fn positions_mark_the_characters_the_match_landed_on() {
        assert_eq!(
            positions("Firefox Developer", "fire dev"),
            Some(vec![0, 1, 2, 3, 8, 9, 10])
        );
        assert_eq!(positions("Firefox", "chrome"), None);
        assert_eq!(positions("Firefox", ""), Some(vec![]));
    }

    #[test]
    fn positions_and_score_agree_on_what_matches() {
        for (text, query) in [("Dock", "dk"), ("Dock", "kd"), ("Night Shift", "nsh")] {
            assert_eq!(
                positions(text, query).is_some(),
                score(text, query).is_some(),
                "{text} / {query}"
            );
        }
    }

    #[test]
    fn a_query_that_is_not_a_subsequence_does_not_match() {
        assert!(score("Firefox", "chrome").is_none());
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("Firefox", "FIRE").is_some());
    }

    #[test]
    fn a_prefix_beats_a_match_in_the_middle() {
        let prefix = score("Terminal", "term").unwrap();
        let middle = score("XTerminal", "term").unwrap();
        assert!(prefix > middle, "{prefix} should beat {middle}");
    }

    #[test]
    fn adjacent_characters_beat_scattered_ones() {
        let adjacent = score("gimp", "gim").unwrap();
        let scattered = score("go into map", "gim").unwrap();
        assert!(adjacent > scattered, "{adjacent} should beat {scattered}");
    }

    #[test]
    fn a_space_in_the_query_crosses_words() {
        assert!(score("Firefox Developer Edition", "fire dev").is_some());
    }
}
