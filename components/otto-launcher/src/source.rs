//! What the launcher can offer, and how a choice is carried out.
//!
//! A source is a list of [`Item`]s plus the meaning of activating one. The
//! launcher itself only knows how to filter and draw them, so adding files —
//! or clipboard history, or a calculator — is a matter of another [`Source`],
//! not another launcher.
//!
//! Sources are asked for their items once, when the launcher opens. Nothing
//! here does I/O while someone is typing: a keystroke must never wait on a
//! disk scan.

/// One row: something that can be picked.
#[derive(Clone, Debug)]
pub struct Item {
    /// The line someone reads and types against.
    pub title: String,
    /// The dimmer second line — a comment, a window's app, a path.
    pub subtitle: Option<String>,
    /// Icon theme name, resolved by the view.
    pub icon: Option<String>,
    /// Extra text that matches but is never shown: keywords, the binary name,
    /// the app id behind a window.
    pub search_terms: Vec<String>,
    /// Which source this came from, and its index there. The launcher hands
    /// this back to activate the item.
    pub origin: Origin,
}

/// Where an item came from, so the right source is asked to act on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub source: usize,
    pub index: usize,
}

/// A provider of items.
pub trait Source {
    /// Short name, shown as the row's badge — "App", "Window".
    fn label(&self) -> &'static str;

    /// The items as they stand now. Called when the launcher opens and
    /// whenever the source says it has changed.
    fn items(&mut self) -> Vec<Item>;

    /// Do whatever picking `index` means. Returning `Ok(())` closes the
    /// launcher.
    fn activate(&mut self, index: usize) -> Result<(), String>;

    /// What to show before anything has been typed.
    ///
    /// Opening the launcher onto every application installed is a wall of
    /// names nobody reads. A source that has a shorter answer to "what did you
    /// want?" gives it here; the default is everything it has, which is right
    /// for a list that is already short.
    fn resting(&mut self) -> Vec<Item> {
        self.items()
    }

    /// An item derived from the query itself, shown first and never ranked.
    ///
    /// Ranking compares a query against the items it might have meant, which
    /// is the wrong shape for a source whose item *is* the query worked out:
    /// the answer to `24.5*3` is `73.5`, and nothing about `73.5` matches what
    /// was typed. An answer is pinned above the matches instead.
    fn answer(&mut self, _query: &str) -> Option<Item> {
        None
    }

    /// Whether the item list has changed since it was last read — a window
    /// opening or closing while the launcher is up.
    fn changed(&mut self) -> bool {
        false
    }

    /// A file descriptor the launcher should wake on, for a source that has
    /// somewhere else to listen. Handed to
    /// [`App::poll_fds`](otto_kit::App::poll_fds).
    fn poll_fd(&self) -> Option<std::os::fd::RawFd> {
        None
    }

    /// Read whatever is waiting on [`Source::poll_fd`]. Called every loop
    /// iteration, so it must not block.
    fn pump(&mut self) {}
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// A matched item, with the score that ordered it.
#[derive(Clone, Copy, Debug)]
pub struct Match {
    pub index: usize,
    pub score: i32,
}

/// Rank `items` against `query`, best first.
///
/// An empty query keeps everything in the order the sources gave it, which is
/// the order someone browsing with the arrow keys expects.
pub fn rank(items: &[Item], query: &str) -> Vec<Match> {
    if query.trim().is_empty() {
        return items
            .iter()
            .enumerate()
            .map(|(index, _)| Match { index, score: 0 })
            .collect();
    }

    let query = query.trim();
    let mut matches: Vec<Match> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            // The title is what someone is aiming at; everything else is a way
            // of still finding the item when they aimed at something adjacent,
            // so it scores lower and can never outrank a title hit.
            let title = score(&item.title, query);
            let secondary = item
                .subtitle
                .iter()
                .map(String::as_str)
                .chain(item.search_terms.iter().map(String::as_str))
                .filter_map(|text| score(text, query))
                .max()
                .map(|score| score / 2 - 20);

            let best = match (title, secondary) {
                (Some(a), Some(b)) => a.max(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => return None,
            };
            Some(Match { index, score: best })
        })
        .collect();

    // Ties keep source order — a stable sort, so an unscored browse and a
    // fully tied query look the same.
    matches.sort_by_key(|m| std::cmp::Reverse(m.score));
    matches
}

/// Score `query` as a subsequence of `text`, or `None` if it is not one.
///
/// The shape of the score is what makes a launcher feel right: a match at the
/// start of a word beats one in the middle, a run of adjacent characters beats
/// the same characters scattered, and a short name beats a long one that
/// happens to contain the same letters.
fn score(text: &str, query: &str) -> Option<i32> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str) -> Item {
        Item {
            title: title.to_string(),
            subtitle: None,
            icon: None,
            search_terms: Vec::new(),
            origin: Origin {
                source: 0,
                index: 0,
            },
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

    #[test]
    fn the_shorter_of_two_matching_names_wins() {
        let items = [item("Files"), item("Files Preferences Dialog")];
        let ranked = rank(&items, "files");
        assert_eq!(ranked[0].index, 0);
    }

    #[test]
    fn an_empty_query_keeps_every_item_in_order() {
        let items = [item("b"), item("a")];
        let ranked = rank(&items, "  ");
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].index, 0);
    }

    #[test]
    fn a_title_hit_outranks_a_keyword_hit() {
        let mut keyworded = item("Zed");
        keyworded.search_terms = vec!["terminal".to_string()];
        let items = [item("Terminal"), keyworded];
        let ranked = rank(&items, "terminal");
        assert_eq!(ranked[0].index, 0);
    }
}
