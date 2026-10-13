//! The window: a floating panel (Popup), a panel behind other windows (Desktop), or a flyout
//! opened from the tray icons (Taskbar). It also hosts the settings window, and keeps itself
//! sized, placed and layered for the chosen mode.

use crate::claude::{self, Limit, Shared, State};
use crate::mascot::Mascot;
use crate::settings::{Mode, Settings, Style, View};
use crate::tray::{Tray, TrayConfig, TrayEvent};
use crate::view::{self, AMBER, DIM, TEXT};
use crate::win;
use chrono::{TimeDelta, Utc};
use eframe::egui::{
    self, Align, Color32, FontData, FontFamily, FontId, IconData, Layout, PointerButton, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, Vec2, ViewportBuilder, ViewportCommand, ViewportId,
    WindowLevel, pos2, vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MARGIN: f32 = 12.0;
const GAP: f32 = 10.0;
/// Wrap width for an error message when there are no meters to line up with.
const NOTE_WIDTH: f32 = 220.0;
const SETTINGS_WIDTH: f32 = 300.0;
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// After a mode change, the reported window position is trusted only once the move landed.
const SETTLE: Duration = Duration::from_millis(500);
/// A tray click this soon after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

const BG: Color32 = Color32::from_rgb(28, 28, 30);
const BORDER: Color32 = Color32::from_rgb(56, 56, 60);

/// A per-view on/off option in the settings grid.
type Toggle = fn(&mut View) -> &mut bool;

/// The panel in Taskbar mode: shown beside the taskbar when an icon is clicked, and hidden
/// again when it loses focus, like Windows' own flyouts.
#[derive(Clone, Copy, PartialEq)]
enum Flyout {
    Closed { at: Option<Instant> },
    /// Shown, waiting for focus. Holds the taskbar and the click, in points.
    Opening(Rect, Pos2),
    Open(Rect, Pos2),
}

pub struct Overlay {
    state: Shared,
    refresh: Sender<()>,
    tray: Tray,
    tray_events: Receiver<TrayEvent>,
    /// The taskbar settings last sent to the tray.
    tray_config: TrayConfig,
    settings: Settings,
    saved: Settings,
    save_at: Option<Instant>,
    mascot: Mascot,
    strong: FontFamily,
    icon: Arc<IconData>,
    keep_bottom: Arc<AtomicBool>,
    keeper_started: bool,
    /// The mode the window was last set up for, and when.
    applied: Option<(Mode, Instant)>,
    size: Vec2,
    flyout: Flyout,
    /// The frame in which the flyout was last shown.
    flyout_frame: u64,
    settings_open: bool,
    /// Settings asked for from the tray. eframe can only create the settings window once the
    /// panel is on screen, so this waits for the flyout's first frame.
    settings_requested: bool,
    settings_height: f32,
}

impl Overlay {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        settings: Settings,
        interval: TimeDelta,
        demo: bool,
        spark: &[u8],
        icon: Arc<IconData>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_theme(egui::Theme::Dark);
        // Selectable labels would swallow the drag that moves the window.
        ctx.all_styles_mut(|style| style.interaction.selectable_labels = false);
        let strong = setup_fonts(ctx);

        let state: Shared = Arc::new(Mutex::new(Arc::new(if demo { claude::demo_state() } else { State::default() })));
        let (tray, tray_events) = Tray::spawn(state.clone(), ctx.clone());
        let on_update = {
            let (ctx, tray) = (ctx.clone(), tray.clone());
            move || {
                ctx.request_repaint();
                tray.refresh();
            }
        };
        let (refresh, wake) = mpsc::channel();
        if demo {
            spawn_demo_toggle(state.clone(), on_update);
        } else {
            claude::spawn_poller(state.clone(), wake, interval, on_update);
        }

        Self {
            state,
            refresh,
            tray,
            tray_events,
            tray_config: TrayConfig::default(),
            saved: settings.clone(),
            settings,
            save_at: None,
            mascot: Mascot::new(ctx, spark),
            strong,
            icon,
            keep_bottom: Arc::new(AtomicBool::new(false)),
            keeper_started: false,
            applied: None,
            size: Vec2::ZERO,
            flyout: Flyout::Closed { at: None },
            flyout_frame: 0,
            settings_open: false,
            settings_requested: false,
            settings_height: 0.0,
        }
    }

    /// Sets the window up for a newly chosen mode.
    fn apply_mode(&mut self, ctx: &egui::Context) {
        let mode = self.settings.mode;
        if self.applied.is_some_and(|(applied, _)| applied == mode) {
            return;
        }
        let level = if mode == Mode::Desktop { WindowLevel::AlwaysOnBottom } else { WindowLevel::AlwaysOnTop };
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
        self.keep_bottom.store(mode == Mode::Desktop, Ordering::Relaxed);
        self.flyout = Flyout::Closed { at: None };
        ctx.send_viewport_cmd(ViewportCommand::Visible(mode != Mode::Taskbar));
        if mode != Mode::Taskbar
            && let Some([x, y]) = self.settings.panel_pos
        {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(x, y)));
        }
        self.applied = Some((mode, Instant::now()));
    }

    /// Tells the tray thread about changed taskbar settings.
    fn sync_tray(&mut self) {
        let enabled = self.settings.mode == Mode::Taskbar;
        let sent = &self.tray_config;
        if sent.enabled != enabled || sent.animate != self.settings.animate || sent.view != self.settings.taskbar {
            self.tray_config = TrayConfig { enabled, animate: self.settings.animate, view: self.settings.taskbar.clone() };
            self.tray.configure(self.tray_config.clone());
        }
    }

    /// A tray click opens the flyout, or closes it when it's already open.
    fn toggle_flyout(&mut self, ctx: &egui::Context) {
        match self.flyout {
            Flyout::Closed { at: Some(at) } if at.elapsed() < REOPEN_GUARD => {}
            Flyout::Closed { .. } => self.open_flyout(ctx),
            Flyout::Opening(..) | Flyout::Open(..) => {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                self.flyout = Flyout::Closed { at: Some(Instant::now()) };
            }
        }
    }

    fn open_flyout(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        let Some(bar) = win::taskbar_rect() else { return };
        let taskbar = Rect::from_min_max(pos2(bar.left as f32, bar.top as f32), pos2(bar.right as f32, bar.bottom as f32)) / ppp;
        let click = win::cursor();
        self.flyout = Flyout::Opening(taskbar, pos2(click.x as f32, click.y as f32) / ppp);
        self.flyout_frame = ctx.cumulative_frame_nr();
        self.place_flyout(ctx);
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    /// Beside the taskbar, centered on the click.
    fn place_flyout(&self, ctx: &egui::Context) {
        let (Flyout::Opening(bar, click) | Flyout::Open(bar, click)) = self.flyout else { return };
        let size = if self.size == Vec2::ZERO { vec2(300.0, 80.0) } else { self.size };
        let along = |click: f32, start: f32, end: f32, length: f32| {
            (click - length / 2.0).clamp(start + 8.0, (end - length - 8.0).max(start + 8.0))
        };
        let position = if bar.width() >= bar.height() {
            let y = if bar.top() > 0.0 { bar.top() - size.y - 8.0 } else { bar.bottom() + 8.0 };
            pos2(along(click.x, bar.left(), bar.right(), size.x), y)
        } else {
            let x = if bar.left() > 0.0 { bar.left() - size.x - 8.0 } else { bar.right() + 8.0 };
            pos2(x, along(click.y, bar.top(), bar.bottom(), size.y))
        };
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position.round()));
    }

    /// Closes the flyout once it loses focus, unless the settings window took it.
    fn track_flyout_focus(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        match self.flyout {
            Flyout::Opening(bar, click) if focused => self.flyout = Flyout::Open(bar, click),
            Flyout::Open(..) if !focused && !self.settings_open => {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                self.flyout = Flyout::Closed { at: Some(Instant::now()) };
            }
            _ => {}
        }
    }

    /// Clawd beside the meters, with the window sized to fit. Returns whether Clawd animates.
    fn panel(&mut self, ui: &mut egui::Ui, state: &State) -> bool {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let docked = self.settings.mode == Mode::Taskbar;

        // Drag to move (the flyout stays put); right-click for settings.
        let background = ui.interact(full, ui.id().with("background"), Sense::click_and_drag());
        if background.secondary_clicked() {
            self.settings_open = !self.settings_open;
        }
        if !docked && background.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let painter = ui.painter().clone();
        let alpha = (self.settings.opacity * 255.0) as u8;
        painter.rect_filled(full, 12, Color32::from_rgba_unmultiplied(BG.r(), BG.g(), BG.b(), alpha));
        painter.rect_stroke(full, 12, Stroke::new(1.0, BORDER), StrokeKind::Inside);

        // Lay everything out first so the window can be sized to fit.
        let view = &self.settings.panel;
        let limits: Vec<&Limit> = state.limits.iter().filter(|l| view.shows(&l.label)).collect();
        let builder = view::Builder { painter: &painter, strong: &self.strong, view, now: Utc::now() };
        let meters = builder.build(&limits);
        let note = note(state).map(|(text, color)| {
            painter.layout(text, FontId::proportional(12.0), color, meters.size.x.max(NOTE_WIDTH))
        });
        let clawd = if view.clawd { Mascot::size(1.0) } else { Vec2::ZERO };
        let animating = self.settings.animate && view.clawd;

        let note_size = note.as_ref().map_or(Vec2::ZERO, |n| n.size());
        let note_gap = if note.is_some() && meters.size.y > 0.0 { 4.0 } else { 0.0 };
        let body = vec2(meters.size.x.max(note_size.x), meters.size.y + note_gap + note_size.y);
        let gap = if clawd.x > 0.0 && body.x > 0.0 { GAP } else { 0.0 };
        let inner = vec2(clawd.x + gap + body.x, clawd.y.max(body.y));
        let size = (inner + Vec2::splat(2.0 * MARGIN)).round();
        if size != self.size {
            self.size = size;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            self.place_flyout(&ctx);
        }

        let origin = full.min + Vec2::splat(MARGIN);
        if clawd.x > 0.0 {
            let time = if self.settings.animate { ui.input(|i| i.time) } else { 0.0 };
            let at = origin + vec2(0.0, (inner.y - clawd.y) / 2.0);
            self.mascot.paint(&painter, Rect::from_min_size(at, clawd), state.working, time);
        }
        let body_at = origin + vec2(clawd.x + gap, (inner.y - body.y) / 2.0);
        let hints = meters.paint(&painter, body_at);
        if let Some(note) = note {
            painter.galley(body_at + vec2(0.0, body.y - note_size.y), note, TEXT);
        }
        for (i, (rect, hint)) in hints.into_iter().enumerate() {
            ui.interact(rect, ui.id().with(("hint", i)), Sense::hover()).on_hover_text(hint);
        }
        animating
    }

    /// Repaints only when something on screen changes: the next animation frame, or the next
    /// minute for the countdowns. Settings are saved a moment after the last change.
    fn schedule(&mut self, ctx: &egui::Context, next_frame: Option<Duration>) {
        let mut next = next_frame;
        if self.settings != self.saved {
            let now = Instant::now();
            let due = *self.save_at.get_or_insert(now + SAVE_DELAY);
            if now >= due {
                self.settings.save();
                self.saved = self.settings.clone();
                self.save_at = None;
            } else {
                next = Some(next.map_or(due - now, |n| n.min(due - now)));
            }
        }
        if let Some(next) = next {
            ctx.request_repaint_after(next);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context, state: &State) {
        let builder = ViewportBuilder::default()
            .with_title("Cuota settings")
            .with_inner_size([SETTINGS_WIDTH, self.settings_height.max(300.0)])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_always_on_top()
            .with_icon(self.icon.clone());
        ctx.show_viewport_immediate(ViewportId::from_hash_of("settings"), builder, |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.settings_open = false;
            }
            let frame = egui::Frame::central_panel(ui.style()).show(ui, |ui| self.settings_ui(ui, state));
            let height = frame.response.rect.height().ceil();
            if (height - self.settings_height).abs() >= 1.0 {
                self.settings_height = height;
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(vec2(SETTINGS_WIDTH, height)));
            }
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, state: &State) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let account = &state.account;
        let plan = account.plan.as_deref().unwrap_or("Claude");
        ui.label(RichText::new(plan).family(self.strong.clone()).size(16.0).color(TEXT));
        for line in [&account.email, &account.organization].into_iter().flatten() {
            ui.add(egui::Label::new(RichText::new(line).color(DIM)).truncate());
        }
        ui.horizontal(|ui| {
            let updated = state.updated.map_or_else(|| "Not updated yet".into(), |t| t.format("Updated %H:%M").to_string());
            ui.label(RichText::new(updated).color(DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Refresh").clicked() {
                    let _ = self.refresh.send(());
                }
            });
        });

        ui.separator();
        let settings = &mut self.settings;
        ui.horizontal(|ui| {
            ui.label("Show as");
            ui.radio_value(&mut settings.mode, Mode::Popup, "Popup");
            ui.radio_value(&mut settings.mode, Mode::Desktop, "Desktop");
            ui.radio_value(&mut settings.mode, Mode::Taskbar, "Taskbar");
        });
        let about = match settings.mode {
            Mode::Popup => "Floats above all windows. Drag to move.",
            Mode::Desktop => "Sits behind your windows, like a desktop widget. Drag to move.",
            Mode::Taskbar => {
                "Icons beside the clock; click one for this panel. New icons start in the ^ overflow, \
                 so drag them onto the taskbar once."
            }
        };
        ui.add(egui::Label::new(RichText::new(about).color(DIM).size(12.0)).wrap());

        ui.separator();
        let Settings { panel, taskbar, .. } = &mut *settings;
        let toggles: [(&str, Toggle); 3] = [
            ("Percentage", |v| &mut v.percent),
            ("Time left", |v| &mut v.time_left),
            ("Clawd", |v| &mut v.clawd),
        ];
        egui::Grid::new("views").num_columns(3).spacing([32.0, 6.0]).show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Panel").color(TEXT));
            ui.label(RichText::new("Taskbar").color(TEXT));
            ui.end_row();
            for limit in &state.limits {
                ui.label(&limit.label);
                for view in [&mut *panel, &mut *taskbar] {
                    let mut shown = view.shows(&limit.label);
                    if ui.checkbox(&mut shown, "").changed() {
                        view.set_shown(&limit.label, shown);
                    }
                }
                ui.end_row();
            }
            for (name, field) in toggles {
                ui.label(name);
                ui.checkbox(field(panel), "");
                ui.checkbox(field(taskbar), "");
                ui.end_row();
            }
            ui.label("Style");
            for view in [&mut *panel, &mut *taskbar] {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut view.style, Style::Bars, "Bars");
                    ui.selectable_value(&mut view.style, Style::Pie, "Pie");
                });
            }
            ui.end_row();
        });
        ui.label(RichText::new("On the taskbar, time left shows when you hover an icon.").color(DIM).size(12.0));

        ui.separator();
        ui.checkbox(&mut settings.animate, "Animate Clawd");
        ui.horizontal(|ui| {
            ui.label("Panel opacity");
            ui.add(egui::Slider::new(&mut settings.opacity, 0.3..=1.0).show_value(false));
        });

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Quit Cuota").clicked() {
                ui.ctx().send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(DIM));
            });
        });
    }
}

impl eframe::App for Overlay {
    /// Runs even while the window is hidden (Taskbar mode), so tray clicks get through.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.keeper_started
            && let Some(window) = native_window(frame)
        {
            win::keep_at_bottom(window, self.keep_bottom.clone());
            self.keeper_started = true;
        }
        self.apply_mode(ctx);
        self.sync_tray();
        while let Ok(TrayEvent::Open { settings }) = self.tray_events.try_recv() {
            if settings {
                // Right-click: settings, with the panel open beside them.
                self.settings_requested = true;
                if matches!(self.flyout, Flyout::Closed { .. }) {
                    self.open_flyout(ctx);
                }
                ctx.request_repaint();
            } else {
                self.toggle_flyout(ctx);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let state = self.state.lock().unwrap().clone();
        let docked = self.settings.mode == Mode::Taskbar;
        let visible = !docked || !matches!(self.flyout, Flyout::Closed { .. });

        let mut next_frame = None;
        if visible {
            let animating = self.panel(ui, &state);
            next_frame = Some(if animating {
                Mascot::frame_time(state.working)
            } else {
                Duration::from_millis(60_000 - Utc::now().timestamp_millis().rem_euclid(60_000) as u64)
            });
            if docked {
                if self.settings_requested && ctx.cumulative_frame_nr() > self.flyout_frame {
                    self.settings_requested = false;
                    self.settings_open = true;
                }
                self.track_flyout_focus(&ctx);
            } else if self.applied.is_some_and(|(_, at)| at.elapsed() > SETTLE)
                && let Some(rect) = ctx.input(|i| i.viewport().outer_rect)
            {
                self.settings.panel_pos = Some([rect.min.x, rect.min.y]);
            }
        }

        if self.settings_open {
            self.settings_window(&ctx, &state);
        }
        self.schedule(&ctx, next_frame);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.tray.close();
        if self.settings != self.saved {
            self.settings.save();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

/// A status line under the meters: the last error, or "Loading..." before any data.
fn note(state: &State) -> Option<(String, Color32)> {
    match (&state.error, state.limits.is_empty()) {
        (Some(error), _) => Some((error.clone(), AMBER)),
        (None, true) => Some(("Loading...".into(), DIM)),
        (None, false) => None,
    }
}

fn native_window(frame: &eframe::Frame) -> Option<isize> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// `--demo`: flips between working and idle so both animations can be previewed.
fn spawn_demo_toggle(state: Shared, on_update: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(6));
            {
                let mut shared = state.lock().unwrap();
                let s = Arc::make_mut(&mut shared);
                s.working = !s.working;
            }
            on_update();
        }
    });
}

/// Windows' own UI font (egui's bundled fonts are left out of the build).
/// Returns the family for emphasized text.
fn setup_fonts(ctx: &egui::Context) -> FontFamily {
    let dir = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
    let load = |names: &[&str]| names.iter().find_map(|name| std::fs::read(dir.join(name)).ok());
    let mut fonts = egui::FontDefinitions::empty();
    let regular = load(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]).expect("Windows has a UI font");
    fonts.font_data.insert("regular".into(), Arc::new(FontData::from_owned(regular)));
    let mut strong_fonts = vec!["regular".to_owned()];
    if let Some(bytes) = load(&["seguisb.ttf", "arialbd.ttf"]) {
        fonts.font_data.insert("semibold".into(), Arc::new(FontData::from_owned(bytes)));
        strong_fonts.insert(0, "semibold".into());
    }
    let strong = FontFamily::Name("strong".into());
    fonts.families.insert(FontFamily::Proportional, vec!["regular".into()]);
    fonts.families.insert(FontFamily::Monospace, vec!["regular".into()]);
    fonts.families.insert(strong.clone(), strong_fonts);
    ctx.set_fonts(fonts);
    strong
}
