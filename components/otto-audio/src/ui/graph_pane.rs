//! The Graph section's chrome: the toolbar, the service banner and the
//! details panel. The canvas itself is `graph_view`.

use super::*;

impl Ui {
    pub(super) fn draw_toolbar(&mut self, canvas: &Canvas, scene: &Scene, theme: &Theme) {
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

        // The view switch: patchbay, matrix or lanes.
        let labels: Vec<String> = GraphMode::ALL
            .iter()
            .map(|mode| mode_label(*mode).to_string())
            .collect();
        let font = styles::CALLOUT.font();
        let segment_w = labels
            .iter()
            .map(|l| font.measure_str(l, None).0 + 24.0)
            .fold(0.0, f32::max);
        let track = Rect::from_xywh(
            x,
            cy - segmented::HEIGHT / 2.0,
            segment_w * labels.len() as f32,
            segmented::HEIGHT,
        );
        let chosen = GraphMode::ALL
            .iter()
            .position(|m| *m == self.view.mode)
            .unwrap_or(0);
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &GraphMode::ALL.map(Action::GraphMode),
            theme,
        );
        x = track.right + 14.0;

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
        if self.view.mode == GraphMode::Patchbay {
            put(self, t!("audio-fit"), Action::Fit, true, ButtonRole::Normal);
        }
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

        // What solid and dashed wires mean, when there is room for it; the
        // matrix marks a cell with a filled dot or a ring instead.
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
            let dots = self.view.mode == GraphMode::Matrix;
            for ((label, saved), w) in entries.iter().zip(&widths) {
                if dots {
                    let c = Point::new(lx + 14.0, cy);
                    if *saved {
                        canvas.draw_circle(c, 5.0, &fill(color));
                    } else {
                        canvas.draw_circle(c, 4.5, &stroke(color, 2.0));
                    }
                } else {
                    let mut paint = stroke(color, 2.0);
                    if !saved {
                        paint.set_path_effect(otto_kit::skia::PathEffect::dash(&[4.0, 3.0], 0.0));
                    }
                    canvas.draw_line(Point::new(lx, cy), Point::new(lx + 20.0, cy), &paint);
                }
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

    pub(super) fn draw_banner(&mut self, canvas: &Canvas, theme: &Theme) {
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

    pub(super) fn draw_details(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        scene: &Scene,
        theme: &Theme,
    ) {
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
        if node.is_airplay() {
            facts.push((
                t!("audio-detail-delay"),
                t!("audio-airplay-delay-short").to_string(),
            ));
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
}

/// The view switch's label for `mode`.
fn mode_label(mode: GraphMode) -> &'static str {
    match mode {
        GraphMode::Patchbay => t!("audio-view-patchbay"),
        GraphMode::Matrix => t!("audio-view-matrix"),
        GraphMode::Lanes => t!("audio-view-lanes"),
    }
}
