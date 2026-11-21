//! The sound indicator: the default output's volume as a speaker glyph, and
//! the sound menu under it.
//!
//! The state is the PipeWire graph, followed live through
//! `otto-audio-graph` on a thread of its own, which never polls. Each change
//! is boiled down to a [`Snapshot`] of what the indicator and the menu show,
//! and the bar wakes only when that changes. Volumes and defaults are the
//! session manager's, so a change here shows in the Audio app, in Settings
//! and in any other mixer. Whether the indicator is there at all is the
//! compositor's `topbar.show_sound` setting. See
//! `specs/audio-routing.md`, "The top bar sound menu".

// Rust guideline compliant 2026-02-21

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use otto_audio_graph::{Connection, Endpoint, Event, Graph, Node};
use otto_kit::components::menu_item::{MenuItem as KitMenuItem, MenuItemIcon, SliderChange};
use otto_kit::components::slider::ScrollSteps;
use otto_kit::dbus::settings::SettingsProxy;
use otto_kit::prelude::*;
use otto_kit::skia::{Canvas, Color, Paint, PathBuilder, Rect};

use crate::battery::with_alpha;
use crate::settings::get_bool;

/// The setting that shows or hides the indicator.
const SHOW_ID: &str = "topbar.show_sound";

/// The program "Audio…" opens.
const AUDIO_APP: &str = "otto-audio";

/// How far a scroll step or a menu wheel moves the volume: 5 %.
const STEP: f32 = 0.05;

/// How long a volume the bar asked for stands in for the one the graph
/// reports: long enough for the server to answer, so quick scroll steps add
/// up rather than each starting from the old volume again.
const PENDING_FOR: Duration = Duration::from_millis(800);

/// What the indicator draws when there is a device to draw for.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// Its node id, which commands name.
    pub id: u32,
    /// The name it shows.
    pub title: String,
    /// From 0.0 to 1.0, as a slider shows it.
    pub volume: f32,
    pub muted: bool,
}

/// What kind of device an output or input is, shown beside its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bluetooth,
    Usb,
    Hdmi,
    /// An AirPlay receiver on the network, which plays about 2 s late.
    AirPlay,
    /// Made in software: a null sink, a loopback, an effect.
    Virtual,
    /// A card's own speakers, jack or microphone.
    BuiltIn,
}

/// One output or input to pick.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// `node.name`, which the default is set by.
    pub name: String,
    pub title: String,
    pub kind: Kind,
    pub default: bool,
}

/// One app playing sound.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub name: String,
    pub nodes: Vec<u32>,
    pub volume: f32,
    pub muted: bool,
}

/// Everything the indicator and the menu show.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// A PipeWire server answered.
    pub connected: bool,
    pub output: Option<Device>,
    pub input: Option<Device>,
    pub outputs: Vec<Choice>,
    pub inputs: Vec<Choice>,
    pub apps: Vec<App>,
}

impl Snapshot {
    /// What `graph` shows.
    #[must_use]
    pub fn of(graph: &Graph) -> Self {
        let device = |endpoint| {
            let node = graph.default_node(endpoint)?;
            let volume = graph.volume(node.id)?;
            Some(Device {
                id: node.id,
                title: node.title().to_string(),
                volume: volume.linear(),
                muted: volume.mute,
            })
        };
        let choices = |endpoint| {
            let default = graph.default_name(endpoint);
            graph
                .endpoints(endpoint)
                .into_iter()
                .map(|node| Choice {
                    name: node.name().to_string(),
                    title: node.title().to_string(),
                    kind: kind_of(node),
                    default: Some(node.name()) == default,
                })
                .collect()
        };
        Self {
            connected: graph.is_connected(),
            output: device(Endpoint::Output),
            input: device(Endpoint::Input),
            outputs: choices(Endpoint::Output),
            inputs: choices(Endpoint::Input),
            apps: graph
                .app_streams()
                .into_iter()
                .map(|app| App {
                    volume: app.volume.linear(),
                    muted: app.volume.mute,
                    name: app.app,
                    nodes: app.nodes,
                })
                .collect(),
        }
    }
}

/// What kind of device `node` is, from what the session manager says of it.
fn kind_of(node: &Node) -> Kind {
    if node.is_airplay() {
        return Kind::AirPlay;
    }
    if !node.is_device() {
        return Kind::Virtual;
    }
    let api = node.prop("device.api").unwrap_or_default();
    let bus = node.prop("device.bus").unwrap_or_default();
    let name = node.name();
    if api == "bluez5" || bus == "bluetooth" {
        Kind::Bluetooth
    } else if bus == "usb" {
        Kind::Usb
    } else if name.contains("hdmi") || name.contains("iec958") {
        Kind::Hdmi
    } else {
        Kind::BuiltIn
    }
}

/// The graph as last reported, applied on the PipeWire thread.
static GRAPH: LazyLock<Mutex<Graph>> = LazyLock::new(|| Mutex::new(Graph::new()));
/// What the indicator and the menu show, made from the graph.
static SNAPSHOT: LazyLock<Mutex<Snapshot>> = LazyLock::new(|| Mutex::new(Snapshot::default()));
/// Bumped whenever the snapshot or the setting changes.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Whether the person wants the indicator. On until the compositor says
/// otherwise, so a bar without Otto still shows the volume.
static SHOWN: AtomicBool = AtomicBool::new(true);
/// The connection commands go through.
static CONNECTION: OnceLock<Connection> = OnceLock::new();
/// A volume the bar asked for and the graph may not report yet: node,
/// volume, and when.
static PENDING: Mutex<Option<(u32, f32, Instant)>> = Mutex::new(None);
/// Scroll travel over the indicator, counted out in steps.
static WHEEL: Mutex<ScrollSteps> = Mutex::new(ScrollSteps::new());

/// The snapshot as last made.
pub fn snapshot() -> Snapshot {
    SNAPSHOT.lock().unwrap().clone()
}

/// Changes whenever anything the indicator or its menu shows changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn bump() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    AppContext::request_wakeup();
}

/// Whether the indicator is drawn.
pub fn visible() -> bool {
    SHOWN.load(Ordering::Relaxed) && SNAPSHOT.lock().unwrap().connected
}

// ---------------------------------------------------------------------------
// Watchers
// ---------------------------------------------------------------------------

/// Follow the graph and the setting. Idempotent.
pub fn spawn_watcher() {
    CONNECTION.get_or_init(|| Connection::start("otto-bar", on_event));
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async {
        if let Err(e) = follow_setting().await {
            tracing::warn!("sound indicator setting: {e}");
        }
    });
}

/// One change in the graph, on the PipeWire thread: applied, then the bar
/// woken if what it shows moved.
fn on_event(event: Event) {
    let mut graph = GRAPH.lock().unwrap();
    graph.apply(event);
    // Until the first sync the graph is still filling in: nothing worth
    // drawing yet, and a snapshot per object would be wasted.
    if graph.is_connected() && !graph.is_synced() {
        return;
    }
    let next = Snapshot::of(&graph);
    drop(graph);
    let mut slot = SNAPSHOT.lock().unwrap();
    if *slot != next {
        *slot = next;
        drop(slot);
        bump();
    }
}

async fn follow_setting() -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let proxy = SettingsProxy::new(&conn).await?;
    let mut changes = proxy.receive_changed().await?;
    let mut owners = proxy.inner().receive_owner_changed().await?;
    read_setting(&proxy).await;
    loop {
        tokio::select! {
            Some(signal) = changes.next() => {
                if signal.args().is_ok_and(|args| args.values.contains_key(SHOW_ID)) {
                    read_setting(&proxy).await;
                }
            }
            Some(owner) = owners.next() => {
                if owner.is_some() {
                    read_setting(&proxy).await;
                }
            }
            else => break,
        }
    }
    Ok(())
}

async fn read_setting(proxy: &SettingsProxy<'_>) {
    if let Some(show) = get_bool(proxy, SHOW_ID).await {
        if SHOWN.swap(show, Ordering::Relaxed) != show {
            bump();
        }
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn connection() -> Option<&'static Connection> {
    CONNECTION.get()
}

/// Set node `id`'s volume, remembering it until the graph catches up.
fn set_volume(id: u32, volume: f32) {
    let volume = volume.clamp(0.0, 1.0);
    *PENDING.lock().unwrap() = Some((id, volume, Instant::now()));
    if let Some(connection) = connection() {
        connection.set_volume(id, volume);
    }
}

/// The volume a new step starts from: the one just asked for while the
/// graph may still report the old one.
fn current_volume(device: &Device) -> f32 {
    match *PENDING.lock().unwrap() {
        Some((id, volume, at)) if id == device.id && at.elapsed() < PENDING_FOR => volume,
        _ => device.volume,
    }
}

/// Scroll over the indicator: `value120` from a wheel, else `absolute`
/// travel. Up is louder.
pub fn scroll(value120: i32, absolute: f64) {
    let steps = WHEEL.lock().unwrap().add(value120, absolute);
    if steps == 0 {
        return;
    }
    let Some(output) = snapshot().output else {
        return;
    };
    let from = current_volume(&output);
    // Back on the 5 % grid, so a volume of 43 % steps to 45 % and 40 %.
    let to = ((from - steps as f32 * STEP) / STEP).round() * STEP;
    set_volume(output.id, to);
}

/// Mute the default output, or unmute it.
pub fn toggle_mute() {
    if let (Some(output), Some(connection)) = (snapshot().output, connection()) {
        connection.set_mute(output.id, !output.muted);
    }
}

/// A slider in the menu changed.
pub fn slider_changed(action_id: &str, change: SliderChange) {
    let snapshot = snapshot();
    let nodes: Vec<u32> = match action_id {
        "output" => snapshot.output.map(|d| vec![d.id]).unwrap_or_default(),
        "input" => snapshot.input.map(|d| vec![d.id]).unwrap_or_default(),
        _ => match action_id.strip_prefix("app:") {
            Some(name) => snapshot
                .apps
                .into_iter()
                .find(|a| a.name == name)
                .map(|a| a.nodes)
                .unwrap_or_default(),
            None => Vec::new(),
        },
    };
    let Some(connection) = connection() else {
        return;
    };
    for id in nodes {
        match change {
            SliderChange::Value(volume) => set_volume(id, volume),
            // The toggle is lit while sound passes.
            SliderChange::Toggled(on) => connection.set_mute(id, !on),
        }
    }
}

/// A row of the menu was chosen. Returns whether it was "Audio…", which the
/// caller launches.
pub fn activate(action_id: &str) -> bool {
    if action_id == "audio" {
        return true;
    }
    let (endpoint, name) = if let Some(name) = action_id.strip_prefix("output:") {
        (Endpoint::Output, name)
    } else if let Some(name) = action_id.strip_prefix("input:") {
        (Endpoint::Input, name)
    } else {
        return false;
    };
    if let Some(connection) = connection() {
        connection.set_default(endpoint, name);
    }
    false
}

/// The program "Audio…" runs.
pub fn audio_app() -> &'static str {
    AUDIO_APP
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Bluetooth => otto_kit::t!("bar-sound-kind-bluetooth"),
        Kind::Usb => otto_kit::t!("bar-sound-kind-usb"),
        Kind::Hdmi => otto_kit::t!("bar-sound-kind-hdmi"),
        Kind::AirPlay => otto_kit::t!("bar-sound-kind-airplay"),
        Kind::Virtual => otto_kit::t!("bar-sound-kind-virtual"),
        Kind::BuiltIn => otto_kit::t!("bar-sound-kind-built-in"),
    }
}

/// The pixels a toggle's glyph is drawn at: twice its size in the menu, so
/// it stays sharp on a doubled display.
const TOGGLE_GLYPH_PX: i32 = 28;

/// The colour a toggle's glyph is drawn in: white on the lit accent while
/// sound passes, the text tone on the grey of a muted one.
fn toggle_tint(muted: bool) -> Color {
    if muted {
        AppContext::current_theme().text_primary
    } else {
        Color::WHITE
    }
}

/// A glyph drawn by `paint` into a square, as the pixmap a menu row takes.
/// `paint` draws into a 20-point square.
fn glyph_pixmap(paint: impl FnOnce(&Canvas)) -> MenuItemIcon {
    let size = TOGGLE_GLYPH_PX;
    let Some(mut surface) = otto_kit::skia::surfaces::raster_n32_premul((size, size)) else {
        return MenuItemIcon::Named("audio-volume-high-symbolic".into());
    };
    let canvas = surface.canvas();
    canvas.clear(Color::TRANSPARENT);
    canvas.scale((size as f32 / 20.0, size as f32 / 20.0));
    paint(canvas);
    let info = otto_kit::skia::ImageInfo::new(
        (size, size),
        otto_kit::skia::ColorType::BGRA8888,
        otto_kit::skia::AlphaType::Premul,
        None,
    );
    let mut bgra = vec![0u8; (size * size * 4) as usize];
    if !surface.read_pixels(&info, &mut bgra, (size * 4) as usize, (0, 0)) {
        return MenuItemIcon::Named("audio-volume-high-symbolic".into());
    }
    // ARGB32 in network byte order, as a menu pixmap is.
    let data = bgra
        .chunks_exact(4)
        .flat_map(|p| [p[3], p[2], p[1], p[0]])
        .collect();
    MenuItemIcon::Pixmap {
        data,
        width: size,
        height: size,
    }
}

fn speaker_icon(muted: bool) -> MenuItemIcon {
    let color = toggle_tint(muted);
    // Full volume, so the waves read as "sound" rather than as a level.
    glyph_pixmap(|canvas| draw_glyph(canvas, 0.0, 20.0, color, 1.0, muted))
}

fn microphone_icon(muted: bool) -> MenuItemIcon {
    let color = toggle_tint(muted);
    glyph_pixmap(|canvas| draw_microphone(canvas, color, muted))
}

/// A microphone in a 20-point square: the capsule, its stand and its foot,
/// struck through when `muted`.
fn draw_microphone(canvas: &Canvas, color: Color, muted: bool) {
    let paint = stroke(color, 1.6);
    let capsule = Rect::from_xywh(7.0, 2.5, 6.0, 10.0);
    canvas.draw_round_rect(capsule, 3.0, 3.0, &paint);
    let cradle = Rect::from_xywh(4.0, 4.5, 12.0, 11.0);
    canvas.draw_arc(cradle, 20.0, 140.0, false, &paint);
    canvas.draw_line((10.0, 15.5), (10.0, 18.0), &paint);
    if muted {
        canvas.draw_line((3.5, 2.5), (16.5, 17.5), &paint);
    }
}

/// One of the two device sections: its caption, the default's slider and
/// the devices to pick from.
fn section(
    items: &mut Vec<KitMenuItem>,
    endpoint: Endpoint,
    device: Option<&Device>,
    choices: &[Choice],
) {
    if device.is_none() && choices.is_empty() {
        return;
    }
    let (caption, id, prefix) = match endpoint {
        Endpoint::Output => (otto_kit::t!("bar-sound-output"), "output", "output:"),
        Endpoint::Input => (otto_kit::t!("bar-sound-input"), "input", "input:"),
    };
    if !items.is_empty() {
        items.push(KitMenuItem::separator());
    }
    items.push(KitMenuItem::header(caption));
    if let Some(device) = device {
        let (name, mute, icon) = match endpoint {
            Endpoint::Output => (
                otto_kit::t!("bar-sound-output-volume"),
                otto_kit::t!("bar-sound-mute-output"),
                speaker_icon(device.muted),
            ),
            Endpoint::Input => (
                otto_kit::t!("bar-sound-input-volume"),
                otto_kit::t!("bar-sound-mute-input"),
                microphone_icon(device.muted),
            ),
        };
        items.push(
            KitMenuItem::slider(id, name, device.volume).with_slider_toggle(
                icon,
                !device.muted,
                mute,
            ),
        );
    }
    for choice in choices {
        items.push(
            KitMenuItem::action(choice.title.clone())
                .with_action_id(format!("{prefix}{}", choice.name))
                .with_shortcut(kind_label(choice.kind))
                .with_checked(choice.default),
        );
    }
}

/// The menu's items for `snapshot`.
pub fn menu_items(snapshot: &Snapshot) -> Vec<KitMenuItem> {
    let mut items = Vec::new();
    section(
        &mut items,
        Endpoint::Output,
        snapshot.output.as_ref(),
        &snapshot.outputs,
    );
    // The level meter under the input's slider waits for the meters (Plan,
    // step 2 in specs/audio-routing.md).
    section(
        &mut items,
        Endpoint::Input,
        snapshot.input.as_ref(),
        &snapshot.inputs,
    );
    if !snapshot.apps.is_empty() {
        if !items.is_empty() {
            items.push(KitMenuItem::separator());
        }
        items.push(KitMenuItem::header(otto_kit::t!("bar-sound-apps")));
        for app in &snapshot.apps {
            items.push(
                KitMenuItem::slider(
                    format!("app:{}", app.name),
                    otto_kit::t_owned!("bar-sound-app-volume", app = app.name.clone()),
                    app.volume,
                )
                .with_slider_label(app.name.clone())
                .with_slider_toggle(
                    speaker_icon(app.muted),
                    !app.muted,
                    otto_kit::t_owned!("bar-sound-mute-app", app = app.name.clone()),
                ),
            );
        }
    }
    if !items.is_empty() {
        items.push(KitMenuItem::separator());
    }
    items.push(KitMenuItem::action(otto_kit::t!("bar-sound-open-audio")).with_action_id("audio"));
    items
}

/// The indicator in words, for a screen reader: the output, its volume,
/// and whether it is muted.
pub fn label() -> String {
    match snapshot().output {
        Some(output) if output.muted => {
            otto_kit::t_owned!("bar-sound-label-muted", device = output.title)
        }
        Some(output) => otto_kit::t_owned!(
            "bar-sound-label",
            percent = (output.volume * 100.0).round() as f64,
            device = output.title
        ),
        None => otto_kit::t!("bar-sound-label-none").to_string(),
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The glyph's width: the speaker and its three waves.
const GLYPH_WIDTH: f32 = 20.0;
/// The glyph's height, centred in the bar.
const GLYPH_HEIGHT: f32 = 14.0;
/// Alpha of a wave that is not lit.
const UNLIT: f32 = 0.3;

/// How many of the three waves are lit for `volume`, 0.0 to 1.0: none at
/// silence, then one per third.
#[must_use]
pub fn waves(volume: f32, muted: bool) -> usize {
    if muted || volume <= 0.005 {
        0
    } else {
        ((volume * 3.0).ceil() as usize).clamp(1, 3)
    }
}

/// How wide the indicator is, or zero when it is not drawn.
pub fn width() -> f32 {
    if visible() {
        GLYPH_WIDTH
    } else {
        0.0
    }
}

/// Draw the indicator at `x`, vertically centred in a bar `height` tall.
pub fn draw(canvas: &Canvas, x: f32, height: f32, color: Color) {
    if !visible() {
        return;
    }
    let output = snapshot().output;
    let (volume, muted) = output.map_or((0.0, false), |o| (o.volume, o.muted));
    draw_glyph(canvas, x, height, color, volume, muted);
}

/// The speaker at `x` for `volume`, 0.0 to 1.0, struck through when
/// `muted`.
pub fn draw_glyph(canvas: &Canvas, x: f32, height: f32, color: Color, volume: f32, muted: bool) {
    let lit = waves(volume, muted);
    let top = ((height - GLYPH_HEIGHT) / 2.0).round();
    let cy = top + GLYPH_HEIGHT / 2.0;

    // The speaker: a box and its cone.
    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_color(color);
    let mut body = PathBuilder::new();
    body.move_to((x + 1.0, cy - 2.5));
    body.line_to((x + 4.0, cy - 2.5));
    body.line_to((x + 8.0, cy - 6.0));
    body.line_to((x + 8.0, cy + 6.0));
    body.line_to((x + 4.0, cy + 2.5));
    body.line_to((x + 1.0, cy + 2.5));
    body.close();
    canvas.draw_path(&body.detach(), &fill);

    // The waves, inside out, each lit or faint.
    for (i, radius) in [3.5_f32, 6.5, 9.5].iter().enumerate() {
        let wave_color = if i < lit {
            color
        } else {
            with_alpha(color, UNLIT * f32::from(color.a()) / 255.0)
        };
        let paint = stroke(wave_color, 1.5);
        let oval = Rect::from_xywh(x + 8.0 - radius, cy - radius, radius * 2.0, radius * 2.0);
        // Skia's angles run clockwise from the right: -45° to 45° is the
        // arc on the speaker's open side.
        let sweep = 80.0 - i as f32 * 8.0;
        canvas.draw_arc(oval, -sweep / 2.0, sweep, false, &paint);
    }

    if muted {
        let paint = stroke(color, 1.6);
        canvas.draw_line(
            (x + 1.0, top),
            (x + GLYPH_WIDTH - 1.0, top + GLYPH_HEIGHT),
            &paint,
        );
    }
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(otto_kit::skia::paint::Style::Stroke);
    paint.set_stroke_width(width);
    paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
    paint.set_color(color);
    paint
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_audio_graph::{NodeState, Volume};

    fn node(id: u32, props: &[(&str, &str)]) -> Event {
        Event::Node(Node {
            id,
            props: props
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            state: NodeState::Running,
        })
    }

    fn volume(id: u32, linear: f32, mute: bool) -> Event {
        Event::Volume {
            node: id,
            volume: Volume {
                channels: vec![otto_audio_graph::to_cubic(linear); 2],
                mute,
            },
        }
    }

    /// Speakers (default) and a virtual mix out, one microphone in, and two
    /// apps, one with two streams.
    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        let events = [
            node(
                1,
                &[
                    ("media.class", "Audio/Sink"),
                    ("node.name", "speakers"),
                    ("node.description", "Built-in Speakers"),
                    ("device.id", "40"),
                ],
            ),
            node(
                2,
                &[
                    ("media.class", "Audio/Sink"),
                    ("node.name", "mix"),
                    ("node.description", "Stream mix"),
                ],
            ),
            node(
                3,
                &[
                    ("media.class", "Audio/Source"),
                    ("node.name", "mic"),
                    ("node.description", "Scarlett 2i2"),
                    ("device.id", "41"),
                    ("device.bus", "usb"),
                ],
            ),
            node(
                10,
                &[
                    ("media.class", "Stream/Output/Audio"),
                    ("application.name", "Firefox"),
                ],
            ),
            node(
                11,
                &[
                    ("media.class", "Stream/Output/Audio"),
                    ("application.name", "Firefox"),
                ],
            ),
            node(
                12,
                &[
                    ("media.class", "Stream/Output/Audio"),
                    ("application.name", "Spotify"),
                ],
            ),
            volume(1, 0.7, false),
            volume(2, 1.0, false),
            volume(3, 0.55, true),
            volume(10, 0.65, false),
            volume(11, 0.65, false),
            volume(12, 0.8, false),
            Event::Default {
                endpoint: Endpoint::Output,
                name: Some("speakers".into()),
            },
            Event::Default {
                endpoint: Endpoint::Input,
                name: Some("mic".into()),
            },
            Event::Synced,
        ];
        for event in events {
            graph.apply(event);
        }
        graph
    }

    fn labels(items: &[KitMenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| {
                if i.slider_data().is_some() {
                    format!("[{}]", i.label().unwrap_or_default())
                } else {
                    i.label().unwrap_or("—").to_string()
                }
            })
            .collect()
    }

    #[test]
    fn the_menu_reads_output_input_apps_then_audio() {
        let snapshot = Snapshot::of(&graph());
        let items = menu_items(&snapshot);
        let expected = vec![
            otto_kit::t!("bar-sound-output").to_string(),
            format!("[{}]", otto_kit::t!("bar-sound-output-volume")),
            "Built-in Speakers".to_string(),
            "Stream mix".to_string(),
            "—".to_string(),
            otto_kit::t!("bar-sound-input").to_string(),
            format!("[{}]", otto_kit::t!("bar-sound-input-volume")),
            "Scarlett 2i2".to_string(),
            "—".to_string(),
            otto_kit::t!("bar-sound-apps").to_string(),
            format!(
                "[{}]",
                otto_kit::t_owned!("bar-sound-app-volume", app = "Firefox")
            ),
            format!(
                "[{}]",
                otto_kit::t_owned!("bar-sound-app-volume", app = "Spotify")
            ),
            "—".to_string(),
            otto_kit::t!("bar-sound-open-audio").to_string(),
        ];
        assert_eq!(labels(&items), expected);
    }

    #[test]
    fn the_default_is_ticked_and_each_device_says_what_it_is() {
        let items = menu_items(&Snapshot::of(&graph()));
        let speakers = items
            .iter()
            .find(|i| i.label() == Some("Built-in Speakers"))
            .expect("the speakers");
        assert_eq!(speakers.checked, Some(true));
        assert_eq!(speakers.action_id(), Some("output:speakers"));
        assert_eq!(
            speakers.shortcut(),
            Some(otto_kit::t!("bar-sound-kind-built-in"))
        );
        let mix = items
            .iter()
            .find(|i| i.label() == Some("Stream mix"))
            .expect("the mix");
        assert_eq!(mix.checked, Some(false));
        assert_eq!(mix.shortcut(), Some(otto_kit::t!("bar-sound-kind-virtual")));
        let mic = items
            .iter()
            .find(|i| i.label() == Some("Scarlett 2i2"))
            .expect("the mic");
        assert_eq!(mic.shortcut(), Some(otto_kit::t!("bar-sound-kind-usb")));
    }

    #[test]
    fn an_airplay_speaker_says_so() {
        let Event::Node(speaker) = node(
            7,
            &[
                ("media.class", "Audio/Sink"),
                ("node.name", "raop_sink.Kitchen.local.192.168.1.30.7000"),
                ("node.description", "Kitchen"),
            ],
        ) else {
            unreachable!()
        };
        assert_eq!(kind_of(&speaker), Kind::AirPlay);
    }

    #[test]
    fn sliders_carry_the_volume_and_a_lit_toggle_while_sound_passes() {
        let items = menu_items(&Snapshot::of(&graph()));
        let slider = |id: &str| {
            items
                .iter()
                .find_map(|i| i.slider_data().filter(|s| s.action_id == id))
                .cloned()
                .expect("a slider")
        };
        let output = slider("output");
        assert!((output.value - 0.7).abs() < 1e-4);
        assert_eq!(output.toggle.map(|t| t.on), Some(true));
        let input = slider("input");
        assert_eq!(input.toggle.map(|t| t.on), Some(false));
        let firefox = slider("app:Firefox");
        assert_eq!(firefox.label.as_deref(), Some("Firefox"));
    }

    #[test]
    fn with_nothing_to_show_the_menu_still_opens_the_app() {
        let items = menu_items(&Snapshot::default());
        assert_eq!(labels(&items), vec![otto_kit::t!("bar-sound-open-audio")]);
    }

    #[test]
    fn the_glyph_lights_a_wave_per_third_and_none_when_muted() {
        assert_eq!(waves(0.0, false), 0);
        assert_eq!(waves(0.2, false), 1);
        assert_eq!(waves(0.5, false), 2);
        assert_eq!(waves(0.9, false), 3);
        assert_eq!(waves(0.9, true), 0);
    }

    #[test]
    fn choosing_a_device_names_it_and_audio_launches() {
        assert!(activate("audio"));
        assert!(!activate("output:speakers"));
        assert!(!activate("nonsense"));
    }
}
