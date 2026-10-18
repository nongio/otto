//! The parts of a report, gathered from the system and the compositor.
//!
//! Every reader takes its root folder and file paths from [`Sources`], so
//! the tests run the same code over a fixture tree.

// Rust guideline compliant 2026-02-21

use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::doctor::compositor::DebugInfo;

/// Which part of the report a section is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionId {
    /// Distribution and kernel.
    Os,
    /// Graphics cards, kernel drivers and the renderer.
    Gpu,
    /// Outputs and their modes.
    Screens,
    /// Otto's version and backend.
    Otto,
    /// The configuration files.
    Config,
    /// The session logs and the panic log.
    Logs,
}

impl SectionId {
    /// Whether the section is short system facts, fit for the issue text.
    pub fn is_summary(self) -> bool {
        matches!(self, Self::Os | Self::Gpu | Self::Screens | Self::Otto)
    }
}

/// One part of the report: a heading, a one-line summary and a Markdown body.
#[derive(Debug, Clone)]
pub struct Section {
    /// Which section this is.
    pub id: SectionId,
    /// The heading.
    pub title: &'static str,
    /// One line, for the list and the issue text.
    pub summary: String,
    /// Markdown, exactly as the report carries it (before redaction).
    pub body: String,
    /// Whether it goes into the report.
    pub include: bool,
}

/// Where the sections read from.
#[derive(Debug, Clone)]
pub struct Sources {
    /// The file system root: `/`, or a fixture tree in tests.
    pub root: PathBuf,
    /// `$XDG_STATE_HOME/otto`, where the session and panic logs are.
    pub log_dir: Option<PathBuf>,
    /// Configuration files to include, lowest priority first.
    pub config_files: Vec<PathBuf>,
    /// Whether `config_files` came from the compositor rather than a guess.
    pub config_from_compositor: bool,
    /// What the compositor reported, or why it could not.
    pub compositor: Result<DebugInfo, String>,
    /// How many lines to keep from the end of each log.
    pub log_lines: usize,
}

/// The log files, with what each one is.
const LOGS: [(&str, &str); 3] = [
    ("session.log", "current session"),
    (
        "session.log.1",
        "previous session, usually the one that went wrong",
    ),
    ("panic.log", "every recorded crash"),
];

/// The most bytes read from the end of one log.
///
/// Logs at `debug` can run to gigabytes; the tail is what matters, and a
/// report much larger than this is one nobody reads or GitHub accepts.
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

/// Gathers every section, with the default selection.
///
/// `all` selects every section, as `--all` does.
pub fn gather(sources: &Sources, all: bool) -> Vec<Section> {
    let mut sections = vec![
        os(sources),
        gpu(sources),
        screens(sources),
        otto(sources),
        config(sources),
        logs(sources),
    ];
    if all {
        for section in &mut sections {
            section.include = true;
        }
    }
    sections
}

/// Distribution and kernel version.
fn os(sources: &Sources) -> Section {
    let release = read(&sources.root.join("etc/os-release"))
        .or_else(|| read(&sources.root.join("usr/lib/os-release")))
        .map(|text| os_release_name(&text))
        .unwrap_or_default();
    let distro = if release.is_empty() {
        "unknown distribution".to_owned()
    } else {
        release
    };
    let kernel = read(&sources.root.join("proc/sys/kernel/osrelease"))
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    Section {
        id: SectionId::Os,
        title: "Operating system",
        summary: format!("{distro}, kernel {kernel}"),
        body: format!("- Distribution: {distro}\n- Kernel: {kernel}\n"),
        include: true,
    }
}

/// `PRETTY_NAME` from os-release, or `NAME VERSION_ID`.
fn os_release_name(text: &str) -> String {
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix(key)?.strip_prefix('=')?;
            Some(unquote(value))
        })
    };
    if let Some(pretty) = field("PRETTY_NAME").filter(|p| !p.is_empty()) {
        return pretty;
    }
    let name = field("NAME").unwrap_or_default();
    let version = field("VERSION_ID").unwrap_or_default();
    format!("{name} {version}").trim().to_owned()
}

/// An os-release value without its shell quoting.
fn unquote(value: &str) -> String {
    let value = value.trim();
    let inner = ['"', '\'']
        .iter()
        .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A graphics card under `/sys/class/drm`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Card {
    name: String,
    vendor_id: String,
    device_id: String,
    driver: Option<String>,
    boot_vga: bool,
}

/// The cards in `root/sys/class/drm`, sorted by name.
fn cards(root: &Path) -> Vec<Card> {
    let drm = root.join("sys/class/drm");
    let Ok(entries) = fs::read_dir(&drm) else {
        return Vec::new();
    };
    let mut cards: Vec<Card> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.strip_prefix("card")?;
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let device = drm.join(&name).join("device");
            let id = |file: &str| {
                read(&device.join(file))
                    .map(|v| v.trim().trim_start_matches("0x").to_owned())
                    .unwrap_or_default()
            };
            let driver = fs::read_link(device.join("driver"))
                .ok()
                .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()));
            Some(Card {
                vendor_id: id("vendor"),
                device_id: id("device"),
                driver,
                boot_vga: read(&device.join("boot_vga")).is_some_and(|v| v.trim() == "1"),
                name,
            })
        })
        .collect();
    cards.sort_by(|a, b| a.name.cmp(&b.name));
    cards
}

/// The vendor behind a PCI vendor ID, for the common graphics vendors.
///
/// IDs from the PCI ID registry; an unknown one is shown as the bare ID.
fn vendor_name(id: &str) -> Option<&'static str> {
    match id {
        "8086" => Some("Intel"),
        "1002" => Some("AMD"),
        "10de" => Some("NVIDIA"),
        "1af4" => Some("virtio"),
        "15ad" => Some("VMware"),
        "1234" => Some("QEMU"),
        "1b36" => Some("Red Hat (QEMU)"),
        "80ee" => Some("VirtualBox"),
        _ => None,
    }
}

/// Graphics cards, kernel drivers and the renderer the session uses.
fn gpu(sources: &Sources) -> Section {
    let cards = cards(&sources.root);
    let mut body = String::new();
    let mut summary = Vec::new();
    if cards.is_empty() {
        body.push_str("- No cards under /sys/class/drm\n");
    }
    for card in &cards {
        let vendor = vendor_name(&card.vendor_id).unwrap_or("unknown vendor");
        let driver = card
            .driver
            .as_deref()
            .map_or_else(|| "no driver".to_owned(), |d| format!("driver {d}"));
        let primary = if card.boot_vga { ", boot VGA" } else { "" };
        let _ = writeln!(
            body,
            "- {}: {vendor} ({}:{}), {driver}{primary}",
            card.name, card.vendor_id, card.device_id
        );
        summary.push(match &card.driver {
            Some(d) => format!("{vendor} {d}"),
            None => vendor.to_owned(),
        });
    }
    match &sources.compositor {
        Ok(info) => {
            match &info.renderer {
                Some(r) => {
                    let api = match r.api.as_str() {
                        "gl" => "OpenGL",
                        "vulkan" => "Vulkan",
                        other => other,
                    };
                    let _ = writeln!(
                        body,
                        "- Renderer: {api}, {} ({}), {}",
                        r.renderer, r.vendor, r.version
                    );
                    summary.push(r.renderer.clone());
                }
                None => body.push_str("- Renderer: not reported by the compositor\n"),
            }
            if let Some(reason) = &info.vulkan_fallback {
                let _ = writeln!(body, "- Vulkan fell back to OpenGL: {reason}");
            }
        }
        Err(reason) => {
            let _ = writeln!(body, "- Renderer: compositor {reason}");
        }
    }
    summary.dedup();
    Section {
        id: SectionId::Gpu,
        title: "Graphics",
        summary: if summary.is_empty() {
            "no graphics card found".to_owned()
        } else {
            summary.join(" · ")
        },
        body,
        include: true,
    }
}

/// A scale without trailing zeros: `2`, `1.5`, `1.25`.
fn scale_text(scale: f64) -> String {
    let text = format!("{scale:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A refresh rate in hertz from millihertz: `60`, `59.95`.
fn refresh_text(mhz: i64) -> String {
    let text = format!("{:.2}", mhz as f64 / 1000.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Every output with its mode, refresh, scale and position.
fn screens(sources: &Sources) -> Section {
    let (summary, body) = match &sources.compositor {
        Ok(info) if info.outputs.is_empty() => (
            "no outputs".to_owned(),
            "- The compositor reports no outputs\n".to_owned(),
        ),
        Ok(info) => {
            let mut body = String::new();
            let mut summary = Vec::new();
            for o in &info.outputs {
                let refresh = refresh_text(o.refresh_mhz);
                let scale = scale_text(o.scale);
                let monitor = [o.make.as_str(), o.model.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let kind = if o.is_virtual { ", virtual" } else { "" };
                let monitor = if monitor.is_empty() {
                    String::new()
                } else {
                    format!(" ({monitor})")
                };
                let _ = writeln!(
                    body,
                    "- {}: {}×{} @ {refresh} Hz, scale {scale}, at {},{}{kind}{monitor}",
                    o.name, o.width, o.height, o.x, o.y
                );
                summary.push(format!(
                    "{} {}×{}@{refresh}Hz ×{scale}",
                    o.name, o.width, o.height
                ));
            }
            (summary.join(", "), body)
        }
        Err(reason) => (
            format!("compositor {reason}"),
            format!("- Compositor {reason}\n"),
        ),
    };
    Section {
        id: SectionId::Screens,
        title: "Screens",
        summary,
        body,
        include: true,
    }
}

/// Otto's version, commit and backend, and this tool's version.
fn otto(sources: &Sources) -> Section {
    let doctor = env!("CARGO_PKG_VERSION");
    let (summary, body) = match &sources.compositor {
        Ok(info) => {
            let commit = info
                .git_commit
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            let mut body = format!(
                "- Version: {}{commit}\n- Backend: {}\n- otto-msg: {doctor}\n",
                info.version, info.backend
            );
            if !info.config_files.is_empty() {
                body.push_str("- Configuration files read:\n");
                for file in &info.config_files {
                    let _ = writeln!(body, "  - {}", file.display());
                }
            }
            (
                format!("{}{commit} on {}", info.version, info.backend),
                body,
            )
        }
        Err(reason) => (
            format!("compositor {reason}"),
            format!("- Compositor {reason}\n- otto-msg: {doctor}\n"),
        ),
    };
    Section {
        id: SectionId::Otto,
        title: "Otto",
        summary,
        body,
        include: true,
    }
}

/// A Markdown code fence that `content` cannot close early.
fn fenced(content: &str, lang: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat((longest + 1).max(3));
    let newline = if content.ends_with('\n') { "" } else { "\n" };
    format!("{fence}{lang}\n{content}{newline}{fence}\n")
}

/// The configuration files, whole.
fn config(sources: &Sources) -> Section {
    let mut body = String::new();
    let mut found = 0;
    let mut lines = 0;
    for path in &sources.config_files {
        let Some(text) = read(path) else {
            continue;
        };
        found += 1;
        lines += text.lines().count();
        let _ = write!(
            body,
            "### {}\n\n{}\n",
            path.display(),
            fenced(&text, "toml")
        );
    }
    if found == 0 {
        body.push_str("No configuration files found; Otto runs on its defaults.\n");
    } else if !sources.config_from_compositor {
        body.insert_str(
            0,
            "The compositor did not list the files it reads, so these are the usual locations.\n\n",
        );
    }
    Section {
        id: SectionId::Config,
        title: "Configuration",
        summary: match found {
            0 => "no files".to_owned(),
            1 => format!("1 file, {}", line_count(lines)),
            n => format!("{n} files, {}", line_count(lines)),
        },
        body,
        include: false,
    }
}

/// The end of each session log and the panic log.
fn logs(sources: &Sources) -> Section {
    let mut body = String::new();
    let mut summary = Vec::new();
    for (name, what) in LOGS {
        let Some(path) = sources.log_dir.as_ref().map(|dir| dir.join(name)) else {
            continue;
        };
        let Some((text, kept, total)) = tail_lines(&path, sources.log_lines) else {
            continue;
        };
        let text = strip_ansi(&text);
        let extent = if kept < total {
            format!("last {kept} of {total} lines")
        } else {
            line_count(kept)
        };
        let _ = write!(
            body,
            "### {name} ({what})\n\n{extent}\n\n{}\n",
            fenced(&text, "text")
        );
        summary.push(format!("{name} {}", line_count(kept)));
    }
    if summary.is_empty() {
        body.push_str("No session or panic logs found.\n");
    }
    Section {
        id: SectionId::Logs,
        title: "Session logs",
        summary: if summary.is_empty() {
            "no logs".to_owned()
        } else {
            summary.join(", ")
        },
        body,
        include: false,
    }
}

/// "1 line" or "`n` lines".
fn line_count(n: usize) -> String {
    if n == 1 {
        "1 line".to_owned()
    } else {
        format!("{n} lines")
    }
}

/// The last `n` lines of `path`: the text, how many lines it holds, and how
/// many the file has in all (counting only the part read, past
/// [`MAX_LOG_BYTES`]).
fn tail_lines(path: &Path, n: usize) -> Option<(String, usize, usize)> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(MAX_LOG_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        // The first line read is cut off part way.
        all.remove(0);
    }
    let total = all.len();
    let kept = &all[total.saturating_sub(n)..];
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Some((out, kept.len(), total))
}

/// `text` without terminal escape sequences (colours, cursor moves).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            // CSI: parameters and intermediates, then one final byte.
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ST.
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// The file at `path` as text, if it can be read.
fn read(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::compositor::{Output, Renderer};

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// A fixture tree with an Intel card, a second card with no driver, and
    /// connector entries that are not cards.
    fn fixture_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "etc/os-release",
            "NAME=\"Arch\"\nPRETTY_NAME=\"Arch \\\"Rolling\\\" Linux\"\nID=arch\n",
        );
        write(root, "proc/sys/kernel/osrelease", "6.12.1-arch1\n");
        write(root, "sys/class/drm/card0/device/vendor", "0x8086\n");
        write(root, "sys/class/drm/card0/device/device", "0x9a49\n");
        write(root, "sys/class/drm/card0/device/boot_vga", "1\n");
        fs::create_dir_all(root.join("sys/bus/pci/drivers/i915")).unwrap();
        std::os::unix::fs::symlink(
            root.join("sys/bus/pci/drivers/i915"),
            root.join("sys/class/drm/card0/device/driver"),
        )
        .unwrap();
        write(root, "sys/class/drm/card1/device/vendor", "0x1af4\n");
        write(root, "sys/class/drm/card1/device/device", "0x1050\n");
        write(root, "sys/class/drm/card0-eDP-1/status", "connected\n");
        write(root, "sys/class/drm/renderD128/dev", "226:128\n");
        dir
    }

    fn sources(root: &Path, compositor: Result<DebugInfo, String>) -> Sources {
        Sources {
            root: root.to_owned(),
            log_dir: Some(root.join("state/otto")),
            config_files: vec![root.join("config/otto/config.toml")],
            config_from_compositor: compositor.is_ok(),
            compositor,
            log_lines: 3,
        }
    }

    fn running() -> DebugInfo {
        DebugInfo {
            version: "1.8.0".into(),
            git_commit: Some("abc1234".into()),
            backend: "udev".into(),
            renderer: Some(Renderer {
                api: "gl".into(),
                vendor: "Intel".into(),
                renderer: "Mesa Intel(R) Xe Graphics (TGL GT2)".into(),
                version: "OpenGL ES 3.2 Mesa 25.1.4".into(),
            }),
            vulkan_fallback: None,
            config_files: vec![],
            outputs: vec![Output {
                name: "eDP-1".into(),
                make: "BOE".into(),
                model: "NE135A1M".into(),
                width: 2880,
                height: 1920,
                refresh_mhz: 120_000,
                scale: 2.0,
                x: 0,
                y: 0,
                is_virtual: false,
            }],
        }
    }

    fn section(sections: &[Section], id: SectionId) -> &Section {
        sections.iter().find(|s| s.id == id).unwrap()
    }

    #[test]
    fn reads_os_and_cards_from_the_root() {
        let dir = fixture_root();
        let sections = gather(&sources(dir.path(), Ok(running())), false);

        let os = section(&sections, SectionId::Os);
        assert_eq!(os.summary, "Arch \"Rolling\" Linux, kernel 6.12.1-arch1");

        let gpu = section(&sections, SectionId::Gpu);
        assert_eq!(
            gpu.body,
            "- card0: Intel (8086:9a49), driver i915, boot VGA\n\
             - card1: virtio (1af4:1050), no driver\n\
             - Renderer: OpenGL, Mesa Intel(R) Xe Graphics (TGL GT2) (Intel), \
             OpenGL ES 3.2 Mesa 25.1.4\n"
        );
        assert_eq!(
            gpu.summary,
            "Intel i915 · virtio · Mesa Intel(R) Xe Graphics (TGL GT2)"
        );
    }

    #[test]
    fn falls_back_to_name_and_version_without_pretty_name() {
        assert_eq!(
            os_release_name("NAME='Debian GNU/Linux'\nVERSION_ID=\"13\"\n"),
            "Debian GNU/Linux 13"
        );
    }

    #[test]
    fn screens_and_otto_come_from_the_compositor() {
        let dir = fixture_root();
        let mut info = running();
        info.outputs.push(Output {
            name: "Virtual-1".into(),
            make: String::new(),
            model: String::new(),
            width: 1920,
            height: 1080,
            refresh_mhz: 59_940,
            scale: 1.25,
            x: 1440,
            y: 0,
            is_virtual: true,
        });
        let sections = gather(&sources(dir.path(), Ok(info)), false);

        let screens = section(&sections, SectionId::Screens);
        assert_eq!(
            screens.body,
            "- eDP-1: 2880×1920 @ 120 Hz, scale 2, at 0,0 (BOE NE135A1M)\n\
             - Virtual-1: 1920×1080 @ 59.94 Hz, scale 1.25, at 1440,0, virtual\n"
        );
        let otto = section(&sections, SectionId::Otto);
        assert_eq!(otto.summary, "1.8.0 (abc1234) on udev");
    }

    #[test]
    fn says_so_when_the_compositor_is_not_reachable() {
        let dir = fixture_root();
        let reason = "not reachable (no owner)".to_owned();
        let sections = gather(&sources(dir.path(), Err(reason)), false);

        for id in [SectionId::Screens, SectionId::Otto] {
            assert_eq!(
                section(&sections, id).summary,
                "compositor not reachable (no owner)"
            );
        }
        assert!(section(&sections, SectionId::Gpu)
            .body
            .contains("- Renderer: compositor not reachable (no owner)\n"));
        assert!(section(&sections, SectionId::Gpu).body.contains("card0"));
    }

    #[test]
    fn default_selection_leaves_out_config_and_logs() {
        let dir = fixture_root();
        let default = gather(&sources(dir.path(), Ok(running())), false);
        let chosen: Vec<SectionId> = default.iter().filter(|s| s.include).map(|s| s.id).collect();
        assert_eq!(
            chosen,
            [
                SectionId::Os,
                SectionId::Gpu,
                SectionId::Screens,
                SectionId::Otto
            ]
        );
        let all = gather(&sources(dir.path(), Ok(running())), true);
        assert!(all.iter().all(|s| s.include));
    }

    #[test]
    fn config_is_fenced_so_its_backticks_cannot_close_the_block() {
        let dir = fixture_root();
        write(
            dir.path(),
            "config/otto/config.toml",
            "# ```not a fence```\ndock_size = 48\n",
        );
        let sections = gather(&sources(dir.path(), Ok(running())), false);
        let config = section(&sections, SectionId::Config);
        assert_eq!(config.summary, "1 file, 2 lines");
        let path = dir.path().join("config/otto/config.toml");
        assert_eq!(
            config.body,
            format!(
                "### {}\n\n````toml\n# ```not a fence```\ndock_size = 48\n````\n\n",
                path.display()
            )
        );
    }

    #[test]
    fn logs_keep_the_last_lines_without_colours() {
        let dir = fixture_root();
        write(
            dir.path(),
            "state/otto/session.log.1",
            "one\ntwo\n\u{1b}[32mINFO\u{1b}[0m three\nfour\nfive\n",
        );
        write(dir.path(), "state/otto/panic.log", "=== otto panic\n");
        let sections = gather(&sources(dir.path(), Ok(running())), false);
        let logs = section(&sections, SectionId::Logs);

        assert_eq!(logs.summary, "session.log.1 3 lines, panic.log 1 line");
        assert!(logs
            .body
            .contains("last 3 of 5 lines\n\n```text\nINFO three\nfour\nfive\n```\n"));
        assert!(!logs.body.contains("two"));
        assert!(!logs.body.contains("### session.log ("));
    }

    #[test]
    fn a_log_larger_than_the_read_limit_drops_its_cut_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let line = "x".repeat(99) + "\n";
        let count = (MAX_LOG_BYTES as usize / line.len()) + 10;
        fs::write(&path, line.repeat(count)).unwrap();

        let (text, kept, total) = tail_lines(&path, usize::MAX).unwrap();
        assert!(total < count, "only the tail was read");
        assert_eq!(kept, total);
        assert!(text.lines().all(|l| l.len() == 99), "no partial line");
    }
}
