//! Takes personal details out of report text before it is shown or written.
//!
//! The home folder becomes `~`, the user and host names become `<user>` and
//! `<host>`, and IPv4, IPv6 and MAC addresses become `<ipv4>`, `<ipv6>` and
//! `<mac>`. Loopback and unspecified addresses are kept: they say nothing
//! about the machine and often matter to a bug.

// Rust guideline compliant 2026-02-21

/// The personal details to look for in report text.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    home: Option<String>,
    user: Option<String>,
    host: Option<String>,
}

impl Redactor {
    /// A redactor for a home folder, a user name and a host name.
    ///
    /// An empty value, or a home folder of `/`, is not looked for.
    pub fn new(home: Option<String>, user: Option<String>, host: Option<String>) -> Self {
        let keep = |value: Option<String>| value.filter(|v| !v.is_empty());
        Self {
            home: keep(home)
                .map(|h| h.trim_end_matches('/').to_owned())
                .filter(|h| !h.is_empty()),
            user: keep(user),
            host: keep(host),
        }
    }

    /// `text` with the personal details replaced.
    pub fn redact(&self, text: &str) -> String {
        let mut out = match &self.home {
            Some(home) => replace_home(text, home),
            None => text.to_owned(),
        };
        // The host first: a host name often contains the user's name
        // ("ana-laptop"), and replacing the user first would leave
        // "<user>-laptop" behind.
        if let Some(host) = &self.host {
            out = replace_word(&out, host, "<host>");
        }
        if let Some(user) = &self.user {
            out = replace_word(&out, user, "<user>");
        }
        replace_addresses(&out)
    }
}

/// Replaces `home` with `~` wherever it is a whole path prefix.
fn replace_home(text: &str, home: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(home) {
        let after = &rest[at + home.len()..];
        let whole = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'));
        out.push_str(&rest[..at]);
        out.push_str(if whole { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Whether `c` continues a word for user and host name matching.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replaces every whole-word occurrence of `word` with `with`.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(at) = rest.find(word) {
        let before = rest[..at].chars().next_back().or(prev);
        let after = rest[at + word.len()..].chars().next();
        let whole = !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char);
        out.push_str(&rest[..at]);
        out.push_str(if whole { with } else { word });
        prev = word.chars().next_back();
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}

/// Whether `c` can be part of an address token.
fn is_address_char(c: char) -> bool {
    c.is_ascii_hexdigit() || c == ':' || c == '.'
}

/// Whether `c` glues a run of address characters to a longer word.
fn joins_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replaces IPv4, IPv6 and MAC addresses.
///
/// Scans for maximal runs of hex digits, colons and dots. A run with a letter,
/// digit or underscore right before or after it is part of a longer word and
/// stays as it is; any other run is classified.
fn replace_addresses(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if !is_address_char(c) {
            out.push(c);
            prev = Some(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let len = rest
            .find(|ch: char| !is_address_char(ch))
            .unwrap_or(rest.len());
        let (run, after) = rest.split_at(len);
        let glued = prev.is_some_and(joins_word) || after.chars().next().is_some_and(joins_word);
        if glued {
            out.push_str(run);
        } else {
            out.push_str(&redact_run(run));
        }
        prev = run.chars().next_back();
        rest = after;
    }
    out
}

/// What to do with a token that is an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Address {
    /// Loopback or unspecified: says nothing about the machine.
    Keep,
    /// Replace with this placeholder.
    Replace(&'static str),
}

/// A run of address characters, with any address in it replaced.
fn redact_run(run: &str) -> String {
    // Sentence punctuation after an address ("at 10.0.0.1.") is not part of it.
    let core = run.trim_end_matches(['.', ':']);
    let tail = &run[core.len()..];
    match classify(core) {
        Some(Address::Replace(placeholder)) => return format!("{placeholder}{tail}"),
        Some(Address::Keep) => return run.to_owned(),
        None => {}
    }
    // An IPv4 address with a port: "10.0.0.1:8080".
    if let Some((host, port)) = core.rsplit_once(':') {
        let is_port = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        if let (true, Some(Address::Replace(placeholder))) = (is_port, ipv4(host)) {
            return format!("{placeholder}:{port}{tail}");
        }
    }
    run.to_owned()
}

/// Whether, and how, `token` is an address as a whole.
fn classify(token: &str) -> Option<Address> {
    ipv4(token).or_else(|| mac(token)).or_else(|| ipv6(token))
}

/// Dotted-quad IPv4; loopback and `0.0.0.0` are kept.
fn ipv4(token: &str) -> Option<Address> {
    let octets = parse_ipv4(token)?;
    if octets[0] == 127 || octets == [0, 0, 0, 0] {
        return Some(Address::Keep);
    }
    Some(Address::Replace("<ipv4>"))
}

/// The four octets of a dotted-quad address.
fn parse_ipv4(token: &str) -> Option<[u8; 4]> {
    let mut octets = [0_u8; 4];
    let mut parts = token.split('.');
    for octet in &mut octets {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *octet = part.parse().ok()?;
    }
    parts.next().is_none().then_some(octets)
}

/// Six groups of two hex digits separated by colons; all zeros is kept.
fn mac(token: &str) -> Option<Address> {
    let groups: Vec<&str> = token.split(':').collect();
    let is_mac = groups.len() == 6
        && groups
            .iter()
            .all(|g| g.len() == 2 && g.bytes().all(|b| b.is_ascii_hexdigit()));
    if !is_mac {
        return None;
    }
    if groups.iter().all(|g| *g == "00") {
        return Some(Address::Keep);
    }
    Some(Address::Replace("<mac>"))
}

/// The fewest hex digits a compressed IPv6 address must have.
///
/// Two short groups around a `::` ("a::b") read more often as a code path
/// than an address, and say little about a machine either way.
const MIN_COMPRESSED_DIGITS: usize = 3;

/// IPv6: eight groups, or fewer around one `::`, optionally ending in a
/// dotted quad. `::` and `::1` are kept.
///
/// Clock times ("12:34:56") have neither eight groups nor a `::`, so they
/// are never taken for an address.
fn ipv6(token: &str) -> Option<Address> {
    if token == "::" || token == "::1" {
        return Some(Address::Keep);
    }
    let mut groups = 0;
    let mut digits = 0;
    let mut body = token;
    if let Some((rest, last)) = token.rsplit_once(':') {
        if last.contains('.') {
            parse_ipv4(last)?;
            groups += 2;
            digits += MIN_COMPRESSED_DIGITS;
            // Keep the "::" when the dotted quad follows it directly.
            body = if rest.ends_with(':') {
                &token[..=rest.len()]
            } else {
                rest
            };
        }
    }
    let hex_groups = |part: &str| -> Option<(usize, usize)> {
        if part.is_empty() {
            return Some((0, 0));
        }
        let mut count = (0, 0);
        for piece in part.split(':') {
            if piece.is_empty() || piece.len() > 4 || !piece.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return None;
            }
            count.0 += 1;
            count.1 += piece.len();
        }
        Some(count)
    };
    let compressed = body.matches("::").count();
    if compressed > 1 || body.contains(":::") {
        return None;
    }
    let valid = if let Some((head, tail)) = body.split_once("::") {
        let (head_groups, head_digits) = hex_groups(head)?;
        let (tail_groups, tail_digits) = hex_groups(tail)?;
        groups += head_groups + tail_groups;
        digits += head_digits + tail_digits;
        (2..8).contains(&groups) && digits >= MIN_COMPRESSED_DIGITS
    } else {
        if body.is_empty() {
            return None;
        }
        let (body_groups, _) = hex_groups(body)?;
        groups + body_groups == 8
    };
    valid.then_some(Address::Replace("<ipv6>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor() -> Redactor {
        Redactor::new(
            Some("/home/ana".into()),
            Some("ana".into()),
            Some("ana-laptop".into()),
        )
    }

    #[test]
    fn replaces_home_user_and_host() {
        let cases = [
            (
                "/home/ana/.config/otto/config.toml",
                "~/.config/otto/config.toml",
            ),
            ("cd /home/ana", "cd ~"),
            ("/home/anaconda/x", "/home/anaconda/x"),
            ("user=ana uid=1000", "user=<user> uid=1000"),
            ("banana and ana.", "banana and <user>."),
            ("ana@ana-laptop", "<user>@<host>"),
            ("hostname ana-laptop.local", "hostname <host>.local"),
        ];
        for (input, expected) in cases {
            assert_eq!(redactor().redact(input), expected, "input: {input}");
        }
    }

    #[test]
    fn replaces_addresses() {
        let cases = [
            ("inet 192.168.1.20/24 brd", "inet <ipv4>/24 brd"),
            (
                "connect to 10.0.0.1:8080 failed",
                "connect to <ipv4>:8080 failed",
            ),
            ("peer 8.8.8.8.", "peer <ipv4>."),
            ("ether 3c:22:fb:12:ab:cd txq", "ether <mac> txq"),
            ("inet6 fe80::1c2b:3aff:fe4d:5e6f/64", "inet6 <ipv6>/64"),
            ("2001:0db8:85a3:0000:0000:8a2e:0370:7334", "<ipv6>"),
            ("mapped ::ffff:192.0.2.128 ok", "mapped <ipv6> ok"),
        ];
        for (input, expected) in cases {
            assert_eq!(redactor().redact(input), expected, "input: {input}");
        }
    }

    #[test]
    fn leaves_lookalikes_alone() {
        let untouched = [
            "2026-10-18T12:34:56.789Z INFO otto::udev: frame",
            "Mesa 25.1.4, OpenGL ES 3.2",
            "listening on 127.0.0.1:631 and ::1",
            "bind 0.0.0.0 and ::",
            "crate::dec::face",
            "card0 vendor 0x8086 device 0x9a49",
            "sha 3f2a9c1d",
            "deadbeef:cafe",
            "999.1.1.1",
        ];
        for input in untouched {
            assert_eq!(redactor().redact(input), input);
        }
    }

    #[test]
    fn without_details_only_addresses_change() {
        let none = Redactor::new(None, Some(String::new()), None);
        assert_eq!(none.redact("/home/ana 10.1.2.3"), "/home/ana <ipv4>");
    }
}
