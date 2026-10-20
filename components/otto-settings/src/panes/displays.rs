//! The displays pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.
//!
//! Almost everything here is in the second category, and deliberately so.
//! Resolution, refresh rate, "use as primary", whether a screen is on, and
//! where its top-left corner sits are all per-output settings — in the
//! compositor's config they live under `displays.named.<connector>`
//! (`DisplaysConfig`/`DisplayProfile` in `src/config/mod.rs`), keyed by
//! connector name. The wire contract
//! (`docs/developer/settings-dbus-api.md`, "Open" section) explicitly leaves
//! that identity unresolved: settings are meant to follow the panel, not the
//! port, and connector names do not survive a display moving to a different
//! port or a docking-station reshuffle. Inventing an identifier now (e.g.
//! `displays.named.HDMI-A-1.resolution`) would bake a wire contract we would
//! have to support forever even after a real display-identity scheme lands,
//! so those rows stay unbound. The same goes for the virtual outputs, which
//! the compositor reads from `[[virtual_outputs]]` at startup
//! (`VirtualOutputConfig` in `src/config/mod.rs`) and serves no setting for
//! at all.
//!
//! What the unbound rows do edit is the session-local arrangement in
//! [`crate::model`] — see the doc comment there for why that exists rather
//! than nothing.
//!
//! Subpixel text is the exception that does reach the compositor: it is
//! written with `SetOutputSubpixel` to `displays.named.<connector>.subpixel`,
//! and read back once at startup with `GetOutputSubpixels`. It applies at the
//! next login (or when the display is next plugged in): the layout is fixed
//! when an output is created, and clients only learn it then.
//!
//! Scale is a global, not a per-display setting: the only scale Otto has is
//! the top-level `screen_scale` (`DisplayProfile` carries no per-output field,
//! see `src/settings/schema.rs`), so it sits in its own group rather than with
//! the selected display's rows.
//!
//! It is written like any other setting and takes effect at the next start,
//! never live. Changing it under a running session does not propagate:
//! otto-bar keeps rendering at the old scale, already-maximized windows keep
//! their pre-change geometry, and this app's own detail view has hardcoded
//! dimensions and does not reflow — applying it live would leave the desktop
//! unusable. The schema marks it `Restart`, so a changed value carries the
//! restart pill until the session comes back.

use crate::model::{self, group, Control, Pane, Row};

// Labels of the rows this pane routes back to itself. They are the only
// handle an unbound row has: `Row::id` is `None`, so there is no identifier
// to key on, and the label is what the hit tests in `view.rs` report.
//
// Now that the labels are translated they are no longer constants, but the
// routing still holds: `t!` hands out one interned `&'static str` per
// message, so a row built with `active()` and a comparison against `active()`
// are the same string whatever the language. It stays a weaker handle than a
// real identifier would be — two rows whose translations collided would
// become indistinguishable — which is worth fixing the day a pane needs it.

fn active() -> &'static str {
    otto_kit::t!("settings-display-active")
}
fn primary() -> &'static str {
    otto_kit::t!("settings-display-primary")
}
fn x_position() -> &'static str {
    otto_kit::t!("settings-display-x-position")
}
fn y_position() -> &'static str {
    otto_kit::t!("settings-display-y-position")
}
fn width() -> &'static str {
    otto_kit::t!("settings-display-width")
}
fn height() -> &'static str {
    otto_kit::t!("settings-display-height")
}
fn refresh() -> &'static str {
    otto_kit::t!("settings-display-refresh")
}
fn virtual_displays() -> &'static str {
    otto_kit::t!("settings-virtual-displays")
}
fn scale() -> &'static str {
    otto_kit::t!("settings-display-scale")
}
/// The push buttons on the virtual-displays row.
fn add() -> &'static str {
    otto_kit::t!("common-add")
}
fn remove() -> &'static str {
    otto_kit::t!("common-remove")
}

/// The two of them as the `'static` slice a button row wants. Resolved once:
/// the labels cannot change without a restart, and a fresh array each call
/// would not outlive the row it is handed to.
fn virtual_buttons() -> &'static [&'static str] {
    static BUTTONS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    BUTTONS.get_or_init(|| vec![add(), remove()])
}

/// Pop-up identifiers for the two rows filled from the display probe.
///
/// They look like setting identifiers because that is the handle a pop-up row
/// needs — `view.rs` reports the row's `id` and `main.rs` keys the menu pool
/// by it — but they name no setting: `org.otto.Settings` serves none for a
/// per-output mode. `main.rs` recognises them by [`menu_choices`] answering
/// and routes what is picked back to [`choose`] instead of onto the bus, the
/// same way a shortcut line's action pop-up is routed to `panes::keyboard`.
pub const RESOLUTION_ID: &str = "display.resolution";
pub const REFRESH_ID: &str = "display.refresh";
/// The subpixel-text pop-up. Routed the same way, and written with
/// `SetOutputSubpixel` rather than `Set`: like the mode, it is a per-display
/// key (`displays.named.<connector>.subpixel`), not a schema setting.
pub const SUBPIXEL_ID: &str = "display.subpixel";

/// The pop-up rows this pane owns, for the menu pool built at startup.
pub fn slot_ids() -> &'static [&'static str] {
    &[RESOLUTION_ID, REFRESH_ID, SUBPIXEL_ID]
}

/// The subpixel-text pop-up's entries, each with the value it writes to the
/// display's `subpixel` key. `None` removes the key: automatic is the
/// connector's own report, then fontconfig's `rgba`, then grayscale.
///
/// RGB and BGR are not translated: they name the order of the stripes, and
/// they are what the panel's spec sheet and fontconfig call them.
fn subpixel_choices() -> [(&'static str, Option<&'static str>); 6] {
    [
        (otto_kit::t!("settings-choice-automatic"), None),
        ("RGB", Some("rgb")),
        ("BGR", Some("bgr")),
        (otto_kit::t!("settings-display-subpixel-vrgb"), Some("vrgb")),
        (otto_kit::t!("settings-display-subpixel-vbgr"), Some("vbgr")),
        (otto_kit::t!("settings-display-subpixel-off"), Some("none")),
    ]
}

/// The value the pop-up entry labelled `label` writes, or `None` if no entry
/// has that label.
fn subpixel_value(label: &str) -> Option<Option<&'static str>> {
    subpixel_choices()
        .into_iter()
        .find(|(choice, _)| *choice == label)
        .map(|(_, value)| value)
}

/// The pop-up entry for a stored value. One the pane does not know — written
/// by hand, say — shows as itself rather than as a choice it is not.
fn subpixel_label(value: Option<&str>) -> String {
    subpixel_choices()
        .into_iter()
        .find(|(_, choice)| *choice == value)
        .map(|(label, _)| label.to_string())
        .unwrap_or_else(|| value.unwrap_or_default().to_string())
}

/// The choices `id`'s pop-up offers, or `None` if `id` is not one of ours.
///
/// Read off the selected display's mode list, which is the compositor's
/// answer rather than this app's guess — see the probe in [`crate::model`].
pub fn menu_choices(id: &str) -> Option<Vec<String>> {
    let outputs = model::outputs();
    let selected = outputs.get(model::selected_output())?;
    match id {
        RESOLUTION_ID => Some(selected.resolutions()),
        REFRESH_ID => Some(selected.refresh_rates()),
        SUBPIXEL_ID => Some(
            subpixel_choices()
                .into_iter()
                .map(|(label, _)| label.to_string())
                .collect(),
        ),
        _ => None,
    }
}

/// Apply a choice made in one of those pop-ups.
pub fn choose(id: &str, value: &str) {
    match id {
        RESOLUTION_ID => model::set_selected_resolution(value),
        REFRESH_ID => model::set_selected_refresh(value),
        SUBPIXEL_ID => {
            save_subpixel(value);
            return;
        }
        _ => return,
    }
    save_selected();
}

/// Write the selected display's subpixel layout, picked by its pop-up label.
///
/// Its own call rather than part of [`save`]: the rest of the profile is
/// written from the arrangement, which knows nothing of the layout, and a
/// mode change should not have to restate it.
fn save_subpixel(label: &str) {
    let Some(value) = subpixel_value(label) else {
        return;
    };
    let outputs = model::outputs();
    let Some(selected) = outputs.get(model::selected_output()) else {
        return;
    };
    if selected.is_virtual() {
        return;
    }
    match crate::settings_client::set_output_subpixel(&selected.name, value) {
        crate::settings_client::SetOutcome::Failed(why) => {
            eprintln!("{}: {why}", selected.name);
        }
        _ => println!(
            "{}: subpixel layout saved, takes effect at the next login",
            selected.name
        ),
    }
}

/// Write the selected display's profile to the configuration.
///
/// Keyed by connector, under `displays.named.<connector>` — the same profile
/// the compositor reads when it brings that output up. It applies at the next
/// start rather than now: a mode change is a modeset, and one made from under
/// a running session that cannot be undone if the display does not come back
/// is worse than one you restart for.
///
/// A virtual output is not written here. It is not a connector the compositor
/// resolves a profile for — it is an entry in `[[virtual_outputs]]`, which has
/// its own writer on the bus.
fn save_selected() {
    let outputs = model::outputs();
    let Some(selected) = outputs.get(model::selected_output()) else {
        return;
    };
    save(selected);
}

/// Write every display's profile.
///
/// Which screen is primary is a choice *among* the displays, not a flag on
/// one: making this one primary takes it off the others, and a configuration
/// that recorded only the winner would come back with two.
fn save_all() {
    for output in model::outputs() {
        save(&output);
    }
}

fn save(output: &model::Output) {
    if output.is_virtual() {
        return;
    }
    let mode = output.current_mode();
    let outcome = crate::settings_client::set_output_profile(
        &output.name,
        mode.map(|m| m.width as u32).unwrap_or(0),
        mode.map(|m| m.height as u32).unwrap_or(0),
        mode.map(|m| m.refresh_mhz as f64 / 1000.0).unwrap_or(0.0),
        output.x as i32,
        output.y as i32,
        output.primary,
    );
    match outcome {
        crate::settings_client::SetOutcome::Failed(why) => {
            eprintln!("{}: {why}", output.name);
        }
        _ => println!(
            "{}: saved to {}, takes effect after a restart",
            output.name,
            crate::settings_client::config_path().unwrap_or_else(|| "the configuration".into()),
        ),
    }
}

pub fn build() -> Pane {
    let outputs = model::outputs();
    let Some(selected) = outputs.get(model::selected_output()) else {
        // The probe found nothing to show. Only reachable when the compositor
        // has announced no output at all — a headless session with no virtual
        // output — and a pane that says so beats one that panics.
        return Pane {
            name: otto_kit::t!("settings-pane-displays"),
            icon: "monitor",
            intro: None,
            groups: vec![model::untitled(vec![Row::new(
                otto_kit::t!("settings-display-none"),
                Control::Value(String::new()),
            )
            .detail(otto_kit::t!("settings-display-none-detail"))])],
        };
    };
    let count = model::virtual_output_count();
    Pane {
        name: otto_kit::t!("settings-pane-displays"),
        icon: "monitor",
        intro: None,
        // The arrangement canvas is drawn by the pane itself, not as a row.
        // Below it sit the settings for whichever display is selected there.
        groups: vec![
            group(
                selected.name.clone(),
                mode_rows(selected)
                    .into_iter()
                    .chain([
                        Row::new(active(), Control::Toggle(selected.enabled))
                            .detail(otto_kit::t!("settings-display-active-detail")),
                        Row::new(primary(), Control::Toggle(selected.primary))
                            .detail(otto_kit::t!("settings-display-primary-detail")),
                        // Positions are logical pixels, which is the space the
                        // arrangement canvas lays out in — see the coordinate
                        // conventions in the repository's CLAUDE.md.
                        Row::new(x_position(), Control::Text(position_text(selected.x)))
                            .detail(otto_kit::t!("settings-display-x-position-detail")),
                        Row::new(y_position(), Control::Text(position_text(selected.y))),
                    ])
                    .collect(),
            ),
            group(
                scale(),
                vec![Row::new(
                    scale(),
                    // Placeholders: the schema owns the range and the step
                    // (0.5 to 4.0 by quarters), and `Row::id` takes them from
                    // it. The readout's `%` is this pane's choice and is what
                    // keeps it rendering as a percentage.
                    Control::Slider {
                        value: 1.0,
                        min: 0.5,
                        max: 4.0,
                        readout: "100%".into(),
                    },
                )
                .id("screen_scale")
                .detail(otto_kit::t!("settings-display-scale-detail"))],
            ),
            group(
                virtual_displays(),
                vec![
                    Row::new(virtual_displays(), Control::Button(virtual_buttons())).detail(
                        otto_kit::t_owned!(
                            "settings-virtual-displays-detail",
                            count = count as f64
                        ),
                    ),
                ],
            ),
        ],
    }
}

/// The rows describing how the selected display is being driven.
///
/// A panel is driven at one of the modes its connector advertises, so its size
/// and its rate are pop-ups over that list. A virtual output has no such list
/// — it is headless, and it is whatever it is told to be — so it gets fields
/// to type in instead. A pop-up with one entry in it is not a choice.
fn mode_rows(selected: &model::Output) -> Vec<Row> {
    let mode = selected.current_mode();
    if selected.is_virtual() {
        return vec![
            Row::new(
                width(),
                Control::Text(mode.map(|m| m.width.to_string()).unwrap_or_default()),
            )
            .detail(otto_kit::t!("settings-display-width-detail")),
            Row::new(
                height(),
                Control::Text(mode.map(|m| m.height.to_string()).unwrap_or_default()),
            ),
            Row::new(
                refresh(),
                Control::Text(
                    mode.map(|m| format!("{:.0}", m.refresh_mhz as f32 / 1000.0))
                        .unwrap_or_default(),
                ),
            )
            .detail(otto_kit::t!("settings-display-refresh-detail")),
        ];
    }
    vec![
        Row::new(
            otto_kit::t!("settings-display-resolution"),
            Control::Select(mode.map(|m| m.resolution()).unwrap_or_default()),
        )
        .id(RESOLUTION_ID),
        Row::new(
            refresh(),
            Control::Select(mode.map(|m| m.refresh()).unwrap_or_default()),
        )
        .id(REFRESH_ID),
        Row::new(
            otto_kit::t!("settings-display-subpixel"),
            Control::Select(subpixel_label(
                crate::settings_client::output_subpixel(&selected.name).as_deref(),
            )),
        )
        .id(SUBPIXEL_ID)
        .detail(otto_kit::t!("settings-display-subpixel-detail")),
    ]
}

/// A coordinate as the text field shows it. Whole logical pixels: a display
/// cannot start on half of one, and "1128" reads as a position where
/// "1128.0" reads as a measurement.
fn position_text(value: f32) -> String {
    format!("{value:.0}")
}

/// Apply a committed edit from one of this pane's text rows.
///
/// Called for any text row with no `id`, since a row that has one goes to the
/// compositor instead. Text that is not a number is dropped rather than
/// guessed at — the field then redraws with the position the screen still
/// has, which is the honest answer to "seventeen-ish".
pub fn commit_text(label: &str, text: &str) {
    let Ok(value) = text.trim().parse::<f32>() else {
        return;
    };
    // Guards rather than patterns: the labels are looked up at runtime now, so
    // they are values, not constants a `match` arm can name.
    match label {
        l if l == x_position() => model::set_selected_position(Some(value), None),
        l if l == y_position() => model::set_selected_position(None, Some(value)),
        l if l == width() => model::set_selected_size(Some(value as i32), None),
        l if l == height() => model::set_selected_size(None, Some(value as i32)),
        l if l == refresh() => model::set_selected_refresh_hz(value),
        _ => return,
    }
    save_selected();
}

/// A press on one of this pane's unbound switches.
pub fn toggle(label: &str) {
    match label {
        l if l == active() => model::toggle_selected_enabled(),
        // Primary is a choice among the displays rather than a per-display
        // flag, so the switch only ever turns *on*: taking it off would leave
        // the desktop with nowhere to put the dock.
        l if l == primary() => {
            model::make_selected_primary();
            save_all();
            return;
        }
        _ => return,
    }
    save_selected();
}

/// A press on one of this pane's push buttons.
pub fn press(row: &str, button: &str) {
    if row != virtual_displays() {
        return;
    }
    match button {
        b if b == add() => model::add_virtual_output(),
        b if b == remove() && !model::remove_selected_virtual_output() => {
            eprintln!("displays: only a virtual display can be removed");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Automatic is the absence of the key, and Off is the config's `none` —
    /// the two entries whose label is not the value they write.
    #[test]
    fn subpixel_entries_write_the_config_spelling() {
        assert_eq!(
            subpixel_value(otto_kit::t!("settings-choice-automatic")),
            Some(None)
        );
        assert_eq!(
            subpixel_value(otto_kit::t!("settings-display-subpixel-off")),
            Some(Some("none"))
        );
        assert_eq!(subpixel_value("BGR"), Some(Some("bgr")));
        assert_eq!(subpixel_value("not an entry"), None);
    }

    /// A stored value shows as the entry that writes it, so picking what the
    /// pop-up already shows changes nothing.
    #[test]
    fn a_stored_subpixel_shows_as_its_entry() {
        for (label, value) in subpixel_choices() {
            assert_eq!(subpixel_label(value), label);
        }
    }
}
