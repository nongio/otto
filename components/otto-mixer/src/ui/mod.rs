//! The window: its chrome, its sections and everything the pointer and
//! keyboard do in it.
//!
//! Every control drawn records its rect and action in `targets` while it is
//! painted, and a press is tested against that list, so a press always lands
//! on what was on screen.

// Rust guideline compliant 2026-02-21

mod device_sound;
mod devices_pane;
mod effects_pane;
mod engine_pane;
mod graph_pane;
mod mixer_pane;
mod status_pane;
mod views_pane;
mod virtual_pane;

use otto_audio_graph::{
    airplay_missing, identity, AirPlayMissing, Connection, Event, Graph, LinkLifetime, MediaType,
    NodeState, Routing, SavedLink, SETTING_FORCE_QUANTUM, SETTING_FORCE_RATE, SETTING_MAX_QUANTUM,
    SETTING_MIN_QUANTUM, SETTING_QUANTUM, SETTING_RATE,
};
use otto_kit::components::push_button::{self, PushButton, Role as ButtonRole};
use otto_kit::components::scroll::ScrollView;
use otto_kit::components::segmented::{self, Segment, SegmentedInteraction};
use otto_kit::components::source_list::{
    self, GlyphSlot, SourceListItem, SourceListLayout, SourceListSize,
};
use otto_kit::components::titlebar::{
    DecorationVariant, WindowControl, WindowControlsState, WindowDecoration,
};
use otto_kit::controls_side::ControlsSide;
use otto_kit::events::{Keysym, Modifiers, PointerEvent, PointerEventKind, BTN_LEFT, BTN_MIDDLE};
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, PaintStyle, PathBuilder, Point, RRect};
use otto_kit::typography::ellipsize;
use otto_kit::{t, t_owned, CursorShape};
use otto_mixer::layout::Layout;
use otto_mixer::SERVICE_APP;

use otto_mixer::graph_view::{
    self, media_color, GraphMode, GraphView, Hit, LinkDrag, RowKey, Scene,
};

/// The sidebar's width.
const SIDEBAR_W: f32 = 216.0;
/// How much of the icon square a sidebar glyph fills, as in Settings.
const SIDEBAR_GLYPH_SHARE: f32 = 0.9;
/// The sidebar's row size: a handful of sections, so the large rows.
const SIDEBAR_SIZE: SourceListSize = SourceListSize::Large;
/// The strip above the canvas with the view switch and the save actions.
const TOOLBAR_H: f32 = 48.0;
/// The service banner under the toolbar.
const BANNER_H: f32 = 44.0;
/// The details panel beside the canvas.
const DETAILS_W: f32 = 300.0;
/// One line of the details panel's property list.
const PROP_LINE_H: f32 = 18.0;
/// The details panel's label column.
const FACT_LABEL_W: f32 = 92.0;
/// One fact in the details panel.
const FACT_H: f32 = 24.0;
/// One connection in the details panel.
const CONNECTION_H: f32 = 34.0;
/// The most connections the details panel lists.
const MAX_CONNECTIONS: usize = 6;
/// The filter chips, and the room between them.
const CHIP_H: f32 = 26.0;
const CHIP_GAP: f32 = 6.0;
/// How far the canvas overlays (filter chips, legend) sit from its edges.
const OVERLAY_INSET: f32 = 12.0;
/// The padding around the chips on their material.
const OVERLAY_PAD: f32 = 4.0;
/// The filter chips' material.
const FILTERS_H: f32 = CHIP_H + 2.0 * OVERLAY_PAD;
/// The legend's material, its padding, and the room between its entries.
const LEGEND_H: f32 = 24.0;
const LEGEND_PAD: f32 = 12.0;
const LEGEND_GAP: f32 = 14.0;
/// A legend entry's mark (a wire or a cell's dot) and the gap to its label.
const LEGEND_MARK_W: f32 = 18.0;
const LEGEND_MARK_GAP: f32 = 6.0;
/// The room the Matrix and Lanes keep above their content for the filter
/// chips, and the Matrix below its cells for the legend: they scroll under
/// fixed headers, so the overlays get a band rather than float.
const FILTERS_BAND_H: f32 = OVERLAY_INSET + FILTERS_H + 6.0;
const LEGEND_BAND_H: f32 = LEGEND_H + OVERLAY_INSET + 6.0;
/// The Engine section's cards.
const ENGINE_CARD_MIN_W: f32 = 280.0;
const ENGINE_CARD_H: f32 = 180.0;
const ENGINE_GAP: f32 = 16.0;
/// The buffer tiles: per row, and their height.
const TILE_COLUMNS: usize = 4;
const TILE_H: f32 = 44.0;
/// The quanta the Engine section offers, in frames; 0 is the engine's choice.
const QUANTA: [u32; 7] = [0, 64, 128, 256, 512, 1024, 2048];
/// The rates the Engine section offers, in Hz; 0 is the engine's choice.
const RATES: [u32; 5] = [0, 44_100, 48_000, 88_200, 96_000];
/// How much one point of Cmd+scroll zooms by.
const SCROLL_ZOOM: f32 = 0.006;

/// The sidebar's sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Mixer,
    Graph,
    Engine,
    Devices,
    VirtualDevices,
    Effects,
    Status,
}

/// The sidebar, top to bottom: the everyday sections, the Advanced heading
/// (`None`) over the ones that show the sound server's own objects, and the
/// status line pinned to the foot.
const SIDEBAR: [Option<Pane>; 8] = [
    Some(Pane::Mixer),
    Some(Pane::Devices),
    None,
    Some(Pane::Graph),
    Some(Pane::Effects),
    Some(Pane::Engine),
    Some(Pane::VirtualDevices),
    Some(Pane::Status),
];
/// The room left under the status line, at the sidebar's foot.
const SIDEBAR_FOOT: f32 = 10.0;

/// What a control does when pressed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Pane(Pane),
    Filter(MediaType),
    SaveRouting,
    SaveWire,
    ForgetWire,
    RemoveWire,
    StartService,
    StartPipeWire,
    /// Turn AirPlay speakers on or off, from the Devices section.
    AirPlay,
    Rate(u32),
    Quantum(u32),
    ResetEngine,
    /// Force a small buffer and a fixed rate, for low latency.
    LowLatency,
    Fit,
    ToggleProperties,
    /// Save the wire between two rows, from the details panel.
    SaveWireOf(RowKey, RowKey),
    /// A control of the Virtual devices section.
    Virtual(virtual_pane::VirtualAction),
    /// A control of the Effects section.
    Effect(effects_pane::EffectAction),
    /// A control of the Mixer section.
    Mixer(mixer_pane::MixerAction),
    /// A control of the Devices section's detail.
    Device(device_sound::DeviceAction),
    /// A fix in the Status section.
    Status(status_pane::StatusAction),
    /// Switch a device to a profile, by its index.
    Profile {
        device: u32,
        index: u32,
    },
    /// Point a device's sink or source `card_device` at a port.
    Port {
        device: u32,
        index: u32,
        card_device: u32,
    },
    /// Open the choices of row `row` of a device's card as a pop-up.
    DeviceMenu {
        device: u32,
        row: usize,
    },
    /// Show the Graph section as a patchbay, a matrix or lanes.
    GraphMode(GraphMode),
    /// Link two rows with their channels paired, from a matrix cell.
    LinkCell(RowKey, RowKey),
    /// Select the wire between two rows, from a matrix cell.
    SelectWire(RowKey, RowKey),
    /// Select a node, from a matrix header or a lane's step.
    SelectNode(u32),
}

/// A pop-up the window should open, as the press that asked for it left it.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuRequest {
    /// The field it opens from, in window points.
    pub rect: Rect,
    /// The press's serial, which the pop-up's grab needs.
    pub serial: u32,
    pub options: Vec<String>,
    pub selected: Option<usize>,
}

/// A pointer drag in progress.
#[derive(Debug, Clone, Copy)]
enum Drag {
    /// Moving the canvas: where the pointer and the pan started.
    Pan { from: Point, pan: Point },
    /// Moving a card: the pointer's offset into it, in world points.
    Card { node: u32, grab: Point, moved: bool },
    /// Drawing a new wire.
    Link(LinkDrag),
}

/// Everything the window shows and remembers.
pub struct Ui {
    pub size: (f32, f32),
    pub variant: DecorationVariant,
    pub active: bool,
    /// Whether the compositor is frosting the window right now, so the
    /// sidebar is left to its material instead of painted flat.
    pub frosted: bool,
    pub controls: WindowControlsState,
    pub modifiers: Modifiers,
    pane: Pane,
    pub graph: Graph,
    routing: Routing,
    connection: Connection,
    view: GraphView,
    pointer: Option<Point>,
    drag: Option<Drag>,
    pressed: Option<Action>,
    targets: Vec<(Rect, Action)>,
    /// The details panel's property list, and whether it is open.
    props_scroll: ScrollView,
    props_open: bool,
    pinch_zoom: Option<f32>,
    /// Whether the server has said once whether it is there, so the window
    /// does not flash "not running" while it connects.
    heard: bool,
    /// The colour scheme last drawn in.
    dark: bool,
    /// The Virtual devices section's form and rename.
    virtual_form: virtual_pane::VirtualForm,
    /// The Devices section's cards, which scroll under its title.
    devices_scroll: ScrollView,
    /// A pop-up asked for and not yet opened, what each of its rows does,
    /// and the control it is open for.
    menu_request: Option<MenuRequest>,
    menu_choices: Vec<Action>,
    menu_open: Option<Action>,
    /// Where the Matrix and Lanes views are scrolled to.
    views: views_pane::ViewScrolls,
    /// The Effects section's selection, form, drag and import.
    effects: effects_pane::EffectsState,
    /// What AirPlay discovery lacks here, read when the Devices section
    /// opens and when the switch is pressed.
    airplay_missing: Option<AirPlayMissing>,
    /// The Mixer section's scroll and drag.
    mixer: mixer_pane::MixerState,
    /// The Devices section's selection, details and loudness drag.
    devices: device_sound::DevicesState,
    /// The dropout counts seen, for the Status section's last ten minutes.
    dropout_log: status_pane::DropoutLog,
    /// The Status section's cards, which scroll under its title.
    status_scroll: ScrollView,
}

impl Ui {
    pub fn new(connection: Connection) -> Self {
        // The Status section counts dropouts; the profiler is followed for
        // as long as the window is open.
        connection.watch_dropouts(true);
        let routing = Routing::load().unwrap_or_else(|error| {
            tracing::warn!(%error, "the saved routing could not be read");
            Routing::default()
        });
        let layout = Layout::path()
            .map(|path| Layout::load_from(&path))
            .unwrap_or_default();
        Self {
            size: (1100.0, 700.0),
            variant: DecorationVariant::Floating,
            active: true,
            frosted: false,
            controls: WindowControlsState::new(),
            modifiers: Modifiers::default(),
            pane: Pane::Mixer,
            graph: Graph::new(),
            routing,
            connection,
            view: GraphView::new(layout),
            pointer: None,
            drag: None,
            pressed: None,
            targets: Vec::new(),
            props_scroll: ScrollView::new(Rect::from_xywh(0.0, 0.0, 0.0, 0.0)),
            props_open: false,
            pinch_zoom: None,
            heard: false,
            dark: false,
            virtual_form: virtual_pane::VirtualForm::default(),
            devices_scroll: ScrollView::new(Rect::from_xywh(0.0, 0.0, 0.0, 0.0)),
            menu_request: None,
            menu_choices: Vec::new(),
            menu_open: None,
            views: views_pane::ViewScrolls::default(),
            effects: effects_pane::EffectsState::default(),
            airplay_missing: airplay_missing(),
            mixer: mixer_pane::MixerState::default(),
            devices: device_sound::DevicesState::default(),
            dropout_log: status_pane::DropoutLog::default(),
            status_scroll: ScrollView::new(Rect::from_xywh(0.0, 0.0, 0.0, 0.0)),
        }
    }

    /// Take one event from PipeWire.
    pub fn apply(&mut self, event: Event) {
        self.heard = true;
        if matches!(event, Event::Connected | Event::Disconnected) {
            self.view.forget_ids();
            self.view.auto_fit = true;
            self.drag = None;
        }
        if let Event::Removed(id) = event {
            if self.view.selected_node == Some(id) {
                self.view.selected_node = None;
            }
        }
        if let Event::Volume { node, .. } = event {
            self.settle_mixer(node);
        }
        if let Event::Dropouts { driver, count, .. } = event {
            self.dropout_log
                .record(driver, count, std::time::Instant::now());
        }
        self.graph.apply(event);
    }

    // -----------------------------------------------------------------------
    // Geometry
    // -----------------------------------------------------------------------

    fn titlebar_h(&self) -> f32 {
        WindowDecoration::sidebar_height_for(self.variant)
    }

    pub fn decoration(&self) -> WindowDecoration {
        let mut decoration = WindowDecoration::new(t!("audio-title"), self.size.0)
            .with_variant(self.variant)
            .with_height(self.titlebar_h())
            .with_active(self.active)
            .with_dark(self.dark)
            .with_controls_only(true);
        decoration.controls_hovered = self.controls.hovered();
        decoration.pressed = self.controls.pressed();
        decoration
    }

    fn content(&self) -> Rect {
        Rect::from_ltrb(SIDEBAR_W, self.titlebar_h(), self.size.0, self.size.1)
    }

    fn service_down(&self) -> bool {
        self.graph.is_connected() && self.graph.is_synced() && !self.graph.has_client(SERVICE_APP)
    }

    fn details_rect(&self) -> Option<Rect> {
        self.view.selected_node?;
        let content = self.content();
        Some(Rect::from_ltrb(
            (content.right - DETAILS_W).max(content.left + 200.0),
            content.top,
            content.right,
            content.bottom,
        ))
    }

    /// The canvas: under the toolbar and banner, left of the details panel.
    /// The filter chips and legend float inside it.
    fn canvas_rect(&self) -> Rect {
        let content = self.content();
        let mut top = content.top + TOOLBAR_H;
        if self.service_down() {
            top += BANNER_H;
        }
        let right = self.details_rect().map_or(content.right, |r| r.left);
        Rect::from_ltrb(
            content.left,
            top,
            right.max(content.left + 1.0),
            content.bottom,
        )
    }

    /// Where the current view draws: the whole canvas for the patchbay,
    /// which pans under the overlays; the Matrix and Lanes leave bands for
    /// them.
    fn viewport(&self) -> Rect {
        let canvas = self.canvas_rect();
        let (top, bottom) = match self.view.mode {
            GraphMode::Patchbay => (0.0, 0.0),
            GraphMode::Matrix => (FILTERS_BAND_H, LEGEND_BAND_H),
            GraphMode::Lanes => (FILTERS_BAND_H, 0.0),
        };
        Rect::from_ltrb(
            canvas.left,
            (canvas.top + top).min(canvas.bottom),
            canvas.right,
            (canvas.bottom - bottom).max(canvas.top + top + 1.0),
        )
    }

    /// Fit the patchbay to the canvas, clear of the filter chips above and
    /// the legend below.
    fn fit_patchbay(&mut self, scene: &Scene) {
        let viewport = self.viewport();
        let inner = Rect::from_ltrb(
            viewport.left,
            viewport.top + FILTERS_H,
            viewport.right,
            (viewport.bottom - LEGEND_H).max(viewport.top + FILTERS_H + 1.0),
        );
        self.view.fit(scene, inner);
        self.view.pan.y += inner.top - viewport.top;
    }

    fn sidebar_layout(&self, items: &[SourceListItem]) -> SourceListLayout {
        SourceListLayout::for_items_in(
            SIDEBAR_SIZE,
            items,
            0.0,
            self.titlebar_h() + 12.0,
            self.size.1 - SIDEBAR_FOOT,
            SIDEBAR_W,
        )
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    /// What the window is called: the app, then the pane you are in.
    pub fn title(&self) -> String {
        t_owned!("audio-window-title", pane = pane_label(self.pane))
    }

    /// Open on `pane`.
    pub fn show(&mut self, pane: Pane) {
        self.set_pane(pane);
    }

    fn set_pane(&mut self, pane: Pane) {
        if pane == Pane::Devices {
            self.airplay_missing = airplay_missing();
        }
        self.pane = pane;
    }

    pub fn draw(&mut self, canvas: &Canvas, theme: &Theme) {
        self.targets.clear();
        self.dark = theme.is_dark();
        canvas.clear(Color::TRANSPARENT);
        canvas.draw_rect(
            Rect::from_ltrb(SIDEBAR_W, 0.0, self.size.0, self.size.1),
            &fill(ground(theme)),
        );
        self.draw_sidebar(canvas, theme);
        if !self.graph.is_connected() {
            self.draw_no_server(canvas, theme);
        } else {
            match self.pane {
                Pane::Graph => self.draw_graph(canvas, theme),
                Pane::Engine => self.draw_engine(canvas, theme),
                Pane::VirtualDevices => self.draw_virtual(canvas, theme),
                Pane::Devices => self.draw_devices(canvas, theme),
                Pane::Effects => self.draw_effects(canvas, theme),
                Pane::Mixer => self.draw_mixer(canvas, theme),
                Pane::Status => self.draw_status(canvas, theme),
            }
        }
        let decoration = self.decoration();
        self.draw_title(canvas, theme);
        decoration.draw(canvas);
        self.draw_readout(canvas, &decoration, theme);
    }

    /// The window's opaque part: the content, short of its rounded
    /// bottom-right corner. The sidebar is the compositor's frost.
    pub fn opaque_region(&self, corner: f32) -> [Rect; 2] {
        let (w, h) = self.size;
        [
            Rect::from_ltrb(SIDEBAR_W, 0.0, w, h - corner),
            Rect::from_ltrb(SIDEBAR_W, h - corner, w - corner, h),
        ]
    }

    /// The title over the content, with a hairline under the band. The
    /// sidebar runs the full height beside it, under the window controls.
    fn draw_title(&self, canvas: &Canvas, theme: &Theme) {
        let h = self.titlebar_h();
        if h <= 0.0 {
            return;
        }
        let x = SIDEBAR_W + 16.0;
        let font = WindowDecoration::title_style_for(self.variant).font();
        let readout = if self.graph.is_connected() && self.graph.is_synced() {
            180.0
        } else {
            16.0
        };
        Label::new(ellipsize(
            &font,
            &self.title(),
            (self.size.0 - x - readout).max(0.0),
        ))
        .with_style(WindowDecoration::title_style_for(self.variant))
        .with_color(if self.active {
            theme.text_primary
        } else {
            theme.text_secondary
        })
        .centered_on(x, h / 2.0)
        .render(canvas);
        canvas.draw_line(
            Point::new(SIDEBAR_W, h - 0.5),
            Point::new(self.size.0, h - 0.5),
            &stroke(theme.hairline, 1.0),
        );
    }

    /// What the engine runs at now: rate, quantum, and whether each is forced.
    fn engine_now(&self) -> EngineNow {
        let rate = self.graph.setting_u32(SETTING_RATE).unwrap_or(48_000);
        let force_rate = self
            .graph
            .setting_u32(SETTING_FORCE_RATE)
            .filter(|r| *r != 0);
        let quantum = self.graph.setting_u32(SETTING_QUANTUM).unwrap_or(1024);
        let force_quantum = self
            .graph
            .setting_u32(SETTING_FORCE_QUANTUM)
            .filter(|q| *q != 0);
        EngineNow {
            rate: force_rate.unwrap_or(rate),
            frames: force_quantum.unwrap_or(quantum),
            default_quantum: quantum,
            min: self.graph.setting_u32(SETTING_MIN_QUANTUM).unwrap_or(32),
            max: self.graph.setting_u32(SETTING_MAX_QUANTUM).unwrap_or(2048),
            force_rate,
            force_quantum,
        }
    }

    /// The engine's rate, buffer and latency in the title bar, on the side
    /// away from the window controls.
    fn draw_readout(&self, canvas: &Canvas, decoration: &WindowDecoration, theme: &Theme) {
        if !self.graph.is_connected() || !self.graph.is_synced() {
            return;
        }
        let now = self.engine_now();
        let text = t_owned!(
            "audio-readout",
            rate = khz(now.rate),
            frames = now.frames,
            ms = format!("{:.1}", now.latency_ms())
        );
        let font = styles::SUBHEADLINE.font();
        let w = font.measure_str(&text, None).0 + 16.0;
        let h = 22.0;
        let cy = self.titlebar_h() / 2.0;
        let x = match decoration.controls_side {
            ControlsSide::Left => self.size.0 - 12.0 - w,
            ControlsSide::Right => 12.0,
        };
        let pill = Rect::from_xywh(x, cy - h / 2.0, w, h);
        canvas.draw_rrect(
            RRect::new_rect_xy(pill, 6.0, 6.0),
            &fill(theme.control_track),
        );
        Label::new(text)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_at(pill.center_x(), cy)
            .render(canvas);
    }

    fn draw_sidebar(&mut self, canvas: &Canvas, theme: &Theme) {
        let top = 0.0;
        if !self.frosted {
            canvas.draw_rect(
                Rect::from_ltrb(0.0, top, SIDEBAR_W, self.size.1),
                &fill(sidebar_ground(theme)),
            );
        }
        canvas.draw_line(
            Point::new(SIDEBAR_W - 0.5, top),
            Point::new(SIDEBAR_W - 0.5, self.size.1),
            &stroke(theme.hairline, 1.0),
        );
        // The status line: whether the audio service runs, or the first
        // thing worth a look, named, once the graph is in.
        let known = self.graph.is_connected() && self.graph.is_synced();
        let issues = if known {
            self.status_issues()
        } else {
            Vec::new()
        };
        let status = match issues.as_slice() {
            _ if !known => t!("audio-pane-status").to_string(),
            [] => t!("audio-status-line-on").to_string(),
            [only] => only.short.clone(),
            [first, rest @ ..] => t_owned!(
                "audio-status-line-more",
                first = first.short.clone(),
                count = rest.len()
            ),
        };
        let dot = if !known {
            theme.fill_secondary
        } else if issues.is_empty() {
            theme.accent_green
        } else {
            theme.accent_orange
        };
        let items = sidebar_items(status);
        let layout = self.sidebar_layout(&items);
        let selected = SIDEBAR.iter().position(|p| *p == Some(self.pane));
        source_list::draw(
            canvas,
            &layout,
            &items,
            selected,
            theme,
            |canvas, index, slot, rect, tint| match (slot, SIDEBAR[index]) {
                (GlyphSlot::Leading, Some(Pane::Status)) => {
                    let ring = selected == Some(index);
                    canvas.draw_circle(
                        Point::new(rect.center_x(), rect.center_y()),
                        5.0,
                        &fill(dot),
                    );
                    if ring {
                        canvas.draw_circle(
                            Point::new(rect.center_x(), rect.center_y()),
                            5.5,
                            &stroke(Color::WHITE, 1.5),
                        );
                    }
                }
                (GlyphSlot::Leading, Some(pane)) => draw_pane_glyph(canvas, pane, rect, tint),
                _ => {}
            },
        );
        for (entry, rect) in SIDEBAR.iter().zip(&layout.item_rects) {
            if let Some(pane) = entry {
                self.targets.push((*rect, Action::Pane(*pane)));
            }
        }
    }

    fn draw_no_server(&mut self, canvas: &Canvas, theme: &Theme) {
        if !self.heard {
            return;
        }
        let content = self.content();
        let cx = content.center_x();
        let cy = content.center_y();
        Label::new(t!("audio-no-server-title"))
            .with_style(styles::TITLE_3_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_at(cx, cy - 36.0)
            .render(canvas);
        Label::new(t!("audio-no-server-body"))
            .with_style(styles::BODY)
            .with_color(theme.text_secondary)
            .centered_at(cx, cy - 10.0)
            .render(canvas);
        let label = t!("audio-no-server-start");
        let w = push_button::width(label);
        let rect = Rect::from_xywh(cx - w / 2.0, cy + 16.0, w, push_button::HEIGHT);
        self.button(canvas, rect, label, Action::StartPipeWire, true, theme);
    }

    fn button(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        label: &str,
        action: Action,
        enabled: bool,
        theme: &Theme,
    ) {
        self.role_button(
            canvas,
            rect,
            label,
            action,
            enabled,
            ButtonRole::Normal,
            theme,
        );
    }

    #[expect(clippy::too_many_arguments, reason = "one call shape for every button")]
    fn role_button(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        label: &str,
        action: Action,
        enabled: bool,
        role: ButtonRole,
        theme: &Theme,
    ) {
        let button = PushButton::new(role)
            .enabled(enabled)
            .pressed(self.pressed == Some(action));
        push_button::draw(canvas, rect, label, button, theme);
        if enabled {
            self.targets.push((rect, action));
        }
    }

    fn draw_graph(&mut self, canvas: &Canvas, theme: &Theme) {
        let scene = self.view.scene(&self.graph, &self.routing);
        let viewport = self.viewport();
        // The Matrix and Lanes bands for the overlays take their view's ground.
        match self.view.mode {
            GraphMode::Patchbay => {}
            GraphMode::Matrix => {
                canvas.draw_rect(self.canvas_rect(), &fill(graph_view::card_ground(theme)));
            }
            GraphMode::Lanes => {
                canvas.draw_rect(self.canvas_rect(), &fill(graph_view::canvas_ground(theme)));
            }
        }
        match self.view.mode {
            GraphMode::Patchbay => self.draw_patchbay(canvas, &scene, viewport, theme),
            GraphMode::Matrix => self.draw_matrix(canvas, &scene, viewport, theme),
            GraphMode::Lanes => self.draw_lanes(canvas, &scene, viewport, theme),
        }
        self.draw_overlays(canvas, &scene, theme);
        self.draw_toolbar(canvas, &scene, theme);
        if self.service_down() {
            self.draw_banner(canvas, theme);
        }
        if let Some(rect) = self.details_rect() {
            self.draw_details(canvas, rect, &scene, theme);
        }
    }

    fn draw_patchbay(&mut self, canvas: &Canvas, scene: &Scene, viewport: Rect, theme: &Theme) {
        if self.view.auto_fit {
            self.fit_patchbay(scene);
        }
        let link_drag = match self.drag {
            Some(Drag::Link(drag)) => Some(drag),
            _ => None,
        };
        graph_view::draw(canvas, scene, &self.view, viewport, link_drag, theme);
        if scene.cards.is_empty() && self.graph.is_synced() {
            Label::new(t!("audio-graph-empty"))
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .centered_at(viewport.center_x(), viewport.center_y())
                .render(canvas);
        }
    }

    /// Whether something drawn is still moving and wants another frame.
    pub fn is_animating(&self) -> bool {
        (self.view.selected_node.is_some() && self.props_open && self.props_scroll.is_animating())
            || (self.pane == Pane::Devices && self.devices_scroll.is_animating())
            || (self.pane == Pane::Graph && self.views.is_animating(self.view.mode))
            || (self.pane == Pane::Mixer && self.mixer.is_animating())
            || (self.pane == Pane::Status && self.status_scroll.is_animating())
    }

    /// The pop-up a press asked for, to open against the window's surface.
    pub fn take_menu_request(&mut self) -> Option<MenuRequest> {
        self.menu_request.take()
    }

    /// The open pop-up's row `index` was chosen.
    pub fn choose(&mut self, index: usize) {
        self.menu_open = None;
        if let Some(action) = self.menu_choices.get(index).copied() {
            self.run(action);
        }
    }

    /// The open pop-up closed with nothing chosen.
    pub fn menu_closed(&mut self) {
        self.menu_open = None;
    }

    fn segments(
        &mut self,
        canvas: &Canvas,
        track: Rect,
        labels: &[String],
        chosen: usize,
        actions: &[Action],
        theme: &Theme,
    ) {
        let segments: Vec<Segment<'_>> = labels.iter().map(|l| Segment::Text(l)).collect();
        segmented::draw(
            canvas,
            track,
            &segments,
            Some(chosen),
            SegmentedInteraction::Normal,
            theme,
        );
        for (index, action) in actions.iter().enumerate() {
            self.targets.push((
                segmented::segment_rect(track, actions.len(), index),
                *action,
            ));
        }
    }

    // -----------------------------------------------------------------------
    // Pointer
    // -----------------------------------------------------------------------

    fn target_at(&self, p: Point) -> Option<Action> {
        self.target_hit(p).map(|(_, action)| action)
    }

    fn target_hit(&self, p: Point) -> Option<(Rect, Action)> {
        self.targets
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(p))
            .copied()
    }

    /// Handle pointer events; returns whether to redraw.
    pub fn pointer(&mut self, events: &[PointerEvent], window: &Window) -> bool {
        let mut redraw = false;
        for event in events {
            let p = Point::new(event.position.0 as f32, event.position.1 as f32);
            match &event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer = Some(p);
                    let control = self.decoration().control_at(p.x, p.y);
                    redraw |= self.controls.on_motion(control);
                    redraw |= self.motion(p);
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer = None;
                    redraw |= self.controls.on_leave();
                }
                PointerEventKind::Press { button, serial, .. } => {
                    redraw |= self.press(p, *button, *serial, window);
                }
                PointerEventKind::Release { button, .. } => {
                    redraw |= self.release(p, *button, window);
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    redraw |= self.scroll(
                        p,
                        horizontal.absolute as f32,
                        vertical.absolute as f32,
                        vertical.discrete != 0,
                        vertical.stop,
                    );
                }
            }
        }
        redraw
    }

    fn motion(&mut self, p: Point) -> bool {
        if self.drag_effect(p) || self.drag_mixer(p) || self.drag_device(p) {
            return true;
        }
        let viewport = self.viewport();
        match &mut self.drag {
            Some(Drag::Pan { from, pan }) => {
                self.view.pan = Point::new(pan.x + p.x - from.x, pan.y + p.y - from.y);
                true
            }
            Some(Drag::Card { node, grab, moved }) => {
                let world = self.view.to_world(viewport, p);
                *moved = true;
                let (node, at) = (*node, world - *grab);
                self.view.move_card(node, at);
                true
            }
            Some(Drag::Link(drag)) => {
                drag.to = self.view.to_world(viewport, p);
                true
            }
            None => false,
        }
    }

    fn press(&mut self, p: Point, button: u32, serial: u32, window: &Window) -> bool {
        let seat = AppContext::seat_state().seats().next();
        if button == BTN_MIDDLE
            && self.pane == Pane::Graph
            && self.view.mode == GraphMode::Patchbay
            && self.viewport().contains(p)
            && !self.overlay_at(p)
        {
            self.drag = Some(Drag::Pan {
                from: p,
                pan: self.view.pan,
            });
            self.view.auto_fit = false;
            AppContext::set_cursor_shape(CursorShape::Grabbing);
            return false;
        }
        if button != BTN_LEFT {
            return false;
        }
        if !window.is_maximized() {
            let edge = otto_kit::components::window::resize::edge_at(
                Rect::from_wh(self.size.0, self.size.1),
                p.x,
                p.y,
            );
            if let (Some(edge), Some(seat)) = (edge, seat.as_ref()) {
                window.start_resize(seat, serial, edge);
                return false;
            }
        }
        let control = self.decoration().control_at(p.x, p.y);
        if self.controls.on_press(control) {
            return true;
        }
        if p.y < self.titlebar_h() {
            if let Some(seat) = seat {
                window.titlebar_press(&seat, serial, p.x, p.y);
            }
            return false;
        }
        if let Some((rect, action)) = self.target_hit(p) {
            if self.open_device_menu(action, rect, serial)
                || self.open_effect_menu(action, rect, serial)
                || self.open_mixer_menu(action, rect, serial)
                || self.open_tuning_menu(action, rect, serial)
                || self.press_effect(action, rect, p)
                || self.press_mixer(action, rect, p)
                || self.press_device(action, rect, p)
            {
                return true;
            }
            self.pressed = Some(action);
            return true;
        }
        if self.overlay_at(p) {
            // The overlays' own ground: a press between chips or on the
            // legend does nothing, rather than reaching the view under it.
            return false;
        }
        if self.pane == Pane::Graph && self.graph.is_connected() && self.viewport().contains(p) {
            return self.press_canvas(p);
        }
        false
    }

    fn press_canvas(&mut self, p: Point) -> bool {
        if self.view.mode != GraphMode::Patchbay {
            return self.press_view();
        }
        let viewport = self.viewport();
        let scene = self.view.scene(&self.graph, &self.routing);
        let world = self.view.to_world(viewport, p);
        match scene.hit(world, self.view.zoom) {
            Hit::Chevron(node) => {
                self.view.toggle_fold(node);
                self.view.selected_wire = None;
            }
            Hit::Row(key) => {
                self.drag = Some(Drag::Link(LinkDrag {
                    from: key,
                    to: world,
                }));
            }
            Hit::Card(node) => {
                let at = scene
                    .card(node)
                    .map_or(world, |c| Point::new(c.rect.left, c.rect.top));
                if self.view.selected_node != Some(node) {
                    self.props_scroll.scroll_to(0.0);
                }
                self.view.selected_node = Some(node);
                self.view.selected_wire = None;
                self.view.auto_fit = false;
                self.drag = Some(Drag::Card {
                    node,
                    grab: world - at,
                    moved: false,
                });
                AppContext::set_cursor_shape(CursorShape::Grabbing);
            }
            Hit::Wire(index) => {
                let wire = &scene.wires[index];
                self.view.selected_wire = Some((wire.from, wire.to));
                self.view.selected_node = None;
            }
            Hit::Nothing => {
                self.view.selected_node = None;
                self.view.selected_wire = None;
                self.view.auto_fit = false;
                self.drag = Some(Drag::Pan {
                    from: p,
                    pan: self.view.pan,
                });
                AppContext::set_cursor_shape(CursorShape::Grabbing);
            }
        }
        true
    }

    fn release(&mut self, p: Point, button: u32, window: &Window) -> bool {
        let mut redraw = self.release_effect() | self.release_mixer() | self.release_device();
        if button == BTN_LEFT {
            let control = self.decoration().control_at(p.x, p.y);
            let armed = self.controls.pressed().is_some();
            match self.controls.on_release(control) {
                Some(WindowControl::Close) => AppContext::request_exit(),
                Some(WindowControl::Minimize) => window.minimize(),
                Some(WindowControl::Zoom) => window.toggle_maximized(),
                None => {}
            }
            redraw |= armed;
            if let Some(action) = self.pressed.take() {
                if self.target_at(p) == Some(action) {
                    self.run(action);
                }
                redraw = true;
            }
        }
        match self.drag.take() {
            Some(Drag::Card { node, moved, .. }) => {
                if moved {
                    self.view.remember(&self.graph, node);
                }
                AppContext::set_cursor_shape(CursorShape::Default);
            }
            Some(Drag::Pan { .. }) => AppContext::set_cursor_shape(CursorShape::Default),
            Some(Drag::Link(drag)) => {
                self.finish_link(drag.from, p);
                redraw = true;
            }
            None => {}
        }
        redraw
    }

    fn finish_link(&mut self, from: RowKey, p: Point) {
        let viewport = self.viewport();
        let scene = self.view.scene(&self.graph, &self.routing);
        let Hit::Row(to) = scene.hit(self.view.to_world(viewport, p), self.view.zoom) else {
            return;
        };
        for (output, input) in self.view.pairs_to_link(&scene, &self.graph, from, to) {
            self.connection.link(output, input, LinkLifetime::Lingering);
        }
    }

    fn scroll(&mut self, p: Point, dx: f32, dy: f32, discrete: bool, stop: bool) -> bool {
        if self.details_rect().is_some_and(|r| r.contains(p)) {
            // A notched wheel moves one step per click; a touchpad stream
            // drives momentum, and lifting the fingers throws it.
            if stop {
                self.props_scroll.on_wheel_end();
                return true;
            }
            return if discrete {
                self.props_scroll.on_wheel_discrete(dy)
            } else {
                self.props_scroll.on_wheel(dy)
            };
        }
        if self.pane == Pane::Devices && self.devices_viewport().contains(p) {
            if stop {
                self.devices_scroll.on_wheel_end();
                return true;
            }
            return if discrete {
                self.devices_scroll.on_wheel_discrete(dy)
            } else {
                self.devices_scroll.on_wheel(dy)
            };
        }
        if self.pane == Pane::Effects {
            return self.scroll_effects(p, dy, discrete, stop);
        }
        if self.pane == Pane::Mixer {
            return self.scroll_mixer(p, dy, discrete, stop);
        }
        if self.pane == Pane::Status {
            return self.scroll_status(p, dy, discrete, stop);
        }
        if self.pane != Pane::Graph || !self.viewport().contains(p) {
            return false;
        }
        if self.view.mode != GraphMode::Patchbay {
            return self.scroll_view(dx, dy, discrete, stop);
        }
        self.view.auto_fit = false;
        if self.modifiers.ctrl || self.modifiers.logo {
            let factor = (-dy * SCROLL_ZOOM).exp();
            self.view.zoom_about(self.viewport(), p, factor);
        } else {
            self.view.pan = Point::new(self.view.pan.x - dx, self.view.pan.y - dy);
        }
        true
    }

    pub fn pinch_begin(&mut self) {
        self.pinch_zoom = Some(self.view.zoom);
    }

    pub fn pinch_update(&mut self, scale: f32) -> bool {
        let (Some(base), Some(p)) = (self.pinch_zoom, self.pointer) else {
            return false;
        };
        if self.pane != Pane::Graph || self.view.mode != GraphMode::Patchbay {
            return false;
        }
        self.view.auto_fit = false;
        let factor = base * scale / self.view.zoom;
        self.view.zoom_about(self.viewport(), p, factor);
        true
    }

    pub fn pinch_end(&mut self) {
        self.pinch_zoom = None;
    }

    // -----------------------------------------------------------------------
    // Keys and actions
    // -----------------------------------------------------------------------

    /// Handle a key press; returns whether to redraw.
    pub fn key(&mut self, keysym: Keysym) -> bool {
        let command = self.modifiers.ctrl || self.modifiers.logo;
        match keysym {
            Keysym::_0 | Keysym::KP_0 if command => {
                if self.pane == Pane::Graph {
                    let scene = self.view.scene(&self.graph, &self.routing);
                    self.fit_patchbay(&scene);
                }
                true
            }
            Keysym::plus | Keysym::equal | Keysym::KP_Add if command => {
                let viewport = self.viewport();
                let center = Point::new(viewport.center_x(), viewport.center_y());
                self.view.auto_fit = false;
                self.view.zoom_about(viewport, center, 1.2);
                true
            }
            Keysym::minus | Keysym::KP_Subtract if command => {
                let viewport = self.viewport();
                let center = Point::new(viewport.center_x(), viewport.center_y());
                self.view.auto_fit = false;
                self.view.zoom_about(viewport, center, 1.0 / 1.2);
                true
            }
            Keysym::Delete | Keysym::BackSpace => {
                self.run(Action::RemoveWire);
                true
            }
            Keysym::Escape => {
                self.view.selected_node = None;
                self.view.selected_wire = None;
                self.drag = None;
                true
            }
            _ => false,
        }
    }

    /// The links of the selected wire.
    fn selected_links(&mut self) -> Vec<u32> {
        let Some((from, to)) = self.view.selected_wire else {
            return Vec::new();
        };
        let scene = self.view.scene(&self.graph, &self.routing);
        scene
            .wire(from, to)
            .map(|w| w.links.clone())
            .unwrap_or_default()
    }

    fn saved_forms(&self, links: &[u32]) -> Vec<SavedLink> {
        links
            .iter()
            .filter_map(|id| self.graph.link(*id))
            .filter_map(|link| SavedLink::of(&self.graph, link))
            .collect()
    }

    fn save_routing_file(&self) {
        if let Err(error) = self.routing.save() {
            tracing::warn!(%error, "the routing could not be saved");
        }
    }

    fn run(&mut self, action: Action) {
        match action {
            Action::Pane(pane) => {
                self.flush_effects();
                self.set_pane(pane);
            }
            Action::Fit => {
                let scene = self.view.scene(&self.graph, &self.routing);
                self.fit_patchbay(&scene);
            }
            Action::ToggleProperties => {
                self.props_open = !self.props_open;
                self.props_scroll.scroll_to(0.0);
            }
            Action::SaveWireOf(from, to) => {
                let scene = self.view.scene(&self.graph, &self.routing);
                let links = scene
                    .wire(from, to)
                    .map(|w| w.links.clone())
                    .unwrap_or_default();
                let mut changed = false;
                for saved in self.saved_forms(&links) {
                    changed |= self.routing.add(saved);
                }
                if changed {
                    self.save_routing_file();
                }
            }
            Action::Filter(media) => {
                self.view.filter.toggle(media);
                self.view.selected_wire = None;
            }
            Action::SaveRouting => {
                let scene = self.view.scene(&self.graph, &self.routing);
                let links: Vec<u32> = scene.wires.iter().flat_map(|w| w.links.clone()).collect();
                let mut changed = false;
                for saved in self.saved_forms(&links) {
                    changed |= self.routing.add(saved);
                }
                if changed {
                    self.save_routing_file();
                }
            }
            Action::SaveWire => {
                let links = self.selected_links();
                let mut changed = false;
                for saved in self.saved_forms(&links) {
                    changed |= self.routing.add(saved);
                }
                if changed {
                    self.save_routing_file();
                }
            }
            Action::ForgetWire => {
                let links = self.selected_links();
                let mut changed = false;
                for saved in self.saved_forms(&links) {
                    changed |= self.routing.remove(&saved);
                }
                if changed {
                    self.save_routing_file();
                }
            }
            Action::RemoveWire => {
                let links = self.selected_links();
                // Forget first, so the service has stopped keeping the link by
                // the time it sees it go.
                let mut changed = false;
                for saved in self.saved_forms(&links) {
                    changed |= self.routing.remove(&saved);
                }
                if changed {
                    self.save_routing_file();
                }
                for link in links {
                    self.connection.unlink(link);
                }
                self.view.selected_wire = None;
            }
            Action::StartService => {
                spawn(&["systemctl", "--user", "start", "otto-mixer-service.service"])
            }
            Action::StartPipeWire => spawn(&[
                "systemctl",
                "--user",
                "start",
                "pipewire.socket",
                "pipewire.service",
                "wireplumber.service",
            ]),
            Action::AirPlay => {
                self.airplay_missing = airplay_missing();
                self.routing.airplay = !self.routing.airplay;
                self.save_routing_file();
            }
            Action::Rate(rate) => {
                self.routing.engine.force_rate = (rate != 0).then_some(rate);
                self.connection
                    .set_setting(SETTING_FORCE_RATE, Some(&rate.to_string()));
                self.save_routing_file();
            }
            Action::Quantum(quantum) => {
                self.routing.engine.force_quantum = (quantum != 0).then_some(quantum);
                self.connection
                    .set_setting(SETTING_FORCE_QUANTUM, Some(&quantum.to_string()));
                self.save_routing_file();
            }
            Action::LowLatency => {
                let (rate, quantum) = (
                    engine_pane::LOW_LATENCY_RATE,
                    engine_pane::LOW_LATENCY_QUANTUM,
                );
                self.routing.engine.force_rate = Some(rate);
                self.routing.engine.force_quantum = Some(quantum);
                self.connection
                    .set_setting(SETTING_FORCE_RATE, Some(&rate.to_string()));
                self.connection
                    .set_setting(SETTING_FORCE_QUANTUM, Some(&quantum.to_string()));
                self.save_routing_file();
            }
            Action::ResetEngine => {
                self.routing.engine = Default::default();
                self.connection.set_setting(SETTING_FORCE_RATE, Some("0"));
                self.connection
                    .set_setting(SETTING_FORCE_QUANTUM, Some("0"));
                self.save_routing_file();
            }
            Action::Virtual(action) => self.run_virtual(action),
            Action::Effect(action) => self.run_effect(action),
            Action::Mixer(action) => self.run_mixer(action),
            Action::Device(action) => self.run_device(action),
            Action::Status(action) => self.run_status(action),
            Action::Profile { device, index } => self.connection.set_profile(device, index),
            Action::Port {
                device,
                index,
                card_device,
            } => self.connection.set_route(device, index, card_device),
            // Opened on the press, by `open_device_menu`.
            Action::DeviceMenu { .. } => {}
            Action::GraphMode(_)
            | Action::LinkCell(..)
            | Action::SelectWire(..)
            | Action::SelectNode(_) => self.run_view(action),
        }
    }
}

/// Run a command without waiting for it.
fn spawn(argv: &[&str]) {
    match std::process::Command::new(argv[0]).args(&argv[1..]).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => tracing::warn!(%error, program = argv[0], "could not start"),
    }
}

/// The sidebar's rows, one per section, the status line saying `status`.
fn sidebar_items(status: String) -> Vec<SourceListItem> {
    SIDEBAR
        .iter()
        .map(|entry| match entry {
            None => SourceListItem::heading(t!("audio-sidebar-advanced")),
            Some(Pane::Status) => SourceListItem::new(status.clone()).pinned_to_bottom(),
            Some(pane) => SourceListItem::new(pane_label(*pane)),
        })
        .collect()
}

fn pane_label(pane: Pane) -> &'static str {
    match pane {
        Pane::Mixer => t!("audio-pane-mixer"),
        Pane::Status => t!("audio-pane-status"),
        Pane::Graph => t!("audio-pane-graph"),
        Pane::Engine => t!("audio-pane-engine"),
        Pane::VirtualDevices => t!("audio-pane-virtual"),
        Pane::Devices => t!("audio-pane-devices"),
        Pane::Effects => t!("audio-pane-effects"),
    }
}

/// A rate in kHz, as people read it: 48 kHz, 44.1 kHz.
fn khz(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        t_owned!("audio-khz", value = format!("{}", khz.round()))
    } else {
        t_owned!("audio-khz", value = format!("{khz:.1}"))
    }
}

/// A rate in kHz for a tight control: 48k, 44.1k.
fn khz_short(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    let value = if (khz - khz.round()).abs() < 0.05 {
        format!("{}", khz.round())
    } else {
        format!("{khz:.1}")
    };
    t_owned!("audio-khz-short", value = value)
}

/// What the engine runs at, read once per frame.
struct EngineNow {
    rate: u32,
    frames: u32,
    default_quantum: u32,
    min: u32,
    max: u32,
    force_rate: Option<u32>,
    force_quantum: Option<u32>,
}

impl EngineNow {
    fn latency_ms(&self) -> f64 {
        f64::from(self.frames) * 1000.0 / f64::from(self.rate.max(1))
    }
}

/// A node's sample format, rate and channels, from what it says of itself.
fn format_of(node: &otto_audio_graph::Node) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(format) = node.prop("audio.format") {
        parts.push(format.to_string());
    }
    if let Some(rate) = node.prop("audio.rate").and_then(|r| r.parse::<u32>().ok()) {
        parts.push(khz(rate));
    }
    if let Some(channels) = node
        .prop("audio.channels")
        .and_then(|c| c.parse::<u32>().ok())
    {
        parts.push(t_owned!("audio-detail-channels", count = channels));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `node.latency`, "256/48000", as milliseconds.
fn latency_of(value: &str) -> Option<String> {
    let (frames, rate) = value.split_once('/')?;
    let frames: f64 = frames.trim().parse().ok()?;
    let rate: f64 = rate.trim().parse().ok()?;
    (rate > 0.0).then(|| {
        t_owned!(
            "audio-ms-value",
            ms = format!("{:.1}", frames * 1000.0 / rate)
        )
    })
}

/// A card's heading: small capitals over its content.
fn card_heading(canvas: &Canvas, text: &str, x: f32, cy: f32, theme: &Theme) {
    Label::new(text.to_uppercase())
        .with_style(styles::CAPTION_2_EMPHASIZED)
        .with_color(theme.text_tertiary)
        .centered_on(x, cy)
        .render(canvas);
}

/// The details panel's disclosure triangle: right when closed, down when open.
fn draw_disclosure(canvas: &Canvas, c: Point, open: bool, theme: &Theme) {
    let mut path = PathBuilder::new();
    if open {
        path.move_to((c.x - 4.0, c.y - 2.0));
        path.line_to((c.x, c.y + 2.5));
        path.line_to((c.x + 4.0, c.y - 2.0));
    } else {
        path.move_to((c.x - 2.0, c.y - 4.0));
        path.line_to((c.x + 2.5, c.y));
        path.line_to((c.x - 2.0, c.y + 4.0));
    }
    let mut paint = stroke(theme.text_secondary, 1.6);
    paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
    paint.set_stroke_join(otto_kit::skia::paint::Join::Round);
    canvas.draw_path(&path.detach(), &paint);
}

fn caption(canvas: &Canvas, text: &str, x: f32, cy: f32, theme: &Theme) {
    Label::new(text)
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_on(x, cy)
        .render(canvas);
}

/// The window's ground: opaque, so the canvas reads as a sheet.
pub fn ground(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x1c, 0x1c, 0x1e)
    } else {
        Color::from_rgb(0xfb, 0xfb, 0xfc)
    }
}

/// The toolbar's and details panel's ground, a step off the canvas.
fn panel_ground(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x26, 0x26, 0x28)
    } else {
        Color::WHITE
    }
}

/// The flat sidebar, for a window the compositor is not frosting.
fn sidebar_ground(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x24, 0x24, 0x26)
    } else {
        Color::from_rgb(0xe3, 0xe3, 0xe7)
    }
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint
}

/// The sidebar glyphs: three linked nodes for Graph, a dial for Engine, a
/// speaker for Devices, three faders for Effects.
///
/// Drawn on a 16-point grid around the centre and scaled to fill `rect`.
/// A pane's sidebar glyph: Otto's Solar one, the same set the Settings
/// sidebar draws, with a drawing of its own where the set isn't installed.
fn draw_pane_glyph(canvas: &Canvas, pane: Pane, rect: Rect, tint: Color) {
    let name = match pane {
        Pane::Graph => "otto-pane-graph-symbolic",
        Pane::Devices => "otto-pane-devices-symbolic",
        Pane::Engine => "otto-pane-engine-symbolic",
        Pane::Effects => "otto-pane-effects-symbolic",
        Pane::VirtualDevices => "otto-pane-virtual-devices-symbolic",
        Pane::Mixer => "otto-pane-sound-symbolic",
        // The status line draws a dot of its own.
        Pane::Status => return,
    };
    // Rasterised in device pixels, so it stays sharp on a scaled output.
    let size = rect.width() * SIDEBAR_GLYPH_SHARE;
    let scale = canvas.local_to_device_as_3x3().scale_x().abs().max(1.0);
    let px = (size * scale).round() as i32;
    if let Some(image) =
        otto_kit::icons::solar_icon(name, px, otto_kit::icons::SolarStyle::Linear, tint)
    {
        let half = size / 2.0;
        let (cx, cy) = (rect.center_x(), rect.center_y());
        let dst = Rect::from_ltrb(cx - half, cy - half, cx + half, cy + half);
        otto_kit::icons::draw_icon(canvas, &image, dst, &Paint::default());
        return;
    }
    draw_pane_glyph_fallback(canvas, pane, rect, tint);
}

fn draw_pane_glyph_fallback(canvas: &Canvas, pane: Pane, rect: Rect, tint: Color) {
    const GRID: f32 = 16.0;
    let scale = rect.width() / GRID;
    canvas.save();
    canvas.translate((rect.center_x(), rect.center_y()));
    canvas.scale((scale, scale));
    let mut paint = stroke(tint, 1.4);
    paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
    let c = Point::new(0.0, 0.0);
    match pane {
        Pane::Graph => {
            let a = Point::new(c.x - 6.0, c.y - 4.0);
            let b = Point::new(c.x - 6.0, c.y + 5.0);
            let d = Point::new(c.x + 6.0, c.y);
            for (from, to) in [(a, d), (b, d)] {
                let mut path = PathBuilder::new();
                path.move_to(from);
                path.cubic_to((from.x + 6.0, from.y), (to.x - 6.0, to.y), to);
                canvas.draw_path(&path.detach(), &paint);
            }
            for dot in [a, b, d] {
                canvas.draw_circle(dot, 2.4, &fill(tint));
            }
        }
        Pane::Engine => {
            let rr =
                RRect::new_rect_xy(Rect::from_xywh(c.x - 7.0, c.y - 7.0, 14.0, 14.0), 7.0, 7.0);
            canvas.draw_rrect(rr, &paint);
            canvas.draw_line(c, Point::new(c.x + 3.5, c.y - 3.5), &paint);
            canvas.draw_circle(c, 1.5, &fill(tint));
        }
        Pane::VirtualDevices => {
            // Two stacked outlines: a device and its copy.
            for (dx, dy) in [(-2.5, -2.5), (2.5, 2.5)] {
                let rr = RRect::new_rect_xy(
                    Rect::from_xywh(c.x - 5.0 + dx, c.y - 5.0 + dy, 10.0, 10.0),
                    2.5,
                    2.5,
                );
                canvas.draw_rrect(rr, &paint);
            }
        }
        Pane::Effects => {
            for (dx, knob) in [(-5.0, 2.5), (0.0, -3.0), (5.0, 1.0)] {
                canvas.draw_line(
                    Point::new(c.x + dx, c.y - 7.0),
                    Point::new(c.x + dx, c.y + 7.0),
                    &paint,
                );
                canvas.draw_circle(Point::new(c.x + dx, c.y + knob), 2.0, &fill(tint));
            }
        }
        Pane::Mixer => {
            // A speaker and two waves.
            let mut path = PathBuilder::new();
            path.move_to((c.x - 7.0, c.y - 2.5));
            path.line_to((c.x - 4.0, c.y - 2.5));
            path.line_to((c.x, c.y - 6.0));
            path.line_to((c.x, c.y + 6.0));
            path.line_to((c.x - 4.0, c.y + 2.5));
            path.line_to((c.x - 7.0, c.y + 2.5));
            path.close();
            canvas.draw_path(&path.detach(), &paint);
            for r in [3.5, 6.5] {
                let oval = Rect::from_xywh(c.x + 1.0 - r, c.y - r, 2.0 * r, 2.0 * r);
                canvas.draw_arc(oval, -45.0, 90.0, false, &paint);
            }
        }
        Pane::Status => {
            canvas.draw_circle(c, 4.0, &fill(tint));
        }
        Pane::Devices => {
            let rr =
                RRect::new_rect_xy(Rect::from_xywh(c.x - 5.5, c.y - 7.5, 11.0, 15.0), 2.5, 2.5);
            canvas.draw_rrect(rr, &paint);
            canvas.draw_circle(Point::new(c.x, c.y + 2.0), 2.6, &paint);
            canvas.draw_circle(Point::new(c.x, c.y - 4.0), 0.9, &fill(tint));
        }
    }
    canvas.restore();
}
