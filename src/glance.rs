//! More the panel can show under its meters, picked and put in order on the Details page of
//! settings: a line or two, or a little chart, from what the expanded panel's details tell.
//! Laid out as shapes at the origin, like the meters, so the window can size itself first.

use crate::claude::{Limit, Session, State};
use crate::details::{self, compact, grouped};
use crate::mascot::CLAUDE_ORANGE;
use crate::settings::Detail;
use crate::stats::{Stats, Tally};
use crate::theme::Palette;
use crate::widgets::single_line;
use chrono::{DateTime, Local, NaiveDate, Utc};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, CornerRadius, FontId, Galley, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use std::sync::Arc;

/// Text size, in points at the panel's normal size.
const SIZE: f32 = 12.0;
/// Between one detail and the next.
const GAP: f32 = 5.0;
/// Running sessions named in the settings row's one-line example.
const TITLES: usize = 3;
const READING: &str = "Reading Claude Code's history...";

pub struct Glance {
    pub size: Vec2,
    shapes: Vec<Shape>,
    /// Areas that say more on hover.
    hints: Vec<(Rect, String)>,
}

impl Glance {
    /// Draws the details with their top-left at `origin`; returns the hover hints, moved there too.
    pub fn paint(self, painter: &Painter, origin: Pos2) -> Vec<(Rect, String)> {
        let offset = origin.to_vec2();
        painter.extend(self.shapes.into_iter().map(|mut shape| {
            shape.translate(offset);
            shape
        }));
        self.hints.into_iter().map(|(rect, hint)| (rect.translate(offset), hint)).collect()
    }
}

/// What the details are drawn from.
#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub state: &'a State,
    /// Claude Code's stats, once read.
    pub stats: Option<&'a Stats>,
    /// The limits the meters show.
    pub limits: &'a [&'a Limit],
    pub now: DateTime<Utc>,
}

/// Lays the details out; the painter is only used to shape text.
pub struct Builder<'a> {
    pub painter: &'a Painter,
    pub source: Source<'a>,
    pub palette: &'a Palette,
    /// How big to draw: 1 is the original size.
    pub scale: f32,
    /// How wide they run: text wraps there, and the charts are this wide.
    pub width: f32,
}

impl Builder<'_> {
    /// `details` one under another, leaving out those with nothing to show.
    pub fn build(&self, details: impl IntoIterator<Item = Detail>) -> Glance {
        let mut glance = Glance { size: Vec2::ZERO, shapes: Vec::new(), hints: Vec::new() };
        let mut y = None;
        // Said once, however many details wait for Claude Code's stats.
        let mut reading = false;
        for detail in details {
            let content = if detail.uses_stats() && self.source.stats.is_none() {
                if std::mem::replace(&mut reading, true) {
                    continue;
                }
                Content::Note(READING)
            } else {
                match content(detail, &self.source, self.palette) {
                    Some(content) => content,
                    None => continue,
                }
            };
            let top = y.map_or(0.0, |y| y + GAP * self.scale);
            y = Some(match content {
                Content::Lines(lines) => self.lines(&mut glance, lines, top),
                Content::Note(note) => self.lines(&mut glance, vec![Line::note(note, self.palette)], top),
                Content::Parts(parts) => self.parts(&mut glance, &parts, top),
                Content::Days(days) => self.days(&mut glance, days, top),
                Content::Titles(sessions) => self.titles(&mut glance, sessions, top),
            });
        }
        glance.size.y = y.unwrap_or(0.0);
        glance
    }

    fn font(&self) -> FontId {
        FontId::proportional(SIZE * self.scale)
    }

    /// The spans as one text, wrapped at `width` if given.
    fn layout(&self, spans: Vec<(String, Color32)>, width: Option<f32>) -> Arc<Galley> {
        let mut job = LayoutJob::default();
        for (text, color) in spans {
            job.append(&text, 0.0, TextFormat::simple(self.font(), color));
        }
        job.wrap.max_width = width.unwrap_or(f32::INFINITY);
        self.painter.layout_job(job)
    }

    /// Returns where the lines end.
    fn lines(&self, glance: &mut Glance, lines: Vec<Line>, mut y: f32) -> f32 {
        for line in lines {
            let galley = self.layout(line.spans, Some(self.width));
            let rect = Rect::from_min_size(pos2(0.0, y), galley.size());
            glance.size.x = glance.size.x.max(rect.width());
            glance.shapes.push(Shape::galley(rect.min, galley, Color32::PLACEHOLDER));
            if let Some(hint) = line.hint {
                glance.hints.push((rect, hint));
            }
            y = rect.bottom();
        }
        y
    }

    /// A bar split by what the week went to, with a legend that flows onto more lines as it
    /// needs. Returns where it ends.
    fn parts(&self, glance: &mut Glance, parts: &[(&str, Color32, f32)], top: f32) -> f32 {
        let s = self.scale;
        let gap = 2.0 * s;
        let total: f32 = parts.iter().map(|part| part.2).sum();
        let round = (2.5 * s).round() as u8;
        let room = self.width - gap * (parts.len() - 1) as f32;
        let mut x = 0.0;
        for (i, (name, color, percent)) in parts.iter().enumerate() {
            let segment = Rect::from_min_size(pos2(x, top), vec2((room * percent / total).max(2.0 * s), 5.0 * s));
            let (first, last) = (if i == 0 { round } else { 0 }, if i == parts.len() - 1 { round } else { 0 });
            glance.shapes.push(Shape::rect_filled(segment, CornerRadius { nw: first, sw: first, ne: last, se: last }, *color));
            glance.hints.push((segment.expand2(vec2(0.0, 3.0 * s)), format!("{name} \u{B7} {percent:.0}%")));
            x = segment.right() + gap;
        }

        let swatch = 8.0 * s;
        let mut at = pos2(0.0, top + 10.0 * s);
        let mut height: f32 = 0.0;
        for (name, color, percent) in parts {
            let spans = vec![(format!("{name} "), self.palette.dim), (format!("{percent:.0}%"), self.palette.text)];
            let label = self.layout(spans, None);
            let width = swatch + 4.0 * s + label.size().x;
            if at.x > 0.0 && at.x + width > self.width {
                at = pos2(0.0, at.y + height);
            }
            height = label.size().y;
            let mark = Rect::from_center_size(pos2(at.x + swatch / 2.0, at.y + height / 2.0), vec2(swatch, swatch));
            glance.shapes.push(Shape::rect_filled(mark, 2.0 * s, *color));
            glance.shapes.push(Shape::galley(pos2(at.x + swatch + 4.0 * s, at.y), label, Color32::PLACEHOLDER));
            at.x += width + 10.0 * s;
        }
        glance.size.x = glance.size.x.max(self.width);
        at.y + height
    }

    /// Little bars of tokens per day, named under them, today on the right. Hovering a day
    /// shows its numbers. Returns where they end.
    fn days(&self, glance: &mut Glance, days: &[(NaiveDate, Tally)], top: f32) -> f32 {
        let s = self.scale;
        let plot = 22.0 * s;
        let base = top + plot;
        let today = Local::now().date_naive();
        let most = days.iter().map(|(_, tally)| tally.tokens.total()).max().unwrap_or(0).max(1);
        let slot = self.width / days.len().max(1) as f32;
        let bar = (slot - 8.0 * s).clamp(3.0 * s, 18.0 * s);
        let round = (2.0 * s).round() as u8;
        let font = FontId::proportional(10.0 * s);
        glance.shapes.push(Shape::hline(0.0..=self.width, base + 0.5, Stroke::new(1.0, self.palette.track)));
        let mut bottom = base;
        for (i, (date, tally)) in days.iter().enumerate() {
            let center = slot * (i as f32 + 0.5);
            let tokens = tally.tokens.total();
            if tokens > 0 {
                let height = (plot * tokens as f32 / most as f32).max(2.0 * s);
                let column = Rect::from_min_max(pos2(center - bar / 2.0, base - height), pos2(center + bar / 2.0, base));
                let corners = CornerRadius { nw: round, ne: round, sw: 0, se: 0 };
                glance.shapes.push(Shape::rect_filled(column, corners, details::PARTS[0]));
            }
            let (text, dim) = (self.palette.text, self.palette.dim);
            let (name, color) = if *date == today { ("Today".to_owned(), text) } else { (date.format("%a").to_string(), dim) };
            let name = self.painter.layout_no_wrap(name, font.clone(), color);
            let at = pos2(center - name.size().x / 2.0, base + 2.0 * s);
            let below = at.y + name.size().y;
            bottom = bottom.max(below);
            glance.shapes.push(Shape::galley(at, name, color));
            let hover = Rect::from_min_max(pos2(center - slot / 2.0, top), pos2(center + slot / 2.0, below));
            glance.hints.push((hover, details::day_about(*date, tally)));
        }
        glance.size.x = glance.size.x.max(self.width);
        bottom
    }

    /// A line for each running session: a dot, lit while it works, and its title. Returns where
    /// they end.
    fn titles(&self, glance: &mut Glance, sessions: &[Session], mut y: f32) -> f32 {
        let s = self.scale;
        let indent = 12.0 * s;
        for session in sessions {
            let color = if session.busy { self.palette.text } else { self.palette.dim };
            let name = session.display_name();
            let title = single_line(self.painter, name, self.font(), color, self.width - indent);
            let rect = Rect::from_min_size(pos2(indent, y), title.size());
            let dot = pos2(4.0 * s, rect.center().y);
            glance.shapes.push(if session.busy {
                Shape::circle_filled(dot, 3.0 * s, CLAUDE_ORANGE)
            } else {
                Shape::circle_stroke(dot, 2.5 * s, Stroke::new(1.0, self.palette.dim))
            });
            let about = details::session_about(session, self.source.now);
            let hint = if title.elided { format!("{name}\n{about}") } else { about };
            if !hint.is_empty() {
                glance.hints.push((rect, hint));
            }
            glance.size.x = glance.size.x.max(rect.right());
            glance.shapes.push(Shape::galley(rect.min, title, color));
            y = rect.bottom();
        }
        y
    }
}

/// What a detail shows.
enum Content<'a> {
    Lines(Vec<Line>),
    /// A dim line standing in for what isn't there.
    Note(&'static str),
    /// What the week went to: a split bar with a legend.
    Parts(Vec<(&'a str, Color32, f32)>),
    /// Tokens per day as little bars, today last.
    Days(&'a [(NaiveDate, Tally)]),
    /// The running sessions' titles, busy ones first.
    Titles(&'a [Session]),
}

/// A line of text in spans of color, and what hovering it says.
struct Line {
    spans: Vec<(String, Color32)>,
    hint: Option<String>,
}

impl Line {
    fn new(spans: Vec<(String, Color32)>) -> Self {
        Self { spans, hint: None }
    }

    /// `value` after a dim `label`, as the meters show a limit and how much of it is used.
    fn fact(label: &str, value: String, color: Color32, palette: &Palette) -> Self {
        Self::new(vec![(format!("{label} "), palette.dim), (value, color)])
    }

    /// One of a limit's notes from the details after its name, read as one sentence: "Weekly on
    /// pace to end at about 62%". Notes dim on the details' cards are bright here, where
    /// they're the point.
    fn limit(limit: &Limit, (note, color): (String, Color32), palette: &Palette) -> Self {
        let color = if color == palette.dim { palette.text } else { color };
        Self::fact(&limit.label, lowercase_first(&note), color, palette)
    }

    fn note(text: impl Into<String>, palette: &Palette) -> Self {
        Self::new(vec![(text.into(), palette.dim)])
    }

    fn hint(self, hint: Option<String>) -> Self {
        Self { hint, ..self }
    }

    fn text(&self) -> String {
        self.spans.iter().map(|(text, _)| text.as_str()).collect()
    }
}

/// What `detail` has to show, if anything, in `palette`'s colors. Those from Claude Code's
/// stats have nothing until they're read.
fn content<'a>(detail: Detail, source: &Source<'a>, palette: &Palette) -> Option<Content<'a>> {
    let &Source { state, stats, limits, now } = source;
    let p = palette;
    let (text, dim) = (p.text, p.dim);
    let lines = |lines: Vec<Line>| (!lines.is_empty()).then_some(Content::Lines(lines));
    match detail {
        Detail::Resets | Detail::Pace => {
            let note = if detail == Detail::Resets { details::reset_note } else { details::pace_note };
            lines(limits.iter().filter_map(|limit| Some(Line::limit(limit, note(limit, now, p)?, p))).collect())
        }
        Detail::WeeklyShare => lines(
            limits
                .iter()
                .filter_map(|limit| {
                    let (used, allowed) = limit.share?;
                    let share = format!("used {used:.0}% of Weekly \u{B7} capped at {allowed:.0}%");
                    Some(Line::fact(&limit.label, share, text, p))
                })
                .collect(),
        ),
        Detail::Apps => {
            let parts = details::app_parts(&state.breakdown.as_ref()?.parts, p);
            Some(if parts.is_empty() { Content::Note("Nothing used yet this week") } else { Content::Parts(parts) })
        }
        Detail::Plan => {
            let account = &state.account;
            let mut spans = vec![(account.plan.clone()?, text)];
            let since = account.since.map(|t| t.with_timezone(&Local));
            spans.extend(since.map(|t| (format!(" \u{B7} since {}", t.format("%b %Y")), dim)));
            // The rest of what the details tell of the account.
            let mut facts: Vec<String> = since.map(|t| format!("Subscribed since {}", t.format("%-d %b %Y"))).into_iter().collect();
            facts.extend(account.billing.as_ref().map(|billing| format!("Billed through {billing}")));
            facts.extend(account.role.as_ref().map(|role| format!("Role: {role}")));
            lines(vec![Line::new(spans).hint((!facts.is_empty()).then(|| facts.join("\n")))])
        }
        Detail::ExtraUsage => {
            let extra = state.extra.as_ref()?;
            let color = if extra.capped { p.red } else { text };
            lines(vec![Line::fact("Extra usage", lowercase_first(&details::extra_usage(extra)), color, p)])
        }
        Detail::Sessions => {
            let working = state.sessions.iter().filter(|session| session.busy).count();
            let running = match (state.sessions.len(), working) {
                (0, _) => return Some(Content::Note("No sessions running")),
                (running, 0) => format!("{running} running \u{B7} idle"),
                (running, working) => format!("{running} running \u{B7} {working} working"),
            };
            lines(vec![Line::fact("Sessions", running, text, p)])
        }
        Detail::SessionTitles => (!state.sessions.is_empty()).then_some(Content::Titles(&state.sessions)),
        Detail::ClaudeCode => {
            let total = &stats?.total;
            if total.replies == 0 {
                return Some(Content::Note("No Claude Code replies yet this week"));
            }
            let spans = vec![
                (grouped(total.replies), text),
                (" replies \u{B7} ".to_owned(), dim),
                (grouped(total.tool_calls), text),
                (" tool calls \u{B7} ".to_owned(), dim),
                (compact(total.tokens.total()), text),
                (" tokens".to_owned(), dim),
            ];
            let hint = format!("Claude Code this week, on this PC\n{}", details::token_kinds(&total.tokens));
            lines(vec![Line::new(spans).hint(Some(hint))])
        }
        Detail::TokensPerDay => {
            let days = &stats?.days;
            days.iter().any(|(_, tally)| tally.tokens.total() > 0).then_some(Content::Days(days))
        }
        Detail::BusiestHour => lines(vec![Line::fact("Busiest hour", details::hour_span(stats?.busiest_hour?), text, p)]),
        Detail::TopModel => lines(vec![top("Top model", stats?.models.first()?, p)]),
        Detail::TopProject => lines(vec![top("Top project", stats?.projects.first()?, p)]),
        Detail::Updated => lines(vec![Line::fact("Updated", state.updated?.format("%H:%M").to_string(), text, p)]),
    }
}

/// The model or project with the most tokens, and how many. Hovering splits them up.
fn top(label: &str, (name, tally): &(String, Tally), palette: &Palette) -> Line {
    let tokens = format!(" \u{B7} {} tokens", compact(tally.tokens.total()));
    let line = Line::new(vec![(format!("{label} "), palette.dim), (name.clone(), palette.text), (tokens, palette.dim)]);
    line.hint(Some(details::token_kinds(&tally.tokens)))
}

/// "On pace…" as "on pace…", to follow a label in one sentence.
fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

/// What a detail is called in settings.
pub fn title(detail: Detail) -> &'static str {
    match detail {
        Detail::Resets => "Reset times",
        Detail::Pace => "Pace",
        Detail::WeeklyShare => "Share of Weekly",
        Detail::Apps => "This week by app",
        Detail::Plan => "Plan",
        Detail::ExtraUsage => "Extra usage",
        Detail::Sessions => "Sessions",
        Detail::SessionTitles => "Session titles",
        Detail::ClaudeCode => "Claude Code this week",
        Detail::TokensPerDay => "Tokens per day",
        Detail::BusiestHour => "Busiest hour",
        Detail::TopModel => "Top model",
        Detail::TopProject => "Top project",
        Detail::Updated => "Last updated",
    }
}

/// An example of what `detail` shows, for its row in settings: what the panel would show now
/// when it has something to show, or else a made-up one.
pub fn example(detail: Detail, source: &Source) -> String {
    // Only the words are used.
    let now = match content(detail, source, &Palette::DARK) {
        Some(Content::Lines(lines)) => lines.first().map(Line::text),
        Some(Content::Parts(parts)) => {
            let parts: Vec<String> = parts.iter().map(|(name, _, percent)| format!("{name} {percent:.0}%")).collect();
            Some(parts.join(" \u{B7} "))
        }
        Some(Content::Days(days)) => {
            days.last().map(|(_, today)| format!("A bar for each day's tokens \u{B7} {} today", compact(today.tokens.total())))
        }
        Some(Content::Titles(sessions)) => {
            Some(sessions.iter().take(TITLES).map(Session::display_name).collect::<Vec<_>>().join(" \u{B7} "))
        }
        Some(Content::Note(_)) | None => None,
    };
    now.unwrap_or_else(|| sample(detail).to_owned())
}

/// A made-up example of what `detail` shows, for when there's nothing to show yet.
fn sample(detail: Detail) -> &'static str {
    match detail {
        Detail::Resets => "Session resets Wed 14:30 \u{B7} in 2h 10m",
        Detail::Pace => "Weekly on pace to end at about 62%",
        Detail::WeeklyShare => "Fable used 46% of Weekly \u{B7} capped at 50%",
        Detail::Apps => "Claude Code 81% \u{B7} Chats 15% \u{B7} Cowork 4%",
        Detail::Plan => "Claude Max 5x \u{B7} since Aug 2025",
        Detail::ExtraUsage => "Extra usage $12.30 of $50.00",
        Detail::Sessions => "Sessions 2 running \u{B7} 1 working",
        Detail::SessionTitles => "Fix the pricing page layout \u{B7} Add a dark theme",
        Detail::ClaudeCode => "1,204 replies \u{B7} 2,318 tool calls \u{B7} 45.6M tokens",
        Detail::TokensPerDay => "A bar for each day's tokens \u{B7} 4.5M today",
        Detail::BusiestHour => "Busiest hour 14:00\u{2013}15:00",
        Detail::TopModel => "Top model Opus 5.5 \u{B7} 38.2M tokens",
        Detail::TopProject => "Top project Cuota \u{B7} 21.7M tokens",
        Detail::Updated => "Updated 14:32",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::demo_state;

    const DARK: Palette = Palette::DARK;

    /// The text of each line `details` show, top to bottom, with the panel's fonts.
    fn drawn(source: Source, details: &[Detail]) -> Vec<String> {
        let ctx = eframe::egui::Context::default();
        crate::overlay::setup_fonts(&ctx);
        let mut texts = Vec::new();
        let _ = ctx.run_ui(Default::default(), |ui| {
            let builder = Builder { painter: ui.painter(), source, palette: &DARK, scale: 1.0, width: 240.0 };
            for shape in builder.build(details.iter().copied()).shapes {
                if let Shape::Text(text) = shape {
                    texts.push(text.galley.text().to_owned());
                }
            }
        });
        texts
    }

    #[test]
    fn details_with_nothing_to_show_are_left_out_and_waiting_for_stats_is_said_once() {
        let state = State::default();
        let source = Source { state: &state, stats: None, limits: &[], now: Utc::now() };
        assert_eq!(drawn(source, &Detail::ALL), ["No sessions running", READING]);
    }

    #[test]
    fn a_limits_notes_follow_its_name_as_one_sentence_bright_unless_theyre_a_warning() {
        let state = demo_state();
        let limits: Vec<&Limit> = state.limits.iter().collect();
        let line = Line::limit(limits[1], ("On pace to end at about 58%".to_owned(), DARK.dim), &DARK);
        assert_eq!(line.text(), "Weekly on pace to end at about 58%");
        assert_eq!(line.spans[1].1, DARK.text);
        let warning = Line::limit(limits[2], ("At this pace, runs out Wed 18:23".to_owned(), DARK.amber), &DARK);
        assert_eq!(warning.spans[1].1, DARK.amber);
        // So too in the light theme, where dim and bright are other colors.
        let light = Palette::LIGHT;
        assert_eq!(Line::limit(limits[1], ("On pace".to_owned(), light.dim), &light).spans[1].1, light.text);
        let source = Source { state: &state, stats: None, limits: &limits, now: Utc::now() };
        let resets = drawn(source, &[Detail::Resets]);
        assert_eq!(resets.len(), 3, "one for each limit the meters show: {resets:?}");
        assert!(resets[0].starts_with("Session resets "), "{resets:?}");
    }

    #[test]
    fn examples_show_what_the_panel_would_now_or_else_a_made_up_one() {
        let state = demo_state();
        let limits: Vec<&Limit> = state.limits.iter().collect();
        let source = Source { state: &state, stats: None, limits: &limits, now: Utc::now() };
        assert_eq!(example(Detail::Plan, &source), "Claude Max 5x \u{B7} since Aug 2025");
        assert_eq!(example(Detail::Sessions, &source), "Sessions 2 running \u{B7} 1 working");
        assert_eq!(example(Detail::Apps, &source), "Claude Code 81% \u{B7} Chats 15% \u{B7} Cowork 4%");
        assert_eq!(example(Detail::TopModel, &source), sample(Detail::TopModel), "no stats yet");
        let nothing = State::default();
        let source = Source { state: &nothing, stats: None, limits: &[], now: Utc::now() };
        for detail in Detail::ALL {
            assert_eq!(example(detail, &source), sample(detail));
        }
    }
}
