//! Ranking emoji against what has been typed.
//!
//! The vocabulary is small — a name, a subgroup, a group — so this is word
//! matching rather than fuzzy search: every word typed has to begin a word of
//! the name, or failing that appear somewhere in it. Fuzzy matching earns its
//! keep on file names and window titles; on "grinning face with big eyes" it
//! mostly finds things the user did not mean.

use crate::data::{Emoji, GROUPS};

/// How well an emoji matches. Lower is better; equal scores keep palette
/// order, which is Unicode's and is sensible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Score {
    /// The name is exactly what was typed.
    Exact,
    /// The name begins with it.
    Prefix,
    /// Every typed word begins a word of the name.
    WordPrefix,
    /// Every typed word is somewhere in the name.
    Substring,
    /// Every typed word is in the name, the subgroup or the group.
    Category,
}

fn score(emoji: &Emoji, query: &str, words: &[&str]) -> Option<Score> {
    let name = emoji.name;
    if name == query {
        return Some(Score::Exact);
    }
    if name.starts_with(query) {
        return Some(Score::Prefix);
    }
    let name_words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words
        .iter()
        .all(|word| name_words.iter().any(|w| w.starts_with(word)))
    {
        return Some(Score::WordPrefix);
    }
    if words.iter().all(|word| name.contains(word)) {
        return Some(Score::Substring);
    }
    let subgroup = emoji.subgroup;
    let group = GROUPS[emoji.group].source_name.to_ascii_lowercase();
    if words
        .iter()
        .all(|word| name.contains(word) || subgroup.contains(word) || group.contains(word))
    {
        return Some(Score::Category);
    }
    None
}

/// Indices into `emoji` of everything matching `query`, best first.
///
/// An empty query matches nothing: the caller shows the palette instead.
pub fn rank(emoji: &[Emoji], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut scored: Vec<(Score, usize)> = emoji
        .iter()
        .enumerate()
        .filter_map(|(index, emoji)| score(emoji, &query, &words).map(|score| (score, index)))
        .collect();
    // Stable, so equal scores stay in palette order.
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Table;

    fn names(table: &Table, query: &str) -> Vec<&'static str> {
        rank(&table.emoji, query)
            .into_iter()
            .map(|index| table.emoji[index].name)
            .collect()
    }

    #[test]
    fn an_exact_name_comes_first() {
        let table = Table::load();
        assert_eq!(names(&table, "red heart")[0], "red heart");
        assert_eq!(names(&table, "Red Heart")[0], "red heart");
    }

    #[test]
    fn a_word_beats_a_fragment() {
        let table = Table::load();
        let found = names(&table, "cat");
        // Whole-word "cat" matches ("cat face", "grinning cat") rank above
        // names that merely contain the letters ("delicatessen"?).
        let first_fragment = found
            .iter()
            .position(|name| !name.split(' ').any(|w| w.starts_with("cat")));
        let last_word = found
            .iter()
            .rposition(|name| name.split(' ').any(|w| w.starts_with("cat")));
        if let (Some(fragment), Some(word)) = (first_fragment, last_word) {
            assert!(word < fragment, "{found:?}");
        }
        assert!(found.contains(&"cat face"));
    }

    #[test]
    fn every_word_has_to_match() {
        let table = Table::load();
        let found = names(&table, "smiling cat");
        assert!(!found.is_empty());
        assert!(found
            .iter()
            .all(|name| name.contains("smiling") && name.contains("cat")));
    }

    #[test]
    fn a_group_name_finds_its_members() {
        let table = Table::load();
        let found = names(&table, "flags");
        assert!(found.len() > 200, "{}", found.len());
    }

    #[test]
    fn nothing_typed_is_nothing_found() {
        let table = Table::load();
        assert!(rank(&table.emoji, "   ").is_empty());
    }
}
