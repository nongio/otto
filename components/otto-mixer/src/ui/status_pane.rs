//! The Status section: what is worth a look, each with a fix, then what is
//! playing now, app to effects to device, and a few facts about the sound
//! server.
//!
//! Things worth a look: the audio service being off; a rule or a moved
//! stream pointing at a device that is not there (fix: follow the default);
//! dropouts on a driver in the last ten minutes (fix: the next larger
//! buffer); and an equaliser boosting past 0 dB, which clips (fix: lower its
//! preamp by the boost). Dropouts come from PipeWire's profiler, which the
//! window follows while it is open, so they count from when it opened.

// Rust guideline compliant 2026-02-21

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use otto_audio_graph::{AirPlay, Endpoint, Stream};

use super::effects_pane::draw_arrow;
use super::mixer_pane::draw_card;
use super::*;

/// How far back dropouts count.
const DROPOUT_WINDOW: Duration = Duration::from_secs(10 * 60);
/// A boost under this, in dB, is rounding, not clipping.
const BOOST_SLACK: f32 = 0.05;
/// The section's margin, and the room between its cards.
const PAD: f32 = 28.0;
const GAP: f32 = 16.0;
const HEADER_H: f32 = 72.0;
const HEADING_H: f32 = 34.0;
/// One item worth a look, one playing chain, one fact.
const ISSUE_H: f32 = 56.0;
const CHAIN_H: f32 = 40.0;
const FACT_ROW_H: f32 = 32.0;
const STEP_H: f32 = 22.0;
/// The facts' label column.
const FACT_LABEL_W: f32 = 200.0;

/// What a control of this section does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusAction {
    /// The fix of the item at this index of [`Ui::status_issues`].
    Fix(usize),
}

/// What an item's fix does.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Fix {
    StartService,
    /// Have the app's rule follow the default.
    RuleToDefault(String),
    /// Have the stream follow the default.
    StreamToDefault(u32),
    /// Run the engine at this quantum.
    Quantum(u32),
    /// Lower the effect's preamp by this many dB.
    LowerPreamp {
        effect: String,
        by: f32,
    },
}

/// One thing worth a look.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Issue {
    pub(super) text: String,
    /// A few words naming it, for the sidebar's status line.
    pub(super) short: String,
    fix_label: String,
    fix: Fix,
}

/// The dropout counts the window has seen, oldest first.
#[derive(Debug, Default)]
pub(super) struct DropoutLog {
    samples: VecDeque<(Instant, u32, u32)>,
}

impl DropoutLog {
    /// Driver `driver` has had `count` dropouts since it started.
    pub(super) fn record(&mut self, driver: u32, count: u32, now: Instant) {
        self.samples.push_back((now, driver, count));
        // Keep one sample per driver older than the window: the baseline.
        let cutoff = now.checked_sub(DROPOUT_WINDOW).unwrap_or(now);
        while self.samples.len() > 1 {
            let (at, oldest_driver, _) = self.samples[0];
            let newer_old = self
                .samples
                .iter()
                .skip(1)
                .any(|(t, d, _)| *d == oldest_driver && *t <= cutoff);
            if at <= cutoff && newer_old {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Driver `driver`'s dropouts in the last ten minutes, as far as the
    /// window has seen.
    pub(super) fn recent(&self, driver: u32, now: Instant) -> u32 {
        let cutoff = now.checked_sub(DROPOUT_WINDOW).unwrap_or(now);
        let mine: Vec<&(Instant, u32, u32)> = self
            .samples
            .iter()
            .filter(|(_, d, _)| *d == driver)
            .collect();
        let (Some(first), Some(last)) = (mine.first(), mine.last()) else {
            return 0;
        };
        let baseline = mine
            .iter()
            .rev()
            .find(|(t, _, _)| *t <= cutoff)
            .unwrap_or(first);
        last.2.saturating_sub(baseline.2)
    }
}

impl Ui {
    /// What is worth a look now, in the order shown.
    pub(super) fn status_issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        if self.service_down() {
            issues.push(Issue {
                text: t!("audio-status-service-off").to_string(),
                short: t!("audio-status-line-off").to_string(),
                fix_label: t!("audio-service-start").to_string(),
                fix: Fix::StartService,
            });
        }
        for rule in self.routing.rules() {
            let Some(output) = rule.output.as_deref() else {
                continue;
            };
            if self.graph.nodes().any(|n| n.name() == output) {
                continue;
            }
            issues.push(Issue {
                text: t_owned!(
                    "audio-status-rule-missing",
                    app = rule.app.clone(),
                    device = output.to_string()
                ),
                short: t_owned!(
                    "audio-status-short-device-missing",
                    device = output.to_string()
                ),
                fix_label: t!("audio-mixer-follow-default").to_string(),
                fix: Fix::RuleToDefault(rule.app.clone()),
            });
        }
        for stream in self.graph.streams(Endpoint::Output) {
            let Some(target) = stream.target.as_deref() else {
                continue;
            };
            if self.graph.node_targeted(target).is_some() {
                continue;
            }
            issues.push(Issue {
                text: t_owned!("audio-status-stream-missing", app = stream.app.clone()),
                short: t_owned!("audio-status-short-no-output", app = stream.app.clone()),
                fix_label: t!("audio-mixer-follow-default").to_string(),
                fix: Fix::StreamToDefault(stream.id),
            });
        }
        let now = Instant::now();
        let quantum = self.engine_now().frames;
        let next = QUANTA.iter().copied().find(|q| *q > quantum);
        for (driver, name, _) in self.graph.dropouts() {
            let recent = self.dropout_log.recent(driver, now);
            if recent == 0 {
                continue;
            }
            let title = self
                .graph
                .node(driver)
                .map_or_else(|| name.to_string(), |n| n.title().to_string());
            let text = t_owned!(
                "audio-status-dropouts",
                count = recent,
                device = title.clone()
            );
            let short = t_owned!("audio-status-short-dropouts", device = title);
            let (fix_label, fix) = match next {
                Some(next) => (
                    t_owned!("audio-status-raise-buffer", frames = next),
                    Fix::Quantum(next),
                ),
                None => continue,
            };
            issues.push(Issue {
                text,
                short,
                fix_label,
                fix,
            });
        }
        for effect in self.routing.effects() {
            if !effect.is_placed() || !effect.enabled {
                continue;
            }
            let Some(peak) = effect.peak_gain_db() else {
                continue;
            };
            if peak <= BOOST_SLACK {
                continue;
            }
            // Down to a half dB step, so the curve ends at or under 0 dB.
            let by = (peak * 2.0).ceil() / 2.0;
            issues.push(Issue {
                text: t_owned!(
                    "audio-status-boost",
                    name = effect.name.clone(),
                    db = format!("{peak:.1}")
                ),
                short: t_owned!("audio-status-short-clipping", name = effect.name.clone()),
                fix_label: t_owned!("audio-status-lower-preamp", db = format!("{by:.1}")),
                fix: Fix::LowerPreamp {
                    effect: effect.id.clone(),
                    by,
                },
            });
        }
        issues
    }

    fn status_viewport(&self) -> Rect {
        let content = self.content();
        Rect::from_ltrb(
            content.left,
            content.top + HEADER_H,
            content.right,
            content.bottom,
        )
    }

    pub(super) fn draw_status(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        canvas.draw_rect(content, &fill(graph_view::canvas_ground(theme)));
        let left = content.left + PAD;
        let right = content.right - PAD;
        let width = (right - left).max(1.0);
        let top = content.top + 24.0;
        Label::new(t!("audio-status-title"))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, top + 12.0)
            .render(canvas);
        let intro = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&intro, t!("audio-status-intro"), width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, top + 38.0)
            .render(canvas);

        let issues = self.status_issues();
        let streams = self.graph.streams(Endpoint::Output);
        let facts = self.status_facts();
        let issues_h = issues.len().max(1) as f32 * ISSUE_H + 8.0;
        let chains_h = streams.len().max(1) as f32 * CHAIN_H + 16.0;
        let facts_h = facts.len() as f32 * FACT_ROW_H + 16.0;
        let viewport = self.status_viewport();
        self.status_scroll.set_viewport(viewport);
        self.status_scroll.set_content_length(
            4.0 + 3.0 * (HEADING_H + GAP) + issues_h + chains_h + facts_h + PAD,
        );
        self.status_scroll.tick();
        let mut y = viewport.top - self.status_scroll.offset() + 4.0;
        let first_target = self.targets.len();
        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));

        heading(canvas, t!("audio-status-worth-a-look"), left, y, theme);
        y += HEADING_H;
        let card = Rect::from_xywh(left, y, width, issues_h);
        draw_card(canvas, card, theme);
        if issues.is_empty() {
            caption(
                canvas,
                t!("audio-status-nothing"),
                card.left + 18.0,
                card.top + 4.0 + ISSUE_H / 2.0,
                theme,
            );
        }
        for (index, issue) in issues.iter().enumerate() {
            let row_top = card.top + 4.0 + index as f32 * ISSUE_H;
            let cy = row_top + ISSUE_H / 2.0;
            if index > 0 {
                canvas.draw_line(
                    Point::new(card.left + 18.0, row_top),
                    Point::new(card.right - 18.0, row_top),
                    &stroke(theme.hairline, 1.0),
                );
            }
            canvas.draw_circle(
                Point::new(card.left + 24.0, cy),
                4.0,
                &fill(theme.accent_orange),
            );
            let w = push_button::width(&issue.fix_label);
            let button = Rect::from_xywh(
                card.right - 18.0 - w,
                cy - push_button::HEIGHT / 2.0,
                w,
                push_button::HEIGHT,
            );
            let body = styles::BODY.font();
            Label::new(ellipsize(
                &body,
                &issue.text,
                (button.left - card.left - 52.0).max(0.0),
            ))
            .with_style(styles::BODY)
            .with_color(theme.text_primary)
            .centered_on(card.left + 38.0, cy)
            .render(canvas);
            let label = issue.fix_label.clone();
            self.button(
                canvas,
                button,
                &label,
                Action::Status(StatusAction::Fix(index)),
                true,
                theme,
            );
        }
        y += issues_h + GAP;

        heading(canvas, t!("audio-status-playing"), left, y, theme);
        y += HEADING_H;
        let card = Rect::from_xywh(left, y, width, chains_h);
        draw_card(canvas, card, theme);
        if streams.is_empty() {
            caption(
                canvas,
                t!("audio-mixer-nothing-playing"),
                card.left + 18.0,
                card.top + 8.0 + CHAIN_H / 2.0,
                theme,
            );
        }
        for (index, stream) in streams.iter().enumerate() {
            let cy = card.top + 8.0 + index as f32 * CHAIN_H + CHAIN_H / 2.0;
            self.draw_playing_chain(
                canvas,
                Point::new(card.left + 18.0, cy),
                card.right - 18.0,
                stream,
                theme,
            );
        }
        y += chains_h + GAP;

        heading(canvas, t!("audio-status-facts"), left, y, theme);
        y += HEADING_H;
        let card = Rect::from_xywh(left, y, width, facts_h);
        draw_card(canvas, card, theme);
        let body = styles::BODY.font();
        for (index, (label, value)) in facts.iter().enumerate() {
            let cy = card.top + 8.0 + index as f32 * FACT_ROW_H + FACT_ROW_H / 2.0;
            Label::new(ellipsize(&body, label, FACT_LABEL_W - 12.0))
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .centered_on(card.left + 18.0, cy)
                .render(canvas);
            Label::new(ellipsize(
                &body,
                value,
                (card.width() - FACT_LABEL_W - 36.0).max(0.0),
            ))
            .with_style(styles::BODY)
            .with_color(theme.text_primary)
            .centered_on(card.left + 18.0 + FACT_LABEL_W, cy)
            .render(canvas);
        }
        canvas.restore();
        let drawn: Vec<(Rect, Action)> = self.targets.drain(first_target..).collect();
        for (rect, action) in drawn {
            let mut visible = rect;
            if visible.intersect(viewport) {
                self.targets.push((visible, action));
            }
        }
        self.status_scroll.render(canvas, theme, |_, _| {});
    }

    /// A stream's path as chips: the app, the effects it passes, the device.
    fn draw_playing_chain(
        &self,
        canvas: &Canvas,
        start: Point,
        right: f32,
        stream: &Stream,
        theme: &Theme,
    ) {
        let mut steps: Vec<(String, bool)> = vec![(stream.app.clone(), false)];
        let mut at = stream
            .linked
            .iter()
            .find_map(|id| self.graph.node(*id))
            .map(|n| n.name().to_string());
        // Bounded, so a hand-edited file that loops cannot hang the window.
        for _ in 0..8 {
            let Some(name) = at.clone() else {
                break;
            };
            match self
                .routing
                .effects()
                .iter()
                .find(|e| e.node_name() == name)
            {
                Some(effect) => {
                    steps.push((effect.name.clone(), true));
                    at = Some(effect.device.clone());
                }
                None => {
                    let title = self
                        .graph
                        .nodes()
                        .find(|n| n.name() == name)
                        .map_or(name, |n| n.title().to_string());
                    steps.push((title, false));
                    break;
                }
            }
        }
        if steps.len() == 1 {
            steps.push((t!("audio-status-not-playing-anywhere").to_string(), false));
        }
        let font = styles::SUBHEADLINE.font();
        let mut x = start.x;
        for (index, (text, effect)) in steps.iter().enumerate() {
            if index > 0 {
                if x + 26.0 > right {
                    break;
                }
                draw_arrow(canvas, Point::new(x + 4.0, start.y), x + 22.0, theme);
                x += 26.0;
            }
            let text = ellipsize(&font, text, 200.0);
            let w = font.measure_str(&text, None).0 + 16.0;
            if x + w > right {
                break;
            }
            let chip = Rect::from_xywh(x, start.y - STEP_H / 2.0, w, STEP_H);
            let a = theme.accent;
            let ground = if *effect {
                Color::from_argb(0x30, a.r(), a.g(), a.b())
            } else {
                theme.fill_quaternary
            };
            canvas.draw_rrect(RRect::new_rect_xy(chip, 6.0, 6.0), &fill(ground));
            Label::new(text)
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_primary)
                .centered_at(chip.center_x(), chip.center_y())
                .render(canvas);
            x += w;
        }
    }

    /// The facts: dropouts, delay, the service, AirPlay.
    fn status_facts(&self) -> Vec<(String, String)> {
        let now = Instant::now();
        let dropouts: u32 = self
            .graph
            .dropouts()
            .map(|(driver, _, _)| self.dropout_log.recent(driver, now))
            .sum();
        let engine = self.engine_now();
        let service = if self.service_down() {
            t!("audio-status-service-is-off")
        } else {
            t!("audio-status-service-is-on")
        };
        let airplay = if !self.routing.airplay {
            t!("audio-status-airplay-off").to_string()
        } else if let Some(missing) = self.airplay_missing {
            match missing {
                otto_audio_graph::AirPlayMissing::Module => {
                    t!("audio-status-airplay-no-module").to_string()
                }
                otto_audio_graph::AirPlayMissing::Avahi => {
                    t!("audio-status-airplay-no-avahi").to_string()
                }
            }
        } else if self.graph.airplay() == AirPlay::Off && self.service_down() {
            t!("audio-airplay-service-down").to_string()
        } else {
            t_owned!(
                "audio-status-airplay-found",
                count = self.graph.airplay_sinks().count()
            )
        };
        vec![
            (
                t!("audio-status-fact-dropouts").to_string(),
                t_owned!("audio-status-dropouts-count", count = dropouts),
            ),
            (
                t!("audio-status-fact-delay").to_string(),
                t_owned!(
                    "audio-status-delay",
                    ms = format!("{:.1}", engine.latency_ms()),
                    frames = engine.frames,
                    rate = khz(engine.rate)
                ),
            ),
            (
                t!("audio-status-fact-service").to_string(),
                service.to_string(),
            ),
            (t!("audio-status-fact-airplay").to_string(), airplay),
        ]
    }

    pub(super) fn scroll_status(&mut self, p: Point, dy: f32, discrete: bool, stop: bool) -> bool {
        if !self.status_viewport().contains(p) {
            return false;
        }
        if stop {
            self.status_scroll.on_wheel_end();
            return true;
        }
        if discrete {
            self.status_scroll.on_wheel_discrete(dy)
        } else {
            self.status_scroll.on_wheel(dy)
        }
    }

    pub(super) fn run_status(&mut self, action: StatusAction) {
        let StatusAction::Fix(index) = action;
        let Some(issue) = self.status_issues().into_iter().nth(index) else {
            return;
        };
        match issue.fix {
            Fix::StartService => self.run(Action::StartService),
            Fix::RuleToDefault(app) => {
                if self.routing.set_rule(&app, None) {
                    self.save_routing_file();
                }
            }
            Fix::StreamToDefault(stream) => self.connection.set_target(stream, None),
            Fix::Quantum(quantum) => self.run(Action::Quantum(quantum)),
            Fix::LowerPreamp { effect, by } => {
                self.change_effect(&effect, false, |e| e.preamp -= by);
            }
        }
    }
}

fn heading(canvas: &Canvas, text: &str, x: f32, top: f32, theme: &Theme) {
    Label::new(text)
        .with_style(styles::HEADLINE_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(x, top + HEADING_H / 2.0 - 2.0)
        .render(canvas);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropouts_count_from_the_window_s_first_sight_and_age_out() {
        let start = Instant::now();
        let mut log = DropoutLog::default();
        // Seen first at 5: those happened before the window watched.
        log.record(1, 5, start);
        assert_eq!(log.recent(1, start), 0);
        log.record(1, 8, start + Duration::from_secs(60));
        assert_eq!(log.recent(1, start + Duration::from_secs(60)), 3);
        assert_eq!(log.recent(2, start), 0);
        // Eleven minutes on, the first three are older than ten minutes.
        let later = start + Duration::from_secs(11 * 60 + 60);
        log.record(1, 9, later);
        assert_eq!(log.recent(1, later), 1);
    }
}
