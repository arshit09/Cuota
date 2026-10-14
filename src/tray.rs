//! Native icons in the notification area. In Taskbar mode, Clawd dances while Claude works and
//! sleeps when idle, and each limit shows its percentage as pixel digits over a bar, or as a
//! ring; otherwise Clawd stands still there as Cuota's own icon. Hovering shows details,
//! clicking opens the panel, and right-clicking opens a menu with details, Settings and Quit. Runs on
//! its own thread with a hidden window, so the icons stay live while the panel is hidden, and
//! only redraws an icon when what it shows changed.

use crate::claude::{self, Limit, Shared, State};
use crate::mascot::{self, CLAUDE_ORANGE, EYE, SNORE};
use crate::settings::{Style, View};
use crate::win::{self, wide};
use chrono::{Local, Utc};
use eframe::egui::{self, Color32};
use std::cell::RefCell;
use std::f32::consts::TAU;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowExW, GetMessageW,
    GetSystemMetrics, HICON, ICONINFO, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_MENUDROPALIGNMENT,
    SendMessageW, SetForegroundWindow, SetTimer, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

const CLASS: &str = "CuotaTray";
const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESH: u32 = WM_APP + 2;
/// Sent by a second launch of Cuota, which exits and leaves this one to open its settings.
const WM_SHOW_SETTINGS: u32 = WM_APP + 3;
const CLAWD_ID: u32 = 1;
/// Cuota's own icon, shown when the taskbar icons are off.
const APP_ID: u32 = 2;
const TIMER: usize = 1;
/// Without animation, icons are still refreshed this often to keep "resets in" tooltips current.
const TOOLTIP_REFRESH_MS: u32 = 30_000;
const RED: Color32 = Color32::from_rgb(229, 72, 77);
const AMBER: Color32 = Color32::from_rgb(232, 163, 61);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayConfig {
    pub enabled: bool,
    pub view: View,
    pub animate: bool,
}

pub enum TrayEvent {
    /// An icon was clicked: open the panel, or hide or show it.
    Click,
    /// Settings, picked from an icon's menu or asked for by launching Cuota again.
    Settings,
    /// Usage details, picked from an icon's menu.
    Details,
    /// Quit, picked from an icon's menu.
    Quit,
}

/// Handle to the tray thread.
#[derive(Clone)]
pub struct Tray {
    window: HWND,
    config: Arc<Mutex<TrayConfig>>,
}

impl Tray {
    /// Starts the tray thread. Clicks arrive on the returned receiver, with a repaint of `ctx`.
    pub fn spawn(state: Shared, ctx: egui::Context) -> (Self, Receiver<TrayEvent>) {
        let (events, received) = mpsc::channel();
        let (ready, window) = mpsc::channel();
        let config = Arc::new(Mutex::new(TrayConfig::default()));
        let thread_config = config.clone();
        std::thread::spawn(move || unsafe {
            win::dark_menus();
            let window = create_window();
            let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            ICONS.set(Some(Icons {
                window,
                state,
                config: thread_config,
                ctx,
                events,
                shown: Vec::new(),
                frame: 0,
                timer: None,
                taskbar_created,
            }));
            let _ = ready.send(window);
            let mut message: MSG = zeroed();
            while GetMessageW(&mut message, 0, 0, 0) > 0 {
                DispatchMessageW(&message);
            }
        });
        (Self { window: window.recv().unwrap_or(0), config }, received)
    }

    pub fn configure(&self, config: TrayConfig) {
        *self.config.lock().unwrap() = config;
        self.refresh();
    }

    /// Redraws the icons from the shared state; cheap, and a no-op for unchanged icons.
    pub fn refresh(&self) {
        unsafe { PostMessageW(self.window, WM_REFRESH, 0, 0) };
    }

    /// Removes the icons. Call before exiting, or they linger until hovered.
    pub fn close(&self) {
        unsafe { SendMessageW(self.window, WM_CLOSE, 0, 0) };
    }
}

struct Icons {
    window: HWND,
    state: Shared,
    config: Arc<Mutex<TrayConfig>>,
    ctx: egui::Context,
    events: Sender<TrayEvent>,
    /// Icons in the tray: id, pixels and tooltip as last sent to Windows.
    shown: Vec<(u32, Vec<u8>, String)>,
    frame: u32,
    timer: Option<u32>,
    /// Broadcast when Explorer restarts, which drops every tray icon.
    taskbar_created: u32,
}

thread_local! {
    static ICONS: RefCell<Option<Icons>> = const { RefCell::new(None) };
}

impl Icons {
    fn handle(&mut self, message: u32, lparam: LPARAM) -> bool {
        match message {
            WM_REFRESH | WM_SETTINGCHANGE => self.refresh(),
            WM_TIMER => {
                self.frame = self.frame.wrapping_add(1);
                self.refresh();
            }
            // Right-clicks open the menu in `window_proc`.
            WM_TRAY if lparam as u32 == WM_LBUTTONUP => self.send(TrayEvent::Click),
            WM_SHOW_SETTINGS => self.send(TrayEvent::Settings),
            WM_CLOSE => {
                for (id, ..) in self.shown.drain(..) {
                    notify(self.window, NIM_DELETE, id, None);
                }
                unsafe { DestroyWindow(self.window) };
            }
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            m if m == self.taskbar_created => {
                self.shown.clear();
                self.refresh();
            }
            _ => return false,
        }
        true
    }

    fn send(&self, event: TrayEvent) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }

    fn refresh(&mut self) {
        let config = self.config.lock().unwrap().clone();
        let state = self.state.lock().unwrap().clone();
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as usize;
        let animating = config.enabled && config.animate && config.view.clawd;
        let frame = if animating { self.frame } else { 0 };

        let mut wanted = Vec::new();
        if config.enabled {
            let ink = Ink::new(win::light_taskbar());
            for limit in state.limits.iter().filter(|l| config.view.shows(&l.label)) {
                let tip = limit_tip(limit, config.view.time_left);
                wanted.push((limit_id(&limit.label), draw_limit(size, limit, &config.view, ink), tip));
            }
            // Clawd first, and always at least one icon to click.
            if config.view.clawd || wanted.is_empty() {
                wanted.insert(0, (CLAWD_ID, draw_clawd(size, state.working, frame), clawd_tip(&state)));
            }
        } else {
            // Without the taskbar icons, Cuota keeps one of its own for the menu.
            wanted.push((APP_ID, draw_app(size), app_tip(&state)));
        }

        let gone: Vec<u32> = self.shown.iter().map(|s| s.0).filter(|id| !wanted.iter().any(|w| w.0 == *id)).collect();
        self.shown.retain(|s| !gone.contains(&s.0));
        for id in gone {
            notify(self.window, NIM_DELETE, id, None);
        }
        for (id, pixels, tip) in wanted {
            match self.shown.iter_mut().find(|s| s.0 == id) {
                Some(shown) if shown.1 == pixels && shown.2 == tip => {}
                Some(shown) => {
                    notify(self.window, NIM_MODIFY, id, Some((&pixels, size, &tip)));
                    *shown = (id, pixels, tip);
                }
                None => {
                    notify(self.window, NIM_ADD, id, Some((&pixels, size, &tip)));
                    self.shown.push((id, pixels, tip));
                }
            }
        }

        let interval = match (animating, state.working) {
            (true, true) => 250,
            (true, false) => 800,
            (false, _) => TOOLTIP_REFRESH_MS,
        };
        if self.timer != Some(interval) {
            self.timer = Some(interval);
            unsafe { SetTimer(self.window, TIMER, interval, None) };
        }
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // The menu runs a message loop of its own, so it opens outside the borrow below, which
    // lets the icons keep updating meanwhile.
    if message == WM_TRAY && lparam as u32 == WM_RBUTTONUP {
        if let Some(event) = pick_from_menu(window) {
            ICONS.with_borrow(|icons| {
                if let Some(icons) = icons {
                    icons.send(event);
                }
            });
        }
        return 0;
    }
    let handled = ICONS.with(|icons| match icons.try_borrow_mut() {
        Ok(mut icons) => icons.as_mut().is_some_and(|icons| icons.handle(message, lparam)),
        // Re-entered while redrawing: try again once the current refresh is done.
        Err(_) if message == WM_REFRESH => (unsafe { PostMessageW(window, WM_REFRESH, 0, 0) }) != 0,
        Err(_) => false,
    });
    if handled { 0 } else { unsafe { DefWindowProcW(window, message, wparam, lparam) } }
}

/// The icons' menu, at the cursor: Cuota's name and version, Usage details, Settings and Quit.
fn pick_from_menu(window: HWND) -> Option<TrayEvent> {
    const SETTINGS: usize = 1;
    const QUIT: usize = 2;
    const DETAILS: usize = 3;
    let title = wide(concat!("Cuota v", env!("CARGO_PKG_VERSION")));
    let (details, settings, quit) = (wide("Usage details"), wide("Settings"), wide("Quit"));
    let picked = unsafe {
        let menu = CreatePopupMenu();
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, title.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, DETAILS, details.as_ptr());
        AppendMenuW(menu, MF_STRING, SETTINGS, settings.as_ptr());
        AppendMenuW(menu, MF_STRING, QUIT, quit.as_ptr());
        // Without this, clicking elsewhere wouldn't close the menu.
        SetForegroundWindow(window);
        let side = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 { TPM_RIGHTALIGN } else { TPM_LEFTALIGN };
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | side;
        let at = win::cursor();
        let picked = TrackPopupMenu(menu, flags, at.x, at.y, 0, window, null());
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
        picked as usize
    };
    match picked {
        SETTINGS => Some(TrayEvent::Settings),
        DETAILS => Some(TrayEvent::Details),
        QUIT => Some(TrayEvent::Quit),
        _ => None,
    }
}

/// Asks the Cuota that's already running to open its settings.
pub fn open_running_settings() {
    let class = wide(CLASS);
    unsafe {
        // Lets it bring its settings window to the front.
        AllowSetForegroundWindow(ASFW_ANY);
        let mut window = 0;
        loop {
            window = FindWindowExW(0, window, class.as_ptr(), null());
            if window == 0 {
                break;
            }
            PostMessageW(window, WM_SHOW_SETTINGS, 0, 0);
        }
    }
}

/// A hidden top-level window: message-only windows miss the TaskbarCreated broadcast.
unsafe fn create_window() -> HWND {
    let class = wide(CLASS);
    unsafe {
        let instance = GetModuleHandleW(null());
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&window_class);
        CreateWindowExW(WS_EX_TOOLWINDOW, class.as_ptr(), class.as_ptr(), WS_POPUP, 0, 0, 0, 0, 0, 0, instance, null())
    }
}

/// Adds, updates or (with no `icon`) deletes tray icon `id`.
fn notify(window: HWND, message: u32, id: u32, icon: Option<(&[u8], usize, &str)>) {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = id;
    let mut handle = 0;
    if let Some((rgba, size, tip)) = icon {
        handle = hicon(rgba, size);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = handle;
        for (slot, unit) in data.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
            *slot = unit;
        }
    }
    unsafe { Shell_NotifyIconW(message, &data) };
    if handle != 0 {
        // Windows keeps its own copy.
        unsafe { DestroyIcon(handle) };
    }
}

/// An icon from straight-alpha RGBA pixels.
fn hicon(rgba: &[u8], size: usize) -> HICON {
    let side = size as i32;
    unsafe {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: side,
            biHeight: -side,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let mut bits = null_mut();
        let color = CreateDIBSection(0, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
        if color == 0 || bits.is_null() {
            return 0;
        }
        let out = std::slice::from_raw_parts_mut(bits.cast::<u8>(), size * size * 4);
        for (dst, src) in out.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        let mask = CreateBitmap(side, side, 1, 1, null());
        let icon = CreateIconIndirect(&ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color });
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// Text and track colors that read on the current taskbar.
#[derive(Clone, Copy)]
struct Ink {
    text: Color32,
    track: Color32,
}

impl Ink {
    fn new(light_taskbar: bool) -> Self {
        if light_taskbar {
            Self { text: Color32::BLACK, track: Color32::from_black_alpha(70) }
        } else {
            Self { text: Color32::WHITE, track: Color32::from_white_alpha(80) }
        }
    }

    fn level(self, percent: f32) -> Color32 {
        match percent {
            p if p >= 90.0 => RED,
            p if p >= 75.0 => AMBER,
            _ => self.text,
        }
    }
}

/// A square icon drawn on a 16-unit grid, scaled by whole pixels to the tray's icon size so
/// the pixel art stays crisp.
struct Canvas {
    size: usize,
    scale: usize,
    offset: usize,
    rgba: Vec<u8>,
}

impl Canvas {
    fn new(size: usize) -> Self {
        let scale = (size / 16).max(1);
        Self { size, scale, offset: (size - 16 * scale) / 2, rgba: vec![0; size * size * 4] }
    }

    fn pixel(&mut self, x: usize, y: usize, color: Color32) {
        if x < self.size && y < self.size {
            let at = (y * self.size + x) * 4;
            self.rgba[at..at + 4].copy_from_slice(&color.to_srgba_unmultiplied());
        }
    }

    /// Fills grid cells `x..x + w`, `y..y + h`.
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color32) {
        for gy in y.max(0)..(y + h).min(16) {
            for gx in x.max(0)..(x + w).min(16) {
                for dy in 0..self.scale {
                    for dx in 0..self.scale {
                        let px = self.offset + gx as usize * self.scale + dx;
                        let py = self.offset + gy as usize * self.scale + dy;
                        self.pixel(px, py, color);
                    }
                }
            }
        }
    }

    /// Digits from a 3x5 pixel font, each font pixel `zoom` cells big, centered on `center_x`.
    fn number(&mut self, value: u32, center_x: i32, top: i32, zoom: i32, color: Color32) {
        let digits: Vec<usize> = value.to_string().bytes().map(|b| usize::from(b - b'0')).collect();
        let width = digits.len() as i32 * 4 * zoom - zoom;
        let mut x = center_x - width / 2;
        for digit in digits {
            for (row, bits) in DIGITS[digit].iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) != 0 {
                        self.rect(x + col * zoom, top + row as i32 * zoom, zoom, zoom, color);
                    }
                }
            }
            x += 4 * zoom;
        }
    }

    /// Clawd's `sprite` with its top-left cell at `x`, `y`.
    fn clawd(&mut self, sprite: &[&str; 8], x: i32, y: i32) {
        let mut y = y;
        for (line, height) in sprite.iter().zip(CLAWD_ROWS) {
            for (col, pixel) in line.bytes().enumerate() {
                let color = match pixel {
                    b'#' => CLAUDE_ORANGE,
                    b'o' | b'-' => EYE,
                    _ => continue,
                };
                self.rect(x + col as i32, y, 1, height, color);
            }
            y += height;
        }
    }

    /// A ring filling clockwise from the top, anti-aliased by supersampling.
    fn ring(&mut self, percent: f32, fill: Color32, track: Color32) {
        const SAMPLES: usize = 4;
        let center = self.size as f32 / 2.0;
        let outer = self.size as f32 * 0.47;
        let inner = outer - self.size as f32 * 0.16;
        let filled = (percent / 100.0).clamp(0.0, 1.0) * TAU;
        for y in 0..self.size {
            for x in 0..self.size {
                let (mut lit, mut dim) = (0, 0);
                for s in 0..SAMPLES * SAMPLES {
                    let dx = x as f32 + ((s % SAMPLES) as f32 + 0.5) / SAMPLES as f32 - center;
                    let dy = y as f32 + ((s / SAMPLES) as f32 + 0.5) / SAMPLES as f32 - center;
                    let distance = dx.hypot(dy);
                    if (inner..=outer).contains(&distance) {
                        // Angle clockwise from 12 o'clock.
                        let angle = dx.atan2(-dy).rem_euclid(TAU);
                        if angle < filled { lit += 1 } else { dim += 1 }
                    }
                }
                let (color, count) = if lit >= dim { (fill, lit) } else { (track, dim) };
                if count > 0 {
                    let coverage = (lit + dim) as f32 / (SAMPLES * SAMPLES) as f32;
                    self.pixel(x, y, color.gamma_multiply(coverage));
                }
            }
        }
    }
}

const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Clawd squeezed onto the 16 grid: one pixel per column and these heights per half-row.
const CLAWD_ROWS: [i32; 8] = [2, 1, 2, 1, 1, 1, 2, 1];
const SNORE_Z: [&str; 4] = ["####", "..#.", ".#..", "####"];

fn draw_clawd(size: usize, working: bool, frame: u32) -> Vec<u8> {
    let (sprite, hop) = mascot::sprite(working, frame);
    let mut canvas = Canvas::new(size);
    canvas.clawd(sprite, 2, 4 - i32::from(hop));
    if !working {
        let top = 1 - (frame % 2) as i32;
        for (row, line) in SNORE_Z.iter().enumerate() {
            for (col, _) in line.bytes().enumerate().filter(|&(_, b)| b == b'#') {
                canvas.rect(11 + col as i32, top + row as i32, 1, 1, SNORE);
            }
        }
    }
    canvas.rgba
}

/// Cuota's own icon: Clawd standing still, centered.
fn draw_app(size: usize) -> Vec<u8> {
    let mut canvas = Canvas::new(size);
    canvas.clawd(&mascot::STAND, 2, 3);
    canvas.rgba
}

fn draw_limit(size: usize, limit: &Limit, view: &View, ink: Ink) -> Vec<u8> {
    let mut canvas = Canvas::new(size);
    let percent = limit.percent.round().clamp(0.0, 999.0) as u32;
    let level = ink.level(limit.percent);
    let filled = ((limit.percent / 100.0).clamp(0.0, 1.0) * 14.0).round() as i32;
    match view.style {
        Style::Bars => {
            if !view.percent {
                canvas.rect(1, 4, 14, 6, ink.track);
                canvas.rect(1, 4, filled, 6, level);
            } else if percent < 100 {
                canvas.number(percent, 8, 1, 2, ink.text);
            } else {
                canvas.number(percent, 8, 4, 1, ink.text);
            }
            canvas.rect(1, 13, 14, 2, ink.track);
            canvas.rect(1, 13, filled, 2, level);
        }
        Style::Pie => {
            canvas.ring(limit.percent, level, ink.track);
            if view.percent {
                canvas.number(percent, 8, 6, 1, ink.text);
            }
        }
    }
    canvas.rgba
}

/// Stable per-limit icon ids, so Windows remembers where the user put each one.
fn limit_id(label: &str) -> u32 {
    let hash = label.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    100 + hash % 1_000_000
}

fn clawd_tip(state: &State) -> String {
    let plan = state.account.plan.as_deref().unwrap_or("Claude");
    let status = if state.working { "working" } else { "idle" };
    usage_tip(format!("{plan} · Claude is {status}"), state)
}

fn app_tip(state: &State) -> String {
    let plan = state.account.plan.as_deref().unwrap_or("Claude");
    usage_tip(format!("Cuota · {plan}"), state)
}

/// `heading`, then each limit with its percentage and time left, and the last error.
fn usage_tip(heading: String, state: &State) -> String {
    let mut tip = heading;
    let now = Utc::now();
    for limit in &state.limits {
        let left = limit.resets_at.map(|t| format!(" · {}", claude::until(t, now))).unwrap_or_default();
        tip += &format!("\n{} {:.0}%{left}", limit.label, limit.percent);
    }
    if let Some(error) = &state.error {
        tip += &format!("\n{error}");
    }
    tip
}

fn limit_tip(limit: &Limit, time_left: bool) -> String {
    let mut tip = format!("{} {:.0}%", limit.label, limit.percent);
    if time_left && let Some(t) = limit.resets_at {
        let when = t.with_timezone(&Local).format("%a %H:%M");
        tip += &format!("\nResets in {} ({when})", claude::until(t, Utc::now()));
    }
    tip
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(rgba: &[u8]) -> usize {
        rgba.chunks(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn draws_icons_at_any_tray_size() {
        let limit = Limit { label: "Weekly".into(), percent: 49.0, resets_at: None, window: None, share: None };
        for size in [16, 20, 24, 32] {
            for style in [Style::Bars, Style::Pie] {
                let view = View { style, ..View::default() };
                let rgba = draw_limit(size, &limit, &view, Ink::new(false));
                assert_eq!(rgba.len(), size * size * 4);
                assert!(opaque(&rgba) > 20, "{style:?} at {size}px is nearly empty");
            }
            assert!(opaque(&draw_clawd(size, true, 1)) > 40);
            assert!(opaque(&draw_app(size)) > 40);
        }
    }

    #[test]
    fn limit_ids_are_stable_and_distinct() {
        assert_eq!(limit_id("Session"), limit_id("Session"));
        assert_ne!(limit_id("Session"), limit_id("Weekly"));
        assert_ne!(limit_id("Fable"), CLAWD_ID);
    }
}
