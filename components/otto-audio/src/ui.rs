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
use otto_kit::components::segmented::{self, Segment, SegmentedInteraction};
use otto_kit::components::source_list::{self, SourceListItem, SourceListLayout};
use otto_kit::components::titlebar::{
    DecorationVariant, WindowControl, WindowControlsState, WindowDecoration,
};
use otto_kit::components::toggle;
use otto_kit::events::{Keysym, Modifiers, PointerEvent, PointerEventKind, BTN_LEFT, BTN_MIDDLE};
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, PaintStyle, PathBuilder, Point, RRect};
use otto_kit::typography::ellipsize;
use otto_kit::{t, t_owned, CursorShape};

use otto_audio::graph_view::{self, GraphView, Hit, LinkDrag, RowKey};

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
    props_scroll: f32,
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
            props_scroll: 0.0,
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
        self.decoration().draw(canvas);
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
        let button = PushButton::new(ButtonRole::Normal)
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
            self.draw_details(canvas, rect, theme);
        }
    }

    fn draw_toolbar(&mut self, canvas: &Canvas, scene: &graph_view::Scene, theme: &Theme) {
        let content = self.content();
        let bar = Rect::from_xywh(content.left, content.top, content.width(), TOOLBAR_H);
        canvas.draw_rect(bar, &fill(ground(theme)));
        canvas.draw_line(
            Point::new(bar.left, bar.bottom),
            Point::new(bar.right, bar.bottom),
            &stroke(theme.hairline, 1.0),
        );
        let cy = bar.center_y();
        let mut x = bar.left + 16.0;
        let filter = self.view.filter;
        for (media, label, on) in [
            (MediaType::Audio, t!("audio-filter-audio"), filter.audio),
            (MediaType::Midi, t!("audio-filter-midi"), filter.midi),
            (MediaType::Video, t!("audio-filter-video"), filter.video),
        ] {
            let font = styles::SUBHEADLINE.font();
            let w = font.measure_str(label, None).0;
            Label::new(label)
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_primary)
                .centered_on(x, cy)
                .render(canvas);
            let switch = Rect::from_xywh(x + w + 8.0, cy - 10.0, 34.0, 20.0);
            toggle::draw(
                canvas,
                switch,
                toggle::knob_fraction_for(on),
                ToggleInteraction::Normal,
                theme,
            );
            self.targets.push((
                Rect::from_ltrb(x - 4.0, bar.top + 6.0, switch.right + 4.0, bar.bottom - 6.0),
                Action::Filter(media),
            ));
            x = switch.right + 20.0;
        }

        // The save actions, from the right edge in.
        let mut right = bar.right - 16.0;
        let top = cy - push_button::HEIGHT / 2.0;
        let mut put = |ui: &mut Self, label: &str, action: Action, enabled: bool| {
            let w = push_button::width(label);
            let rect = Rect::from_xywh(right - w, top, w, push_button::HEIGHT);
            ui.button(canvas, rect, label, action, enabled, theme);
            right = rect.left - 8.0;
        };
        let any_unsaved = scene.wires.iter().any(|w| !w.saved);
        put(
            self,
            t!("audio-save-routing"),
            Action::SaveRouting,
            any_unsaved,
        );
        if let Some(wire) = self
            .view
            .selected_wire
            .and_then(|(from, to)| scene.wire(from, to))
        {
            let saved = wire.saved;
            put(self, t!("audio-remove-link"), Action::RemoveWire, true);
            if saved {
                put(self, t!("audio-forget-link"), Action::ForgetWire, true);
            } else {
                put(self, t!("audio-save-link"), Action::SaveWire, true);
            }
        }
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

    fn draw_details(&mut self, canvas: &Canvas, rect: Rect, theme: &Theme) {
        let Some(node) = self.view.selected_node.and_then(|id| self.graph.node(id)) else {
            return;
        };
        canvas.draw_rect(rect, &fill(sidebar_ground(theme)));
        canvas.draw_line(
            Point::new(rect.left, rect.top),
            Point::new(rect.left, rect.bottom),
            &stroke(theme.hairline, 1.0),
        );
        canvas.save();
        canvas.clip_rect(rect, None, Some(true));
        let left = rect.left + 16.0;
        let width = rect.width() - 32.0;
        let title_font = styles::HEADLINE.font();
        Label::new(ellipsize(&title_font, node.title(), width))
            .with_style(styles::HEADLINE)
            .with_color(theme.text_primary)
            .centered_on(left, rect.top + 24.0)
            .render(canvas);

        let state = match node.state {
            NodeState::Running => t!("audio-state-running"),
            NodeState::Idle => t!("audio-state-idle"),
            NodeState::Suspended => t!("audio-state-suspended"),
            NodeState::Error => t!("audio-state-error"),
            NodeState::Creating => t!("audio-state-creating"),
        };
        let who = identity(node).map(|i| i.to_string()).unwrap_or_default();
        let facts = [
            (
                t!("audio-detail-app"),
                node.app().unwrap_or("—").to_string(),
            ),
            (t!("audio-detail-class"), node.media_class().to_string()),
            (t!("audio-detail-state"), state.to_string()),
            (t!("audio-detail-identity"), who),
        ];
        let mut y = rect.top + 52.0;
        let value_font = styles::FOOTNOTE.font();
        for (label, value) in facts {
            Label::new(label)
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_secondary)
                .centered_on(left, y)
                .render(canvas);
            Label::new(ellipsize(&value_font, &value, width))
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_primary)
                .centered_on(left, y + 16.0)
                .render(canvas);
            y += 38.0;
        }
        Label::new(t!("audio-detail-properties"))
            .with_style(styles::CAPTION_1_EMPHASIZED)
            .with_color(theme.text_secondary)
            .centered_on(left, y + 4.0)
            .render(canvas);
        let list_top = y + 18.0;
        let list = Rect::from_ltrb(rect.left, list_top, rect.right, rect.bottom);
        canvas.clip_rect(list, None, Some(true));
        let key_font = styles::CAPTION_2.font();
        let lines = node.props.len() as f32;
        let max_scroll = (lines * PROP_LINE_H * 2.0 - list.height() + 12.0).max(0.0);
        self.props_scroll = self.props_scroll.clamp(0.0, max_scroll);
        let mut y = list_top + 8.0 - self.props_scroll;
        for (key, value) in &node.props {
            if y > rect.bottom {
                break;
            }
            if y + 2.0 * PROP_LINE_H >= list_top {
                Label::new(ellipsize(&key_font, key, width))
                    .with_style(styles::CAPTION_2)
                    .with_color(theme.text_secondary)
                    .centered_on(left, y)
                    .render(canvas);
                Label::new(ellipsize(&value_font, value, width))
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_primary)
                    .centered_on(left, y + 15.0)
                    .render(canvas);
            }
            y += 2.0 * PROP_LINE_H;
        }
        canvas.restore();
    }

    fn draw_engine(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        let left = content.left + 32.0;
        let width = (content.width() - 64.0).min(560.0);
        let mut y = content.top + 36.0;
        Label::new(t!("audio-engine-title"))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, y)
            .render(canvas);
        y += 44.0;

        let rate = self.graph.setting_u32(SETTING_RATE).unwrap_or(48_000);
        let force_rate = self.graph.setting_u32(SETTING_FORCE_RATE);
        let running_rate = force_rate.unwrap_or(rate);
        let quantum = self.graph.setting_u32(SETTING_QUANTUM).unwrap_or(1024);
        let force_quantum = self.graph.setting_u32(SETTING_FORCE_QUANTUM);
        let min = self.graph.setting_u32(SETTING_MIN_QUANTUM).unwrap_or(32);
        let max = self.graph.setting_u32(SETTING_MAX_QUANTUM).unwrap_or(2048);

        // Sample rate.
        heading(canvas, t!("audio-engine-rate"), left, y, theme);
        let value = if force_rate.is_some() {
            t_owned!("audio-engine-rate-forced", rate = khz(running_rate))
        } else {
            t_owned!("audio-engine-rate-auto", rate = khz(rate))
        };
        caption(canvas, &value, left, y + 20.0, theme);
        y += 40.0;
        let labels: Vec<String> = RATES
            .iter()
            .map(|r| {
                if *r == 0 {
                    t!("audio-engine-auto").to_string()
                } else {
                    khz(*r)
                }
            })
            .collect();
        let chosen = RATES
            .iter()
            .position(|r| Some(*r) == force_rate)
            .unwrap_or(0);
        let track = Rect::from_xywh(left, y, width, segmented::HEIGHT);
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &RATES.map(Action::Rate),
            theme,
        );
        y += 64.0;

        // Quantum.
        heading(canvas, t!("audio-engine-quantum"), left, y, theme);
        let frames = force_quantum.unwrap_or(quantum);
        let ms = f64::from(frames) * 1000.0 / f64::from(running_rate.max(1));
        let value = t_owned!(
            "audio-engine-quantum-now",
            frames = frames,
            ms = format!("{ms:.1}")
        );
        caption(canvas, &value, left, y + 20.0, theme);
        y += 40.0;
        let labels: Vec<String> = QUANTA
            .iter()
            .map(|q| {
                if *q == 0 {
                    t!("audio-engine-auto").to_string()
                } else {
                    q.to_string()
                }
            })
            .collect();
        let chosen = QUANTA
            .iter()
            .position(|q| Some(*q) == force_quantum)
            .unwrap_or(0);
        let track = Rect::from_xywh(left, y, width, segmented::HEIGHT);
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &QUANTA.map(Action::Quantum),
            theme,
        );
        y += 40.0;
        let range = t_owned!(
            "audio-engine-quantum-range",
            default = quantum,
            min = min,
            max = max
        );
        caption(canvas, &range, left, y, theme);
        y += 40.0;

        let label = t!("audio-engine-reset");
        let w = push_button::width(label);
        let forced = force_rate.is_some() || force_quantum.is_some();
        self.button(
            canvas,
            Rect::from_xywh(left, y, w, push_button::HEIGHT),
            label,
            Action::ResetEngine,
            forced || !self.routing.engine.is_empty(),
            theme,
        );
        y += push_button::HEIGHT + 24.0;
        caption(canvas, t!("audio-engine-note"), left, y, theme);
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
                    redraw |= self.scroll(p, horizontal.absolute as f32, vertical.absolute as f32);
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
                    self.props_scroll = 0.0;
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

    fn scroll(&mut self, p: Point, dx: f32, dy: f32) -> bool {
        if self.details_rect().is_some_and(|r| r.contains(p)) {
            self.props_scroll += dy;
            return true;
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

fn heading(canvas: &Canvas, text: &str, x: f32, cy: f32, theme: &Theme) {
    Label::new(text)
        .with_style(styles::HEADLINE)
        .with_color(theme.text_primary)
        .centered_on(x, cy)
        .render(canvas);
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
