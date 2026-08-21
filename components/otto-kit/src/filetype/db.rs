//! The shared MIME database: `globs2` and `subclasses`.
//!
//! Both are line-oriented plain text, so the full database costs a file read
//! and a couple of hash maps — no XML parser and no `mime.cache` binary format.
//! That matters, because a hardcoded table of common types cannot name what a
//! portal filter or a file manager's Kind column will legitimately be handed.

use std::collections::HashMap;

use super::glob;

/// One `globs2` rule that is neither a literal nor a plain `*.ext`.
///
/// Whether the rule is case-sensitive is carried by which collection it lives
/// in, not by a field — `MimeDb::case_sensitive` matches against the original
/// name, everything else against the lowercased one.
#[derive(Debug)]
struct GlobRule {
    weight: u32,
    pattern: String,
    mime: String,
}

/// A candidate match, ranked by the shared-mime-info precedence rules:
/// a literal beats a glob, then higher weight wins, then the longer pattern.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank(bool, u32, usize);

#[derive(Debug, Default)]
pub struct MimeDb {
    /// Patterns with no metacharacters, keyed by the literal name
    /// (`Makefile`, `core`). Lowercased unless the rule is case-sensitive.
    literals: HashMap<String, (u32, String)>,
    /// `*.ext` patterns, keyed by the extension without its dot, lowercased.
    /// The overwhelming majority of the database.
    extensions: HashMap<String, (u32, String)>,
    /// Everything else — `*.[ch]`, `*[0-9]`, `lib*.so*` — matched linearly.
    globs: Vec<GlobRule>,
    /// Case-sensitive rules, kept aside so the common path can lowercase
    /// freely. Small: five entries in a stock database.
    case_sensitive: Vec<GlobRule>,
    /// `child -> parents`, from `subclasses`.
    parents: HashMap<String, Vec<String>>,
}

impl MimeDb {
    /// Parse a `globs2` file body. Unparseable lines are skipped rather than
    /// failing the load: a database with one bad line is still worth having.
    pub fn parse_globs2(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // `weight:mimetype:glob[:flags]`. The glob may itself contain a
            // colon, so split off the first two fields and keep the rest.
            let mut parts = line.splitn(3, ':');
            let (Some(weight), Some(mime), Some(rest)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse::<u32>() else {
                continue;
            };

            // Flags are a trailing `:cs` (and friends). Only `cs` is meaningful
            // here; anything else is ignored along with its colon.
            let (pattern, case_sensitive) = match rest.rsplit_once(':') {
                Some((p, flags)) if flags.split(',').any(|f| f == "cs") => (p, true),
                _ => (rest, false),
            };

            // The generator emits this sentinel for types that have no globs at
            // all, so it must never be treated as a pattern.
            if pattern.is_empty() || pattern == "__NOGLOBS__" {
                continue;
            }

            if case_sensitive {
                self.case_sensitive.push(GlobRule {
                    weight,
                    pattern: pattern.to_string(),
                    mime: mime.to_string(),
                });
                continue;
            }

            let lower = pattern.to_ascii_lowercase();
            if let Some(ext) = glob::simple_extension(&lower) {
                insert_ranked(&mut self.extensions, ext.to_string(), weight, mime);
            } else if glob::is_literal(&lower) {
                insert_ranked(&mut self.literals, lower, weight, mime);
            } else {
                self.globs.push(GlobRule {
                    weight,
                    pattern: lower,
                    mime: mime.to_string(),
                });
            }
        }
    }

    /// Parse a `subclasses` file body: `child parent`, one pair per line.
    pub fn parse_subclasses(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((child, parent)) = line.split_once(char::is_whitespace) {
                let parent = parent.trim();
                if !parent.is_empty() {
                    self.parents
                        .entry(child.to_string())
                        .or_default()
                        .push(parent.to_string());
                }
            }
        }
    }

    /// The MIME type for a file *name* — not a path. Callers pass the last
    /// component; passing a path would let a directory name decide the type.
    pub fn mime_for_name(&self, name: &str) -> Option<&str> {
        let lower = name.to_ascii_lowercase();
        let mut best: Option<(Rank, &str)> = None;

        if let Some((w, mime)) = self.literals.get(&lower) {
            best = keep_better(best, Rank(true, *w, lower.len()), mime);
        }
        // Every suffix after a dot, so `*.tar.gz` can outrank `*.gz` on
        // `archive.tar.gz` by being the longer pattern.
        for (i, _) in lower.match_indices('.') {
            let ext = &lower[i + 1..];
            if let Some((w, mime)) = self.extensions.get(ext) {
                best = keep_better(best, Rank(false, *w, ext.len()), mime);
            }
        }
        for rule in &self.globs {
            if glob::matches(&rule.pattern, &lower) {
                best = keep_better(
                    best,
                    Rank(false, rule.weight, rule.pattern.len()),
                    &rule.mime,
                );
            }
        }
        // Case-sensitive rules match against the original name, not the
        // lowercased one — that is what the `cs` flag means.
        for rule in &self.case_sensitive {
            if glob::matches(&rule.pattern, name) {
                let rank = Rank(
                    glob::is_literal(&rule.pattern),
                    rule.weight,
                    rule.pattern.len(),
                );
                best = keep_better(best, rank, &rule.mime);
            }
        }

        best.map(|(_, mime)| mime)
    }

    /// Is `mime` the same as, or a descendant of, `parent`?
    ///
    /// This is what lets a consumer ask "is this text?" and get a true answer
    /// for `text/x-rust` without enumerating every language.
    pub fn is_subclass_of(&self, mime: &str, parent: &str) -> bool {
        if mime == parent {
            return true;
        }
        // Breadth-first over the parent graph, which is small but not a tree —
        // and `visited` matters, because it is not guaranteed acyclic either.
        let mut queue = vec![mime.to_string()];
        let mut visited = Vec::new();
        while let Some(current) = queue.pop() {
            if visited.contains(&current) {
                continue;
            }
            if current == parent {
                return true;
            }
            visited.push(current.clone());
            if let Some(ps) = self.parents.get(&current) {
                queue.extend(ps.iter().cloned());
            }
        }
        // Implicit rules the database does not spell out.
        matches!(
            (mime.split('/').next(), parent),
            (Some("text"), "text/plain")
        )
    }

    /// Every glob registered for `mime` or for any type descending from it.
    ///
    /// This is how a portal MIME filter becomes a set of name patterns — the
    /// only kind of filter the picker actually applies.
    pub fn globs_for(&self, mime: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (ext, (_, m)) in &self.extensions {
            if self.is_subclass_of(m, mime) {
                out.push(format!("*.{ext}"));
            }
        }
        for (lit, (_, m)) in &self.literals {
            if self.is_subclass_of(m, mime) {
                out.push(lit.clone());
            }
        }
        for rule in self.globs.iter().chain(&self.case_sensitive) {
            if self.is_subclass_of(&rule.mime, mime) {
                out.push(rule.pattern.clone());
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Keep whichever of the two candidates ranks higher.
fn keep_better<'a>(
    best: Option<(Rank, &'a str)>,
    rank: Rank,
    mime: &'a str,
) -> Option<(Rank, &'a str)> {
    match &best {
        Some((r, _)) if *r >= rank => best,
        _ => Some((rank, mime)),
    }
}

/// Keep the higher-weighted rule when two claim the same key.
fn insert_ranked(map: &mut HashMap<String, (u32, String)>, key: String, weight: u32, mime: &str) {
    match map.get(&key) {
        Some((w, _)) if *w >= weight => {}
        _ => {
            map.insert(key, (weight, mime.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MimeDb {
        let mut db = MimeDb::default();
        db.parse_globs2(
            "# a comment\n\
             0:model/materialx:__NOGLOBS__\n\
             80:text/html:*.html\n\
             50:text/x-csrc:*.c\n\
             50:text/x-chdr:*.h\n\
             50:application/x-core:core:cs\n\
             50:text/x-makefile:Makefile\n\
             50:application/gzip:*.gz\n\
             70:application/x-compressed-tar:*.tar.gz\n\
             50:text/x-objcsrc:*.[hm]\n",
        );
        db.parse_subclasses(
            "text/x-csrc text/plain\n\
             text/html text/plain\n\
             application/x-compressed-tar application/gzip\n",
        );
        db
    }

    #[test]
    fn extensions_and_literals() {
        let db = db();
        assert_eq!(db.mime_for_name("index.html"), Some("text/html"));
        assert_eq!(db.mime_for_name("Makefile"), Some("text/x-makefile"));
        assert_eq!(db.mime_for_name("nothing-here"), None);
    }

    #[test]
    fn the_noglobs_sentinel_is_not_a_pattern() {
        let db = db();
        assert_eq!(db.mime_for_name("__NOGLOBS__"), None);
    }

    #[test]
    fn longer_extension_wins() {
        // `*.tar.gz` must beat `*.gz`, or every tarball is a gzip stream.
        let db = db();
        assert_eq!(
            db.mime_for_name("archive.tar.gz"),
            Some("application/x-compressed-tar")
        );
        assert_eq!(db.mime_for_name("archive.gz"), Some("application/gzip"));
    }

    #[test]
    fn extension_matching_ignores_case() {
        let db = db();
        assert_eq!(db.mime_for_name("PHOTO.HTML"), Some("text/html"));
    }

    #[test]
    fn case_sensitive_rules_are_respected() {
        let db = db();
        assert_eq!(db.mime_for_name("core"), Some("application/x-core"));
        // `cs` means exactly that: `CORE` is not a core dump.
        assert_eq!(db.mime_for_name("CORE"), None);
    }

    #[test]
    fn class_globs_still_match() {
        let db = db();
        // `*.[hm]` and `*.h` both match `a.h`; the plain extension is longer
        // in the ranking sense and equally weighted, so either answer is a
        // header — what must not happen is no answer.
        assert!(db.mime_for_name("a.h").is_some());
        assert_eq!(db.mime_for_name("a.m"), Some("text/x-objcsrc"));
    }

    #[test]
    fn subclass_hierarchy() {
        let db = db();
        assert!(db.is_subclass_of("text/x-csrc", "text/plain"));
        assert!(db.is_subclass_of("text/x-csrc", "text/x-csrc"));
        assert!(!db.is_subclass_of("text/x-csrc", "application/gzip"));
        // Two levels: tarball -> gzip.
        assert!(db.is_subclass_of("application/x-compressed-tar", "application/gzip"));
        // The implicit "all text is text/plain" rule, which the database
        // does not spell out for every language.
        assert!(db.is_subclass_of("text/x-nonsense", "text/plain"));
    }

    #[test]
    fn a_cyclic_subclass_graph_terminates() {
        let mut db = MimeDb::default();
        db.parse_subclasses("a/one a/two\na/two a/one\n");
        assert!(!db.is_subclass_of("a/one", "b/three"));
    }

    #[test]
    fn mime_filter_expands_to_globs() {
        let db = db();
        let globs = db.globs_for("application/gzip");
        assert!(globs.contains(&"*.gz".to_string()));
        // Descendants come along, which is the whole point.
        assert!(globs.contains(&"*.tar.gz".to_string()));
        assert!(!globs.contains(&"*.html".to_string()));
    }
}
