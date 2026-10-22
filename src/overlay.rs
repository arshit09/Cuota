//! The window: a floating panel (Always on top), a panel behind other windows (Desktop), or a flyout
//! opened from the tray icons (Taskbar alone). Double-clicking it expands it with usage
//! details. It also hosts the settings window, and keeps itself sized, placed and layered for
//! the chosen modes. Picked to show on several monitors, the panel has a window on each.

use crate::animations::{self, Act, Clawd};
use crate::claude::{self, Limit, Shared, State, Step};
use crate::details;
use crate::dodge::Dodge;
use crate::glance;
use crate::log;
use crate::mascot::{CLAUDE_ORANGE, Mascot};
use crate::monitors::{self, Identify, Screen};
use crate::pet::{self, Pet};
use crate::scripts::{self, Scripts};
use crate::settings::{DoubleClick, Mode, SIZES, Settings, SoundMode, Style, Theme, View, size_step};
use crate::sound::{self, Cues};
use crate::stats::Transcripts;
use crate::status::Status;
use crate::theme::Palette;
use crate::tray::{Tray, TrayConfig, TrayEvent};
use crate::update::{self, Updater};
use crate::usage::{self, History, Task};
use crate::view;
use crate::widgets::{self, Fonts, NavIcon, SortRow, Sorted, icon, single_line};
use crate::win;
use chrono::{TimeDelta, Utc};
use eframe::egui::{
    self, Align2, Color32, FontData, FontFamily, FontId, IconData, Id, Label, Margin, MouseWheelUnit, PointerButton,
    Popup, PopupCloseBehavior, Pos2, Rect, RectAlign, RichText, Sense, Stroke, StrokeKind, UiBuilder, Vec2,
    ViewportBuilder, ViewportCommand, ViewportId, WindowLevel, pos2, vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const MARGIN: f32 = 12.0;
/// The panel's corner radius.
pub(crate) const CORNER: f32 = 12.0;
const GAP: f32 = 10.0;
/// Between Clawd's status and the meters under it.
const STATUS_GAP: f32 = 4.0;
/// Wrap width for an error message, or the details picked to show under the meters, when the
/// meters are narrower.
pub(crate) const NOTE_WIDTH: f32 = 220.0;
/// Between the meters and those details.
pub(crate) const GLANCE_GAP: f32 = 6.0;
/// The settings window, a landscape one like other Windows apps' settings: this big when the
/// screen has room, and resizable down to `SETTINGS_MIN`. A sidebar lists the pages, and the
/// one shown runs no wider than `PAGE_WIDTH`.
const SETTINGS_SIZE: Vec2 = vec2(800.0, 600.0);
const SETTINGS_MIN: Vec2 = vec2(640.0, 580.0);
const SIDEBAR_WIDTH: f32 = 248.0;
/// The menu the account at the foot of the sidebar opens.
const ACCOUNT_MENU_WIDTH: f32 = 296.0;
const PAGE_WIDTH: f32 = 760.0;
/// The page's title, above what scrolls, and the room either side of the page.
const PAGE_TITLE: f32 = 68.0;
const PAGE_GUTTER: f32 = 16.0;
/// Where Cuota's code lives, which the About page links to.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// Where Cuota's author makes content, and his other projects, which About also links to.
const INSTAGRAM: &str = "https://www.instagram.com/geek_updates/";
const WEBSITE: &str = "https://arshitvaghasiya.com";
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
/// The most tasks the Usage page lists. More are kept, and count in the averages.
const SHOWN_TASKS: usize = 100;
/// How often the panel reads what Claude Code has done since it last looked, while it shows
/// any of it.
const STATS_REFRESH: Duration = Duration::from_secs(15);
/// Keeps an expanded panel this far from its screen's edges.
const SCREEN_GAP: f32 = 8.0;
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// The shortest wait between frames: one at 60 Hz.
pub(crate) const FRAME_TIME: Duration = Duration::from_millis(16);
/// After a Show as change, the reported window position is trusted only once the move landed.
const SETTLE: Duration = Duration::from_millis(500);
/// A tray click this soon after the flyout closed is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// How often the monitors are listed again, as they're plugged in, unplugged or rearranged.
pub(crate) const MONITOR_CHECK: Duration = Duration::from_secs(2);
/// How often a panel window dropped off its monitor looks again for the mouse button's release,
/// to go back onto it.
const RELEASE_CHECK: Duration = Duration::from_millis(100);

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
/// The Updates page, like Windows Update's: whether a new Cuota is out, with a button to look
/// now or to update to it, then whether Cuota looks by itself, and every release's notes.
fn update_cards(ui: &mut egui::Ui, fonts: &Fonts, settings: &mut Settings, updater: &mut Updater) {
    use update::Status;
    let current = env!("CARGO_PKG_VERSION");
    let checked = |at: &chrono::DateTime<chrono::Local>| {
        let today = at.date_naive() == chrono::Local::now().date_naive();
        at.format(if today { "today at %H:%M" } else { "%b %-d at %H:%M" }).to_string()
    };
    let status = updater.status();
    let (title, about) = match &status {
        Status::Unchecked => (format!("Cuota {current}"), "Looks on GitHub for a newer release".to_owned()),
        Status::Checking => (format!("Cuota {current}"), "Checking for updates…".to_owned()),
        Status::UpToDate(at) => ("You're up to date".to_owned(), format!("Cuota {current}, last checked {}", checked(at))),
        Status::Available(release) if release.installable() => (
            format!("Cuota {} is out", release.version),
            format!("You have {current}. Cuota restarts to update, and keeps your settings."),
        ),
        Status::Available(release) => {
            (format!("Cuota {} is out", release.version), format!("You have {current}. Download it from its page."))
        }
        Status::Downloading(release, done) => {
            (format!("Downloading Cuota {}", release.version), format!("{:.0}%", done * 100.0))
        }
        Status::Restarting(release) => (format!("Installing Cuota {}", release.version), "Just a moment".to_owned()),
        Status::Failed { error, release: Some(release) } => (format!("Couldn't update to Cuota {}", release.version), error.clone()),
        Status::Failed { error, release: None } => ("Couldn't check for updates".to_owned(), error.clone()),
    };

    // What the button does, and the release whose page it can also open.
    let (install, release) = match &status {
        Status::Available(release) | Status::Failed { release: Some(release), .. } => (release.installable(), Some(release)),
        _ => (false, None),
    };
    let (glyph, label) = match (&status, install, release) {
        (Status::Failed { .. }, true, _) => (icon::DOWNLOAD, "Try again"),
        (_, true, _) => (icon::DOWNLOAD, "Update now"),
        (_, false, Some(_)) => (icon::EXTERNAL, "Open release page"),
        _ => (icon::REFRESH, "Check now"),
    };
    let link = install && release.is_some();
    let link_width = if link { 28.0 + ui.spacing().item_spacing.x } else { 0.0 };
    let width = widgets::button_width(ui, fonts, label) + link_width;
    let busy = matches!(status, Status::Checking | Status::Downloading(..) | Status::Restarting(_));
    widgets::setting(ui, fonts, icon::UPDATE, &title, Some(&about), width, |ui| {
        let clicked = ui
            .add_enabled_ui(!busy, |ui| {
                if install && !matches!(status, Status::Failed { .. }) {
                    widgets::accent_button(ui, fonts, glyph, label)
                } else {
                    widgets::button(ui, fonts, glyph, label)
                }
            })
            .inner
            .clicked();
        if clicked {
            match release {
                Some(_) if install => updater.install(ui.ctx()),
                Some(release) => win::open_page(&release.page),
                None => updater.check(ui.ctx()),
            }
        }
        if let Some(release) = release.filter(|_| link) {
            let notes = widgets::icon_button(ui, fonts, icon::EXTERNAL, "What's new");
            if notes.on_hover_text("What's new").clicked() {
                win::open_page(&release.page);
            }
        }
    });

    let about = Some("Looks on GitHub for a new release once a day, and installs it");
    widgets::setting(ui, fonts, icon::REFRESH, "Update automatically", about, widgets::TOGGLE_WIDTH, |ui| {
        widgets::toggle(ui, &mut settings.check_updates);
    });

    let about = Some("What changed in each version");
    let width = widgets::button_width(ui, fonts, "Open");
    widgets::setting(ui, fonts, icon::LOG, "Release notes", about, width, |ui| {
        let releases = format!("{REPOSITORY}/releases");
        if widgets::button(ui, fonts, icon::EXTERNAL, "Open").on_hover_text(&releases).clicked() {
            win::open_page(&releases);
        }
    });
}

/// The About page, like Windows' own: Cuota's icon, the Claude spark, by its name, version and
/// what it is, then links out to its code, its author's Instagram and his other projects.
fn about_cards(ui: &mut egui::Ui, fonts: &Fonts, mascot: &Mascot) {
    let version = concat!("Version ", env!("CARGO_PKG_VERSION"), " \u{B7} Free software, ", env!("CARGO_PKG_LICENSE"));
    let lines = [env!("CARGO_PKG_DESCRIPTION"), version];
    widgets::app_card(ui, fonts, "Cuota", &lines, |painter, picture| {
        mascot.paint_logo(painter, picture.center(), picture.width() / 2.0 - 4.0);
    });
    ui.add_space(8.0);

    let ctx = ui.ctx().clone();
    let links: [(NavIcon, _, _, _); 3] = [
        (widgets::github_mark(&ctx).into(), "View on GitHub", "Cuota's code, issues and releases", REPOSITORY),
        (widgets::camera_mark(&ctx).into(), "Follow on Instagram", "@geek_updates, where Arshit creates content", INSTAGRAM),
        (icon::GLOBE.into(), "More projects by Arshit", "arshitvaghasiya.com", WEBSITE),
    ];
    for (icon, title, about, url) in links {
        if widgets::link_card(ui, fonts, icon, title, Some(about)).on_hover_text(url).clicked() {
            win::open_page(url);
        }
    }
}

/// A page of the settings window, picked in its sidebar.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Page {
    /// Where Cuota shows, what double-clicking it does, and starting with Windows.
    General,
    /// The panel's size and opacity, and Clawd's animation.
    Appearance,
    /// The sound effects, how many there are, and a list to hear each one.
    Sounds,
    /// What the panel shows.
    Panel,
    /// More the panel shows under its meters, and in what order.
    Details,
    /// What the taskbar icons show.
    Taskbar,
    /// Clawd on his own on the desktop, and the card he shows.
    Pet,
    /// How much of the limits each of Claude Code's tasks took, and on average.
    Usage,
    /// Whether a new Cuota is out on GitHub, and updating to it.
    Updates,
    /// Cuota's icon, name and version, and links to its code and its author.
    About,
}

impl Page {
    const ALL: [Self; 10] = [
        Self::General,
        Self::Appearance,
        Self::Sounds,
        Self::Panel,
        Self::Details,
        Self::Taskbar,
        Self::Pet,
        Self::Usage,
        Self::Updates,
        Self::About,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Sounds => "Sounds",
            Self::Panel => "Panel",
            Self::Details => "Details",
            Self::Taskbar => "Taskbar",
            Self::Pet => "Desktop pet",
            Self::Usage => "Usage",
            Self::Updates => "Updates",
            Self::About => "About",
        }
    }

    fn icon(self, ctx: &egui::Context) -> NavIcon {
        match self {
            Self::General => icon::SETTINGS.into(),
            Self::Appearance => icon::APPEARANCE.into(),
            Self::Sounds => icon::SOUND.into(),
            Self::Panel => icon::PANEL.into(),
            Self::Details => icon::DETAILS.into(),
            Self::Taskbar => icon::TASKBAR.into(),
            Self::Pet => widgets::clawd_mark(ctx).into(),
            Self::Usage => icon::HISTORY.into(),
            Self::Updates => icon::UPDATE.into(),
            Self::About => icon::INFO.into(),
        }
    }
}

/// Log out, in the account menu at the foot of the settings sidebar, which signs Claude Code out
/// on this PC.
enum LogOut {
    Idle,
    /// Clicked once, and waiting for a second click to be sure.
    Confirming,
    /// Claude Code is signing out, on a thread of its own, which says how that went.
    Running(Receiver<Result<(), String>>),
    /// Why it couldn't, for the menu to say.
    Failed(String),
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

/// A window the panel shows in: its own (the root viewport), or a copy of it on another
/// monitor while it shows on several.
#[derive(Default)]
struct PanelWindow {
    /// The native window, once it has been set up.
    window: Option<isize>,
    size: Vec2,
    /// The window's actual size last frame, to notice when something else resizes it.
    window_size: Option<Vec2>,
    wheel: Wheel,
    /// Slides it out of the pointer's way, when that's on.
    dodge: Dodge,
    /// It shows usage details under its meters.
    expanded: bool,
    /// How far the expanded panel moved up and left to stay on screen, to move back when it
    /// shrinks.
    lift: Vec2,
    /// How tall the details were when last drawn.
    details_height: f32,
    /// The monitor it's kept on, by name, while the panel shows on several, or while it moves
    /// to the one monitor picked.
    monitor: Option<String>,
    /// On its way to its spot on `monitor`.
    placing: bool,
    /// A copy is on screen, rather than hidden until it has drawn itself where it goes.
    shown: bool,
}

impl PanelWindow {
    /// A copy of the panel for `monitor`, which goes to its spot there.
    fn copy(monitor: &str) -> Self {
        Self { monitor: Some(monitor.to_owned()), placing: true, ..Self::default() }
    }

    fn set_expanded(&mut self, expanded: bool) {
        if expanded && !self.expanded {
            // Laid out at full height first, so the window grows once rather than twice.
            self.details_height = DETAILS_HEIGHT;
        }
        self.expanded = expanded;
    }

    /// Where it sits when it isn't expanded or moved out of the pointer's way, given where its
    /// window is.
    fn home(&self, window: Rect) -> Pos2 {
        self.dodge.home().unwrap_or(window.min + self.lift)
    }

    /// Moves it up or left as little as it takes for the expanded panel to stay on screen, and
    /// back again when it shrinks. A resize by mouse wheel moves it by `wheel_shift` first, and
    /// keeps it on screen too.
    fn fit_on_screen(&mut self, ctx: &egui::Context, size: Vec2, wheel_shift: Option<Vec2>) {
        let Some(rect) = ctx.input(|i| i.viewport().outer_rect) else { return };
        // Aside for the pointer, or sliding, it's where the slide puts it.
        if self.dodge.moving() {
            return;
        }
        let mut at = self.home(rect) + wheel_shift.unwrap_or_default();
        self.lift = Vec2::ZERO;
        if (self.expanded || wheel_shift.is_some())
            && let Some(work) = work_area(ctx)
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

    /// Slides it out of the pointer's way while that's `wanted` and it sits on screen in its
    /// usual size (`placed`). Otherwise it goes straight back to its spot.
    fn dodge_pointer(&mut self, ctx: &egui::Context, wanted: bool, placed: bool) {
        let Some(window) = self.window else { return };
        let seen = ctx.input(|i| i.viewport().outer_rect).map(|rect| rect.min + self.lift);
        if let Some(at) = self.dodge.update(ctx, window, wanted && placed, !placed, seen) {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
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
    /// Clawd on his own on the desktop, while Show as includes him.
    pet: Option<Pet>,
    cues: Cues,
    fonts: Fonts,
    scripts: Scripts,
    icon: Arc<IconData>,
    keep_bottom: Arc<AtomicBool>,
    /// The panel's own window, the root viewport's.
    main: PanelWindow,
    /// Copies of it on the other monitors picked, while it shows on several.
    copies: Vec<PanelWindow>,
    /// The monitors, as last listed, and when.
    screens: Vec<Screen>,
    screens_read: Option<Instant>,
    /// The monitor the panel's own window is on, by name.
    here: Option<String>,
    /// The monitors picked for the panel that were plugged in when its windows were last
    /// arranged, if they have been.
    arranged: Option<BTreeSet<String>>,
    /// Each monitor's number on it, for a moment, after Identify is clicked in settings.
    identify: Identify,
    /// The Show as choice the window was last set up for, and when.
    applied: Option<(BTreeSet<Mode>, Instant)>,
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
    /// The size the settings window opens at, fitted to its screen.
    settings_size: Vec2,
    /// Where on screen, in pixels, the desktop pet or the tray click was when settings were
    /// asked for, so they open on that screen rather than the panel's.
    settings_near: Option<Pos2>,
    settings_page: Page,
    /// Which of the settings' expanders are open, by page and title.
    open_expanders: HashMap<(Page, &'static str), bool>,
    /// Whether Windows starts Cuota at sign-in, as last read from the registry.
    starts_with_windows: bool,
    /// Whether the Claude app is installed, as last looked up.
    claude_app_installed: bool,
    /// The Open dialog picking what a Custom double-click opens, while it's up.
    browsing: Option<Receiver<Option<String>>>,
    transcripts: Transcripts,
    /// When the panel last asked for Claude Code's stats.
    stats_asked: Option<Instant>,
    /// How much of the limits Claude Code's tasks took.
    history: History,
    /// Clear history was clicked once, and waits for a second click to be sure.
    clearing: bool,
    /// Where the account menu's Log out has got to.
    log_out: LogOut,
    /// Looks for new releases of Cuota, and puts one in place.
    updater: Updater,
    /// Whether Windows' apps are in light mode, as last read, for the panel's Windows theme.
    /// Read again whenever Windows says a setting changed.
    light_windows: bool,
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
            pet: None,
            cues: Cues::new(),
            fonts,
            scripts: Scripts::default(),
            icon,
            keep_bottom: Arc::new(AtomicBool::new(false)),
            main: PanelWindow::default(),
            copies: Vec::new(),
            screens: monitors::list(),
            screens_read: Some(Instant::now()),
            here: None,
            arranged: None,
            identify: Identify::default(),
            applied: None,
            flyout: Flyout::Closed { at: None },
            hidden: false,
            shown_frame: 0,
            settings_open: false,
            settings_requested: false,
            settings_shown: false,
            settings_size: SETTINGS_SIZE,
            settings_near: None,
            settings_page: Page::General,
            open_expanders: HashMap::new(),
            starts_with_windows: false,
            claude_app_installed: false,
            browsing: None,
            transcripts,
            stats_asked: None,
            history: if demo { History::demo() } else { History::load() },
            clearing: false,
            log_out: LogOut::Idle,
            updater: Updater::new(!demo),
            light_windows: win::light_apps(),
        }
    }

    /// The panel's colors, as picked under Appearance.
    fn palette(&self) -> &'static Palette {
        Palette::of(self.settings.theme, self.light_windows)
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
        // On several monitors, it goes to its spot on its own.
        self.main.placing = self.main.monitor.is_some();
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
        self.main.set_expanded(false);
    }

    fn open_flyout(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        let Some(bar) = win::taskbar_rect() else { return };
        let taskbar = Rect::from_min_max(pos2(bar.left as f32, bar.top as f32), pos2(bar.right as f32, bar.bottom as f32)) / ppp;
        let click = win::cursor();
        self.flyout = Flyout::Opening(taskbar, pos2(click.x as f32, click.y as f32) / ppp);
        self.shown_frame = ctx.cumulative_frame_nr();
        self.place_flyout(ctx, self.main.size);
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    /// Beside the taskbar, centered on the click, at `size` once that's known.
    fn place_flyout(&self, ctx: &egui::Context, size: Vec2) {
        let (Flyout::Opening(bar, click) | Flyout::Open(bar, click)) = self.flyout else { return };
        let size = if size == Vec2::ZERO { vec2(300.0, 80.0) } else { size };
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

    /// The monitors picked for the panel that are plugged in now.
    fn picked_screens(&self) -> Vec<&Screen> {
        self.screens.iter().filter(|screen| self.settings.panel_monitors.contains(&screen.name)).collect()
    }

    /// The panel shows on several monitors: two or more are picked and plugged in, and Always
    /// on top or Desktop keeps it on screen.
    fn on_several(&self) -> bool {
        self.settings.pinned().is_some() && self.picked_screens().len() >= 2
    }

    /// Works out the panel's windows, as monitors are picked, plugged in and unplugged: on
    /// several monitors, its own window stays on the one it's on (or else goes to the first
    /// picked) and a copy goes on each of the others. Down to one picked that it isn't on, it
    /// moves there. Runs with the root viewport's `ctx`.
    fn arrange(&mut self, ctx: &egui::Context) {
        if self.screens_read.is_none_or(|read| read.elapsed() >= MONITOR_CHECK) {
            self.screens = monitors::list();
            self.screens_read = Some(Instant::now());
        }
        let ppp = ctx.pixels_per_point();
        // Not before it's known where the panel is.
        let Some(center) = ctx.input(|i| i.viewport().outer_rect).map(|rect| rect.center() * ppp) else { return };
        self.here = monitors::at(&self.screens, center).map(|screen| screen.name.clone());

        let picked: BTreeSet<String> = self.picked_screens().into_iter().map(|screen| screen.name.clone()).collect();
        let several = self.on_several();
        if self.arranged.as_ref() != Some(&picked) {
            let here = self.here.clone().filter(|here| picked.contains(here));
            let first = picked.iter().next().cloned();
            let own = if several {
                here.clone().or(first)
            } else if self.arranged.is_some() && here.is_none() && picked.len() == 1 {
                first
            } else {
                None
            };
            // On the monitor it's on already, it stays where it is.
            self.main.placing = own.is_some() && own != here;
            self.main.monitor = own;
            self.arranged = Some(picked.clone());
        }
        if !several || self.hidden {
            // Hidden from the tray, they close, and open again with it.
            self.copies.clear();
            return;
        }
        let own = self.main.monitor.clone();
        self.copies.retain(|copy| copy.monitor.as_ref().is_some_and(|name| picked.contains(name) && Some(name) != own.as_ref()));
        for name in picked {
            if Some(&name) != own.as_ref() && !self.copies.iter().any(|copy| copy.monitor.as_ref() == Some(&name)) {
                self.first_spot(ctx, &name);
                self.copies.push(PanelWindow::copy(&name));
            }
        }
    }

    /// Where a copy goes on a monitor it hasn't been on: as far across and down it as the panel
    /// is on its own monitor. Runs with the root viewport's `ctx`.
    fn first_spot(&mut self, ctx: &egui::Context, name: &str) {
        if self.settings.panel_spots.contains_key(name) {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let Some(rect) = ctx.input(|i| i.viewport().outer_rect) else { return };
        let Some(own) = monitors::at(&self.screens, rect.center() * ppp) else { return };
        let Some(screen) = self.screens.iter().find(|screen| screen.name == name) else { return };
        // From the top-left of the room the panel has to move in, in pixels.
        let room = |work: Rect, size: Vec2| (work.size() - size).max(Vec2::splat(1.0));
        let across = ((self.main.home(rect) * ppp - own.work.min) / room(own.work, self.main.size * ppp)).clamp(Vec2::ZERO, Vec2::splat(1.0));
        let spot = across * room(screen.work, self.main.size * screen.scale) / screen.scale;
        self.settings.panel_spots.insert(name.to_owned(), [spot.x, spot.y]);
    }

    /// Keeps one of the panel's windows on its monitor while it shows on several: puts it on its
    /// spot there, back on that monitor when it's dropped on another, and remembers where it's
    /// dropped. Runs with that window's `ctx`.
    fn keep_on_monitor(&mut self, ctx: &egui::Context, window: &mut PanelWindow) {
        let Some(name) = window.monitor.clone() else { return };
        if !window.placing && !self.on_several() {
            // Moved to the one monitor picked, it goes wherever it's dragged from there.
            window.monitor = None;
            return;
        }
        let Some(screen) = self.screens.iter().find(|screen| screen.name == name) else { return };
        let Some(rect) = ctx.input(|i| i.viewport().outer_rect) else { return };
        // Laid out first, so it's known how much room it takes; and left be while it slides
        // out of the pointer's way, or expands.
        if window.size == Vec2::ZERO || window.dodge.moving() || window.expanded {
            return;
        }
        // Its monitor in this window's points, which are its monitor's own once it's there.
        let ppp = ctx.pixels_per_point();
        let (work, size) = (screen.work / ppp, window.size);
        let fit = |at: Pos2| at.min(work.max - size).max(work.min);
        let home = window.home(rect);
        if window.placing {
            let spot = self.settings.panel_spots.get(&name).map_or(Vec2::splat(SCREEN_GAP), |&[x, y]| vec2(x, y));
            let to = fit(work.min + spot * screen.scale / ppp).round();
            if (home - to).length() * ppp > 1.0 {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(to));
                ctx.request_repaint();
                return;
            }
            window.placing = false;
        }
        // Dropped partly or wholly off its monitor, it comes back on. Not while a button's
        // held, which may be dragging it.
        let fitted = fit(home).round();
        if (fitted - home).length() * ppp > 1.0 {
            if win::mouse_down() {
                repaint_after(ctx, RELEASE_CHECK);
            } else {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(fitted));
            }
            return;
        }
        let spot = (home - work.min) * ppp / screen.scale;
        let saved = self.settings.panel_spots.get(&name).map(|&[x, y]| vec2(x, y));
        if saved.is_none_or(|saved| (saved - spot).length() > 0.5) {
            self.settings.panel_spots.insert(name, [spot.x.round(), spot.y.round()]);
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

    /// The panel in its own window, the root viewport's. Returns when Clawd's next frame is
    /// due, if he animates.
    fn main_panel(&mut self, ui: &mut egui::Ui, state: &State) -> Option<Duration> {
        // Out of `self` while it's drawn, as a copy is.
        let mut main = std::mem::take(&mut self.main);
        self.keep_on_monitor(ui.ctx(), &mut main);
        let next_frame = self.panel(ui, state, &mut main);
        self.main = main;
        next_frame
    }

    /// A copy of the panel on another monitor, in a window of its own, hidden until it has drawn
    /// itself on its spot there. Returns when Clawd's next frame is due, if he animates.
    fn copy_panel(&mut self, ctx: &egui::Context, state: &State, copy: &mut PanelWindow) -> Option<Duration> {
        let name = copy.monitor.clone()?;
        let title = format!("Cuota on {name}");
        let level = if self.settings.pinned() == Some(Mode::Desktop) { WindowLevel::AlwaysOnBottom } else { WindowLevel::AlwaysOnTop };
        // As the panel's own window starts out (see main.rs), but without taking the keyboard
        // as it shows.
        let builder = ViewportBuilder::default()
            .with_title(&title)
            .with_active(false)
            .with_inner_size([300.0, 70.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_window_level(level)
            .with_visible(false)
            .with_icon(self.icon.clone());
        let id = ViewportId::from_hash_of(("panel", &name));
        show_immediate(ctx, id, builder, |ui| {
            let ctx = ui.ctx().clone();
            if copy.window.is_none()
                && let Some(window) = win::own_window(&title)
            {
                win::set_up_panel(window);
                win::keep_at_bottom(window, self.keep_bottom.clone());
                copy.window = Some(window);
            }
            if let Some(window) = copy.window {
                win::round_corners(window, CORNER * self.settings.size * ctx.pixels_per_point());
            }
            let placed = copy.shown && !copy.placing && !copy.expanded;
            copy.dodge_pointer(&ctx, self.settings.move_aside, placed);
            self.keep_on_monitor(&ctx, copy);
            let next_frame = self.panel(ui, state, copy);
            // On screen once it's on its spot at its size, already drawn there.
            if !copy.shown && !copy.placing && copy.window_size == Some(copy.size) {
                copy.shown = true;
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            }
            if !copy.shown {
                ctx.request_repaint();
            }
            next_frame
        })
        .flatten()
    }

    /// Clawd beside the meters, and usage details under them when expanded, in `window`, which
    /// is sized to fit. Returns when Clawd's next frame is due, if he animates.
    fn panel(&mut self, ui: &mut egui::Ui, state: &State, window: &mut PanelWindow) -> Option<Duration> {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let docked = self.settings.pinned().is_none();

        // Drag to move (the flyout stays put); right-click for settings; double-click as set.
        // Moved out of the pointer's way, or sliding there or back, it ignores clicks.
        let idle = window.dodge.idle();
        let sense = if idle { Sense::click_and_drag() } else { Sense::hover() };
        let background = ui.interact(full, ui.id().with("background"), sense);
        // Settings open on its screen, which for a copy isn't the panel's own.
        let near = ctx.input(|i| i.viewport().outer_rect).map(|rect| rect.center() * ctx.pixels_per_point());
        if background.secondary_clicked() {
            if self.settings_open {
                self.settings_open = false;
            } else {
                self.open_settings_near(&ctx, near);
            }
        }
        if background.double_clicked() {
            match self.settings.double_click {
                DoubleClick::Details => {
                    if !window.expanded {
                        self.stats_asked = None;
                    }
                    window.set_expanded(!window.expanded);
                }
                DoubleClick::Website => win::open_claude(false),
                DoubleClick::App => win::open_claude(true),
                // Nothing picked yet: the settings are where to pick it.
                DoubleClick::Custom if self.settings.custom_target.is_empty() => self.open_settings_near(&ctx, near),
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
        let palette = self.palette();
        painter.rect_filled(full, CORNER * scale, palette.background(self.settings.opacity));
        painter.rect_stroke(full, CORNER * scale, Stroke::new(1.0, palette.border), StrokeKind::Inside);

        // Lay everything out first so the window can be sized to fit.
        let view = &self.settings.panel;
        let now = Utc::now();
        let limits: Vec<&Limit> = state.limits.iter().filter(|l| view.shows(&l.label)).collect();
        let builder = view::Builder { painter: &painter, strong: &self.fonts.strong, view, palette, now, scale };
        let meters = builder.build(&limits);
        let width = meters.size.x.max(NOTE_WIDTH * scale);
        let font = FontId::proportional(12.0 * scale);
        let note = note(state, palette).map(|(text, color)| painter.layout(text, font, color, width));
        // Whatever more was picked to show under them.
        let stats = self.transcripts.latest();
        let source = glance::Source { state, stats: stats.as_deref(), limits: &limits, now };
        let glance = glance::Builder { painter: &painter, source, palette, scale, width }.build(view.details.shown());
        // Clawd's status, when it shows, heads the meters.
        let animate = self.settings.animate;
        let status = view.clawd && view.status;
        let clawd = if view.clawd {
            self.clawd.update(state, animate, Act::Mood);
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
        let details = window.expanded.then(|| {
            let room = work_area(&ctx).map_or(DETAILS_HEIGHT, |work| work.height() - row.y - 2.0 * (margin + SCREEN_GAP));
            vec2(DETAILS_WIDTH, window.details_height.min(DETAILS_HEIGHT).min(room).max(0.0))
        });
        let inner = details.map_or(row, |details| vec2(row.x.max(details.x), row.y + details.y));
        let size = (inner + Vec2::splat(2.0 * margin)).round();
        let wheel_shift = window.wheel.shift.take();
        // Scripts and window managers can resize it too (one put it back to its first size,
        // cropping the panel), so a size changed to anything else is asked for again. Only
        // when it changes, so a size Windows won't allow isn't asked for every frame.
        let window_size = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size().round());
        let resized = window_size != window.window_size && window_size.is_some_and(|actual| actual != size);
        window.window_size = window_size;
        if size != window.size || resized {
            window.size = size;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            self.place_flyout(&ctx, size);
            if !docked {
                window.fit_on_screen(&ctx, size, wheel_shift);
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
            let next = self.status.paint(&painter, body_at, working, animate, scale, palette);
            next_frame = next_frame.into_iter().chain(next).min();
        }
        let meters_at = body_at + vec2(0.0, status_size.y + status_gap);
        let mut below = meters_at.y + meters.size.y + note_gap;
        let mut hints = meters.paint(&painter, meters_at);
        if let Some(note) = note {
            painter.galley(pos2(body_at.x, below), note, palette.text);
            below += note_size.y;
        }
        hints.extend(glance.paint(&painter, pos2(body_at.x, below + glance_gap)));
        for (i, (rect, hint)) in hints.into_iter().enumerate() {
            ui.interact(rect, ui.id().with(("hint", i)), Sense::hover()).on_hover_text(hint);
        }

        if let Some(area) = details {
            painter.hline(full.left() + margin..=full.right() - margin, area.top() + 6.0, Stroke::new(1.0, palette.border));
            let mut slot = ui.new_child(UiBuilder::new().max_rect(area));
            // A scroll bar that shows up on a light panel. Hover text stays dark: egui gives
            // all windows one style for it.
            if *palette == Palette::LIGHT {
                slot.style_mut().visuals = egui::Visuals::light();
            }
            // Dragging anywhere moves the panel, so the details scroll by wheel and scroll bar.
            let scroll_by = egui::scroll_area::ScrollSource { drag: egui::scroll_area::ScrollSource::NONE.drag, ..Default::default() };
            let scroll = egui::ScrollArea::vertical()
                .id_salt("details")
                .max_height(area.height())
                .auto_shrink([false, true])
                .scroll_source(scroll_by)
                .show(&mut slot, |ui| {
                    let lane = Margin { right: SCROLL_LANE as i8, ..Margin::ZERO };
                    let details = |ui: &mut egui::Ui| details::ui(ui, &self.fonts, palette, state, stats.as_deref());
                    egui::Frame::new().inner_margin(lane).show(ui, details);
                });
            let height = scroll.content_size.y.ceil();
            if (height - window.details_height).abs() >= 1.0 {
                window.details_height = height;
                ctx.request_repaint();
            }
        }
        if self.settings.wheel_resize && idle {
            window.wheel.resize(ui, details, &mut self.settings);
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
        // No sooner than a frame: Clawd's next frame, his status's star and its typing each ask
        // for their own next change, and together they'd draw the panel a few milliseconds apart.
        if let Some(next) = next {
            repaint_after(ctx, next.max(FRAME_TIME));
        }
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        self.open_settings_near(ctx, None);
    }

    /// Opens settings on the screen with `near` on it, a point in pixels, or else the panel's.
    fn open_settings_near(&mut self, ctx: &egui::Context, near: Option<Pos2>) {
        if !self.settings_open {
            self.settings_shown = false;
            self.settings_page = Page::General;
            self.settings_near = near;
            // Its usual size, or as much of it as fits on its screen.
            self.settings_size = self.settings_screen(ctx).map_or(SETTINGS_SIZE, |screen| {
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

    /// Opens settings at `page`, on the screen with `near` on it, as right-clicking the desktop
    /// pet does.
    fn open_settings_at(&mut self, ctx: &egui::Context, page: Page, near: Pos2) {
        self.open_settings_near(ctx, Some(near));
        self.settings_page = page;
    }

    /// The part of the screen settings open on that the taskbar leaves free, in points: the
    /// desktop pet's when he asked for them, the panel's otherwise.
    fn settings_screen(&self, ctx: &egui::Context) -> Option<Rect> {
        let Some(near) = self.settings_near else { return work_area(ctx) };
        let work = win::work_area(near.x as i32, near.y as i32)?;
        let work = Rect::from_min_max(pos2(work.left as f32, work.top as f32), pos2(work.right as f32, work.bottom as f32));
        Some(work / ctx.pixels_per_point())
    }

    /// Keeps the desktop pet on screen while Show as includes him, handing him what he shows,
    /// and does what he asks. His window closes once this stops being called.
    fn keep_pet(&mut self, ctx: &egui::Context, state: &Arc<State>) {
        if !self.settings.shows_as(Mode::Pet) {
            self.pet = None;
            return;
        }
        let given = pet::Given {
            state: state.clone(),
            stats: self.transcripts.latest(),
            pet: self.settings.pet.clone(),
            animate: self.settings.animate,
            opacity: self.settings.opacity,
            palette: *self.palette(),
            double_click: self.settings.double_click,
            custom_target: self.settings.custom_target.clone(),
        };
        let pet = self.pet.get_or_insert_with(|| Pet::new(ctx, &self.fonts, given.clone()));
        pet.show(ctx, given);
        let asks = pet.take_asks();
        if let Some(near) = asks.settings {
            if self.settings_open {
                self.settings_open = false;
            } else {
                self.open_settings_at(ctx, Page::Pet, near);
            }
        }
        if let Some(spot) = asks.spot {
            self.settings.pet.pos = Some(spot);
        }
        if asks.stats {
            self.refresh_stats(state);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context, state: &State) {
        // Its screen, in pixels: where a new window opens.
        let panel_screen =
            if self.settings_shown { None } else { self.settings_screen(ctx).map(|work| work * ctx.pixels_per_point()) };
        let builder = ViewportBuilder::default()
            .with_title("Cuota Settings")
            .with_inner_size(self.settings_size)
            .with_min_inner_size(SETTINGS_MIN)
            .with_always_on_top()
            .with_visible(false)
            .with_icon(self.icon.clone());
        show_immediate(ctx, settings_viewport(), builder, |ui| {
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

    /// A sidebar with the account, the pages and GitHub, beside the page picked in it, as in
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

    /// The pages, then the account at the bottom. Quitting is left to the tray icon's menu, and
    /// Cuota's version and GitHub to the About page.
    fn settings_sidebar(&mut self, ui: &mut egui::Ui, state: &State) {
        ui.spacing_mut().item_spacing.y = 4.0;
        for page in Page::ALL {
            if widgets::nav_item(ui, &self.fonts, page.icon(ui.ctx()), page.name(), self.settings_page == page).clicked() {
                self.settings_page = page;
            }
        }

        let bottom = ui.max_rect();
        let account_at = pos2(bottom.left(), bottom.bottom() - widgets::ACCOUNT_HEIGHT);
        let account = Rect::from_min_size(account_at, vec2(bottom.width(), widgets::ACCOUNT_HEIGHT));
        let mut slot = ui.new_child(UiBuilder::new().max_rect(account));
        self.settings_account(&mut slot, state);

        // Once a new release is out, it says so above the account, and a click goes to where
        // it's installed.
        let update = match self.updater.status() {
            update::Status::Available(release) => Some(release.version),
            _ => None,
        };
        let Some(update) = update else { return };
        // Lined up with the entries' icons.
        let at = pos2(account.left() + 14.0, account.top() - 4.0 - 10.0);
        let text = format!("Update to v{update}");
        let rect = ui.painter().text(at, Align2::LEFT_CENTER, text, FontId::proportional(12.0), CLAUDE_ORANGE);
        let response = ui.interact(rect.expand(4.0), ui.id().with("update"), Sense::click());
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Go to Updates").clicked() {
            self.settings_page = Page::Updates;
        }
    }

    /// The account Claude Code is signed in with, by its email and plan. A click opens a menu
    /// above it with the plan's details, Refresh and Log out.
    fn settings_account(&mut self, ui: &mut egui::Ui, state: &State) {
        let account = &state.account;
        let menu = Id::new("account menu");
        let open = Popup::is_id_open(ui.ctx(), menu);
        // A second click is only sure while the menu that asked is still open.
        if !open && matches!(self.log_out, LogOut::Confirming | LogOut::Failed(_)) {
            self.log_out = LogOut::Idle;
        }
        self.finish_log_out(ui.ctx(), menu);
        // The plan goes under the email, unless it's standing in for it.
        let caption = account.email.as_ref().and(account.plan.as_deref());
        let initial = initial(account.email.as_deref());
        let tile = widgets::account_item(ui, &self.fonts, initial, account_name(account), caption, open);
        Popup::menu(&tile)
            .id(menu)
            .align(RectAlign::TOP_START)
            .align_alternatives(&[RectAlign::BOTTOM_START])
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .width(ACCOUNT_MENU_WIDTH)
            .gap(4.0)
            .frame(widgets::flyout_frame())
            .show(|ui| self.account_menu(ui, state));
    }

    /// The account menu: who's signed in, what Claude Code knows of the plan and when usage was
    /// last fetched, then Refresh, and Log out, which takes a second click to be sure.
    fn account_menu(&mut self, ui: &mut egui::Ui, state: &State) {
        const PICTURE: f32 = 20.0;
        let account = &state.account;
        let signed_in = account.email.is_some() || account.plan.is_some();
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_min_width(ui.available_width());

        let width = ui.available_width();
        let text_left = 12.0 + 2.0 * PICTURE + 12.0;
        let room = width - text_left - 12.0;
        let painter = ui.painter().clone();
        let name = single_line(&painter, account_name(account), FontId::new(14.0, self.fonts.strong.clone()), widgets::PRIMARY, room);
        let caption = match (&account.email, &account.plan) {
            (Some(_), Some(plan)) => Some(plan.as_str()),
            (None, None) => Some("Sign in with /login in Claude Code"),
            _ => None,
        };
        let caption = caption.map(|caption| single_line(&painter, caption, FontId::proportional(12.0), widgets::SECONDARY, room));
        let lines: Vec<_> = account_lines(account, state.updated)
            .iter()
            .map(|line| single_line(&painter, line, FontId::proportional(12.0), widgets::SECONDARY, room))
            .collect();
        // The email and plan beside the picture, and the details under them.
        let header_height = name.size().y + caption.as_ref().map_or(0.0, |caption| caption.size().y);
        let header = header_height.max(2.0 * PICTURE);
        let lines_height: f32 = lines.iter().map(|line| line.size().y).sum();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 10.0 + header + 6.0 + lines_height + 10.0), Sense::hover());
        let middle = rect.top() + 10.0 + header / 2.0;
        let picture = pos2(rect.left() + 12.0 + PICTURE, middle);
        widgets::avatar(&painter, &self.fonts, picture, PICTURE, initial(account.email.as_deref()));
        let mut at = pos2(rect.left() + text_left, middle - header_height / 2.0);
        let paint = |galleys: Vec<Arc<egui::Galley>>, at: &mut Pos2| {
            for galley in galleys {
                let height = galley.size().y;
                painter.galley(*at, galley, widgets::PRIMARY);
                at.y += height;
            }
        };
        paint([Some(name), caption].into_iter().flatten().collect(), &mut at);
        at.y = middle + header / 2.0 + 6.0;
        paint(lines, &mut at);

        widgets::menu_separator(ui);
        if widgets::menu_item(ui, &self.fonts, icon::REFRESH, "Refresh").on_hover_text("Fetch usage now").clicked() {
            let _ = self.refresh.send(());
        }
        if !signed_in {
            return;
        }
        let running = matches!(self.log_out, LogOut::Running(_));
        let label = match self.log_out {
            LogOut::Confirming => "Yes, log out",
            LogOut::Running(_) => "Logging out…",
            LogOut::Idle | LogOut::Failed(_) => "Log out",
        };
        let item = ui.add_enabled_ui(!running, |ui| widgets::menu_item(ui, &self.fonts, icon::SIGN_OUT, label)).inner;
        if item.on_hover_text("Signs Claude Code out on this PC").clicked() {
            if matches!(self.log_out, LogOut::Confirming) {
                self.start_log_out(ui.ctx());
            } else {
                self.log_out = LogOut::Confirming;
            }
        }
        let note = match &self.log_out {
            LogOut::Confirming => "Click again to sign Claude Code out on this PC. Cuota uses its login, so it signs out too.",
            LogOut::Failed(why) => why.as_str(),
            LogOut::Idle | LogOut::Running(_) => return,
        };
        egui::Frame::new().inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.add(Label::new(RichText::new(note).size(12.0).color(widgets::SECONDARY)).wrap());
        });
    }

    /// Runs Claude Code's logout on a thread of its own, then fetches usage again, so settings
    /// show who's signed in now.
    fn start_log_out(&mut self, ctx: &egui::Context) {
        let (done, result) = mpsc::channel();
        let (ctx, refresh) = (ctx.clone(), self.refresh.clone());
        std::thread::spawn(move || {
            let _ = done.send(claude::log_out());
            let _ = refresh.send(());
            ctx.request_repaint();
        });
        self.log_out = LogOut::Running(result);
    }

    /// Once Claude Code has logged out, closes the account `menu`, or opens it again to say why
    /// it couldn't.
    fn finish_log_out(&mut self, ctx: &egui::Context, menu: Id) {
        let LogOut::Running(result) = &self.log_out else { return };
        match result.try_recv() {
            Ok(Ok(())) => {
                self.log_out = LogOut::Idle;
                Popup::close_id(ctx, menu);
            }
            Ok(Err(why)) => {
                self.log_out = LogOut::Failed(why);
                Popup::open_id(ctx, menu);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.log_out = LogOut::Idle,
        }
    }

    /// The page picked in the sidebar: a card for each setting, as in Windows' Settings.
    fn settings_page(&mut self, ui: &mut egui::Ui, state: &State) {
        let Self {
            settings,
            settings_page: page,
            open_expanders,
            fonts,
            starts_with_windows,
            claude_app_installed,
            browsing,
            screens,
            here,
            identify,
            history,
            clearing,
            updater,
            light_windows,
            mascot,
            ..
        } = self;
        let page = *page;

        let note = match page {
            Page::Panel | Page::Details if settings.pinned().is_none() && settings.shows_as(Mode::Taskbar) => {
                Some("The panel opens when you click a taskbar icon.")
            }
            Page::Panel | Page::Details if settings.pinned().is_none() => {
                Some("These apply once Show as, under General, includes Always on top, Desktop or Taskbar.")
            }
            Page::Taskbar if !settings.shows_as(Mode::Taskbar) => {
                Some("These apply once Show as, under General, includes Taskbar.")
            }
            Page::Pet if !settings.shows_as(Mode::Pet) => {
                Some("These apply once Show as, under General, includes Desktop pet.")
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

                // For the panel that stays on screen, not the flyout. Picked on fewer than two,
                // it's the one it's on.
                ui.add_enabled_ui(settings.pinned().is_some(), |ui| {
                    let present = |name: &String| screens.iter().any(|screen| screen.name == *name);
                    let mut picked: BTreeSet<String> = settings.panel_monitors.iter().filter(|name| present(name)).cloned().collect();
                    if picked.len() < 2 {
                        picked = here.iter().cloned().collect();
                    }
                    let about = "Show a copy of the panel on each monitor picked";
                    if monitor_list(ui, fonts, open_expanders, page, about, screens, &mut picked, identify) {
                        // Monitors unplugged now stay picked for when they're back.
                        settings.panel_monitors.retain(|name| !present(name));
                        settings.panel_monitors.extend(picked);
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

                // For the panel that stays on screen, not the flyout.
                ui.add_enabled_ui(settings.pinned().is_some(), |ui| {
                    let about = Some("Slides away when the pointer slows down coming to it. Hold Ctrl to reach it.");
                    let title = "Move aside for the pointer";
                    widgets::setting(ui, fonts, icon::MOVE_ASIDE, title, about, widgets::TOGGLE_WIDTH, |ui| {
                        widgets::toggle(ui, &mut settings.move_aside);
                    });
                });

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
                let themes = [(Theme::Dark, "Dark"), (Theme::Light, "Light"), (Theme::Windows, "Use Windows setting")];
                let width = widgets::dropdown_width(ui, &themes.map(|(_, name)| name));
                let about = match settings.theme {
                    Theme::Windows if *light_windows => "Light now, as Windows' apps are",
                    Theme::Windows => "Dark now, as Windows' apps are",
                    _ => "Panel colors",
                };
                widgets::setting(ui, fonts, icon::THEME, "Theme", Some(about), width, |ui| {
                    widgets::dropdown(ui, fonts, &mut settings.theme, &themes, width);
                });
                let about = Some("Turn off to use almost no CPU");
                widgets::setting(ui, fonts, icon::ANIMATE, "Animate Clawd", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut settings.animate);
                });
            }
            Page::Sounds => {
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
                style_and_limits(ui, fonts, open_expanders, page, view, state);
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
                let stats = self.transcripts.latest();
                let about = "Pick what the panel shows there, and drag them into order";
                details_list(ui, fonts, &mut settings.panel, state, stats.as_deref(), "Under the meters", about);
            }
            Page::Pet => {
                let pet = &mut settings.pet;
                let size = format!("{:.0}%", pet.size * 100.0);
                widgets::setting(ui, fonts, icon::SIZE, "Size", Some("Small to large"), SLIDER_ROW, |ui| {
                    value_label(ui, size);
                    let mut step = size_step(pet.size);
                    widgets::stepped_slider(ui, &mut step, SIZES.len(), SLIDER_WIDTH);
                    pet.size = SIZES[step];
                });
                let about = Some("What he's up to, like Percolating…, under him");
                widgets::setting(ui, fonts, icon::STATUS, "Status", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut pet.status);
                });
                let about = Some("His eyes follow it. He comes over when Claude needs you, and now and then to say hi, keep it company or nap beside it.");
                widgets::setting(ui, fonts, icon::REACT, "React to pointer", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut pet.react);
                });
                let present = |name: &String| screens.iter().any(|screen| screen.name == *name);
                let about = Some("He walks over to the monitors picked below, not just the one he's on");
                widgets::setting(ui, fonts, icon::TRAVEL, "Travel across monitors", about, widgets::TOGGLE_WIDTH, |ui| {
                    // At first, all of them.
                    if widgets::toggle(ui, &mut pet.travel).changed() && pet.travel && !pet.monitors.iter().any(present) {
                        pet.monitors.extend(screens.iter().map(|screen| screen.name.clone()));
                    }
                });
                ui.add_enabled_ui(pet.travel, |ui| {
                    let mut picked: BTreeSet<String> = pet.monitors.iter().filter(|name| present(name)).cloned().collect();
                    let about = "The ones he walks across";
                    if monitor_list(ui, fonts, open_expanders, page, about, screens, &mut picked, identify) {
                        pet.monitors.retain(|name| !present(name));
                        pet.monitors.extend(picked);
                    }
                });
                let about = Some("Rest the pointer on him to see your usage beside him. Passing over him doesn't count.");
                widgets::setting(ui, fonts, icon::HOVER_CARD, "Card on hover", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut pet.hover);
                });
                // What the card shows.
                let stats = self.transcripts.latest();
                ui.add_enabled_ui(pet.hover, |ui| {
                    let card = &mut pet.card;
                    style_and_limits(ui, fonts, open_expanders, page, card, state);
                    for (glyph, title, about, on) in [
                        (icon::PERCENT, "Percentage", None, &mut card.percent),
                        (icon::TIME_LEFT, "Time left", Some("Until each limit resets"), &mut card.time_left),
                    ] {
                        widgets::setting(ui, fonts, glyph, title, about, widgets::TOGGLE_WIDTH, |ui| {
                            widgets::toggle(ui, on);
                        });
                    }
                    let about = "Pick what the card shows under the meters, and drag them into order";
                    details_list(ui, fonts, card, state, stats.as_deref(), "Also in the card", about);
                });
            }
            Page::Usage => {
                let about = Some("Notes your limits as a Claude Code session starts a task, and again once it's done");
                widgets::setting(ui, fonts, icon::HISTORY, "Track usage per task", about, widgets::TOGGLE_WIDTH, |ui| {
                    widgets::toggle(ui, &mut settings.track_usage);
                });
                if !settings.track_usage {
                    ui.add_space(8.0);
                    let note = "Turn on tracking to see how much of your limits each task takes. Nothing is noted while it's off.";
                    widgets::info_bar(ui, fonts, note);
                } else if history.tasks.is_empty() {
                    ui.add_space(8.0);
                    let note = "No tasks yet. Each one shows up here once a Claude Code session finishes it and Cuota has fetched usage after it.";
                    widgets::info_bar(ui, fonts, note);
                } else {
                    usage_history(ui, fonts, history, clearing);
                }
            }
            Page::Updates => update_cards(ui, fonts, settings, updater),
            Page::About => about_cards(ui, fonts, mascot),
        }
    }
}

/// What tasks took on average on each plan, the tasks themselves, newest first, and a button
/// to clear them.
fn usage_history(ui: &mut egui::Ui, fonts: &Fonts, history: &mut History, clearing: &mut bool) {
    let tasks = |count: usize| if count == 1 { "1 task".to_owned() } else { format!("{count} tasks") };
    let averages = history.averages();
    if !averages.is_empty() {
        let about = Some("By plan, since a bigger plan's limits go further. Tasks run alongside other sessions are left out.");
        let plans = if averages.len() == 1 { "1 plan".to_owned() } else { format!("{} plans", averages.len()) };
        widgets::group(ui, fonts, icon::AVERAGE, "Average task", about, &plans, |ui| {
            for average in &averages {
                let used: Vec<String> =
                    average.used.iter().map(|(label, used)| format!("{label} {}", usage::used_text(*used))).collect();
                let epoch = chrono::DateTime::UNIX_EPOCH;
                let length = claude::until(epoch + average.duration, epoch);
                let description = format!("{} \u{B7} {length} each\n{}", tasks(average.tasks), used.join(" \u{B7} "));
                let value = average.used.first().map_or_else(String::new, |(_, used)| usage::used_text(*used));
                widgets::info_item(ui, fonts, &average.plan, Some(&description), &value);
            }
        });
    }

    let about = Some("Newest first, with how much of each limit they took");
    widgets::group(ui, fonts, icon::HISTORY, "Tasks", about, &tasks(history.tasks.len()), |ui| {
        let today = chrono::Local::now().date_naive();
        for task in history.tasks.iter().take(SHOWN_TASKS) {
            let value = task.headline().map_or_else(String::new, |change| usage::used_text(change.used()));
            widgets::info_item(ui, fonts, &task.session, Some(&task_about(task, today)), &value);
        }
    });

    let count = tasks(history.tasks.len());
    let (about, label) = if *clearing {
        (format!("Click again to delete all {count}"), "Yes, clear")
    } else {
        (format!("Deletes the {count} noted so far"), "Clear")
    };
    let width = widgets::button_width(ui, fonts, "Yes, clear");
    widgets::setting(ui, fonts, icon::CLEAR, "Clear history", Some(&about), width, |ui| {
        if widgets::button(ui, fonts, icon::CLEAR, label).clicked() {
            if *clearing {
                history.clear();
            }
            *clearing = !*clearing;
        }
    });
    if let Some(path) = History::path().filter(|path| path.exists()) {
        let about = Some("Where the tasks are kept, as JSON");
        let width = widgets::button_width(ui, fonts, "Open folder");
        widgets::setting(ui, fonts, icon::LOG, "History file", about, width, |ui| {
            let button = widgets::button(ui, fonts, icon::FOLDER, "Open folder");
            if button.on_hover_text(path.display().to_string()).clicked() {
                win::show_in_folder(&path);
            }
        });
    }
}

/// What names the account at the foot of the settings sidebar: its email, or its plan without
/// one, or that no one's signed in.
fn account_name(account: &claude::Account) -> &str {
    account.email.as_deref().or(account.plan.as_deref()).unwrap_or("Not signed in")
}

/// The first letter or digit of `email`, in capitals, for the account's picture.
fn initial(email: Option<&str>) -> Option<char> {
    let first = email?.chars().find(|c| c.is_alphanumeric())?;
    first.to_uppercase().next()
}

/// What the account menu tells under the email and plan: only what Claude Code knows of the
/// account, a line each, then when usage was last fetched.
fn account_lines(account: &claude::Account, updated: Option<chrono::DateTime<chrono::Local>>) -> Vec<String> {
    let mut lines: Vec<String> = account.organization.iter().cloned().collect();
    lines.extend(account.role.as_ref().map(|role| format!("Role: {role}")));
    lines.extend(account.billing.as_ref().map(|billing| format!("Billed through {billing}")));
    let since = account.since.map(|since| since.with_timezone(&chrono::Local));
    lines.extend(since.map(|since| format!("Subscribed since {}", since.format("%b %Y"))));
    lines.push(updated.map_or_else(|| "Usage not fetched yet".to_owned(), |at| at.format("Usage updated %H:%M").to_string()));
    lines
}

/// When a task ran and where, then what it took of each limit.
fn task_about(task: &Task, today: chrono::NaiveDate) -> String {
    let started = task.started.with_timezone(&chrono::Local);
    let day = match (today - started.date_naive()).num_days() {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ => started.format("%b %-d").to_string(),
    };
    let mut about = vec![format!("{day} {}", started.format("%H:%M")), claude::until(task.ended, task.started)];
    about.extend(task.project.clone());
    about.push(task.app.clone());
    about.extend(task.plan.clone());
    let changes: Vec<String> = task
        .changes
        .iter()
        .map(|c| {
            if c.reset {
                format!("{} reset, then {:.0}%", c.label, c.after)
            } else {
                format!("{} {:.0}% \u{2192} {:.0}%", c.label, c.before, c.after)
            }
        })
        .collect();
    let mut text = format!("{}\n{}", about.join(" \u{B7} "), changes.join(" \u{B7} "));
    if task.shared {
        text.push_str("\nRan alongside other sessions, so this counts their usage too");
    }
    text
}

/// How a view's limits are drawn, and which ones it shows.
fn style_and_limits(
    ui: &mut egui::Ui,
    fonts: &Fonts,
    open_expanders: &mut HashMap<(Page, &'static str), bool>,
    page: Page,
    view: &mut View,
    state: &State,
) {
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
}

/// An expander listing the monitors by number, with their size and place, to pick by checking,
/// and a row that shows each one's number on it. At least one stays picked. Returns whether
/// `picked` changed.
fn monitor_list(
    ui: &mut egui::Ui,
    fonts: &Fonts,
    open_expanders: &mut HashMap<(Page, &'static str), bool>,
    page: Page,
    about: &str,
    screens: &[Screen],
    picked: &mut BTreeSet<String>,
    identify: &mut Identify,
) -> bool {
    let numbers: Vec<usize> = screens.iter().filter(|screen| picked.contains(&screen.name)).map(|screen| screen.number).collect();
    let open = open_expanders.entry((page, "Monitors")).or_default();
    let mut changed = false;
    widgets::expander(ui, fonts, open, icon::MONITORS, "Monitors", Some(about), &monitors::summary(&numbers), |ui| {
        for screen in screens {
            let mut on = picked.contains(&screen.name);
            let last = on && picked.len() == 1;
            let about = screen.about(screens);
            let item = ui.add_enabled(!last, |ui: &mut egui::Ui| widgets::check_item(ui, fonts, &mut on, &screen.title(), Some(&about)));
            if item.on_disabled_hover_text("At least one stays on").changed() {
                if on {
                    picked.insert(screen.name.clone());
                } else {
                    picked.remove(&screen.name);
                }
                changed = true;
            }
        }
        let about = Some("Shows each monitor's number on it for a moment");
        if widgets::action_item(ui, fonts, icon::IDENTIFY, "Identify", about).clicked() {
            identify.start();
        }
    });
    changed
}

/// The details a view can show under its meters, to pick and drag into order, in a group
/// called `title`.
fn details_list(
    ui: &mut egui::Ui,
    fonts: &Fonts,
    view: &mut View,
    state: &State,
    stats: Option<&crate::stats::Stats>,
    title: &str,
    about: &str,
) {
    // Each row's example is what would show now, where it can.
    let limits: Vec<&Limit> = state.limits.iter().filter(|limit| view.shows(&limit.label)).collect();
    let source = glance::Source { state, stats, limits: &limits, now: Utc::now() };
    let items = view.details.items();
    let examples: Vec<String> = items.iter().map(|&(detail, _)| glance::example(detail, &source)).collect();
    let rows: Vec<SortRow> = items
        .iter()
        .zip(&examples)
        .map(|(&(detail, on), example)| SortRow {
            id: Id::new((title, detail)),
            on,
            title: glance::title(detail),
            description: example,
        })
        .collect();
    let summary = match view.details.shown().count() {
        0 => "None".to_owned(),
        shown => format!("{shown} shown"),
    };
    let sorted = widgets::group(ui, fonts, icon::CHECKLIST, title, Some(about), &summary, |ui| {
        widgets::sortable(ui, fonts, &rows)
    });
    match sorted {
        Some(Sorted::Toggled(index)) => view.details.toggle(index),
        Some(Sorted::Moved { from, to }) => view.details.move_item(from, to),
        None => {}
    }
}

impl eframe::App for Overlay {
    /// Runs even while the window is hidden, so tray clicks get through.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.main.window.is_none()
            && let Some(window) = native_window(frame)
        {
            win::set_up_panel(window);
            win::keep_at_bottom(window, self.keep_bottom.clone());
            win::use_exe_icon(window);
            self.main.window = Some(window);
        }
        if let Some(window) = self.main.window {
            win::round_corners(window, CORNER * self.settings.size * ctx.pixels_per_point());
        }
        self.apply_modes(ctx);
        self.arrange(ctx);
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
                // Light or dark mode may have changed, for the Windows theme to follow.
                TrayEvent::WindowsSettings => {
                    self.light_windows = win::light_apps();
                    continue;
                }
            };
            if details {
                if !self.main.expanded {
                    self.stats_asked = None;
                }
                self.main.set_expanded(true);
            }
            if self.settings.pinned().is_some() {
                // A click hides or shows the panel; settings and details bring it back if it was hidden.
                self.set_hidden(ctx, !settings && !details && !self.hidden);
                self.settings_requested |= settings;
                ctx.request_repaint();
            } else if !self.settings.shows_as(Mode::Taskbar) {
                // Only the desktop pet shows, so there's no panel to open: a click on the tray
                // icon opens settings instead, as do its menu and starting Cuota again. They
                // open by the pointer, on the taskbar's screen rather than the hidden panel's.
                if !details {
                    let at = win::cursor();
                    self.open_settings_near(ctx, Some(pos2(at.x as f32, at.y as f32)));
                }
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

        // Out of the pointer's way, while the panel sits on screen in its usual size. Hidden,
        // expanded or being placed again, it goes straight back to its spot. Its copies on
        // other monitors do the same as they're drawn.
        let settled = self.applied.as_ref().is_some_and(|(_, at)| at.elapsed() > SETTLE);
        let main = &self.main;
        let placed = self.settings.pinned().is_some() && !self.hidden && !main.expanded && !main.placing && settled;
        self.main.dodge_pointer(ctx, self.settings.move_aside, placed);

        // Kept on screen whatever puts it away, such as Windows signing in or another program.
        // What hides it at sign-in isn't known yet, so the log says what was found.
        if settled
            && self.settings.pinned().is_some()
            && !self.hidden
            && !self.main.dodge.moving()
            && let Some(window) = self.main.window
            && let Some(away) = win::put_away(window)
        {
            log::warn!("Something left the panel {away}, so Cuota put it back on screen");
            win::bring_back(window, self.settings.pinned() == Some(Mode::AlwaysOnTop));
            ctx.request_repaint();
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
            repaint_after(ctx, due.saturating_duration_since(now));
        }
        self.keep_pet(ctx, &state);

        // Like the sounds, tasks are followed whether or not anything shows them.
        if self.history.update(&state, self.settings.track_usage, Utc::now()) && self.settings_open {
            ctx.request_repaint();
        }

        if let Some(due) = self.updater.tick(ctx, self.settings.check_updates) {
            repaint_after(ctx, due);
        }
        // The new copy waits for this one to quit before it shows.
        if matches!(self.updater.status(), update::Status::Restarting(_)) {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        }

        // Titles, folders and the account come in whatever script they were written in.
        let stats = self.transcripts.latest();
        let sessions = state.sessions.iter().flat_map(|s| [s.title.as_deref(), Some(&*s.name), s.project.as_deref()]);
        let tasks = self.history.tasks.iter().take(SHOWN_TASKS).flat_map(|t| [Some(&*t.session), t.project.as_deref()]);
        let sessions = sessions.chain(tasks);
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
            let mut next = self.main_panel(ui, &state);
            for index in 0..self.copies.len() {
                // Out of `self` while it's drawn, as the panel's own window is.
                let mut copy = std::mem::take(&mut self.copies[index]);
                let copy_next = self.copy_panel(&ctx, &state, &mut copy);
                self.copies[index] = copy;
                next = next.into_iter().chain(copy_next).min();
            }
            next_frame = Some(next.unwrap_or_else(|| {
                Duration::from_millis(60_000 - Utc::now().timestamp_millis().rem_euclid(60_000) as u64)
            }));
            // Claude Code's stats, while something shows them.
            let details = &self.settings.panel.details;
            let expanded = self.main.expanded || self.copies.iter().any(|copy| copy.expanded);
            if expanded || details.use_stats() || self.settings_open && self.settings_page == Page::Details {
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
                // Where it sits when not expanded or moved aside, so it starts there next time.
                let at = self.main.home(rect);
                self.settings.panel_pos = Some([at.x, at.y]);
            }
        }

        if self.settings_open {
            self.settings_window(&ctx, &state);
        }
        self.identify.show(&ctx, &self.fonts);
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

/// Shows a window drawn by `add_contents` during this pass (an immediate viewport), as the
/// settings, the panel's copies on other monitors and Identify's numbers are. Returns what
/// `add_contents` does, once the window is open.
///
/// eframe (0.35 to 0.37) draws a hidden panel from outside winit's event handlers, where an
/// immediate viewport can't open its window, and panics: "the user callback was never called".
/// That's how right-clicking the desktop pet, with no panel showing, crashed Cuota. So a window
/// first opens as an empty deferred viewport, which eframe gives a window as this pass ends,
/// whatever called it, and is drawn in from the next pass. Open it hidden, to show once drawn.
pub(crate) fn show_immediate<R>(
    ctx: &egui::Context,
    id: ViewportId,
    builder: ViewportBuilder,
    mut add_contents: impl FnMut(&mut egui::Ui) -> R,
) -> Option<R> {
    if !ctx.input(|i| i.raw.viewports.contains_key(&id)) {
        ctx.show_viewport_deferred(id, builder, |_, _| {});
        ctx.request_repaint();
        return None;
    }
    Some(ctx.show_viewport_immediate(id, builder, |ui, _| add_contents(ui)))
}

/// The part of the monitor the window being drawn is on that the taskbar leaves free, in points.
fn work_area(ctx: &egui::Context) -> Option<Rect> {
    let ppp = ctx.pixels_per_point();
    let center = ctx.input(|i| i.viewport().outer_rect)?.center();
    let work = win::work_area((center.x * ppp) as i32, (center.y * ppp) as i32)?;
    let work = Rect::from_min_max(pos2(work.left as f32, work.top as f32), pos2(work.right as f32, work.bottom as f32));
    Some(work / ppp)
}

/// The Show as options: each one's mode, name and what it does.
fn show_as_options(settings: &Settings) -> [(Mode, &'static str, String); 4] {
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
        (
            Mode::Pet,
            "Desktop pet",
            "Clawd on his own, walking about your screen and doing what Claude's doing. Right-click him \
             for settings."
                .to_owned(),
        ),
    ]
}

/// A slider's value, beside it.
fn value_label(ui: &mut egui::Ui, value: String) {
    ui.add_sized([VALUE_WIDTH, 20.0], Label::new(RichText::new(value).size(14.0).color(widgets::SECONDARY)));
}

/// Asks for another frame after `wait`. egui starts every wait a frame's time early, so the
/// frame is ready on time, which turns any wait shorter than a frame into none at all: an
/// animation waiting for its next frame then drew again and again until it came due, keeping
/// a CPU core a fifth busy. That frame's time is added back here.
pub(crate) fn repaint_after(ctx: &egui::Context, wait: Duration) {
    let frame = Duration::try_from_secs_f32(ctx.input(|i| i.predicted_dt)).unwrap_or_default();
    ctx.request_repaint_after(wait + frame);
}

/// A status line under the meters, in `palette`'s colors: the last error, or "Loading..."
/// before any data.
pub(crate) fn note(state: &State, palette: &Palette) -> Option<(String, Color32)> {
    match (&state.error, state.limits.is_empty()) {
        (Some(error), _) => Some((error.clone(), palette.amber)),
        (None, true) => Some(("Loading...".into(), palette.dim)),
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
            overlay.main_panel(ui, state);
        });
        overlay.main.size
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

    /// The panel's background, and everything it and its details write, are in the colors of
    /// the theme picked, the Windows one following Windows.
    #[test]
    fn the_panel_and_its_details_draw_in_the_theme_picked() {
        let (ctx, mut overlay, state) = settings_overlay();
        overlay.main.set_expanded(true);
        let themes = [
            (Theme::Dark, true, Palette::DARK),
            (Theme::Light, false, Palette::LIGHT),
            (Theme::Windows, true, Palette::LIGHT),
            (Theme::Windows, false, Palette::DARK),
        ];
        for (theme, light_windows, palette) in themes {
            overlay.settings.theme = theme;
            overlay.light_windows = light_windows;
            // The second frame has the details' height from the first.
            let _ = ctx.run_ui(window(vec2(1600.0, 1200.0)), |ui| {
                overlay.main_panel(ui, &state);
            });
            let output = ctx.run_ui(window(vec2(1600.0, 1200.0)), |ui| {
                overlay.main_panel(ui, &state);
            });
            let background = palette.background(overlay.settings.opacity);
            let filled = |shape: &egui::Shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill == background);
            assert!(output.shapes.iter().any(|clipped| filled(&clipped.shape)), "{theme:?} has no {background:?}");
            let colors = [palette.text, palette.dim, palette.amber, palette.red, CLAUDE_ORANGE];
            let mut texts = 0;
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    texts += 1;
                    for section in &text.galley.job.sections {
                        let color = section.format.color;
                        assert!(colors.contains(&color), "{:?} is in {color:?} in {theme:?}", text.galley.text());
                    }
                }
            }
            assert!(texts > 20, "the details show too: {texts} texts");
        }
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
                    overlay.main_panel(ui, &state);
                });
                // He's the only picture in the panel, clipped to his slot plus SPILL all round.
                let slot = output.shapes.iter().find(|clipped| clipped.shape.texture_id() != egui::TextureId::default());
                let slot = slot.expect("Clawd is drawn").clip_rect;
                let middle = overlay.main.size.y / 2.0;
                assert!((slot.center().y - middle).abs() <= 1.0, "at {size} he's at {slot:?} in {:?}", overlay.main.size);
            }
            assert!(overlay.main.size.y > (animations::SIZE.y + 2.0 * MARGIN) * size, "the details run past him at {size}");
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
        ctx.memory_mut(|memory| memory.request_focus(Id::new(("Under the meters", Detail::ALL[0]))));
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
        let mut overlay = Overlay::new(&cc, Settings::default(), TimeDelta::minutes(5), true, &spark, icon);
        let state = overlay.state.lock().unwrap().clone();
        assert!(!state.limits.is_empty(), "the demo's limits fill the Limits expander");
        // Monitors of its own, not listed again while it's tested, with the panel on the first.
        overlay.screens = test_screens();
        overlay.screens_read = Some(Instant::now() + Duration::from_secs(3600));
        overlay.here = Some(monitor(1));
        // The first pass loads the fonts.
        let _ = ctx.run_ui(Default::default(), |_| {});
        (ctx, overlay, state)
    }

    /// Three monitors, each with a taskbar: the main one, one lower down on its left, and one
    /// standing on its end on its right.
    fn test_screens() -> Vec<Screen> {
        let screen = |number: usize, min: Pos2, size: Vec2| {
            let area = Rect::from_min_size(min, size);
            let work = Rect::from_min_max(area.min, area.max - vec2(0.0, 48.0));
            Screen { name: monitor(number), number, area, work, primary: number == 1, scale: 1.0 }
        };
        vec![
            screen(1, Pos2::ZERO, vec2(2560.0, 1440.0)),
            screen(2, pos2(-1920.0, 200.0), vec2(1920.0, 1080.0)),
            screen(3, pos2(2560.0, 0.0), vec2(1080.0, 1920.0)),
        ]
    }

    /// Windows' name for a monitor.
    fn monitor(number: usize) -> String {
        format!(r"\\.\DISPLAY{number}")
    }

    /// A frame with the panel's own window at `rect`, running `run`.
    fn panel_at(
        ctx: &egui::Context,
        overlay: &mut Overlay,
        rect: Rect,
        mut run: impl FnMut(&egui::Context, &mut Overlay),
    ) -> egui::FullOutput {
        let mut input = window(vec2(1600.0, 1200.0));
        let info = egui::ViewportInfo { outer_rect: Some(rect), inner_rect: Some(rect), ..Default::default() };
        input.viewports.insert(ViewportId::ROOT, info);
        ctx.run_ui(input, |ui| run(ui.ctx(), overlay))
    }

    /// Where a frame sent the window being drawn.
    fn moved_to(output: &egui::FullOutput) -> Option<Pos2> {
        let commands = &output.viewport_output.get(&ViewportId::ROOT)?.commands;
        commands.iter().find_map(|command| match command {
            ViewportCommand::OuterPosition(at) => Some(*at),
            _ => None,
        })
    }

    #[test]
    fn on_several_monitors_the_panel_stays_on_its_own_and_a_copy_goes_on_each_of_the_others() {
        let (ctx, mut overlay, _) = settings_overlay();
        overlay.main.size = vec2(300.0, 100.0);
        // On the left monitor, a quarter of the way across the room it has to move in, at the top.
        let rect = Rect::from_min_size(pos2(-1920.0 + 405.0, 200.0), vec2(300.0, 100.0));
        let arrange = |overlay: &mut Overlay, picked: &[usize]| {
            overlay.settings.panel_monitors = picked.iter().map(|&number| monitor(number)).collect();
            panel_at(&ctx, overlay, rect, |ctx, overlay| overlay.arrange(ctx));
            let copies: Vec<String> = overlay.copies.iter().filter_map(|copy| copy.monitor.clone()).collect();
            (overlay.main.monitor.clone(), overlay.main.placing, copies)
        };
        assert_eq!(arrange(&mut overlay, &[1, 2]), (Some(monitor(2)), false, vec![monitor(1)]));
        assert_eq!(overlay.here, Some(monitor(2)));
        // As far across and down the main monitor as it is on its own.
        assert_eq!(overlay.settings.panel_spots.get(&monitor(1)), Some(&[565.0, 0.0]));
        assert_eq!(arrange(&mut overlay, &[1, 2, 3]), (Some(monitor(2)), false, vec![monitor(1), monitor(3)]));
        // Its own monitor unpicked, it moves to the first of the others, and that copy goes.
        assert_eq!(arrange(&mut overlay, &[1, 3]), (Some(monitor(1)), true, vec![monitor(3)]));
        // Down to one, it moves there, on its own.
        assert_eq!(arrange(&mut overlay, &[3]), (Some(monitor(3)), true, vec![]));
        // Hidden from the tray, the copies close too.
        arrange(&mut overlay, &[1, 2]);
        overlay.hidden = true;
        assert_eq!(arrange(&mut overlay, &[1, 2]).2, Vec::<String>::new());

        // One picked that it isn't on, from before it started: it stays where it was left.
        let (ctx, mut overlay, _) = settings_overlay();
        overlay.settings.panel_monitors = [monitor(1)].into();
        panel_at(&ctx, &mut overlay, rect, |ctx, overlay| overlay.arrange(ctx));
        assert_eq!((overlay.main.monitor.clone(), overlay.copies.len()), (None, 0));
    }

    #[test]
    fn each_window_goes_to_its_spot_on_its_monitor_and_keeps_where_its_dragged() {
        let (ctx, mut overlay, _) = settings_overlay();
        overlay.settings.panel_monitors = [monitor(1), monitor(2)].into();
        overlay.settings.panel_spots.insert(monitor(1), [100.0, 50.0]);
        let mut copy = PanelWindow { size: vec2(300.0, 100.0), ..PanelWindow::copy(&monitor(1)) };
        let mut frame = |overlay: &mut Overlay, at: Pos2| {
            let rect = Rect::from_min_size(at, vec2(300.0, 100.0));
            moved_to(&panel_at(&ctx, overlay, rect, |ctx, overlay| overlay.keep_on_monitor(ctx, &mut copy)))
        };
        // Opened elsewhere, it's sent there, and once there it stays.
        assert_eq!(frame(&mut overlay, pos2(-1000.0, 400.0)), Some(pos2(100.0, 50.0)));
        assert_eq!(frame(&mut overlay, pos2(100.0, 50.0)), None);
        // Dragged somewhere else on its monitor, that's its spot from then on.
        assert_eq!(frame(&mut overlay, pos2(700.0, 300.0)), None);
        assert_eq!(overlay.settings.panel_spots.get(&monitor(1)), Some(&[700.0, 300.0]));
        assert!(!copy.placing);
    }

    #[test]
    fn picking_a_second_monitor_adds_it_to_the_one_the_panel_is_on_and_one_stays_picked() {
        let (ctx, mut overlay, state) = settings_overlay();
        overlay.open_expanders.insert((Page::General, "Monitors"), true);
        // One picked before, now unplugged.
        overlay.settings.panel_monitors = [monitor(9)].into();
        let mut time = 0.0;
        let mut click = |overlay: &mut Overlay, text: &str| {
            let mut frame = |overlay: &mut Overlay, events: Vec<egui::Event>| {
                time += 0.05;
                // Tall enough for the whole page, so nothing's scrolled out of reach.
                let input = egui::RawInput { events, time: Some(time), ..window(vec2(800.0, 1400.0)) };
                ctx.run_ui(input, |ui| {
                    overlay.settings_ui(ui, &state);
                })
            };
            let at = drawn(&frame(overlay, vec![]), text).center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(overlay, vec![egui::Event::PointerMoved(at)]);
            frame(overlay, vec![button(true)]);
            frame(overlay, vec![button(false)]);
        };
        let picked = |overlay: &Overlay| overlay.settings.panel_monitors.iter().cloned().collect::<Vec<_>>();
        click(&mut overlay, "Monitor 2");
        assert_eq!(picked(&overlay), [monitor(1), monitor(2), monitor(9)]);
        click(&mut overlay, "Monitor 1");
        assert_eq!(picked(&overlay), [monitor(2), monitor(9)]);
        // Once the panel has moved there, that one stays picked.
        overlay.here = Some(monitor(2));
        click(&mut overlay, "Monitor 2");
        assert_eq!(picked(&overlay), [monitor(2), monitor(9)]);
    }

    #[test]
    fn the_account_goes_by_its_email_and_its_menu_tells_only_what_claude_code_knows() {
        use chrono::Timelike;
        let account = claude::demo_state().account;
        assert_eq!(account_name(&account), "you@example.com");
        assert_eq!(initial(account.email.as_deref()), Some('Y'));
        assert_eq!(initial(Some("_émile@example.com")), Some('É'));
        assert_eq!(initial(Some("@")), None);
        let updated = chrono::Local::now().with_hour(9).and_then(|now| now.with_minute(5));
        assert_eq!(
            account_lines(&account, updated),
            ["Example Organization", "Role: Admin", "Billed through Stripe", "Subscribed since Aug 2025", "Usage updated 09:05"]
        );

        let plan_only = claude::Account { plan: Some("Claude Pro".into()), ..Default::default() };
        assert_eq!(account_name(&plan_only), "Claude Pro");
        assert_eq!(initial(plan_only.email.as_deref()), None);
        let nobody = claude::Account::default();
        assert_eq!(account_name(&nobody), "Not signed in");
        assert_eq!(account_lines(&nobody, None), ["Usage not fetched yet"]);
    }

    /// Only the first click on Log out: a second would sign Claude Code out.
    #[test]
    fn the_account_opens_its_menu_above_it_and_log_out_asks_to_be_sure() {
        let (ctx, mut overlay, state) = settings_overlay();
        let mut time = 0.0;
        let mut frame = |overlay: &mut Overlay, events: Vec<egui::Event>| {
            time += 0.05;
            let input = egui::RawInput { events, time: Some(time), ..window(SETTINGS_MIN) };
            ctx.run_ui(input, |ui| {
                overlay.settings_ui(ui, &state);
            })
        };
        let email = drawn(&frame(&mut overlay, vec![]), "you@example.com");
        let mut click = |overlay: &mut Overlay, at: Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(overlay, vec![egui::Event::PointerMoved(at)]);
            frame(overlay, vec![button(true)]);
            frame(overlay, vec![button(false)]);
            frame(overlay, vec![])
        };
        let menu = click(&mut overlay, email.center());
        let log_out = drawn(&menu, "Log out");
        assert!(log_out.bottom() < email.top(), "the menu opens above the account");
        drawn(&menu, "Subscribed since Aug 2025");
        drawn(&menu, "Refresh");

        let asked = click(&mut overlay, log_out.center());
        assert!(matches!(overlay.log_out, LogOut::Confirming));
        drawn(&asked, "Yes, log out");
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
            // Details is a list of everything the panel can show, which scrolls like any long
            // list, Desktop pet has that list for his card, and Usage lists tasks.
            for page in Page::ALL.into_iter().filter(|&page| ![Page::Details, Page::Pet, Page::Usage].contains(&page)) {
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
                    for title in ["Show as", "Monitors", "Limits", "Hear the sounds"] {
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

    #[test]
    fn about_shows_cuotas_name_and_version_and_its_links() {
        let (ctx, mut overlay, state) = settings_overlay();
        overlay.settings_page = Page::About;
        let output = ctx.run_ui(window(SETTINGS_MIN), |ui| {
            overlay.settings_ui(ui, &state);
        });
        let version = drawn(&output, concat!("Version ", env!("CARGO_PKG_VERSION"), " \u{B7} Free software, GPL-3.0-or-later"));
        let name = drawn(&output, "Cuota");
        assert!(name.bottom() <= version.top());
        let mut above = version.bottom();
        for link in ["View on GitHub", "Follow on Instagram", "More projects by Arshit"] {
            let at = drawn(&output, link);
            assert!(at.top() > above, "{link} is out of order");
            above = at.bottom();
        }
    }

    /// The account hangs from the foot of the sidebar, so a short window brings it up toward the
    /// pages, and an update waiting puts a line above it.
    #[test]
    fn the_account_stays_clear_of_the_pages_in_the_smallest_window() {
        let (ctx, mut overlay, state) = settings_overlay();
        let release = update::Release { version: "9.9.9".into(), page: String::new(), download: None };
        overlay.updater.set_status(update::Status::Available(release));
        let output = ctx.run_ui(window(SETTINGS_MIN), |ui| {
            overlay.settings_ui(ui, &state);
        });
        let laid_out = |text: &str| {
            let in_sidebar = output.shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) if shape.galley.text() == text && shape.pos.x < SIDEBAR_WIDTH => {
                    Some(Rect::from_min_size(shape.pos, shape.galley.size()))
                }
                _ => None,
            });
            in_sidebar.unwrap_or_else(|| panic!("the sidebar has no {text:?}"))
        };
        let last_page = Page::ALL.last().unwrap().name();
        // Sidebar entries are 36 points tall, centered on their names. The account is centered
        // on its email and plan.
        let pages_end = laid_out(last_page).center().y + 18.0;
        let account = (laid_out("you@example.com").top() + laid_out("Claude Max 5x").bottom()) / 2.0;
        let account_top = account - widgets::ACCOUNT_HEIGHT / 2.0;
        assert!(laid_out("Update to v9.9.9").bottom() <= account_top);
        let gap = laid_out("Update to v9.9.9").top() - pages_end;
        assert!(gap >= 4.0, "the update line is {gap} below {last_page}");
    }
}
