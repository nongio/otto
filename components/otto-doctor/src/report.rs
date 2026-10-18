//! The Markdown report, its file name, and the pre-filled issue link.

// Rust guideline compliant 2026-02-21

use std::fmt::Write as _;

use crate::redact::Redactor;
use crate::sections::Section;

/// Where a new issue is opened.
pub const NEW_ISSUE_URL: &str = "https://github.com/nongio/otto/issues/new";

/// The longest issue link we hand out.
///
/// Browsers and GitHub accept longer URLs, but a pre-filled body past a few
/// thousand characters is cut off or rejected depending on the browser, so
/// the issue text carries only the summary and the report is attached.
const MAX_ISSUE_URL: usize = 6000;

/// The longest summary line put into the issue text.
const MAX_SUMMARY_CHARS: usize = 200;

/// A local wall-clock time, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// Four-digit year.
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    /// 0 to 23.
    pub hour: u32,
    /// 0 to 59.
    pub minute: u32,
    /// 0 to 60.
    pub second: u32,
}

impl Timestamp {
    /// The current local time, or UTC if the time zone cannot be read.
    pub fn now() -> Self {
        // SAFETY: `time` with a null pointer only returns the time.
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        // SAFETY: an all-zero `tm` is a valid value for `localtime_r` to
        // overwrite; it writes only into the buffer it is given.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers point to live values of the right type.
        let ok = !unsafe { libc::localtime_r(&now, &mut tm) }.is_null();
        if !ok {
            // SAFETY: as above, for the UTC variant.
            unsafe { libc::gmtime_r(&now, &mut tm) };
        }
        let field = |v: libc::c_int| u32::try_from(v).unwrap_or_default();
        Self {
            year: tm.tm_year + 1900,
            month: field(tm.tm_mon + 1),
            day: field(tm.tm_mday),
            hour: field(tm.tm_hour),
            minute: field(tm.tm_min),
            second: field(tm.tm_sec),
        }
    }

    /// `otto-report-YYYYMMDD-HHMMSS.md`.
    pub fn file_name(self) -> String {
        format!(
            "otto-report-{:04}{:02}{:02}-{:02}{:02}{:02}.md",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `YYYY-MM-DD HH:MM:SS`.
    pub fn display(self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// How the report treats personal details.
#[derive(Debug, Clone, Copy)]
pub enum Redaction<'a> {
    /// Replace them with this redactor.
    On(&'a Redactor),
    /// Keep the text as gathered.
    Off,
}

impl Redaction<'_> {
    /// `text`, redacted when redaction is on.
    pub fn apply(&self, text: &str) -> String {
        match self {
            Redaction::On(redactor) => redactor.redact(text),
            Redaction::Off => text.to_owned(),
        }
    }
}

/// One section as it appears in the report: heading and body.
pub fn render_section(section: &Section, redaction: Redaction<'_>) -> String {
    format!("## {}\n\n{}", section.title, redaction.apply(&section.body))
}

/// The whole report: a header, then every included section.
pub fn render(sections: &[Section], redaction: Redaction<'_>, when: Timestamp) -> String {
    let mut out = String::from("# Otto bug report\n\n");
    let _ = writeln!(
        out,
        "Written by otto-doctor {} on {}.",
        env!("CARGO_PKG_VERSION"),
        when.display()
    );
    out.push_str(match redaction {
        Redaction::On(_) => {
            "Personal details are redacted: the home folder, user and host names, \
             and IP and MAC addresses.\n"
        }
        Redaction::Off => "Nothing is redacted.\n",
    });
    for section in sections.iter().filter(|s| s.include) {
        out.push('\n');
        out.push_str(&render_section(section, redaction));
    }
    out
}

/// Percent-encodes `text` for a URL query value.
fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The text a new issue opens with: a prompt to describe the bug, the summary
/// of the included system sections, and a reminder to attach the report.
fn issue_body(summaries: &[String], file_name: &str) -> String {
    let mut body = String::from(
        "**What happened**\n\n<!-- What you did, what you saw, and what you expected. -->\n\n\
         **Steps to reproduce**\n\n1. \n\n**System**\n\n",
    );
    for line in summaries {
        let _ = writeln!(body, "- {line}");
    }
    let _ = write!(
        body,
        "\nThe full report is {file_name}, written by otto-doctor. \
         Please attach it here (drag the file into this box)."
    );
    body
}

/// A link that opens a new issue with the summary filled in.
///
/// Only the short system sections go into the link, and summary lines are
/// dropped from the end until it fits [`MAX_ISSUE_URL`].
pub fn issue_url(sections: &[Section], redaction: Redaction<'_>, file_name: &str) -> String {
    let mut summaries: Vec<String> = sections
        .iter()
        .filter(|s| s.include && s.id.is_summary())
        .map(|s| {
            let line = redaction.apply(&format!("{}: {}", s.title, s.summary));
            match line.char_indices().nth(MAX_SUMMARY_CHARS) {
                Some((at, _)) => format!("{}…", &line[..at]),
                None => line,
            }
        })
        .collect();
    loop {
        let url = format!(
            "{NEW_ISSUE_URL}?body={}",
            url_encode(&issue_body(&summaries, file_name))
        );
        if url.len() <= MAX_ISSUE_URL || summaries.pop().is_none() {
            return url;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sections::SectionId;

    fn section(id: SectionId, title: &'static str, summary: &str, include: bool) -> Section {
        Section {
            id,
            title,
            summary: summary.into(),
            body: format!("- {summary} at /home/ana\n"),
            include,
        }
    }

    fn sections() -> Vec<Section> {
        vec![
            section(SectionId::Os, "Operating system", "Arch Linux", true),
            section(SectionId::Otto, "Otto", "1.8.0 on ana-box", true),
            section(SectionId::Config, "Configuration", "1 file", false),
            section(SectionId::Logs, "Session logs", "session.log 5 lines", true),
        ]
    }

    const WHEN: Timestamp = Timestamp {
        year: 2026,
        month: 3,
        day: 7,
        hour: 9,
        minute: 5,
        second: 2,
    };

    #[test]
    fn file_name_is_sortable_by_time() {
        assert_eq!(WHEN.file_name(), "otto-report-20260307-090502.md");
    }

    #[test]
    fn report_holds_the_included_sections_redacted() {
        let redactor = Redactor::new(Some("/home/ana".into()), None, Some("ana-box".into()));
        let report = render(&sections(), Redaction::On(&redactor), WHEN);

        assert!(report.starts_with("# Otto bug report\n\n"));
        assert!(report.contains("on 2026-03-07 09:05:02."));
        assert!(report.contains("## Operating system\n\n- Arch Linux at ~\n"));
        assert!(report.contains("## Otto\n\n- 1.8.0 on <host> at ~\n"));
        assert!(report.contains("## Session logs\n"));
        assert!(!report.contains("## Configuration"));
        assert!(!report.contains("/home/ana"));

        let plain = render(&sections(), Redaction::Off, WHEN);
        assert!(plain.contains("- 1.8.0 on ana-box at /home/ana\n"));
    }

    #[test]
    fn issue_link_carries_only_the_system_summary() {
        let url = issue_url(&sections(), Redaction::Off, "otto-report-x.md");
        let body = url
            .strip_prefix("https://github.com/nongio/otto/issues/new?body=")
            .unwrap();
        assert!(body.contains("-%20Operating%20system%3A%20Arch%20Linux%0A"));
        assert!(body.contains("-%20Otto%3A%201.8.0%20on%20ana-box%0A"));
        assert!(!body.contains("Session%20logs"), "logs stay in the file");
        assert!(body.contains("otto-report-x.md"));
    }

    #[test]
    fn issue_link_stays_under_the_limit() {
        let long = "é".repeat(5000);
        let many: Vec<Section> = (0..20)
            .map(|_| section(SectionId::Gpu, "Graphics", &long, true))
            .collect();
        let url = issue_url(&many, Redaction::Off, "r.md");
        assert!(url.len() <= MAX_ISSUE_URL, "{} bytes", url.len());
        assert!(url.contains("r.md"), "the attach reminder survives");
    }
}
