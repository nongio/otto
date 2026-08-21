//! Shell-style glob matching, shared by the MIME database and by the file
//! picker's portal filters.
//!
//! Supports the subset that actually appears in `globs2` and in portal filter
//! patterns: `*`, `?`, and `[...]` character classes with ranges and negation.
//! Deliberately not a full `fnmatch`: no `{a,b}` alternation, no backslash
//! escaping, and `*` matches `/` because these patterns are matched against a
//! single file name, never a path.

/// Does `text` match `pattern`?
///
/// Matching is byte-oriented over the pattern's structure but compares `char`s,
/// so a multi-byte character is one `?`.
pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    match_from(&p, 0, &t, 0)
}

/// Case-insensitive `matches`, folding both sides with ASCII case rules.
pub fn matches_ignore_case(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let t: Vec<char> = text.chars().map(|c| c.to_ascii_lowercase()).collect();
    match_from(&p, 0, &t, 0)
}

/// Iterative backtracking match. `*` remembers where it last consumed from, so
/// a pattern with several stars stays linear in practice rather than blowing up
/// the way naive recursion does on `*a*a*a*`.
fn match_from(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
    // Where to resume if the current attempt fails: the `*` we backtrack into,
    // and the text position it should consume one more character from.
    let mut star: Option<usize> = None;
    let mut star_ti = 0usize;

    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_ti = ti;
            pi += 1;
            continue;
        }
        if pi < p.len() && matches_one(p, pi, t[ti]) {
            pi = skip_one(p, pi);
            ti += 1;
            continue;
        }
        // Mismatch: backtrack into the last `*`, letting it swallow one more.
        match star {
            Some(s) => {
                pi = s + 1;
                star_ti += 1;
                ti = star_ti;
            }
            None => return false,
        }
    }

    // Text exhausted: the rest of the pattern must be all stars.
    p[pi.min(p.len())..].iter().all(|&c| c == '*')
}

/// Does the single pattern element starting at `pi` match `c`?
fn matches_one(p: &[char], pi: usize, c: char) -> bool {
    match p[pi] {
        '?' => true,
        '[' => match class_end(p, pi) {
            Some(end) => class_matches(&p[pi + 1..end], c),
            // An unterminated `[` is a literal `[`, as every shell does.
            None => c == '[',
        },
        lit => lit == c,
    }
}

/// Index just past the pattern element starting at `pi`.
fn skip_one(p: &[char], pi: usize) -> usize {
    if p[pi] == '[' {
        if let Some(end) = class_end(p, pi) {
            return end + 1;
        }
    }
    pi + 1
}

/// Index of the `]` closing the class opened at `pi`, if there is one.
///
/// A `]` immediately after the opening bracket (or after its `!`) is a literal
/// member rather than the terminator — `[]]` is the class containing `]`.
fn class_end(p: &[char], pi: usize) -> Option<usize> {
    let mut i = pi + 1;
    if i < p.len() && (p[i] == '!' || p[i] == '^') {
        i += 1;
    }
    if i < p.len() && p[i] == ']' {
        i += 1;
    }
    while i < p.len() {
        if p[i] == ']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Does `c` satisfy the class body (everything between the brackets)?
fn class_matches(body: &[char], c: char) -> bool {
    let (negated, body) = match body.first() {
        Some('!') | Some('^') => (true, &body[1..]),
        _ => (false, body),
    };

    let mut found = false;
    let mut i = 0;
    while i < body.len() {
        // A `-` that has a character either side is a range; one at either end
        // of the body is a literal `-`.
        if i + 2 < body.len() && body[i + 1] == '-' {
            if body[i] <= c && c <= body[i + 2] {
                found = true;
            }
            i += 3;
        } else {
            if body[i] == c {
                found = true;
            }
            i += 1;
        }
    }

    found != negated
}

/// Is this pattern a plain literal, with no glob metacharacters?
///
/// The MIME precedence rules rank a literal match above any glob, so this
/// decides which bucket a `globs2` pattern is filed under.
pub fn is_literal(pattern: &str) -> bool {
    !pattern.contains(['*', '?', '['])
}

/// If the pattern is exactly `*.ext` — by far the commonest shape in `globs2` —
/// return `ext` without its dot. Such patterns go in a hash map instead of a
/// linear scan.
pub fn simple_extension(pattern: &str) -> Option<&str> {
    let rest = pattern.strip_prefix("*.")?;
    is_literal(rest).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_wildcards() {
        assert!(matches("Makefile", "Makefile"));
        assert!(!matches("Makefile", "makefile"));
        assert!(matches("*.png", "photo.png"));
        assert!(!matches("*.png", "photo.png.bak"));
        assert!(matches("*", "anything"));
        assert!(matches("*", ""));
        assert!(matches("?.c", "a.c"));
        assert!(!matches("?.c", "ab.c"));
    }

    #[test]
    fn several_stars_do_not_blow_up() {
        // The shape that kills naive recursion.
        assert!(!matches(
            "*a*a*a*a*a*a*b",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(matches("*a*b*c*", "xxaxxbxxcxx"));
    }

    #[test]
    fn character_classes() {
        assert!(matches("*.[ch]", "main.c"));
        assert!(matches("*.[ch]", "main.h"));
        assert!(!matches("*.[ch]", "main.o"));
        assert!(matches("f[0-9].txt", "f7.txt"));
        assert!(!matches("f[0-9].txt", "fx.txt"));
        assert!(matches("f[!0-9].txt", "fx.txt"));
        assert!(!matches("f[!0-9].txt", "f7.txt"));
    }

    #[test]
    fn class_edge_cases() {
        // `]` as the first member is literal, not the terminator.
        assert!(matches("[]]", "]"));
        // A trailing `-` is a literal member.
        assert!(matches("[a-]", "-"));
        assert!(matches("[a-]", "a"));
        // An unterminated `[` is a literal bracket.
        assert!(matches("[abc", "[abc"));
    }

    #[test]
    fn case_folding_is_opt_in() {
        assert!(!matches("*.png", "PHOTO.PNG"));
        assert!(matches_ignore_case("*.png", "PHOTO.PNG"));
        assert!(matches_ignore_case("*.PNG", "photo.png"));
    }

    #[test]
    fn multibyte_is_one_character() {
        assert!(matches("?.txt", "é.txt"));
        assert!(matches_ignore_case("*.txt", "日本語.txt"));
    }

    #[test]
    fn classification_helpers() {
        assert!(is_literal("Makefile"));
        assert!(!is_literal("*.png"));
        assert_eq!(simple_extension("*.png"), Some("png"));
        assert_eq!(simple_extension("*.tar.gz"), Some("tar.gz"));
        assert_eq!(simple_extension("*.[ch]"), None);
        assert_eq!(simple_extension("Makefile"), None);
    }
}
