use xdg_desktop_portal_otto::portal::{
    fallback_mapping_id, make_output_mapping_id, validate_cursor_mode, validate_persist_mode,
    CURSOR_MODE_EMBEDDED, CURSOR_MODE_HIDDEN, CURSOR_MODE_METADATA,
};
use zbus::DBusError;

#[test]
fn cursor_mode_accepts_supported_values() {
    assert_eq!(
        validate_cursor_mode(CURSOR_MODE_HIDDEN).unwrap(),
        CURSOR_MODE_HIDDEN
    );
    assert_eq!(
        validate_cursor_mode(CURSOR_MODE_EMBEDDED).unwrap(),
        CURSOR_MODE_EMBEDDED
    );
}

/// `AvailableCursorModes` is `HIDDEN | EMBEDDED` — metadata cursors are not
/// implemented, so asking for one is out of contract, not a silent downgrade.
/// See `specs/screenshare.md` (Portal implementation properties and cursor
/// modes).
#[test]
fn cursor_mode_rejects_the_unadvertised_metadata_mode() {
    let err = validate_cursor_mode(CURSOR_MODE_METADATA)
        .expect_err("metadata cursor mode is not advertised");
    assert_eq!(
        err.name().as_str(),
        "org.freedesktop.DBus.Error.InvalidArgs"
    );
}

#[test]
fn cursor_mode_rejects_unknown_bits() {
    let err = validate_cursor_mode(8).expect_err("expected invalid cursor mode");
    assert_eq!(
        err.name().as_str(),
        "org.freedesktop.DBus.Error.InvalidArgs"
    );
}

#[test]
fn persist_mode_accepts_spec_values() {
    assert_eq!(validate_persist_mode(0).unwrap(), 0);
    assert_eq!(validate_persist_mode(1).unwrap(), 1);
    assert_eq!(validate_persist_mode(2).unwrap(), 2);
}

#[test]
fn persist_mode_rejects_out_of_range() {
    let err = validate_persist_mode(3).expect_err("expected invalid persist mode");
    assert_eq!(
        err.name().as_str(),
        "org.freedesktop.DBus.Error.InvalidArgs"
    );
}

#[test]
fn mapping_helpers_sanitize_output_names() {
    assert_eq!(
        fallback_mapping_id(""),
        "screencomposer:output-default".to_string()
    );

    assert_eq!(
        make_output_mapping_id("DP-1"),
        "screencomposer:output-DP-1".to_string()
    );

    assert_eq!(
        make_output_mapping_id("HDMI/奇"),
        "screencomposer:output-HDMI__".to_string()
    );
}
