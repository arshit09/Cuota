//! Native icons in the notification area. In Taskbar mode, Clawd dances while Claude works and
//! sleeps when idle, and each limit shows its percentage as pixel digits over a bar, or as a
//! ring; otherwise Clawd stands still there as Cuota's own icon. Hovering shows details,
//! clicking opens the panel (or settings, with only the desktop pet), and right-clicking opens
//! a menu with details, Settings and Quit. Runs on its own thread with a hidden window, so the
//! icons stay live while the panel is hidden, and only redraws an icon when what it shows
//! changed.

use crate::claude::{self, Limit, Shared, State};
use crate::mascot::{self, CLAUDE_ORANGE, EYE, SNORE};
use crate::settings::{Style, View};
use crate::widgets::icon;
use crate::win::{self, wide};
use chrono::{Local, Utc};
use eframe::egui::{self, Color32};
use std::cell::RefCell;
use std::f32::consts::TAU;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, COLOR_MENUTEXT, CreateBitmap,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, GdiFlush, GetSysColor,
    GetTextFaceW, HBITMAP, HDC, HFONT, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowExW, GetMenuItemCount,
    GetMessageW, GetSystemMetrics, HICON, HMENU, ICONINFO, MENUINFO, MENUITEMINFOW, MF_SEPARATOR, MF_STRING,
    MIIM_BITMAP, MIM_STYLE, MNS_CHECKORBMP, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SM_MENUDROPALIGNMENT, SendMessageW, SetForegroundWindow, SetMenuDefaultItem,
    SetMenuInfo, SetMenuItemInfoW, SetTimer, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

const CLASS: &str = crate::install::TRAY_CLASS;
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
/// How soon to try again when Windows doesn't take an icon, as when Cuota starts at sign-in
/// before the notification area is ready.
const RETRY_MS: u32 = 2_000;
const RED: Color32 = Color32::from_rgb(229, 72, 77);
const AMBER: Color32 = Color32::from_rgb(232, 163, 61);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayConfig {
    pub enabled: bool,
    pub view: View,
    pub animate: bool,
}

pub enum TrayEvent {
    /// An icon was clicked, or Cuota's name in its menu: open the panel, or hide or show it;
    /// with only the desktop pet showing, open settings.
    Click,
    /// Settings, picked from an icon's menu or asked for by launching Cuota again.
    Settings,
    /// Usage details, picked from an icon's menu.
    Details,
    /// Quit, picked from an icon's menu.
    Quit,
    /// One of Windows' settings changed, light or dark mode perhaps, which the panel can follow.
    WindowsSettings,
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
                missing: false,
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
    /// Windows didn't take an icon, which is tried again until it does.
    missing: bool,
    /// Broadcast when Explorer restarts, which drops every tray icon.
    taskbar_created: u32,
}

thread_local! {
    static ICONS: RefCell<Option<Icons>> = const { RefCell::new(None) };
}

impl Icons {
    fn handle(&mut self, message: u32, lparam: LPARAM) -> bool {
        match message {
            WM_REFRESH => self.refresh(),
            WM_SETTINGCHANGE => {
                self.refresh();
                self.send(TrayEvent::WindowsSettings);
            }
            WM_TIMER => {
                self.frame = self.frame.wrapping_add(1);
                self.refresh();
            }
            // Right-clicks open the menu in `window_proc`.
            WM_TRAY if lparam as u32 == WM_LBUTTONUP => self.send(TrayEvent::Click),
            WM_SHOW_SETTINGS => self.send(TrayEvent::Settings),
            // From setup, before it replaces this exe, or an uninstall.
            crate::install::WM_QUIT_CUOTA => self.send(TrayEvent::Quit),
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
        let mut missing = Vec::new();
        for (id, pixels, tip) in wanted {
            match self.shown.iter_mut().find(|s| s.0 == id) {
                Some(shown) if shown.1 == pixels && shown.2 == tip => {}
                Some(shown) => {
                    notify(self.window, NIM_MODIFY, id, Some((&pixels, size, &tip)));
                    *shown = (id, pixels, tip);
                }
                None if notify(self.window, NIM_ADD, id, Some((&pixels, size, &tip))) => {
                    self.shown.push((id, pixels, tip));
                }
                None => missing.push(id),
            }
        }
        // Updates go out many times a minute, so only a missing icon is worth logging, once.
        if !missing.is_empty() && !self.missing {
            crate::log::warn!("Windows didn't add tray icons {missing:?}, so Cuota keeps trying");
        } else if missing.is_empty() && self.missing {
            crate::log::info!("Windows added the tray icons it didn't before");
        }
        self.missing = !missing.is_empty();

        let interval = match (animating, state.working) {
            _ if self.missing => RETRY_MS,
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

/// The icons' menu, at the cursor: Clawd by Cuota's name and version, then Usage details,
/// Settings and Quit, each by its icon.
fn pick_from_menu(window: HWND) -> Option<TrayEvent> {
    const SETTINGS: usize = 1;
    const QUIT: usize = 2;
    const DETAILS: usize = 3;
    const OPEN: usize = 4;
    let title = wide(concat!("Cuota v", env!("CARGO_PKG_VERSION")));
    let (details, settings, quit) = (wide("Usage details"), wide("Settings"), wide("Quit"));
    // Drawn on each opening, so the icons follow the menu into dark mode and back.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as usize;
    let ink = menu_ink();
    let glyph = |glyph| draw_glyph(glyph, size, ink).map_or(0, |rgba| bitmap(&rgba, size, true));
    let icons = [bitmap(&draw_app(size), size, true), glyph(icon::DETAILS), glyph(icon::SETTINGS), glyph(icon::QUIT)];
    let picked = unsafe {
        let menu = CreatePopupMenu();
        // With no check marks to show, the icons take their place, as in Explorer's menus.
        let style =
            MENUINFO { cbSize: size_of::<MENUINFO>() as u32, fMask: MIM_STYLE, dwStyle: MNS_CHECKORBMP, ..zeroed() };
        SetMenuInfo(menu, &style);
        // The title does what clicking the icon does, so it's the menu's default, in bold. A
        // grayed-out title would gray out Clawd with it.
        append(menu, MF_STRING, OPEN, &title, icons[0]);
        SetMenuDefaultItem(menu, OPEN as u32, 0);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        append(menu, MF_STRING, DETAILS, &details, icons[1]);
        append(menu, MF_STRING, SETTINGS, &settings, icons[2]);
        append(menu, MF_STRING, QUIT, &quit, icons[3]);
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
    // The menu leaves its items' bitmaps to their owner.
    for bitmap in icons.into_iter().filter(|&b| b != 0) {
        unsafe { DeleteObject(bitmap) };
    }
    match picked {
        OPEN => Some(TrayEvent::Click),
        SETTINGS => Some(TrayEvent::Settings),
        DETAILS => Some(TrayEvent::Details),
        QUIT => Some(TrayEvent::Quit),
        _ => None,
    }
}

/// Appends an item to `menu`, with `bitmap` (if any) as its icon.
unsafe fn append(menu: HMENU, flags: u32, id: usize, text: &[u16], bitmap: HBITMAP) {
    unsafe {
        AppendMenuW(menu, flags, id, text.as_ptr());
        if bitmap != 0 {
            let info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_BITMAP,
                hbmpItem: bitmap,
                ..zeroed()
            };
            SetMenuItemInfoW(menu, (GetMenuItemCount(menu) - 1) as u32, TRUE, &info);
        }
    }
}

/// The menu's text color, for its icons. Dark menus are drawn by the theme, which the system
/// colors don't follow.
fn menu_ink() -> Color32 {
    if win::menus_are_dark() {
        return Color32::WHITE;
    }
    let [r, g, b, _] = unsafe { GetSysColor(COLOR_MENUTEXT) }.to_le_bytes();
    Color32::from_rgb(r, g, b)
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
/// Returns whether Windows did it.
fn notify(window: HWND, message: u32, id: u32, icon: Option<(&[u8], usize, &str)>) -> bool {
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
    let done = unsafe { Shell_NotifyIconW(message, &data) } != 0;
    if handle != 0 {
        // Windows keeps its own copy.
        unsafe { DestroyIcon(handle) };
    }
    done
}

/// An icon from straight-alpha RGBA pixels.
fn hicon(rgba: &[u8], size: usize) -> HICON {
    let color = bitmap(rgba, size, false);
    if color == 0 {
        return 0;
    }
    let side = size as i32;
    unsafe {
        let mask = CreateBitmap(side, side, 1, 1, null());
        let icon = CreateIconIndirect(&ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color });
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// A 32-bit bitmap from straight-alpha RGBA pixels: kept straight for icons, `premultiply`d
/// for menus, which blend bitmaps that way.
fn bitmap(rgba: &[u8], size: usize, premultiply: bool) -> HBITMAP {
    let (bitmap, bits) = dib(0, size);
    if bitmap != 0 {
        let out = unsafe { std::slice::from_raw_parts_mut(bits, size * size * 4) };
        for (dst, src) in out.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            let alpha = if premultiply { u16::from(src[3]) } else { 255 };
            let [r, g, b] = [src[0], src[1], src[2]].map(|c| (u16::from(c) * alpha / 255) as u8);
            dst.copy_from_slice(&[b, g, r, src[3]]);
        }
    }
    bitmap
}

/// A blank top-down 32-bit `size` square for `dc`, and its BGRA pixels; 0 if it couldn't be made.
fn dib(dc: HDC, size: usize) -> (HBITMAP, *mut u8) {
    let side = size as i32;
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: side,
        biHeight: -side,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { zeroed() }
    };
    let mut bits = null_mut();
    let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, 0, 0) };
    if bitmap != 0 && bits.is_null() {
        unsafe { DeleteObject(bitmap) };
        return (0, null_mut());
    }
    (bitmap, bits.cast())
}

/// `glyph` from Windows' icon font (see `widgets::icon`) in `color`, as `size`-pixel RGBA;
/// None without the font.
fn draw_glyph(glyph: char, size: usize, color: Color32) -> Option<Vec<u8>> {
    let mut text = [0u16; 2];
    let text = glyph.encode_utf16(&mut text);
    unsafe {
        let dc = CreateCompatibleDC(0);
        let (target, bits) = dib(dc, size);
        let font = icon_font(dc, size as i32);
        let drawn = match font {
            Some(font) if target != 0 => {
                let previous_target = SelectObject(dc, target);
                let previous_font = SelectObject(dc, font);
                // White on the blank (black) bitmap, so each pixel's green is the glyph's coverage.
                SetTextColor(dc, 0x00FF_FFFF);
                SetBkMode(dc, TRANSPARENT as i32);
                let mut rect = RECT { left: 0, top: 0, right: size as i32, bottom: size as i32 };
                let format = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
                DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut rect, format);
                GdiFlush();
                SelectObject(dc, previous_font);
                SelectObject(dc, previous_target);
                let [r, g, b, _] = color.to_srgba_unmultiplied();
                let pixels = std::slice::from_raw_parts(bits, size * size * 4);
                Some(pixels.chunks_exact(4).flat_map(|p| [r, g, b, p[1]]).collect())
            }
            _ => None,
        };
        if let Some(font) = font {
            DeleteObject(font);
        }
        if target != 0 {
            DeleteObject(target);
        }
        DeleteDC(dc);
        drawn
    }
}

/// Windows' icon font at `size` pixels: Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on
/// Windows 10. Checked by name, since GDI quietly stands in some other font for a missing one.
fn icon_font(dc: HDC, size: i32) -> Option<HFONT> {
    for face in ["Segoe Fluent Icons", "Segoe MDL2 Assets"] {
        let name = wide(face);
        let mut chosen = [0u16; 32];
        unsafe {
            let font = CreateFontW(
                -size,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET),
                u32::from(OUT_DEFAULT_PRECIS),
                u32::from(CLIP_DEFAULT_PRECIS),
                u32::from(ANTIALIASED_QUALITY),
                u32::from(DEFAULT_PITCH),
                name.as_ptr(),
            );
            let previous = SelectObject(dc, font);
            GetTextFaceW(dc, chosen.len() as i32, chosen.as_mut_ptr());
            SelectObject(dc, previous);
            if String::from_utf16_lossy(&chosen).trim_end_matches('\0') == face {
                return Some(font);
            }
            DeleteObject(font);
        }
    }
    None
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
            for glyph in [icon::DETAILS, icon::SETTINGS, icon::QUIT] {
                let rgba = draw_glyph(glyph, size, Color32::WHITE).expect("Windows' icon font");
                assert_eq!(rgba.len(), size * size * 4);
                assert!(opaque(&rgba) > 20, "{glyph:?} at {size}px is nearly empty");
            }
        }
    }

    #[test]
    fn limit_ids_are_stable_and_distinct() {
        assert_eq!(limit_id("Session"), limit_id("Session"));
        assert_ne!(limit_id("Session"), limit_id("Weekly"));
        assert_ne!(limit_id("Fable"), CLAWD_ID);
    }
}
