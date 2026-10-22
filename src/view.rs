//! The panel's usage meters. Each layout is built as shapes at the origin, so the window can
//! size itself before placing them: rows of bars, or a row of rings.

use crate::claude::{self, Limit};
use crate::settings::{Style, View};
use crate::theme::Palette;
use chrono::{DateTime, Local, Utc};
use eframe::egui::{Align2, Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use std::f32::consts::TAU;
use std::sync::Arc;

pub struct Meters {
    pub size: Vec2,
    shapes: Vec<Shape>,
    /// Areas that show the exact reset time on hover.
    hints: Vec<(Rect, String)>,
}

impl Meters {
    /// Draws the meters with their top-left at `origin`; returns the hover hints, moved there too.
    pub fn paint(self, painter: &Painter, origin: Pos2) -> Vec<(Rect, String)> {
        let offset = origin.to_vec2();
        painter.extend(self.shapes.into_iter().map(|mut shape| {
            shape.translate(offset);
            shape
        }));
        self.hints.into_iter().map(|(rect, hint)| (rect.translate(offset), hint)).collect()
    }

    fn new() -> Self {
        Self { size: Vec2::ZERO, shapes: Vec::new(), hints: Vec::new() }
    }

    /// Places text laid out in its colors.
    fn text(&mut self, galley: &Arc<Galley>, anchor: Align2, at: Pos2) {
        let rect = anchor.anchor_size(at, galley.size());
        self.shapes.push(Shape::galley(rect.min, galley.clone(), Color32::PLACEHOLDER));
    }

    fn bar(&mut self, track: Rect, percent: f32, palette: &Palette) {
        let radius = track.height() / 2.0;
        self.shapes.push(Shape::rect_filled(track, radius, palette.track));
        let fraction = (percent / 100.0).clamp(0.0, 1.0);
        if fraction > 0.0 {
            let mut fill = track;
            fill.set_width((track.width() * fraction).max(track.height()));
            self.shapes.push(Shape::rect_filled(fill, radius, palette.level(percent)));
        }
    }

    /// A ring filling clockwise from the top.
    fn ring(&mut self, center: Pos2, radius: f32, width: f32, percent: f32, palette: &Palette) {
        self.shapes.push(Shape::circle_stroke(center, radius, Stroke::new(width, palette.track)));
        let fraction = (percent / 100.0).clamp(0.0, 1.0);
        if fraction > 0.0 {
            let steps = (64.0 * fraction).ceil().max(2.0) as usize;
            let points = (0..=steps)
                .map(|i| {
                    let angle = -TAU / 4.0 + TAU * fraction * i as f32 / steps as f32;
                    center + radius * vec2(angle.cos(), angle.sin())
                })
                .collect();
            self.shapes.push(Shape::line(points, Stroke::new(width, palette.level(percent))));
        }
    }

    fn hint(&mut self, rect: Rect, limit: &Limit) {
        if let Some(t) = limit.resets_at {
            let when = t.with_timezone(&Local).format("Resets %a %H:%M");
            self.hints.push((rect, format!("{} · {when}", limit.label)));
        }
    }
}

/// Lays out text and meters; the painter is only used to shape text.
pub struct Builder<'a> {
    pub painter: &'a Painter,
    pub strong: &'a FontFamily,
    pub view: &'a View,
    pub palette: &'a Palette,
    pub now: DateTime<Utc>,
    /// How big to draw: 1 is the original size.
    pub scale: f32,
}

impl Builder<'_> {
    pub fn build(&self, limits: &[&Limit]) -> Meters {
        match self.view.style {
            Style::Bars => self.bars(limits),
            Style::Pie => self.pies(limits),
        }
    }

    /// `Session  ━━━━━──────  23%  1h 14m`, one row per limit.
    fn bars(&self, limits: &[&Limit]) -> Meters {
        const ROW: f32 = 18.0;
        const BAR: f32 = 96.0;
        let s = self.scale;
        let labels: Vec<_> = limits.iter().map(|l| self.text(&l.label, 12.5, self.palette.dim)).collect();
        let percents = self.percents(limits, 12.5);
        let lefts = self.time_lefts(limits, 12.5);

        let bar_x = widest(&labels) + 10.0 * s;
        let percent_end = bar_x + BAR * s + if percents.is_empty() { 0.0 } else { 10.0 * s + widest(&percents) };
        let width = percent_end + if lefts.is_empty() { 0.0 } else { 12.0 * s + widest(&lefts) };
        let mut meters = Meters::new();
        meters.size = vec2(width, ROW * s * limits.len() as f32);
        for (i, limit) in limits.iter().enumerate() {
            let mid = ROW * s * (i as f32 + 0.5);
            meters.text(&labels[i], Align2::LEFT_CENTER, pos2(0.0, mid));
            let track = Rect::from_min_size(pos2(bar_x, mid - 2.0 * s), vec2(BAR * s, 4.0 * s));
            meters.bar(track, limit.percent, self.palette);
            if let Some(g) = percents.get(i) {
                meters.text(g, Align2::RIGHT_CENTER, pos2(percent_end, mid));
            }
            if let Some(g) = lefts.get(i) {
                meters.text(g, Align2::RIGHT_CENTER, pos2(width, mid));
            }
            meters.hint(Rect::from_min_size(pos2(0.0, mid - ROW * s / 2.0), vec2(width, ROW * s)), limit);
        }
        meters
    }

    /// A ring per limit with the percentage inside, label and time left underneath.
    fn pies(&self, limits: &[&Limit]) -> Meters {
        const RING: f32 = 36.0;
        const GAP: f32 = 14.0;
        let s = self.scale;
        let (ring, gap) = (RING * s, GAP * s);
        let percents = self.percents(limits, 11.5);
        let lefts = self.time_lefts(limits, 11.0);
        let mut meters = Meters::new();
        let mut x = 0.0;
        for (i, limit) in limits.iter().enumerate() {
            let label = self.text(&limit.label, 11.5, self.palette.dim);
            let column = ring.max(label.size().x).max(lefts.get(i).map_or(0.0, |g| g.size().x));
            let center = x + column / 2.0;
            meters.ring(pos2(center, ring / 2.0), ring / 2.0 - 2.5 * s, 5.0 * s, limit.percent, self.palette);
            if let Some(g) = percents.get(i) {
                meters.text(g, Align2::CENTER_CENTER, pos2(center, ring / 2.0));
            }
            let mut y = ring + 4.0 * s;
            for g in std::iter::once(&label).chain(lefts.get(i)) {
                meters.text(g, Align2::CENTER_TOP, pos2(center, y));
                y += g.size().y;
            }
            meters.size.y = meters.size.y.max(y);
            meters.hint(Rect::from_min_max(pos2(x, 0.0), pos2(x + column, y)), limit);
            x += column + gap;
        }
        meters.size.x = (x - gap).max(0.0);
        meters
    }

    fn text(&self, text: &str, size: f32, color: Color32) -> Arc<Galley> {
        self.painter.layout_no_wrap(text.to_owned(), FontId::proportional(size * self.scale), color)
    }

    /// Empty when the view hides percentages.
    fn percents(&self, limits: &[&Limit], size: f32) -> Vec<Arc<Galley>> {
        if !self.view.percent {
            return Vec::new();
        }
        let font = FontId::new(size * self.scale, self.strong.clone());
        limits
            .iter()
            .map(|l| self.painter.layout_no_wrap(format!("{:.0}%", l.percent), font.clone(), self.palette.text))
            .collect()
    }

    /// Empty when the view hides time left.
    fn time_lefts(&self, limits: &[&Limit], size: f32) -> Vec<Arc<Galley>> {
        if !self.view.time_left {
            return Vec::new();
        }
        limits
            .iter()
            .map(|l| self.text(&l.resets_at.map(|t| claude::until(t, self.now)).unwrap_or_default(), size, self.palette.dim))
            .collect()
    }
}

fn widest(galleys: &[Arc<Galley>]) -> f32 {
    galleys.iter().map(|g| g.size().x).fold(0.0, f32::max)
}
