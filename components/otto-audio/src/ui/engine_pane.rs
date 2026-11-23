//! The Engine section: latency now, the sample rate and the buffer size.

use super::*;

impl Ui {
    pub(super) fn draw_engine(&mut self, canvas: &Canvas, theme: &Theme) {
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
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, card.top + 98.0)
            .render(canvas);

        // Who chose each: the engine, or this app.
        let column_w = (card.width() - 40.0) / 2.0;
        let font = styles::SUBHEADLINE.font();
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
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(x, card.top + 132.0)
                .render(canvas);
            Label::new(chosen)
                .with_style(styles::SUBHEADLINE_EMPHASIZED)
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
        let font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&font, &value, width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(left, card.top + 104.0)
            .render(canvas);
        let caption_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(
            &caption_font,
            t!("audio-engine-rate-note"),
            width,
        ))
        .with_style(styles::SUBHEADLINE)
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
                theme.fill_primary
            } else {
                theme.control_track
            };
            canvas.draw_rrect(rr, &fill(ground));
            let cx = tile.center_x();
            let cy = tile.center_y();
            if *quantum == 0 {
                Label::new(t!("audio-engine-auto"))
                    .with_style(styles::SUBHEADLINE_EMPHASIZED)
                    .with_color(text)
                    .centered_at(cx, cy)
                    .render(canvas);
            } else {
                let ms = f64::from(*quantum) * 1000.0 / f64::from(now.rate.max(1));
                Label::new(quantum.to_string())
                    .with_style(styles::SUBHEADLINE_EMPHASIZED)
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
        let font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&font, &range, width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, card.top + 158.0)
            .render(canvas);
    }
}
