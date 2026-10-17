//! The about pane: which Otto this is, and the machine it runs on.
//!
//! Nothing here is a setting. Every value is read when the pane is built,
//! from the files the kernel and the distribution keep for exactly this, so
//! there is no service to ask and nothing to go stale.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    let read = |path: &str| std::fs::read_to_string(path).ok();
    let facts = [
        (
            otto_kit::t!("settings-about-computer-name"),
            read_trimmed("/proc/sys/kernel/hostname"),
        ),
        (
            otto_kit::t!("settings-about-os"),
            read("/etc/os-release").and_then(|text| os_release_name(&text)),
        ),
        (
            otto_kit::t!("settings-about-kernel"),
            read_trimmed("/proc/sys/kernel/osrelease"),
        ),
        (
            otto_kit::t!("settings-about-processor"),
            read("/proc/cpuinfo").and_then(|text| cpu_model(&text)),
        ),
        (
            otto_kit::t!("settings-about-memory"),
            read("/proc/meminfo").and_then(|text| memory_total(&text)),
        ),
    ];
    // The value goes on the detail line under its label: a processor's full
    // name is longer than the room beside the label at the narrowest window.
    let machine: Vec<Row> = facts
        .into_iter()
        .filter_map(|(label, value)| {
            Some(Row::new(label, Control::Value(String::new())).detail(value?))
        })
        .collect();

    // The Otto mark, name and version are drawn above the groups by the
    // view; see `render_about_hero`.
    let mut groups = Vec::new();
    if !machine.is_empty() {
        groups.push(group(otto_kit::t!("settings-group-about-machine"), machine));
    }

    Pane {
        name: otto_kit::t!("settings-pane-about"),
        icon: "about",
        intro: None,
        groups,
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The distribution's own name for itself: `PRETTY_NAME`, falling back to
/// `NAME`, with the shell quoting os-release allows taken off.
fn os_release_name(text: &str) -> Option<String> {
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix('=')?;
            let value = value.trim().trim_matches(['"', '\'']);
            (!value.is_empty()).then(|| value.to_string())
        })
    };
    field("PRETTY_NAME").or_else(|| field("NAME"))
}

/// The processor's marketing name, taken from the first core: every core of
/// a desktop machine carries the same one.
fn cpu_model(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        let value = value.trim();
        (key.trim() == "model name" && !value.is_empty()).then(|| value.to_string())
    })
}

/// Installed memory in whole gigabytes. `MemTotal` is what the kernel kept
/// after firmware reservations, so it rounds up to what is fitted.
fn memory_total(text: &str) -> Option<String> {
    let kib: u64 = text.lines().find_map(|line| {
        line.strip_prefix("MemTotal:")?
            .trim()
            .strip_suffix("kB")?
            .trim()
            .parse()
            .ok()
    })?;
    let gib = (kib as f64 / (1024.0 * 1024.0)).ceil();
    Some(otto_kit::t_owned!("settings-about-memory-gb", size = gib))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_prefers_the_pretty_name_and_drops_quotes() {
        let text = "NAME=\"Manjaro Linux\"\nPRETTY_NAME=\"Manjaro Linux 25.0\"\nID=manjaro\n";
        assert_eq!(os_release_name(text).as_deref(), Some("Manjaro Linux 25.0"));
        assert_eq!(os_release_name("NAME=Arch\n").as_deref(), Some("Arch"));
        assert_eq!(os_release_name("ID=x\n"), None);
    }

    #[test]
    fn the_processor_is_the_first_model_name() {
        let text = "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i7\nprocessor\t: 1\n";
        assert_eq!(cpu_model(text).as_deref(), Some("Intel(R) Core(TM) i7"));
        assert_eq!(cpu_model("processor\t: 0\n"), None);
    }

    #[test]
    fn memory_rounds_up_to_what_is_fitted() {
        // 16 GiB fitted, a little under that reported.
        let text = "MemTotal:       16127384 kB\nMemFree: 1 kB\n";
        let size = memory_total(text).expect("parsed");
        assert!(size.contains("16"), "{size}");
        assert_eq!(memory_total("MemFree: 1 kB\n"), None);
    }
}
