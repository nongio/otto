//! Static stand-in for the settings schema.
//!
//! The real app fetches this from the compositor over `org.otto.Settings`
//! (see `specs/settings-app.md`). Nothing here is wired: the values are
//! plausible constants chosen to exercise every control the panes need.

use std::borrow::Cow;
use std::sync::{OnceLock, RwLock};

use crate::panes;
use crate::settings_client::{self, Value};

/// One row in a pane.
#[derive(Clone)]
pub enum Control {
    /// On/off switch.
    Toggle(bool),
    /// Continuous value: current, min, max, and the formatted readout.
    Slider {
        value: f32,
        min: f32,
        max: f32,
        readout: String,
    },
    /// Pop-up button showing the current choice.
    Select(String),
    /// Colour swatch plus its hex value.
    Color(u32),
    /// Free text (paths, command lines, numbers), edited in place — see
    /// `Settings::text_hit` and the `Editing` session in `main.rs`.
    Text(String),
    /// Push buttons at the row's trailing edge, labelled by these strings.
    /// For a row that *does* something rather than holding a value.
    Button(&'static [&'static str]),
    /// A file the user picks through the desktop portal. Holds the chosen
    /// path, empty until one is chosen.
    File(String),
    /// Static informational value, not editable here.
    Value(String),
    /// One editable shortcut line: the action pop-up, the key combination
    /// field, and the button that deletes it.
    ///
    /// `index` is the line's position in [`crate::panes::keyboard`]'s list,
    /// which is what every hit test and edit is addressed by — the row itself
    /// holds no state, since the model is rebuilt on every frame.
    Shortcut { index: usize },
    /// The trailing line that appends a shortcut.
    AddShortcut,
}

pub struct Row {
    pub label: &'static str,
    /// Secondary line under the label. `None` keeps the row single-height.
    ///
    /// Usually the served schema's description — help text belongs to the
    /// compositor, which owns the setting — but a pane can write its own for
    /// something the schema cannot know, such as a caveat about where a
    /// setting takes effect.
    pub detail: Option<Cow<'static, str>>,
    pub control: Control,
    /// Row differs from the inherited value, so it offers a reset.
    pub overridden: bool,
    /// Change is persisted but needs a restart to take effect.
    pub restart_required: bool,
    /// The `org.otto.Settings` identifier this row edits. `None` means the row
    /// is not wired to the compositor yet and is display-only.
    pub id: Option<&'static str>,
}

impl Row {
    pub(crate) fn new(label: &'static str, control: Control) -> Self {
        Self {
            label,
            detail: None,
            control,
            overridden: false,
            restart_required: false,
            id: None,
        }
    }

    /// Bind the row to a settings identifier, and take its current value and
    /// override state from the compositor when it is serving them.
    pub(crate) fn id(mut self, id: &'static str) -> Self {
        self.id = Some(id);
        self.overridden = settings_client::is_overridden(id);

        let desc = settings_client::describe(id);
        let step = desc.as_ref().and_then(|d| d.step);

        // The schema owns the range. A pane's min/max is a placeholder written
        // before the contract existed, and where the two disagree the pane is
        // simply wrong — a slider that stops at 1000 ms cannot reach a value
        // the compositor will happily accept up to 2000.
        if let Some(desc) = desc.as_ref() {
            if let Control::Slider {
                value,
                min,
                max,
                readout,
            } = self.control
            {
                self.control = Control::Slider {
                    value,
                    min: desc.min.map(|m| m as f32).unwrap_or(min),
                    max: desc.max.map(|m| m as f32).unwrap_or(max),
                    readout,
                };
            }
        }

        if let Some(value) = settings_client::value(id) {
            self.control = self.control.with_value(&value, step);
        }
        // Only badge a setting that has actually been changed and is waiting
        // on a restart. Almost everything is `Apply::Restart` today, so
        // badging from the schema alone would put a pill on every row and say
        // nothing.
        self.restart_required = settings_client::is_pending_restart(id);

        // Help text comes from the schema so there is one source of truth: the
        // compositor owns the setting, so it owns the sentence explaining it.
        // A pane that already wrote its own detail keeps it.
        if self.detail.is_none() {
            if let Some(desc) = desc {
                if !desc.description.is_empty() {
                    self.detail = Some(Cow::Owned(desc.description));
                }
            }
        }
        self
    }

    pub(crate) fn detail(mut self, detail: impl Into<Cow<'static, str>>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// No pane marks a row overridden since the shortcuts group stopped being
    /// a static list; kept because the badge it sets is drawn and the next
    /// setting that can be overridden will want it.
    #[allow(dead_code)]
    pub(crate) fn overridden(mut self) -> Self {
        self.overridden = true;
        self
    }

    /// Mark a row as restart-required by hand.
    ///
    /// Bound rows take this from the served schema instead, so this is only
    /// for rows that have no identifier yet and are known to need a restart.
    #[allow(dead_code)]
    pub(crate) fn restart(mut self) -> Self {
        self.restart_required = true;
        self
    }
}

impl Control {
    /// Replace the control's value with one served by the compositor, keeping
    /// everything the schema does not own — the readout format the app chose
    /// for it. `step` sets how many decimals that readout needs.
    fn with_value(self, value: &Value, step: Option<f64>) -> Self {
        match (self, value) {
            (Control::Toggle(_), Value::Bool(on)) => Control::Toggle(*on),
            (
                Control::Slider {
                    min, max, readout, ..
                },
                value,
            ) => {
                let new = value.as_f32().unwrap_or(min);
                Control::Slider {
                    // The readout is a format, not a value: re-render it
                    // around the new number rather than keeping stale text.
                    readout: reformat_readout(&readout, new, step),
                    value: new,
                    min,
                    max,
                }
            }
            (Control::Select(_), Value::Text(text)) => Control::Select(text.clone()),
            (Control::Text(_), Value::Text(text)) => Control::Text(text.clone()),
            // A list setting edited as text — preferred languages, xkb options
            // — reads and writes the comma-separated form.
            (Control::Text(_), Value::List(items)) => Control::Text(items.join(", ")),
            (Control::File(_), Value::Text(text)) => Control::File(text.clone()),
            // Enumerated colour settings (the accent) carry a name rather than
            // hex, so a well that only understood hex drew them as black.
            (Control::Color(_), Value::Text(text)) => Control::Color(
                parse_hex(text)
                    .or_else(|| named_argb(text))
                    .unwrap_or(0xFF000000),
            ),
            (Control::Value(_), Value::List(items)) => Control::Value(items.join(", ")),
            (Control::Value(_), Value::Text(text)) => Control::Value(text.clone()),
            // A value whose type does not fit the control it was bound to:
            // keep drawing the placeholder rather than inventing something.
            (control, _) => control,
        }
    }
}

/// Rebuild a readout string around a new number, preserving whatever unit or
/// formatting the pane chose ("24 px", "150%", "0.50").
fn reformat_readout(previous: &str, value: f32, step: Option<f64>) -> String {
    if previous.ends_with('%') {
        format!("{:.0}%", value * 100.0)
    } else if let Some(unit) = previous.split_once(char::is_whitespace).map(|(_, u)| u) {
        format!("{value:.0} {unit}")
    } else if previous.contains('.') {
        format!("{value:.precision$}", precision = decimals(step))
    } else {
        format!("{value:.0}")
    }
}

/// Decimals a readout needs to show every position its slider can stop on.
/// Showing more is how `0.0` became `-0.01`; showing fewer would make two
/// distinct steps read identically.
fn decimals(step: Option<f64>) -> usize {
    match step {
        Some(step) if step >= 1.0 => 0,
        Some(step) if step >= 0.1 => 1,
        _ => 2,
    }
}

/// The compositor's accent palette, keyed by the name it stores.
///
/// These are the values `src/theme/colors_dark.rs` paints with, so the swatch
/// the user picks is the colour they get.
pub fn named_argb(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "red" => 0xFFFF453A,
        "orange" => 0xFFFF9F0A,
        "yellow" => 0xFFFFD60A,
        "green" => 0xFF32D74B,
        "mint" => 0xFF66D4CF,
        "teal" => 0xFF6AC4DC,
        "cyan" => 0xFF5AC8F5,
        "blue" => 0xFF0A84FF,
        "indigo" => 0xFF5E5CE6,
        "purple" => 0xFFBF5AF2,
        "pink" => 0xFFFF375F,
        "gray" | "grey" | "graphite" => 0xFF98989D,
        "brown" => 0xFFAC8E68,
        _ => return None,
    })
}

fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    u32::from_str_radix(digits, 16)
        .ok()
        .map(|rgb| 0xFF00_0000 | rgb)
}

/// A titled run of rows inside a pane.
pub struct Group {
    /// `Cow` rather than `&'static str` because the displays pane titles a
    /// group with the name of the screen the canvas has selected.
    pub title: Option<Cow<'static, str>>,
    pub rows: Vec<Row>,
}

pub struct Pane {
    pub name: &'static str,
    /// Sidebar glyph name, drawn by `glyphs::draw`.
    pub icon: &'static str,
    pub groups: Vec<Group>,
}

/// The eight panes from the spec, in sidebar order.
pub fn panes() -> Vec<Pane> {
    vec![
        panes::general::build(),
        panes::displays::build(),
        panes::dock::build(),
        panes::keyboard::build(),
        panes::pointing::build(),
        panes::sound::build(),
        panes::power::build(),
        panes::lock_and_login::build(),
    ]
}

pub(crate) fn group(title: impl Into<Cow<'static, str>>, rows: Vec<Row>) -> Group {
    Group {
        title: Some(title.into()),
        rows,
    }
}

/// A group with no heading of its own — its rows read as the pane's first
/// block, under the pane's own title.
///
/// Separate from [`group`] rather than a `None` passed to it: the title is
/// generic now (the Displays pane names its group after the selected screen),
/// and `None` on its own tells the compiler nothing about which type it is
/// `None` of.
pub(crate) fn untitled(rows: Vec<Row>) -> Group {
    Group { title: None, rows }
}

/// What is behind an output.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// A connector with a panel on the end of it.
    Physical,
    /// A headless output streamed over PipeWire — what `otto-rdp` serves to a
    /// remote client, and what an AirPlay receiver consumes. See
    /// `src/virtual_output/` in the compositor.
    Virtual,
}

/// One mode a display can be driven at, as `wl_output` reports it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    /// Millihertz — the units `wl_output` uses. Zero where the compositor has
    /// no meaningful rate to report, which is the normal case for a virtual
    /// output.
    pub refresh_mhz: i32,
}

impl Mode {
    /// The mode's resolution, as the Resolution pop-up lists it.
    pub fn resolution(&self) -> String {
        format!("{} \u{00d7} {}", self.width, self.height)
    }

    /// The mode's refresh rate, as the Refresh rate pop-up lists it. Two
    /// decimals because 59.94 and 60.00 are different modes and a display
    /// often offers both.
    pub fn refresh(&self) -> String {
        format!("{:.2} Hz", self.refresh_mhz as f32 / 1000.0)
    }
}

/// Outputs shown in the Displays arrangement canvas. Positions and sizes are
/// in compositor logical pixels; the canvas scales them to fit.
#[derive(Clone)]
pub struct Output {
    /// Connector name for a physical output, the configured name for a
    /// virtual one. Owned rather than `&'static str` because a virtual output
    /// added from the pane is named at runtime.
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub primary: bool,
    pub kind: OutputKind,
    /// Whether the compositor is driving this output. A disabled one keeps
    /// its place in the arrangement so re-enabling it puts it back where it
    /// was.
    pub enabled: bool,
    /// Every mode the display can be driven at, in the order the compositor
    /// announced them. Empty only for an output that reported none.
    pub modes: Vec<Mode>,
    /// Index into `modes` of the one in use — what the two pop-ups show.
    pub mode: usize,
    /// Added in the pane rather than found by the probe. Such an output has no
    /// counterpart in the compositor, so [`sync`] has to carry it across
    /// instead of expecting to see it again.
    local: bool,
}

impl Output {
    pub fn is_virtual(&self) -> bool {
        matches!(self.kind, OutputKind::Virtual)
    }

    /// The mode this display is being driven at.
    pub fn current_mode(&self) -> Option<Mode> {
        self.modes.get(self.mode).copied()
    }

    /// The resolutions on offer, each listed once. A display commonly
    /// advertises the same size at several rates; the Resolution pop-up is
    /// about size alone, and the rate is the other pop-up's business.
    pub fn resolutions(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for mode in &self.modes {
            let label = mode.resolution();
            if !seen.contains(&label) {
                seen.push(label);
            }
        }
        seen
    }

    /// The rates on offer *at the current resolution*. Listing every rate the
    /// display supports would offer combinations it cannot actually drive.
    pub fn refresh_rates(&self) -> Vec<String> {
        let Some(current) = self.current_mode() else {
            return Vec::new();
        };
        let mut seen: Vec<String> = Vec::new();
        for mode in &self.modes {
            if (mode.width, mode.height) != (current.width, current.height) {
                continue;
            }
            let label = mode.refresh();
            if !seen.contains(&label) {
                seen.push(label);
            }
        }
        seen
    }
}

/// The arrangement the Displays pane is showing, and which screen in it the
/// pane's rows belong to.
///
/// **Nothing here reaches the compositor.** `org.otto.Settings` serves no
/// per-output setting at all (see the doc comment on [`crate::panes::displays`]),
/// so an arrangement the user edits lives exactly as long as the window does.
/// It is kept anyway because the alternative is worse: a canvas that ignored
/// every click, and add/remove buttons that did nothing, would read as broken
/// rather than as unwired.
struct Arrangement {
    outputs: Vec<Output>,
    /// Index into `outputs`. Kept in range by every mutator here, so the pane
    /// can index with it directly.
    selected: usize,
}

impl Default for Arrangement {
    /// Empty. The compositor's outputs arrive after the app has started, so
    /// there is nothing to probe yet at the moment this is first asked for —
    /// see [`sync`], which fills it in and keeps it current.
    fn default() -> Self {
        Self {
            outputs: Vec::new(),
            selected: 0,
        }
    }
}

/// Bring the arrangement up to date with what the compositor is driving.
///
/// Run on every read rather than once at startup, because the answer changes:
/// the outputs are announced after the app is up — an arrangement built at
/// window-setup time is always empty — and they come and go with hotplug
/// afterwards.
///
/// Reconciled rather than replaced, so the edits made in the pane survive it.
/// An output is matched by name; what the compositor owns (its modes, and its
/// size) is taken from the probe, and what only this app knows (where the
/// pane has been told to put it, whether it has been switched off, which
/// screen is selected) is kept.
fn sync() {
    let probed = probe();
    let mut arrangement = arrangement().write().unwrap();

    let selected_name = arrangement
        .outputs
        .get(arrangement.selected)
        .map(|o| o.name.clone());

    let held = std::mem::take(&mut arrangement.outputs);
    let mut outputs: Vec<Output> = probed
        .into_iter()
        .map(|mut output| {
            if let Some(previous) = held.iter().find(|o| o.name == output.name) {
                output.x = previous.x;
                output.y = previous.y;
                output.enabled = previous.enabled;
                output.primary = previous.primary;
                // Keep the mode the pane picked, but only while the display
                // still offers it: a panel that came back on a different port
                // may not.
                if let Some(mode) = previous.current_mode() {
                    if let Some(index) = output.modes.iter().position(|m| *m == mode) {
                        output.mode = index;
                    }
                }
            }
            output
        })
        .collect();

    // Virtual outputs added in the pane are not running anywhere, so the probe
    // cannot report them — they would disappear the moment they were added.
    outputs.extend(held.into_iter().filter(|o| o.local));

    // Nothing in the protocol says which display is primary, and
    // `org.otto.Settings` serves no per-output setting to ask (see the doc
    // comment on `crate::panes::displays`). The compositor announces its
    // outputs in the order it brought them up, which starts with the one it
    // made primary, so the first is the closest thing to an answer available.
    if !outputs.iter().any(|o| o.primary) {
        if let Some(first) = outputs.first_mut() {
            first.primary = true;
        }
    }

    arrangement.selected = selected_name
        .and_then(|name| outputs.iter().position(|o| o.name == name))
        .unwrap_or(0);
    arrangement.outputs = outputs;
}

/// Ask the compositor what it is driving.
fn probe() -> Vec<Output> {
    otto_kit::AppContext::outputs()
        .into_iter()
        .enumerate()
        .map(|(index, info)| {
            let current = info.modes.iter().position(|m| m.current).unwrap_or(0);
            let modes: Vec<Mode> = info
                .modes
                .iter()
                .map(|m| Mode {
                    width: m.dimensions.0,
                    height: m.dimensions.1,
                    refresh_mhz: m.refresh_rate,
                })
                .collect();
            // Logical size and position are what the arrangement lays out in;
            // the compositor sends them through `xdg_output`. The fallback
            // divides the mode by the integer scale, which is the same figure
            // whenever the scale is not fractional.
            let (width, height) = info.logical_size.unwrap_or_else(|| {
                let (w, h) = modes
                    .get(current)
                    .map(|m| (m.width, m.height))
                    .unwrap_or((1920, 1080));
                (w / info.scale_factor.max(1), h / info.scale_factor.max(1))
            });
            let (x, y) = info.logical_position.unwrap_or(info.location);
            Output {
                // `name` is the connector for a physical output. It is only
                // absent on a compositor too old to send it, where the model
                // string is the only handle left.
                name: info.name.clone().unwrap_or_else(|| info.model.clone()),
                x: x as f32,
                y: y as f32,
                width: width as f32,
                height: height as f32,
                primary: index == 0,
                // What `src/virtual_output/mod.rs` stamps on the outputs it
                // creates. Nothing else in the protocol distinguishes a
                // headless output from a panel.
                kind: if info.make == "Otto" && info.model == "Virtual" {
                    OutputKind::Virtual
                } else {
                    OutputKind::Physical
                },
                // An output the compositor has turned off has no `wl_output`
                // at all, so everything the probe returns is running.
                enabled: true,
                mode: current,
                modes,
                local: false,
            }
        })
        .collect()
}

fn arrangement() -> &'static RwLock<Arrangement> {
    static ARRANGEMENT: OnceLock<RwLock<Arrangement>> = OnceLock::new();
    ARRANGEMENT.get_or_init(|| RwLock::new(Arrangement::default()))
}

/// Every output in the arrangement, in the order the canvas draws them — which
/// is also the order [`selected_output`] and [`select_output`] index into.
pub fn outputs() -> Vec<Output> {
    sync();
    arrangement().read().unwrap().outputs.clone()
}

/// Index of the screen whose settings the pane is showing.
pub fn selected_output() -> usize {
    sync();
    arrangement().read().unwrap().selected
}

/// Show `index`'s settings. Out-of-range indices are ignored rather than
/// clamped: they can only come from a hit test that disagrees with the canvas,
/// and silently selecting a different screen would hide that.
pub fn select_output(index: usize) {
    let mut arrangement = arrangement().write().unwrap();
    if index < arrangement.outputs.len() {
        arrangement.selected = index;
    }
}

/// Run `edit` on the selected output.
fn with_selected(edit: impl FnOnce(&mut Output)) {
    let mut arrangement = arrangement().write().unwrap();
    let selected = arrangement.selected;
    if let Some(output) = arrangement.outputs.get_mut(selected) {
        edit(output);
    }
}

/// Turn the selected screen on or off.
pub fn toggle_selected_enabled() {
    with_selected(|output| output.enabled = !output.enabled);
}

/// Make the selected screen the primary one — the display the dock and the
/// bar live on. Exactly one output is primary, so this takes it off the
/// others rather than toggling.
pub fn make_selected_primary() {
    let mut arrangement = arrangement().write().unwrap();
    let selected = arrangement.selected;
    for (index, output) in arrangement.outputs.iter_mut().enumerate() {
        output.primary = index == selected;
    }
}

/// Drive the selected screen at `resolution`, keeping its refresh rate where
/// the display offers that size at the same rate. The labels come from
/// [`Mode::resolution`], so they always match a mode this output has.
pub fn set_selected_resolution(resolution: &str) {
    with_selected(|output| {
        let rate = output.current_mode().map(|m| m.refresh_mhz);
        let matches = |m: &Mode| m.resolution() == resolution;
        // The same rate first, so changing only the resolution does not
        // silently change the refresh too.
        let index = output
            .modes
            .iter()
            .position(|m| matches(m) && Some(m.refresh_mhz) == rate)
            .or_else(|| output.modes.iter().position(matches));
        if let Some(index) = index {
            output.mode = index;
        }
    });
}

/// Drive the selected screen at `rate`, at the resolution it is already on.
pub fn set_selected_refresh(rate: &str) {
    with_selected(|output| {
        let Some(current) = output.current_mode() else {
            return;
        };
        let index = output.modes.iter().position(|m| {
            (m.width, m.height) == (current.width, current.height) && m.refresh() == rate
        });
        if let Some(index) = index {
            output.mode = index;
        }
    });
}

/// Resize the selected screen.
///
/// Only meaningful for a virtual output: a panel is driven at one of the
/// modes its connector advertises, and there is nothing to type. A headless
/// output has no such list — its one mode is whatever it is told to be — so
/// the pane gives it plain fields instead of pop-ups.
pub fn set_selected_size(width: Option<i32>, height: Option<i32>) {
    with_selected(|output| {
        let mut mode = output.current_mode().unwrap_or(Mode {
            width: output.width as i32,
            height: output.height as i32,
            refresh_mhz: 60_000,
        });
        if let Some(width) = width {
            mode.width = width.max(1);
        }
        if let Some(height) = height {
            mode.height = height.max(1);
        }
        output.modes = vec![mode];
        output.mode = 0;
        // A virtual output runs unscaled, so its mode is also its size in the
        // arrangement's logical pixels.
        output.width = mode.width as f32;
        output.height = mode.height as f32;
    });
}

/// Set the selected screen's refresh rate, in whole hertz. Virtual outputs
/// only, for the same reason as [`set_selected_size`].
pub fn set_selected_refresh_hz(hz: f32) {
    with_selected(|output| {
        let Some(mut mode) = output.current_mode() else {
            return;
        };
        mode.refresh_mhz = ((hz.max(1.0)) * 1000.0) as i32;
        output.modes = vec![mode];
        output.mode = 0;
    });
}

/// Move the selected screen's top-left corner.
pub fn set_selected_position(x: Option<f32>, y: Option<f32>) {
    with_selected(|output| {
        if let Some(x) = x {
            output.x = x;
        }
        if let Some(y) = y {
            output.y = y;
        }
    });
}

/// Add a virtual output, and select it so its rows are the ones on screen.
///
/// Placed to the right of everything else at the resolution
/// `otto_config.example.toml` uses, since a new headless output has no
/// hardware to take a mode from.
pub fn add_virtual_output() {
    let mut arrangement = arrangement().write().unwrap();
    // Names have to be unique — the compositor keys virtual outputs by name —
    // so count up past whatever is already there rather than past the number
    // of virtual outputs, which repeats a name after a removal.
    let next = (1..)
        .find(|n| {
            let name = format!("virtual-{n}");
            !arrangement.outputs.iter().any(|o| o.name == name)
        })
        .expect("an unused name exists");
    let right = arrangement
        .outputs
        .iter()
        .map(|o| o.x + o.width)
        .fold(0.0_f32, f32::max);

    arrangement.outputs.push(Output {
        name: format!("virtual-{next}"),
        x: right,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
        primary: false,
        kind: OutputKind::Virtual,
        enabled: true,
        local: true,
        // A headless output has no hardware to take a mode list from, so the
        // one it is created at is the only one it offers.
        modes: vec![Mode {
            width: 1920,
            height: 1080,
            refresh_mhz: 60_000,
        }],
        mode: 0,
    });
    arrangement.selected = arrangement.outputs.len() - 1;
}

/// Remove the selected screen, if it is a virtual one.
///
/// Physical outputs are not removable: the arrangement describes hardware
/// that is plugged in, and a panel does not stop existing because the pane
/// stopped listing it. Returns whether anything was removed.
pub fn remove_selected_virtual_output() -> bool {
    let mut arrangement = arrangement().write().unwrap();
    let selected = arrangement.selected;
    match arrangement.outputs.get(selected) {
        Some(output) if output.is_virtual() => {}
        _ => return false,
    }
    arrangement.outputs.remove(selected);
    arrangement.selected = selected.min(arrangement.outputs.len().saturating_sub(1));
    true
}

/// How many virtual outputs the arrangement holds, for the pane's summary.
pub fn virtual_output_count() -> usize {
    arrangement()
        .read()
        .unwrap()
        .outputs
        .iter()
        .filter(|o| o.is_virtual())
        .count()
}
