//! The Graph section: every node as a card, links as wires between them.
//!
//! The scene is built from the graph each time it is drawn or hit, in canvas
//! points ("world" coordinates); the view's pan and zoom map it onto the
//! window. Cards keep the place they were first given, or the one the person
//! dragged them to, so nothing moves under the pointer when a node appears.

// Rust guideline compliant 2026-02-21

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::layout::{card_key, Layout};
use otto_audio_graph::{
    identity, pair_channels, Direction, Graph, MediaType, Node, NodeState, Routing, SavedLink,
};
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, PaintStyle, PathBuilder, PathEffect, Point, RRect};
use otto_kit::typography::ellipsize;

/// A card's width.
pub const CARD_W: f32 = 220.0;
/// The header with the node's name and what it is.
const HEADER_H: f32 = 44.0;
/// One port row.
const ROW_H: f32 = 22.0;
/// Room under the last row.
const CARD_PAD: f32 = 8.0;
/// Between two columns of cards: room for the wires to curve.
const COLUMN_GAP: f32 = 90.0;
/// Between two cards of one column.
const CARD_GAP: f32 = 18.0;
/// A port's connector dot.
const CONNECTOR_R: f32 = 4.5;
/// How close, in window points, a press must be to a wire to pick it.
const WIRE_SLOP: f32 = 6.0;
/// The fold chevron's square, at the header's right end.
const CHEVRON: f32 = 22.0;
/// The card corner radius.
const CARD_RADIUS: f32 = 10.0;
/// The canvas's dot grid, in world points.
const GRID: f32 = 20.0;
/// How far above a column's first card its heading sits, in world points.
const COLUMN_HEADING: f32 = 22.0;
/// The zoom range.
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 2.0;

/// The most lanes the Lanes view lists: paths multiply through a busy mixer,
/// and past a few hundred the list stops being readable anyway.
const MAX_LANES: usize = 400;

/// How the Graph section shows the routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphMode {
    /// Cards and wires on a canvas.
    #[default]
    Patchbay,
    /// What makes sound as rows, what takes it as columns, a cell per pair.
    Matrix,
    /// Every path from a source to a destination, one per lane.
    Lanes,
}

impl GraphMode {
    /// Every view, in the order the toolbar's switch shows them.
    pub const ALL: [Self; 3] = [Self::Patchbay, Self::Matrix, Self::Lanes];

    /// The name it is remembered by in the layout file.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Patchbay => "patchbay",
            Self::Matrix => "matrix",
            Self::Lanes => "lanes",
        }
    }

    /// The view remembered as `key`; `None` for anything else.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

/// Which media the canvas shows.
#[derive(Debug, Clone, Copy)]
pub struct Filter {
    pub audio: bool,
    pub midi: bool,
    pub video: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            audio: true,
            midi: true,
            video: false,
        }
    }
}

impl Filter {
    /// Whether ports of `media` are shown.
    pub fn shows(self, media: MediaType) -> bool {
        match media {
            MediaType::Audio => self.audio,
            MediaType::Midi => self.midi,
            MediaType::Video => self.video,
            MediaType::Other => false,
        }
    }

    pub fn toggle(&mut self, media: MediaType) {
        match media {
            MediaType::Audio => self.audio = !self.audio,
            MediaType::Midi => self.midi = !self.midi,
            MediaType::Video => self.video = !self.video,
            MediaType::Other => {}
        }
    }
}

/// One row of ports on a card: a single port, or every port of one media in
/// one direction when the card is folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowKey {
    pub node: u32,
    pub direction: Direction,
    pub media: MediaType,
    /// The port, or `None` for a folded row.
    pub port: Option<u32>,
}

/// A row as laid out.
#[derive(Debug, Clone)]
pub struct Row {
    pub key: RowKey,
    /// Its ports with their channels, in port order.
    pub ports: Vec<(u32, Option<String>)>,
    pub label: String,
    /// Where wires attach, in world points.
    pub anchor: Point,
}

/// A card as laid out.
#[derive(Debug, Clone)]
pub struct Card {
    pub node: u32,
    pub rect: Rect,
    pub title: String,
    pub subtitle: String,
    pub state: NodeState,
    pub inputs: Vec<Row>,
    pub outputs: Vec<Row>,
    /// Whether any row stands for more than one port, or could.
    pub foldable: bool,
    pub folded: bool,
}

impl Card {
    fn chevron_rect(&self) -> Rect {
        Rect::from_xywh(
            self.rect.right - CHEVRON - 6.0,
            self.rect.top + (HEADER_H - CHEVRON) / 2.0,
            CHEVRON,
            CHEVRON,
        )
    }

    fn rows(&self) -> impl Iterator<Item = &Row> {
        self.inputs.iter().chain(&self.outputs)
    }
}

/// A wire as laid out: every link between two rows, drawn as one.
#[derive(Debug, Clone)]
pub struct Wire {
    pub from: RowKey,
    pub to: RowKey,
    pub a: Point,
    pub b: Point,
    pub links: Vec<u32>,
    pub media: MediaType,
    /// Every link of it is saved.
    pub saved: bool,
}

impl Wire {
    fn folded(&self) -> bool {
        self.from.port.is_none() || self.to.port.is_none()
    }

    /// Points along the curve, for hit-testing.
    fn samples(&self) -> impl Iterator<Item = Point> + '_ {
        const STEPS: usize = 24;
        let (c1, c2) = controls(self.a, self.b);
        (0..=STEPS).map(move |i| {
            let t = i as f32 / STEPS as f32;
            let u = 1.0 - t;
            let x = u * u * u * self.a.x
                + 3.0 * u * u * t * c1.x
                + 3.0 * u * t * t * c2.x
                + t * t * t * self.b.x;
            let y = u * u * u * self.a.y
                + 3.0 * u * u * t * c1.y
                + 3.0 * u * t * t * c2.y
                + t * t * t * self.b.y;
            Point::new(x, y)
        })
    }
}

/// The curve's control points: out of the output to the right, into the
/// input from the left.
fn controls(a: Point, b: Point) -> (Point, Point) {
    let dx = ((b.x - a.x).abs() * 0.5).max(60.0);
    (Point::new(a.x + dx, a.y), Point::new(b.x - dx, b.y))
}

/// Everything on the canvas.
#[derive(Debug, Default)]
pub struct Scene {
    pub cards: Vec<Card>,
    pub wires: Vec<Wire>,
    /// The default columns some card belongs to, left to right.
    pub columns: Vec<usize>,
}

/// What a press landed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Chevron(u32),
    Row(RowKey),
    /// The card itself: its header or its body between rows.
    Card(u32),
    Wire(usize),
    Nothing,
}

impl Scene {
    pub fn card(&self, node: u32) -> Option<&Card> {
        self.cards.iter().find(|c| c.node == node)
    }

    pub fn row(&self, key: RowKey) -> Option<&Row> {
        self.card(key.node)?.rows().find(|r| r.key == key)
    }

    pub fn wire(&self, from: RowKey, to: RowKey) -> Option<&Wire> {
        self.wires.iter().find(|w| w.from == from && w.to == to)
    }

    /// The wires into or out of `node`.
    pub fn wires_of(&self, node: u32) -> impl Iterator<Item = &Wire> {
        self.wires
            .iter()
            .filter(move |w| w.from.node == node || w.to.node == node)
    }

    /// What is under `p` (world points) at `zoom`.
    pub fn hit(&self, p: Point, zoom: f32) -> Hit {
        for card in self.cards.iter().rev() {
            if !card.rect.with_outset((CONNECTOR_R * 2.0, 0.0)).contains(p) {
                continue;
            }
            if card.foldable && card.chevron_rect().contains(p) {
                return Hit::Chevron(card.node);
            }
            let half = card.rect.center_x();
            for row in &card.inputs {
                if (p.y - row.anchor.y).abs() <= ROW_H / 2.0 && p.x <= half {
                    return Hit::Row(row.key);
                }
            }
            for row in &card.outputs {
                if (p.y - row.anchor.y).abs() <= ROW_H / 2.0 && p.x > half {
                    return Hit::Row(row.key);
                }
            }
            if card.rect.contains(p) {
                return Hit::Card(card.node);
            }
        }
        let slop = WIRE_SLOP / zoom;
        let mut best: Option<(usize, f32)> = None;
        for (index, wire) in self.wires.iter().enumerate() {
            let samples: Vec<Point> = wire.samples().collect();
            for pair in samples.windows(2) {
                let d = distance_to_segment(p, pair[0], pair[1]);
                if d <= slop && best.is_none_or(|(_, b)| d < b) {
                    best = Some((index, d));
                }
            }
        }
        best.map_or(Hit::Nothing, |(index, _)| Hit::Wire(index))
    }

    /// The union of every card's rect, and the column headings over them.
    pub fn bounds(&self) -> Option<Rect> {
        let mut cards = self.cards.iter().map(|c| c.rect);
        let first = cards.next()?;
        let headings = (!self.columns.is_empty())
            .then(|| Rect::from_xywh(0.0, -COLUMN_HEADING - 8.0, CARD_W, COLUMN_HEADING));
        Some(cards.chain(headings).fold(first, |acc, r| {
            Rect::from_ltrb(
                acc.left.min(r.left),
                acc.top.min(r.top),
                acc.right.max(r.right),
                acc.bottom.max(r.bottom),
            )
        }))
    }
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let ab = b - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2).clamp(0.0, 1.0)
    };
    let q = Point::new(a.x + ab.x * t, a.y + ab.y * t);
    ((p.x - q.x).powi(2) + (p.y - q.y).powi(2)).sqrt()
}

/// The column a node goes in by default: 0 what produces sound, 1 effects and
/// virtual devices, 2 what plays or records it, 3 MIDI and video.
pub fn column(node: &Node) -> usize {
    match node.media_type() {
        MediaType::Midi | MediaType::Video => return 3,
        MediaType::Audio | MediaType::Other => {}
    }
    let class = node.media_class();
    let internal = node.prop("node.virtual") == Some("true") && node.app().is_none();
    if node.is_stream() && internal {
        1
    } else if class.starts_with("Stream/Output") {
        0
    } else if class.starts_with("Stream/Input") {
        2
    } else if class.starts_with("Audio/Source") && node.is_device() {
        0
    } else if class.starts_with("Audio/Sink") && (node.is_device() || node.is_airplay()) {
        2
    } else {
        1
    }
}

/// The heading over a default column.
fn column_heading(column: usize) -> &'static str {
    match column {
        0 => otto_kit::t!("audio-column-sources"),
        1 => otto_kit::t!("audio-column-between"),
        2 => otto_kit::t!("audio-column-destinations"),
        _ => otto_kit::t!("audio-column-midi-video"),
    }
}

/// What a node is, in a few words, for its card.
pub fn role(node: &Node) -> String {
    let class = node.media_class();
    let what = if class.starts_with("Stream/Output") {
        otto_kit::t!("audio-role-playing")
    } else if class.starts_with("Stream/Input") {
        otto_kit::t!("audio-role-recording")
    } else if class.starts_with("Audio/Sink") {
        if node.is_airplay() {
            otto_kit::t!("audio-role-airplay")
        } else if node.is_device() {
            otto_kit::t!("audio-role-output-device")
        } else {
            otto_kit::t!("audio-role-virtual-output")
        }
    } else if class.starts_with("Audio/Source") {
        if node.is_device() {
            otto_kit::t!("audio-role-input-device")
        } else {
            otto_kit::t!("audio-role-virtual-input")
        }
    } else if class.starts_with("Audio/Duplex") {
        otto_kit::t!("audio-role-duplex")
    } else if class.starts_with("Midi") {
        otto_kit::t!("audio-role-midi")
    } else if class.contains("Video") {
        otto_kit::t!("audio-role-video")
    } else {
        otto_kit::t!("audio-role-other")
    };
    match node.app().filter(|app| *app != node.title()) {
        Some(app) => format!("{what} · {app}"),
        None => what.to_string(),
    }
}

/// The label of a folded row: its channels, or how many there are.
fn folded_label(ports: &[(u32, Option<String>)], names: &[&str], monitor: bool) -> String {
    let channels: Vec<&str> = ports
        .iter()
        .zip(names)
        .map(|((_, channel), name)| channel.as_deref().unwrap_or(name))
        .collect();
    let list = if channels.len() <= 4 {
        channels.join(" ")
    } else {
        otto_kit::t_owned!("audio-channels", count = channels.len())
    };
    if monitor {
        otto_kit::t_owned!("audio-monitor", channels = list)
    } else {
        list
    }
}

/// The canvas's state: where the view is, where each card is, what is
/// selected and folded.
#[derive(Debug)]
pub struct GraphView {
    /// Window points from the viewport's top-left to the world origin.
    pub pan: Point,
    pub zoom: f32,
    pub filter: Filter,
    /// Every card's place, by node id, once given.
    positions: HashMap<u32, Point>,
    /// Places remembered across launches.
    pub layout: Layout,
    unfolded: HashSet<u32>,
    pub selected_node: Option<u32>,
    pub selected_wire: Option<(RowKey, RowKey)>,
    /// Fit the view to the cards on every draw, until the person moves it.
    pub auto_fit: bool,
    /// Which view the section shows.
    pub mode: GraphMode,
}

impl GraphView {
    pub fn new(layout: Layout) -> Self {
        Self {
            mode: layout
                .view()
                .and_then(GraphMode::from_key)
                .unwrap_or_default(),
            pan: Point::new(24.0, 24.0),
            zoom: 1.0,
            filter: Filter::default(),
            positions: HashMap::new(),
            layout,
            unfolded: HashSet::new(),
            selected_node: None,
            selected_wire: None,
            auto_fit: true,
        }
    }

    /// The server went away: every id is meaningless now.
    pub fn forget_ids(&mut self) {
        self.positions.clear();
        self.unfolded.clear();
        self.selected_node = None;
        self.selected_wire = None;
    }

    pub fn toggle_fold(&mut self, node: u32) {
        if !self.unfolded.remove(&node) {
            self.unfolded.insert(node);
        }
    }

    pub fn to_world(&self, viewport: Rect, p: Point) -> Point {
        Point::new(
            (p.x - viewport.left - self.pan.x) / self.zoom,
            (p.y - viewport.top - self.pan.y) / self.zoom,
        )
    }

    /// Zoom by `factor` keeping the world point under `focus` (window points)
    /// where it is.
    pub fn zoom_about(&mut self, viewport: Rect, focus: Point, factor: f32) {
        let before = self.to_world(viewport, focus);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = Point::new(
            focus.x - viewport.left - before.x * self.zoom,
            focus.y - viewport.top - before.y * self.zoom,
        );
    }

    /// Zoom and pan so every card fits `viewport`.
    pub fn fit(&mut self, scene: &Scene, viewport: Rect) {
        let Some(bounds) = scene.bounds() else {
            return;
        };
        let margin = 32.0;
        let zoom_x = (viewport.width() - 2.0 * margin) / bounds.width().max(1.0);
        let zoom_y = (viewport.height() - 2.0 * margin) / bounds.height().max(1.0);
        self.zoom = zoom_x.min(zoom_y).clamp(MIN_ZOOM, 1.0);
        self.pan = Point::new(
            (viewport.width() - bounds.width() * self.zoom) / 2.0 - bounds.left * self.zoom,
            (viewport.height() - bounds.height() * self.zoom) / 2.0 - bounds.top * self.zoom,
        );
    }

    /// Move `node`'s card to `at` (world points).
    pub fn move_card(&mut self, node: u32, at: Point) {
        self.positions.insert(node, at);
    }

    /// Remember where `node`'s card is, across launches.
    pub fn remember(&mut self, graph: &Graph, node: u32) {
        let (Some(key), Some(at)) = (self.card_key(graph, node), self.positions.get(&node)) else {
            return;
        };
        self.layout.set(key, (at.x, at.y));
        self.save_layout();
    }

    /// Show `mode`, and remember it across launches.
    pub fn set_mode(&mut self, mode: GraphMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.layout.set_view(mode.key());
        self.save_layout();
    }

    fn save_layout(&self) {
        if let Some(path) = Layout::path() {
            if let Err(error) = self.layout.save_to(&path) {
                tracing::warn!(%error, "the card layout could not be saved");
            }
        }
    }

    /// The key `node` is remembered by.
    fn card_key(&self, graph: &Graph, node: u32) -> Option<String> {
        let own = identity(graph.node(node)?)?;
        let index = graph
            .nodes()
            .filter(|n| n.id < node && identity(n).as_ref() == Some(&own))
            .count();
        Some(card_key(&own.to_string(), index))
    }

    /// Lay the canvas out from `graph`.
    pub fn scene(&mut self, graph: &Graph, routing: &Routing) -> Scene {
        let mut scene = Scene::default();
        let mut columns: BTreeMap<u32, usize> = BTreeMap::new();
        let mut unplaced = Vec::new();
        for node in graph.nodes() {
            // A node shows once its details arrive, so it lands in the right
            // column the first time.
            if node.state == NodeState::Creating {
                continue;
            }
            let Some(mut card) = self.card(graph, node) else {
                continue;
            };
            columns.insert(node.id, column(node));
            if let Some(at) = self.positions.get(&node.id) {
                place(&mut card, *at);
                scene.cards.push(card);
            } else {
                unplaced.push(card);
            }
        }
        for mut card in unplaced {
            let at = self
                .card_key(graph, card.node)
                .and_then(|key| self.layout.get(&key))
                .map(|(x, y)| Point::new(x, y))
                .unwrap_or_else(|| {
                    let col = columns.get(&card.node).copied().unwrap_or(1);
                    let bottom = scene
                        .cards
                        .iter()
                        .filter(|c| columns.get(&c.node) == Some(&col))
                        .map(|c| c.rect.bottom + CARD_GAP)
                        .fold(0.0_f32, f32::max);
                    Point::new(col as f32 * (CARD_W + COLUMN_GAP), bottom)
                });
            self.positions.insert(card.node, at);
            place(&mut card, at);
            scene.cards.push(card);
        }
        scene.wires = wires(&scene, graph, routing);
        let mut used: Vec<usize> = columns.values().copied().collect();
        used.sort_unstable();
        used.dedup();
        scene.columns = used;
        scene
    }

    /// One node's card, laid out at the origin; `None` when the filter hides
    /// every port it has.
    fn card(&self, graph: &Graph, node: &Node) -> Option<Card> {
        let folded = !self.unfolded.contains(&node.id);
        let mut foldable = false;
        let mut side = |direction: Direction| -> Vec<Row> {
            let mut groups: Vec<(MediaType, Vec<&otto_audio_graph::Port>)> = Vec::new();
            // In the node's own port order (FL before FR), which global ids
            // do not always follow.
            let mut ports: Vec<&otto_audio_graph::Port> =
                graph.ports_of(node.id, direction).collect();
            ports.sort_by_key(|p| {
                p.props
                    .get("port.id")
                    .and_then(|i| i.parse::<u32>().ok())
                    .unwrap_or(u32::MAX)
            });
            for port in ports {
                let media = graph.port_media(port);
                if !self.filter.shows(media) {
                    continue;
                }
                match groups.iter_mut().find(|(m, _)| *m == media) {
                    Some((_, ports)) => ports.push(port),
                    None => groups.push((media, vec![port])),
                }
            }
            let mut rows = Vec::new();
            for (media, ports) in groups {
                foldable |= ports.len() > 1;
                let key = |port: Option<u32>| RowKey {
                    node: node.id,
                    direction,
                    media,
                    port,
                };
                if folded && ports.len() > 1 {
                    let entries: Vec<(u32, Option<String>)> =
                        ports.iter().map(|p| (p.id, p.channel.clone())).collect();
                    let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
                    let monitor = ports.iter().all(|p| p.monitor);
                    rows.push(Row {
                        key: key(None),
                        label: folded_label(&entries, &names, monitor),
                        ports: entries,
                        anchor: Point::default(),
                    });
                } else {
                    for port in ports {
                        rows.push(Row {
                            key: key(Some(port.id)),
                            ports: vec![(port.id, port.channel.clone())],
                            label: port.label().to_string(),
                            anchor: Point::default(),
                        });
                    }
                }
            }
            rows
        };
        let inputs = side(Direction::Input);
        let outputs = side(Direction::Output);
        if inputs.is_empty() && outputs.is_empty() {
            return None;
        }
        let rows = inputs.len().max(outputs.len()) as f32;
        Some(Card {
            node: node.id,
            rect: Rect::from_wh(CARD_W, HEADER_H + rows * ROW_H + CARD_PAD),
            title: node.title().to_string(),
            subtitle: role(node),
            state: node.state,
            inputs,
            outputs,
            foldable,
            folded,
        })
    }

    /// The links between the ports of `from` and `to`, to make: their
    /// channels paired, and any pair already linked left out. Empty when the
    /// two rows cannot be linked.
    pub fn pairs_to_link(
        &self,
        scene: &Scene,
        graph: &Graph,
        from: RowKey,
        to: RowKey,
    ) -> Vec<(u32, u32)> {
        if !compatible(from, to) {
            return Vec::new();
        }
        let (out, inp) = match from.direction {
            Direction::Output => (from, to),
            Direction::Input => (to, from),
        };
        let (Some(out), Some(inp)) = (scene.row(out), scene.row(inp)) else {
            return Vec::new();
        };
        let side = |row: &Row| -> Vec<(u32, Option<String>)> { row.ports.clone() };
        let outs = side(out);
        let ins = side(inp);
        let outs: Vec<(u32, Option<&str>)> =
            outs.iter().map(|(id, c)| (*id, c.as_deref())).collect();
        let ins: Vec<(u32, Option<&str>)> = ins.iter().map(|(id, c)| (*id, c.as_deref())).collect();
        pair_channels(&outs, &ins)
            .into_iter()
            .filter(|(o, i)| graph.link_between(*o, *i).is_none())
            .collect()
    }
}

/// Whether a wire can be drawn from row `a` to row `b`.
pub fn compatible(a: RowKey, b: RowKey) -> bool {
    a.node != b.node && a.direction != b.direction && a.media == b.media
}

fn place(card: &mut Card, at: Point) {
    card.rect = Rect::from_xywh(at.x, at.y, card.rect.width(), card.rect.height());
    let top = card.rect.top + HEADER_H + ROW_H / 2.0;
    for (i, row) in card.inputs.iter_mut().enumerate() {
        row.anchor = Point::new(card.rect.left, top + i as f32 * ROW_H);
    }
    for (i, row) in card.outputs.iter_mut().enumerate() {
        row.anchor = Point::new(card.rect.right, top + i as f32 * ROW_H);
    }
}

fn wires(scene: &Scene, graph: &Graph, routing: &Routing) -> Vec<Wire> {
    let mut row_of: HashMap<u32, (RowKey, Point)> = HashMap::new();
    for card in &scene.cards {
        for row in card.rows() {
            for (port, _) in &row.ports {
                row_of.insert(*port, (row.key, row.anchor));
            }
        }
    }
    let mut wires: Vec<Wire> = Vec::new();
    for link in graph.links() {
        let (Some((from, a)), Some((to, b))) =
            (row_of.get(&link.output_port), row_of.get(&link.input_port))
        else {
            continue;
        };
        let saved = SavedLink::of(graph, link).is_some_and(|s| routing.contains(&s));
        match wires.iter_mut().find(|w| w.from == *from && w.to == *to) {
            Some(wire) => {
                wire.links.push(link.id);
                wire.saved &= saved;
            }
            None => wires.push(Wire {
                from: *from,
                to: *to,
                a: *a,
                b: *b,
                links: vec![link.id],
                media: from.media,
                saved,
            }),
        }
    }
    wires
}

// ---------------------------------------------------------------------------
// Matrix
// ---------------------------------------------------------------------------

/// One row or column of the matrix: one row of ports of a card, folded or
/// not as the patchbay shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct MatrixLine {
    pub key: RowKey,
    /// The node's name.
    pub title: String,
    /// What the node is, with the row's channels when the card has several
    /// rows on that side.
    pub kind: String,
}

/// What one cell of the matrix shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// The two cannot be linked: one node, or different media.
    Unavailable,
    /// Not linked; a press links it.
    Open,
    /// Linked by `links` links, all of them saved or not.
    Linked { saved: bool, links: usize },
}

/// The routing as a grid: what makes sound down the side, what takes it
/// across the top.
#[derive(Debug, Default)]
pub struct Matrix {
    pub rows: Vec<MatrixLine>,
    pub columns: Vec<MatrixLine>,
    /// Row by row.
    cells: Vec<Cell>,
}

impl Matrix {
    /// The cell where `row` meets `column`.
    #[must_use]
    pub fn cell(&self, row: usize, column: usize) -> Cell {
        self.cells
            .get(row * self.columns.len() + column)
            .copied()
            .unwrap_or(Cell::Unavailable)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() || self.columns.is_empty()
    }
}

/// The matrix of `scene`: every output row against every input row, sources
/// first, as the patchbay's columns run.
#[must_use]
pub fn matrix(scene: &Scene, graph: &Graph) -> Matrix {
    let lines = |direction: Direction| -> Vec<MatrixLine> {
        let mut lines: Vec<(usize, String, MatrixLine)> = Vec::new();
        for card in &scene.cards {
            let rows = match direction {
                Direction::Output => &card.outputs,
                Direction::Input => &card.inputs,
            };
            let col = graph.node(card.node).map_or(1, column);
            for row in rows {
                let kind = if rows.len() > 1 {
                    format!("{} · {}", card.subtitle, row.label)
                } else {
                    card.subtitle.clone()
                };
                lines.push((
                    col,
                    card.title.to_lowercase(),
                    MatrixLine {
                        key: row.key,
                        title: card.title.clone(),
                        kind,
                    },
                ));
            }
        }
        lines.sort_by(|a, b| (a.0, &a.1, a.2.key).cmp(&(b.0, &b.1, b.2.key)));
        lines.into_iter().map(|(_, _, line)| line).collect()
    };
    let rows = lines(Direction::Output);
    let columns = lines(Direction::Input);
    let mut cells = Vec::with_capacity(rows.len() * columns.len());
    for row in &rows {
        for col in &columns {
            let cell = if !compatible(row.key, col.key) {
                Cell::Unavailable
            } else {
                match scene.wire(row.key, col.key) {
                    Some(wire) => Cell::Linked {
                        saved: wire.saved,
                        links: wire.links.len(),
                    },
                    None => Cell::Open,
                }
            };
            cells.push(cell);
        }
    }
    Matrix {
        rows,
        columns,
        cells,
    }
}

// ---------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------

/// One path the sound takes, from where it starts to where it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    /// The nodes along it, source first.
    pub steps: Vec<u32>,
    /// Whether each hop, from `steps[i]` to `steps[i + 1]`, is saved: every
    /// link between the two.
    pub saved: Vec<bool>,
    pub media: MediaType,
}

impl Lane {
    /// Whether every hop along it is saved.
    #[must_use]
    pub fn is_saved(&self) -> bool {
        self.saved.iter().all(|s| *s)
    }
}

/// Every path through the links of `scene`, one media at a time: from each
/// node nothing links into, through whatever it is linked to, to each node
/// that links nowhere further. A loop ends a path where it would come back.
/// Links only on a loop, with no start outside it, get paths of their own.
#[must_use]
pub fn lanes(scene: &Scene, graph: &Graph) -> Vec<Lane> {
    // Every hop between two nodes, per media, saved when all its wires are.
    let mut hops: BTreeMap<(MediaType, u32, u32), bool> = BTreeMap::new();
    for wire in &scene.wires {
        *hops
            .entry((wire.media, wire.from.node, wire.to.node))
            .or_insert(true) &= wire.saved;
    }
    let mut lanes: Vec<Lane> = Vec::new();
    let mut covered: HashSet<(MediaType, u32, u32)> = HashSet::new();
    let medias: Vec<MediaType> = {
        let mut m: Vec<MediaType> = hops.keys().map(|(m, _, _)| *m).collect();
        m.dedup();
        m
    };
    for media in medias {
        let mut next: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut fed: HashSet<u32> = HashSet::new();
        for (_, from, to) in hops.keys().filter(|(m, _, _)| *m == media) {
            next.entry(*from).or_default().push(*to);
            fed.insert(*to);
        }
        let starts: Vec<u32> = next.keys().copied().filter(|n| !fed.contains(n)).collect();
        // Hops on a loop with no way in: start where the first of them does.
        let walk = |start: u32, lanes: &mut Vec<Lane>, covered: &mut HashSet<_>| {
            let mut stack = vec![vec![start]];
            while let Some(path) = stack.pop() {
                if lanes.len() >= MAX_LANES {
                    return;
                }
                let last = *path.last().unwrap_or(&start);
                let onward: Vec<u32> = next
                    .get(&last)
                    .map(|n| n.iter().copied().filter(|n| !path.contains(n)).collect())
                    .unwrap_or_default();
                if onward.is_empty() {
                    if path.len() > 1 {
                        let saved = path
                            .windows(2)
                            .map(|w| {
                                covered.insert((media, w[0], w[1]));
                                hops.get(&(media, w[0], w[1])).copied().unwrap_or(false)
                            })
                            .collect();
                        lanes.push(Lane {
                            steps: path,
                            saved,
                            media,
                        });
                    }
                    continue;
                }
                // Reversed, so the first way on is walked first.
                for n in onward.into_iter().rev() {
                    let mut longer = path.clone();
                    longer.push(n);
                    stack.push(longer);
                }
            }
        };
        for start in starts {
            walk(start, &mut lanes, &mut covered);
        }
        let loose: Vec<u32> = hops
            .keys()
            .filter(|(m, _, _)| *m == media)
            .map(|(_, from, _)| *from)
            .collect();
        for from in loose {
            let uncovered = next
                .get(&from)
                .is_some_and(|n| n.iter().any(|to| !covered.contains(&(media, from, *to))));
            if uncovered {
                walk(from, &mut lanes, &mut covered);
            }
        }
    }
    let name = |node: u32| scene.card(node).map(|c| c.title.to_lowercase());
    let col = |node: u32| graph.node(node).map_or(1, column);
    lanes.sort_by_cached_key(|lane| {
        let last = *lane.steps.last().unwrap_or(&0);
        let names: Vec<Option<String>> = lane.steps.iter().map(|n| name(*n)).collect();
        (
            lane.media,
            col(last),
            name(last),
            last,
            names,
            lane.steps.clone(),
        )
    });
    lanes
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The colour of `media`'s wires and connectors.
pub fn media_color(theme: &Theme, media: MediaType) -> Color {
    match media {
        MediaType::Audio => theme.accent_blue,
        MediaType::Midi => theme.accent_orange,
        MediaType::Video => theme.accent_purple,
        MediaType::Other => theme.accent_gray,
    }
}

/// The canvas's ground, under the dot grid.
pub fn canvas_ground(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x1c, 0x1c, 0x1e)
    } else {
        Color::from_rgb(0xf7, 0xf7, 0xf9)
    }
}

fn grid_dot(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x3a, 0x3a, 0x3e)
    } else {
        Color::from_rgb(0xd9, 0xd9, 0xe0)
    }
}

/// A card's ground.
pub fn card_ground(theme: &Theme) -> Color {
    if theme.is_dark() {
        Color::from_rgb(0x2c, 0x2c, 0x2e)
    } else {
        Color::WHITE
    }
}

/// What a link drag in progress shows.
#[derive(Debug, Clone, Copy)]
pub struct LinkDrag {
    pub from: RowKey,
    /// The pointer, in world points.
    pub to: Point,
}

/// Draw the scene into `viewport`, the world mapped by the view.
pub fn draw(
    canvas: &Canvas,
    scene: &Scene,
    view: &GraphView,
    viewport: Rect,
    link_drag: Option<LinkDrag>,
    theme: &Theme,
) {
    canvas.save();
    canvas.clip_rect(viewport, None, Some(true));
    canvas.draw_rect(viewport, &fill(canvas_ground(theme)));
    draw_grid(canvas, view, viewport, theme);
    canvas.translate((viewport.left + view.pan.x, viewport.top + view.pan.y));
    canvas.scale((view.zoom, view.zoom));

    draw_column_headings(canvas, scene, theme);
    for wire in &scene.wires {
        let selected = view.selected_wire == Some((wire.from, wire.to));
        draw_wire(canvas, wire, selected, theme);
    }
    if let Some(drag) = link_drag {
        if let Some(row) = scene.row(drag.from) {
            let (a, b) = match drag.from.direction {
                Direction::Output => (row.anchor, drag.to),
                Direction::Input => (drag.to, row.anchor),
            };
            let mut paint = stroke(media_color(theme, drag.from.media), 2.0);
            paint.set_path_effect(PathEffect::dash(&[6.0, 4.0], 0.0));
            canvas.draw_path(&curve(a, b), &paint);
        }
    }
    let linked: HashSet<RowKey> = scene.wires.iter().flat_map(|w| [w.from, w.to]).collect();
    for card in &scene.cards {
        draw_card(
            canvas,
            card,
            view.selected_node == Some(card.node),
            &linked,
            link_drag,
            theme,
        );
    }
    canvas.restore();
}

/// A dot every grid step, in window space so the dots stay round and crisp
/// at any zoom; left out when zoomed so far out they would read as a tint.
fn draw_grid(canvas: &Canvas, view: &GraphView, viewport: Rect, theme: &Theme) {
    let step = GRID * view.zoom;
    if step < 8.0 {
        return;
    }
    let origin_x = viewport.left + view.pan.x.rem_euclid(step);
    let origin_y = viewport.top + view.pan.y.rem_euclid(step);
    let paint = fill(grid_dot(theme));
    let mut y = origin_y;
    while y < viewport.bottom {
        let mut x = origin_x;
        while x < viewport.right {
            canvas.draw_rect(Rect::from_xywh(x - 0.5, y - 0.5, 1.2, 1.2), &paint);
            x += step;
        }
        y += step;
    }
}

/// The default columns' headings, over where their cards start.
fn draw_column_headings(canvas: &Canvas, scene: &Scene, theme: &Theme) {
    let font = styles::CAPTION_2_EMPHASIZED.font();
    for column in &scene.columns {
        let x = *column as f32 * (CARD_W + COLUMN_GAP);
        let text = column_heading(*column).to_uppercase();
        Label::new(ellipsize(&font, &text, CARD_W))
            .with_style(styles::CAPTION_2_EMPHASIZED)
            .with_color(theme.text_tertiary)
            .centered_on(x + 2.0, -COLUMN_HEADING)
            .render(canvas);
    }
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint.set_color(color);
    paint
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn curve(a: Point, b: Point) -> otto_kit::skia::Path {
    let (c1, c2) = controls(a, b);
    let mut path = PathBuilder::new();
    path.move_to(a);
    path.cubic_to(c1, c2, b);
    path.detach()
}

fn draw_wire(canvas: &Canvas, wire: &Wire, selected: bool, theme: &Theme) {
    let color = media_color(theme, wire.media);
    let path = curve(wire.a, wire.b);
    if selected {
        let halo = Color::from_argb(0x55, color.r(), color.g(), color.b());
        canvas.draw_path(&path, &stroke(halo, 9.0));
    }
    let mut paint = stroke(color, if selected { 3.0 } else { 2.0 });
    if !wire.saved {
        paint.set_path_effect(PathEffect::dash(&[7.0, 5.0], 0.0));
    }
    canvas.draw_path(&path, &paint);

    // Folded wires carry several links; say how many when it isn't the usual
    // two of a stereo pair.
    let count = wire.links.len();
    if wire.folded() && count != 2 {
        let mid = wire.samples().nth(12).unwrap_or(wire.a);
        let text = count.to_string();
        let font = styles::CAPTION_2_EMPHASIZED.font();
        let w = font.measure_str(&text, None).0 + 10.0;
        let pill = Rect::from_xywh(mid.x - w / 2.0, mid.y - 8.0, w, 16.0);
        canvas.draw_rrect(RRect::new_rect_xy(pill, 8.0, 8.0), &fill(color));
        Label::new(text)
            .with_style(styles::CAPTION_2_EMPHASIZED)
            .with_color(Color::WHITE)
            .centered_at(mid.x, mid.y)
            .render(canvas);
    }
}

fn draw_card(
    canvas: &Canvas,
    card: &Card,
    selected: bool,
    linked: &HashSet<RowKey>,
    link_drag: Option<LinkDrag>,
    theme: &Theme,
) {
    let rrect = RRect::new_rect_xy(card.rect, CARD_RADIUS, CARD_RADIUS);
    if selected {
        let a = theme.accent;
        let halo = RRect::new_rect_xy(
            card.rect.with_outset((4.0, 4.0)),
            CARD_RADIUS + 4.0,
            CARD_RADIUS + 4.0,
        );
        canvas.draw_rrect(halo, &fill(Color::from_argb(0x30, a.r(), a.g(), a.b())));
    }
    let mut shadow = fill(Color::from_argb(
        (theme.shadow.a() as f32 * 0.5) as u8,
        theme.shadow.r(),
        theme.shadow.g(),
        theme.shadow.b(),
    ));
    shadow.set_mask_filter(otto_kit::skia::MaskFilter::blur(
        otto_kit::skia::BlurStyle::Normal,
        4.0,
        false,
    ));
    canvas.draw_rrect(rrect.with_offset((0.0, 2.0)), &shadow);
    canvas.draw_rrect(rrect, &fill(card_ground(theme)));
    let edge = if selected {
        stroke(theme.accent, 2.0)
    } else {
        stroke(theme.hairline, 1.0)
    };
    canvas.draw_rrect(rrect, &edge);

    // Header: a state dot, the name, what it is, and the fold chevron.
    let left = card.rect.left + 12.0;
    let dot = match card.state {
        NodeState::Running => theme.accent_green,
        NodeState::Error => theme.accent_red,
        _ => theme.fill_secondary,
    };
    canvas.draw_circle(
        Point::new(left + 3.0, card.rect.top + 16.0),
        3.5,
        &fill(dot),
    );
    let text_left = left + 12.0;
    let text_right = card.rect.right - CHEVRON - 10.0;
    let title_font = styles::SUBHEADLINE_EMPHASIZED.font();
    Label::new(ellipsize(&title_font, &card.title, text_right - text_left))
        .with_style(styles::SUBHEADLINE_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(text_left, card.rect.top + 16.0)
        .render(canvas);
    let sub_font = styles::CAPTION_1.font();
    Label::new(ellipsize(&sub_font, &card.subtitle, text_right - left))
        .with_style(styles::CAPTION_1)
        .with_color(theme.text_secondary)
        .centered_on(left, card.rect.top + 32.0)
        .render(canvas);
    if card.foldable {
        draw_chevron(canvas, card.chevron_rect(), card.folded, theme);
    }
    let rule_y = card.rect.top + HEADER_H - 2.0;
    canvas.draw_line(
        Point::new(card.rect.left + 1.0, rule_y),
        Point::new(card.rect.right - 1.0, rule_y),
        &stroke(theme.hairline, 1.0),
    );

    let row_font = styles::CAPTION_1.font();
    let half = CARD_W / 2.0 - 16.0;
    for row in card.rows() {
        let dim = link_drag.is_some_and(|d| d.from != row.key && !compatible(d.from, row.key));
        let color = media_color(theme, row.key.media);
        let (dot, text) = if dim {
            (theme.fill_secondary, theme.text_tertiary)
        } else {
            (color, theme.text_primary)
        };
        // A ring, filled in once something is linked to it.
        let inside = if linked.contains(&row.key) && !dim {
            dot
        } else {
            card_ground(theme)
        };
        canvas.draw_circle(row.anchor, CONNECTOR_R, &fill(inside));
        canvas.draw_circle(row.anchor, CONNECTOR_R - 1.0, &stroke(dot, 2.2));
        let label = ellipsize(&row_font, &row.label, half);
        let label_w = row_font.measure_str(&label, None).0;
        let x = match row.key.direction {
            Direction::Input => row.anchor.x + 12.0,
            Direction::Output => row.anchor.x - 12.0 - label_w,
        };
        Label::new(label)
            .with_style(styles::CAPTION_1)
            .with_color(text)
            .centered_on(x, row.anchor.y)
            .render(canvas);
    }
}

fn draw_chevron(canvas: &Canvas, rect: Rect, folded: bool, theme: &Theme) {
    let c = Point::new(rect.center_x(), rect.center_y());
    let mut path = PathBuilder::new();
    if folded {
        path.move_to((c.x - 4.0, c.y - 2.0));
        path.line_to((c.x, c.y + 2.5));
        path.line_to((c.x + 4.0, c.y - 2.0));
    } else {
        path.move_to((c.x - 4.0, c.y + 2.0));
        path.line_to((c.x, c.y - 2.5));
        path.line_to((c.x + 4.0, c.y + 2.0));
    }
    let mut paint = stroke(theme.text_secondary, 1.6);
    paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
    paint.set_stroke_join(otto_kit::skia::paint::Join::Round);
    canvas.draw_path(&path.detach(), &paint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_audio_graph::{Event, Link, Port};

    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        let node = |id: u32, class: &str, name: &str, device: bool| {
            let mut props: BTreeMap<String, String> = [
                ("media.class".to_string(), class.to_string()),
                ("node.name".to_string(), name.to_string()),
            ]
            .into();
            if device {
                props.insert("device.id".into(), "1".into());
            }
            Node {
                id,
                props,
                state: NodeState::Idle,
            }
        };
        graph.apply(Event::Node(node(1, "Stream/Output/Audio", "player", false)));
        graph.apply(Event::Node(node(2, "Audio/Sink", "speakers", true)));
        let port = |id: u32, node: u32, direction: Direction, channel: &str| Port {
            id,
            node,
            direction: Some(direction),
            name: format!("port_{channel}"),
            channel: Some(channel.into()),
            format: Some(MediaType::Audio),
            ..Port::default()
        };
        graph.apply(Event::Port(port(11, 1, Direction::Output, "FL")));
        graph.apply(Event::Port(port(12, 1, Direction::Output, "FR")));
        graph.apply(Event::Port(port(21, 2, Direction::Input, "FL")));
        graph.apply(Event::Port(port(22, 2, Direction::Input, "FR")));
        for (id, o, i) in [(31, 11, 21), (32, 12, 22)] {
            graph.apply(Event::Link(Link {
                id,
                output_node: 1,
                output_port: o,
                input_node: 2,
                input_port: i,
                client: None,
            }));
        }
        graph
    }

    #[test]
    fn players_and_speakers_take_their_columns() {
        let graph = graph();
        assert_eq!(column(graph.node(1).unwrap()), 0);
        assert_eq!(column(graph.node(2).unwrap()), 2);
    }

    #[test]
    fn an_airplay_speaker_is_a_destination() {
        let speaker = Node {
            id: 5,
            props: [
                ("media.class".to_string(), "Audio/Sink".to_string()),
                (
                    "node.name".to_string(),
                    "raop_sink.Kitchen.local.192.168.1.30.7000".to_string(),
                ),
            ]
            .into(),
            state: NodeState::Idle,
        };
        assert_eq!(column(&speaker), 2);
    }

    #[test]
    fn folded_stereo_links_draw_as_one_wire() {
        let graph = graph();
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        assert_eq!(scene.cards.len(), 2);
        assert_eq!(scene.wires.len(), 1);
        assert_eq!(scene.wires[0].links, vec![31, 32]);
        assert!(!scene.wires[0].saved);
    }

    #[test]
    fn unfolding_splits_the_wire_per_channel() {
        let graph = graph();
        let mut view = GraphView::new(Layout::default());
        view.toggle_fold(1);
        view.toggle_fold(2);
        let scene = view.scene(&graph, &Routing::default());
        assert_eq!(scene.wires.len(), 2);
    }

    #[test]
    fn a_card_keeps_its_place_when_another_appears() {
        let mut graph = graph();
        let mut view = GraphView::new(Layout::default());
        let before = view
            .scene(&graph, &Routing::default())
            .card(2)
            .unwrap()
            .rect;
        let mut props: BTreeMap<String, String> = [
            ("media.class".to_string(), "Audio/Sink".to_string()),
            ("node.name".to_string(), "headset".to_string()),
            ("device.id".to_string(), "2".to_string()),
        ]
        .into();
        props.insert("node.description".into(), "Headset".into());
        graph.apply(Event::Node(Node {
            id: 0,
            props,
            state: NodeState::Idle,
        }));
        graph.apply(Event::Port(Port {
            id: 5,
            node: 0,
            direction: Some(Direction::Input),
            name: "playback_MONO".into(),
            format: Some(MediaType::Audio),
            ..Port::default()
        }));
        let scene = view.scene(&graph, &Routing::default());
        assert_eq!(scene.card(2).unwrap().rect, before);
        assert!(scene.card(0).unwrap().rect.top >= before.bottom);
    }

    #[test]
    fn dragging_between_folded_rows_pairs_unlinked_channels() {
        let mut graph = graph();
        graph.apply(Event::Removed(32));
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        let from = scene.card(1).unwrap().outputs[0].key;
        let to = scene.card(2).unwrap().inputs[0].key;
        assert_eq!(view.pairs_to_link(&scene, &graph, from, to), vec![(12, 22)]);
        assert_eq!(view.pairs_to_link(&scene, &graph, to, from), vec![(12, 22)]);
        assert!(view.pairs_to_link(&scene, &graph, from, from).is_empty());
    }

    #[test]
    fn the_video_filter_hides_video_only_nodes() {
        let mut graph = graph();
        graph.apply(Event::Node(Node {
            id: 3,
            props: [
                ("media.class".to_string(), "Video/Source".to_string()),
                ("node.name".to_string(), "camera".to_string()),
            ]
            .into(),
            state: NodeState::Idle,
        }));
        graph.apply(Event::Port(Port {
            id: 40,
            node: 3,
            direction: Some(Direction::Output),
            name: "capture_1".into(),
            ..Port::default()
        }));
        let mut view = GraphView::new(Layout::default());
        assert!(view.scene(&graph, &Routing::default()).card(3).is_none());
        view.filter.toggle(MediaType::Video);
        assert!(view.scene(&graph, &Routing::default()).card(3).is_some());
    }

    #[test]
    fn a_press_on_a_wire_picks_it() {
        let graph = graph();
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        let wire = &scene.wires[0];
        let mid = wire.samples().nth(12).unwrap();
        assert_eq!(scene.hit(mid, 1.0), Hit::Wire(0));
        assert_eq!(
            scene.hit(Point::new(mid.x, mid.y + 40.0), 1.0),
            Hit::Nothing
        );
    }

    fn link(graph: &mut Graph, id: u32, output: (u32, u32), input: (u32, u32)) {
        graph.apply(Event::Link(Link {
            id,
            output_node: output.0,
            output_port: output.1,
            input_node: input.0,
            input_port: input.1,
            client: None,
        }));
    }

    /// `graph()` with an effect beside the two: a virtual sink with stereo
    /// ports on both sides, linked to nothing yet.
    fn graph_with_effect() -> Graph {
        let mut graph = graph();
        graph.apply(Event::Node(Node {
            id: 3,
            props: [
                ("media.class".to_string(), "Audio/Sink".to_string()),
                ("node.name".to_string(), "equaliser".to_string()),
            ]
            .into(),
            state: NodeState::Idle,
        }));
        for (id, direction, channel) in [
            (41, Direction::Input, "FL"),
            (42, Direction::Input, "FR"),
            (43, Direction::Output, "FL"),
            (44, Direction::Output, "FR"),
        ] {
            graph.apply(Event::Port(Port {
                id,
                node: 3,
                direction: Some(direction),
                name: format!("port_{channel}"),
                channel: Some(channel.into()),
                format: Some(MediaType::Audio),
                ..Port::default()
            }));
        }
        graph
    }

    /// A routing that keeps the links `ids` of `graph`.
    fn saving(graph: &Graph, ids: &[u32]) -> Routing {
        let mut routing = Routing::default();
        for id in ids {
            routing.add(SavedLink::of(graph, graph.link(*id).unwrap()).unwrap());
        }
        routing
    }

    #[test]
    fn the_matrix_crosses_outputs_with_inputs() {
        let graph = graph();
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        let m = matrix(&scene, &graph);
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.columns.len(), 1);
        assert_eq!(m.rows[0].key.node, 1);
        assert_eq!(m.columns[0].key.node, 2);
        assert_eq!(
            m.cell(0, 0),
            Cell::Linked {
                saved: false,
                links: 2
            }
        );
        let saved = saving(&graph, &[31, 32]);
        let scene = view.scene(&graph, &saved);
        assert_eq!(
            matrix(&scene, &graph).cell(0, 0),
            Cell::Linked {
                saved: true,
                links: 2
            }
        );
    }

    #[test]
    fn a_matrix_cell_links_like_a_drag() {
        let graph = graph_with_effect();
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        let m = matrix(&scene, &graph);
        // Sources first, then the effect; the effect, then the speakers.
        let rows: Vec<u32> = m.rows.iter().map(|l| l.key.node).collect();
        let columns: Vec<u32> = m.columns.iter().map(|l| l.key.node).collect();
        assert_eq!(rows, vec![1, 3]);
        assert_eq!(columns, vec![3, 2]);
        assert_eq!(m.cell(1, 0), Cell::Unavailable);
        assert_eq!(m.cell(0, 0), Cell::Open);
        assert_eq!(m.cell(1, 1), Cell::Open);
        let (from, to) = (m.rows[0].key, m.columns[0].key);
        assert_eq!(
            view.pairs_to_link(&scene, &graph, from, to),
            vec![(11, 41), (12, 42)]
        );
    }

    #[test]
    fn the_matrix_follows_the_filter() {
        let graph = graph();
        let mut view = GraphView::new(Layout::default());
        view.filter.toggle(MediaType::Audio);
        let scene = view.scene(&graph, &Routing::default());
        assert!(matrix(&scene, &graph).is_empty());
    }

    #[test]
    fn a_lane_runs_from_source_through_effect_to_destination() {
        let mut graph = graph_with_effect();
        graph.apply(Event::Removed(31));
        graph.apply(Event::Removed(32));
        link(&mut graph, 51, (1, 11), (3, 41));
        link(&mut graph, 52, (1, 12), (3, 42));
        link(&mut graph, 53, (3, 43), (2, 21));
        link(&mut graph, 54, (3, 44), (2, 22));
        let mut view = GraphView::new(Layout::default());
        let routing = saving(&graph, &[51, 52]);
        let scene = view.scene(&graph, &routing);
        let lanes = lanes(&scene, &graph);
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].steps, vec![1, 3, 2]);
        assert_eq!(lanes[0].saved, vec![true, false]);
        assert!(!lanes[0].is_saved());
        assert_eq!(lanes[0].media, MediaType::Audio);
    }

    #[test]
    fn every_path_to_a_destination_is_a_lane() {
        let mut graph = graph_with_effect();
        link(&mut graph, 51, (1, 11), (3, 41));
        link(&mut graph, 53, (3, 43), (2, 21));
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        let mut steps: Vec<Vec<u32>> = lanes(&scene, &graph).into_iter().map(|l| l.steps).collect();
        steps.sort();
        assert_eq!(steps, vec![vec![1, 2], vec![1, 3, 2]]);
    }

    #[test]
    fn a_loop_ends_its_lane() {
        let mut graph = graph_with_effect();
        graph.apply(Event::Removed(31));
        graph.apply(Event::Removed(32));
        // The player and the effect feed each other, with no way in from
        // outside the loop.
        graph.apply(Event::Port(Port {
            id: 13,
            node: 1,
            direction: Some(Direction::Input),
            name: "port_in".into(),
            channel: Some("MONO".into()),
            format: Some(MediaType::Audio),
            ..Port::default()
        }));
        link(&mut graph, 51, (1, 11), (3, 41));
        link(&mut graph, 55, (3, 43), (1, 13));
        let mut view = GraphView::new(Layout::default());
        let scene = view.scene(&graph, &Routing::default());
        let mut steps: Vec<Vec<u32>> = lanes(&scene, &graph).into_iter().map(|l| l.steps).collect();
        steps.sort();
        assert_eq!(steps, vec![vec![1, 3], vec![3, 1]]);
    }

    #[test]
    fn a_graph_mode_keeps_its_name() {
        for mode in GraphMode::ALL {
            assert_eq!(GraphMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(GraphMode::from_key("waveform"), None);
    }
}
