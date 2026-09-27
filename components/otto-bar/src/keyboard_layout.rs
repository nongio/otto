//! The keyboard layout indicator: which layout is active, and a menu to pick
//! another.
//!
//! The compositor owns the keymap, so the bar asks it: `GetInputs` on
//! `org.otto.Shell1` once, and then `InputChanged` for every switch and every
//! change to the configured layouts. The shapes are sway's `GET_INPUTS` and
//! `input` event, with two Otto additions: `otto_layout_codes`, the XKB names
//! the icon is written from, and `otto_show_in_bar`, the setting that decides
//! whether the icon is there at all. See `docs/developer/shell-dbus-api.md`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use futures_util::StreamExt;
use otto_kit::prelude::*;
use skia_safe::{Canvas, Paint, RRect, Rect, TextBlob};
use zbus::{proxy, Connection};

/// The layouts as the compositor last reported them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layouts {
    /// Full names, as the keymap gives them ("English (US)").
    pub names: Vec<String>,
    /// XKB layout names, one per entry in `names` ("us").
    pub codes: Vec<String>,
    /// Index of the active layout.
    pub active: usize,
    /// Whether the user asked for the indicator in the bar.
    pub show: bool,
}

impl Layouts {
    /// What the icon says: the active layout's XKB name in capitals, at most
    /// three letters. Falls back to the start of the full name for a
    /// compositor that sends no codes.
    pub fn short_name(&self) -> String {
        let source = self
            .codes
            .get(self.active)
            .filter(|code| !code.is_empty())
            .or_else(|| self.names.get(self.active))
            .map(String::as_str)
            .unwrap_or("");
        source
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(3)
            .collect::<String>()
            .to_uppercase()
    }

    /// The active layout's full name, for the menu and for a screen reader.
    pub fn active_name(&self) -> String {
        self.names.get(self.active).cloned().unwrap_or_default()
    }
}

#[proxy(
    interface = "org.otto.Shell1",
    default_service = "org.otto.Shell1",
    default_path = "/org/otto/Shell1"
)]
trait Shell {
    fn get_inputs(&self) -> zbus::Result<String>;
    fn run_command(&self, command: &str) -> zbus::Result<Vec<(bool, String)>>;
    #[zbus(signal)]
    fn input_changed(&self, event: String) -> zbus::Result<()>;
}

static STATE: LazyLock<Mutex<Option<Layouts>>> = LazyLock::new(|| Mutex::new(None));
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SESSION_BUS: LazyLock<Mutex<Option<Connection>>> = LazyLock::new(|| Mutex::new(None));

/// The layouts, if the compositor has reported any.
pub fn layouts() -> Option<Layouts> {
    STATE.lock().unwrap().clone()
}

/// Changes whenever what the indicator shows changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Whether the indicator is drawn: the user asked for it, and there is more
/// than one layout to tell apart.
pub fn visible() -> bool {
    layouts().is_some_and(|l| l.show && l.names.len() > 1)
}

fn store(next: Option<Layouts>) {
    let mut state = STATE.lock().unwrap();
    if *state != next {
        *state = next;
        drop(state);
        GENERATION.fetch_add(1, Ordering::Relaxed);
        AppContext::request_wakeup();
    }
}

/// The keyboard among the inputs `GetInputs` lists.
fn keyboard_from_value(input: &serde_json::Value) -> Option<Layouts> {
    if input.get("type")?.as_str()? != "keyboard" {
        return None;
    }
    let strings = |key: &str| -> Vec<String> {
        input
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let names = strings("xkb_layout_names");
    let active = input
        .get("xkb_active_layout_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    Some(Layouts {
        active: active.min(names.len().saturating_sub(1)),
        codes: strings("otto_layout_codes"),
        names,
        show: input
            .get("otto_show_in_bar")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// Read the keyboard out of a `GetInputs` answer.
pub fn parse_inputs(json: &str) -> Option<Layouts> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value.as_array()?.iter().find_map(keyboard_from_value)
}

/// Read the keyboard out of an `InputChanged` event.
pub fn parse_event(json: &str) -> Option<Layouts> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    keyboard_from_value(value.get("input")?)
}

pub fn spawn_watcher() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("keyboard layout indicator: {e}");
        }
    });
}

async fn run_watcher() -> zbus::Result<()> {
    let conn = Connection::session().await?;
    *SESSION_BUS.lock().unwrap() = Some(conn.clone());
    let proxy = ShellProxy::new(&conn).await?;

    let mut events = proxy.receive_input_changed().await?;
    // The compositor's bus name coming and going: a compositor that starts
    // after the bar, or restarts under it, is asked again rather than left
    // showing nothing.
    let mut owners = proxy.inner().receive_owner_changed().await?;

    store(proxy.get_inputs().await.ok().and_then(|j| parse_inputs(&j)));

    loop {
        tokio::select! {
            Some(signal) = events.next() => {
                if let Ok(args) = signal.args() {
                    if let Some(layouts) = parse_event(args.event()) {
                        store(Some(layouts));
                    }
                }
            }
            Some(owner) = owners.next() => {
                if owner.is_some() {
                    store(proxy.get_inputs().await.ok().and_then(|j| parse_inputs(&j)));
                } else {
                    store(None);
                }
            }
            else => break,
        }
    }
    Ok(())
}

/// Make layout `index` the active one. The indicator follows when the
/// compositor announces the switch.
pub fn switch_to(index: usize) {
    let Some(conn) = SESSION_BUS.lock().unwrap().clone() else {
        return;
    };
    tokio::spawn(async move {
        let command = format!("input type:keyboard xkb_switch_layout {index}");
        let result = match ShellProxy::new(&conn).await {
            Ok(proxy) => proxy.run_command(&command).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(results) => {
                for (ok, message) in results {
                    if !ok {
                        tracing::warn!("{command}: {message}");
                    }
                }
            }
            Err(e) => tracing::warn!("{command}: {e}"),
        }
    });
}

/// Height of the keycap outline.
const CAP_HEIGHT: f32 = 16.0;
/// Space between the outline and the letters.
const CAP_PADDING: f32 = 4.0;
/// The narrowest the keycap gets, so "IT" and "US" are the same key.
const CAP_MIN_WIDTH: f32 = 24.0;
const CAP_RADIUS: f32 = 3.5;
const CAP_STROKE: f32 = 1.2;

fn font() -> skia_safe::Font {
    otto_kit::typography::get_font_with_fallback(
        "Inter",
        skia_safe::FontStyle::new(
            skia_safe::font_style::Weight::SEMI_BOLD,
            skia_safe::font_style::Width::NORMAL,
            skia_safe::font_style::Slant::Upright,
        ),
        10.0,
    )
}

/// How wide the indicator is, or zero when it is not drawn.
pub fn width() -> f32 {
    if !visible() {
        return 0.0;
    }
    let text = layouts().map(|l| l.short_name()).unwrap_or_default();
    let text_w = font().measure_str(&text, None).0;
    (text_w + CAP_PADDING * 2.0).max(CAP_MIN_WIDTH).ceil()
}

/// Draw the keycap at `x`, vertically centred in a bar `height` tall.
pub fn draw(canvas: &Canvas, x: f32, height: f32, color: skia_safe::Color) {
    let w = width();
    if w <= 0.0 {
        return;
    }
    let text = layouts().map(|l| l.short_name()).unwrap_or_default();
    let top = ((height - CAP_HEIGHT) / 2.0).round();
    let half = CAP_STROKE / 2.0;
    let cap = Rect::from_xywh(
        x + half,
        top + half,
        w - CAP_STROKE,
        CAP_HEIGHT - CAP_STROKE,
    );

    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(skia_safe::paint::Style::Stroke);
    stroke.set_stroke_width(CAP_STROKE);
    stroke.set_color(color);
    canvas.draw_rrect(RRect::new_rect_xy(cap, CAP_RADIUS, CAP_RADIUS), &stroke);

    let font = font();
    let text_w = font.measure_str(&text, None).0;
    let (_, metrics) = font.metrics();
    let baseline = top + (CAP_HEIGHT + metrics.cap_height) / 2.0;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    if let Some(blob) = TextBlob::new(&text, &font) {
        canvas.draw_text_blob(&blob, (x + (w - text_w) / 2.0, baseline), &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUTS: &str = r#"[
        {"identifier":"otto:pointer","type":"pointer"},
        {"identifier":"otto:keyboard","name":"Keyboard","type":"keyboard",
         "xkb_layout_names":["English (US)","Italian"],
         "xkb_active_layout_index":1,
         "xkb_active_layout_name":"Italian",
         "otto_layout_codes":["us","it"],
         "otto_show_in_bar":true}
    ]"#;

    #[test]
    fn the_keyboard_is_found_among_the_inputs() {
        let layouts = parse_inputs(INPUTS).expect("a keyboard");
        assert_eq!(layouts.names, ["English (US)", "Italian"]);
        assert_eq!(layouts.codes, ["us", "it"]);
        assert_eq!(layouts.active, 1);
        assert!(layouts.show);
        assert_eq!(layouts.short_name(), "IT");
        assert_eq!(layouts.active_name(), "Italian");
    }

    #[test]
    fn an_event_carries_the_same_keyboard() {
        let event = r#"{"change":"xkb_layout","input":{"type":"keyboard",
            "xkb_layout_names":["English (US)","Italian"],
            "xkb_active_layout_index":0,"otto_layout_codes":["us","it"],
            "otto_show_in_bar":false}}"#;
        let layouts = parse_event(event).expect("a keyboard");
        assert_eq!(layouts.active, 0);
        assert!(!layouts.show);
        assert_eq!(layouts.short_name(), "US");
    }

    #[test]
    fn nothing_usable_is_none() {
        assert_eq!(parse_inputs("not json"), None);
        assert_eq!(parse_inputs(r#"[{"type":"pointer"}]"#), None);
        assert_eq!(parse_event(r#"{"change":"xkb_layout"}"#), None);
    }

    #[test]
    fn the_short_name_falls_back_to_the_full_name() {
        let layouts = Layouts {
            names: vec!["Deutsch".into(), "Français".into()],
            codes: Vec::new(),
            active: 1,
            show: true,
        };
        assert_eq!(layouts.short_name(), "FRA");
    }

    #[test]
    fn an_index_past_the_end_is_clamped() {
        let json = r#"[{"type":"keyboard","xkb_layout_names":["A","B"],
            "xkb_active_layout_index":7}]"#;
        assert_eq!(parse_inputs(json).expect("keyboard").active, 1);
    }
}
