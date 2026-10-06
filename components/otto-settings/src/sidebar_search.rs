//! The search field at the top of the sidebar, and the list of matches that
//! drops down from it.
//!
//! Every pane and every row the panes hold is something to find. The list is
//! the launcher's — the same rows, ranked by the same matching, so typing
//! finds the same way here as it does there — drawn in the accent: the
//! selected match is filled with it, and the letters each match was found by
//! are drawn in it.
//!
//! Picking a match goes to it: its pane is selected, the pane scrolls the row
//! into view, the keyboard lands on its control and the row flashes in the
//! accent so the eye finds it. `main.rs` does that part, because it owns the
//! pane and the focus; this module only says which row was meant.
//!
//! The list is drawn on a subsurface of its own, stacked over the pane's — it
//! is wider than the sidebar, and anything the window painted would sit under
//! the pane's surfaces. Like the password sheet's, that surface takes no
//! input: the pointer and the keyboard stay on the window, whose handlers ask
//! here where they landed.

use otto_kit::components::item_list::item::{rank, Item, Origin};
use otto_kit::components::item_list::rows::{
    paint_item_rows_styled, RowIcons, RowStyle, MAX_ROWS, ROW_H,
};
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::prelude::*;
use otto_kit::typography::styles;
use skia_safe::{BlurStyle, Contains, MaskFilter, PaintStyle, Point, RRect};

use crate::glyphs;
use crate::model::{Control, Pane};
use crate::settings_client;
use crate::view;
use crate::widgets;

/// How wide the list is. Wider than the sidebar it drops from: a row names a
/// setting and the pane and group it is in, and squeezing that into the
/// sidebar's width would crop most of them.
const PANEL_W: f32 = 360.0;
/// Space between the field and the list, and around the rows inside it.
const PANEL_GAP: f32 = 6.0;
const PANEL_PAD: f32 = 6.0;
const PANEL_RADIUS: f32 = 12.0;
/// What the list keeps clear of the window's bottom edge.
const PANEL_MARGIN: f32 = 10.0;
/// How tall the list is when nothing matches: one line saying so.
const EMPTY_H: f32 = 44.0;

/// Where a match goes: a pane, and the row in it where the match is a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub pane: usize,
    /// The row's handle — its setting's identifier, or its label where it has
    /// none (see `Row::handle`). `None` is the pane itself.
    pub row: Option<&'static str>,
}

/// One thing the field can find.
pub struct Entry {
    pub target: Target,
    /// The pane's sidebar glyph, which marks the row in the list.
    pub icon: &'static str,
    pub item: Item,
    /// Words that place the row without being its name — its pane, its
    /// group, its setting's identifier and the choices it offers — run
    /// together with the name. See [`search`].
    context: String,
    /// The row's help text. See [`search`].
    detail: String,
}

/// Everything there is to find, in sidebar order: each pane, then its rows.
///
/// Built from the panes as they are now, so a row that appears with what it
/// describes — an app in Privacy, an agent — is findable while it is there.
pub fn index(panes: &[Pane]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (pane_index, pane) in panes.iter().enumerate() {
        entries.push(Entry {
            target: Target {
                pane: pane_index,
                row: None,
            },
            icon: pane.icon,
            item: item(pane.name.to_string(), None, Vec::new()),
            context: String::new(),
            detail: String::new(),
        });
        for group in &pane.groups {
            for row in &group.rows {
                // A shortcut line is three controls with no label of its own,
                // and the line that adds one is not a setting.
                if row.label.is_empty()
                    || matches!(row.control, Control::Shortcut { .. } | Control::AddShortcut)
                {
                    continue;
                }
                let place = match group.title.as_deref() {
                    Some(group) if !group.is_empty() => format!("{} › {group}", pane.name),
                    _ => pane.name.to_string(),
                };
                // What someone might type that is not the label alone: the
                // pane or the group with it ("dock size"), and the setting's
                // identifier, which reads the same in every language.
                let mut terms = vec![format!("{} {}", pane.name, row.label)];
                if let Some(group) = group.title.as_deref() {
                    terms.push(format!("{group} {}", row.label));
                }
                if let Some(id) = row.id {
                    terms.push(id.to_string());
                }
                let mut context = vec![
                    pane.name.to_string(),
                    group.title.as_deref().unwrap_or_default().to_string(),
                    row.label.to_string(),
                ];
                context.extend(row.id.map(str::to_string));
                // A pop-up's choices, as it shows them: "dark" finds the row
                // that offers Dark.
                if let (Control::Select(_), Some(id)) = (&row.control, row.id) {
                    if let Some(desc) = settings_client::describe(id) {
                        context.extend(desc.choices.iter().map(|choice| desc.display(choice)));
                    }
                }
                entries.push(Entry {
                    target: Target {
                        pane: pane_index,
                        row: Some(row.handle()),
                    },
                    icon: pane.icon,
                    item: item(row.label.to_string(), Some(place), terms),
                    context: context.join(" "),
                    detail: row.detail.as_deref().unwrap_or_default().to_string(),
                });
            }
        }
    }
    entries
}

fn item(title: String, subtitle: Option<String>, search_terms: Vec<String>) -> Item {
    Item {
        title,
        subtitle,
        icon: None,
        activity: None,
        checked: None,
        search_terms,
        origin: Origin {
            source: 0,
            index: 0,
        },
    }
}

/// The entries `query` finds, best first, at most a list's worth.
///
/// Word by word first, the way someone names a setting: a row whose name has
/// a word starting with each word typed, then one whose pane, group,
/// identifier or choices supply the words its name does not ("dock size",
/// "timeout", "dark"), then one whose help text does. Within each, the order
/// is the launcher's ranking of the name, so "Tap to click" comes before
/// "Tap and drag" for the same reasons it would there.
///
/// Only when nothing is found that way do the launcher's looser matches
/// count — letters in order, not necessarily at a word's start — so "sz"
/// still finds Size, while "dock" is not answered with "Drag lock" for having
/// d, o, c, k in it.
pub fn search(entries: &[Entry], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let items: Vec<Item> = entries.iter().map(|entry| entry.item.clone()).collect();
    let ranked = rank(&items, query);
    let mut score = vec![i32::MIN; entries.len()];
    for m in &ranked {
        score[m.index] = m.score;
    }

    let mut found: Vec<(u8, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let tier = if words_start(&entry.item.title, query) {
                0
            } else if words_start(&entry.context, query) {
                1
            } else if words_start(&entry.detail, query) {
                2
            } else {
                return None;
            };
            Some((tier, index))
        })
        .collect();
    // Stable, so ties stay in sidebar order.
    found.sort_by_key(|&(tier, index)| (tier, std::cmp::Reverse(score[index])));
    let mut found: Vec<usize> = found.into_iter().map(|(_, index)| index).collect();

    if found.is_empty() {
        found = ranked
            .into_iter()
            .filter(|m| m.score >= floor(query))
            .map(|m| m.index)
            .collect();
    }
    found.truncate(MAX_ROWS);
    found
}

/// The least score a match by name needs to be listed.
///
/// Subsequence matching finds "dock" in "Display on cursor kind": every letter
/// is there, in order, scattered over four words. Each letter that lands is
/// worth 8 and each one that lands on a word's first letter 14 more, so a
/// typed word that starts a word of the name and runs on from there scores
/// far above one strewn across it — the floor asks for about half of what the
/// letters would be worth landing together.
fn floor(query: &str) -> i32 {
    let letters = query.chars().filter(|c| !c.is_whitespace()).count() as i32;
    (letters * 8 + 14) / 2
}

/// Whether every word of `query` starts a word of `text`, ignoring case.
///
/// Words are runs of letters and digits, in both: `dock.size` typed is the
/// words "dock" and "size", as the identifier it names is.
fn words_start(text: &str, query: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    };
    let have = words(text);
    let wanted = words(query);
    !have.is_empty()
        && !wanted.is_empty()
        && wanted
            .iter()
            .all(|wanted| have.iter().any(|word| word.starts_with(wanted.as_str())))
}

/// The field, what it has found, and which match is selected.
///
/// Shared between the window's handlers and its draw closure, so it lives
/// behind a lock in `main.rs`; everything that changes it bumps
/// [`Self::revision`], which is what tells the list's surface to repaint.
pub struct Search {
    pub input: TextInput,
    entries: Vec<Entry>,
    /// Indices into the index, best first.
    results: Vec<usize>,
    /// Which result is selected: the one Enter picks, drawn in the accent.
    pub selected: usize,
    /// Whether the list is down. It drops when the field has something in it
    /// and the keyboard, and goes up on Escape, a pick, or a press elsewhere.
    pub open: bool,
    /// The result a press on the list went down on: it is picked on release
    /// over the same row, as a button is.
    pub pressed: Option<usize>,
    /// A match picked, for `main.rs` to go to on its next update.
    pub pick: Option<Target>,
    /// The keyboard focus asked for from a pointer handler, which cannot move
    /// it itself; `main.rs` moves it on its next update. `Some(None)` takes
    /// the focus off the field.
    pub focus: Option<Option<FocusId>>,
    /// How many rows the window has room for: no more of the results than
    /// that are drawn, walked, picked or described.
    fit: usize,
    pub revision: u64,
}

impl Search {
    pub fn new(dark: bool) -> Self {
        let mut input = TextInput::new(String::new(), field_style(dark))
            .with_size(field_input_rect().width(), field_input_rect().height());
        input.state = input
            .state
            .clone()
            .with_placeholder(otto_kit::t!("settings-sidebar-search"));
        Self {
            input,
            entries: Vec::new(),
            results: Vec::new(),
            selected: 0,
            open: false,
            pressed: None,
            pick: None,
            focus: None,
            fit: MAX_ROWS,
            revision: 0,
        }
    }

    /// Draw the field for the scheme now in force.
    pub fn restyle(&mut self, dark: bool) {
        self.input.style = field_style(dark);
        self.touch();
    }

    /// Something about the field or the list changed and has to be drawn.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    /// Search again for what the field holds, against the panes as they are
    /// now, and drop the list if there is anything to show.
    pub fn refresh(&mut self, panes: &[Pane]) {
        self.entries = index(panes);
        self.results = search(&self.entries, self.query());
        self.selected = 0;
        self.open = !self.query().trim().is_empty();
        self.touch();
    }

    /// Empty the field and put the list away.
    pub fn clear(&mut self) {
        self.input.set_value(String::new());
        self.results.clear();
        self.selected = 0;
        self.open = false;
        self.pressed = None;
        self.touch();
    }

    /// Put the list away, keeping what was typed.
    pub fn close(&mut self) {
        if self.open || self.pressed.is_some() {
            self.open = false;
            self.pressed = None;
            self.touch();
        }
    }

    /// How many results the list shows.
    pub fn result_count(&self) -> usize {
        self.results.len().min(self.fit)
    }

    /// Show only as many results as a window `height` tall has room for.
    pub fn fit(&mut self, height: f32) {
        let fit = rows_that_fit(height);
        if fit != self.fit {
            self.fit = fit;
            self.selected = self.selected.min(fit - 1);
            self.touch();
        }
    }

    /// The `index`th result.
    pub fn result(&self, index: usize) -> Option<&Entry> {
        self.entries.get(*self.results.get(index)?)
    }

    /// Move the selection by `delta`, stopping at the ends. Returns whether it
    /// moved.
    pub fn move_selection(&mut self, delta: isize) -> bool {
        if self.results.is_empty() {
            return false;
        }
        let last = self.result_count() as isize - 1;
        let moved = (self.selected as isize + delta).clamp(0, last) as usize;
        if moved == self.selected {
            return false;
        }
        self.selected = moved;
        self.touch();
        true
    }

    /// Select the result the pointer is over, as the launcher's list does.
    pub fn hover(&mut self, index: usize) {
        if index < self.result_count() && index != self.selected {
            self.selected = index;
            self.touch();
        }
    }

    /// Go to the `index`th result: the list goes up and `main.rs` is asked to
    /// take the user there.
    pub fn choose(&mut self, index: usize) {
        let Some(target) = self.result(index).map(|entry| entry.target) else {
            return;
        };
        self.pick = Some(target);
        self.open = false;
        self.pressed = None;
        self.touch();
    }

    /// Where `handle` — a row's, as `--setting` names it — is, if any pane
    /// has it.
    pub fn find(panes: &[Pane], handle: &str) -> Option<Target> {
        index(panes)
            .into_iter()
            .map(|entry| entry.target)
            .find(|target| target.row == Some(handle))
    }
}

// ---------------------------------------------------------------------------
// The field
// ---------------------------------------------------------------------------

/// Height of the field.
pub const FIELD_H: f32 = 28.0;
/// The magnifier's place at the field's leading edge.
const GLYPH_W: f32 = 26.0;
/// The clear button's place at its trailing edge.
const CLEAR_W: f32 = 24.0;

/// Where the field sits: at the top of the sidebar, under the titlebar.
pub fn field_rect() -> Rect {
    Rect::from_xywh(
        10.0,
        view::titlebar_h() + 8.0,
        view::SIDEBAR_W - 20.0,
        FIELD_H,
    )
}

/// The part of the field the text is typed into, between the magnifier and
/// the clear button.
pub fn field_input_rect() -> Rect {
    let field = field_rect();
    Rect::from_ltrb(
        field.left + GLYPH_W,
        field.top,
        field.right - CLEAR_W,
        field.bottom,
    )
}

/// The button that empties the field, shown while it holds something.
pub fn clear_rect() -> Rect {
    let field = field_rect();
    Rect::from_ltrb(field.right - CLEAR_W, field.top, field.right, field.bottom)
}

/// The text's looks: no box of its own, because the field draws one.
fn field_style(dark: bool) -> TextInputStyle {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut style = TextInputStyle::with_theme(theme.clone());
    style.text_style = styles::BODY;
    style.horizontal_padding = 2.0;
    style.corner_radius = 0.0;
    style.focus_ring_width = 0.0;
    style.background = Color::TRANSPARENT;
    style.text_color = theme.text_primary;
    style.placeholder_color = theme.text_tertiary;
    style.caret_color = theme.accent;
    style
}

/// What the sidebar draws of the field, carried into the window's draw.
#[derive(Clone)]
pub struct FieldView {
    pub input: TextInput,
    /// Whether the field has the keyboard.
    pub focused: bool,
}

/// Draw the field: its box, the magnifier, what has been typed (or the
/// placeholder), and the clear button once there is something to clear.
///
/// The magnifier turns the accent while there is a query, so the sidebar says
/// it is searching even after the list has gone up.
pub fn paint_field(canvas: &Canvas, field: &FieldView, theme: &Theme, dark: bool) {
    let rect = field_rect();
    let rrect = RRect::new_rect_xy(rect, 7.0, 7.0);
    if field.focused {
        otto_kit::focus::draw_focus_ring(canvas, rect, 7.0);
    }
    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_color(theme.fill_quaternary);
    canvas.draw_rrect(rrect, &fill);
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(theme.fill_tertiary);
    canvas.draw_rrect(rrect, &edge);

    let searching = !field.input.value().is_empty();
    glyphs::draw(
        canvas,
        "search",
        rect.left + GLYPH_W / 2.0 + 1.0,
        rect.center_y(),
        13.0,
        if searching {
            theme.accent
        } else {
            theme.text_secondary
        },
    );

    let input = field_input_rect();
    canvas.save();
    canvas.translate((input.left, input.top));
    field.input.render_at(canvas, input.width(), input.height());
    canvas.restore();

    if searching {
        let clear = clear_rect();
        let centre = Point::new(clear.center_x() - 2.0, clear.center_y());
        let mut disc = Paint::default();
        disc.set_anti_alias(true);
        disc.set_color(theme.text_tertiary);
        canvas.draw_circle(centre, 7.0, &disc);
        let mut cross = Paint::default();
        cross.set_anti_alias(true);
        cross.set_style(PaintStyle::Stroke);
        cross.set_stroke_width(1.5);
        cross.set_color(view::pane_background(dark));
        let arm = 2.75;
        canvas.draw_line(
            (centre.x - arm, centre.y - arm),
            (centre.x + arm, centre.y + arm),
            &cross,
        );
        canvas.draw_line(
            (centre.x - arm, centre.y + arm),
            (centre.x + arm, centre.y - arm),
            &cross,
        );
    }
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

/// Where the list is drawn for `count` results in a window `width` x
/// `height`: under the field, as many rows tall as fit above the window's
/// bottom edge, and one line tall when nothing matched.
pub fn panel_rect(count: usize, width: f32, height: f32) -> Rect {
    let field = field_rect();
    let top = field.bottom + PANEL_GAP;
    let rows = count.min(rows_that_fit(height));
    let body = if rows == 0 {
        EMPTY_H
    } else {
        rows as f32 * ROW_H + PANEL_PAD * 2.0
    };
    let panel_w = PANEL_W.min(width - field.left - PANEL_MARGIN);
    Rect::from_xywh(field.left, top, panel_w, body)
}

/// How many rows of the list fit above the bottom edge of a window `height`
/// tall — at least one, and never more than [`MAX_ROWS`].
pub fn rows_that_fit(height: f32) -> usize {
    let top = field_rect().bottom + PANEL_GAP;
    let room = (height - PANEL_MARGIN - top - PANEL_PAD * 2.0).max(ROW_H);
    ((room / ROW_H).floor() as usize).clamp(1, MAX_ROWS)
}

/// The rect of the `index`th row of a list drawn at `panel`.
pub fn row_rect(panel: Rect, index: usize) -> Rect {
    Rect::from_xywh(
        panel.left,
        panel.top + PANEL_PAD + index as f32 * ROW_H,
        panel.width(),
        ROW_H,
    )
}

/// Which row of the list `(x, y)` is over, if any.
pub fn row_at(panel: Rect, count: usize, x: f32, y: f32) -> Option<usize> {
    if !panel.contains(Point::new(x, y)) {
        return None;
    }
    let rows = ((panel.height() - PANEL_PAD * 2.0) / ROW_H).round() as usize;
    (0..count.min(rows)).find(|&i| row_rect(panel, i).contains(Point::new(x, y)))
}

impl Search {
    /// Where the list is right now, in a window `width` x `height`, or
    /// `None` while it is up.
    pub fn panel(&self, width: f32, height: f32) -> Option<Rect> {
        self.open
            .then(|| panel_rect(self.results.len(), width, height))
    }
}

/// Paint the list over a window drawn at the canvas origin — onto its own
/// surface, which covers the whole window and which the caller has cleared:
/// a card under the field, with the matches in it.
///
/// `theme` is the window's — its accent muted while the window is in the
/// background, as everything else the window draws in the accent is.
pub fn paint_panel(
    canvas: &Canvas,
    (width, height): (f32, f32),
    dark: bool,
    theme: &Theme,
    search: &Search,
    icons: &RowIcons,
) {
    let Some(panel) = search.panel(width, height) else {
        return;
    };

    let card = RRect::new_rect_xy(panel, PANEL_RADIUS, PANEL_RADIUS);
    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_color(Color::from_argb(if dark { 0x66 } else { 0x40 }, 0, 0, 0));
    shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 12.0, None));
    canvas.draw_rrect(card.with_offset((0.0, 5.0)), &shadow);
    let mut ground = Paint::default();
    ground.set_anti_alias(true);
    ground.set_color(view::pane_background(dark));
    canvas.draw_rrect(card, &ground);
    let mut hairline = Paint::default();
    hairline.set_anti_alias(true);
    hairline.set_style(PaintStyle::Stroke);
    hairline.set_stroke_width(1.0);
    hairline.set_color(theme.fill_secondary);
    canvas.draw_rrect(card, &hairline);

    canvas.save();
    canvas.clip_rrect(card, skia_safe::ClipOp::Intersect, true);

    if search.results.is_empty() {
        let message = otto_kit::t_owned!("settings-sidebar-search-none", query = search.query());
        let message = otto_kit::typography::ellipsize(
            &styles::BODY.font(),
            &message,
            panel.width() - PANEL_PAD * 4.0,
        );
        widgets::text_centered_y(
            canvas,
            &message,
            panel.left + PANEL_PAD * 2.0 + 4.0,
            panel.center_y(),
            styles::BODY,
            theme.text_secondary,
        );
        canvas.restore();
        return;
    }

    let rows = ((panel.height() - PANEL_PAD * 2.0) / ROW_H).round() as usize;
    let shown: Vec<&Entry> = (0..search.results.len().min(rows))
        .filter_map(|i| search.result(i))
        .collect();
    let items: Vec<&Item> = shown.iter().map(|entry| &entry.item).collect();
    let selected = search.selected;
    // The pane's glyph in the icon's place: in the accent on every row but
    // the selected one, which is the accent itself and draws it white.
    let icon = |canvas: &Canvas, index: usize, (cx, cy): (f32, f32), _side: f32, _: Color| {
        let Some(entry) = shown.get(index) else {
            return;
        };
        let color = if index == selected {
            Color::WHITE
        } else {
            theme.accent
        };
        glyphs::draw(canvas, entry.icon, cx, cy, 17.0, color);
    };
    let style = RowStyle {
        selected: Some(selected),
        selection: Some(theme.material_selection_focused),
        on_selection: Some(Color::WHITE),
        query: search.query(),
        mark: Some(theme.accent),
        icon: Some(&icon),
    };

    canvas.save();
    canvas.translate((panel.left, panel.top + PANEL_PAD));
    paint_item_rows_styled(
        canvas,
        Rect::from_wh(panel.width(), rows as f32 * ROW_H),
        &items,
        &[""],
        panel.width(),
        dark,
        icons,
        &style,
    );
    canvas.restore();
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{group, untitled, Row};

    fn pane(name: &'static str, icon: &'static str, rows: Vec<Row>) -> Pane {
        Pane {
            name,
            icon,
            intro: None,
            groups: vec![group("Size", rows)],
        }
    }

    fn found(entries: &[Entry], query: &str) -> Vec<String> {
        search(entries, query)
            .into_iter()
            .map(|i| entries[i].item.title.clone())
            .collect()
    }

    fn panes() -> Vec<Pane> {
        let mut magnify = Row::new("Magnification", Control::Toggle(false));
        magnify.id = Some("dock.magnification");
        let mut size = Row::new(
            "Icon size",
            Control::Slider {
                value: 48.0,
                min: 16.0,
                max: 128.0,
                readout: "48 px".into(),
            },
        );
        size.id = Some("dock.size");
        let tap = Row::new("Tap to click", Control::Toggle(true))
            .detail("A light touch on the trackpad counts as a click.");
        vec![
            pane("Dock", "dock", vec![size, magnify]),
            Pane {
                name: "Trackpad",
                icon: "pointing",
                intro: None,
                groups: vec![untitled(vec![tap])],
            },
        ]
    }

    #[test]
    fn every_row_and_every_pane_can_be_found() {
        let entries = index(&panes());
        let targets: Vec<Target> = entries.iter().map(|e| e.target).collect();
        assert!(targets.contains(&Target { pane: 0, row: None }));
        assert!(targets.contains(&Target {
            pane: 0,
            row: Some("dock.size")
        }));
        assert!(targets.contains(&Target {
            pane: 1,
            row: Some("Tap to click")
        }));
    }

    #[test]
    fn a_row_is_found_by_its_label_first() {
        let entries = index(&panes());
        assert_eq!(found(&entries, "magn")[0], "Magnification");
    }

    #[test]
    fn the_pane_and_the_label_together_find_the_row() {
        let entries = index(&panes());
        assert_eq!(found(&entries, "dock size")[0], "Icon size");
    }

    #[test]
    fn a_setting_is_found_by_its_identifier() {
        let entries = index(&panes());
        assert!(found(&entries, "dock.size").contains(&"Icon size".to_string()));
    }

    #[test]
    fn help_text_finds_a_row_only_by_whole_word_starts() {
        let entries = index(&panes());
        assert_eq!(found(&entries, "light touch"), vec!["Tap to click"]);
        // Every letter of "lgt" is in the help text, in order, but no word of
        // it starts that way.
        assert!(!found(&entries, "lgt").contains(&"Tap to click".to_string()));
    }

    #[test]
    fn letters_strewn_across_a_name_are_not_a_match() {
        let entries = index(&panes());
        // d·o·c·k are all in "Tap to click" in order, scattered over it.
        assert!(!found(&entries, "dock").contains(&"Tap to click".to_string()));
    }

    #[test]
    fn a_word_of_the_identifier_finds_a_row_its_name_does_not_say() {
        let mut after = Row::new("Lock after", Control::Select(String::new()));
        after.id = Some("lock.auto_lock_timeout");
        let entries = index(&[pane("Lock & Login", "lock", vec![after])]);
        assert_eq!(found(&entries, "timeout"), vec!["Lock after"]);
    }

    #[test]
    fn letters_in_order_are_enough_when_nothing_starts_a_word() {
        let entries = index(&panes());
        assert_eq!(found(&entries, "sz")[0], "Icon size");
    }

    #[test]
    fn a_name_that_starts_with_the_query_comes_before_the_pane_it_is_in() {
        let entries = index(&panes());
        // "Icon size" and "Magnification" are found through the pane's name;
        // the pane itself is found by its own.
        assert_eq!(found(&entries, "dock")[0], "Dock");
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        let entries = index(&panes());
        assert!(search(&entries, "   ").is_empty());
    }

    #[test]
    fn a_row_named_by_its_handle_is_found_for_the_command_line() {
        assert_eq!(
            Search::find(&panes(), "dock.magnification"),
            Some(Target {
                pane: 0,
                row: Some("dock.magnification")
            })
        );
        assert_eq!(Search::find(&panes(), "nope"), None);
    }

    #[test]
    fn the_list_is_never_taller_than_the_window_has_room_for() {
        let panel = panel_rect(MAX_ROWS, 900.0, 300.0);
        assert!(panel.bottom <= 300.0 - PANEL_MARGIN + 0.5, "{panel:?}");
        assert!(panel.height() >= ROW_H);
    }

    #[test]
    fn only_the_rows_drawn_can_be_walked_to() {
        let mut search = Search::new(false);
        search.input.set_value("dock");
        search.refresh(&panes());
        assert!(search.result_count() > 1);
        search.move_selection(1);

        // A window with room for one row: the selection comes back to it,
        // and the arrows go nowhere a row is not drawn.
        search.fit(0.0);
        assert_eq!(search.result_count(), 1);
        assert_eq!(search.selected, 0);
        assert!(!search.move_selection(1));
        assert_eq!(
            panel_rect(search.result_count(), 900.0, 0.0).height(),
            ROW_H + PANEL_PAD * 2.0
        );
    }

    #[test]
    fn a_press_lands_on_the_row_drawn_there() {
        let panel = panel_rect(3, 900.0, 640.0);
        let second = row_rect(panel, 1);
        assert_eq!(
            row_at(panel, 3, second.center_x(), second.center_y()),
            Some(1)
        );
        assert_eq!(row_at(panel, 3, panel.right + 4.0, second.center_y()), None);
    }
}
