//! The window: a floating panel (Popup), a panel behind other windows (Desktop), or a flyout
//! opened from the tray icons (Taskbar alone). It also hosts the settings window, and keeps
//! itself sized, placed and layered for the chosen modes.

use crate::claude::{self, Limit, Shared, State};
use crate::mascot::Mascot;
use crate::settings::{Mode, Settings, Style};
use crate::tray::{Tray, TrayConfig, TrayEvent};
use crate::view::{self, AMBER, DIM, TEXT};
use crate::widgets::{self, Fonts, icon};
use crate::win;
use chrono::{TimeDelta, Utc};
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontFamily, FontId, Galley, IconData, Label, Layout,
    Painter, PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, Vec2,
    ViewportBuilder, ViewportCommand, ViewportId, WindowLevel, pos2,
    text::{LayoutJob, TextWrapping},
    vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MARGIN: f32 = 12.0;
const GAP: f32 = 10.0;
/// Wrap width for an error message when there are no meters to line up with.
const NOTE_WIDTH: f32 = 220.0;
const SETTINGS_WIDTH: f32 = 340.0;
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// After a Show as change, the reported window position is trusted only once the move landed.
const SETTLE: Duration = Duration::from_millis(500);
/// A tray click this soon after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

const BG: Color32 = Color32::from_rgb(28, 28, 30);
const BORDER: Color32 = Color32::from_rgb(56, 56, 60);

/// Which view the settings' Display section edits.
#[derive(Clone, Copy, PartialEq)]
enum Surface {
    Panel,
    Taskbar,
}

impl Surface {
    /// The one the user most likely means to change: the panel, unless only the icons show.
    fn of(settings: &Settings) -> Self {
        if settings.pinned().is_none() { Self::Taskbar } else { Self::Panel }
    }
}

/// The panel with only Taskbar on: shown beside the taskbar when an icon is clicked, and hidden
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
    fonts: Fonts,
    icon: Arc<IconData>,
    keep_bottom: Arc<AtomicBool>,
    /// Whether the native window has been fixed up and handed to the Desktop-mode keeper.
    window_ready: bool,
    /// The Show as choice the window was last set up for, and when.
    applied: Option<(BTreeSet<Mode>, Instant)>,
    size: Vec2,
    flyout: Flyout,
    /// The Popup or Desktop panel was hidden with a click on a tray icon.
    hidden: bool,
    /// The frame in which the flyout or a hidden panel was last shown.
    shown_frame: u64,
    settings_open: bool,
    /// Settings asked for from the tray. eframe can only create the settings window once the
    /// panel is on screen, so this waits for the panel's first frame.
    settings_requested: bool,
    /// Whether the settings window is on screen yet. A new one stays hidden until it has drawn
    /// itself at its full height, or Windows shows it blank white for a moment.
    settings_shown: bool,
    settings_height: f32,
    settings_surface: Surface,
    /// Whether Windows starts Cuota at sign-in, as last read from the registry.
    starts_with_windows: bool,
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
        let fonts = setup_fonts(ctx);

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
            fonts,
            icon,
            keep_bottom: Arc::new(AtomicBool::new(false)),
            window_ready: false,
            applied: None,
            size: Vec2::ZERO,
            flyout: Flyout::Closed { at: None },
            hidden: false,
            shown_frame: 0,
            settings_open: false,
            settings_requested: false,
            settings_shown: false,
            settings_height: 0.0,
            settings_surface: Surface::Panel,
            starts_with_windows: false,
        }
    }

    /// Sets the window up for a newly chosen set of modes.
    fn apply_modes(&mut self, ctx: &egui::Context) {
        if self.applied.as_ref().is_some_and(|(applied, _)| *applied == self.settings.show) {
            return;
        }
        let pinned = self.settings.pinned();
        let desktop = pinned == Some(Mode::Desktop);
        let level = if desktop { WindowLevel::AlwaysOnBottom } else { WindowLevel::AlwaysOnTop };
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
        self.keep_bottom.store(desktop, Ordering::Relaxed);
        self.flyout = Flyout::Closed { at: None };
        // Also brings back a panel hidden from the tray, in case the tray icons were just turned off.
        self.hidden = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(pinned.is_some()));
        if pinned.is_some()
            && let Some([x, y]) = self.settings.panel_pos
        {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(x, y)));
        }
        self.applied = Some((self.settings.show.clone(), Instant::now()));
    }

    /// Tells the tray thread about changed taskbar settings.
    fn sync_tray(&mut self) {
        let enabled = self.settings.shows_as(Mode::Taskbar);
        let sent = &self.tray_config;
        if sent.enabled != enabled || sent.animate != self.settings.animate || sent.view != self.settings.taskbar {
            self.tray_config = TrayConfig { enabled, animate: self.settings.animate, view: self.settings.taskbar.clone() };
            self.tray.configure(self.tray_config.clone());
        }
    }

    /// A tray click when Popup or Desktop keeps the panel on screen: hides it, or shows it again.
    fn set_hidden(&mut self, ctx: &egui::Context, hidden: bool) {
        if hidden != self.hidden {
            self.hidden = hidden;
            self.shown_frame = ctx.cumulative_frame_nr();
            ctx.send_viewport_cmd(ViewportCommand::Visible(!hidden));
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
        self.shown_frame = ctx.cumulative_frame_nr();
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
        let docked = self.settings.pinned().is_none();

        // Drag to move (the flyout stays put); right-click for settings.
        let background = ui.interact(full, ui.id().with("background"), Sense::click_and_drag());
        if background.secondary_clicked() {
            if self.settings_open {
                self.settings_open = false;
            } else {
                self.open_settings(&ctx);
            }
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
        let builder = view::Builder { painter: &painter, strong: &self.fonts.strong, view, now: Utc::now() };
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

    fn open_settings(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            self.settings_shown = false;
        }
        self.settings_open = true;
        self.settings_surface = Surface::of(&self.settings);
        self.starts_with_windows = win::starts_with_windows();
        // Already open, maybe behind other windows.
        ctx.send_viewport_cmd_to(settings_viewport(), ViewportCommand::Focus);
    }

    fn settings_window(&mut self, ctx: &egui::Context, state: &State) {
        let builder = ViewportBuilder::default()
            .with_title("Cuota settings")
            .with_inner_size([SETTINGS_WIDTH, self.settings_height.max(300.0)])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_always_on_top()
            .with_visible(false)
            .with_icon(self.icon.clone());
        ctx.show_viewport_immediate(settings_viewport(), builder, |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.settings_open = false;
            }
            ui.painter().rect_filled(ui.max_rect(), 0, widgets::WINDOW);
            // Scrolls rather than running off a short screen, leaving room for the title bar and taskbar.
            let fits = ui.ctx().input(|i| i.viewport().monitor_size).map_or(f32::INFINITY, |size| size.y - 100.0);
            let scroll = egui::ScrollArea::vertical().max_height(fits).show(ui, |ui| {
                egui::Frame::new().inner_margin(16).show(ui, |ui| self.settings_ui(ui, state));
            });
            let height = scroll.content_size.y.min(fits).ceil();
            if (height - self.settings_height).abs() >= 1.0 {
                self.settings_height = height;
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(vec2(SETTINGS_WIDTH, height)));
                // Drawn at the old height, so a hidden window waits for the next frame.
                ui.ctx().request_repaint();
            } else if !self.settings_shown {
                // These run once this frame is on the window, so it appears already drawn.
                self.settings_shown = true;
                ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
                ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
            }
        });
    }

    /// The account on top, then General (where and how Cuota shows) and Display (what the
    /// panel or the taskbar icons show), and Quit at the bottom.
    fn settings_ui(&mut self, ui: &mut egui::Ui, state: &State) {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        self.settings_header(ui, state);
        let Self { settings, settings_surface: surface, fonts, starts_with_windows, .. } = self;

        ui.add_space(16.0);
        widgets::heading(ui, fonts, "General", |_| {});
        ui.add_space(4.0);
        widgets::card(ui, |ui| {
            let about = show_as_about(settings);
            widgets::row_below(ui, fonts, icon::SHOW_AS, "Show as", Some(&about), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (mode, name) in [(Mode::Popup, "Popup"), (Mode::Desktop, "Desktop"), (Mode::Taskbar, "Taskbar")] {
                        let mut shown = settings.shows_as(mode);
                        // The last one stays on, so there's always something to right-click for settings.
                        let last = shown && settings.show.len() == 1;
                        let chip = ui.add_enabled_ui(!last, |ui| widgets::chip(ui, fonts, &mut shown, name)).inner;
                        if chip.on_disabled_hover_text("At least one stays on").changed() {
                            settings.set_shown_as(mode, shown);
                            // Show the options for what was just turned on.
                            *surface = match (mode, shown) {
                                (Mode::Taskbar, true) => Surface::Taskbar,
                                (_, true) => Surface::Panel,
                                (_, false) => Surface::of(settings),
                            };
                        }
                    }
                });
            });
            widgets::divider(ui, fonts);
            let about = Some("Opens Cuota when you sign in");
            widgets::row(ui, fonts, icon::STARTUP, "Start with Windows", about, widgets::TOGGLE_WIDTH, |ui| {
                let mut on = *starts_with_windows;
                if widgets::toggle(ui, &mut on).changed() {
                    win::set_start_with_windows(on);
                    *starts_with_windows = win::starts_with_windows();
                }
            });
            widgets::divider(ui, fonts);
            let opacity = format!("{:.0}%", settings.opacity * 100.0);
            widgets::row(ui, fonts, icon::OPACITY, "Opacity", Some("Panel background"), 134.0, |ui| {
                ui.add_sized([34.0, 20.0], Label::new(RichText::new(opacity).size(12.0).color(DIM)));
                widgets::slider(ui, &mut settings.opacity, 0.3..=1.0, 92.0);
            });
            widgets::divider(ui, fonts);
            let about = Some("Turn off to use almost no CPU");
            widgets::row(ui, fonts, icon::ANIMATE, "Animate Clawd", about, widgets::TOGGLE_WIDTH, |ui| {
                widgets::toggle(ui, &mut settings.animate);
            });
        });

        ui.add_space(16.0);
        widgets::heading(ui, fonts, "Display", |ui| {
            widgets::segmented(ui, surface, &[(Surface::Panel, "Panel"), (Surface::Taskbar, "Taskbar")], None);
        });
        let taskbar = *surface == Surface::Taskbar;
        let note = match (taskbar, settings.shows_as(Mode::Taskbar), settings.pinned()) {
            (true, false, _) => Some("Used when Show as includes Taskbar."),
            (false, _, None) => Some("The panel opens when you click a taskbar icon."),
            _ => None,
        };
        if let Some(note) = note {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.add(Label::new(RichText::new(note).size(11.5).color(DIM)).wrap());
            });
        }
        ui.add_space(4.0);
        let view = if taskbar { &mut settings.taskbar } else { &mut settings.panel };
        widgets::card(ui, |ui| {
            let styles = [(Style::Bars, "Bars"), (Style::Pie, "Pie")];
            let width = widgets::segmented_width(ui, &styles.map(|(_, name)| name));
            widgets::row(ui, fonts, icon::STYLE, "Style", None, width, |ui| {
                widgets::segmented(ui, &mut view.style, &styles, None);
            });
            widgets::divider(ui, fonts);
            widgets::row_below(ui, fonts, icon::LIMITS, "Limits", Some("Pick the ones to show"), |ui| {
                if state.limits.is_empty() {
                    ui.label(RichText::new("They appear here once usage has loaded.").size(11.5).color(DIM));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for limit in &state.limits {
                        let mut shown = view.shows(&limit.label);
                        if widgets::chip(ui, fonts, &mut shown, &limit.label).changed() {
                            view.set_shown(&limit.label, shown);
                        }
                    }
                });
            });
            let (time_left, clawd) = if taskbar {
                ("Shows when you hover an icon", "Shows if Claude is working")
            } else {
                ("Until each limit resets", "Dances while Claude works")
            };
            for (glyph, title, about, on) in [
                (icon::PERCENT, "Percentage", None, &mut view.percent),
                (icon::TIME_LEFT, "Time left", Some(time_left), &mut view.time_left),
                (icon::CLAWD, "Clawd", Some(clawd), &mut view.clawd),
            ] {
                widgets::divider(ui, fonts);
                widgets::row(ui, fonts, glyph, title, about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, on);
                });
            }
        });

        ui.add_space(16.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let mut slot = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
        if widgets::button(&mut slot, fonts, icon::QUIT, "Quit Cuota").clicked() {
            ui.ctx().send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        }
        let version = concat!("Cuota v", env!("CARGO_PKG_VERSION"));
        ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, version, FontId::proportional(11.5), DIM);
    }

    /// The Claude spark and the account, with when usage was last fetched and a button to
    /// fetch it now.
    fn settings_header(&self, ui: &mut egui::Ui, state: &State) {
        const LOGO: f32 = 36.0;
        const BUTTON: f32 = 28.0;
        let account = &state.account;
        let painter = ui.painter().clone();
        let width = ui.available_width();

        let updated = state.updated.map_or_else(|| "Not updated yet".into(), |t| t.format("Updated %H:%M").to_string());
        let updated = painter.layout_no_wrap(updated, FontId::proportional(11.5), DIM);
        let text_left = LOGO + 12.0;
        let text_width = width - text_left - updated.size().x - BUTTON - 18.0;
        let plan = account.plan.as_deref().unwrap_or("Claude");
        let mut lines = vec![single_line(&painter, plan, FontId::new(16.0, self.fonts.strong.clone()), TEXT, text_width)];
        for detail in [&account.email, &account.organization].into_iter().flatten() {
            lines.push(single_line(&painter, detail, FontId::proportional(12.0), DIM, text_width));
        }
        let text_height: f32 = lines.iter().map(|line| line.size().y).sum();
        let (rect, _) = ui.allocate_exact_size(vec2(width, text_height.max(LOGO)), Sense::hover());

        self.mascot.paint_logo(&painter, pos2(rect.left() + LOGO / 2.0, rect.center().y), LOGO / 2.0 - 3.0);
        let mut y = rect.center().y - text_height / 2.0;
        for line in lines {
            let height = line.size().y;
            painter.galley(pos2(rect.left() + text_left, y), line, TEXT);
            y += height;
        }
        let button = Rect::from_center_size(pos2(rect.right() - BUTTON / 2.0, rect.center().y), vec2(BUTTON, BUTTON));
        let at = pos2(button.left() - 6.0 - updated.size().x, rect.center().y - updated.size().y / 2.0);
        painter.galley(at, updated, DIM);
        let mut slot = ui.new_child(UiBuilder::new().max_rect(button));
        if widgets::icon_button(&mut slot, &self.fonts, icon::REFRESH, "Refresh").on_hover_text("Fetch usage now").clicked() {
            let _ = self.refresh.send(());
        }
    }
}

impl eframe::App for Overlay {
    /// Runs even while the window is hidden, so tray clicks get through.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.window_ready
            && let Some(window) = native_window(frame)
        {
            win::fill_window(window);
            win::keep_at_bottom(window, self.keep_bottom.clone());
            self.window_ready = true;
        }
        self.apply_modes(ctx);
        self.sync_tray();
        while let Ok(event) = self.tray_events.try_recv() {
            let settings = match event {
                TrayEvent::Click => false,
                TrayEvent::Settings => true,
                TrayEvent::Quit => {
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                    continue;
                }
            };
            if self.settings.pinned().is_some() {
                // A click hides or shows the panel; settings bring it back if it was hidden.
                self.set_hidden(ctx, !settings && !self.hidden);
                self.settings_requested |= settings;
                ctx.request_repaint();
            } else if settings {
                // Settings, with the panel open beside them.
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
        let docked = self.settings.pinned().is_none();
        let visible = if docked { !matches!(self.flyout, Flyout::Closed { .. }) } else { !self.hidden };

        let mut next_frame = None;
        if visible {
            let animating = self.panel(ui, &state);
            next_frame = Some(if animating {
                Mascot::frame_time(state.working)
            } else {
                Duration::from_millis(60_000 - Utc::now().timestamp_millis().rem_euclid(60_000) as u64)
            });
            if self.settings_requested && ctx.cumulative_frame_nr() > self.shown_frame {
                self.settings_requested = false;
                self.open_settings(&ctx);
            }
            if docked {
                self.track_flyout_focus(&ctx);
            } else if self.applied.as_ref().is_some_and(|(_, at)| at.elapsed() > SETTLE)
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

fn settings_viewport() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

/// What the chosen Show as options do.
fn show_as_about(settings: &Settings) -> String {
    let mut about = match settings.pinned() {
        Some(Mode::Popup) => "Floats above all windows. Drag to move.".to_owned(),
        Some(Mode::Desktop) => "Sits behind your windows, like a desktop widget. Drag to move.".to_owned(),
        _ => String::new(),
    };
    if settings.shows_as(Mode::Taskbar) {
        let click = if settings.pinned().is_some() { "to hide or show this panel" } else { "for this panel" };
        about += &format!(
            " Icons beside the clock; click one {click}. New icons start in the ^ overflow, so drag \
             them onto the taskbar once."
        );
    }
    about.trim_start().to_owned()
}

/// `text` on one line, cut short with an ellipsis past `width`.
fn single_line(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
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

/// Windows' own UI and icon fonts (egui's bundled fonts are left out of the build).
fn setup_fonts(ctx: &egui::Context) -> Fonts {
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
    // Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10.
    let icons = load(&["SegoeIcons.ttf", "segmdl2.ttf"]).map(|bytes| {
        fonts.font_data.insert("icons".into(), Arc::new(FontData::from_owned(bytes)));
        let family = FontFamily::Name("icons".into());
        fonts.families.insert(family.clone(), vec!["icons".into()]);
        family
    });
    ctx.set_fonts(fonts);
    Fonts { strong, icons }
}
