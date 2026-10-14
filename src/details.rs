//! The usage details the panel expands with when double-clicked: each limit and where it's
//! headed, what the week's usage went to, the account, the Claude Code sessions running now,
//! and what Claude Code did this week on this PC.

use crate::claude::{self, Extra, Limit, Pace, Session, State};
use crate::mascot::CLAUDE_ORANGE;
use crate::stats::{Stats, Tally, Tokens};
use crate::view::{self, AMBER, DIM, RED, TEXT, TRACK};
use crate::widgets::{self, Fonts};
use chrono::{DateTime, Local, NaiveDate, Utc};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Label, Rect, Response, RichText, Sense, Stroke, Ui, pos2, vec2,
};
use std::sync::Arc;

/// What the week went to, colored in the order the API lists it: a set checked to stay apart
/// for color-blind eyes on the card. "Other" is gray. Tokens per day are drawn in the first.
pub(crate) const PARTS: [Color32; 4] = [
    Color32::from_rgb(0x39, 0x87, 0xe5),
    Color32::from_rgb(0xd9, 0x59, 0x26),
    Color32::from_rgb(0x19, 0x9e, 0x70),
    Color32::from_rgb(0xc9, 0x85, 0x00),
];
const OTHER: Color32 = Color32::from_rgb(110, 110, 118);
const BODY: f32 = 13.0;
const SMALL: f32 = 11.5;
/// Projects listed by name; the rest are counted.
const TOP_PROJECTS: usize = 5;

pub fn ui(ui: &mut Ui, fonts: &Fonts, state: &State, stats: Option<&Stats>) {
    let now = Utc::now();
    let updated = state.updated.map(|t| t.format("Updated %H:%M").to_string());
    section(ui, fonts, "Limits", updated.as_deref(), |ui| limits(ui, fonts, state, now));
    if let Some(breakdown) = &state.breakdown {
        let since = breakdown.since.map(|t| format!("Since {}", t.with_timezone(&Local).format("%a %-d %b")));
        section(ui, fonts, "This week by app", since.as_deref(), |ui| parts(ui, &breakdown.parts));
    }
    section(ui, fonts, "Account", None, |ui| account(ui, state));
    section(ui, fonts, "Running now", None, |ui| sessions(ui, &state.sessions, now));
    claude_code(ui, fonts, stats);
}

/// A heading, with an optional note on its right, over a card.
fn section(ui: &mut Ui, fonts: &Fonts, title: &str, aside: Option<&str>, add_rows: impl FnOnce(&mut Ui)) {
    ui.add_space(16.0);
    widgets::heading(ui, fonts, title, |ui| {
        if let Some(aside) = aside {
            ui.label(RichText::new(aside).size(SMALL).color(DIM));
        }
    });
    ui.add_space(4.0);
    widgets::card(ui, add_rows);
}

fn limits(ui: &mut Ui, fonts: &Fonts, state: &State, now: DateTime<Utc>) {
    if state.limits.is_empty() {
        note(ui, state.error.as_deref().unwrap_or("Loading..."));
        return;
    }
    for (i, limit) in state.limits.iter().enumerate() {
        if i > 0 {
            widgets::rule(ui);
        }
        ui.add_space(10.0);
        let percent = strong(ui, fonts, format!("{:.0}%", limit.percent), BODY);
        let label = text(ui, &limit.label, BODY, TEXT);
        split(ui, 0.0, label, Some(percent), 18.0);
        ui.add_space(6.0);
        meter(ui, limit, now);
        ui.add_space(6.0);
        for (line, color) in limit_notes(limit, now) {
            ui.add(Label::new(RichText::new(line).size(SMALL).color(color)).wrap());
        }
        ui.add_space(10.0);
    }
}

/// The limit's bar, with a tick for how much of its window has passed.
fn meter(ui: &mut Ui, limit: &Limit, now: DateTime<Utc>) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 6.0));
    let painter = ui.painter();
    painter.rect_filled(track, 3, TRACK);
    let fraction = (limit.percent / 100.0).clamp(0.0, 1.0);
    if fraction > 0.0 {
        let mut fill = track;
        fill.set_width((track.width() * fraction).max(track.height()));
        painter.rect_filled(fill, 3, view::level(limit.percent));
    }
    if let Some(elapsed) = limit.elapsed(now) {
        let x = track.left() + track.width() * elapsed;
        // Ringed in the card's color to stand out from the bar under it.
        painter.vline(x, rect.y_range(), Stroke::new(4.0, widgets::CARD));
        painter.vline(x, rect.y_range(), Stroke::new(2.0, TEXT));
        response.on_hover_text(format!("The tick is now: {:.0}% of this window has passed", elapsed * 100.0));
    }
}

/// When the limit resets, what share of the weekly limit a model's limit is, and where it's
/// headed.
fn limit_notes(limit: &Limit, now: DateTime<Utc>) -> Vec<(String, Color32)> {
    let share = limit.share.map(|(used, allowed)| {
        (format!("{used:.0}% of your Weekly limit so far \u{B7} capped at {allowed:.0}% of it"), DIM)
    });
    reset_note(limit, now).into_iter().chain(share).chain(pace_note(limit, now)).collect()
}

/// When the limit resets and how long until then, or that its window hasn't started.
pub(crate) fn reset_note(limit: &Limit, now: DateTime<Utc>) -> Option<(String, Color32)> {
    match limit.resets_at {
        Some(t) => Some((format!("Resets {} \u{B7} in {}", local(t), claude::until(t, now)), DIM)),
        None if limit.window.is_some() => Some(("Not started yet".to_owned(), DIM)),
        None => None,
    }
}

/// Where the limit is headed at the pace it's going, in amber or red when that's trouble.
pub(crate) fn pace_note(limit: &Limit, now: DateTime<Utc>) -> Option<(String, Color32)> {
    match limit.pace(now)? {
        Pace::Reached => Some(("Limit reached".to_owned(), RED)),
        Pace::Finish(end) => Some((format!("On pace to end at about {end:.0}%"), DIM)),
        Pace::RunsOut(at) => Some((format!("At this pace, runs out {}", local(at)), AMBER)),
    }
}

/// What the week went to, each with its color and percent, leaving out what wasn't used.
pub(crate) fn app_parts(parts: &[(String, String, f32)]) -> Vec<(&str, Color32, f32)> {
    let mut slot = 0;
    parts
        .iter()
        .map(|(key, name, percent)| {
            let color = if key == "other" { OTHER } else { PARTS.get(slot).copied().unwrap_or(OTHER) };
            slot += usize::from(key != "other");
            (name.as_str(), color, *percent)
        })
        .filter(|(_, _, percent)| *percent > 0.0)
        .collect()
}

/// One bar split by what the week went to, with a legend under it.
fn parts(ui: &mut Ui, parts: &[(String, String, f32)]) {
    const GAP: f32 = 2.0;
    let parts = app_parts(parts);
    let total: f32 = parts.iter().map(|p| p.2).sum();
    if total <= 0.0 {
        note(ui, "Nothing used yet this week.");
        return;
    }

    ui.add_space(14.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let room = rect.width() - GAP * (parts.len() - 1) as f32;
    let mut x = rect.left();
    for (i, (name, color, percent)) in parts.iter().enumerate() {
        let segment = Rect::from_min_size(pos2(x, rect.top()), vec2((room * percent / total).max(2.0), rect.height()));
        let (first, last) = (if i == 0 { 4 } else { 0 }, if i == parts.len() - 1 { 4 } else { 0 });
        ui.painter().rect_filled(segment, CornerRadius { nw: first, sw: first, ne: last, se: last }, *color);
        ui.interact(segment, ui.id().with(("part", i)), Sense::hover()).on_hover_text(format!("{name} \u{B7} {percent:.0}%"));
        x = segment.right() + GAP;
    }
    ui.add_space(8.0);
    for (name, color, percent) in &parts {
        let percent = text(ui, format!("{percent:.0}%"), BODY, TEXT);
        let name = text(ui, *name, BODY, TEXT);
        let rect = split(ui, 18.0, name, Some(percent), 24.0).rect;
        let swatch = Rect::from_center_size(pos2(rect.left() + 5.0, rect.center().y), vec2(10.0, 10.0));
        ui.painter().rect_filled(swatch, 3, *color);
    }
    ui.add_space(6.0);
}

fn account(ui: &mut Ui, state: &State) {
    let account = &state.account;
    let mut facts = Vec::new();
    if let Some(plan) = &account.plan {
        facts.push(("Plan", plan.clone()));
    }
    if let Some(since) = account.since {
        facts.push(("Subscribed since", since.with_timezone(&Local).format("%-d %b %Y").to_string()));
    }
    if let Some(billing) = &account.billing {
        facts.push(("Billed through", billing.clone()));
    }
    if let Some(role) = &account.role {
        facts.push(("Role", role.clone()));
    }
    if let Some(extra) = &state.extra {
        facts.push(("Extra usage", extra_usage(extra)));
    }
    if facts.is_empty() {
        note(ui, "Shows once Claude Code is logged in.");
    }
    for (i, (label, value)) in facts.into_iter().enumerate() {
        if i > 0 {
            widgets::rule(ui);
        }
        fact(ui, label, value);
    }
}

pub(crate) fn extra_usage(extra: &Extra) -> String {
    let spent = match (extra.enabled, &extra.used, &extra.limit) {
        (false, ..) => return "Off".into(),
        (true, Some(used), Some(limit)) => format!("{used} of {limit}"),
        (true, Some(used), None) => format!("{used} used"),
        (true, None, _) => "On".into(),
    };
    if extra.capped { spent + " \u{B7} limit reached" } else { spent }
}

fn sessions(ui: &mut Ui, sessions: &[Session], now: DateTime<Utc>) {
    if sessions.is_empty() {
        note(ui, "No Claude Code sessions are running.");
    }
    for (i, session) in sessions.iter().enumerate() {
        if i > 0 {
            widgets::rule(ui);
        }
        session_row(ui, session, now);
    }
}

/// A dot, lit while working, then the conversation's title with where it runs underneath.
fn session_row(ui: &mut Ui, session: &Session, now: DateTime<Utc>) {
    const INDENT: f32 = 16.0;
    const PADDING: f32 = 9.0;
    let (status, color) = if session.busy { ("Working", TEXT) } else { ("Idle", DIM) };
    let status = text(ui, status, SMALL, color);
    let width = ui.available_width();
    let room = width - INDENT - status.size().x - 12.0;
    let painter = ui.painter().clone();
    let name = widgets::single_line(&painter, session.display_name(), FontId::proportional(BODY), TEXT, room);
    let about = widgets::single_line(&painter, &session_about(session, now), FontId::proportional(SMALL), DIM, room);

    let name_height = name.size().y;
    let height = name_height + about.size().y + 2.0 * PADDING;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if name.elided {
        response.on_hover_text(session.display_name());
    }
    let top = rect.top() + PADDING;
    let dot = pos2(rect.left() + 4.0, top + name_height / 2.0);
    if session.busy {
        painter.circle_filled(dot, 4.0, CLAUDE_ORANGE);
    } else {
        painter.circle_stroke(dot, 3.5, Stroke::new(1.0, DIM));
    }
    painter.galley(pos2(rect.left() + INDENT, top), name, TEXT);
    painter.galley(pos2(rect.left() + INDENT, top + name_height), about, DIM);
    painter.galley(rect.right_center() - vec2(status.size().x, status.size().y / 2.0), status, TEXT);
}

/// Where a session runs and since when: "Cuota · VS Code · started 1h 35m ago".
pub(crate) fn session_about(session: &Session, now: DateTime<Utc>) -> String {
    let mut about: Vec<String> = session.project.iter().cloned().collect();
    about.extend(Some(session.app.clone()).filter(|app| !app.is_empty()));
    // `until` counts from the second time to the first.
    about.extend(session.started.map(|t| format!("started {} ago", claude::until(now, t))));
    about.join(" \u{B7} ")
}

fn claude_code(ui: &mut Ui, fonts: &Fonts, stats: Option<&Stats>) {
    const TITLE: &str = "Claude Code this week";
    let Some(stats) = stats else {
        section(ui, fonts, TITLE, Some("On this PC"), |ui| note(ui, "Reading Claude Code's history..."));
        return;
    };
    section(ui, fonts, TITLE, Some("On this PC"), |ui| {
        ui.add_space(12.0);
        let total = &stats.total;
        tiles(ui, fonts, &[
            (grouped(total.replies), "Replies"),
            (grouped(total.tool_calls), "Tool calls"),
            (compact(total.tokens.total()), "Tokens"),
        ]);
        ui.add_space(12.0);
        widgets::rule(ui);
        ui.add_space(10.0);
        days(ui, &stats.days);
        ui.add_space(8.0);
        if let Some(hour) = stats.busiest_hour {
            widgets::rule(ui);
            fact(ui, "Busiest hour", hour_span(hour));
        }
    });
    if !stats.models.is_empty() {
        section(ui, fonts, "By model", None, |ui| {
            for (i, (name, tally)) in stats.models.iter().enumerate() {
                if i > 0 {
                    widgets::rule(ui);
                }
                tally_row(ui, fonts, name, tally);
            }
        });
    }
    if !stats.projects.is_empty() {
        section(ui, fonts, "By project", None, |ui| {
            for (i, (name, tally)) in stats.projects.iter().take(TOP_PROJECTS).enumerate() {
                if i > 0 {
                    widgets::rule(ui);
                }
                tally_row(ui, fonts, name, tally);
            }
            let more = stats.projects.len().saturating_sub(TOP_PROJECTS);
            if more > 0 {
                widgets::rule(ui);
                note(ui, &format!("And {more} more"));
            }
        });
    }
    ui.add_space(8.0);
    let about = "Counts Claude Code on this PC only, not claude.ai or other devices. Most tokens are \
                 cached input, which costs far less of your limits than new tokens.";
    ui.add(Label::new(RichText::new(about).size(SMALL).color(DIM)).wrap());
}

/// Big numbers side by side, each with its label underneath.
fn tiles(ui: &mut Ui, fonts: &Fonts, tiles: &[(String, &str)]) {
    let values: Vec<_> = tiles.iter().map(|(value, _)| strong(ui, fonts, value, 20.0)).collect();
    let labels: Vec<_> = tiles.iter().map(|(_, label)| text(ui, *label, SMALL, DIM)).collect();
    let value_height = values.iter().map(|v| v.size().y).fold(0.0, f32::max);
    let height = value_height + labels.iter().map(|l| l.size().y).fold(0.0, f32::max);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let column = rect.width() / tiles.len() as f32;
    for (i, (value, label)) in values.into_iter().zip(labels).enumerate() {
        let x = rect.left() + column * i as f32;
        ui.painter().galley(pos2(x, rect.top()), value, TEXT);
        ui.painter().galley(pos2(x, rect.top() + value_height), label, DIM);
    }
}

/// Tokens per day as bars, today on the right. Hovering a day shows its numbers.
fn days(ui: &mut Ui, days: &[(NaiveDate, Tally)]) {
    const PLOT: f32 = 64.0;
    /// Room above the tallest bar for its value.
    const HEADROOM: f32 = 14.0;
    const LABELS: f32 = 18.0;
    let today = Local::now().date_naive();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PLOT + LABELS), Sense::hover());
    let painter = ui.painter().clone();
    let base = rect.top() + PLOT;
    let most = days.iter().map(|(_, t)| t.tokens.total()).max().unwrap_or(0);
    let slot = rect.width() / days.len().max(1) as f32;
    let bar = (slot - 12.0).clamp(4.0, 26.0);
    painter.hline(rect.x_range(), base + 0.5, Stroke::new(1.0, TRACK));
    for (i, (date, tally)) in days.iter().enumerate() {
        let center = rect.left() + slot * (i as f32 + 0.5);
        let tokens = tally.tokens.total();
        if tokens > 0 {
            let height = ((PLOT - HEADROOM) * tokens as f32 / most as f32).max(2.0);
            let column = Rect::from_min_max(pos2(center - bar / 2.0, base - height), pos2(center + bar / 2.0, base));
            painter.rect_filled(column, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, PARTS[0]);
            if tokens == most {
                let at = pos2(center, column.top() - 2.0);
                painter.text(at, Align2::CENTER_BOTTOM, compact(tokens), FontId::proportional(10.5), DIM);
            }
        }
        let (name, color) = if *date == today { ("Today".to_owned(), TEXT) } else { (date.format("%a").to_string(), DIM) };
        painter.text(pos2(center, base + 4.0), Align2::CENTER_TOP, name, FontId::proportional(11.0), color);
        let hover = Rect::from_min_max(pos2(center - slot / 2.0, rect.top()), pos2(center + slot / 2.0, rect.bottom()));
        ui.interact(hover, ui.id().with(("day", i)), Sense::hover()).on_hover_text(day_about(*date, tally));
    }
}

/// A day's numbers: "Wed 8 Oct · 4.5M tokens · 120 replies · 96 tool calls".
pub(crate) fn day_about(date: NaiveDate, tally: &Tally) -> String {
    format!(
        "{} \u{B7} {} tokens \u{B7} {} replies \u{B7} {} tool calls",
        date.format("%a %-d %b"),
        compact(tally.tokens.total()),
        grouped(tally.replies),
        grouped(tally.tool_calls),
    )
}

/// "14:00–15:00".
pub(crate) fn hour_span(hour: u32) -> String {
    format!("{hour:02}:00\u{2013}{:02}:00", (hour + 1) % 24)
}

/// Tokens split up by kind, as hovering a model or project shows them.
pub(crate) fn token_kinds(tokens: &Tokens) -> String {
    format!(
        "Input {} \u{B7} Output {} \u{B7} Cache read {} \u{B7} Cache write {}",
        compact(tokens.input),
        compact(tokens.output),
        compact(tokens.cache_read),
        compact(tokens.cache_write),
    )
}

/// A model or project: its tokens, with replies and tool calls underneath. Hovering splits
/// the tokens up.
fn tally_row(ui: &mut Ui, fonts: &Fonts, name: &str, tally: &Tally) {
    const PADDING: f32 = 9.0;
    let tokens = strong(ui, fonts, compact(tally.tokens.total()), BODY);
    let room = ui.available_width() - tokens.size().x - 12.0;
    let painter = ui.painter().clone();
    let name = widgets::single_line(&painter, name, FontId::proportional(BODY), TEXT, room);
    let counts = format!("{} replies \u{B7} {} tool calls", grouped(tally.replies), grouped(tally.tool_calls));
    let counts = text(ui, counts, SMALL, DIM);

    let name_height = name.size().y;
    let height = name_height + counts.size().y + 2.0 * PADDING;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    painter.galley(pos2(rect.left(), rect.top() + PADDING), name, TEXT);
    painter.galley(pos2(rect.left(), rect.top() + PADDING + name_height), counts, DIM);
    painter.galley(rect.right_center() - vec2(tokens.size().x, tokens.size().y / 2.0), tokens, TEXT);
    response.on_hover_text(token_kinds(&tally.tokens));
}

/// A label and its value on one line.
fn fact(ui: &mut Ui, label: &str, value: String) {
    let value = text(ui, value, BODY, TEXT);
    let label = text(ui, label, BODY, DIM);
    split(ui, 0.0, label, Some(value), 34.0);
}

fn note(ui: &mut Ui, note: &str) {
    ui.add_space(12.0);
    ui.add(Label::new(RichText::new(note).size(SMALL).color(DIM)).wrap());
    ui.add_space(12.0);
}

/// `left`, `indent` in, and `right` flush right, on a line `height` tall.
fn split(ui: &mut Ui, indent: f32, left: Arc<Galley>, right: Option<Arc<Galley>>, height: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    painter.galley(rect.left_center() + vec2(indent, -left.size().y / 2.0), left, TEXT);
    if let Some(right) = right {
        painter.galley(rect.right_center() - vec2(right.size().x, right.size().y / 2.0), right, TEXT);
    }
    response
}

fn text(ui: &Ui, text: impl Into<String>, size: f32, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.into(), FontId::proportional(size), color)
}

fn strong(ui: &Ui, fonts: &Fonts, text: impl Into<String>, size: f32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.into(), FontId::new(size, fonts.strong.clone()), TEXT)
}

/// "Wed 08:30", in local time.
pub(crate) fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%a %H:%M").to_string()
}

/// 950, 12.3K, 4.5M, 120M, 1.2B.
pub(crate) fn compact(n: u64) -> String {
    for (size, suffix) in [(1e9, "B"), (1e6, "M"), (1e3, "K")] {
        let scaled = n as f64 / size;
        if scaled >= 1.0 {
            return if scaled >= 100.0 { format!("{scaled:.0}{suffix}") } else { format!("{scaled:.1}{suffix}") };
        }
    }
    n.to_string()
}

/// 12,345.
pub(crate) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_counts() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(12_345), "12.3K");
        assert_eq!(compact(4_500_000), "4.5M");
        assert_eq!(compact(123_400_000), "123M");
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(12_345), "12,345");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn describes_extra_usage() {
        let money = |amount| Some(claude::Money { amount, currency: "USD".into() });
        assert_eq!(extra_usage(&Extra::default()), "Off");
        let on = Extra { enabled: true, used: money(12.3), limit: money(50.0), capped: false };
        assert_eq!(extra_usage(&on), "$12.30 of $50.00");
        assert_eq!(extra_usage(&Extra { capped: true, limit: None, ..on }), "$12.30 used \u{B7} limit reached");
    }
}
