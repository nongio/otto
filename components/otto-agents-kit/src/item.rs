//! One row of a list, and how a query ranks the rows.
//!
//! The launcher lists apps, windows and agent sessions as [`Item`]s, and the
//! side canvas lists sessions the same way; [`rank`] orders them against what
//! was typed.

use otto_kit::matching::score;

/// One row: something that can be picked.
#[derive(Clone, Debug)]
pub struct Item {
    /// The line someone reads and types against.
    pub title: String,
    /// The dimmer second line — a comment, a window's app, a path.
    pub subtitle: Option<String>,
    /// Icon theme name, resolved by the view.
    pub icon: Option<String>,
    /// What the thing behind the row is doing, drawn as a small dot in the
    /// icon's place — an agent session at work, idle, or waiting on someone.
    pub activity: Option<Activity>,
    /// A box in the icon's place, ticked or not: an option of a question
    /// that takes several answers.
    pub checked: Option<bool>,
    /// Extra text that matches but is never shown: keywords, the binary name,
    /// the app id behind a window.
    pub search_terms: Vec<String>,
    /// Which source this came from, and its index there. The launcher hands
    /// this back to activate the item.
    pub origin: Origin,
}

/// What a row's item is doing. The view picks the colour from the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// Busy: the accent.
    Working,
    /// Nothing happening, or stopped: a faint gray.
    Idle,
    /// Blocked until someone answers: yellow.
    Waiting,
}

/// Where an item came from, so the right source is asked to act on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub source: usize,
    pub index: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str) -> Item {
        Item {
            title: title.to_string(),
            subtitle: None,
            icon: None,
            activity: None,
            checked: None,
            search_terms: Vec::new(),
            origin: Origin {
                source: 0,
                index: 0,
            },
        }
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
