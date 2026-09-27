//! The keyboard pane's input sources: the layouts, their variants and the key
//! combination that switches between them.
//!
//! On the bus these are three settings — `input.xkb_layout` and
//! `input.xkb_variant`, each a comma-separated list with one entry per
//! layout, and `input.xkb_options` — and the compositor applies all three
//! live by rebuilding the seat's keymap. The pane shows them as one pop-up per
//! layout and per variant instead, filled from the XKB registry (see
//! [`crate::xkb_registry`]), and writes the lists back whole.
//!
//! The pop-ups are keyed by identifiers that look like settings but name
//! none, the way the Displays pane's mode pop-ups are: `main.rs` asks
//! [`menu_choices`] first and routes what is picked to [`choose`].

use crate::discovery::Choice;
use crate::model::{Control, Row};
use crate::settings_client::{self, SetOutcome, Value};
use crate::xkb_registry::registry;

const LAYOUT_ID: &str = "input.xkb_layout";
const VARIANT_ID: &str = "input.xkb_variant";
const OPTIONS_ID: &str = "input.xkb_options";
const SHOW_IN_BAR_ID: &str = "input.show_layout_in_bar";

/// How many layouts the pane holds: XKB switches between at most four groups.
pub const MAX_LAYOUTS: usize = 4;

const LAYOUT_SLOTS: [&str; MAX_LAYOUTS] = [
    "keyboard.layout.0",
    "keyboard.layout.1",
    "keyboard.layout.2",
    "keyboard.layout.3",
];
const VARIANT_SLOTS: [&str; MAX_LAYOUTS] = [
    "keyboard.variant.0",
    "keyboard.variant.1",
    "keyboard.variant.2",
    "keyboard.variant.3",
];
const SWITCH_ID: &str = "keyboard.switch";

/// The value of the pop-up entry that takes a layout out of the list. Not a
/// name XKB could ever use, so it cannot collide with a real layout.
const REMOVE: &str = "\u{0}remove";

/// The prefix of the XKB options that switch layout.
const SWITCH_PREFIX: &str = "grp:";

/// Every pop-up this module owns, for the menu pool built at startup.
pub fn slot_ids() -> Vec<&'static str> {
    LAYOUT_SLOTS
        .iter()
        .chain(VARIANT_SLOTS.iter())
        .copied()
        .chain([SWITCH_ID])
        .collect()
}

fn layouts_label() -> &'static str {
    otto_kit::t!("settings-xkb-layouts")
}

fn add_label() -> &'static str {
    otto_kit::t!("common-add")
}

fn add_buttons() -> &'static [&'static str] {
    static BUTTONS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    BUTTONS.get_or_init(|| vec![add_label()])
}

/// The layouts in force, each with its variant. Never empty: an unset layout
/// is one slot holding the empty name, which XKB reads as its default.
fn sources() -> Vec<(String, String)> {
    let text = |id| {
        settings_client::value(id)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    pair_up(&text(LAYOUT_ID), &text(VARIANT_ID))
}

/// Split the two comma-separated lists into (layout, variant) pairs, one per
/// layout; a variant list shorter than the layout list means "no variant" for
/// the rest.
fn pair_up(layouts: &str, variants: &str) -> Vec<(String, String)> {
    let mut variants = variants.split(',').map(|v| v.trim().to_string());
    layouts
        .split(',')
        .map(|layout| {
            (
                layout.trim().to_string(),
                variants.next().unwrap_or_default(),
            )
        })
        .collect()
}

/// A list the way the configuration writes it: comma-separated, with the
/// empty entries at the end left off.
fn join(items: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let joined = items
        .into_iter()
        .map(|item| item.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(",");
    joined.trim_end_matches(',').to_string()
}

/// The `Set`s that take the configuration from `old` to `new`, in order.
///
/// Layout and variant are two settings, and the compositor compiles a keymap
/// after each one: a variant paired, even for a moment, with a layout that
/// does not have it is refused, and the change with it. So the variants no
/// longer valid are cleared first — keeping only the pairs that are the same
/// before and after — then the layouts are written, then the new variants.
fn plan(old: &[(String, String)], new: &[(String, String)]) -> Vec<(&'static str, String)> {
    let old_variants = join(old.iter().map(|(_, v)| v));
    let interim = join(
        old.iter()
            .zip(new)
            .map(|((old_l, old_v), (new_l, _))| if old_l == new_l { old_v.as_str() } else { "" }),
    );
    let old_layouts = join(old.iter().map(|(l, _)| l));
    let new_layouts = join(new.iter().map(|(l, _)| l));
    let new_variants = join(new.iter().map(|(_, v)| v));

    let mut steps = Vec::new();
    if interim != old_variants {
        steps.push((VARIANT_ID, interim.clone()));
    }
    if new_layouts != old_layouts {
        steps.push((LAYOUT_ID, new_layouts));
    }
    if new_variants != interim {
        steps.push((VARIANT_ID, new_variants));
    }
    steps
}

/// Write `new` over what is in force, stopping at the first refusal so the
/// compositor is never left further from a working keymap than it was.
fn write(new: &[(String, String)]) {
    for (id, value) in plan(&sources(), new) {
        if let SetOutcome::Failed(why) = settings_client::set(id, Value::Text(value.clone())) {
            eprintln!("{id} = {value:?}: {why}");
            return;
        }
    }
}

fn options() -> Vec<String> {
    match settings_client::value(OPTIONS_ID) {
        Some(Value::List(items)) => items,
        _ => Vec::new(),
    }
}

/// The layout-switch option in force, or empty for none.
fn switch_option(options: &[String]) -> String {
    options
        .iter()
        .find(|option| option.starts_with(SWITCH_PREFIX))
        .cloned()
        .unwrap_or_default()
}

/// `options` with its layout-switch options replaced by `switch`, which may
/// be empty for none. The rest keep their order.
fn with_switch(options: &[String], switch: &str) -> Vec<String> {
    let mut out: Vec<String> = options
        .iter()
        .filter(|option| !option.starts_with(SWITCH_PREFIX))
        .cloned()
        .collect();
    if !switch.is_empty() {
        out.push(switch.to_string());
    }
    out
}

/// The layout the session would start on with nothing configured.
fn default_layout() -> String {
    std::env::var("XKB_DEFAULT_LAYOUT")
        .ok()
        .and_then(|layout| layout.split(',').next().map(str::to_string))
        .filter(|layout| !layout.is_empty())
        .unwrap_or_else(|| "us".to_string())
}

/// A layout to add next: the one the locale's country or language names,
/// when the registry has it and it is not already in the list, else the
/// first one that is not.
fn suggested_layout(in_use: &[(String, String)]) -> Option<String> {
    let registry = registry();
    let unused = |name: &str| !in_use.iter().any(|(layout, _)| layout == name);
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let locale = locale.split(['.', '@']).next().unwrap_or_default();
    let (language, country) = locale.split_once('_').unwrap_or((locale, ""));
    [country.to_lowercase(), language.to_lowercase()]
        .into_iter()
        .filter(|name| !name.is_empty())
        .find(|name| registry.layout(name).is_some() && unused(name))
        .or_else(|| {
            registry
                .layouts
                .iter()
                .map(|layout| layout.name.clone())
                .find(|name| unused(name))
        })
        .or_else(|| unused("us").then(|| "us".to_string()))
}

fn slot(ids: &[&str; MAX_LAYOUTS], id: &str) -> Option<usize> {
    ids.iter().position(|slot| *slot == id)
}

fn layout_label(name: &str) -> String {
    if name.is_empty() {
        return otto_kit::t_owned!("settings-xkb-layout-default");
    }
    registry()
        .layout(name)
        .map(|layout| layout.description.clone())
        .unwrap_or_else(|| name.to_string())
}

fn variant_label(layout: &str, variant: &str) -> String {
    if variant.is_empty() {
        return otto_kit::t_owned!("settings-xkb-variant-standard");
    }
    registry()
        .variant_description(layout, variant)
        .map(str::to_string)
        .unwrap_or_else(|| variant.to_string())
}

fn switch_label(option: &str) -> String {
    if option.is_empty() {
        return otto_kit::t_owned!("settings-xkb-switch-none");
    }
    registry()
        .switch_description(option)
        .map(str::to_string)
        .unwrap_or_else(|| option.to_string())
}

/// What a pop-up of ours shows for `value`, or `None` if `id` is not ours.
pub fn display(id: &str, value: &str) -> Option<String> {
    if slot(&LAYOUT_SLOTS, id).is_some() {
        return Some(layout_label(value));
    }
    if let Some(index) = slot(&VARIANT_SLOTS, id) {
        let sources = sources();
        let layout = sources.get(index).map(|(l, _)| l.as_str()).unwrap_or("");
        return Some(variant_label(layout, value));
    }
    (id == SWITCH_ID).then(|| switch_label(value))
}

/// Put the value in force first when the list does not have it, so the menu
/// always shows a selection.
fn with_current(
    mut choices: Vec<Choice>,
    current: &str,
    label: impl FnOnce() -> String,
) -> Vec<Choice> {
    if !choices.iter().any(|choice| choice.value == current) {
        choices.insert(
            0,
            Choice {
                label: label(),
                value: current.to_string(),
            },
        );
    }
    choices
}

/// The entries `id`'s pop-up offers, or `None` if `id` is not ours.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    let sources = sources();
    let registry = registry();

    if let Some(index) = slot(&LAYOUT_SLOTS, id) {
        let current = sources
            .get(index)
            .map(|(l, _)| l.clone())
            .unwrap_or_default();
        let mut choices = Vec::new();
        // "System default" is only a layout on its own: next to others it
        // would be an unnamed one in the middle of the list.
        if sources.len() == 1 {
            choices.push(Choice {
                label: layout_label(""),
                value: String::new(),
            });
        }
        choices.extend(registry.layouts.iter().map(|layout| Choice {
            label: layout.description.clone(),
            value: layout.name.clone(),
        }));
        let mut choices = with_current(choices, &current, || layout_label(&current));
        if index > 0 {
            choices.push(Choice {
                label: otto_kit::t_owned!("settings-xkb-layout-remove"),
                value: REMOVE.to_string(),
            });
        }
        return Some(choices);
    }

    if let Some(index) = slot(&VARIANT_SLOTS, id) {
        let (layout, current) = sources.get(index).cloned().unwrap_or_default();
        let mut choices = vec![Choice {
            label: variant_label(&layout, ""),
            value: String::new(),
        }];
        if let Some(entry) = registry.layout(&layout) {
            choices.extend(entry.variants.iter().map(|variant| Choice {
                label: variant.description.clone(),
                value: variant.name.clone(),
            }));
        }
        return Some(with_current(choices, &current, || {
            variant_label(&layout, &current)
        }));
    }

    if id == SWITCH_ID {
        let current = switch_option(&options());
        let mut choices = vec![Choice {
            label: switch_label(""),
            value: String::new(),
        }];
        choices.extend(registry.switch_options.iter().map(|option| Choice {
            label: option.description.clone(),
            value: option.name.clone(),
        }));
        return Some(with_current(choices, &current, || switch_label(&current)));
    }

    None
}

/// Apply a choice made in one of our pop-ups. Returns whether `id` was ours.
pub fn choose(id: &str, value: &str) -> bool {
    let mut sources = sources();

    if let Some(index) = slot(&LAYOUT_SLOTS, id) {
        if index >= sources.len() {
            return true;
        }
        if value == REMOVE {
            sources.remove(index);
        } else {
            // A variant belongs to the layout it was picked for.
            sources[index] = (value.to_string(), String::new());
        }
        write(&sources);
        return true;
    }

    if let Some(index) = slot(&VARIANT_SLOTS, id) {
        if let Some(source) = sources.get_mut(index) {
            source.1 = value.to_string();
            write(&sources);
        }
        return true;
    }

    if id == SWITCH_ID {
        let next = with_switch(&options(), value);
        if let SetOutcome::Failed(why) = settings_client::set(OPTIONS_ID, Value::List(next)) {
            eprintln!("{OPTIONS_ID}: {why}");
        }
        return true;
    }

    false
}

/// Take out the layout whose row's "−" button was pressed, with its variant.
/// The first layout has no button: there is always one.
pub fn remove(id: &str) {
    let Some(index) = slot(&LAYOUT_SLOTS, id).filter(|index| *index > 0) else {
        return;
    };
    let mut sources = sources();
    if index < sources.len() {
        sources.remove(index);
        write(&sources);
    }
}

/// Do what a push button of ours does. Rows are routed by label, see
/// `main.rs`'s `activate`.
pub fn press(row: &str, button: &str) {
    if row != layouts_label() || button != add_label() {
        return;
    }
    let mut sources = sources();
    if sources.len() >= MAX_LAYOUTS {
        return;
    }
    // The unnamed default cannot sit next to a second layout, so it is named
    // first: the layout it already stands for.
    if let Some(first) = sources.first_mut().filter(|(layout, _)| layout.is_empty()) {
        first.0 = default_layout();
    }
    if let Some(next) = suggested_layout(&sources) {
        sources.push((next, String::new()));
        write(&sources);
    }
}

/// The pane's rows for the input sources.
pub fn rows() -> Vec<Row> {
    let sources = sources();
    let mut rows = Vec::new();
    for (index, (layout, variant)) in sources.iter().enumerate() {
        rows.push(
            Row::new(
                otto_kit::t!("settings-xkb-layout-nth", n = index + 1),
                Control::Select(layout.clone()),
            )
            .id(LAYOUT_SLOTS[index])
            .removable(index > 0),
        );
        let has_variants = registry()
            .layout(layout)
            .is_some_and(|entry| !entry.variants.is_empty());
        if has_variants || !variant.is_empty() {
            rows.push(
                Row::new(
                    otto_kit::t!("settings-xkb-variant"),
                    Control::Select(variant.clone()),
                )
                .id(VARIANT_SLOTS[index]),
            );
        }
    }
    rows.push(
        Row::new(layouts_label(), Control::Button(add_buttons()))
            .detail(otto_kit::t!("settings-xkb-layouts-detail"))
            .inactive(sources.len() >= MAX_LAYOUTS),
    );
    if sources.len() > 1 {
        rows.push(
            Row::new(
                otto_kit::t!("settings-xkb-switch"),
                Control::Select(switch_option(&options())),
            )
            .id(SWITCH_ID)
            .detail(otto_kit::t!("settings-xkb-switch-detail")),
        );
        // The bar's indicator only has something to say with a second
        // layout, so the switch for it is only offered then.
        rows.push(
            Row::new(
                otto_kit::t!("settings-xkb-show-in-bar"),
                Control::Toggle(false),
            )
            .id(SHOW_IN_BAR_ID),
        );
    }
    rows.push(
        Row::new(
            otto_kit::t!("settings-xkb-options"),
            Control::Text(String::new()),
        )
        .id(OPTIONS_ID),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_lists_pair_up_by_position() {
        assert_eq!(
            pair_up("us,it", "dvorak"),
            pairs(&[("us", "dvorak"), ("it", "")])
        );
        assert_eq!(pair_up("", ""), pairs(&[("", "")]));
        assert_eq!(join(["intl", "", ""]), "intl");
        assert_eq!(join(["", "intl"]), ",intl");
    }

    #[test]
    fn adding_a_layout_writes_only_the_layouts() {
        let old = pairs(&[("us", "intl")]);
        let new = pairs(&[("us", "intl"), ("it", "")]);
        assert_eq!(plan(&old, &new), [(LAYOUT_ID, "us,it".to_string())]);
    }

    /// A variant is never paired with a layout it does not belong to, even
    /// between two of the writes: the compositor would refuse the keymap.
    #[test]
    fn a_stale_variant_is_cleared_before_its_layout_changes() {
        let old = pairs(&[("us", "dvorak"), ("it", "")]);
        let new = pairs(&[("it", ""), ("us", "dvorak")]);
        assert_eq!(
            plan(&old, &new),
            [
                (VARIANT_ID, String::new()),
                (LAYOUT_ID, "it,us".to_string()),
                (VARIANT_ID, ",dvorak".to_string()),
            ]
        );
    }

    #[test]
    fn removing_a_layout_drops_its_variant_first() {
        let old = pairs(&[("us", ""), ("it", "mac")]);
        let new = pairs(&[("us", "")]);
        assert_eq!(
            plan(&old, &new),
            [(VARIANT_ID, String::new()), (LAYOUT_ID, "us".to_string())]
        );
    }

    #[test]
    fn picking_a_variant_writes_only_the_variants() {
        let old = pairs(&[("us", ""), ("it", "")]);
        let new = pairs(&[("us", ""), ("it", "mac")]);
        assert_eq!(plan(&old, &new), [(VARIANT_ID, ",mac".to_string())]);
    }

    #[test]
    fn the_switch_option_replaces_the_previous_one_only() {
        let options: Vec<String> = ["caps:escape", "grp:alt_shift_toggle", "compose:ralt"]
            .map(String::from)
            .into();
        assert_eq!(switch_option(&options), "grp:alt_shift_toggle");
        assert_eq!(
            with_switch(&options, "grp:win_space_toggle"),
            ["caps:escape", "compose:ralt", "grp:win_space_toggle"]
        );
        assert_eq!(with_switch(&options, ""), ["caps:escape", "compose:ralt"]);
    }
}
