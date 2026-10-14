//! The settings window's building blocks: section headings, cards of setting rows (an icon, a
//! title, an optional description and a control), and the controls themselves.

use crate::mascot::CLAUDE_ORANGE;
use crate::view::{DIM, TEXT};
use eframe::egui::{
    Align, Align2, Color32, CursorIcon, EventFilter, FontFamily, FontId, Frame, Galley, Key, Layout,
    Margin, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, WidgetType, emath,
    pos2,
    text::{LayoutJob, TextWrapping},
    vec2,
};
use std::ops::RangeInclusive;
use std::sync::Arc;

pub const WINDOW: Color32 = Color32::from_rgb(28, 28, 30);
pub const CARD: Color32 = Color32::from_rgb(37, 37, 41);
const LINE: Color32 = Color32::from_rgb(49, 49, 54);
const TRACK: Color32 = Color32::from_rgb(56, 56, 62);
const HOVER: Color32 = Color32::from_rgb(68, 68, 75);
const SELECTED: Color32 = Color32::from_rgb(86, 86, 94);
const ACCENT: Color32 = CLAUDE_ORANGE;

/// Glyphs of Segoe Fluent Icons (Windows 11), which keeps Segoe MDL2 Assets' (Windows 10) code points.
pub mod icon {
    pub const ANIMATE: char = '\u{E768}';
    pub const CHECK: char = '\u{E73E}';
    pub const CLAWD: char = '\u{E76E}';
    pub const DOUBLE_CLICK: char = '\u{E962}';
    pub const LIMITS: char = '\u{E9F9}';
    pub const OPACITY: char = '\u{E793}';
    pub const PERCENT: char = '\u{E94C}';
    pub const QUIT: char = '\u{E7E8}';
    pub const REFRESH: char = '\u{E72C}';
    pub const SHOW_AS: char = '\u{E7F4}';
    pub const STARTUP: char = '\u{EC4A}';
    pub const STYLE: char = '\u{EB05}';
    pub const TIME_LEFT: char = '\u{E916}';
}

const TITLE_SIZE: f32 = 13.0;
const DESCRIPTION_SIZE: f32 = 11.5;
const CONTROL_SIZE: f32 = 12.5;
const ICON_SIZE: f32 = 16.0;
/// Where titles start, past the icon.
const INDENT: f32 = 30.0;
const CARD_PADDING: f32 = 14.0;
const ROW_PADDING: f32 = 8.0;
const CONTROL_HEIGHT: f32 = 24.0;
pub const TOGGLE_WIDTH: f32 = 34.0;

/// The font families the UI draws with.
#[derive(Clone)]
pub struct Fonts {
    /// Semibold, for emphasis.
    pub strong: FontFamily,
    /// Windows' icon font. Without it, rows go without icons.
    pub icons: Option<FontFamily>,
}

impl Fonts {
    fn indent(&self) -> f32 {
        if self.icons.is_some() { INDENT } else { 0.0 }
    }

    fn icon(&self, size: f32) -> Option<FontId> {
        self.icons.clone().map(|family| FontId::new(size, family))
    }
}

/// A small caps section title, with an optional control on the right.
pub fn heading(ui: &mut Ui, fonts: &Fonts, text: &str, add_control: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_HEIGHT), Sense::hover());
    let mut job = LayoutJob::simple_singleline(text.to_uppercase(), FontId::new(11.0, fonts.strong.clone()), DIM);
    job.sections[0].format.extra_letter_spacing = 0.6;
    let galley = ui.painter().layout_job(job);
    let at = pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, DIM);
    let mut slot = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
    add_control(&mut slot);
}

/// A rounded group of rows. Put a [`divider`] between rows.
pub fn card<R>(ui: &mut Ui, add_rows: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(CARD_PADDING as i8, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_rows(ui)
        })
        .inner
}

/// A hairline between rows, starting under the titles and running to the card's edge.
pub fn divider(ui: &mut Ui, fonts: &Fonts) {
    hairline(ui, fonts.indent());
}

/// A hairline between rows that have no icons.
pub fn rule(ui: &mut Ui) {
    hairline(ui, 0.0);
}

fn hairline(ui: &mut Ui, indent: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    let right = rect.right() + CARD_PADDING - 1.0;
    ui.painter().hline(rect.left() + indent..=right, rect.center().y, Stroke::new(1.0, LINE));
}

/// A setting with its control on the right. `control_width` is the room `add_control` needs.
pub fn row(
    ui: &mut Ui,
    fonts: &Fonts,
    glyph: char,
    title: &str,
    description: Option<&str>,
    control_width: f32,
    add_control: impl FnOnce(&mut Ui),
) {
    let slot = label(ui, fonts, glyph, title, description, control_width, ROW_PADDING);
    let mut slot = ui.new_child(UiBuilder::new().max_rect(slot).layout(Layout::right_to_left(Align::Center)));
    add_control(&mut slot);
}

/// A setting with its control underneath, lined up with the title.
pub fn row_below(
    ui: &mut Ui,
    fonts: &Fonts,
    glyph: char,
    title: &str,
    description: Option<&str>,
    add_control: impl FnOnce(&mut Ui),
) {
    label(ui, fonts, glyph, title, description, 0.0, 8.0);
    ui.horizontal(|ui| {
        ui.add_space(fonts.indent());
        ui.vertical(add_control);
    });
    ui.add_space(ROW_PADDING);
}

/// Draws a row's icon, title and description, leaving `control_width` free on the right.
/// Returns the space for the control, as tall as the text.
fn label(
    ui: &mut Ui,
    fonts: &Fonts,
    glyph: char,
    title: &str,
    description: Option<&str>,
    control_width: f32,
    padding_bottom: f32,
) -> Rect {
    let width = ui.available_width();
    let indent = fonts.indent();
    let gap = if control_width > 0.0 { 12.0 } else { 0.0 };
    let wrap = (width - indent - control_width - gap).max(60.0);
    let painter = ui.painter().clone();
    let title = painter.layout(title.to_owned(), FontId::proportional(TITLE_SIZE), TEXT, wrap);
    let description = description.map(|d| painter.layout(d.to_owned(), FontId::proportional(DESCRIPTION_SIZE), DIM, wrap));
    let text_height = title.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
    let height = text_height.max(CONTROL_HEIGHT);

    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_PADDING + height + padding_bottom), Sense::hover());
    let body = Rect::from_min_size(rect.min + vec2(0.0, ROW_PADDING), vec2(width, height));
    if let Some(font) = fonts.icon(ICON_SIZE) {
        painter.text(pos2(body.left() + ICON_SIZE / 2.0, body.center().y), Align2::CENTER_CENTER, glyph, font, TEXT);
    }
    let mut at = pos2(body.left() + indent, body.center().y - text_height / 2.0);
    let title_height = title.size().y;
    painter.galley(at, title, TEXT);
    if let Some(description) = description {
        at.y += title_height + 1.0;
        painter.galley(at, description, DIM);
    }
    Rect::from_min_max(pos2(body.right() - control_width, body.top()), body.max)
}

/// An on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(TOGGLE_WIDTH, 18.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, ""));

    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let hovered = response.hovered();
    let off = if hovered { HOVER } else { TRACK };
    let on_color = if hovered { ACCENT.lerp_to_gamma(Color32::WHITE, 0.12) } else { ACCENT };
    let radius = rect.height() / 2.0;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, off.lerp_to_gamma(on_color, t));
    let x = emath::lerp(rect.left() + radius..=rect.right() - radius, t);
    painter.circle_filled(pos2(x, rect.center().y), radius - 3.5, DIM.lerp_to_gamma(Color32::WHITE, t));
    focus_ring(ui, &response, rect, radius);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The width [`segmented`] takes for these labels.
pub fn segmented_width(ui: &Ui, labels: &[&str]) -> f32 {
    let widest = labels.iter().map(|l| control_text(ui, l).size().x).fold(0.0, f32::max);
    (widest + 24.0) * labels.len() as f32 + 4.0
}

/// Mutually exclusive options side by side. Hugs its labels unless given a `width`.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
    width: Option<f32>,
) -> Response {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    let width = width.unwrap_or_else(|| segmented_width(ui, &labels));
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::hover());
    let inner = rect.shrink(2.0);
    let segment_width = inner.width() / options.len() as f32;
    let segment_rect =
        |i: usize| Rect::from_min_size(inner.min + vec2(i as f32 * segment_width, 0.0), vec2(segment_width, inner.height()));

    let segments: Vec<Response> = options
        .iter()
        .enumerate()
        .map(|(i, (option, label))| {
            let segment = ui.interact(segment_rect(i), response.id.with(i), Sense::click());
            segment.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, *value == *option, *label));
            if segment.clicked() && *value != *option {
                *value = *option;
                response.mark_changed();
            }
            segment.on_hover_cursor(CursorIcon::PointingHand)
        })
        .collect();

    let selected = options.iter().position(|(option, _)| option == value).unwrap_or(0);
    let slide = ui.ctx().animate_value_with_time(response.id, selected as f32, 0.12);
    let painter = ui.painter();
    painter.rect_filled(rect, 6, TRACK);
    let pill = Rect::from_min_size(inner.min + vec2(slide * segment_width, 0.0), vec2(segment_width, inner.height()));
    painter.rect_filled(pill, 5, SELECTED);
    for (i, (label, segment)) in labels.iter().zip(&segments).enumerate() {
        let galley = control_text(ui, label);
        let color = if i == selected || segment.hovered() { TEXT } else { DIM };
        ui.painter().galley(segment_rect(i).center() - galley.size() / 2.0, galley, color);
        focus_ring(ui, segment, segment_rect(i), 5.0);
    }
    response
}

/// An on/off pill, for picking items out of a few.
pub fn chip(ui: &mut Ui, fonts: &Fonts, on: &mut bool, label: &str) -> Response {
    let text = control_text(ui, label);
    let check = fonts.icon(10.0).map(|font| ui.painter().layout_no_wrap(icon::CHECK.into(), font, ACCENT));
    let check_width = check.as_ref().map_or(0.0, |c| c.size().x + 6.0);
    let size = vec2(12.0 + check_width + text.size().x + 12.0, 26.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, label));

    let radius = rect.height() / 2.0;
    let painter = ui.painter();
    if *on {
        let fill = ACCENT.gamma_multiply(if response.hovered() { 0.28 } else { 0.18 });
        painter.rect(rect, radius, fill, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Inside);
        let mut x = rect.left() + 12.0;
        if let Some(check) = check {
            let y = rect.center().y - check.size().y / 2.0;
            x += check_width;
            painter.galley(pos2(rect.left() + 12.0, y), check, ACCENT);
        }
        painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, TEXT);
    } else {
        let fill = if response.hovered() { TRACK } else { Color32::TRANSPARENT };
        painter.rect(rect, radius, fill, Stroke::new(1.0, TRACK), StrokeKind::Inside);
        painter.galley(rect.center() - text.size() / 2.0, text, DIM);
    }
    focus_ring(ui, &response, rect, radius);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A horizontal slider. Arrow keys nudge it when focused.
pub fn slider(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>, width: f32) -> Response {
    const KNOB: f32 = 7.0;
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    let track = rect.shrink2(vec2(KNOB, 0.0));
    let before = *value;
    if let Some(pointer) = response.interact_pointer_pos() {
        let t = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
        *value = emath::lerp(range.clone(), t);
    }
    if response.has_focus() {
        let filter = EventFilter { horizontal_arrows: true, ..Default::default() };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, filter));
        let step = (range.end() - range.start()) / 20.0;
        let steps = ui.input(|i| i.num_presses(Key::ArrowRight) as f32 - i.num_presses(Key::ArrowLeft) as f32);
        *value = (*value + steps * step).clamp(*range.start(), *range.end());
    }
    if *value != before {
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(*value), ""));

    let x = emath::lerp(track.x_range(), emath::remap_clamp(*value, range, 0.0..=1.0));
    let y = rect.center().y;
    let bar = |from: f32, to: f32| Rect::from_min_max(pos2(from, y - 2.0), pos2(to, y + 2.0));
    let painter = ui.painter();
    painter.rect_filled(bar(track.left(), track.right()), 2, TRACK);
    painter.rect_filled(bar(track.left(), x), 2, ACCENT);
    let active = response.hovered() || response.dragged();
    painter.circle_filled(pos2(x, y), if active { KNOB + 0.5 } else { KNOB }, Color32::WHITE);
    focus_ring(ui, &response, Rect::from_center_size(pos2(x, y), vec2(2.0 * KNOB, 2.0 * KNOB)), KNOB);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A borderless square button showing an icon, or a plain button saying `label` without the
/// icon font. `label` also names it for screen readers.
pub fn icon_button(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str) -> Response {
    let Some(font) = fonts.icon(14.0) else { return ui.button(label) };
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 6, if response.is_pointer_button_down_on() { SELECTED } else { HOVER });
    }
    painter.text(rect.center(), Align2::CENTER_CENTER, glyph, font, if hovered { TEXT } else { DIM });
    focus_ring(ui, &response, rect, 6.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A filled button with an icon before its label.
pub fn button(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str) -> Response {
    let text = control_text(ui, label);
    let icon = fonts.icon(12.0).map(|font| ui.painter().layout_no_wrap(glyph.into(), font, TEXT));
    let icon_width = icon.as_ref().map_or(0.0, |i| i.size().x + 8.0);
    let (rect, response) = ui.allocate_exact_size(vec2(12.0 + icon_width + text.size().x + 12.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let fill = match (response.hovered(), response.is_pointer_button_down_on()) {
        (_, true) => SELECTED,
        (true, false) => HOVER,
        _ => TRACK,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 6, fill);
    let mut x = rect.left() + 12.0;
    if let Some(icon) = icon {
        painter.galley(pos2(x, rect.center().y - icon.size().y / 2.0), icon, TEXT);
        x += icon_width;
    }
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, TEXT);
    focus_ring(ui, &response, rect, 6.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// `text` on one line, cut short with an ellipsis past `width`.
pub fn single_line(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// Text for a control, colored when painted.
fn control_text(ui: &Ui, text: &str) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(CONTROL_SIZE), Color32::PLACEHOLDER)
}

/// Shows which control has keyboard focus.
fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: f32) {
    if response.has_focus() {
        let stroke = Stroke::new(1.5, ACCENT.lerp_to_gamma(Color32::WHITE, 0.3));
        ui.painter().rect_stroke(rect.expand(2.0), radius + 2.0, stroke, StrokeKind::Outside);
    }
}
