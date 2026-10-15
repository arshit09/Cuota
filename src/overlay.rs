//! The window: a floating panel (Always on top), a panel behind other windows (Desktop), or a flyout
//! opened from the tray icons (Taskbar alone). Double-clicking it expands it with usage
//! details. It also hosts the settings window, and keeps itself sized, placed and layered for
//! the chosen modes.

use crate::animations::{self, Clawd};
use crate::claude::{self, Limit, Shared, State, Step};
use crate::details;
use crate::glance;
use crate::log;
use crate::mascot::Mascot;
use crate::scripts::{self, Scripts};
use crate::settings::{DoubleClick, Mode, SIZES, Settings, SoundMode, Style};
use crate::sound::{self, Cues};
use crate::stats::Transcripts;
use crate::status::Status;
use crate::tray::{Tray, TrayConfig, TrayEvent};
use crate::view::{self, AMBER, DIM, TEXT};
use crate::widgets::{self, Fonts, SortRow, Sorted, icon, single_line};
use crate::win;
use chrono::{TimeDelta, Utc};
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontFamily, FontId, IconData, Id, Label, Layout, Margin, MouseWheelUnit,
    PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId, WindowLevel, pos2, vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MARGIN: f32 = 12.0;
/// The panel's corner radius.
const CORNER: f32 = 12.0;
const GAP: f32 = 10.0;
/// Between Clawd's status and the meters under it.
const STATUS_GAP: f32 = 4.0;
/// Wrap width for an error message, or the details picked to show under the meters, when the
/// meters are narrower.
const NOTE_WIDTH: f32 = 220.0;
/// Between the meters and those details.
const GLANCE_GAP: f32 = 6.0;
/// The settings window, a landscape one like other Windows apps' settings: this big when the
/// screen has room, and resizable down to `SETTINGS_MIN`. A sidebar lists the pages, and the
/// one shown runs no wider than `PAGE_WIDTH`.
const SETTINGS_SIZE: Vec2 = vec2(800.0, 600.0);
const SETTINGS_MIN: Vec2 = vec2(640.0, 420.0);
const SIDEBAR_WIDTH: f32 = 248.0;
const PAGE_WIDTH: f32 = 760.0;
/// The page's title, above what scrolls, and the room either side of the page.
const PAGE_TITLE: f32 = 68.0;
const PAGE_GUTTER: f32 = 16.0;
/// Where Cuota's code lives, which the sidebar links to.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// Between a page's cards.
const CARD_GAP: f32 = 4.0;
/// A slider and its value beside it.
const SLIDER_WIDTH: f32 = 140.0;
const VALUE_WIDTH: f32 = 40.0;
const SLIDER_ROW: f32 = SLIDER_WIDTH + 8.0 + VALUE_WIDTH;
/// What a window's title bar and borders add to its size, until the settings window can
/// measure its own.
const TITLE_BAR: Vec2 = vec2(16.0, 39.0);
/// The expanded panel's details: a fixed width plus a lane for the scroll bar, which egui would
/// otherwise draw over them, and scrolling past this height.
const SCROLL_LANE: f32 = 10.0;
const DETAILS_WIDTH: f32 = 348.0 + SCROLL_LANE;
const DETAILS_HEIGHT: f32 = 520.0;
/// How often the panel reads what Claude Code has done since it last looked, while it shows
/// any of it.
const STATS_REFRESH: Duration = Duration::from_secs(15);
/// Keeps an expanded panel this far from its screen's edges.
const SCREEN_GAP: f32 = 8.0;
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// After a Show as change, the reported window position is trusted only once the move landed.
const SETTLE: Duration = Duration::from_millis(500);
/// A tray click this soon after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

const BG: Color32 = Color32::from_rgb(28, 28, 30);
const BORDER: Color32 = Color32::from_rgb(56, 56, 60);

/// The card under a Custom double-click: what it opens, and a Browse button to pick it. The
/// Open dialog runs on a thread of its own, so what it picked arrives on a later frame.
fn custom_target(
    ui: &mut egui::Ui,
    fonts: &Fonts,
    settings: &mut Settings,
    browsing: &mut Option<Receiver<Option<String>>>,
) {
    match browsing.as_ref().map(Receiver::try_recv) {
        Some(Ok(Some(path))) => {
            settings.custom_target = path;
            *browsing = None;
        }
        Some(Ok(None) | Err(TryRecvError::Disconnected)) => *browsing = None,
        Some(Err(TryRecvError::Empty)) | None => {}
    }
    // Its name, and the folder it's in under that.
    let path = Path::new(&settings.custom_target);
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
    let folder = path.parent().map(|folder| folder.to_string_lossy().into_owned());
    let (title, about) = match &name {
        Some(name) => (name.as_str(), folder.as_deref().filter(|folder| !folder.is_empty())),
        None => ("Nothing picked yet", Some("Pick a file, program or script to open")),
    };
    let width = widgets::button_width(ui, fonts, "Browse…");
    let card = widgets::setting(ui, fonts, icon::BROWSE, title, about, width, |ui| {
        let browse = ui.add_enabled_ui(browsing.is_none(), |ui| widgets::button(ui, fonts, icon::BROWSE, "Browse…")).inner;
        if browse.clicked() {
            let (picked, dialog) = mpsc::channel();
            let ctx = ui.ctx().clone();
            win::browse_file(&settings.custom_target, move |path| {
                let _ = picked.send(path);
                ctx.request_repaint();
            });
            *browsing = Some(dialog);
        }
    });
    if name.is_some() {
        card.on_hover_text(&settings.custom_target);
    }
}

/// A page of the settings window, picked in its sidebar.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Page {
    /// Where Cuota shows, what double-clicking it does, and starting with Windows.
    General,
    /// The panel's size and opacity, Clawd's animation, and the sound effects.
    Appearance,
    /// What the panel shows.
    Panel,
    /// More the panel shows under its meters, and in what order.
    Details,
    /// What the taskbar icons show.
    Taskbar,
}

impl Page {
    const ALL: [Self; 5] = [Self::General, Self::Appearance, Self::Panel, Self::Details, Self::Taskbar];

    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Panel => "Panel",
            Self::Details => "Details",
            Self::Taskbar => "Taskbar",
        }
    }

    fn glyph(self) -> char {
        match self {
            Self::General => icon::SETTINGS,
            Self::Appearance => icon::APPEARANCE,
            Self::Panel => icon::PANEL,
            Self::Details => icon::DETAILS,
            Self::Taskbar => icon::TASKBAR,
        }
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

/// Resizing the panel by turning the mouse wheel over it.
#[derive(Default)]
struct Wheel {
    /// Turned over the panel but not yet a whole size step, in lines.
    lines: f32,
    /// How far the last resize moves the panel, so what's under the pointer stays put. Taken by
    /// the frame that resizes it.
    shift: Option<Vec2>,
}

impl Wheel {
    /// Steps the size up or down a step a notch, unless the pointer is off the panel or over its
    /// `details`, which the wheel scrolls instead. The next frame draws the new size.
    fn resize(&mut self, ui: &egui::Ui, details: Option<Rect>, settings: &mut Settings) {
        let full = ui.max_rect();
        let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
        let Some(pointer) = pointer.filter(|&at| full.contains(at) && !details.is_some_and(|area| area.contains(at)))
        else {
            self.lines = 0.0;
            return;
        };
        // A notch is a line on most mice; touchpads turn a fraction of one at a time.
        let line = ui.ctx().options(|options| options.input_options.line_scroll_speed);
        self.lines += ui.ctx().input(|i| {
            let lines = |event: &egui::Event| match event {
                egui::Event::MouseWheel { unit: MouseWheelUnit::Point, delta, .. } => delta.y / line,
                egui::Event::MouseWheel { delta, .. } => delta.y,
                _ => 0.0,
            };
            i.events.iter().map(lines).sum::<f32>()
        });
        let steps = self.lines.trunc();
        if steps == 0.0 {
            return;
        }
        self.lines -= steps;
        let before = settings.size;
        // Up makes it bigger.
        settings.step_size(steps as i32);
        if settings.size != before {
            // It all scales from the top-left corner, so the corner moves by what the point under
            // the pointer would have.
            self.shift = Some((pointer - full.min) * (1.0 - settings.size / before));
            ui.ctx().request_repaint();
        }
    }
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
    clawd: Clawd,
    status: Status,
    cues: Cues,
    fonts: Fonts,
    scripts: Scripts,
    icon: Arc<IconData>,
    keep_bottom: Arc<AtomicBool>,
    /// The native window, once it has been set up and handed to the Desktop-mode keeper.
    window: Option<isize>,
    /// The Show as choice the window was last set up for, and when.
    applied: Option<(BTreeSet<Mode>, Instant)>,
    size: Vec2,
    /// The window's actual size last frame, to notice when something else resizes it.
    window_size: Option<Vec2>,
    wheel: Wheel,
    flyout: Flyout,
    /// The Always on top or Desktop panel was hidden with a click on a tray icon.
    hidden: bool,
    /// The frame in which the flyout or a hidden panel was last shown.
    shown_frame: u64,
    settings_open: bool,
    /// Settings asked for from the tray. eframe can only create the settings window once the
    /// panel is on screen, so this waits for the panel's first frame.
    settings_requested: bool,
    /// Whether the settings window is on screen yet. A new one stays hidden until it has drawn
    /// itself once, or Windows shows it blank white for a moment.
    settings_shown: bool,
    /// The size the settings window opens at, fitted to the panel's screen.
    settings_size: Vec2,
    settings_page: Page,
    /// Which of the settings' expanders are open, by page and title.
    open_expanders: HashMap<(Page, &'static str), bool>,
    /// Whether Windows starts Cuota at sign-in, as last read from the registry.
    starts_with_windows: bool,
    /// Whether the Claude app is installed, as last looked up.
    claude_app_installed: bool,
    /// The Open dialog picking what a Custom double-click opens, while it's up.
    browsing: Option<Receiver<Option<String>>>,
    /// The panel shows usage details under its meters.
    expanded: bool,
    /// How far the expanded panel moved up and left to stay on screen, to move back when it
    /// shrinks.
    lift: Vec2,
    /// How tall the details were when last drawn.
    details_height: f32,
    transcripts: Transcripts,
    /// When the panel last asked for Claude Code's stats.
    stats_asked: Option<Instant>,
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
        let transcripts = if demo {
            let since = state.lock().unwrap().week_start();
            spawn_demo_story(state.clone(), on_update);
            Transcripts::demo(since)
        } else {
            claude::spawn_poller(state.clone(), wake, interval, on_update);
            let ctx = ctx.clone();
            Transcripts::spawn(move || ctx.request_repaint())
        };

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
            clawd: Clawd::new(ctx),
            status: Status::new(),
            cues: Cues::new(),
            fonts,
            scripts: Scripts::default(),
            icon,
            keep_bottom: Arc::new(AtomicBool::new(false)),
            window: None,
            applied: None,
            size: Vec2::ZERO,
            window_size: None,
            wheel: Wheel::default(),
            flyout: Flyout::Closed { at: None },
            hidden: false,
            shown_frame: 0,
            settings_open: false,
            settings_requested: false,
            settings_shown: false,
            settings_size: SETTINGS_SIZE,
            settings_page: Page::General,
            open_expanders: HashMap::new(),
            starts_with_windows: false,
            claude_app_installed: false,
            browsing: None,
            expanded: false,
            lift: Vec2::ZERO,
            details_height: 0.0,
            transcripts,
            stats_asked: None,
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

    /// A tray click when Always on top or Desktop keeps the panel on screen: hides it, or shows it again.
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
            Flyout::Opening(..) | Flyout::Open(..) => self.close_flyout(ctx),
        }
    }

    /// Like Windows' own flyouts, it opens fresh each time, without details.
    fn close_flyout(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.flyout = Flyout::Closed { at: Some(Instant::now()) };
        self.set_expanded(false);
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
            Flyout::Open(..) if !focused && !self.settings_open => self.close_flyout(ctx),
            _ => {}
        }
    }

    fn set_expanded(&mut self, expanded: bool) {
        if expanded && !self.expanded {
            // Laid out at full height first, so the window grows once rather than twice.
            self.details_height = DETAILS_HEIGHT;
            self.stats_asked = None;
        }
        self.expanded = expanded;
    }

    /// The part of the panel's monitor that the taskbar leaves free, in points.
    fn work_area(&self, ctx: &egui::Context) -> Option<Rect> {
        let ppp = ctx.pixels_per_point();
        let center = ctx.input(|i| i.viewport().outer_rect)?.center();
        let work = win::work_area((center.x * ppp) as i32, (center.y * ppp) as i32)?;
        let work = Rect::from_min_max(pos2(work.left as f32, work.top as f32), pos2(work.right as f32, work.bottom as f32));
        Some(work / ppp)
    }

    /// Moves the Always on top or Desktop panel up or left as little as it takes for the expanded panel
    /// to stay on screen, and back again when it shrinks. A resize by mouse wheel moves it by
    /// `wheel_shift` first, and keeps it on screen too.
    fn fit_on_screen(&mut self, ctx: &egui::Context, size: Vec2, wheel_shift: Option<Vec2>) {
        let Some(rect) = ctx.input(|i| i.viewport().outer_rect) else { return };
        // Where the panel sits when it isn't expanded.
        let mut at = rect.min + self.lift + wheel_shift.unwrap_or_default();
        self.lift = Vec2::ZERO;
        if (self.expanded || wheel_shift.is_some())
            && let Some(work) = self.work_area(ctx)
        {
            let gap = Vec2::splat(SCREEN_GAP);
            let fitted = at.min(work.max - size - gap).max(work.min + gap);
            // Only the details move it back when they close; a resize stays where it landed.
            if self.expanded {
                self.lift = at - fitted;
            }
            at = fitted;
        }
        if at != rect.min {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at.round()));
        }
    }

    /// Asks for Claude Code's latest stats every `STATS_REFRESH` while the panel is expanded,
    /// shows a detail from them, or settings show examples of those. Returns how long until the
    /// next ask.
    fn refresh_stats(&mut self, state: &State) -> Duration {
        let wait = self.stats_asked.map_or(Duration::ZERO, |at| STATS_REFRESH.saturating_sub(at.elapsed()));
        if !wait.is_zero() {
            return wait;
        }
        self.transcripts.refresh(state.week_start());
        self.stats_asked = Some(Instant::now());
        STATS_REFRESH
    }

    /// Clawd beside the meters, and usage details under them when expanded, with the window
    /// sized to fit. Returns when Clawd's next frame is due, if he animates.
    fn panel(&mut self, ui: &mut egui::Ui, state: &State) -> Option<Duration> {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let docked = self.settings.pinned().is_none();

        // Drag to move (the flyout stays put); right-click for settings; double-click as set.
        let background = ui.interact(full, ui.id().with("background"), Sense::click_and_drag());
        if background.secondary_clicked() {
            if self.settings_open {
                self.settings_open = false;
            } else {
                self.open_settings(&ctx);
            }
        }
        if background.double_clicked() {
            match self.settings.double_click {
                DoubleClick::Details => self.set_expanded(!self.expanded),
                DoubleClick::Website => win::open_claude(false),
                DoubleClick::App => win::open_claude(true),
                // Nothing picked yet: the settings are where to pick it.
                DoubleClick::Custom if self.settings.custom_target.is_empty() => self.open_settings(&ctx),
                DoubleClick::Custom => win::open_target(&self.settings.custom_target),
            }
        }
        if !docked && background.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let painter = ui.painter().clone();
        // Everything but the details is drawn at the chosen size.
        let scale = self.settings.size;
        let margin = MARGIN * scale;
        let alpha = (self.settings.opacity * 255.0) as u8;
        painter.rect_filled(full, CORNER * scale, Color32::from_rgba_unmultiplied(BG.r(), BG.g(), BG.b(), alpha));
        painter.rect_stroke(full, CORNER * scale, Stroke::new(1.0, BORDER), StrokeKind::Inside);

        // Lay everything out first so the window can be sized to fit.
        let view = &self.settings.panel;
        let now = Utc::now();
        let limits: Vec<&Limit> = state.limits.iter().filter(|l| view.shows(&l.label)).collect();
        let builder = view::Builder { painter: &painter, strong: &self.fonts.strong, view, now, scale };
        let meters = builder.build(&limits);
        let width = meters.size.x.max(NOTE_WIDTH * scale);
        let note = note(state).map(|(text, color)| painter.layout(text, FontId::proportional(12.0 * scale), color, width));
        // Whatever more was picked to show under them.
        let stats = self.transcripts.latest();
        let source = glance::Source { state, stats: stats.as_deref(), limits: &limits, now };
        let glance = glance::Builder { painter: &painter, source, scale, width }.build(view.details.shown());
        // Clawd's status, when it shows, heads the meters.
        let animate = self.settings.animate;
        let status = view.clawd && view.status;
        let clawd = if view.clawd {
            self.clawd.update(state, animate);
            animations::SIZE * scale
        } else {
            Vec2::ZERO
        };
        if status {
            self.status.set(self.clawd.status().0, animate);
        }
        let status_size = if status { self.status.size(&painter, scale) } else { Vec2::ZERO };

        let note_size = note.as_ref().map_or(Vec2::ZERO, |n| n.size());
        let note_gap = if note.is_some() && meters.size.y > 0.0 { 4.0 * scale } else { 0.0 };
        let status_gap = if status && meters.size.y + note_size.y > 0.0 { STATUS_GAP * scale } else { 0.0 };
        // The meters with the status over them and the note under them, then any details.
        let head = status_size.y + status_gap + meters.size.y + note_gap + note_size.y;
        let glance_gap = if glance.size.y > 0.0 && head > 0.0 { GLANCE_GAP * scale } else { 0.0 };
        let body = vec2(
            meters.size.x.max(note_size.x).max(status_size.x).max(glance.size.x),
            head + glance_gap + glance.size.y,
        );
        let gap = if clawd.x > 0.0 && body.x > 0.0 { GAP * scale } else { 0.0 };
        let row = vec2(clawd.x + gap + body.x, clawd.y.max(body.y));
        // Details scroll once they'd run past DETAILS_HEIGHT or off the screen.
        let details = self.expanded.then(|| {
            let room = self
                .work_area(&ctx)
                .map_or(DETAILS_HEIGHT, |work| work.height() - row.y - 2.0 * (margin + SCREEN_GAP));
            vec2(DETAILS_WIDTH, self.details_height.min(DETAILS_HEIGHT).min(room).max(0.0))
        });
        let inner = details.map_or(row, |details| vec2(row.x.max(details.x), row.y + details.y));
        let size = (inner + Vec2::splat(2.0 * margin)).round();
        let wheel_shift = self.wheel.shift.take();
        // Scripts and window managers can resize it too (one put it back to its first size,
        // cropping the panel), so a size changed to anything else is asked for again. Only
        // when it changes, so a size Windows won't allow isn't asked for every frame.
        let window_size = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size().round());
        let resized = window_size != self.window_size && window_size.is_some_and(|actual| actual != size);
        self.window_size = window_size;
        if size != self.size || resized {
            self.size = size;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            self.place_flyout(&ctx);
            if !docked {
                self.fit_on_screen(&ctx, size, wheel_shift);
            }
        }

        let origin = full.min + Vec2::splat(margin);
        let details = details.map(|details| Rect::from_min_size(origin + vec2(0.0, row.y), vec2(inner.x, details.y)));
        let mut next_frame = None;
        if clawd.x > 0.0 {
            // Halfway down what's beside him, so a long list of details doesn't leave him
            // perched at the top with a gap under him.
            let slot = Rect::from_min_size(origin + vec2(0.0, (row.y - clawd.y) / 2.0), clawd);
            next_frame = self.clawd.paint(&painter, slot, animate, scale);
        }
        let body_at = origin + vec2(clawd.x + gap, (row.y - body.y) / 2.0);
        if status {
            let working = self.clawd.status().1;
            let next = self.status.paint(&painter, body_at, working, animate, scale);
            next_frame = next_frame.into_iter().chain(next).min();
        }
        let meters_at = body_at + vec2(0.0, status_size.y + status_gap);
        let mut below = meters_at.y + meters.size.y + note_gap;
        let mut hints = meters.paint(&painter, meters_at);
        if let Some(note) = note {
            painter.galley(pos2(body_at.x, below), note, TEXT);
            below += note_size.y;
        }
        hints.extend(glance.paint(&painter, pos2(body_at.x, below + glance_gap)));
        for (i, (rect, hint)) in hints.into_iter().enumerate() {
            ui.interact(rect, ui.id().with(("hint", i)), Sense::hover()).on_hover_text(hint);
        }

        if let Some(area) = details {
            painter.hline(full.left() + margin..=full.right() - margin, area.top() + 6.0, Stroke::new(1.0, BORDER));
            let mut slot = ui.new_child(UiBuilder::new().max_rect(area));
            // Dragging anywhere moves the panel, so the details scroll by wheel and scroll bar.
            let scroll_by = egui::scroll_area::ScrollSource { drag: egui::scroll_area::ScrollSource::NONE.drag, ..Default::default() };
            let scroll = egui::ScrollArea::vertical()
                .id_salt("details")
                .max_height(area.height())
                .auto_shrink([false, true])
                .scroll_source(scroll_by)
                .show(&mut slot, |ui| {
                    let lane = Margin { right: SCROLL_LANE as i8, ..Margin::ZERO };
                    egui::Frame::new().inner_margin(lane).show(ui, |ui| details::ui(ui, &self.fonts, state, stats.as_deref()));
                });
            let height = scroll.content_size.y.ceil();
            if (height - self.details_height).abs() >= 1.0 {
                self.details_height = height;
                ctx.request_repaint();
            }
        }
        if self.settings.wheel_resize {
            self.wheel.resize(ui, details, &mut self.settings);
        }
        next_frame
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
            self.settings_page = Page::General;
            // Its usual size, or as much of it as fits on the panel's screen.
            self.settings_size = self.work_area(ctx).map_or(SETTINGS_SIZE, |screen| {
                let room = screen.size() - TITLE_BAR - Vec2::splat(2.0 * SCREEN_GAP);
                SETTINGS_SIZE.min(room).max(SETTINGS_MIN)
            });
        }
        self.settings_open = true;
        self.starts_with_windows = win::starts_with_windows();
        self.claude_app_installed = win::claude_app_installed();
        // Already open, maybe behind other windows.
        ctx.send_viewport_cmd_to(settings_viewport(), ViewportCommand::Focus);
    }

    fn settings_window(&mut self, ctx: &egui::Context, state: &State) {
        // The panel's screen, in pixels: where a new window opens.
        let panel_screen =
            if self.settings_shown { None } else { self.work_area(ctx).map(|work| work * ctx.pixels_per_point()) };
        let builder = ViewportBuilder::default()
            .with_title("Cuota settings")
            .with_inner_size(self.settings_size)
            .with_min_inner_size(SETTINGS_MIN)
            .with_always_on_top()
            .with_visible(false)
            .with_icon(self.icon.clone());
        ctx.show_viewport_immediate(settings_viewport(), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested()) {
                self.settings_open = false;
            }
            ui.painter().rect_filled(ui.max_rect(), 0, widgets::WINDOW);
            self.settings_ui(ui, state);
            if !self.settings_shown {
                self.settings_shown = true;
                // Centred on the panel's screen. Windows' own spot for a new window can leave the
                // bottom under the taskbar.
                if let Some(screen) = panel_screen.map(|work| work / ctx.pixels_per_point()) {
                    let title_bar = ctx
                        .input(|i| Some(i.viewport().outer_rect?.size() - i.viewport().inner_rect?.size()))
                        .unwrap_or(TITLE_BAR);
                    let outer = self.settings_size + title_bar;
                    let at = (screen.center() - outer / 2.0).max(screen.min + Vec2::splat(SCREEN_GAP));
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at.round()));
                }
                // These run once this frame is on the window, so it appears already drawn.
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
        });
    }

    /// A sidebar with the account, the pages, GitHub and Quit, beside the page picked in it, as in
    /// Windows' Settings: the page's title stays put while its settings scroll under it. Returns
    /// how tall the page is, whether or not the window has room for all of it.
    fn settings_ui(&mut self, ui: &mut egui::Ui, state: &State) -> f32 {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        let full = ui.max_rect();
        let (sidebar, page) = full.split_left_right_at_x(full.left() + SIDEBAR_WIDTH);
        let mut slot = ui.new_child(UiBuilder::new().max_rect(sidebar.shrink2(vec2(12.0, 16.0))));
        self.settings_sidebar(&mut slot, state);

        let title = pos2(page.left() + PAGE_GUTTER, page.top() + 14.0);
        let font = FontId::new(28.0, self.fonts.strong.clone());
        ui.painter().text(title, Align2::LEFT_TOP, self.settings_page.name(), font, widgets::PRIMARY);
        let body = Rect::from_min_max(pos2(page.left(), page.top() + PAGE_TITLE), page.max);
        let mut slot = ui.new_child(UiBuilder::new().max_rect(body));
        // Scrolls once the window is too small for the page. Each page keeps its own place.
        let scroll = egui::ScrollArea::vertical().id_salt(("settings", self.settings_page)).auto_shrink(false);
        let scroll = scroll.show(&mut slot, |ui| {
            // The right margin leaves a lane for the scroll bar.
            let margin = Margin { left: PAGE_GUTTER as i8, right: PAGE_GUTTER as i8 + 8, top: 0, bottom: 24 };
            egui::Frame::new().inner_margin(margin).show(ui, |ui| {
                // As wide as the window allows, up to an easy reading width.
                ui.set_max_width(ui.available_width().min(PAGE_WIDTH));
                ui.spacing_mut().item_spacing.y = CARD_GAP;
                self.settings_page(ui, state);
            });
        });
        PAGE_TITLE + scroll.content_size.y
    }

    /// The account on top, then the pages, and a link to Cuota's GitHub over Quit at the bottom.
    fn settings_sidebar(&mut self, ui: &mut egui::Ui, state: &State) {
        self.settings_account(ui, state);
        ui.add_space(20.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        for page in Page::ALL {
            if widgets::nav_item(ui, &self.fonts, page.glyph(), page.name(), self.settings_page == page).clicked() {
                self.settings_page = page;
            }
        }

        let bottom = ui.max_rect();
        let footer = Rect::from_min_max(pos2(bottom.left(), bottom.bottom() - 32.0), bottom.max);
        let link = Rect::from_min_size(pos2(bottom.left(), footer.top() - 8.0 - 36.0), vec2(bottom.width(), 36.0));
        let mut slot = ui.new_child(UiBuilder::new().max_rect(link));
        let github = widgets::nav_link(&mut slot, &self.fonts, widgets::github_mark(ui.ctx()), "View on GitHub");
        if github.on_hover_text(REPOSITORY).clicked() {
            win::open_page(REPOSITORY);
        }
        let mut slot = ui.new_child(UiBuilder::new().max_rect(footer).layout(Layout::left_to_right(Align::Center)));
        if widgets::button(&mut slot, &self.fonts, icon::QUIT, "Quit Cuota").clicked() {
            ui.ctx().send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        }
        let version = concat!("Cuota v", env!("CARGO_PKG_VERSION"));
        let font = FontId::proportional(12.0);
        ui.painter().text(footer.right_center(), Align2::RIGHT_CENTER, version, font, widgets::SECONDARY);
    }

    /// The Claude spark and the account, with when usage was last fetched and a button to
    /// fetch it now.
    fn settings_account(&self, ui: &mut egui::Ui, state: &State) {
        const LOGO: f32 = 40.0;
        let account = &state.account;
        let painter = ui.painter().clone();
        let width = ui.available_width();

        let text_left = LOGO + 12.0;
        let text_width = width - text_left;
        let plan = account.plan.as_deref().unwrap_or("Claude");
        let strong = FontId::new(16.0, self.fonts.strong.clone());
        let mut lines = vec![single_line(&painter, plan, strong, widgets::PRIMARY, text_width)];
        for detail in [&account.email, &account.organization].into_iter().flatten() {
            lines.push(single_line(&painter, detail, FontId::proportional(12.0), widgets::SECONDARY, text_width));
        }
        let text_height: f32 = lines.iter().map(|line| line.size().y).sum();
        let (rect, _) = ui.allocate_exact_size(vec2(width, text_height.max(LOGO)), Sense::hover());

        self.mascot.paint_logo(&painter, pos2(rect.left() + LOGO / 2.0, rect.center().y), LOGO / 2.0 - 3.0);
        let mut y = rect.center().y - text_height / 2.0;
        for line in lines {
            let height = line.size().y;
            painter.galley(pos2(rect.left() + text_left, y), line, widgets::PRIMARY);
            y += height;
        }

        let updated = state.updated.map_or_else(|| "Not updated yet".into(), |t| t.format("Updated %H:%M").to_string());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(text_left);
            ui.label(RichText::new(updated).size(12.0).color(widgets::SECONDARY));
            if widgets::icon_button(ui, &self.fonts, icon::REFRESH, "Refresh").on_hover_text("Fetch usage now").clicked() {
                let _ = self.refresh.send(());
            }
        });
    }

    /// The page picked in the sidebar: a card for each setting, as in Windows' Settings.
    fn settings_page(&mut self, ui: &mut egui::Ui, state: &State) {
        let Self { settings, settings_page: page, open_expanders, fonts, starts_with_windows, claude_app_installed, browsing, .. } =
            self;
        let page = *page;

        let note = match page {
            Page::Panel | Page::Details if settings.pinned().is_none() => {
                Some("The panel opens when you click a taskbar icon.")
            }
            Page::Taskbar if !settings.shows_as(Mode::Taskbar) => {
                Some("These apply once Show as, under General, includes Taskbar.")
            }
            _ => None,
        };
        if let Some(note) = note {
            widgets::info_bar(ui, fonts, note);
            ui.add_space(8.0);
        }

        match page {
            Page::General => {
                let options = show_as_options(settings);
                let shown: Vec<&str> = options.iter().filter(|o| settings.shows_as(o.0)).map(|o| o.1).collect();
                let about = Some("Where Cuota appears on your screen");
                let open = open_expanders.entry((page, "Show as")).or_default();
                widgets::expander(ui, fonts, open, icon::SHOW_AS, "Show as", about, &shown.join(", "), |ui| {
                    for (mode, name, about) in options {
                        let mut shown = settings.shows_as(mode);
                        // The last one stays on, so there's always something to right-click for settings.
                        let last = shown && settings.show.len() == 1;
                        let item = ui.add_enabled(!last, |ui: &mut egui::Ui| {
                            widgets::check_item(ui, fonts, &mut shown, name, Some(&about))
                        });
                        if item.on_disabled_hover_text("At least one stays on").changed() {
                            settings.set_shown_as(mode, shown);
                        }
                    }
                });

                let about = match settings.double_click {
                    DoubleClick::Details => "Expands the panel with usage details. Double-click again to shrink it.",
                    DoubleClick::Website => "Opens your usage page on claude.ai",
                    DoubleClick::App if *claude_app_installed => "Opens the Claude app",
                    DoubleClick::App => "The Claude app isn't installed, so this opens claude.ai",
                    DoubleClick::Custom => "Opens the file, program or script below",
                };
                let options = [
                    (DoubleClick::Details, "Show details"),
                    (DoubleClick::Website, "Open usage page"),
                    (DoubleClick::App, "Open Claude app"),
                    (DoubleClick::Custom, "Open a file or program"),
                ];
                let width = widgets::dropdown_width(ui, &options.map(|(_, name)| name));
                widgets::setting(ui, fonts, icon::DOUBLE_CLICK, "Double-click", Some(about), width, |ui| {
                    widgets::dropdown(ui, fonts, &mut settings.double_click, &options, width);
                });
                if settings.double_click == DoubleClick::Custom {
                    custom_target(ui, fonts, settings, browsing);
                }

                let about = Some("Opens Cuota when you sign in");
                widgets::setting(ui, fonts, icon::STARTUP, "Start with Windows", about, widgets::TOGGLE_WIDTH, |ui| {
                    let mut on = *starts_with_windows;
                    if widgets::toggle(ui, &mut on).changed() {
                        win::set_start_with_windows(on);
                        *starts_with_windows = win::starts_with_windows();
                    }
                });

                if let Some(log) = log::path() {
                    let about = Some("Rate limits, errors and other things Cuota ran into");
                    let width = widgets::button_width(ui, fonts, "Open folder");
                    widgets::setting(ui, fonts, icon::LOG, "Log file", about, width, |ui| {
                        let button = widgets::button(ui, fonts, icon::FOLDER, "Open folder");
                        if button.on_hover_text(log.display().to_string()).clicked() {
                            win::show_in_folder(&log);
                        }
                    });
                }
            }
            Page::Appearance => {
                let size = format!("{:.0}%", settings.size * 100.0);
                widgets::setting(ui, fonts, icon::SIZE, "Size", Some("Small to large"), SLIDER_ROW, |ui| {
                    value_label(ui, size);
                    let mut step = settings.size_step();
                    widgets::stepped_slider(ui, &mut step, SIZES.len(), SLIDER_WIDTH);
                    settings.size = SIZES[step];
                });
                let about = Some("Scroll up or down over the panel");
                widgets::setting(ui, fonts, icon::WHEEL, "Resize with mouse wheel", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut settings.wheel_resize);
                });
                let opacity = format!("{:.0}%", settings.opacity * 100.0);
                widgets::setting(ui, fonts, icon::OPACITY, "Opacity", Some("Panel background"), SLIDER_ROW, |ui| {
                    value_label(ui, opacity);
                    widgets::slider(ui, &mut settings.opacity, 0.3..=1.0, SLIDER_WIDTH);
                });
                let about = Some("Turn off to use almost no CPU");
                widgets::setting(ui, fonts, icon::ANIMATE, "Animate Clawd", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut settings.animate);
                });
                let about = Some("Pixel blips while Claude works");
                widgets::setting(ui, fonts, icon::SOUND, "Sound effects", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut settings.sounds);
                });
                let modes = [(SoundMode::Normal, "Normal"), (SoundMode::Lively, "Lively")];
                let width = widgets::dropdown_width(ui, &modes.map(|(_, name)| name));
                let about = match settings.sound_mode {
                    SoundMode::Normal => "Only what matters: work starts, Claude needs you, work's done",
                    SoundMode::Lively => "A sound for each kind of step too, to hear what Claude's up to",
                };
                ui.add_enabled_ui(settings.sounds, |ui| {
                    widgets::setting(ui, fonts, icon::SOUND_MODE, "Sound mode", Some(about), width, |ui| {
                        widgets::dropdown(ui, fonts, &mut settings.sound_mode, &modes, width);
                    });
                });
                // Even with sounds off, to hear them before turning them on.
                let open = open_expanders.entry((page, "Hear the sounds")).or_default();
                let about = Some("Learn what each one means");
                widgets::expander(ui, fonts, open, icon::HEAR, "Hear the sounds", about, "", |ui| {
                    for guide in &sound::GUIDE {
                        let about =
                            if guide.normal { guide.about.to_owned() } else { format!("{} \u{B7} Lively only", guide.about) };
                        if widgets::action_item(ui, fonts, icon::PLAY, guide.title, Some(&about)).clicked() {
                            sound::play(guide.sound);
                        }
                    }
                });
            }
            Page::Panel | Page::Taskbar => {
                let taskbar = page == Page::Taskbar;
                let view = if taskbar { &mut settings.taskbar } else { &mut settings.panel };
                let styles = [(Style::Bars, "Bars"), (Style::Pie, "Pie")];
                let width = widgets::dropdown_width(ui, &styles.map(|(_, name)| name));
                widgets::setting(ui, fonts, icon::STYLE, "Style", Some("How each limit is drawn"), width, |ui| {
                    widgets::dropdown(ui, fonts, &mut view.style, &styles, width);
                });
                if state.limits.is_empty() {
                    let about = Some("They appear here once usage has loaded");
                    widgets::setting(ui, fonts, icon::LIMITS, "Limits", about, 0.0, |_| {});
                } else {
                    let shown: Vec<&str> =
                        state.limits.iter().filter(|limit| view.shows(&limit.label)).map(|limit| limit.label.as_str()).collect();
                    let summary = if shown.is_empty() { "None".to_owned() } else { shown.join(", ") };
                    let open = open_expanders.entry((page, "Limits")).or_default();
                    let about = Some("Pick the ones to show");
                    widgets::expander(ui, fonts, open, icon::LIMITS, "Limits", about, &summary, |ui| {
                        for limit in &state.limits {
                            let mut shown = view.shows(&limit.label);
                            if widgets::check_item(ui, fonts, &mut shown, &limit.label, None).changed() {
                                view.set_shown(&limit.label, shown);
                            }
                        }
                    });
                }
                let (time_left, clawd) = if taskbar {
                    ("Shows when you hover an icon", "Shows if Claude is working")
                } else {
                    ("Until each limit resets", "A random animation while Claude works")
                };
                for (glyph, title, about, on) in [
                    (icon::PERCENT, "Percentage", None, &mut view.percent),
                    (icon::TIME_LEFT, "Time left", Some(time_left), &mut view.time_left),
                    (icon::CLAWD, "Clawd", Some(clawd), &mut view.clawd),
                ] {
                    widgets::setting(ui, fonts, glyph, title, about, widgets::TOGGLE_WIDTH, |ui| {
                        widgets::toggle(ui, on);
                    });
                }
                // Clawd's status heads the panel's meters, so the panel only, and only with him.
                if !taskbar {
                    ui.add_enabled_ui(view.clawd, |ui| {
                        let about = Some("What Clawd's up to, like Percolating…");
                        widgets::setting(ui, fonts, icon::STATUS, "Status", about, widgets::TOGGLE_WIDTH, |ui| {
                            widgets::toggle(ui, &mut view.status);
                        });
                    });
                }
            }
            Page::Details => {
                let view = &mut settings.panel;
                // Each row's example is what the panel would show now, where it can.
                let stats = self.transcripts.latest();
                let limits: Vec<&Limit> = state.limits.iter().filter(|limit| view.shows(&limit.label)).collect();
                let source = glance::Source { state, stats: stats.as_deref(), limits: &limits, now: Utc::now() };
                let items = view.details.items();
                let examples: Vec<String> = items.iter().map(|&(detail, _)| glance::example(detail, &source)).collect();
                let rows: Vec<SortRow> = items
                    .iter()
                    .zip(&examples)
                    .map(|(&(detail, on), example)| SortRow {
                        id: Id::new(("detail", detail)),
                        on,
                        title: glance::title(detail),
                        description: example,
                    })
                    .collect();
                let summary = match view.details.shown().count() {
                    0 => "None".to_owned(),
                    shown => format!("{shown} shown"),
                };
                let about = Some("Pick what the panel shows there, and drag them into order");
                let sorted = widgets::group(ui, fonts, icon::CHECKLIST, "Under the meters", about, &summary, |ui| {
                    widgets::sortable(ui, fonts, &rows)
                });
                match sorted {
                    Some(Sorted::Toggled(index)) => view.details.toggle(index),
                    Some(Sorted::Moved { from, to }) => view.details.move_item(from, to),
                    None => {}
                }
            }
        }
    }
}

impl eframe::App for Overlay {
    /// Runs even while the window is hidden, so tray clicks get through.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.window.is_none()
            && let Some(window) = native_window(frame)
        {
            win::set_up_panel(window);
            win::keep_at_bottom(window, self.keep_bottom.clone());
            self.window = Some(window);
        }
        if let Some(window) = self.window {
            win::round_corners(window, CORNER * self.settings.size * ctx.pixels_per_point());
        }
        self.apply_modes(ctx);
        self.sync_tray();
        while let Ok(event) = self.tray_events.try_recv() {
            let (settings, details) = match event {
                TrayEvent::Click => (false, false),
                TrayEvent::Settings => (true, false),
                TrayEvent::Details => (false, true),
                TrayEvent::Quit => {
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                    continue;
                }
            };
            if details {
                self.set_expanded(true);
            }
            if self.settings.pinned().is_some() {
                // A click hides or shows the panel; settings and details bring it back if it was hidden.
                self.set_hidden(ctx, !settings && !details && !self.hidden);
                self.settings_requested |= settings;
                ctx.request_repaint();
            } else if settings || details {
                // Settings with the panel open beside them, or the panel with its details.
                self.settings_requested |= settings;
                if matches!(self.flyout, Flyout::Closed { .. }) {
                    self.open_flyout(ctx);
                }
                ctx.request_repaint();
            } else {
                self.toggle_flyout(ctx);
            }
        }

        // Sounds follow Claude's sessions, whether or not the panel shows. They're made in the
        // background as Cuota starts, or once they're turned on.
        if self.settings.sounds {
            sound::prepare();
        }
        let now = Instant::now();
        let state = self.state.lock().unwrap().clone();
        if let Some(sound) = self.cues.update(&state, self.settings.sounds, self.settings.sound_mode, now) {
            sound::play(sound);
        }
        if let Some(due) = self.cues.due() {
            ctx.request_repaint_after(due.saturating_duration_since(now));
        }

        // Titles, folders and the account come in whatever script they were written in.
        let stats = self.transcripts.latest();
        let sessions = state.sessions.iter().flat_map(|s| [s.title.as_deref(), Some(&*s.name), s.project.as_deref()]);
        let account = [state.account.email.as_deref(), state.account.organization.as_deref(), state.error.as_deref()];
        let projects = stats.iter().flat_map(|stats| stats.projects.iter().map(|(name, _)| Some(name.as_str())));
        let custom = Some(self.settings.custom_target.as_str());
        let texts = sessions.chain(account).chain(projects).chain([custom]).flatten();
        if self.scripts.cover(ctx, &self.fonts, texts) {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let state = self.state.lock().unwrap().clone();
        let docked = self.settings.pinned().is_none();
        let visible = if docked { !matches!(self.flyout, Flyout::Closed { .. }) } else { !self.hidden };

        let mut next_frame = None;
        if visible {
            // Clawd's next frame, or the next minute for the countdowns.
            next_frame = Some(self.panel(ui, &state).unwrap_or_else(|| {
                Duration::from_millis(60_000 - Utc::now().timestamp_millis().rem_euclid(60_000) as u64)
            }));
            // Claude Code's stats, while something shows them.
            let details = &self.settings.panel.details;
            if self.expanded || details.use_stats() || self.settings_open && self.settings_page == Page::Details {
                let wait = self.refresh_stats(&state);
                next_frame = next_frame.map(|next| next.min(wait));
            }
            if self.settings_requested && ctx.cumulative_frame_nr() > self.shown_frame {
                self.settings_requested = false;
                self.open_settings(&ctx);
            }
            if docked {
                self.track_flyout_focus(&ctx);
            } else if self.applied.as_ref().is_some_and(|(_, at)| at.elapsed() > SETTLE)
                && let Some(rect) = ctx.input(|i| i.viewport().outer_rect)
            {
                // Where it sits when not expanded, so it starts there next time.
                let at = rect.min + self.lift;
                self.settings.panel_pos = Some([at.x, at.y]);
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

/// The Show as options: each one's mode, name and what it does.
fn show_as_options(settings: &Settings) -> [(Mode, &'static str, String); 3] {
    let click = if settings.pinned().is_some() { "to hide or show the panel" } else { "to open the panel" };
    [
        (Mode::AlwaysOnTop, "Always on top", "Floats above all windows. Drag it to move it.".to_owned()),
        (Mode::Desktop, "Desktop", "Sits behind your windows, like a desktop widget. Drag it to move it.".to_owned()),
        (
            Mode::Taskbar,
            "Taskbar",
            format!(
                "Icons beside the clock; click one {click}. New icons start in the ^ overflow, so drag them \
                 onto the taskbar once."
            ),
        ),
    ]
}

/// A slider's value, beside it.
fn value_label(ui: &mut egui::Ui, value: String) {
    ui.add_sized([VALUE_WIDTH, 20.0], Label::new(RichText::new(value).size(14.0).color(widgets::SECONDARY)));
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

/// Plays a session out over and over in the demo, a scene every 3 seconds: work starting, a
/// step of each kind, a question for you, and a rest, so Clawd and the sounds have something
/// to follow.
fn spawn_demo_story(state: Shared, on_update: impl Fn() + Send + 'static) {
    const WORK: (bool, bool) = (true, false);
    const ASK: (bool, bool) = (false, true);
    const REST: (bool, bool) = (false, false);
    const STORY: [((bool, bool), Option<Step>); 16] = [
        (WORK, Some(Step::Think)),
        (WORK, Some(Step::Look)),
        (WORK, Some(Step::Look)),
        (WORK, Some(Step::Edit)),
        (WORK, Some(Step::Run)),
        (ASK, None),
        (ASK, None),
        (WORK, Some(Step::Web)),
        (WORK, Some(Step::Delegate)),
        (WORK, Some(Step::Plan)),
        (WORK, Some(Step::Fail)),
        (WORK, Some(Step::Edit)),
        (WORK, Some(Step::Compact)),
        (REST, None),
        (REST, None),
        (REST, None),
    ];
    std::thread::spawn(move || {
        for ((busy, waiting), step) in STORY.into_iter().cycle() {
            std::thread::sleep(Duration::from_secs(3));
            {
                let mut shared = state.lock().unwrap();
                let s = Arc::make_mut(&mut shared);
                if let Some(session) = s.sessions.first_mut() {
                    session.busy = busy;
                    session.waiting = waiting;
                    if step.is_some() {
                        session.steps += 1;
                        session.step = step;
                    }
                }
                s.working = s.sessions.iter().any(|session| session.busy);
            }
            on_update();
        }
    });
}

/// Windows' own UI and icon fonts (egui's bundled fonts are left out of the build).
pub(crate) fn setup_fonts(ctx: &egui::Context) -> Fonts {
    let dir = scripts::folder();
    let load = |names: &[&str]| names.iter().find_map(|name| std::fs::read(dir.join(name)).ok());
    let mut fonts = egui::FontDefinitions::empty();
    let regular = load(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]).expect("Windows has a UI font");
    fonts.font_data.insert("regular".into(), Arc::new(FontData::from_owned(regular)));
    let mut strong_fonts = vec!["regular".to_owned()];
    if let Some(bytes) = load(&["seguisb.ttf", "arialbd.ttf"]) {
        fonts.font_data.insert("semibold".into(), Arc::new(FontData::from_owned(bytes)));
        strong_fonts.insert(0, "semibold".into());
    }
    let mut proportional = vec!["regular".to_owned()];
    // For the stars in Clawd's status, which Segoe UI lacks.
    if let Some(bytes) = load(&["seguisym.ttf"]) {
        fonts.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(bytes)));
        proportional.push("symbols".into());
    }
    let strong = FontFamily::Name("strong".into());
    fonts.families.insert(FontFamily::Proportional, proportional);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Detail, Details};

    #[test]
    fn windows_fonts_have_the_stars_of_clawds_status() {
        let ctx = egui::Context::default();
        setup_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |_| {});
        // egui's has_glyph says no to everything in the font its replacement glyph comes from,
        // Segoe UI Symbol here, so check instead that no star is drawn as that replacement.
        let drawn = |c: char| {
            let font = FontId::proportional(12.5);
            let galley = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(c.to_string(), font, Color32::WHITE));
            galley.rows[0].glyphs[0].uv_rect
        };
        let missing = drawn('\u{378}');
        for star in "·✢*✶✻✽…".chars() {
            assert_ne!(drawn(star), missing, "{star}");
        }
    }

    #[test]
    fn titles_in_any_script_are_drawn() {
        let ctx = egui::Context::default();
        let fonts = setup_fonts(&ctx);
        let mut scripts = Scripts::default();
        let font = FontId::proportional(12.5);
        // Glyphs drawn as the replacement for a missing one. Cluster marks shaped into the glyph
        // before them are drawn as nothing.
        let missing = |text: &str| {
            let _ = ctx.run_ui(Default::default(), |_| {});
            let replacement = ctx.fonts_mut(|f| f.layout_no_wrap("\u{378}".into(), font.clone(), Color32::WHITE));
            let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.into(), font.clone(), Color32::WHITE));
            galley.rows[0].glyphs.iter().filter(|g| g.uv_rect == replacement.rows[0].glyphs[0].uv_rect).count()
        };
        let gujarati = "Gemini બ્રાઉઝર ઇમેજ જનરેશન એક્સટેન્શન";
        assert!(missing(gujarati) > 0, "Segoe UI has no Gujarati");
        assert!(scripts.cover(&ctx, &fonts, [gujarati]));
        assert_eq!(missing(gujarati), 0);
        assert!(!scripts.cover(&ctx, &fonts, [gujarati]), "read once");
        // Shaped, not drawn letter by letter: ક્ષ is one glyph.
        let conjunct = ctx.fonts_mut(|f| f.layout_no_wrap("ક્ષ".into(), font.clone(), Color32::WHITE));
        assert_eq!(conjunct.rows[0].glyphs.iter().filter(|g| !g.uv_rect.is_nothing()).count(), 1);
        for text in ["हिन्दी में", "தமிழ்", "ภาษาไทย", "ພາສາລາວ", "ខ្មែរ", "བོད་ཡིག", "မြန်မာ", "አማርኛ", "ᏣᎳᎩ", "한국어", "中文标题", "日本語のタイトル"] {
            scripts.cover(&ctx, &fonts, [text]);
            assert_eq!(missing(text), 0, "{text}");
        }
    }

    /// A frame of a 300×200 panel, details in its bottom half, with the pointer at `at` and the
    /// wheel turned `lines` (up when positive). Returns how far the panel moves.
    fn turn(ctx: &egui::Context, wheel: &mut Wheel, settings: &mut Settings, at: Pos2, lines: f32) -> Option<Vec2> {
        let details = Rect::from_min_max(pos2(0.0, 100.0), pos2(300.0, 200.0));
        let turned = egui::Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 200.0))),
            events: vec![egui::Event::PointerMoved(at), turned],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| wheel.resize(ui, Some(details), settings));
        wheel.shift.take()
    }

    #[test]
    fn the_wheel_over_the_panel_steps_its_size_around_the_pointer_but_scrolls_the_details() {
        let (ctx, mut wheel, mut settings) = (egui::Context::default(), Wheel::default(), Settings::default());
        let meters = pos2(150.0, 40.0);
        // Up a notch: a step bigger, and the corner moves up and left so the meters stay under the pointer.
        let shift = turn(&ctx, &mut wheel, &mut settings, meters, 1.0).unwrap();
        assert_eq!(settings.size, 1.1);
        assert!((shift - vec2(-15.0, -4.0)).length() < 0.01, "{shift:?}");
        // Over the details, or off the panel, nothing changes.
        assert_eq!(turn(&ctx, &mut wheel, &mut settings, pos2(150.0, 150.0), 1.0), None);
        assert_eq!(turn(&ctx, &mut wheel, &mut settings, pos2(350.0, 40.0), 1.0), None);
        assert_eq!(settings.size, 1.1);
        // Down two notches: two steps smaller, and the corner moves the other way.
        let shift = turn(&ctx, &mut wheel, &mut settings, meters, -2.0).unwrap();
        assert_eq!(settings.size, 0.9);
        assert!(shift.x > 0.0 && shift.y > 0.0, "{shift:?}");
    }

    #[test]
    fn a_touchpad_steps_once_its_turned_a_whole_notch_and_the_size_stops_at_either_end() {
        let (ctx, mut wheel, mut settings) = (egui::Context::default(), Wheel::default(), Settings::default());
        let meters = pos2(150.0, 40.0);
        assert_eq!(turn(&ctx, &mut wheel, &mut settings, meters, 0.4), None);
        assert_eq!(turn(&ctx, &mut wheel, &mut settings, meters, 0.4), None);
        assert!(turn(&ctx, &mut wheel, &mut settings, meters, 0.4).is_some());
        assert_eq!(settings.size, 1.1);
        settings.size = SIZES[9];
        assert_eq!(turn(&ctx, &mut wheel, &mut settings, meters, 1.0), None);
        assert_eq!(settings.size, SIZES[9]);
    }

    /// Lays the panel out in a frame, and returns the size it asks the window to be.
    fn panel_size(ctx: &egui::Context, overlay: &mut Overlay, state: &State) -> Vec2 {
        let _ = ctx.run_ui(window(vec2(1600.0, 1200.0)), |ui| {
            overlay.panel(ui, state);
        });
        overlay.size
    }

    #[test]
    fn the_panel_grows_to_fit_the_details_it_shows_but_no_wider_than_its_meters() {
        let (ctx, mut overlay, state) = settings_overlay();
        let all: Details = Detail::ALL.map(|detail| (detail, true)).into_iter().collect();
        let mut grown = Vec::new();
        for size in [SIZES[0], 1.0, SIZES[9]] {
            overlay.settings.size = size;
            // Without Clawd the panel is as tall as what would be beside him.
            overlay.settings.panel.clawd = false;
            overlay.settings.panel.details = Details::default();
            let plain = panel_size(&ctx, &mut overlay, &state);
            let widest = plain.x.max(((NOTE_WIDTH + 2.0 * MARGIN) * size).round());
            // The demo has something for every detail to show.
            for detail in Detail::ALL {
                overlay.settings.panel.details = [(detail, true)].into_iter().collect();
                let shown = panel_size(&ctx, &mut overlay, &state);
                assert!(shown.y > plain.y, "{detail:?} at {size} leaves the panel at {shown:?}");
                assert!(shown.x <= widest, "{detail:?} at {size} widens the panel to {shown:?}");
            }
            overlay.settings.panel.details = all.clone();
            let everything = panel_size(&ctx, &mut overlay, &state);
            assert!(everything.x <= widest, "at {size} all of them widen the panel to {everything:?}");
            grown.push(everything.y - plain.y);
            // Beside Clawd too, they make room for themselves.
            overlay.settings.panel.clawd = true;
            assert!(panel_size(&ctx, &mut overlay, &state).y > (animations::SIZE.y + 2.0 * MARGIN) * size);
        }
        assert!(grown.windows(2).all(|pair| pair[0] < pair[1]), "they scale with the panel: {grown:?}");
    }

    #[test]
    fn clawd_sits_halfway_down_the_panel_however_many_details_it_lists() {
        let (ctx, mut overlay, state) = settings_overlay();
        let all: Details = Detail::ALL.map(|detail| (detail, true)).into_iter().collect();
        for size in [SIZES[0], 1.0, SIZES[9]] {
            overlay.settings.size = size;
            for details in [Details::default(), all.clone()] {
                overlay.settings.panel.details = details;
                let output = ctx.run_ui(window(vec2(1600.0, 1200.0)), |ui| {
                    overlay.panel(ui, &state);
                });
                // He's the only picture in the panel, clipped to his slot plus SPILL all round.
                let slot = output.shapes.iter().find(|clipped| clipped.shape.texture_id() != egui::TextureId::default());
                let slot = slot.expect("Clawd is drawn").clip_rect;
                let middle = overlay.size.y / 2.0;
                assert!((slot.center().y - middle).abs() <= 1.0, "at {size} he's at {slot:?} in {:?}", overlay.size);
            }
            assert!(overlay.size.y > (animations::SIZE.y + 2.0 * MARGIN) * size, "the details run past him at {size}");
        }
    }

    /// Where the settings drew `text`, the first time.
    fn drawn(output: &egui::FullOutput, text: &str) -> Rect {
        let rect = output.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => Some(shape.visual_bounding_rect()),
            _ => None,
        });
        rect.unwrap_or_else(|| panic!("nothing says {text:?}"))
    }

    #[test]
    fn a_detail_dragged_by_its_gripper_lands_where_it_is_dropped() {
        let (ctx, mut overlay, state) = settings_overlay();
        overlay.settings_page = Page::Details;
        let mut time = 0.0;
        let mut frame = |overlay: &mut Overlay, events: Vec<egui::Event>| {
            time += 0.05;
            let input = egui::RawInput { events, time: Some(time), ..window(SETTINGS_SIZE) };
            ctx.run_ui(input, |ui| {
                overlay.settings_ui(ui, &state);
            })
        };
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let order = |overlay: &Overlay| -> Vec<Detail> {
            overlay.settings.panel.details.items().iter().map(|(detail, _)| *detail).collect()
        };
        let [first, second, third] = [0, 1, 2].map(|i| glance::title(Detail::ALL[i]));

        let output = frame(&mut overlay, vec![]);
        let (top, row) = (drawn(&output, first).top(), drawn(&output, second).top() - drawn(&output, first).top());
        // The gripper on the first row, the one level with its title.
        let grip = output.shapes.iter().filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) if shape.galley.text() == icon::REORDER.to_string() => {
                Some(shape.visual_bounding_rect().center())
            }
            _ => None,
        });
        let grip = grip.min_by(|a, b| (a.y - top).abs().total_cmp(&(b.y - top).abs())).unwrap();
        assert!(grip.x < drawn(&output, first).left());

        // Picked up and carried down past the middles of the next two rows.
        frame(&mut overlay, vec![egui::Event::PointerMoved(grip)]);
        frame(&mut overlay, vec![button(grip, true)]);
        let carried = 2.25 * row;
        let mut output = None;
        for step in 1..=9 {
            let at = grip + vec2(0.0, carried * step as f32 / 9.0);
            output = Some(frame(&mut overlay, vec![egui::Event::PointerMoved(at)]));
        }
        // Held there while the rows slide.
        for _ in 0..4 {
            output = Some(frame(&mut overlay, vec![]));
        }
        let output = output.unwrap();
        assert!((drawn(&output, first).top() - (top + carried)).abs() < 1.0, "it follows the pointer");
        for (title, slot) in [(second, 0.0), (third, 1.0)] {
            assert!((drawn(&output, title).top() - (top + slot * row)).abs() < 1.0, "{title} makes room");
        }
        assert_eq!(order(&overlay), Detail::ALL, "nothing moves until it's dropped");
        frame(&mut overlay, vec![button(grip + vec2(0.0, carried), false)]);
        let dropped = order(&overlay);
        assert_eq!(dropped[..4], [Detail::ALL[1], Detail::ALL[2], Detail::ALL[0], Detail::ALL[3]]);

        // Escape puts a dragged row back.
        let grip = grip + vec2(0.0, 2.0 * row);
        frame(&mut overlay, vec![egui::Event::PointerMoved(grip)]);
        frame(&mut overlay, vec![button(grip, true)]);
        frame(&mut overlay, vec![egui::Event::PointerMoved(grip - vec2(0.0, row))]);
        frame(&mut overlay, vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(&mut overlay, vec![button(grip - vec2(0.0, 2.0 * row), false)]);
        assert_eq!(order(&overlay), dropped);

        // A click on a row checks it, but not one on its gripper.
        let title = drawn(&frame(&mut overlay, vec![]), glance::title(dropped[3])).center();
        for at in [grip, title] {
            frame(&mut overlay, vec![egui::Event::PointerMoved(at)]);
            frame(&mut overlay, vec![button(at, true)]);
            frame(&mut overlay, vec![button(at, false)]);
        }
        let shown: Vec<Detail> = overlay.settings.panel.details.shown().collect();
        assert_eq!(shown, [dropped[3]]);
    }

    #[test]
    fn alt_and_an_arrow_move_the_detail_with_keyboard_focus() {
        let (ctx, mut overlay, state) = settings_overlay();
        overlay.settings_page = Page::Details;
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        };
        let frame = |overlay: &mut Overlay, events| {
            let _ = ctx.run_ui(egui::RawInput { events, ..window(SETTINGS_SIZE) }, |ui| {
                overlay.settings_ui(ui, &state);
            });
        };
        frame(&mut overlay, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(Id::new(("detail", Detail::ALL[0]))));
        frame(&mut overlay, vec![key(egui::Key::ArrowDown)]);
        frame(&mut overlay, vec![key(egui::Key::ArrowDown)]);
        let order: Vec<Detail> = overlay.settings.panel.details.items().iter().map(|(detail, _)| *detail).collect();
        assert_eq!(order[..3], [Detail::ALL[1], Detail::ALL[2], Detail::ALL[0]], "focus goes with it");
        frame(&mut overlay, vec![key(egui::Key::ArrowUp)]);
        assert_eq!(overlay.settings.panel.details.items()[1].0, Detail::ALL[0]);
    }

    /// An overlay showing the demo's usage, the context it draws in, and that usage.
    fn settings_overlay() -> (egui::Context, Overlay, Arc<State>) {
        let ctx = egui::Context::default();
        let spark = crate::mascot::spark_rgba(crate::mascot::SPARK_PIXELS);
        let side = crate::mascot::SPARK_PIXELS as u32;
        let icon = Arc::new(IconData { rgba: spark.clone(), width: side, height: side });
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let overlay = Overlay::new(&cc, Settings::default(), TimeDelta::minutes(5), true, &spark, icon);
        let state = overlay.state.lock().unwrap().clone();
        assert!(!state.limits.is_empty(), "the demo's limits fill the Limits expander");
        // The first pass loads the fonts.
        let _ = ctx.run_ui(Default::default(), |_| {});
        (ctx, overlay, state)
    }

    fn window(size: Vec2) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() }
    }

    /// Settings at their tallest: Show as's longest descriptions, a Custom double-click's card,
    /// and the Panel and Taskbar pages' notes.
    fn tallest_settings() -> impl Iterator<Item = Settings> {
        let shows = [
            BTreeSet::from([Mode::Desktop, Mode::Taskbar]),
            BTreeSet::from([Mode::Taskbar]),
            BTreeSet::from([Mode::AlwaysOnTop]),
        ];
        shows.into_iter().map(|show| Settings {
            show,
            double_click: DoubleClick::Custom,
            custom_target: r"C:\Users\Someone\Documents\A folder with a long name\open-the-usage-dashboard.ahk".into(),
            ..Settings::default()
        })
    }

    #[test]
    fn every_settings_page_fits_the_window_without_scrolling() {
        let (ctx, mut overlay, state) = settings_overlay();
        for settings in tallest_settings() {
            let show = settings.show.clone();
            overlay.settings = settings;
            // Details is a list of everything the panel can show, which scrolls like any long list.
            for page in Page::ALL.into_iter().filter(|&page| page != Page::Details) {
                overlay.settings_page = page;
                let mut height = 0.0;
                let _ = ctx.run_ui(window(SETTINGS_SIZE), |ui| height = overlay.settings_ui(ui, &state));
                assert!(height <= SETTINGS_SIZE.y, "{page:?} with {show:?} is {height} tall");
            }
        }
    }

    /// The pages once ran wider than the window, cutting off their cards' right ends and controls.
    #[test]
    fn nothing_in_settings_runs_past_the_windows_right_edge() {
        let (ctx, mut overlay, state) = settings_overlay();
        for size in [SETTINGS_SIZE, SETTINGS_MIN, vec2(1400.0, 800.0)] {
            for settings in tallest_settings() {
                overlay.settings = settings;
                for page in Page::ALL {
                    overlay.settings_page = page;
                    for title in ["Show as", "Limits", "Hear the sounds"] {
                        overlay.open_expanders.insert((page, title), true);
                    }
                    let output = ctx.run_ui(window(size), |ui| {
                        overlay.settings_ui(ui, &state);
                    });
                    // Leaving out what debug builds of egui draw over everything, such as a red
                    // box where a widget's id changed since the last page.
                    for clipped in output.shapes.iter().filter(|clipped| clipped.clip_rect != Rect::EVERYTHING) {
                        let right = clipped.shape.visual_bounding_rect().right();
                        assert!(right <= size.x + 0.5, "{page:?} at {size:?} draws out to x = {right}: {clipped:?}");
                    }
                }
            }
        }
    }

    /// The GitHub link hangs from the foot of the sidebar, so a short window brings it up
    /// toward the pages.
    #[test]
    fn the_github_link_stays_clear_of_the_pages_in_the_smallest_window() {
        let (ctx, mut overlay, state) = settings_overlay();
        let output = ctx.run_ui(window(SETTINGS_MIN), |ui| {
            overlay.settings_ui(ui, &state);
        });
        let middle_of = |name: &str| {
            let in_sidebar = output.shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == name && text.pos.x < SIDEBAR_WIDTH => {
                    Some(text.visual_bounding_rect().center().y)
                }
                _ => None,
            });
            in_sidebar.unwrap_or_else(|| panic!("the sidebar has no {name:?}"))
        };
        let last_page = Page::ALL.last().unwrap().name();
        // Sidebar entries are 36 points tall, centered on their names, with 4 between them.
        let gap = middle_of("View on GitHub") - middle_of(last_page) - 36.0;
        assert!(gap >= 4.0, "the GitHub link is {gap} below {last_page}");
    }
}
