//! The window: its chrome, its two sections and everything the pointer and
//! keyboard do in it.
//!
//! Every control drawn records its rect and action in `targets` while it is
//! painted, and a press is tested against that list, so a press always lands
//! on what was on screen.

// Rust guideline compliant 2026-02-21

use otto_audio::layout::Layout;
use otto_audio::SERVICE_APP;
use otto_audio_graph::{
    identity, Connection, Event, Graph, LinkLifetime, MediaType, NodeState, Routing, SavedLink,
    SETTING_FORCE_QUANTUM, SETTING_FORCE_RATE, SETTING_MAX_QUANTUM, SETTING_MIN_QUANTUM,
    SETTING_QUANTUM, SETTING_RATE,
};
use otto_kit::components::push_button::{self, PushButton, Role as ButtonRole};
use otto_kit::components::scroll::ScrollView;
use otto_kit::components::segmented::{self, Segment, SegmentedInteraction};
use otto_kit::components::source_list::{self, SourceListItem, SourceListLayout};
use otto_kit::components::titlebar::{
    DecorationVariant, WindowControl, WindowControlsState, WindowDecoration,
};
use otto_kit::controls_side::ControlsSide;
use otto_kit::events::{Keysym, Modifiers, PointerEvent, PointerEventKind, BTN_LEFT, BTN_MIDDLE};
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, PaintStyle, PathBuilder, Point, RRect};
use otto_kit::typography::ellipsize;
use otto_kit::{t, t_owned, CursorShape};

use otto_audio::graph_view::{self, media_color, GraphView, Hit, LinkDrag, RowKey, Scene};

/// The sidebar's width.
const SIDEBAR_W: f32 = 196.0;
/// The strip above the canvas with the filters and the save actions.
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
/// The toolbar's filter chips.
const CHIP_H: f32 = 26.0;
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
    Graph,
    Engine,
}

const PANES: [Pane; 2] = [Pane::Graph, Pane::Engine];

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
    Rate(u32),
    Quantum(u32),
    ResetEngine,
    Fit,
    ToggleProperties,
    /// Save the wire between two rows, from the details panel.
    SaveWireOf(RowKey, RowKey),
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
}

impl Ui {
    pub fn new(connection: Connection) -> Self {
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
            controls: WindowControlsState::new(),
            modifiers: Modifiers::default(),
            pane: Pane::Graph,
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
        self.graph.apply(event);
    }

    // -----------------------------------------------------------------------
    // Geometry
    // -----------------------------------------------------------------------

    fn titlebar_h(&self) -> f32 {
        WindowDecoration::height_for(self.variant)
    }

    pub fn decoration(&self) -> WindowDecoration {
        let mut decoration = WindowDecoration::new(t!("audio-title"), self.size.0)
            .with_variant(self.variant)
            .with_active(self.active)
            .with_dark(self.dark)
            .with_blurred(false);
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
    fn viewport(&self) -> Rect {
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

    fn sidebar_layout(&self) -> SourceListLayout {
        SourceListLayout::compute(PANES.len(), 0.0, self.titlebar_h() + 12.0, SIDEBAR_W)
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    /// Open on `pane`.
    pub fn show(&mut self, pane: Pane) {
        self.pane = pane;
    }

    pub fn draw(&mut self, canvas: &Canvas, theme: &Theme) {
        self.targets.clear();
        self.dark = theme.is_dark();
        canvas.clear(ground(theme));
        self.draw_sidebar(canvas, theme);
        if !self.graph.is_connected() {
            self.draw_no_server(canvas, theme);
        } else {
            match self.pane {
                Pane::Graph => self.draw_graph(canvas, theme),
                Pane::Engine => self.draw_engine(canvas, theme),
            }
        }
        let decoration = self.decoration();
        decoration.draw(canvas);
        self.draw_readout(canvas, &decoration, theme);
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
        let font = styles::CAPTION_1.font();
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
            &fill(theme.fill_quaternary),
        );
        Label::new(text)
            .with_style(styles::CAPTION_1)
            .with_color(theme.text_secondary)
            .centered_at(pill.center_x(), cy)
            .render(canvas);
    }

    fn draw_sidebar(&mut self, canvas: &Canvas, theme: &Theme) {
        let top = self.titlebar_h();
        canvas.draw_rect(
            Rect::from_ltrb(0.0, top, SIDEBAR_W, self.size.1),
            &fill(sidebar_ground(theme)),
        );
        canvas.draw_line(
            Point::new(SIDEBAR_W, top),
            Point::new(SIDEBAR_W, self.size.1),
            &stroke(theme.hairline, 1.0),
        );
        let items: Vec<SourceListItem> = PANES
            .iter()
            .map(|pane| SourceListItem::new(pane_label(*pane)))
            .collect();
        let layout = self.sidebar_layout();
        let selected = PANES.iter().position(|p| *p == self.pane);
        source_list::draw(
            canvas,
            &layout,
            &items,
            selected,
            theme,
            |canvas, index, rect, tint| draw_pane_glyph(canvas, PANES[index], rect, tint),
        );
        for (pane, rect) in PANES.iter().zip(&layout.item_rects) {
            self.targets.push((*rect, Action::Pane(*pane)));
        }

        // Whether saved routing is being kept, at the foot.
        if self.graph.is_connected() && self.graph.is_synced() {
            let (dot, text) = if self.service_down() {
                (theme.fill_secondary, t!("audio-service-off"))
            } else {
                (theme.accent_green, t!("audio-service-on"))
            };
            let cy = self.size.1 - 22.0;
            canvas.draw_circle(Point::new(20.0, cy), 4.0, &fill(dot));
            let font = styles::CAPTION_1.font();
            Label::new(ellipsize(&font, text, SIDEBAR_W - 44.0))
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_secondary)
                .centered_on(32.0, cy)
                .render(canvas);
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
        if self.view.auto_fit {
            self.view.fit(&scene, viewport);
        }
        let link_drag = match self.drag {
            Some(Drag::Link(drag)) => Some(drag),
            _ => None,
        };
        graph_view::draw(canvas, &scene, &self.view, viewport, link_drag, theme);
        if scene.cards.is_empty() && self.graph.is_synced() {
            Label::new(t!("audio-graph-empty"))
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .centered_at(viewport.center_x(), viewport.center_y())
                .render(canvas);
        }
        self.draw_toolbar(canvas, &scene, theme);
        if self.service_down() {
            self.draw_banner(canvas, theme);
        }
        if let Some(rect) = self.details_rect() {
            self.draw_details(canvas, rect, &scene, theme);
        }
    }

    fn draw_toolbar(&mut self, canvas: &Canvas, scene: &Scene, theme: &Theme) {
        let content = self.content();
        let bar = Rect::from_xywh(content.left, content.top, content.width(), TOOLBAR_H);
        canvas.draw_rect(bar, &fill(panel_ground(theme)));
        canvas.draw_line(
            Point::new(bar.left, bar.bottom),
            Point::new(bar.right, bar.bottom),
            &stroke(theme.hairline, 1.0),
        );
        let cy = bar.center_y();
        let mut x = bar.left + 12.0;
        let filter = self.view.filter;
        for (media, label, on) in [
            (MediaType::Audio, t!("audio-filter-audio"), filter.audio),
            (MediaType::Midi, t!("audio-filter-midi"), filter.midi),
            (MediaType::Video, t!("audio-filter-video"), filter.video),
        ] {
            let rect = self.chip(canvas, x, cy, label, media, on, theme);
            x = rect.right + 6.0;
        }

        // The actions, from the right edge in.
        let mut right = bar.right - 12.0;
        let top = cy - push_button::HEIGHT / 2.0;
        let mut put = |ui: &mut Self, label: &str, action: Action, enabled: bool, role| {
            let w = push_button::width(label);
            let rect = Rect::from_xywh(right - w, top, w, push_button::HEIGHT);
            ui.role_button(canvas, rect, label, action, enabled, role, theme);
            right = rect.left - 8.0;
        };
        let any_unsaved = scene.wires.iter().any(|w| !w.saved);
        put(
            self,
            t!("audio-save-routing"),
            Action::SaveRouting,
            any_unsaved,
            ButtonRole::Default,
        );
        put(self, t!("audio-fit"), Action::Fit, true, ButtonRole::Normal);
        if let Some(wire) = self
            .view
            .selected_wire
            .and_then(|(from, to)| scene.wire(from, to))
        {
            let saved = wire.saved;
            put(
                self,
                t!("audio-remove-link"),
                Action::RemoveWire,
                true,
                ButtonRole::Normal,
            );
            if saved {
                put(
                    self,
                    t!("audio-forget-link"),
                    Action::ForgetWire,
                    true,
                    ButtonRole::Normal,
                );
            } else {
                put(
                    self,
                    t!("audio-save-link"),
                    Action::SaveWire,
                    true,
                    ButtonRole::Normal,
                );
            }
        }

        // What solid and dashed wires mean, when there is room for it.
        let font = styles::CAPTION_1.font();
        let entries = [
            (t!("audio-legend-saved"), true),
            (t!("audio-legend-unsaved"), false),
        ];
        let widths: Vec<f32> = entries
            .iter()
            .map(|(label, _)| 28.0 + font.measure_str(label, None).0)
            .collect();
        let legend_w = widths.iter().sum::<f32>() + 14.0;
        let mut lx = right - legend_w;
        if lx > x + 16.0 {
            let color = media_color(theme, MediaType::Audio);
            for ((label, saved), w) in entries.iter().zip(&widths) {
                let mut paint = stroke(color, 2.0);
                if !saved {
                    paint.set_path_effect(otto_kit::skia::PathEffect::dash(&[4.0, 3.0], 0.0));
                }
                canvas.draw_line(Point::new(lx, cy), Point::new(lx + 20.0, cy), &paint);
                Label::new(*label)
                    .with_style(styles::CAPTION_1)
                    .with_color(theme.text_secondary)
                    .centered_on(lx + 26.0, cy)
                    .render(canvas);
                lx += w + 14.0;
            }
        }
    }

    /// A filter chip: tinted in its media's colour while on.
    #[expect(
        clippy::too_many_arguments,
        reason = "a chip is placed, labelled and coloured"
    )]
    fn chip(
        &mut self,
        canvas: &Canvas,
        x: f32,
        cy: f32,
        label: &str,
        media: MediaType,
        on: bool,
        theme: &Theme,
    ) -> Rect {
        let font = styles::FOOTNOTE_EMPHASIZED.font();
        let w = font.measure_str(label, None).0 + 36.0;
        let rect = Rect::from_xywh(x, cy - CHIP_H / 2.0, w, CHIP_H);
        let rr = RRect::new_rect_xy(rect, CHIP_H / 2.0, CHIP_H / 2.0);
        let color = media_color(theme, media);
        let action = Action::Filter(media);
        let pressed = self.pressed == Some(action);
        let tint = |alpha: u8| Color::from_argb(alpha, color.r(), color.g(), color.b());
        if on {
            canvas.draw_rrect(rr, &fill(tint(if pressed { 0x38 } else { 0x22 })));
            canvas.draw_rrect(rr, &stroke(tint(0x70), 1.0));
            canvas.draw_circle(Point::new(rect.left + 14.0, cy), 4.0, &fill(color));
        } else {
            if pressed {
                canvas.draw_rrect(rr, &fill(theme.fill_quaternary));
            }
            canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
            canvas.draw_circle(
                Point::new(rect.left + 14.0, cy),
                3.5,
                &stroke(theme.text_tertiary, 1.5),
            );
        }
        let text = if on {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        Label::new(label)
            .with_style(styles::FOOTNOTE_EMPHASIZED)
            .with_color(text)
            .centered_on(rect.left + 24.0, cy)
            .render(canvas);
        self.targets.push((rect, action));
        rect
    }

    fn draw_banner(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        let band = Rect::from_xywh(
            content.left,
            content.top + TOOLBAR_H,
            content.width(),
            BANNER_H,
        );
        let tint = theme.accent_yellow;
        canvas.draw_rect(
            band,
            &fill(Color::from_argb(0x30, tint.r(), tint.g(), tint.b())),
        );
        canvas.draw_line(
            Point::new(band.left, band.bottom),
            Point::new(band.right, band.bottom),
            &stroke(theme.hairline, 1.0),
        );
        let label = t!("audio-service-start");
        let w = push_button::width(label);
        let button = Rect::from_xywh(
            band.right - 16.0 - w,
            band.center_y() - push_button::HEIGHT / 2.0,
            w,
            push_button::HEIGHT,
        );
        let font = styles::SUBHEADLINE.font();
        let text = ellipsize(
            &font,
            t!("audio-service-down"),
            button.left - band.left - 32.0,
        );
        Label::new(text)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(band.left + 16.0, band.center_y())
            .render(canvas);
        self.button(canvas, button, label, Action::StartService, true, theme);
    }

    fn draw_details(&mut self, canvas: &Canvas, rect: Rect, scene: &Scene, theme: &Theme) {
        let Some(id) = self.view.selected_node else {
            return;
        };
        let Some(node) = self.graph.node(id) else {
            return;
        };
        // Everything read from the node up front, so drawing can record
        // targets.
        let title = node.title().to_string();
        let subtitle = graph_view::role(node);
        let (state, state_dot) = match node.state {
            NodeState::Running => (t!("audio-state-running"), theme.accent_green),
            NodeState::Idle => (t!("audio-state-idle"), theme.fill_secondary),
            NodeState::Suspended => (t!("audio-state-suspended"), theme.fill_secondary),
            NodeState::Error => (t!("audio-state-error"), theme.accent_red),
            NodeState::Creating => (t!("audio-state-creating"), theme.fill_secondary),
        };
        let mut facts: Vec<(&str, String)> = Vec::new();
        if let Some(app) = node.app() {
            facts.push((t!("audio-detail-app"), app.to_string()));
        }
        if let Some(format) = format_of(node) {
            facts.push((t!("audio-detail-format"), format));
        }
        if let Some(latency) = node.prop("node.latency").and_then(latency_of) {
            facts.push((t!("audio-detail-latency"), latency));
        }
        facts.push((t!("audio-detail-class"), node.media_class().to_string()));
        if let Some(who) = identity(node) {
            facts.push((t!("audio-detail-identity"), who.to_string()));
        }
        let props: Vec<(String, String)> = node
            .props
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let connections: Vec<(String, bool, RowKey, RowKey)> = scene
            .wires_of(id)
            .map(|wire| {
                let (other, arrow) = if wire.from.node == id {
                    (wire.to.node, "→")
                } else {
                    (wire.from.node, "←")
                };
                let name = self
                    .graph
                    .node(other)
                    .map_or_else(String::new, |n| n.title().to_string());
                (format!("{arrow}  {name}"), wire.saved, wire.from, wire.to)
            })
            .collect();

        canvas.draw_rect(rect, &fill(panel_ground(theme)));
        canvas.draw_line(
            Point::new(rect.left, rect.top),
            Point::new(rect.left, rect.bottom),
            &stroke(theme.hairline, 1.0),
        );
        canvas.save();
        canvas.clip_rect(rect, None, Some(true));
        let left = rect.left + 20.0;
        let width = rect.width() - 40.0;
        let right = left + width;

        let mut y = rect.top + 24.0;
        Label::new(t!("audio-detail-selected").to_uppercase())
            .with_style(styles::CAPTION_2_EMPHASIZED)
            .with_color(theme.text_tertiary)
            .centered_on(left, y)
            .render(canvas);
        y += 24.0;
        let title_font = styles::TITLE_3_EMPHASIZED.font();
        Label::new(ellipsize(&title_font, &title, width))
            .with_style(styles::TITLE_3_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, y)
            .render(canvas);
        y += 22.0;
        let body_font = styles::FOOTNOTE.font();
        Label::new(ellipsize(&body_font, &subtitle, width))
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_secondary)
            .centered_on(left, y)
            .render(canvas);
        y += 30.0;

        // Facts: a label column and its values, the state with its dot.
        let value_x = left + FACT_LABEL_W;
        let value_w = width - FACT_LABEL_W;
        let fact = |label: &str, y: f32| {
            Label::new(ellipsize(&body_font, label, FACT_LABEL_W - 8.0))
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_secondary)
                .centered_on(left, y)
                .render(canvas);
        };
        fact(t!("audio-detail-state"), y);
        canvas.draw_circle(Point::new(value_x + 4.0, y), 4.0, &fill(state_dot));
        Label::new(state)
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_primary)
            .centered_on(value_x + 14.0, y)
            .render(canvas);
        y += FACT_H;
        for (label, value) in &facts {
            fact(label, y);
            Label::new(ellipsize(&body_font, value, value_w))
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_primary)
                .centered_on(value_x, y)
                .render(canvas);
            y += FACT_H;
        }
        y += 14.0;

        // What it is linked to, each with whether it is kept.
        Label::new(t!("audio-detail-links"))
            .with_style(styles::FOOTNOTE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, y)
            .render(canvas);
        y += 14.0;
        if connections.is_empty() {
            y += CONNECTION_H / 2.0;
            Label::new(t!("audio-detail-no-links"))
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_secondary)
                .centered_on(left, y)
                .render(canvas);
            y += CONNECTION_H / 2.0;
        }
        let shown = connections.len().min(MAX_CONNECTIONS);
        for (name, saved, from, to) in connections.iter().take(shown) {
            canvas.draw_line(
                Point::new(left, y),
                Point::new(right, y),
                &stroke(theme.hairline, 1.0),
            );
            let cy = y + CONNECTION_H / 2.0;
            let saved_label = t!("audio-detail-saved");
            let save_label = t!("audio-save");
            let tail_w = if *saved {
                styles::CAPTION_1.font().measure_str(saved_label, None).0
            } else {
                push_button::width(save_label)
            };
            Label::new(ellipsize(&body_font, name, width - tail_w - 12.0))
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_primary)
                .centered_on(left, cy)
                .render(canvas);
            if *saved {
                Label::new(saved_label)
                    .with_style(styles::CAPTION_1)
                    .with_color(theme.text_secondary)
                    .centered_on(right - tail_w, cy)
                    .render(canvas);
            } else {
                let button = Rect::from_xywh(
                    right - tail_w,
                    cy - push_button::HEIGHT / 2.0,
                    tail_w,
                    push_button::HEIGHT,
                );
                self.button(
                    canvas,
                    button,
                    save_label,
                    Action::SaveWireOf(*from, *to),
                    true,
                    theme,
                );
            }
            y += CONNECTION_H;
        }
        if connections.len() > shown {
            let more = t_owned!("audio-detail-more-links", count = connections.len() - shown);
            Label::new(more)
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_secondary)
                .centered_on(left, y + 12.0)
                .render(canvas);
            y += 24.0;
        }
        y += 12.0;

        // Every property, folded away until asked for.
        canvas.draw_line(
            Point::new(left, y),
            Point::new(right, y),
            &stroke(theme.hairline, 1.0),
        );
        let row = Rect::from_xywh(rect.left, y, rect.width(), 32.0);
        let cy = row.center_y();
        draw_disclosure(canvas, Point::new(left + 4.0, cy), self.props_open, theme);
        Label::new(t_owned!(
            "audio-detail-properties-count",
            count = props.len()
        ))
        .with_style(styles::FOOTNOTE_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(left + 16.0, cy)
        .render(canvas);
        self.targets.push((row, Action::ToggleProperties));
        y = row.bottom;
        if self.props_open {
            let list = Rect::from_ltrb(rect.left, y, rect.right, rect.bottom.max(y + 1.0));
            let key_font = styles::CAPTION_2.font();
            let row_h = 2.0 * PROP_LINE_H;
            self.props_scroll.set_viewport(list);
            self.props_scroll
                .set_content_length(props.len() as f32 * row_h + 12.0);
            self.props_scroll.tick();
            // Content-local: x from the list's left edge, y from its top.
            let x = left - rect.left;
            self.props_scroll.render(canvas, theme, |canvas, band| {
                let mut y = 8.0;
                for (key, value) in &props {
                    if y > band.bottom {
                        break;
                    }
                    if y + row_h >= band.top {
                        Label::new(ellipsize(&key_font, key, width))
                            .with_style(styles::CAPTION_2)
                            .with_color(theme.text_secondary)
                            .centered_on(x, y)
                            .render(canvas);
                        Label::new(ellipsize(&body_font, value, width))
                            .with_style(styles::FOOTNOTE)
                            .with_color(theme.text_primary)
                            .centered_on(x, y + 15.0)
                            .render(canvas);
                    }
                    y += row_h;
                }
            });
        }
        canvas.restore();
    }

    /// Whether something drawn is still moving and wants another frame.
    pub fn is_animating(&self) -> bool {
        self.view.selected_node.is_some() && self.props_open && self.props_scroll.is_animating()
    }

    fn draw_engine(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        canvas.draw_rect(content, &fill(graph_view::canvas_ground(theme)));
        let left = content.left + 28.0;
        let right = content.right - 28.0;
        let width = right - left;
        let now = self.engine_now();

        // Title, what the section is for, and Reset.
        let label = t!("audio-engine-reset");
        let reset_w = push_button::width(label);
        let top = content.top + 24.0;
        let forced = now.force_rate.is_some() || now.force_quantum.is_some();
        self.button(
            canvas,
            Rect::from_xywh(right - reset_w, top + 4.0, reset_w, push_button::HEIGHT),
            label,
            Action::ResetEngine,
            forced || !self.routing.engine.is_empty(),
            theme,
        );
        Label::new(t!("audio-engine-title"))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, top + 12.0)
            .render(canvas);
        let intro_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(
            &intro_font,
            t!("audio-engine-intro"),
            width - reset_w - 16.0,
        ))
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_on(left, top + 38.0)
        .render(canvas);

        // Three cards, side by side when there is room.
        let columns = ((width + ENGINE_GAP) / (ENGINE_CARD_MIN_W + ENGINE_GAP))
            .floor()
            .clamp(1.0, 3.0) as usize;
        let card_w = (width - (columns - 1) as f32 * ENGINE_GAP) / columns as f32;
        let card_at = |index: usize| {
            let (col, row) = (index % columns, index / columns);
            Rect::from_xywh(
                left + col as f32 * (card_w + ENGINE_GAP),
                top + 72.0 + row as f32 * (ENGINE_CARD_H + ENGINE_GAP),
                card_w,
                ENGINE_CARD_H,
            )
        };
        for index in 0..3 {
            let rr = RRect::new_rect_xy(card_at(index), 12.0, 12.0);
            canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
            canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        }
        self.draw_latency_card(canvas, card_at(0), &now, theme);
        self.draw_rate_card(canvas, card_at(1), &now, theme);
        self.draw_buffer_card(canvas, card_at(2), &now, theme);

        let below = card_at(2).bottom + 24.0;
        caption(canvas, t!("audio-engine-note"), left, below, theme);
    }

    fn draw_latency_card(&self, canvas: &Canvas, card: Rect, now: &EngineNow, theme: &Theme) {
        let left = card.left + 20.0;
        card_heading(
            canvas,
            t!("audio-engine-latency-now"),
            left,
            card.top + 22.0,
            theme,
        );
        let value = format!("{:.1}", now.latency_ms());
        let big = styles::LARGE_TITLE_EMPHASIZED;
        let w = big.font().measure_str(&value, None).0;
        let cy = card.top + 64.0;
        Label::new(&value)
            .with_style(big)
            .with_color(theme.text_primary)
            .centered_on(left, cy)
            .render(canvas);
        Label::new(t!("audio-ms"))
            .with_style(styles::TITLE_3)
            .with_color(theme.text_secondary)
            .centered_on(left + w + 6.0, cy + 3.0)
            .render(canvas);
        let detail = t_owned!(
            "audio-engine-frames-at",
            frames = now.frames,
            rate = now.rate
        );
        Label::new(detail)
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_secondary)
            .centered_on(left, card.top + 98.0)
            .render(canvas);

        // Who chose each: the engine, or this app.
        let column_w = (card.width() - 40.0) / 2.0;
        let font = styles::CAPTION_1.font();
        for (i, (label, forced)) in [
            (t!("audio-engine-rate"), now.force_rate.is_some()),
            (t!("audio-engine-quantum"), now.force_quantum.is_some()),
        ]
        .into_iter()
        .enumerate()
        {
            let x = left + i as f32 * column_w;
            let chosen = if forced {
                t!("audio-engine-set-here")
            } else {
                t!("audio-engine-auto")
            };
            Label::new(ellipsize(&font, label, column_w - 8.0))
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_secondary)
                .centered_on(x, card.top + 132.0)
                .render(canvas);
            Label::new(chosen)
                .with_style(styles::FOOTNOTE_EMPHASIZED)
                .with_color(theme.text_primary)
                .centered_on(x, card.top + 150.0)
                .render(canvas);
        }
    }

    fn draw_rate_card(&mut self, canvas: &Canvas, card: Rect, now: &EngineNow, theme: &Theme) {
        let left = card.left + 20.0;
        let width = card.width() - 40.0;
        card_heading(
            canvas,
            t!("audio-engine-rate"),
            left,
            card.top + 22.0,
            theme,
        );
        let labels: Vec<String> = RATES
            .iter()
            .map(|r| {
                if *r == 0 {
                    t!("audio-engine-auto").to_string()
                } else {
                    khz_short(*r)
                }
            })
            .collect();
        let chosen = RATES
            .iter()
            .position(|r| Some(*r) == now.force_rate)
            .unwrap_or(0);
        let track = Rect::from_xywh(left, card.top + 44.0, width, segmented::HEIGHT);
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &RATES.map(Action::Rate),
            theme,
        );
        let value = if now.force_rate.is_some() {
            t_owned!("audio-engine-rate-forced", rate = khz(now.rate))
        } else {
            t_owned!("audio-engine-rate-auto", rate = khz(now.rate))
        };
        let font = styles::FOOTNOTE.font();
        Label::new(ellipsize(&font, &value, width))
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_primary)
            .centered_on(left, card.top + 104.0)
            .render(canvas);
        let caption_font = styles::CAPTION_1.font();
        Label::new(ellipsize(
            &caption_font,
            t!("audio-engine-rate-note"),
            width,
        ))
        .with_style(styles::CAPTION_1)
        .with_color(theme.text_secondary)
        .centered_on(left, card.top + 126.0)
        .render(canvas);
    }

    fn draw_buffer_card(&mut self, canvas: &Canvas, card: Rect, now: &EngineNow, theme: &Theme) {
        let left = card.left + 20.0;
        let width = card.width() - 40.0;
        card_heading(
            canvas,
            t!("audio-engine-quantum"),
            left,
            card.top + 22.0,
            theme,
        );
        let gap = 6.0;
        let tile_w = (width - (TILE_COLUMNS - 1) as f32 * gap) / TILE_COLUMNS as f32;
        for (index, quantum) in QUANTA.iter().enumerate() {
            let (col, row) = (index % TILE_COLUMNS, index / TILE_COLUMNS);
            let tile = Rect::from_xywh(
                left + col as f32 * (tile_w + gap),
                card.top + 40.0 + row as f32 * (TILE_H + gap),
                tile_w,
                TILE_H,
            );
            let action = Action::Quantum(*quantum);
            let on = now.force_quantum.unwrap_or(0) == *quantum;
            let pressed = self.pressed == Some(action);
            let rr = RRect::new_rect_xy(tile, 8.0, 8.0);
            let (text, sub) = if on {
                (Color::WHITE, Color::from_argb(0xc0, 0xff, 0xff, 0xff))
            } else {
                (theme.text_primary, theme.text_secondary)
            };
            let ground = if on {
                theme.accent
            } else if pressed {
                theme.fill_tertiary
            } else {
                theme.fill_quaternary
            };
            canvas.draw_rrect(rr, &fill(ground));
            let cx = tile.center_x();
            let cy = tile.center_y();
            if *quantum == 0 {
                Label::new(t!("audio-engine-auto"))
                    .with_style(styles::FOOTNOTE_EMPHASIZED)
                    .with_color(text)
                    .centered_at(cx, cy)
                    .render(canvas);
            } else {
                let ms = f64::from(*quantum) * 1000.0 / f64::from(now.rate.max(1));
                Label::new(quantum.to_string())
                    .with_style(styles::FOOTNOTE_EMPHASIZED)
                    .with_color(text)
                    .centered_at(cx, cy - 7.0)
                    .render(canvas);
                Label::new(t_owned!("audio-ms-value", ms = format!("{ms:.1}")))
                    .with_style(styles::CAPTION_2)
                    .with_color(sub)
                    .centered_at(cx, cy + 9.0)
                    .render(canvas);
            }
            self.targets.push((tile, action));
        }
        let range = t_owned!(
            "audio-engine-quantum-range-short",
            default = now.default_quantum,
            min = now.min,
            max = now.max
        );
        let font = styles::CAPTION_1.font();
        Label::new(ellipsize(&font, &range, width))
            .with_style(styles::CAPTION_1)
            .with_color(theme.text_secondary)
            .centered_on(left, card.top + 158.0)
            .render(canvas);
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
        self.targets
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(p))
            .map(|(_, action)| *action)
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
        if button == BTN_MIDDLE && self.pane == Pane::Graph && self.viewport().contains(p) {
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
        if let Some(action) = self.target_at(p) {
            self.pressed = Some(action);
            return true;
        }
        if self.pane == Pane::Graph && self.graph.is_connected() && self.viewport().contains(p) {
            return self.press_canvas(p);
        }
        false
    }

    fn press_canvas(&mut self, p: Point) -> bool {
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
        let mut redraw = false;
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
        if self.pane != Pane::Graph || !self.viewport().contains(p) {
            return false;
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
        if self.pane != Pane::Graph {
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
                    self.view.fit(&scene, self.viewport());
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
            Action::Pane(pane) => self.pane = pane,
            Action::Fit => {
                let scene = self.view.scene(&self.graph, &self.routing);
                self.view.fit(&scene, self.viewport());
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
                spawn(&["systemctl", "--user", "start", "otto-audio-service.service"])
            }
            Action::StartPipeWire => spawn(&[
                "systemctl",
                "--user",
                "start",
                "pipewire.socket",
                "pipewire.service",
                "wireplumber.service",
            ]),
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
            Action::ResetEngine => {
                self.routing.engine = Default::default();
                self.connection.set_setting(SETTING_FORCE_RATE, Some("0"));
                self.connection
                    .set_setting(SETTING_FORCE_QUANTUM, Some("0"));
                self.save_routing_file();
            }
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

fn pane_label(pane: Pane) -> &'static str {
    match pane {
        Pane::Graph => t!("audio-pane-graph"),
        Pane::Engine => t!("audio-pane-engine"),
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
        Color::from_rgb(0xf5, 0xf5, 0xf7)
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

fn sidebar_ground(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x24, 0x24, 0x26)
    } else {
        Color::from_rgb(0xeb, 0xeb, 0xee)
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

/// The sidebar glyphs: three linked nodes for Graph, a dial for Engine.
fn draw_pane_glyph(canvas: &Canvas, pane: Pane, rect: Rect, tint: Color) {
    let mut paint = stroke(tint, 1.5);
    paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
    let c = Point::new(rect.center_x(), rect.center_y());
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
    }
}
