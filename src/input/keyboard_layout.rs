//! The active keyboard layout, as a status bar sees it.
//!
//! `org.otto.Shell1` answers `GetInputs` and sends `InputChanged` in sway's
//! `GET_INPUTS` and `input` event shapes, so a bar learns which layout is
//! active without polling. The layout can change in three ways, and each one
//! announces it here: a key combination XKB itself acts on (a `grp:` option),
//! checked after every key with one comparison; the `xkb_switch_layout`
//! command; and a new keymap from the settings.

use serde_json::{json, Value};
use smithay::input::keyboard::Layout;

use crate::config::Config;
use crate::shell_service::{self, EventKind};
use crate::state::{Backend, Otto};
use crate::workspaces::tiling::command::LayoutTarget;

/// The one keyboard's identifier. Otto merges every physical keyboard into
/// the seat's, so there is nothing finer to address.
const IDENTIFIER: &str = "otto:keyboard";

/// The configured layout codes, one per layout in the keymap: `us,it` is
/// `["us", "it"]`. An unset layout is XKB's default, which is
/// `XKB_DEFAULT_LAYOUT` or `us`.
fn layout_codes(configured: &str) -> Vec<String> {
    let codes: Vec<String> = configured
        .split(',')
        .map(|code| code.trim().to_string())
        .collect();
    if codes.iter().all(String::is_empty) {
        let fallback = std::env::var("XKB_DEFAULT_LAYOUT")
            .ok()
            .filter(|layout| !layout.is_empty())
            .unwrap_or_else(|| "us".to_string());
        return fallback.split(',').map(|c| c.trim().to_string()).collect();
    }
    codes
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// The layouts in the keymap by name, and the index of the active one.
    fn keymap_layouts(&mut self) -> (Vec<String>, u32) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return (Vec::new(), 0);
        };
        keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().unwrap();
            let names = xkb
                .layouts()
                .map(|layout| xkb.layout_name(layout).to_string())
                .collect();
            (names, xkb.active_layout().0)
        })
    }

    /// The keyboard in sway's `GET_INPUTS` shape, plus the two `otto_` keys a
    /// bar needs: the short codes to draw, and whether to draw them.
    pub fn keyboard_json(&mut self) -> Value {
        let (names, active) = self.keymap_layouts();
        let (codes, show) =
            Config::with(|c| (c.input.xkb_layout.clone(), c.input.show_layout_in_bar));
        json!({
            "identifier": IDENTIFIER,
            "name": "Keyboard",
            "type": "keyboard",
            "xkb_active_layout_name": names.get(active as usize).cloned().unwrap_or_default(),
            "xkb_layout_names": names,
            "xkb_active_layout_index": active,
            "otto_layout_codes": layout_codes(codes.as_deref().unwrap_or_default()),
            "otto_show_in_bar": show,
        })
    }

    /// `GetInputs`: every input device, in sway's shape. Just the keyboard.
    pub fn inputs_json(&mut self) -> Value {
        Value::Array(vec![self.keyboard_json()])
    }

    fn announce_input(&mut self, change: &str) {
        let input = self.keyboard_json();
        shell_service::announce(EventKind::Input, json!({"change": change, "input": input}));
    }

    /// Announce the active layout if it is not the one last announced.
    ///
    /// Run after every key event: the layout effective index is part of the
    /// modifier state the keyboard already holds, so nothing is asked of XKB
    /// unless it moved.
    pub fn note_active_layout(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let active = keyboard.modifier_state().serialized.layout_effective;
        if active != self.announced_layout {
            self.announced_layout = active;
            self.announce_input("xkb_layout");
        }
    }

    /// Announce a rebuilt keymap, or a change to how the bar shows it. A new
    /// keymap starts on its first layout.
    pub fn announce_keymap(&mut self) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            self.announced_layout = keyboard.modifier_state().serialized.layout_effective;
        }
        self.announce_input("xkb_keymap");
    }

    /// `input type:keyboard xkb_switch_layout <n>|next|prev`.
    pub fn switch_layout(&mut self, target: LayoutTarget) -> Result<(), String> {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return Err("the seat has no keyboard".to_string());
        };
        keyboard.with_xkb_state(self, |mut context| {
            let count = context.xkb().lock().unwrap().layouts().count();
            match target {
                LayoutTarget::Next => context.cycle_next_layout(),
                LayoutTarget::Prev => context.cycle_prev_layout(),
                LayoutTarget::Index(index) if index < count => {
                    context.set_layout(Layout(index as u32))
                }
                LayoutTarget::Index(index) => {
                    return Err(format!(
                        "there is no layout {index}: the keymap has {count} (counted from 0)"
                    ))
                }
            }
            Ok(())
        })?;
        self.current_modifiers = keyboard.modifier_state();
        self.note_active_layout();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_follow_the_configured_list() {
        assert_eq!(layout_codes("us,it"), ["us", "it"]);
        assert_eq!(layout_codes(" gb , fr"), ["gb", "fr"]);
    }

    #[test]
    fn an_unset_layout_is_the_xkb_default() {
        let expected = std::env::var("XKB_DEFAULT_LAYOUT")
            .ok()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "us".to_string());
        assert_eq!(layout_codes("").join(","), expected);
    }
}
