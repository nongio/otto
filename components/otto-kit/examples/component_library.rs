//! Render a picture of every visual otto-kit component, offscreen.
//!
//! Each component is drawn in a representative state, at scale 2 in the
//! light theme, on the window background, and written to its own PNG named
//! after its module. These are the pictures in the developer guide's
//! otto-kit library page; run this again after changing how a component
//! looks.
//!
//! ```sh
//! cargo run -p otto-kit --example component_library -- docs/developer/images/otto-kit/
//! ```

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::time::Instant;

use otto_kit::components::attachments::{Attachment, AttachmentList, Layout, Options};
use otto_kit::components::avatar;
use otto_kit::components::button::{Button, ButtonState};
use otto_kit::components::color_picker::panel::{self, HexField, Mode};
use otto_kit::components::color_picker::well::{self, WellInteraction};
use otto_kit::components::context_menu::ContextMenuRenderer;
use otto_kit::components::dropdown::field::{self, DropdownInteraction};
use otto_kit::components::icon::{GeometricIcon, Icon, IconShape};
use otto_kit::components::item_list::item::{Activity, Item, Origin};
use otto_kit::components::item_list::rows::{self, RowIcons, RowStyle};
use otto_kit::components::list::{self, ListLayout, ListRow};
use otto_kit::components::menu_bar::{MenuBarRenderer, MenuBarState, MenuBarStyle};
use otto_kit::components::menu_item::{MenuItem, MenuItemGroup, MenuItemStyle, VisualState};
use otto_kit::components::push_button::{self, PushButton, Role};
use otto_kit::components::scroll::{ScrollRenderer, ScrollState};
use otto_kit::components::segmented::{self, Segment, SegmentedInteraction};
use otto_kit::components::selectable_text::{TextRun, TextSelection};
use otto_kit::components::selection_list::{
    self, SelectionListItem, SelectionListLayout, SelectionListState,
};
use otto_kit::components::slider::{self, SliderInteraction};
use otto_kit::components::source_list::{self, SourceListItem, SourceListLayout};
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::components::titlebar::WindowDecoration;
use otto_kit::components::toggle::{self, ToggleInteraction};
use otto_kit::components::toolbar::{Toolbar, ToolbarGroup};
use otto_kit::icons::{solar_icon, SolarStyle};
use otto_kit::prelude::*;
use skia_safe::{surfaces, EncodedImageFormat, RRect};

/// Pixels per point: the scale most HiDPI laptops run Otto at.
const SCALE: f32 = 2.0;
/// Room around each component, in points, so nothing touches the edge.
const PAD: f32 = 12.0;
/// The window body every otto-kit window starts out painted with
/// (`Window::new`'s default background).
const WINDOW_BACKGROUND: Color = Color::from_rgb(245, 245, 245);

/// Draws one component's picture at the canvas origin.
type Draw = fn(&Canvas, &Theme);

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "component_library".to_string()),
    );
    std::fs::create_dir_all(&dir).expect("create the output directory");

    let mut theme = Theme::light_palette();
    theme.with_solid_materials(false);

    let samples: Vec<(&str, (f32, f32), Draw)> = vec![
        ("label", (300.0, 92.0), draw_label),
        ("icon", (300.0, 64.0), draw_icon),
        ("avatar", (196.0, 44.0), draw_avatar),
        ("container", (330.0, 110.0), draw_container),
        ("toolbar", (420.0, 48.0), draw_toolbar),
        ("scroll", (260.0, 150.0), draw_scroll),
        ("button", (440.0, 92.0), draw_button),
        ("push_button", (200.0, 28.0), draw_push_button),
        ("toggle", (100.0, 24.0), draw_toggle),
        ("slider", (240.0, 24.0), draw_slider),
        ("segmented", (300.0, 26.0), draw_segmented),
        ("text_input", (260.0, 64.0), draw_text_input),
        ("selectable_text", (260.0, 60.0), draw_selectable_text),
        ("dropdown", (400.0, 150.0), draw_dropdown),
        (
            "color_picker",
            (panel::panel_size(swatches().len()).0, 0.0),
            draw_color_picker,
        ),
        ("list", (380.0, 0.0), draw_list),
        ("source_list", (200.0, 0.0), draw_source_list),
        ("selection_list", (300.0, 0.0), draw_selection_list),
        ("item_list", (420.0, 0.0), draw_item_list),
        ("menu_bar", (420.0, 28.0), draw_menu_bar),
        ("menu_item", (240.0, 0.0), draw_menu_item),
        ("context_menu", (440.0, 0.0), draw_context_menu),
        ("titlebar", (440.0, 116.0), draw_titlebar),
        ("attachments", (300.0, 0.0), draw_attachments),
    ];

    for (name, (width, height), draw) in samples {
        // A height of zero asks for the component's own height.
        let height = if height > 0.0 {
            height
        } else {
            natural_height(name, width, &theme)
        };
        save(&dir, name, (width, height), |canvas| draw(canvas, &theme));
    }
}

/// Height of the components whose size follows from their content.
fn natural_height(name: &str, width: f32, theme: &Theme) -> f32 {
    match name {
        "color_picker" => {
            let (_, h) = panel::panel_size(swatches().len());
            well::HEIGHT + 12.0 + h
        }
        "list" => ListLayout::compute(&list_rows(), true, 0.0, 0.0, width).total_height(),
        "source_list" => SourceListLayout::compute(source_items().len(), 0.0, 0.0, width).height(),
        "selection_list" => SelectionListLayout::height_for(selection_items().len()),
        "item_list" => rows::ROW_H * launcher_items().len() as f32,
        "menu_item" => menu_item_group(width).height(),
        "context_menu" => context_menu_height(theme),
        "attachments" => {
            attachment_layout(&attachment_items(), width, theme)
                .1
                .height
        }
        _ => unreachable!("{name} has no natural height"),
    }
}

/// Draw one component into `<dir>/<name>.png`, `size` points plus padding.
fn save(dir: &Path, name: &str, (width, height): (f32, f32), draw: impl FnOnce(&Canvas)) {
    let px = |points: f32| ((points + PAD * 2.0) * SCALE).ceil() as i32;
    let mut surface = surfaces::raster_n32_premul((px(width), px(height))).expect("raster surface");
    let canvas = surface.canvas();
    canvas.clear(WINDOW_BACKGROUND);
    canvas.scale((SCALE, SCALE));
    canvas.translate((PAD, PAD));
    draw(canvas);

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    let path = dir.join(format!("{name}.png"));
    std::fs::write(&path, data.as_bytes()).expect("write png");
    println!("wrote {}", path.display());
}

// --- Text and images ------------------------------------------------------

fn draw_label(canvas: &Canvas, theme: &Theme) {
    let lines = [
        (
            "Title 2 emphasized",
            styles::TITLE_2_EMPHASIZED,
            theme.text_primary,
        ),
        (
            "Body: the default text style",
            styles::BODY,
            theme.text_primary,
        ),
        (
            "Subheadline, secondary",
            styles::SUBHEADLINE,
            theme.text_secondary,
        ),
        ("Caption, tertiary", styles::CAPTION_1, theme.text_tertiary),
    ];
    let mut cy = 12.0;
    for (text, style, color) in lines {
        Label::new(text)
            .with_style(style)
            .with_color(color)
            .centered_on(0.0, cy)
            .render(canvas);
        cy += 23.0;
    }
}

fn draw_icon(canvas: &Canvas, theme: &Theme) {
    let names = [
        "home", "search", "mail", "bell", "star", "settings", "trash",
    ];
    for (i, name) in names.iter().enumerate() {
        Icon::new(*name)
            .at(i as f32 * 40.0, 0.0)
            .with_size(24.0)
            .with_color(theme.text_primary)
            .render(canvas);
    }
    let shapes = [
        IconShape::Circle,
        IconShape::Square,
        IconShape::Triangle,
        IconShape::Star,
        IconShape::ChevronRight,
        IconShape::Check,
        IconShape::Plus,
    ];
    for (i, shape) in shapes.into_iter().enumerate() {
        GeometricIcon::new(shape)
            .at(i as f32 * 40.0, 40.0)
            .with_size(24.0)
            .with_color(theme.accent)
            .render(canvas);
    }
}

fn draw_avatar(canvas: &Canvas, _theme: &Theme) {
    let names = ["Ana López", "Riccardo", "Mira Okafor", ""];
    for (i, name) in names.iter().enumerate() {
        let rect = Rect::from_xywh(i as f32 * 52.0, 0.0, 44.0, 44.0);
        avatar::draw(canvas, rect, None, name);
    }
}

// --- Containers -----------------------------------------------------------

fn draw_container(canvas: &Canvas, theme: &Theme) {
    let caption = |text: &'static str, color: Color| {
        move |canvas: &Canvas, bounds: Rect| {
            Label::new(text)
                .with_style(styles::SUBHEADLINE_EMPHASIZED)
                .with_color(color)
                .centered_at(bounds.center_x(), bounds.center_y())
                .render(canvas);
        }
    };
    let mut stack = Stack::new(StackDirection::Horizontal).with_gap(12.0);
    stack.add(Box::new(
        FrameBuilder::new(100.0, 100.0)
            .with_background(Color::WHITE)
            .with_corner_radius(10.0)
            .with_border(theme.hairline, 1.0)
            .with_draw_fn(caption("Border", theme.text_secondary))
            .build(),
    ));
    stack.add(Box::new(
        FrameBuilder::new(100.0, 100.0)
            .with_background(Color::WHITE)
            .with_corner_radius(10.0)
            .with_shadow(BoxShadow::new(Color::from_argb(40, 0, 0, 0), 0.0, 2.0, 6.0))
            .with_draw_fn(caption("Shadow", theme.text_secondary))
            .build(),
    ));
    stack.add(Box::new(
        FrameBuilder::new(100.0, 100.0)
            .with_background(theme.accent)
            .with_corner_radius(10.0)
            .with_draw_fn(caption("Fill", Color::WHITE))
            .build(),
    ));
    stack.set_position(0.0, 2.0);
    stack.render(canvas);
}

fn draw_toolbar(canvas: &Canvas, theme: &Theme) {
    let leading = ToolbarGroup::new()
        .add(Button::icon("chevron-left").ghost())
        .add(Button::icon("chevron-right").ghost())
        .add_space(12.0)
        .add(
            Label::new("Documents")
                .with_style(styles::TITLE_3_EMPHASIZED)
                .with_color(theme.text_primary),
        )
        .build();
    let trailing = ToolbarGroup::new()
        .add(Button::icon("search").ghost())
        .add_separator()
        .add(Button::icon("settings").ghost())
        .build();
    Toolbar::new()
        .with_width(420.0)
        .with_background(theme.material_titlebar)
        .with_border_bottom(theme.fill_secondary)
        .with_leading(leading)
        .with_trailing(trailing)
        .render(canvas);
}

fn draw_scroll(canvas: &Canvas, theme: &Theme) {
    let viewport = Rect::from_wh(260.0, 150.0);
    let row_h = 30.0;
    let rows = 14;
    let mut state = ScrollState::new(viewport);
    state.set_content_length(row_h * rows as f32);
    state.set_offset(96.0);
    state.set_scrollbar_opacity(1.0);

    let mut ground = Paint::default();
    ground.set_anti_alias(true);
    ground.set_color(Color::WHITE);
    canvas.draw_rrect(RRect::new_rect_xy(viewport, 8.0, 8.0), &ground);
    canvas.save();
    canvas.clip_rrect(RRect::new_rect_xy(viewport, 8.0, 8.0), None, true);
    ScrollRenderer::draw(canvas, &state, theme, |canvas, visible| {
        let mut divider = Paint::default();
        divider.set_color(theme.fill_secondary);
        for i in 0..rows {
            let top = i as f32 * row_h;
            if top + row_h < visible.top || top > visible.bottom {
                continue;
            }
            Label::new(format!("Row {}", i + 1))
                .with_style(styles::BODY)
                .with_color(theme.text_primary)
                .centered_on(14.0, top + row_h / 2.0)
                .render(canvas);
            canvas.draw_line((14.0, top + row_h), (260.0, top + row_h), &divider);
        }
    });
    canvas.restore();
}

// --- Controls -------------------------------------------------------------

fn draw_button(canvas: &Canvas, _theme: &Theme) {
    let row = [
        Button::new("Primary").primary(),
        Button::new("Secondary").secondary(),
        Button::new("Outline").outline(),
        Button::new("Ghost").ghost(),
        Button::new("Danger").danger(),
    ];
    let mut x = 0.0;
    for button in row {
        let width = button.intrinsic_size().map_or(80.0, |(w, _)| w);
        button.at(x, 0.0).render(canvas);
        x += width + 8.0;
    }
    let row = [
        Button::new("Hovered").with_state(ButtonState::Hovered),
        Button::new("Pressed").with_state(ButtonState::Pressed),
        Button::new("Disabled").disabled(),
        Button::new("Upload").with_icon("upload").secondary(),
        Button::icon("heart").outline(),
    ];
    let mut x = 0.0;
    for button in row {
        let width = button.intrinsic_size().map_or(80.0, |(w, _)| w);
        button.at(x, 50.0).render(canvas);
        x += width + 8.0;
    }
}

fn draw_push_button(canvas: &Canvas, theme: &Theme) {
    let cancel = push_button::width("Cancel");
    push_button::draw(
        canvas,
        Rect::from_xywh(0.0, 0.0, cancel, push_button::HEIGHT),
        "Cancel",
        PushButton::new(Role::Normal),
        theme,
    );
    let save = push_button::width("Save");
    push_button::draw(
        canvas,
        Rect::from_xywh(cancel + 10.0, 0.0, save, push_button::HEIGHT),
        "Save",
        PushButton::new(Role::Default),
        theme,
    );
}

fn draw_toggle(canvas: &Canvas, theme: &Theme) {
    for (i, on) in [true, false].into_iter().enumerate() {
        toggle::draw(
            canvas,
            Rect::from_xywh(i as f32 * 56.0, 0.0, toggle::WIDTH, toggle::HEIGHT),
            toggle::knob_fraction_for(on),
            ToggleInteraction::Normal,
            theme,
        );
    }
}

fn draw_slider(canvas: &Canvas, theme: &Theme) {
    slider::draw(
        canvas,
        Rect::from_xywh(0.0, 0.0, 180.0, 24.0),
        48.0,
        0.0,
        100.0,
        Some("48%"),
        SliderInteraction::Normal,
        theme,
    );
}

fn draw_segmented(canvas: &Canvas, theme: &Theme) {
    let text = [
        Segment::Text("Light"),
        Segment::Text("Dark"),
        Segment::Text("Auto"),
    ];
    segmented::draw(
        canvas,
        Rect::from_xywh(0.0, 0.0, 180.0, segmented::HEIGHT),
        &text,
        Some(1),
        SegmentedInteraction::Normal,
        theme,
    );
    let glyphs = [
        "otto-view-grid-symbolic",
        "otto-view-list-symbolic",
        "otto-view-columns-symbolic",
    ]
    .map(|name| Segment::Glyph {
        name,
        fallback: None,
    });
    segmented::draw(
        canvas,
        Rect::from_xywh(196.0, 0.0, 104.0, segmented::HEIGHT),
        &glyphs,
        Some(0),
        SegmentedInteraction::Normal,
        theme,
    );
}

fn draw_text_input(canvas: &Canvas, theme: &Theme) {
    let style = TextInputStyle::with_theme(theme.clone());
    let mut selected = TextInput::new("Holiday photos 2026", style.clone()).with_size(260.0, 26.0);
    selected.state.set_focused(true);
    selected.state.select_range(8..14);
    selected.render(canvas);

    let mut empty = TextInput::new("", style).with_size(260.0, 26.0);
    empty.state = empty.state.clone().with_placeholder("Search");
    canvas.save();
    canvas.translate((0.0, 38.0));
    empty.render(canvas);
    canvas.restore();
}

fn draw_selectable_text(canvas: &Canvas, theme: &Theme) {
    let facts = [
        ("Kind", "PNG image"),
        ("Size", "2.4 MB"),
        ("Where", "~/Pictures/Holiday"),
    ];
    let mut runs = Vec::new();
    for (i, (key, value)) in facts.iter().enumerate() {
        let cy = 10.0 + i as f32 * 20.0;
        runs.push(TextRun::at(
            *key,
            styles::SUBHEADLINE,
            theme.text_secondary,
            0.0,
            cy,
        ));
        runs.push(TextRun::at(
            *value,
            styles::SUBHEADLINE,
            theme.text_primary,
            70.0,
            cy,
        ));
    }
    // A drag from the middle of the first value down into the second one.
    let mut selection = TextSelection::new();
    let first = &runs[1];
    let second = &runs[3];
    selection.press(&runs, first.left + 30.0, first.cy, Instant::now());
    selection.drag(&runs, second.left + 24.0, second.cy);
    selection.release();
    selection.draw_highlight(canvas, &runs, theme.accent.with_a(70));
    for run in &runs {
        run.draw(canvas);
    }
}

fn draw_dropdown(canvas: &Canvas, theme: &Theme) {
    let width = 170.0;
    field::draw(
        canvas,
        Rect::from_xywh(0.0, 0.0, width, field::HEIGHT),
        "Automatic",
        DropdownInteraction::Normal,
        theme,
    );

    let left = 200.0;
    field::draw(
        canvas,
        Rect::from_xywh(left, 0.0, width + 30.0, field::HEIGHT),
        "Medium",
        DropdownInteraction::Open,
        theme,
    );
    // The menu a pop-up button opens, as `DropdownMenu` styles it: rows at the
    // field's text size, the chosen one marked in the trailing slot.
    let options = ["Small", "Medium", "Large", "Extra large"];
    let items: Vec<MenuItem> = options
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let item = MenuItem::action(*label);
            match i {
                1 => item.with_shortcut("\u{2713}"),
                2 => item.with_visual_state(VisualState::Hovered),
                _ => item,
            }
        })
        .collect();
    let style = ContextMenuStyle::default()
        .with_theme(theme.clone())
        .with_item_metrics(field::LABEL_STYLE.size, 26.0)
        .with_width(width + 30.0)
        .with_min_width(width + 30.0);
    let (w, h) = ContextMenuRenderer::measure_items(&items, &style);
    canvas.save();
    canvas.translate((left, field::HEIGHT + 4.0));
    menu_shadow(canvas, w, h, style.corner_radius);
    ContextMenuRenderer::render_depth(canvas, &items, Some(2), &style, w, h, 0.0);
    canvas.restore();
}

fn swatches() -> Vec<panel::Swatch> {
    vec![
        panel::Swatch::new("Blue", Color::from_rgb(0x0A, 0x84, 0xFF)),
        panel::Swatch::new("Purple", Color::from_rgb(0xBF, 0x5A, 0xF2)),
        panel::Swatch::new("Pink", Color::from_rgb(0xFF, 0x2D, 0x55)),
        panel::Swatch::new("Red", Color::from_rgb(0xFF, 0x3B, 0x30)),
        panel::Swatch::new("Orange", Color::from_rgb(0xFF, 0x95, 0x00)),
        panel::Swatch::new("Yellow", Color::from_rgb(0xFF, 0xCC, 0x00)),
        panel::Swatch::new("Green", Color::from_rgb(0x34, 0xC7, 0x59)),
        panel::Swatch::new("Graphite", Color::from_rgb(0x8E, 0x8E, 0x93)),
    ]
}

fn draw_color_picker(canvas: &Canvas, theme: &Theme) {
    let color = Color::from_rgb(0x2E, 0xB8, 0x5C);
    well::draw(
        canvas,
        Rect::from_xywh(0.0, 0.0, well::measure(color), well::HEIGHT),
        color,
        WellInteraction::Open,
        theme,
    );
    let swatches = swatches();
    let (w, h) = panel::panel_size(swatches.len());
    let rect = Rect::from_xywh(0.0, well::HEIGHT + 12.0, w, h);
    menu_shadow_at(canvas, rect, 10.0);
    panel::draw(
        canvas,
        rect,
        Mode::Hsv,
        color,
        &swatches,
        Some(6),
        Some(HexField::Hex),
        theme,
    );
}

// --- Collections ----------------------------------------------------------

fn list_rows() -> Vec<ListRow> {
    vec![
        ListRow::new("Wi-Fi"),
        ListRow::new("Bluetooth"),
        ListRow::new("Night light").with_detail("Warmer colours after sunset"),
    ]
}

fn draw_list(canvas: &Canvas, theme: &Theme) {
    let rows = list_rows();
    let layout = ListLayout::compute(&rows, true, 0.0, 0.0, 380.0);
    list::draw(
        canvas,
        &layout,
        &rows,
        Some("CONNECTIONS"),
        theme,
        list::default_card_background(false),
        |canvas, index, rect| {
            let slot = list::trailing_rect(rect, toggle::WIDTH);
            let slot = Rect::from_xywh(
                slot.left,
                slot.center_y() - toggle::HEIGHT / 2.0,
                toggle::WIDTH,
                toggle::HEIGHT,
            );
            toggle::draw(
                canvas,
                slot,
                toggle::knob_fraction_for(index != 1),
                ToggleInteraction::Normal,
                theme,
            );
        },
    );
}

fn source_items() -> Vec<(SourceListItem, &'static str)> {
    vec![
        (
            SourceListItem::new("Appearance"),
            "otto-pane-appearance-symbolic",
        ),
        (
            SourceListItem::new("Displays"),
            "otto-pane-monitor-symbolic",
        ),
        (SourceListItem::new("Dock"), "otto-pane-dock-symbolic"),
        (SourceListItem::new("Sound"), "otto-pane-sound-symbolic"),
        (
            SourceListItem::new("Keyboard"),
            "otto-pane-keyboard-symbolic",
        ),
    ]
}

fn draw_source_list(canvas: &Canvas, theme: &Theme) {
    let entries = source_items();
    let items: Vec<SourceListItem> = entries.iter().map(|(item, _)| item.clone()).collect();
    let layout = SourceListLayout::compute(items.len(), 0.0, 0.0, 200.0);
    source_list::draw(
        canvas,
        &layout,
        &items,
        Some(1),
        theme,
        |canvas, index, rect, tint| {
            draw_glyph(canvas, entries[index].1, rect, tint);
        },
    );
}

fn selection_items() -> Vec<SelectionListItem> {
    vec![
        SelectionListItem::new("Ana López").with_subtitle("Administrator"),
        SelectionListItem::new("Mira Okafor").with_subtitle("Standard"),
        SelectionListItem::new("Guest"),
    ]
}

fn draw_selection_list(canvas: &Canvas, theme: &Theme) {
    let items = selection_items();
    let bounds = Rect::from_wh(300.0, SelectionListLayout::height_for(items.len()));
    let layout = SelectionListLayout::compute(items.len(), bounds);
    let state = SelectionListState {
        selected: Some(0),
        can_add: true,
        can_remove: true,
        pressed: None,
    };
    selection_list::draw(
        canvas,
        &layout,
        &items,
        &state,
        theme,
        list::default_card_background(false),
        |canvas, index, rect| {
            avatar::draw(canvas, rect, None, &items[index].title);
        },
    );
}

fn launcher_items() -> Vec<Item> {
    let item = |title: &str, subtitle: Option<&str>, source: usize| Item {
        title: title.to_string(),
        subtitle: subtitle.map(str::to_string),
        icon: None,
        activity: None,
        checked: None,
        search_terms: Vec::new(),
        pill: None,
        origin: Origin { source, index: 0 },
    };
    vec![
        item("Files", Some("Browse your folders"), 0),
        item("Pictures", Some("~/Pictures"), 1),
        Item {
            activity: Some(Activity::Working),
            ..item("Tidy the downloads folder", Some("Agent session"), 2)
        },
        Item {
            activity: Some(Activity::Waiting),
            ..item("Find last year's invoices", Some("Agent session"), 2)
        },
    ]
}

fn draw_item_list(canvas: &Canvas, theme: &Theme) {
    let items = launcher_items();
    let refs: Vec<&Item> = items.iter().collect();
    let glyphs = ["folder", "folder-pictures"];
    let painter = |canvas: &Canvas, index: usize, (cx, cy): (f32, f32), side: f32, color| {
        if let Some(name) = glyphs.get(index) {
            let rect = Rect::from_xywh(cx - side / 2.0, cy - side / 2.0, side, side);
            draw_glyph(canvas, name, rect, color);
        }
    };
    let style = RowStyle {
        selected: Some(0),
        selection: Some(theme.accent),
        on_selection: Some(Color::WHITE),
        query: "fi",
        mark: Some(theme.accent),
        icon: Some(&painter),
    };
    let mut card = Paint::default();
    card.set_anti_alias(true);
    card.set_color(Color::WHITE);
    let height = rows::ROW_H * items.len() as f32;
    canvas.draw_rrect(
        RRect::new_rect_xy(Rect::from_wh(420.0, height), 12.0, 12.0),
        &card,
    );
    rows::paint_item_rows_styled(
        canvas,
        Rect::from_wh(420.0, height),
        &refs,
        &["App", "Folder", "Ask"],
        420.0,
        false,
        &RowIcons::default(),
        &style,
    );
}

// --- Menus ----------------------------------------------------------------

fn draw_menu_bar(canvas: &Canvas, _theme: &Theme) {
    let mut state = MenuBarState::new();
    for label in ["Files", "File", "Edit", "View", "Go", "Window"] {
        state.add_item(label);
    }
    state.set_active(Some(2));
    let style = MenuBarStyle {
        first_item_font_weight: Some(skia_safe::font_style::Weight::BOLD),
        ..MenuBarStyle::default()
    };
    MenuBarRenderer::render(canvas, &state, &style, 420.0);
}

fn menu_item_group(width: f32) -> MenuItemGroup {
    MenuItemGroup::new().with_width(width).items(vec![
        MenuItem::action("Open").with_shortcut("Ctrl+O"),
        MenuItem::action("Rename…")
            .with_shortcut("F2")
            .with_visual_state(VisualState::Hovered),
        MenuItem::action("Paste").with_shortcut("Ctrl+V").disabled(),
        MenuItem::separator(),
        MenuItem::action("Show hidden files")
            .with_checked(true)
            .with_check_gutter(),
    ])
}

fn draw_menu_item(canvas: &Canvas, _theme: &Theme) {
    let group = menu_item_group(240.0).with_style(MenuItemStyle::default());
    let mut ground = Paint::default();
    ground.set_anti_alias(true);
    ground.set_color(Color::WHITE);
    canvas.draw_rrect(
        RRect::new_rect_xy(Rect::from_wh(240.0, group.height()), 6.0, 6.0),
        &ground,
    );
    group.render(canvas);
}

fn menu_style(theme: &Theme) -> ContextMenuStyle {
    ContextMenuStyle::default().with_theme(theme.clone())
}

fn context_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Open"),
        MenuItem::action("Open With").with_shortcut("Ctrl+Enter"),
        MenuItem::separator(),
        MenuItem::action("Copy").with_shortcut("Ctrl+C"),
        MenuItem::action("Move to Trash").with_shortcut("Delete"),
        MenuItem::separator(),
        MenuItem::submenu(
            "Sort By",
            vec![
                MenuItem::action("Name").with_shortcut("\u{2713}"),
                MenuItem::action("Date Modified"),
                MenuItem::action("Size"),
                MenuItem::action("Kind"),
            ],
        ),
        MenuItem::action("Get Info").with_shortcut("Ctrl+I"),
    ]
}

/// The row whose submenu is shown open, its items and style, and how far
/// down the menu the submenu's top sits.
fn context_submenu(theme: &Theme) -> (usize, Vec<MenuItem>, ContextMenuStyle, f32) {
    let style = menu_style(theme);
    let items = context_items();
    let open = 6;
    let sub = items[open].submenu_items().unwrap_or(&[]).to_vec();
    let top: f32 = items[..open]
        .iter()
        .map(|item| style.item_height_of(item))
        .sum();
    (open, sub, menu_style(theme).with_min_width(150.0), top)
}

fn context_menu_height(theme: &Theme) -> f32 {
    let style = menu_style(theme);
    let (_, sub, sub_style, top) = context_submenu(theme);
    let main = ContextMenuRenderer::measure_items(&context_items(), &style).1;
    main.max(top + ContextMenuRenderer::measure_items(&sub, &sub_style).1)
}

fn draw_context_menu(canvas: &Canvas, theme: &Theme) {
    let style = menu_style(theme);
    let items = context_items();
    let (w, h) = ContextMenuRenderer::measure_items(&items, &style);
    menu_shadow(canvas, w, h, style.corner_radius);
    // The submenu row is the highlighted one, its submenu open beside it.
    let (open, sub, sub_style, top) = context_submenu(theme);
    ContextMenuRenderer::render_depth(canvas, &items, Some(open), &style, w, h, 0.0);

    let (sw, sh) = ContextMenuRenderer::measure_items(&sub, &sub_style);
    canvas.save();
    canvas.translate((w - 4.0, top));
    menu_shadow(canvas, sw, sh, sub_style.corner_radius);
    ContextMenuRenderer::render_depth(canvas, &sub, Some(1), &sub_style, sw, sh, 0.0);
    canvas.restore();
}

// --- Window chrome --------------------------------------------------------

fn draw_titlebar(canvas: &Canvas, _theme: &Theme) {
    let bars = [
        WindowDecoration::new("Documents", 440.0).with_corner_radius(12.0),
        WindowDecoration::new("Documents", 440.0)
            .with_corner_radius(12.0)
            .with_active(false),
        WindowDecoration::new("Call with Ana", 440.0)
            .with_corner_radius(12.0)
            .with_sharing(true),
    ];
    let mut y = 0.0;
    for bar in bars {
        canvas.save();
        canvas.translate((0.0, y));
        bar.draw(canvas);
        canvas.restore();
        y += bar.titlebar_height + 16.0;
    }
}

// --- Ask and stash --------------------------------------------------------

fn attachment_options(width: f32) -> Options {
    Options {
        width,
        newest_first: false,
        removable: true,
    }
}

/// A made-up picture written to `name` in the temporary directory: a sun
/// over a horizon in `sky`, with a pale card standing in for a window.
fn made_up_picture(name: &str, (width, height): (i32, i32), sky: Color) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    let (w, h) = (width as f32, height as f32);
    let mut surface = surfaces::raster_n32_premul((width, height)).expect("raster surface");
    let canvas = surface.canvas();
    canvas.clear(sky);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_rgb(0xF2, 0xB8, 0x5A));
    canvas.draw_circle((w * 0.77, h * 0.33), h * 0.18, &paint);
    paint.set_color(Color::from_rgb(0x1E, 0x33, 0x4C));
    canvas.draw_rect(Rect::from_xywh(0.0, h * 0.7, w, h * 0.3), &paint);
    paint.set_color(Color::from_rgb(0xE8, 0xED, 0xF2));
    canvas.draw_rrect(
        RRect::new_rect_xy(
            Rect::from_xywh(w * 0.07, h * 0.17, w * 0.4, h * 0.62),
            12.0,
            12.0,
        ),
        &paint,
    );
    let data = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&path, data.as_bytes()).expect("write made-up picture");
    path
}

fn attachment_items() -> Vec<Attachment> {
    vec![
        Attachment::Text("Every component draws from a bare canvas.".to_string()),
        Attachment::Region(made_up_picture(
            "otto-component-library-region.png",
            (560, 240),
            Color::from_rgb(0x2F, 0x4A, 0x6B),
        )),
        Attachment::File(made_up_picture(
            "Harbour at dusk.png",
            (320, 240),
            Color::from_rgb(0x6B, 0x4A, 0x6F),
        )),
    ]
}

/// Lay `items` out, handing in each picture file's thumbnail as Files does:
/// the list asks, the app decodes, and the next layout shows it.
fn attachment_layout(items: &[Attachment], width: f32, theme: &Theme) -> (AttachmentList, Layout) {
    let mut list = AttachmentList::default();
    let refs: Vec<(&Attachment, bool)> = items.iter().map(|item| (item, false)).collect();
    list.layout(&refs, attachment_options(width), theme);
    for path in list.thumbnails_wanted() {
        let image = std::fs::read(&path)
            .ok()
            .and_then(|bytes| skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes)));
        list.set_thumbnail(&path, image);
    }
    let layout = list.layout(&refs, attachment_options(width), theme);
    (list, layout)
}

fn draw_attachments(canvas: &Canvas, theme: &Theme) {
    let (list, layout) = attachment_layout(&attachment_items(), 300.0, theme);
    list.paint(canvas, &layout, theme, None);
}

// --- Helpers --------------------------------------------------------------

/// Draw the Solar glyph `name` into `rect`, tinted `color`.
fn draw_glyph(canvas: &Canvas, name: &str, rect: Rect, color: Color) {
    let px = (rect.width() * SCALE).round() as i32;
    if let Some(image) = solar_icon(name, px, SolarStyle::Linear, color) {
        canvas.draw_image_rect(&image, None, rect, &Paint::default());
    }
}

/// The soft shadow the compositor gives a popup, which a bare canvas lacks.
fn menu_shadow(canvas: &Canvas, width: f32, height: f32, radius: f32) {
    menu_shadow_at(canvas, Rect::from_wh(width, height), radius);
}

fn menu_shadow_at(canvas: &Canvas, rect: Rect, radius: f32) {
    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_color(Color::from_argb(46, 0, 0, 0));
    shadow.set_mask_filter(skia_safe::MaskFilter::blur(
        skia_safe::BlurStyle::Normal,
        6.0,
        false,
    ));
    canvas.draw_rrect(
        RRect::new_rect_xy(rect.with_offset((0.0, 3.0)), radius, radius),
        &shadow,
    );
}
