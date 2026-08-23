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
//! Scale is the one exception: `DisplayProfile` has no per-output scale field
//! at all today, only the single top-level `screen_scale` (see
//! `src/settings/schema.rs`). So although this row is drawn under a
//! per-display header, it is bound to that global identifier — it is the
//! only scale setting Otto actually has.

use crate::model::{self, group, Control, Pane, Row};

/// Labels of the rows this pane routes back to itself. They are the only
/// handle an unbound row has: `Row::id` is `None`, so there is no identifier
/// to key on, and the label is what the hit tests in `view.rs` report.
const ACTIVE: &str = "Active";
const PRIMARY: &str = "Use as primary";
const X_POSITION: &str = "X position";
const Y_POSITION: &str = "Y position";
const WIDTH: &str = "Width";
const HEIGHT: &str = "Height";
const REFRESH: &str = "Refresh rate";
const VIRTUAL: &str = "Virtual displays";

/// The push buttons on the [`VIRTUAL`] row.
const ADD: &str = "Add";
const REMOVE: &str = "Remove";

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

/// The pop-up rows this pane owns, for the menu pool built at startup.
pub fn slot_ids() -> &'static [&'static str] {
    &[RESOLUTION_ID, REFRESH_ID]
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
        _ => None,
    }
}

/// Apply a choice made in one of those pop-ups.
pub fn choose(id: &str, value: &str) {
    match id {
        RESOLUTION_ID => model::set_selected_resolution(value),
        REFRESH_ID => model::set_selected_refresh(value),
        _ => return,
    }
    save_selected();
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
            name: "Displays",
            icon: "monitor",
            groups: vec![model::untitled(vec![Row::new(
                "No displays",
                Control::Value(String::new()),
            )
            .detail("The compositor is not driving any output")])],
        };
    };
    let count = model::virtual_output_count();
    Pane {
        name: "Displays",
        icon: "monitor",
        // The arrangement canvas is drawn by the pane itself, not as a row.
        // Below it sit the settings for whichever display is selected there.
        groups: vec![
            group(
                selected.name.clone(),
                mode_rows(selected)
                    .into_iter()
                    .chain([
                        Row::new(
                            "Scale",
                            Control::Slider {
                                value: 2.0,
                                min: 0.5,
                                max: 4.0,
                                readout: "200%".into(),
                            },
                        )
                        .id("screen_scale"),
                        Row::new(ACTIVE, Control::Toggle(selected.enabled))
                            .detail("An inactive display keeps its place in the arrangement"),
                        Row::new(PRIMARY, Control::Toggle(selected.primary))
                            .detail("The dock and the bar live on the primary display"),
                        // Positions are logical pixels, which is the space the
                        // arrangement canvas lays out in — see the coordinate
                        // conventions in the repository's CLAUDE.md.
                        Row::new(X_POSITION, Control::Text(position_text(selected.x)))
                            .detail("Top-left corner in the desktop's coordinate space"),
                        Row::new(Y_POSITION, Control::Text(position_text(selected.y))),
                    ])
                    .collect(),
            ),
            group(
                VIRTUAL,
                vec![
                    Row::new(VIRTUAL, Control::Button(&[ADD, REMOVE])).detail(format!(
                        "{count} headless {}, streamed over PipeWire. Remove takes away the \
                     selected one",
                        if count == 1 { "output" } else { "outputs" },
                    )),
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
                WIDTH,
                Control::Text(mode.map(|m| m.width.to_string()).unwrap_or_default()),
            )
            .detail("Pixels. A headless output can be any size"),
            Row::new(
                HEIGHT,
                Control::Text(mode.map(|m| m.height.to_string()).unwrap_or_default()),
            ),
            Row::new(
                REFRESH,
                Control::Text(
                    mode.map(|m| format!("{:.0}", m.refresh_mhz as f32 / 1000.0))
                        .unwrap_or_default(),
                ),
            )
            .detail("Hertz — how often the stream is fed a frame"),
        ];
    }
    vec![
        Row::new(
            "Resolution",
            Control::Select(mode.map(|m| m.resolution()).unwrap_or_default()),
        )
        .id(RESOLUTION_ID),
        Row::new(
            REFRESH,
            Control::Select(mode.map(|m| m.refresh()).unwrap_or_default()),
        )
        .id(REFRESH_ID),
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
    match label {
        X_POSITION => model::set_selected_position(Some(value), None),
        Y_POSITION => model::set_selected_position(None, Some(value)),
        WIDTH => model::set_selected_size(Some(value as i32), None),
        HEIGHT => model::set_selected_size(None, Some(value as i32)),
        REFRESH => model::set_selected_refresh_hz(value),
        _ => return,
    }
    save_selected();
}

/// A press on one of this pane's unbound switches.
pub fn toggle(label: &str) {
    match label {
        ACTIVE => model::toggle_selected_enabled(),
        // Primary is a choice among the displays rather than a per-display
        // flag, so the switch only ever turns *on*: taking it off would leave
        // the desktop with nowhere to put the dock.
        PRIMARY => {
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
    if row != VIRTUAL {
        return;
    }
    match button {
        ADD => model::add_virtual_output(),
        REMOVE => {
            if !model::remove_selected_virtual_output() {
                eprintln!("displays: only a virtual display can be removed");
            }
        }
        _ => {}
    }
}
