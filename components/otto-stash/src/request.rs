//! What is stashed: selections read from the focused field and files handed
//! in, kept in order until they are sent to Ask.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

/// The text around the caret, as the focused field reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surrounding {
    pub text: String,
    /// Byte offset of the caret in `text`.
    pub cursor: u32,
    /// Byte offset of the other end of the selection; equal to `cursor` when
    /// nothing is selected.
    pub anchor: u32,
}

impl Surrounding {
    /// The selected text, or `None` when nothing is selected or the offsets
    /// don't fall on character boundaries.
    pub fn selection(&self) -> Option<&str> {
        let (start, end) = if self.cursor <= self.anchor {
            (self.cursor, self.anchor)
        } else {
            (self.anchor, self.cursor)
        };
        if start == end {
            return None;
        }
        self.text.get(start as usize..end as usize)
    }
}

/// One thing stashed: the attachment Ask shows it as.
pub use otto_kit::components::attachments::Attachment as Item;

/// The items stashed so far, oldest first.
#[derive(Debug, Default)]
pub struct Stash {
    pub items: Vec<Item>,
    /// Per item, whether it is struck out: kept on the card, but not sent.
    struck: Vec<bool>,
    /// Per item, the number its text is handed over under; 0 for a file.
    /// Fixed when the text is added, so its file keeps its name while other
    /// items come and go.
    text_numbers: Vec<u32>,
    texts_added: u32,
}

impl Stash {
    /// Add `item`, unless it is already stashed: the same text, file or
    /// region is stashed once. Adding again what was struck out brings it
    /// back. Returns whether the stash changed.
    pub fn add(&mut self, item: Item) -> bool {
        if let Some(index) = self.items.iter().position(|stashed| *stashed == item) {
            let struck = std::mem::take(&mut self.struck[index]);
            return struck;
        }
        let number = if matches!(item, Item::Text(_)) {
            self.texts_added += 1;
            self.texts_added
        } else {
            0
        };
        self.items.push(item);
        self.struck.push(false);
        self.text_numbers.push(number);
        true
    }

    /// Strike the item at `index` out, or bring it back.
    pub fn toggle(&mut self, index: usize) {
        if let Some(struck) = self.struck.get_mut(index) {
            *struck = !*struck;
        }
    }

    /// Whether the item at `index` is struck out.
    pub fn is_struck(&self, index: usize) -> bool {
        self.struck.get(index).copied().unwrap_or(false)
    }

    /// How many items will be sent.
    pub fn included(&self) -> usize {
        self.struck.iter().filter(|struck| !**struck).count()
    }

    /// Take the item at `index` out; nothing happens past the end.
    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.struck.remove(index);
            self.text_numbers.remove(index);
        }
    }

    /// Everything stashed, as files for Ask, each with whether it is
    /// struck out: text items in `dir` as `selection-N.txt`, written the
    /// first time they are handed over, files and regions as they are.
    ///
    /// # Errors
    ///
    /// When `dir` cannot be created or a text file cannot be written.
    pub fn hand_over(&self, dir: &Path) -> std::io::Result<Vec<(PathBuf, bool)>> {
        std::fs::create_dir_all(dir)?;
        self.items
            .iter()
            .zip(&self.struck)
            .zip(&self.text_numbers)
            .map(|((item, &struck), number)| {
                let path = match item {
                    Item::Text(text) => {
                        let path = dir.join(format!("selection-{number}.txt"));
                        if !path.exists() {
                            std::fs::write(&path, text)?;
                        }
                        path
                    }
                    Item::File(path) | Item::Region(path) => path.clone(),
                };
                Ok((path, struck))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str, cursor: u32, anchor: u32) -> Surrounding {
        Surrounding {
            text: text.into(),
            cursor,
            anchor,
        }
    }

    #[test]
    fn selection_works_in_either_direction() {
        assert_eq!(field("hello world", 6, 11).selection(), Some("world"));
        assert_eq!(field("hello world", 11, 6).selection(), Some("world"));
        assert_eq!(field("hello world", 3, 3).selection(), None);
    }

    #[test]
    fn selection_off_a_character_boundary_is_none() {
        // "è" is two bytes; offset 1 is inside it.
        assert_eq!(field("è ok", 1, 4).selection(), None);
        assert_eq!(field("è ok", 0, 2).selection(), Some("è"));
    }

    #[test]
    fn the_same_item_is_stashed_once() {
        let mut stash = Stash::default();
        assert!(stash.add(Item::Text("a".into())));
        assert!(!stash.add(Item::Text("a".into())));
        assert!(stash.add(Item::Text("b".into())));
        assert!(!stash.add(Item::Text("a".into())));
        assert_eq!(stash.items.len(), 2);
    }

    #[test]
    fn adding_a_struck_item_again_brings_it_back() {
        let mut stash = Stash::default();
        stash.add(Item::Text("a".into()));
        stash.toggle(0);
        assert!(stash.add(Item::Text("a".into())));
        assert!(!stash.is_struck(0));
        assert_eq!(stash.items.len(), 1);
    }

    #[test]
    fn text_items_become_files_in_order() {
        let dir = std::env::temp_dir().join(format!("otto-stash-test-{}", std::process::id()));
        let mut stash = Stash::default();
        stash.add(Item::Text("first".into()));
        stash.add(Item::File("/tmp/shot.png".into()));
        stash.add(Item::Text("second".into()));
        let files = stash.hand_over(&dir).unwrap();
        assert_eq!(
            files,
            [
                (dir.join("selection-1.txt"), false),
                (PathBuf::from("/tmp/shot.png"), false),
                (dir.join("selection-2.txt"), false),
            ]
        );
        assert_eq!(std::fs::read_to_string(&files[2].0).unwrap(), "second");
        // A text keeps its file when the items before it go.
        stash.remove(0);
        assert_eq!(
            stash.hand_over(&dir).unwrap()[1].0,
            dir.join("selection-2.txt")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn struck_items_are_handed_over_struck() {
        let dir = std::env::temp_dir().join(format!("otto-stash-strike-{}", std::process::id()));
        let mut stash = Stash::default();
        stash.add(Item::File("/tmp/a".into()));
        stash.add(Item::File("/tmp/b".into()));
        stash.add(Item::File("/tmp/c".into()));
        stash.toggle(1);
        assert_eq!(stash.included(), 2);
        assert_eq!(
            stash.hand_over(&dir).unwrap(),
            [
                (PathBuf::from("/tmp/a"), false),
                (PathBuf::from("/tmp/b"), true),
                (PathBuf::from("/tmp/c"), false),
            ]
        );
        // A second click brings it back.
        stash.toggle(1);
        assert_eq!(stash.included(), 3);
        // Removing keeps the marks on the items that stay.
        stash.toggle(2);
        stash.remove(0);
        assert!(!stash.is_struck(0));
        assert!(stash.is_struck(1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
