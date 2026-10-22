//! Cuota on more than one monitor: the monitors Windows has, numbered as the settings list
//! them, and Identify, which shows each one's number on it for a moment, as Windows' own
//! display settings do, so you can tell which is which.

use crate::overlay::{CORNER, repaint_after, show_immediate};
use crate::theme::Palette;
use crate::widgets::Fonts;
use crate::win;
use eframe::egui::{
    Align2, Color32, Context, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2, ViewportBuilder, ViewportCommand, ViewportId,
    WindowLevel, pos2, vec2,
};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::RECT;

/// How long Identify shows the numbers.
const IDENTIFY_TIME: Duration = Duration::from_secs(3);
/// Each number's card, in points, and how far it sits from its monitor's bottom-left corner.
const CARD: Vec2 = vec2(136.0, 136.0);
const INSET: f32 = 32.0;
const NUMBER: f32 = 88.0;

/// A monitor, numbered as the settings list them. Rectangles are in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// Windows' name for it, like `\\.\DISPLAY2`, which settings keep it by.
    pub name: String,
    /// 1 for the first in the list, 2 for the next, and so on.
    pub number: usize,
    /// All of it, and the part the taskbar leaves free.
    pub area: Rect,
    pub work: Rect,
    /// The main display.
    pub primary: bool,
    /// Physical pixels per point on it.
    pub scale: f32,
}

impl Screen {
    /// Like "Monitor 2".
    pub fn title(&self) -> String {
        format!("Monitor {}", self.number)
    }

    /// What tells it apart from the others in `all`: its size, and where it is, like
    /// "1920 × 1080 · Left of the main display".
    pub fn about(&self, all: &[Screen]) -> String {
        let size = format!("{} \u{D7} {}", self.area.width().round(), self.area.height().round());
        let main = all.iter().find(|screen| screen.primary);
        let place = match main {
            _ if self.primary => "Main display".to_owned(),
            Some(main) => {
                let off = self.area.center() - main.area.center();
                let side = match (off.x.abs() >= off.y.abs(), off.x < 0.0, off.y < 0.0) {
                    (true, true, _) => "Left of",
                    (true, false, _) => "Right of",
                    (false, _, true) => "Above",
                    (false, _, false) => "Below",
                };
                format!("{side} the main display")
            }
            None => return size,
        };
        format!("{size} \u{B7} {place}")
    }
}

/// The monitors plugged in now, numbered from 1.
pub fn list() -> Vec<Screen> {
    let rect = |r: RECT| Rect::from_min_max(pos2(r.left as f32, r.top as f32), pos2(r.right as f32, r.bottom as f32));
    win::monitors()
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| Screen {
            name: monitor.name,
            number: index + 1,
            area: rect(monitor.area),
            work: rect(monitor.work),
            primary: monitor.primary,
            scale: monitor.scale,
        })
        .collect()
}

/// The monitor a point is on, in pixels, or else the one nearest it.
pub fn at(screens: &[Screen], point: Pos2) -> Option<&Screen> {
    let off = |screen: &&Screen| screen.area.distance_sq_to_pos(point);
    screens.iter().min_by(|a, b| off(a).total_cmp(&off(b)))
}

/// Like "Monitors 1, 2 and 3", for the monitors numbered `numbers`.
pub fn summary(numbers: &[usize]) -> String {
    let words: Vec<String> = numbers.iter().map(usize::to_string).collect();
    match words.as_slice() {
        [] => "None".to_owned(),
        [one] => format!("Monitor {one}"),
        [rest @ .., last] => format!("Monitors {} and {last}", rest.join(", ")),
    }
}

/// Each monitor's number on it for a few seconds, in a window of its own.
#[derive(Default)]
pub struct Identify {
    /// Since when, and the monitors as they were then.
    showing: Option<(Instant, Vec<Screen>)>,
    /// The monitors whose number's window is set up, and whose number is on screen yet. Each
    /// window stays hidden until it's drawn where it goes.
    set_up: HashSet<String>,
    shown: HashSet<String>,
}

impl Identify {
    pub fn start(&mut self) {
        self.showing = Some((Instant::now(), list()));
        self.set_up.clear();
        self.shown.clear();
    }

    /// Shows the numbers while they're due. Their windows close once this stops showing them,
    /// so it's called every time the panel runs.
    pub fn show(&mut self, ctx: &Context, fonts: &Fonts) {
        let Self { showing, set_up, shown } = self;
        let Some((since, screens)) = showing else { return };
        let left = IDENTIFY_TIME.saturating_sub(since.elapsed());
        if left.is_zero() {
            *showing = None;
            return;
        }
        repaint_after(ctx, left);
        for screen in screens.iter() {
            let title = format!("Cuota: {}", screen.title());
            let builder = ViewportBuilder::default()
                .with_title(&title)
                .with_active(false)
                .with_inner_size(CARD)
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_taskbar(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_visible(false);
            let id = ViewportId::from_hash_of(("identify", &screen.name));
            show_immediate(ctx, id, builder, |ui| {
                let ctx = ui.ctx().clone();
                // Like the desktop pet's window, it never takes the keyboard, and tools that
                // move new windows about leave it be.
                if !set_up.contains(&screen.name)
                    && let Some(window) = win::own_window(&title)
                {
                    win::set_up_pet(window);
                    set_up.insert(screen.name.clone());
                }
                // In the monitor's bottom-left corner, as Windows' own Identify puts it. Its
                // pixels, in this window's points, which are its monitor's once it's there.
                let ppp = ctx.pixels_per_point();
                let corner = pos2(screen.work.left() + INSET * ppp, screen.work.bottom() - (INSET + CARD.y) * ppp);
                let seen = ctx.input(|i| i.viewport().outer_rect);
                if seen.is_some_and(|rect| (rect.min * ppp - corner).length() <= 1.5) {
                    if shown.insert(screen.name.clone()) {
                        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    }
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition((corner / ppp).round()));
                    ctx.request_repaint();
                }
                let rect = ui.max_rect();
                let painter = ui.painter();
                // Dark whatever the panel's theme, like the settings window that shows them.
                let dark = Palette::DARK;
                painter.rect_filled(rect, CORNER, dark.background);
                painter.rect_stroke(rect, CORNER, Stroke::new(1.0, dark.border), StrokeKind::Inside);
                let font = FontId::new(NUMBER, fonts.strong.clone());
                painter.text(rect.center(), Align2::CENTER_CENTER, screen.number, font, Color32::WHITE);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(number: usize, area: Rect, primary: bool) -> Screen {
        let work = Rect::from_min_max(area.min, area.max - vec2(0.0, 48.0));
        Screen { name: format!(r"\\.\DISPLAY{number}"), number, area, work, primary, scale: 1.0 }
    }

    #[test]
    fn each_monitor_says_how_big_it_is_and_where_it_is_from_the_main_one() {
        let main = screen(1, Rect::from_min_size(Pos2::ZERO, vec2(2560.0, 1440.0)), true);
        let left = screen(2, Rect::from_min_size(pos2(-1920.0, 200.0), vec2(1920.0, 1080.0)), false);
        let above = screen(3, Rect::from_min_size(pos2(300.0, -1080.0), vec2(1920.0, 1080.0)), false);
        let all = [main.clone(), left.clone(), above.clone()];
        assert_eq!(main.about(&all), "2560 \u{D7} 1440 \u{B7} Main display");
        assert_eq!(left.about(&all), "1920 \u{D7} 1080 \u{B7} Left of the main display");
        assert_eq!(above.about(&all), "1920 \u{D7} 1080 \u{B7} Above the main display");
        assert_eq!(left.title(), "Monitor 2");
        // A point between them, or off them all, is on the nearest.
        assert_eq!(at(&all, pos2(-10.0, 500.0)).map(|s| s.number), Some(2));
        assert_eq!(at(&all, pos2(5000.0, 100.0)).map(|s| s.number), Some(1));
        assert_eq!(at(&[], Pos2::ZERO), None);
    }

    #[test]
    fn a_summary_names_the_monitors_picked() {
        assert_eq!(summary(&[]), "None");
        assert_eq!(summary(&[2]), "Monitor 2");
        assert_eq!(summary(&[1, 2]), "Monitors 1 and 2");
        assert_eq!(summary(&[1, 3, 4]), "Monitors 1, 3 and 4");
    }
}
