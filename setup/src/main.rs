//! Cuota's setup: a small window where Clawd waits by an Install button, then works while each
//! step of the install is written under him.

// Release builds are GUI-only; debug builds keep a console for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clip;
#[allow(dead_code)]
#[path = "../../src/install.rs"]
mod install;
mod job;
mod shortcut;

use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontFamily, FontId, IconData, Key, PointerButton, Rect, Sense, Stroke,
    StrokeKind, ViewportBuilder, ViewportCommand, pos2, text::LayoutJob, vec2,
};
use job::Stage;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::Path;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Graphics::Dwm::{
    DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};

/// As Cuota's settings window.
const WINDOW: Color32 = Color32::from_rgb(32, 32, 32);
/// Windows 11's edge around dark windows.
const EDGE: Color32 = Color32::from_rgb(58, 58, 58);
const TEXT: Color32 = Color32::from_rgb(208, 208, 208);
/// Windows' dark theme draws black on its accent color.
const ON_ACCENT: Color32 = Color32::BLACK;
/// Windows 11's close button, hovered.
const CLOSE_RED: Color32 = Color32::from_rgb(196, 43, 28);
const SIZE: [f32; 2] = [300.0, 300.0];
/// As Windows 11 rounds its windows.
const CORNER: f32 = 8.0;
/// Minimize and close, in the top-right corner.
const CONTROL: [f32; 2] = [32.0, 28.0];
const MINIMIZE_GLYPH: char = '\u{E921}';
const CLOSE_GLYPH: char = '\u{E8BB}';
/// How tall Clawd's clip shows, in points; its width follows the GIF's.
const CLAWD_HEIGHT: f32 = 160.0;
const BUTTON: [f32; 2] = [140.0, 32.0];

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Setup running itself as admin, for the files alone.
    if let [flag, folder] = args.as_slice()
        && flag == job::COPY_FILES
    {
        std::process::exit(if job::put_files(Path::new(folder), job::CUOTA).is_ok() { 0 } else { 1 });
    }
    let icon = window_icon(job::ICON, 64);
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Cuota Setup")
            .with_inner_size(SIZE)
            // A rounded popup that draws its own minimize and close buttons.
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_icon(Arc::new(icon)),
        centered: true,
        ..Default::default()
    };
    eframe::run_native("Cuota Setup", options, Box::new(|cc| Ok(Box::new(Setup::new(cc)))))
}

struct Setup {
    clawd: clip::Player,
    stage: Arc<Mutex<Stage>>,
    window: isize,
    fonts: Fonts,
}

impl Setup {
    fn new(cc: &eframe::CreationContext) -> Self {
        let fonts = set_up_fonts(&cc.egui_ctx);
        let window = match cc.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
            _ => 0,
        };
        see_through(window);
        Self {
            clawd: clip::Player::new(&cc.egui_ctx, clip::GREETING),
            stage: Arc::new(Mutex::new(Stage::Ready)),
            window,
            fonts,
        }
    }

    /// Minimize and close, drawn as Windows 11 draws them. Close waits while installing.
    fn window_controls(&self, ui: &mut egui::Ui, area: Rect, working: bool) {
        let ctx = ui.ctx().clone();
        let top_right = area.right_top() + vec2(-6.0, 6.0);
        let close = Rect::from_min_size(top_right - vec2(CONTROL[0], 0.0), CONTROL.into());
        let minimize = close.translate(vec2(-CONTROL[0] - 2.0, 0.0));
        let Some(icons) = &self.fonts.icons else { return };
        for (rect, glyph, enabled) in [(minimize, MINIMIZE_GLYPH, true), (close, CLOSE_GLYPH, !working)] {
            let response = ui.interact(rect, ui.id().with(glyph), Sense::click());
            let hovered = enabled && response.hovered();
            let (fill, ink) = match (glyph == CLOSE_GLYPH, hovered) {
                (true, true) => (CLOSE_RED, Color32::WHITE),
                (false, true) => (Color32::from_white_alpha(14), TEXT),
                (_, false) => (Color32::TRANSPARENT, if enabled { TEXT } else { Color32::from_gray(90) }),
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, fill);
            painter.text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::new(10.0, icons.clone()), ink);
            if enabled && response.clicked() {
                let command = if glyph == CLOSE_GLYPH { ViewportCommand::Close } else { ViewportCommand::Minimized(true) };
                ctx.send_viewport_cmd(command);
            }
        }
    }
}

impl eframe::App for Setup {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let stage = self.stage.lock().unwrap().clone();
        let working = matches!(stage, Stage::Working(_) | Stage::Asking);
        // Stopping halfway would leave half an install.
        if working && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }

        let area = ui.max_rect();
        // Anywhere not a button moves the window.
        let drag = ui.interact(area, ui.id().with("drag"), Sense::click_and_drag());
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let painter = ui.painter();
        // The desktop shows past the rounded corners. A pixel-wide edge, inside the window.
        let pixel = 1.0 / ctx.pixels_per_point();
        painter.rect(area, CORNER, WINDOW, Stroke::new(pixel, EDGE), StrokeKind::Inside);

        self.clawd.play(match stage {
            Stage::Ready => clip::GREETING,
            Stage::Working(_) => clip::WORKING,
            Stage::Asking => clip::WAITING,
            Stage::Done => clip::DONE,
            Stage::Failed(_) => clip::ERROR,
        });
        let (texture, [w, h], next) = self.clawd.frame();
        let size = vec2(CLAWD_HEIGHT * w as f32 / h as f32, CLAWD_HEIGHT);
        let clawd = Rect::from_center_size(pos2(area.center().x, area.top() + 20.0 + size.y / 2.0), size);
        painter.image(texture.id(), clawd, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ctx.request_repaint_after(next);

        let status = match &stage {
            Stage::Ready => format!("Ready to install Cuota {}", job::VERSION),
            Stage::Working(step) => (*step).to_owned(),
            Stage::Asking => "Waiting for your permission…".to_owned(),
            Stage::Done => "Cuota is installed. Find it in Start anytime.".to_owned(),
            Stage::Failed(why) => why.clone(),
        };
        let mut job = LayoutJob::simple(status, FontId::proportional(14.0), TEXT, area.width() - 48.0);
        job.halign = Align::Center;
        let galley = painter.layout_job(job);
        painter.galley(pos2(area.center().x, clawd.bottom() + 6.0), galley, TEXT);
        self.window_controls(ui, area, working);

        let label = match stage {
            Stage::Ready => "Install",
            Stage::Done => "Close",
            Stage::Failed(_) => "Try again",
            Stage::Working(_) | Stage::Asking => return,
        };
        let button = Rect::from_center_size(pos2(area.center().x, area.bottom() - 24.0 - BUTTON[1] / 2.0), BUTTON.into());
        let pressed = accent_button(ui, button, label, FontId::new(14.0, self.fonts.strong.clone()));
        if pressed || ctx.input(|i| i.key_pressed(Key::Enter)) {
            if stage == Stage::Done {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            } else {
                job::start(self.stage.clone(), ctx.clone(), self.window);
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Clear past the rounded corners.
        [0.0; 4]
    }
}

/// A button in Claude's orange, drawn as Cuota's settings draw theirs. Returns whether it was
/// clicked.
fn accent_button(ui: &mut egui::Ui, rect: Rect, label: &str, font: FontId) -> bool {
    const ACCENT: Color32 = Color32::from_rgb(217, 119, 87);
    let response = ui.interact(rect, ui.id().with(label), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        ACCENT.gamma_multiply(0.8)
    } else if response.hovered() {
        ACCENT.gamma_multiply(0.9)
    } else {
        ACCENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, fill);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, label, font, ON_ACCENT);
    response.clicked()
}

/// The `size` image of `ico`, which holds the 32-bit bitmaps that Cuota's tests draw.
fn window_icon(ico: &[u8], size: u8) -> IconData {
    let u32_at = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap());
    let count = usize::from(ico[4]);
    let entry = (0..count).map(|i| 6 + 16 * i).find(|&at| ico[at] == size).expect("the icon has the size");
    // Past the bitmap's header, rows of BGRA from the bottom up.
    let pixels = &ico[u32_at(entry + 12) as usize + 40..][..usize::from(size) * usize::from(size) * 4];
    let rows = pixels.chunks_exact(usize::from(size) * 4).rev();
    let rgba = rows.flat_map(|row| row.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]])).collect();
    IconData { rgba, width: size.into(), height: size.into() }
}

struct Fonts {
    /// Semibold, for the button.
    strong: FontFamily,
    /// Windows' icon font, for minimize and close.
    icons: Option<FontFamily>,
}

/// Segoe UI from Windows (egui's own fonts are left out of the build), its semibold, and the
/// icon font: Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10.
fn set_up_fonts(ctx: &egui::Context) -> Fonts {
    let windows = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let load = |names: &[&str]| names.iter().find_map(|name| std::fs::read(Path::new(&windows).join("Fonts").join(name)).ok());
    let mut fonts = egui::FontDefinitions::empty();
    let regular = load(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]).expect("Windows has a UI font");
    fonts.font_data.insert("regular".into(), Arc::new(FontData::from_owned(regular)));
    let mut strong = vec!["regular".to_owned()];
    if let Some(bytes) = load(&["seguisb.ttf", "arialbd.ttf"]) {
        fonts.font_data.insert("semibold".into(), Arc::new(FontData::from_owned(bytes)));
        strong.insert(0, "semibold".into());
    }
    let family = FontFamily::Name("strong".into());
    fonts.families.insert(FontFamily::Proportional, vec!["regular".into()]);
    fonts.families.insert(FontFamily::Monospace, vec!["regular".into()]);
    fonts.families.insert(family.clone(), strong);
    let icons = load(&["SegoeIcons.ttf", "segmdl2.ttf"]).map(|bytes| {
        fonts.font_data.insert("icons".into(), Arc::new(FontData::from_owned(bytes)));
        let family = FontFamily::Name("icons".into());
        fonts.families.insert(family.clone(), vec!["icons".into()]);
        family
    });
    ctx.set_fonts(fonts);
    Fonts { strong: family, icons }
}

/// Lets the desktop show through the window's clear corners, as Cuota's panel does: an empty
/// blur region is see-through without blurring. Windows 11 also rounds the window itself.
fn see_through(window: isize) {
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let behind = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: 1,
            hRgnBlur: region,
            fTransitionOnMaximized: 0,
        };
        DwmEnableBlurBehindWindow(window, &behind);
        DeleteObject(region);
        let round = DWMWCP_ROUND;
        DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const round).cast(), 4);
    }
}
