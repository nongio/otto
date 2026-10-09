//! Numbers as the desktop writes them for people.

/// A byte count, the way a file manager writes it: "999 bytes", "1.5 KB",
/// "15 KB", "2.0 MB".
///
/// Powers of 1000, so the units are the SI ones — the way GNOME Files and
/// macOS Finder count, and the way a disk's label does. Every app shows the
/// same size for the same file: Files, Peek, a preview card, an attachment.
///
/// Localised through the `size-*` catalogue keys. Under a kilobyte the count
/// is exact and needs a plural rule — one byte, two bytes, and whatever the
/// local grammar does with 2 and 5.
pub fn file_size(bytes: u64) -> String {
    if bytes < 1000 {
        return crate::t_owned!("size-bytes", count = bytes as f64);
    }
    let (value, unit) = scaled(bytes);
    // One decimal below ten, none above: the extra digit stops a 1 GB file and
    // a 9 GB file from looking the same, and is noise once the number is wide.
    let rendered = if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    crate::t_owned!(UNITS[unit], value = rendered)
}

const UNITS: [&str; 4] = ["size-kb", "size-mb", "size-gb", "size-tb"];

/// `bytes` (at least 1000) as a value and an index into [`UNITS`].
fn scaled(bytes: u64) -> (f64, usize) {
    // Divided once up front: anything reaching here is at least a kilobyte,
    // and UNITS starts at KB rather than at bytes, so the counter and the unit
    // it names stay in step.
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    (value, unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compared against the catalogue rather than against English prose.
    ///
    /// What this guards is the threshold each size crosses and how many
    /// decimals survive it — 1.5 KB rather than 1.5 kB, 15 KB rather than
    /// 15.0. The words around the number are the catalogue's business.
    #[test]
    fn sizes_read_the_way_a_file_manager_writes_them() {
        assert_eq!(file_size(0), crate::t_owned!("size-bytes", count = 0.0));
        assert_eq!(file_size(999), crate::t_owned!("size-bytes", count = 999.0));
        assert_eq!(file_size(1_500), crate::t_owned!("size-kb", value = "1.5"));
        assert_eq!(file_size(15_000), crate::t_owned!("size-kb", value = "15"));
        assert_eq!(
            file_size(2_000_000),
            crate::t_owned!("size-mb", value = "2.0")
        );
    }

    #[test]
    fn units_step_by_a_thousand_and_stop_at_terabytes() {
        assert_eq!(scaled(1_000), (1.0, 0));
        assert_eq!(scaled(1_000_000), (1.0, 1));
        assert_eq!(scaled(3_000_000_000), (3.0, 2));
        assert_eq!(scaled(5_000_000_000_000_000), (5000.0, 3));
    }

    #[test]
    fn every_unit_is_in_the_source_catalogue() {
        let keys = crate::i18n::source_keys();
        for key in UNITS.iter().chain(&["size-bytes"]) {
            assert!(keys.contains(*key), "{key} missing from en-GB.ftl");
        }
    }
}
