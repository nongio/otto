//! The Effects section: the effects the audio service hosts, a list of them
//! by the device each sits on, a form to add one, and the selected one's
//! editor.
//!
//! An equaliser is edited on its response curve, computed from the very
//! biquads the service runs: drag a band's point for its frequency and gain,
//! scroll over it for its width (Q). Its cards scrub the same values. The
//! other kinds have a slider per parameter.
//!
//! Like Virtual devices, the section edits the routing file and the service
//! makes the effects. While a value is being dragged it is also set straight
//! on the effect's node, so it is heard at once, and the file is written
//! when the drag ends.

use std::sync::{Arc, Mutex};

use otto_audio_graph::{
    flat_bands, parse_autoeq, response_db, support, Band, BandType, Effect, EffectKind, EffectSide,
    Endpoint, ParamSpec, Support, Unit, EFFECT_PREFIX, MAX_BANDS, MAX_FREQ, MAX_GAIN, MAX_Q,
    MIN_FREQ, MIN_Q,
};
use otto_kit::components::slider::{self, SliderInteraction};
use otto_kit::components::toggle::{self, ToggleInteraction};
use otto_kit::portal_file_chooser::{Outcome, Request, Rule};

use super::virtual_pane::wrap;
use super::*;

/// The list of effects beside the editor.
const LIST_W: f32 = 232.0;
/// One effect in the list.
const ITEM_H: f32 = 50.0;
/// The editor's margin.
const PAD: f32 = 24.0;
/// The notice shown while the service is off, or a plugin is missing.
const NOTICE_H: f32 = 44.0;
/// One step of the chain line.
const STEP_H: f32 = 22.0;
/// The response card.
const RESPONSE_H: f32 = 260.0;
const RESPONSE_MIN_H: f32 = 180.0;
/// The band cards.
const BAND_CARD_MIN_W: f32 = 128.0;
const BAND_CARD_H: f32 = 100.0;
const BAND_GAP: f32 = 10.0;
/// One slider row of a limiter, compressor or noise suppressor.
const PARAM_ROW_H: f32 = 48.0;
/// The plot shows this many dB above and below 0.
const DB_RANGE: f32 = 18.0;
/// A band's point on the curve, and how far from it a press still takes it.
const POINT_R: f32 = 9.0;
const POINT_HIT: f32 = 14.0;
/// How much one point of scrolling widens or narrows a band (its Q).
const Q_PER_POINT: f32 = 0.006;
/// How far one point of horizontal scrubbing moves a value: frequency and Q
/// multiply by `exp(dx * SCRUB_RATIO)`, gains add `dx * SCRUB_DB`.
const SCRUB_RATIO: f32 = 0.01;
const SCRUB_DB: f32 = 0.05;
/// The largest file taken as an AutoEQ profile; a real one is under 1 KiB.
const MAX_IMPORT_BYTES: u64 = 256 * 1024;
/// The new effect form's rows.
const FORM_KIND_H: f32 = 46.0;
const FORM_ROW_H: f32 = 44.0;
const FORM_LABEL_W: f32 = 78.0;
/// The frequencies the plot marks.
const GRID_FREQS: [f32; 8] = [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0];

/// A value a band card or the response header scrubs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Freq,
    Gain,
    Q,
    /// The equaliser's preamp; the band index is ignored.
    Preamp,
}

/// What a control in this section does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum EffectAction {
    /// Show the effect at this index of the routing's list.
    Select(usize),
    New,
    CancelNew,
    FormKind(EffectKind),
    FormSide(EffectSide),
    FormDeviceMenu,
    /// The device at this index of the menu just opened.
    FormDevice(usize),
    Create,
    Toggle,
    Remove,
    PresetMenu,
    Flat,
    Import,
    /// A band's point on the curve: dragged.
    Point(usize),
    /// A band's card: selects it.
    Band(usize),
    BandTypeMenu(usize),
    BandType(usize, BandType),
    RemoveBand(usize),
    AddBand,
    /// A value scrubbed by dragging sideways.
    Scrub(usize, Field),
    /// The slider of the parameter at this index.
    Param(usize),
    /// The plot itself, under the points.
    Plot,
}

/// A drag in progress in this section.
#[derive(Debug, Clone, Copy)]
enum EffectDrag {
    Point(usize),
    Scrub {
        band: usize,
        field: Field,
        from_x: f32,
        start: f32,
    },
    Param {
        index: usize,
        track: Rect,
    },
}

/// What a step of the chain line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    App,
    This,
    Other,
}

/// What an import came back with, handed over from the dialog's thread.
type ImportAnswer = (String, Outcome);

/// The section's own state.
pub(super) struct EffectsState {
    /// The id of the effect shown.
    selected: Option<String>,
    /// Whether the new effect form is shown instead of an effect.
    creating: bool,
    form_kind: EffectKind,
    form_side: EffectSide,
    /// The `node.name` the new effect will sit on.
    form_device: Option<String>,
    /// The devices the open pop-up offers, by `node.name`.
    device_choices: Vec<String>,
    /// Whether each kind can be made here, read when the form opens.
    supported: Vec<(EffectKind, Support)>,
    /// The band selected on the curve and in the cards.
    band: Option<usize>,
    drag: Option<EffectDrag>,
    /// A change heard live but not yet written to the routing file.
    unsaved: bool,
    /// Where the plot was last drawn, which a point drag maps through.
    plot: Rect,
    /// The answer of an import dialog, once there is one.
    import: Arc<Mutex<Option<ImportAnswer>>>,
    /// What the last import said, shown under the bands.
    note: Option<String>,
}

impl Default for EffectsState {
    fn default() -> Self {
        Self {
            selected: None,
            creating: false,
            form_kind: EffectKind::Equaliser,
            form_side: EffectSide::Output,
            form_device: None,
            device_choices: Vec::new(),
            supported: supported(),
            band: None,
            drag: None,
            unsaved: false,
            plot: Rect::from_xywh(0.0, 0.0, 0.0, 0.0),
            import: Arc::new(Mutex::new(None)),
            note: None,
        }
    }
}

fn supported() -> Vec<(EffectKind, Support)> {
    EffectKind::ALL.iter().map(|k| (*k, support(*k))).collect()
}

impl Ui {
    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    pub(super) fn draw_effects(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        let list = Rect::from_ltrb(
            content.left,
            content.top,
            content.left + LIST_W,
            content.bottom,
        );
        let main = Rect::from_ltrb(list.right, content.top, content.right, content.bottom);
        canvas.draw_rect(main, &fill(graph_view::canvas_ground(theme)));
        self.draw_effect_list(canvas, list, theme);

        let left = main.left + PAD;
        let right = main.right - PAD;
        let mut top = main.top + PAD;
        if self.service_down() {
            self.draw_service_notice(
                canvas,
                Rect::from_ltrb(left, top, right, top + NOTICE_H),
                t!("audio-effects-service-down"),
                theme,
            );
            top += NOTICE_H + 16.0;
        }
        let area = Rect::from_ltrb(left, top, right, main.bottom - PAD);
        match self.shown_effect() {
            Some(effect) if !self.effects.creating => {
                self.draw_editor(canvas, area, &effect, theme)
            }
            _ => self.draw_new_form(canvas, area, theme),
        }
    }

    /// The effect the editor shows: the selected one, else the first.
    fn shown_effect(&self) -> Option<Effect> {
        let effects = self.routing.effects();
        self.effects
            .selected
            .as_deref()
            .and_then(|id| effects.iter().find(|e| e.id == id))
            .or_else(|| effects.first())
            .cloned()
    }

    fn draw_effect_list(&mut self, canvas: &Canvas, list: Rect, theme: &Theme) {
        canvas.draw_rect(list, &fill(panel_ground(theme)));
        canvas.draw_line(
            Point::new(list.right, list.top),
            Point::new(list.right, list.bottom),
            &stroke(theme.hairline, 1.0),
        );
        let shown = (!self.effects.creating)
            .then(|| self.shown_effect().map(|e| e.id))
            .flatten();
        let effects = self.routing.effects().to_vec();
        let left = list.left + 12.0;
        let width = list.width() - 24.0;
        let mut y = list.top + 16.0;
        for (side, heading) in [
            (EffectSide::Output, t!("audio-effects-outputs")),
            (EffectSide::Input, t!("audio-effects-inputs")),
        ] {
            card_heading(canvas, heading, left + 12.0, y + 8.0, theme);
            y += 22.0;
            let mut any = false;
            for (index, effect) in effects.iter().enumerate().filter(|(_, e)| e.side == side) {
                any = true;
                let item = Rect::from_xywh(left, y, width, ITEM_H);
                let selected = shown.as_deref() == Some(effect.id.as_str());
                if selected {
                    let rr = RRect::new_rect_xy(item, 10.0, 10.0);
                    let a = theme.accent;
                    canvas.draw_rrect(rr, &fill(Color::from_argb(0x22, a.r(), a.g(), a.b())));
                    canvas.draw_rrect(
                        rr,
                        &stroke(Color::from_argb(0x66, a.r(), a.g(), a.b()), 1.0),
                    );
                }
                let font = styles::SUBHEADLINE_EMPHASIZED.font();
                Label::new(ellipsize(&font, &effect.name, width - 24.0))
                    .with_style(styles::SUBHEADLINE_EMPHASIZED)
                    .with_color(theme.text_primary)
                    .centered_on(item.left + 12.0, item.top + 17.0)
                    .render(canvas);
                let font = styles::FOOTNOTE.font();
                Label::new(ellipsize(&font, &self.effect_caption(effect), width - 24.0))
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_secondary)
                    .centered_on(item.left + 12.0, item.top + 35.0)
                    .render(canvas);
                self.targets
                    .push((item, Action::Effect(EffectAction::Select(index))));
                y += ITEM_H + 4.0;
            }
            if !any {
                Label::new(t!("audio-effects-none"))
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_tertiary)
                    .centered_on(left + 12.0, y + 10.0)
                    .render(canvas);
                y += 24.0;
            }
            y += 14.0;
        }

        let label = t!("audio-effects-new");
        let button = Rect::from_xywh(
            left,
            list.bottom - 16.0 - push_button::HEIGHT,
            width,
            push_button::HEIGHT,
        );
        self.button(
            canvas,
            button,
            label,
            Action::Effect(EffectAction::New),
            !self.effects.creating,
            theme,
        );
    }

    /// An effect's kind and where it sits, as the list shows it.
    fn effect_caption(&self, effect: &Effect) -> String {
        let place = match self.effect_named(&effect.device) {
            Some(under) => t_owned!("audio-effects-after", name = under.name.clone()),
            None => self.node_title(&effect.device),
        };
        let caption = t_owned!(
            "audio-effects-caption",
            kind = kind_label(effect.kind),
            place = place
        );
        if effect.enabled {
            caption
        } else {
            t_owned!("audio-effects-caption-off", caption = caption)
        }
    }

    /// The effect whose node is called `name`.
    fn effect_named(&self, name: &str) -> Option<&Effect> {
        name.strip_prefix(EFFECT_PREFIX)
            .and_then(|id| self.routing.effect(id))
    }

    /// What a node called `name` is shown as, whether it exists now or not.
    fn node_title(&self, name: &str) -> String {
        if let Some(effect) = self.effect_named(name) {
            return effect.name.clone();
        }
        self.graph
            .nodes()
            .find(|n| n.name() == name)
            .map_or_else(|| name.to_string(), |n| n.title().to_string())
    }

    fn draw_editor(&mut self, canvas: &Canvas, area: Rect, effect: &Effect, theme: &Theme) {
        // The title, and the controls on its right, from the right edge in.
        let cy = area.top + 14.0;
        let mut right = area.right;
        let remove = t!("audio-effects-remove");
        let w = push_button::width(remove);
        right -= w;
        self.button(
            canvas,
            Rect::from_xywh(
                right,
                cy - push_button::HEIGHT / 2.0,
                w,
                push_button::HEIGHT,
            ),
            remove,
            Action::Effect(EffectAction::Remove),
            true,
            theme,
        );
        right -= 20.0 + toggle::WIDTH;
        let switch = Rect::from_xywh(
            right,
            cy - toggle::HEIGHT / 2.0,
            toggle::WIDTH,
            toggle::HEIGHT,
        );
        toggle::draw(
            canvas,
            switch,
            toggle::knob_fraction_for(effect.enabled),
            ToggleInteraction::Normal,
            theme,
        );
        let on = t!("audio-effects-on");
        let on_w = styles::SUBHEADLINE.font().measure_str(on, None).0;
        Label::new(on)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(right - 8.0 - on_w, cy)
            .render(canvas);
        self.targets.push((
            Rect::from_ltrb(right - 8.0 - on_w, switch.top, switch.right, switch.bottom),
            Action::Effect(EffectAction::Toggle),
        ));
        right -= 8.0 + on_w + 20.0;
        if effect.kind == EffectKind::Equaliser {
            let import = t!("audio-effects-import");
            let w = push_button::width(import);
            right -= w;
            self.button(
                canvas,
                Rect::from_xywh(
                    right,
                    cy - push_button::HEIGHT / 2.0,
                    w,
                    push_button::HEIGHT,
                ),
                import,
                Action::Effect(EffectAction::Import),
                true,
                theme,
            );
            let preset = effect
                .preset
                .clone()
                .unwrap_or_else(|| t!("audio-effects-preset-custom").to_string());
            let font = styles::BODY.font();
            let preset = ellipsize(&font, &preset, 160.0);
            let w = push_button::width(&format!("{preset}  ▾"));
            right -= 8.0 + w;
            self.button(
                canvas,
                Rect::from_xywh(
                    right,
                    cy - push_button::HEIGHT / 2.0,
                    w,
                    push_button::HEIGHT,
                ),
                &format!("{preset}  ▾"),
                Action::Effect(EffectAction::PresetMenu),
                true,
                theme,
            );
            right -= 12.0;
        }
        let font = styles::TITLE_2_EMPHASIZED.font();
        Label::new(ellipsize(
            &font,
            &effect.name,
            (right - area.left - 12.0).max(40.0),
        ))
        .with_style(styles::TITLE_2_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(area.left, cy)
        .render(canvas);

        let mut y = area.top + 40.0;
        self.draw_chain(
            canvas,
            Point::new(area.left, y + STEP_H / 2.0),
            area.right,
            effect,
            theme,
        );
        y += STEP_H + 18.0;

        if let Support::Missing { package, .. } = support(effect.kind) {
            let band = Rect::from_ltrb(area.left, y, area.right, y + NOTICE_H);
            draw_missing(
                canvas,
                band,
                &t_owned!("audio-effects-missing-made", package = package),
                theme,
            );
            y += NOTICE_H + 16.0;
        }

        let rest = Rect::from_ltrb(area.left, y, area.right, area.bottom);
        if effect.kind == EffectKind::Equaliser {
            self.draw_equaliser(canvas, rest, effect, theme);
        } else {
            self.draw_params(canvas, rest, effect, theme);
        }
    }

    /// The chain the effect is part of, as the sound flows: the apps on it,
    /// the effect, then the effects and the device it plays on; reversed for
    /// an input.
    fn draw_chain(
        &mut self,
        canvas: &Canvas,
        start: Point,
        right: f32,
        effect: &Effect,
        theme: &Theme,
    ) {
        let mut steps: Vec<(String, Step)> = Vec::new();
        let apps = self.apps_on(effect);
        if apps.is_empty() {
            steps.push((t!("audio-effects-chain-no-apps").to_string(), Step::App));
        } else {
            steps.extend(apps.into_iter().map(|a| (a, Step::App)));
        }
        steps.push((effect.name.clone(), Step::This));
        let mut device = effect.device.clone();
        // Bounded, so a hand-edited file that loops cannot hang the window.
        for _ in 0..8 {
            match self.effect_named(&device) {
                Some(under) => {
                    steps.push((under.name.clone(), Step::Other));
                    device.clone_from(&under.device);
                }
                None => {
                    steps.push((self.node_title(&device), Step::Other));
                    break;
                }
            }
        }
        if effect.side == EffectSide::Input {
            steps.reverse();
        }

        let font = styles::CAPTION_1.font();
        let mut x = start.x;
        let mut previous: Option<Step> = None;
        for (text, step) in &steps {
            match previous {
                // Apps side by side, not one after the other.
                Some(Step::App) if *step == Step::App => x += 6.0,
                Some(_) => {
                    if x + 26.0 > right {
                        break;
                    }
                    draw_arrow(canvas, Point::new(x + 4.0, start.y), x + 22.0, theme);
                    x += 26.0;
                }
                None => {}
            }
            previous = Some(*step);
            let own = *step == Step::This;
            let text = ellipsize(&font, text, 180.0);
            let w = font.measure_str(&text, None).0 + 16.0;
            if x + w > right {
                break;
            }
            let chip = Rect::from_xywh(x, start.y - STEP_H / 2.0, w, STEP_H);
            let rr = RRect::new_rect_xy(chip, 6.0, 6.0);
            let (ground, ink) = if own {
                (theme.accent, Color::WHITE)
            } else {
                (theme.fill_quaternary, theme.text_primary)
            };
            canvas.draw_rrect(rr, &fill(ground));
            Label::new(text)
                .with_style(if own {
                    styles::CAPTION_1_EMPHASIZED
                } else {
                    styles::CAPTION_1
                })
                .with_color(ink)
                .centered_at(chip.center_x(), chip.center_y())
                .render(canvas);
            x += w;
        }
    }

    /// The apps whose streams go through `effect` now, at most three.
    fn apps_on(&self, effect: &Effect) -> Vec<String> {
        let name = effect.node_name();
        let Some(node) = self.graph.nodes().find(|n| n.name() == name) else {
            return Vec::new();
        };
        let mut apps: Vec<String> = Vec::new();
        for link in self.graph.links() {
            let other = match effect.side {
                EffectSide::Output if link.input_node == node.id => link.output_node,
                EffectSide::Input if link.output_node == node.id => link.input_node,
                _ => continue,
            };
            let Some(stream) = self.graph.node(other) else {
                continue;
            };
            if !stream.is_stream() || stream.prop("node.link-group").is_some() {
                continue;
            }
            let app = stream.app().unwrap_or_else(|| stream.title()).to_string();
            if !apps.contains(&app) {
                apps.push(app);
            }
        }
        apps.truncate(3);
        apps
    }

    fn draw_equaliser(&mut self, canvas: &Canvas, area: Rect, effect: &Effect, theme: &Theme) {
        let cells = effect.bands.len() + 1;
        let columns = ((area.width() + BAND_GAP) / (BAND_CARD_MIN_W + BAND_GAP))
            .floor()
            .max(1.0) as usize;
        let card_rows = cells.div_ceil(columns).max(1) as f32;
        let grid_h = card_rows * (BAND_CARD_H + BAND_GAP);
        let response_h = (area.height() - grid_h - 40.0).clamp(RESPONSE_MIN_H, RESPONSE_H);
        let card = Rect::from_xywh(area.left, area.top, area.width(), response_h);
        self.draw_response(canvas, card, effect, theme);

        let top = card.bottom + 16.0;
        let card_w = (area.width() - (columns - 1) as f32 * BAND_GAP) / columns as f32;
        let cell = |index: usize| {
            Rect::from_xywh(
                area.left + (index % columns) as f32 * (card_w + BAND_GAP),
                top + (index / columns) as f32 * (BAND_CARD_H + BAND_GAP),
                card_w,
                BAND_CARD_H,
            )
        };
        for (index, band) in effect.bands.iter().enumerate() {
            self.draw_band_card(canvas, cell(index), index, band, theme);
        }
        let mut bottom = top;
        if effect.bands.len() < MAX_BANDS {
            let add = cell(effect.bands.len());
            let rr = RRect::new_rect_xy(add, 10.0, 10.0);
            canvas.draw_rrect(rr, &stroke(theme.fill_secondary, 1.5));
            Label::new(t!("audio-effects-add-band"))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_at(add.center_x(), add.center_y())
                .render(canvas);
            self.targets
                .push((add, Action::Effect(EffectAction::AddBand)));
            bottom = add.bottom;
        } else if let Some(last) = effect.bands.len().checked_sub(1) {
            bottom = cell(last).bottom;
        }
        let note = self
            .effects
            .note
            .clone()
            .unwrap_or_else(|| t!("audio-effects-hint").to_string());
        let font = styles::FOOTNOTE.font();
        for (i, line) in wrap(&font, &note, area.width(), 2).iter().enumerate() {
            Label::new(line)
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_secondary)
                .centered_on(area.left, bottom + 18.0 + i as f32 * 16.0)
                .render(canvas);
        }
    }

    fn draw_response(&mut self, canvas: &Canvas, card: Rect, effect: &Effect, theme: &Theme) {
        let rr = RRect::new_rect_xy(card, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        card_heading(
            canvas,
            t!("audio-effects-response"),
            card.left + 16.0,
            card.top + 18.0,
            theme,
        );

        // The preamp, scrubbed like a band's values.
        let value = db_text(effect.preamp);
        let value_font = styles::FOOTNOTE_EMPHASIZED.font();
        let value_w = value_font.measure_str(&value, None).0;
        let label = t!("audio-effects-preamp");
        let label_w = styles::FOOTNOTE.font().measure_str(label, None).0;
        let vx = card.right - 16.0 - value_w;
        Label::new(value)
            .with_style(styles::FOOTNOTE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(vx, card.top + 18.0)
            .render(canvas);
        Label::new(label)
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_secondary)
            .centered_on(vx - 6.0 - label_w, card.top + 18.0)
            .render(canvas);
        self.targets.push((
            Rect::from_ltrb(
                vx - 6.0 - label_w,
                card.top + 6.0,
                card.right - 10.0,
                card.top + 30.0,
            ),
            Action::Effect(EffectAction::Scrub(0, Field::Preamp)),
        ));

        let plot = Rect::from_ltrb(
            card.left + 40.0,
            card.top + 36.0,
            card.right - 16.0,
            card.bottom - 26.0,
        );
        self.effects.plot = plot;
        self.targets
            .push((plot, Action::Effect(EffectAction::Plot)));
        let grid = stroke(theme.hairline, 1.0);
        let caption = |canvas: &Canvas, text: &str, x: f32, y: f32, centred: bool| {
            let label = Label::new(text)
                .with_style(styles::CAPTION_2)
                .with_color(theme.text_tertiary);
            let label = if centred {
                label.centered_at(x, y)
            } else {
                label.centered_on(x, y)
            };
            label.render(canvas);
        };
        for freq in GRID_FREQS {
            let x = x_of(plot, freq);
            canvas.draw_line(Point::new(x, plot.top), Point::new(x, plot.bottom), &grid);
            caption(canvas, &freq_short(freq), x, plot.bottom + 12.0, true);
        }
        for db in [-12.0, -6.0, 6.0, 12.0] {
            let y = y_of(plot, db);
            canvas.draw_line(Point::new(plot.left, y), Point::new(plot.right, y), &grid);
        }
        let zero = y_of(plot, 0.0);
        canvas.draw_line(
            Point::new(plot.left, zero),
            Point::new(plot.right, zero),
            &stroke(theme.fill_secondary, 1.0),
        );
        for (db, text) in [(12.0, "+12"), (0.0, "0"), (-12.0, "\u{2212}12")] {
            caption(canvas, text, card.left + 12.0, y_of(plot, db), false);
        }

        // The curve, and the area between it and 0 dB.
        canvas.save();
        canvas.clip_rect(plot, None, Some(true));
        let steps = (plot.width() / 2.0).max(8.0) as usize;
        let mut line = PathBuilder::new();
        let mut area = PathBuilder::new();
        area.move_to((plot.left, zero));
        for i in 0..=steps {
            let x = plot.left + plot.width() * i as f32 / steps as f32;
            let db = response_db(effect.preamp, &effect.bands, freq_at(plot, x));
            let y = y_of(plot, db);
            if i == 0 {
                line.move_to((x, y));
            } else {
                line.line_to((x, y));
            }
            area.line_to((x, y));
        }
        area.line_to((plot.right, zero));
        area.close();
        let a = theme.accent;
        let tint = if effect.enabled {
            a
        } else {
            theme.fill_secondary
        };
        canvas.draw_path(
            &area.detach(),
            &fill(Color::from_argb(0x1f, tint.r(), tint.g(), tint.b())),
        );
        let mut curve = stroke(tint, 2.5);
        curve.set_stroke_join(otto_kit::skia::paint::Join::Round);
        canvas.draw_path(&line.detach(), &curve);
        canvas.restore();

        // The bands' points, on top.
        for (index, band) in effect.bands.iter().enumerate() {
            let center = self.point_of(effect, band);
            let selected = self.effects.band == Some(index);
            let r = if selected { POINT_R + 2.0 } else { POINT_R };
            let ground = if selected {
                tint
            } else {
                graph_view::card_ground(theme)
            };
            canvas.draw_circle(center, r, &fill(ground));
            canvas.draw_circle(center, r, &stroke(tint, 2.0));
            Label::new((index + 1).to_string())
                .with_style(styles::CAPTION_2_EMPHASIZED)
                .with_color(if selected { Color::WHITE } else { tint })
                .centered_at(center.x, center.y)
                .render(canvas);
            self.targets.push((
                Rect::from_xywh(
                    center.x - POINT_HIT,
                    center.y - POINT_HIT,
                    POINT_HIT * 2.0,
                    POINT_HIT * 2.0,
                ),
                Action::Effect(EffectAction::Point(index)),
            ));
        }
    }

    /// Where `band`'s point sits on the plot: at its frequency and gain, or
    /// on the curve for a filter with no gain.
    fn point_of(&self, effect: &Effect, band: &Band) -> Point {
        let plot = self.effects.plot;
        let db = if band.kind.has_gain() {
            band.gain
        } else {
            response_db(effect.preamp, &effect.bands, band.freq)
        };
        Point::new(
            x_of(plot, band.freq),
            y_of(plot, db.clamp(-DB_RANGE, DB_RANGE)),
        )
    }

    fn draw_band_card(
        &mut self,
        canvas: &Canvas,
        card: Rect,
        index: usize,
        band: &Band,
        theme: &Theme,
    ) {
        let selected = self.effects.band == Some(index);
        let rr = RRect::new_rect_xy(card, 10.0, 10.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        if selected {
            canvas.draw_rrect(rr, &stroke(theme.accent, 2.0));
        } else {
            canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        }
        self.targets
            .push((card, Action::Effect(EffectAction::Band(index))));
        let left = card.left + 12.0;
        let head = card.top + 18.0;
        let title = t_owned!("audio-effects-band", number = index + 1);
        let title_w = styles::FOOTNOTE_EMPHASIZED
            .font()
            .measure_str(&title, None)
            .0;
        Label::new(title)
            .with_style(styles::FOOTNOTE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, head)
            .render(canvas);

        // Remove, top right, and the type as a pop-up left of it.
        let close = Rect::from_xywh(card.right - 26.0, head - 9.0, 18.0, 18.0);
        let c = Point::new(close.center_x(), close.center_y());
        let mut cross = stroke(theme.text_tertiary, 1.4);
        cross.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
        canvas.draw_line(
            Point::new(c.x - 3.5, c.y - 3.5),
            Point::new(c.x + 3.5, c.y + 3.5),
            &cross,
        );
        canvas.draw_line(
            Point::new(c.x - 3.5, c.y + 3.5),
            Point::new(c.x + 3.5, c.y - 3.5),
            &cross,
        );
        self.targets
            .push((close, Action::Effect(EffectAction::RemoveBand(index))));
        let font = styles::CAPTION_1.font();
        let kind = format!("{} ▾", band_label(band.kind));
        let kind = ellipsize(&font, &kind, close.left - 10.0 - left - title_w);
        let kind_w = font.measure_str(&kind, None).0;
        let kind_x = close.left - 4.0 - kind_w;
        Label::new(kind)
            .with_style(styles::CAPTION_1)
            .with_color(theme.text_secondary)
            .centered_on(kind_x, head)
            .render(canvas);
        self.targets.push((
            Rect::from_ltrb(kind_x - 4.0, head - 10.0, close.left, head + 10.0),
            Action::Effect(EffectAction::BandTypeMenu(index)),
        ));

        let gain = if band.kind.has_gain() {
            db_text(band.gain)
        } else {
            "\u{2014}".to_string()
        };
        let rows = [
            (t!("audio-effects-freq"), freq_text(band.freq), Field::Freq),
            (t!("audio-effects-gain"), gain, Field::Gain),
            (t!("audio-effects-q"), format!("{:.2}", band.q), Field::Q),
        ];
        for (row, (label, value, field)) in rows.into_iter().enumerate() {
            let cy = card.top + 44.0 + row as f32 * 20.0;
            Label::new(label)
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_tertiary)
                .centered_on(left, cy)
                .render(canvas);
            Label::new(value)
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_primary)
                .centered_on(left + 44.0, cy)
                .render(canvas);
            if field != Field::Gain || band.kind.has_gain() {
                self.targets.push((
                    Rect::from_ltrb(left + 40.0, cy - 9.0, card.right - 8.0, cy + 9.0),
                    Action::Effect(EffectAction::Scrub(index, field)),
                ));
            }
        }
    }

    fn draw_params(&mut self, canvas: &Canvas, area: Rect, effect: &Effect, theme: &Theme) {
        let specs = effect.kind.params();
        let h = 20.0 + specs.len() as f32 * PARAM_ROW_H;
        let card = Rect::from_xywh(area.left, area.top, area.width(), h);
        let rr = RRect::new_rect_xy(card, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        let label_w = 170.0;
        let readout_w = 84.0;
        let dragging = match self.effects.drag {
            Some(EffectDrag::Param { index, .. }) => Some(index),
            _ => None,
        };
        for (index, spec) in specs.iter().enumerate() {
            let cy = card.top + 10.0 + PARAM_ROW_H * (index as f32 + 0.5);
            let font = styles::SUBHEADLINE.font();
            Label::new(ellipsize(
                &font,
                param_label(effect.kind, spec),
                label_w - 12.0,
            ))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(card.left + 18.0, cy)
            .render(canvas);
            let track = Rect::from_ltrb(
                card.left + 18.0 + label_w,
                cy - slider::KNOB_RADIUS,
                card.right - 18.0 - readout_w,
                cy + slider::KNOB_RADIUS,
            );
            let value = effect.param(spec);
            let interaction = if dragging == Some(index) {
                SliderInteraction::Pressed
            } else {
                SliderInteraction::Normal
            };
            slider::draw(
                canvas,
                track,
                value,
                spec.min,
                spec.max,
                None,
                interaction,
                theme,
            );
            Label::new(param_text(spec, value))
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_secondary)
                .centered_on(track.right + 16.0, cy)
                .render(canvas);
            self.targets.push((
                track.with_outset((slider::KNOB_RADIUS, 6.0)),
                Action::Effect(EffectAction::Param(index)),
            ));
            if index + 1 < specs.len() {
                let y = cy + PARAM_ROW_H / 2.0;
                canvas.draw_line(
                    Point::new(card.left + 18.0, y),
                    Point::new(card.right - 18.0, y),
                    &stroke(theme.hairline, 1.0),
                );
            }
        }
        if let Some(about) = about_effect(effect.kind) {
            let font = styles::FOOTNOTE.font();
            for (i, line) in wrap(&font, about, area.width(), 3).iter().enumerate() {
                Label::new(line)
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_secondary)
                    .centered_on(area.left, card.bottom + 20.0 + i as f32 * 16.0)
                    .render(canvas);
            }
        }
    }

    fn draw_new_form(&mut self, canvas: &Canvas, area: Rect, theme: &Theme) {
        let h = 44.0 + EffectKind::ALL.len() as f32 * FORM_KIND_H + 2.0 * FORM_ROW_H + 64.0;
        let card = Rect::from_xywh(area.left, area.top, area.width().min(620.0), h);
        let rr = RRect::new_rect_xy(card, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        let left = card.left + 20.0;
        let inner_right = card.right - 20.0;
        card_heading(
            canvas,
            t!("audio-effects-new-heading"),
            left,
            card.top + 22.0,
            theme,
        );

        // The kinds, each with what it does or what it is missing.
        let mut y = card.top + 40.0;
        let supported = self.effects.supported.clone();
        for (kind, support) in supported {
            let row = Rect::from_xywh(left - 8.0, y, inner_right - left + 16.0, FORM_KIND_H - 4.0);
            let chosen = self.effects.form_kind == kind;
            let ready = support == Support::Ready;
            if chosen {
                let a = theme.accent;
                canvas.draw_rrect(
                    RRect::new_rect_xy(row, 8.0, 8.0),
                    &fill(Color::from_argb(0x1c, a.r(), a.g(), a.b())),
                );
            }
            let c = Point::new(row.left + 16.0, row.center_y());
            if chosen {
                canvas.draw_circle(c, 7.0, &fill(theme.accent));
                canvas.draw_circle(c, 2.6, &fill(Color::WHITE));
            } else {
                canvas.draw_circle(c, 7.0, &stroke(theme.fill_secondary, 1.5));
            }
            let ink = if ready {
                theme.text_primary
            } else {
                theme.text_tertiary
            };
            Label::new(kind_label(kind))
                .with_style(styles::SUBHEADLINE_EMPHASIZED)
                .with_color(ink)
                .centered_on(row.left + 34.0, row.top + 13.0)
                .render(canvas);
            let about = match &support {
                Support::Ready => kind_about(kind).to_string(),
                Support::Missing { package, .. } => {
                    t_owned!("audio-effects-missing", package = *package)
                }
            };
            let font = styles::FOOTNOTE.font();
            Label::new(ellipsize(&font, &about, row.width() - 44.0))
                .with_style(styles::FOOTNOTE)
                .with_color(if ready {
                    theme.text_secondary
                } else {
                    theme.accent_red
                })
                .centered_on(row.left + 34.0, row.top + 30.0)
                .render(canvas);
            if ready {
                self.targets
                    .push((row, Action::Effect(EffectAction::FormKind(kind))));
            }
            y += FORM_KIND_H;
        }

        let label_font = styles::SUBHEADLINE.font();
        let row_label = |canvas: &Canvas, text: &str, cy: f32| {
            Label::new(ellipsize(&label_font, text, FORM_LABEL_W - 8.0))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(left, cy)
                .render(canvas);
        };
        let control_left = left + FORM_LABEL_W;
        let control_w = (inner_right - control_left).min(360.0);

        // Output or input; noise suppression is for inputs only.
        let cy = y + FORM_ROW_H / 2.0;
        row_label(canvas, t!("audio-effects-side"), cy);
        let sides = [EffectSide::Output, EffectSide::Input];
        let labels: Vec<String> = sides.iter().map(|s| side_label(*s).to_string()).collect();
        let chosen = sides
            .iter()
            .position(|s| *s == self.effects.form_side)
            .unwrap_or(0);
        let track = Rect::from_xywh(
            control_left,
            cy - segmented::HEIGHT / 2.0,
            control_w,
            segmented::HEIGHT,
        );
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &sides.map(|s| Action::Effect(EffectAction::FormSide(s))),
            theme,
        );
        y += FORM_ROW_H;

        // The device it sits on.
        let cy = y + FORM_ROW_H / 2.0;
        row_label(canvas, t!("audio-effects-device"), cy);
        let device = self.form_device();
        let text = device.as_deref().map_or_else(
            || t!("audio-effects-no-device").to_string(),
            |name| self.node_title(name),
        );
        let text = format!(
            "{}  ▾",
            ellipsize(&styles::BODY.font(), &text, control_w - 40.0)
        );
        let field = Rect::from_xywh(
            control_left,
            cy - push_button::HEIGHT / 2.0,
            control_w,
            push_button::HEIGHT,
        );
        self.button(
            canvas,
            field,
            &text,
            Action::Effect(EffectAction::FormDeviceMenu),
            true,
            theme,
        );

        // Add, and Cancel when there is an effect to go back to.
        let add = t!("audio-effects-add");
        let w = push_button::width(add);
        let button = Rect::from_xywh(
            inner_right - w,
            card.bottom - 20.0 - push_button::HEIGHT,
            w,
            push_button::HEIGHT,
        );
        let ready = device.is_some() && support(self.effects.form_kind) == Support::Ready;
        self.role_button(
            canvas,
            button,
            add,
            Action::Effect(EffectAction::Create),
            ready,
            ButtonRole::Default,
            theme,
        );
        if !self.routing.effects().is_empty() {
            let cancel = t!("audio-effects-cancel");
            let cw = push_button::width(cancel);
            self.button(
                canvas,
                Rect::from_xywh(button.left - 8.0 - cw, button.top, cw, push_button::HEIGHT),
                cancel,
                Action::Effect(EffectAction::CancelNew),
                true,
                theme,
            );
        }
    }

    /// The devices a new effect of the chosen side can sit on, by
    /// `node.name`: the default first, then the rest by name.
    fn device_choices(&self) -> Vec<String> {
        let side = self.effects.form_side;
        let endpoint = match side {
            EffectSide::Output => Endpoint::Output,
            EffectSide::Input => Endpoint::Input,
        };
        let default = self.graph.default_name(endpoint).map(str::to_string);
        let mut nodes: Vec<(String, String)> = self
            .graph
            .nodes()
            .filter(|n| match side {
                EffectSide::Output => n.media_class() == "Audio/Sink",
                EffectSide::Input => n.media_class().starts_with("Audio/Source"),
            })
            .map(|n| (n.title().to_string(), n.name().to_string()))
            .filter(|(_, name)| !name.is_empty())
            .collect();
        nodes.sort();
        let mut names: Vec<String> = nodes.into_iter().map(|(_, name)| name).collect();
        if let Some(default) = default {
            if let Some(at) = names.iter().position(|n| *n == default) {
                let first = names.remove(at);
                names.insert(0, first);
            }
        }
        names
    }

    /// The device the form will use: the one picked, else the default.
    fn form_device(&self) -> Option<String> {
        let choices = self.device_choices();
        self.effects
            .form_device
            .clone()
            .filter(|d| choices.contains(d))
            .or_else(|| choices.into_iter().next())
    }

    // -----------------------------------------------------------------------
    // Pointer
    // -----------------------------------------------------------------------

    /// Open the pop-up `action` asks for; `false` when it asks for none.
    pub(super) fn open_effect_menu(&mut self, action: Action, rect: Rect, serial: u32) -> bool {
        let Action::Effect(effect_action) = action else {
            return false;
        };
        let (options, choices, selected): (Vec<String>, Vec<Action>, Option<usize>) =
            match effect_action {
                EffectAction::FormDeviceMenu => {
                    let names = self.device_choices();
                    let current = self.form_device();
                    let selected = names.iter().position(|n| Some(n) == current.as_ref());
                    let options = names.iter().map(|n| self.node_title(n)).collect();
                    let choices = (0..names.len())
                        .map(|i| Action::Effect(EffectAction::FormDevice(i)))
                        .collect();
                    self.effects.device_choices = names;
                    (options, choices, selected)
                }
                EffectAction::PresetMenu => (
                    vec![
                        t!("audio-effects-preset-flat").to_string(),
                        t!("audio-effects-preset-import").to_string(),
                    ],
                    vec![
                        Action::Effect(EffectAction::Flat),
                        Action::Effect(EffectAction::Import),
                    ],
                    None,
                ),
                EffectAction::BandTypeMenu(band) => {
                    let current = self
                        .shown_effect()
                        .and_then(|e| e.bands.get(band).map(|b| b.kind));
                    (
                        BandType::ALL
                            .iter()
                            .map(|t| band_label(*t).to_string())
                            .collect(),
                        BandType::ALL
                            .iter()
                            .map(|t| Action::Effect(EffectAction::BandType(band, *t)))
                            .collect(),
                        BandType::ALL.iter().position(|t| Some(*t) == current),
                    )
                }
                _ => return false,
            };
        if options.is_empty() {
            return true;
        }
        self.menu_choices = choices;
        self.menu_open = Some(action);
        self.menu_request = Some(MenuRequest {
            rect,
            serial,
            options,
            selected,
        });
        true
    }

    /// Start a drag for `action` pressed at `p`; `false` for a control that
    /// acts on release instead.
    pub(super) fn press_effect(&mut self, action: Action, rect: Rect, p: Point) -> bool {
        let Action::Effect(action) = action else {
            return false;
        };
        let Some(effect) = self.shown_effect() else {
            return false;
        };
        match action {
            EffectAction::Point(band) => {
                self.effects.band = Some(band);
                self.effects.drag = Some(EffectDrag::Point(band));
                true
            }
            EffectAction::Scrub(band, field) => {
                let start = match field {
                    Field::Preamp => effect.preamp,
                    _ => {
                        let Some(b) = effect.bands.get(band) else {
                            return false;
                        };
                        self.effects.band = Some(band);
                        match field {
                            Field::Freq => b.freq,
                            Field::Gain => b.gain,
                            _ => b.q,
                        }
                    }
                };
                self.effects.drag = Some(EffectDrag::Scrub {
                    band,
                    field,
                    from_x: p.x,
                    start,
                });
                AppContext::set_cursor_shape(CursorShape::EwResize);
                true
            }
            EffectAction::Param(index) => {
                let track = rect.with_inset((slider::KNOB_RADIUS, 6.0));
                self.effects.drag = Some(EffectDrag::Param { index, track });
                self.drag_effect(p);
                true
            }
            _ => false,
        }
    }

    /// Follow a drag of this section; `false` when there is none.
    pub(super) fn drag_effect(&mut self, p: Point) -> bool {
        let Some(drag) = self.effects.drag else {
            return false;
        };
        let Some(effect) = self.shown_effect() else {
            return false;
        };
        let id = effect.id.clone();
        match drag {
            EffectDrag::Point(band) => {
                let plot = self.effects.plot;
                let freq = freq_at(plot, p.x.clamp(plot.left, plot.right));
                let gain = db_at(plot, p.y.clamp(plot.top, plot.bottom));
                self.change_effect(&id, true, |e| {
                    if let Some(b) = e.bands.get_mut(band) {
                        b.freq = freq;
                        if b.kind.has_gain() {
                            b.gain = (gain * 10.0).round() / 10.0;
                        }
                    }
                    e.preset = None;
                });
            }
            EffectDrag::Scrub {
                band,
                field,
                from_x,
                start,
            } => {
                let dx = p.x - from_x;
                self.change_effect(&id, true, |e| {
                    set_field(e, band, field, scrubbed(field, start, dx))
                });
            }
            EffectDrag::Param { index, track } => {
                let Some(spec) = effect.kind.params().get(index).copied() else {
                    return false;
                };
                let value = slider::value_at(track, spec.min, spec.max, None, p.x);
                self.change_effect(&id, true, |e| {
                    e.params.insert(spec.key.to_string(), value);
                });
            }
        }
        true
    }

    /// End a drag of this section; `true` when one ended.
    pub(super) fn release_effect(&mut self) -> bool {
        let Some(drag) = self.effects.drag.take() else {
            return false;
        };
        if matches!(drag, EffectDrag::Scrub { .. }) {
            AppContext::set_cursor_shape(CursorShape::Default);
        }
        self.flush_effects();
        true
    }

    /// Scroll over the section: a band's point or the plot changes the
    /// band's width, a scrubbed value moves, a slider steps.
    pub(super) fn scroll_effects(&mut self, p: Point, dy: f32, discrete: bool, stop: bool) -> bool {
        if stop {
            self.flush_effects();
            return false;
        }
        let Some(effect) = self.shown_effect() else {
            return false;
        };
        let id = effect.id.clone();
        let target = self.target_at(p);
        let changed = match target {
            Some(Action::Effect(EffectAction::Point(band) | EffectAction::Band(band))) => {
                self.effects.band = Some(band);
                self.change_effect(&id, true, |e| {
                    if let Some(b) = e.bands.get_mut(band) {
                        b.q = (b.q * (-dy * Q_PER_POINT).exp()).clamp(MIN_Q, MAX_Q);
                    }
                    e.preset = None;
                });
                true
            }
            Some(Action::Effect(EffectAction::Plot)) => match self.effects.band {
                Some(band) => {
                    self.change_effect(&id, true, |e| {
                        if let Some(b) = e.bands.get_mut(band) {
                            b.q = (b.q * (-dy * Q_PER_POINT).exp()).clamp(MIN_Q, MAX_Q);
                        }
                        e.preset = None;
                    });
                    true
                }
                None => false,
            },
            Some(Action::Effect(EffectAction::Scrub(band, field))) => {
                let current = field_value(&effect, band, field);
                self.change_effect(&id, true, |e| {
                    set_field(e, band, field, scrubbed(field, current, -dy));
                });
                true
            }
            Some(Action::Effect(EffectAction::Param(index))) => {
                let Some(spec) = effect.kind.params().get(index).copied() else {
                    return false;
                };
                let step = (spec.max - spec.min) / 200.0;
                let value = (effect.param(&spec) - dy * step).clamp(spec.min, spec.max);
                self.change_effect(&id, true, |e| {
                    e.params.insert(spec.key.to_string(), value);
                });
                true
            }
            _ => false,
        };
        if changed && discrete {
            self.flush_effects();
        }
        changed
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Change the effect `id`. Its controls are set on its node at once; the
    /// routing file is written now, or with `live` when the drag or scroll
    /// ends.
    fn change_effect(&mut self, id: &str, live: bool, change: impl FnOnce(&mut Effect)) {
        if !self.routing.update_effect(id, change) {
            return;
        }
        if let Some(effect) = self.routing.effect(id) {
            self.connection.set_effect_controls(effect);
        }
        if live {
            self.effects.unsaved = true;
        } else {
            self.effects.unsaved = false;
            self.save_routing_file();
        }
    }

    /// Write a change heard live but not saved yet.
    pub(super) fn flush_effects(&mut self) {
        if std::mem::take(&mut self.effects.unsaved) {
            self.save_routing_file();
        }
    }

    pub(super) fn run_effect(&mut self, action: EffectAction) {
        let shown = self.shown_effect();
        let id = shown.as_ref().map(|e| e.id.clone()).unwrap_or_default();
        match action {
            EffectAction::Select(index) => {
                if let Some(effect) = self.routing.effects().get(index) {
                    if self.effects.selected.as_deref() != Some(effect.id.as_str()) {
                        self.effects.band = None;
                        self.effects.note = None;
                    }
                    self.effects.selected = Some(effect.id.clone());
                    self.effects.creating = false;
                }
            }
            EffectAction::New => {
                self.effects.creating = true;
                self.effects.supported = supported();
                self.effects.form_device = None;
            }
            EffectAction::CancelNew => self.effects.creating = false,
            EffectAction::FormKind(kind) => {
                self.effects.form_kind = kind;
                if kind == EffectKind::NoiseSuppression
                    && self.effects.form_side != EffectSide::Input
                {
                    self.effects.form_side = EffectSide::Input;
                    self.effects.form_device = None;
                }
            }
            EffectAction::FormSide(side) => {
                if self.effects.form_kind == EffectKind::NoiseSuppression {
                    return;
                }
                if self.effects.form_side != side {
                    self.effects.form_side = side;
                    self.effects.form_device = None;
                }
            }
            EffectAction::FormDevice(index) => {
                self.effects.form_device = self.effects.device_choices.get(index).cloned();
            }
            EffectAction::Create => self.create_effect(),
            EffectAction::Toggle => self.change_effect(&id, false, |e| e.enabled = !e.enabled),
            EffectAction::Remove => {
                if self.routing.remove_effect(&id) {
                    self.save_routing_file();
                }
                self.effects.selected = None;
                self.effects.band = None;
                self.effects.note = None;
            }
            EffectAction::Flat => {
                self.change_effect(&id, false, |e| {
                    e.preamp = 0.0;
                    e.bands = flat_bands();
                    e.preset = Some(t!("audio-effects-preset-flat").to_string());
                });
                self.effects.band = None;
                self.effects.note = None;
            }
            EffectAction::Import => self.import_autoeq(&id),
            EffectAction::Band(band) => self.effects.band = Some(band),
            EffectAction::BandType(band, kind) => self.change_effect(&id, false, |e| {
                if let Some(b) = e.bands.get_mut(band) {
                    b.kind = kind;
                }
                e.preset = None;
            }),
            EffectAction::RemoveBand(band) => {
                self.change_effect(&id, false, |e| {
                    if band < e.bands.len() {
                        e.bands.remove(band);
                    }
                    e.preset = None;
                });
                self.effects.band = None;
            }
            EffectAction::AddBand => {
                let Some(effect) = shown else {
                    return;
                };
                let band = new_band(&effect.bands);
                self.change_effect(&id, false, |e| {
                    e.bands.push(band);
                    e.preset = None;
                });
                self.effects.band = Some(effect.bands.len());
            }
            EffectAction::Plot => self.effects.band = None,
            // Opened on the press, by `open_effect_menu`, or dragged.
            EffectAction::FormDeviceMenu
            | EffectAction::PresetMenu
            | EffectAction::BandTypeMenu(_)
            | EffectAction::Point(_)
            | EffectAction::Scrub(..)
            | EffectAction::Param(_) => {}
        }
    }

    fn create_effect(&mut self) {
        let Some(device) = self.form_device() else {
            return;
        };
        let kind = self.effects.form_kind;
        let base = kind_label(kind);
        let taken = |name: &str| self.routing.effects().iter().any(|e| e.name == name);
        let mut name = base.to_string();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base} {n}");
            n += 1;
        }
        let id = self
            .routing
            .add_effect(&name, kind, self.effects.form_side, &device);
        self.save_routing_file();
        self.effects.selected = Some(id);
        self.effects.creating = false;
        self.effects.band = None;
        self.effects.note = None;
    }

    /// Ask for an AutoEQ file through the portal; the answer comes back in
    /// [`Ui::poll_import`].
    fn import_autoeq(&mut self, id: &str) {
        let answer = Arc::clone(&self.effects.import);
        let id = id.to_string();
        let spawned = Request::open_file(t!("audio-effects-import-title"))
            .filter(
                t!("audio-effects-import-filter"),
                &[Rule::Glob("*.txt".into())],
            )
            .filter(t!("audio-effects-import-all"), &[Rule::Glob("*".into())])
            .spawn(move |outcome| {
                if let Ok(mut answer) = answer.lock() {
                    *answer = Some((id, outcome));
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not open the file dialog");
        }
    }

    /// Apply an import the dialog answered; returns whether to redraw.
    pub fn poll_import(&mut self) -> bool {
        let answer = self.effects.import.lock().ok().and_then(|mut a| a.take());
        let Some((id, outcome)) = answer else {
            return false;
        };
        let path = match outcome {
            Outcome::Chosen(paths) => paths.into_iter().next(),
            Outcome::Dismissed => None,
            Outcome::Failed(why) => {
                tracing::warn!(why, "the file dialog failed");
                None
            }
        };
        let Some(path) = path else {
            return false;
        };
        let text = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= MAX_IMPORT_BYTES)
            .and_then(|_| std::fs::read_to_string(&path).ok());
        let parsed = text.as_deref().map(parse_autoeq);
        match parsed {
            Some(Ok(eq)) => {
                let name = path
                    .file_stem()
                    .map(|s| {
                        s.to_string_lossy()
                            .trim_end_matches(" ParametricEQ")
                            .to_string()
                    })
                    .unwrap_or_default();
                let count = eq.bands.len();
                self.change_effect(&id, false, |e| {
                    e.preamp = eq.preamp;
                    e.bands = eq.bands;
                    e.preset = Some(t_owned!("audio-effects-preset-autoeq", name = name));
                });
                self.effects.band = None;
                self.effects.note = Some(if eq.skipped == 0 {
                    t_owned!("audio-effects-imported", count = count)
                } else {
                    t_owned!(
                        "audio-effects-imported-skipped",
                        count = count,
                        skipped = eq.skipped
                    )
                });
            }
            Some(Err(error)) => {
                tracing::warn!(%error, path = %path.display(), "not an AutoEQ profile");
                self.effects.note = Some(t!("audio-effects-import-failed").to_string());
            }
            None => {
                self.effects.note = Some(t!("audio-effects-import-unreadable").to_string());
            }
        }
        true
    }

    /// Fill the section for the `--png` preview, without writing the
    /// routing file: an equaliser like the design's, a limiter after it and
    /// noise suppression on the microphone.
    pub fn preview_effects(&mut self) {
        if !self.routing.effects().is_empty() {
            return;
        }
        let sink = self
            .graph
            .default_name(Endpoint::Output)
            .unwrap_or("speakers")
            .to_string();
        let source = self
            .graph
            .default_name(Endpoint::Input)
            .unwrap_or("microphone")
            .to_string();
        let eq = self.routing.add_effect(
            t!("audio-effects-sample-eq"),
            EffectKind::Equaliser,
            EffectSide::Output,
            &sink,
        );
        let band = |kind, freq, gain, q| Band {
            kind,
            freq,
            gain,
            q,
        };
        self.routing.update_effect(&eq, |e| {
            e.preamp = -4.0;
            e.bands = vec![
                band(BandType::LowShelf, 105.0, 4.0, 0.7),
                band(BandType::Peaking, 240.0, -1.0, 1.2),
                band(BandType::Peaking, 1100.0, 0.5, 0.9),
                band(BandType::Peaking, 3200.0, -3.0, 2.4),
                band(BandType::Peaking, 7800.0, 2.0, 3.0),
                band(BandType::LowPass, 18_000.0, 0.0, 0.71),
            ];
            e.preset = Some(t!("audio-effects-sample-preset").to_string());
        });
        let eq_node = self
            .routing
            .effect(&eq)
            .map(Effect::node_name)
            .unwrap_or_default();
        self.routing.add_effect(
            t!("audio-effects-kind-limiter"),
            EffectKind::Limiter,
            EffectSide::Output,
            &eq_node,
        );
        self.routing.add_effect(
            t!("audio-effects-sample-mic"),
            EffectKind::NoiseSuppression,
            EffectSide::Input,
            &source,
        );
        self.effects.selected = Some(eq);
        self.effects.band = Some(3);
    }
}

/// A band for Add band: a flat peak in the widest gap between the bands.
fn new_band(bands: &[Band]) -> Band {
    let mut freqs: Vec<f32> = bands.iter().map(|b| b.freq.ln()).collect();
    freqs.push(MIN_FREQ.ln());
    freqs.push(MAX_FREQ.ln());
    freqs.sort_by(f32::total_cmp);
    let (low, high) = freqs
        .windows(2)
        .map(|w| (w[0], w[1]))
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
        .unwrap_or((MIN_FREQ.ln(), MAX_FREQ.ln()));
    Band {
        kind: BandType::Peaking,
        freq: ((low + high) / 2.0).exp(),
        gain: 0.0,
        q: 1.0,
    }
}

fn field_value(effect: &Effect, band: usize, field: Field) -> f32 {
    match field {
        Field::Preamp => effect.preamp,
        Field::Freq => effect.bands.get(band).map_or(1000.0, |b| b.freq),
        Field::Gain => effect.bands.get(band).map_or(0.0, |b| b.gain),
        Field::Q => effect.bands.get(band).map_or(1.0, |b| b.q),
    }
}

/// `start` moved by `dx` points of scrubbing.
fn scrubbed(field: Field, start: f32, dx: f32) -> f32 {
    match field {
        Field::Freq => (start * (dx * SCRUB_RATIO).exp()).clamp(MIN_FREQ, MAX_FREQ),
        Field::Q => (start * (dx * SCRUB_RATIO).exp()).clamp(MIN_Q, MAX_Q),
        Field::Gain | Field::Preamp => {
            let value = ((start + dx * SCRUB_DB) * 10.0).round() / 10.0;
            value.clamp(-MAX_GAIN, MAX_GAIN)
        }
    }
}

fn set_field(effect: &mut Effect, band: usize, field: Field, value: f32) {
    match field {
        Field::Preamp => effect.preamp = value,
        _ => {
            let Some(b) = effect.bands.get_mut(band) else {
                return;
            };
            match field {
                Field::Freq => b.freq = value,
                Field::Gain => b.gain = value,
                _ => b.q = value,
            }
        }
    }
    effect.preset = None;
}

/// The plot's x for `freq`, on a log scale from 20 Hz to 20 kHz.
fn x_of(plot: Rect, freq: f32) -> f32 {
    let t = (freq.max(MIN_FREQ) / MIN_FREQ).ln() / (MAX_FREQ / MIN_FREQ).ln();
    plot.left + plot.width() * t.clamp(0.0, 1.0)
}

/// The frequency at the plot's `x`.
fn freq_at(plot: Rect, x: f32) -> f32 {
    let t = ((x - plot.left) / plot.width().max(1.0)).clamp(0.0, 1.0);
    MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t)
}

fn y_of(plot: Rect, db: f32) -> f32 {
    plot.center_y() - db / DB_RANGE * plot.height() / 2.0
}

fn db_at(plot: Rect, y: f32) -> f32 {
    (plot.center_y() - y) / (plot.height() / 2.0).max(1.0) * DB_RANGE
}

/// A gain as people read it: +4.0 dB, −3.0 dB, with a true minus sign.
fn db_text(db: f32) -> String {
    let value = format!("{db:+.1}").replace('-', "\u{2212}");
    t_owned!("audio-effects-db", value = value)
}

/// A frequency: 105 Hz, 1.1 kHz, 18 kHz.
fn freq_text(freq: f32) -> String {
    if freq < 1000.0 {
        t_owned!("audio-effects-hz", value = format!("{freq:.0}"))
    } else if freq < 10_000.0 {
        t_owned!("audio-effects-khz", value = format!("{:.1}", freq / 1000.0))
    } else {
        t_owned!("audio-effects-khz", value = format!("{:.0}", freq / 1000.0))
    }
}

/// A grid frequency's short label: 50, 200, 1k, 10k.
fn freq_short(freq: f32) -> String {
    if freq < 1000.0 {
        format!("{freq:.0}")
    } else {
        t_owned!("audio-khz-short", value = format!("{:.0}", freq / 1000.0))
    }
}

fn param_text(spec: &ParamSpec, value: f32) -> String {
    match spec.unit {
        Unit::Decibels => db_text(value),
        Unit::Milliseconds => t_owned!("audio-ms-value", ms = format!("{value:.0}")),
        Unit::Seconds => t_owned!("audio-effects-seconds", value = format!("{value:.2}")),
        Unit::Ratio => t_owned!("audio-effects-ratio", value = format!("{value:.1}")),
        Unit::Percent => t_owned!("audio-effects-percent", value = format!("{value:.0}")),
    }
}

fn kind_label(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Equaliser => t!("audio-effects-kind-equaliser"),
        EffectKind::Limiter => t!("audio-effects-kind-limiter"),
        EffectKind::Compressor => t!("audio-effects-kind-compressor"),
        EffectKind::NoiseSuppression => t!("audio-effects-kind-noise"),
    }
}

fn kind_about(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Equaliser => t!("audio-effects-about-equaliser"),
        EffectKind::Limiter => t!("audio-effects-about-limiter"),
        EffectKind::Compressor => t!("audio-effects-about-compressor"),
        EffectKind::NoiseSuppression => t!("audio-effects-about-noise"),
    }
}

/// What the editor says under a kind's sliders.
fn about_effect(kind: EffectKind) -> Option<&'static str> {
    (kind != EffectKind::Equaliser).then(|| kind_about(kind))
}

fn side_label(side: EffectSide) -> &'static str {
    match side {
        EffectSide::Output => t!("audio-effects-side-output"),
        EffectSide::Input => t!("audio-effects-side-input"),
    }
}

fn band_label(kind: BandType) -> &'static str {
    match kind {
        BandType::LowShelf => t!("audio-effects-band-low-shelf"),
        BandType::Peaking => t!("audio-effects-band-peak"),
        BandType::HighShelf => t!("audio-effects-band-high-shelf"),
        BandType::LowPass => t!("audio-effects-band-low-pass"),
        BandType::HighPass => t!("audio-effects-band-high-pass"),
    }
}

fn param_label(kind: EffectKind, spec: &ParamSpec) -> &'static str {
    match (kind, spec.key) {
        (EffectKind::Limiter, "input-gain") => t!("audio-effects-param-input-gain"),
        (EffectKind::Limiter, "limit") => t!("audio-effects-param-limit"),
        (EffectKind::Limiter | EffectKind::Compressor, "release") => {
            t!("audio-effects-param-release")
        }
        (EffectKind::Compressor, "threshold") => t!("audio-effects-param-threshold"),
        (EffectKind::Compressor, "ratio") => t!("audio-effects-param-ratio"),
        (EffectKind::Compressor, "attack") => t!("audio-effects-param-attack"),
        (EffectKind::Compressor, "knee") => t!("audio-effects-param-knee"),
        (EffectKind::Compressor, "makeup") => t!("audio-effects-param-makeup"),
        (EffectKind::NoiseSuppression, "threshold") => t!("audio-effects-param-voice-threshold"),
        (EffectKind::NoiseSuppression, "grace") => t!("audio-effects-param-grace"),
        _ => spec.key,
    }
}

/// A band saying what is missing, without a button.
fn draw_missing(canvas: &Canvas, band: Rect, text: &str, theme: &Theme) {
    let tint = theme.accent_red;
    canvas.draw_rrect(
        RRect::new_rect_xy(band, 10.0, 10.0),
        &fill(Color::from_argb(0x26, tint.r(), tint.g(), tint.b())),
    );
    let font = styles::SUBHEADLINE.font();
    Label::new(ellipsize(&font, text, band.width() - 32.0))
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_primary)
        .centered_on(band.left + 16.0, band.center_y())
        .render(canvas);
}

/// The arrow between two steps of the chain line, from `from` to `to_x`.
fn draw_arrow(canvas: &Canvas, from: Point, to_x: f32, theme: &Theme) {
    let mut paint = stroke(theme.text_tertiary, 1.4);
    paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
    paint.set_stroke_join(otto_kit::skia::paint::Join::Round);
    canvas.draw_line(from, Point::new(to_x, from.y), &paint);
    let mut head = PathBuilder::new();
    head.move_to((to_x - 3.5, from.y - 3.5));
    head.line_to((to_x, from.y));
    head.line_to((to_x - 3.5, from.y + 3.5));
    canvas.draw_path(&head.detach(), &paint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plot_maps_frequencies_and_gains_both_ways() {
        let plot = Rect::from_xywh(10.0, 20.0, 600.0, 200.0);
        for freq in [20.0, 100.0, 1000.0, 20_000.0] {
            assert!((freq_at(plot, x_of(plot, freq)) - freq).abs() / freq < 1e-3);
        }
        assert_eq!(x_of(plot, 20.0), plot.left);
        assert_eq!(x_of(plot, 20_000.0), plot.right);
        assert!((db_at(plot, y_of(plot, 6.0)) - 6.0).abs() < 1e-4);
        assert_eq!(y_of(plot, DB_RANGE), plot.top);
    }

    #[test]
    fn a_new_band_goes_in_the_widest_gap() {
        let bands = flat_bands();
        let band = new_band(&bands);
        assert_eq!(band.kind, BandType::Peaking);
        // The widest gap of the flat set is below its low shelf.
        assert!(band.freq > MIN_FREQ && band.freq < 105.0, "{}", band.freq);
        assert!((new_band(&[]).freq - 632.0).abs() < 1.0);
    }

    #[test]
    fn scrubbing_stays_in_range() {
        assert_eq!(scrubbed(Field::Freq, 19_000.0, 500.0), MAX_FREQ);
        assert_eq!(scrubbed(Field::Q, 0.2, -500.0), MIN_Q);
        assert_eq!(scrubbed(Field::Gain, 1.0, 20.0), 2.0);
        assert_eq!(scrubbed(Field::Preamp, 0.0, -10_000.0), -MAX_GAIN);
    }
}
