//! The few Win32 calls the window needs beyond winit. Coordinates here are physical pixels.

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, transmute, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HWND, LPARAM,
    LRESULT, POINT, RECT, STILL_ACTIVE, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow};
use windows_sys::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO,
    MonitorFromPoint, RGN_OR, SetWindowRgn,
};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_NODEREFERENCELINKS,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetActiveWindow, GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows_sys::Win32::UI::Shell::{
    ABM_GETTASKBARPOS, APPBARDATA, DefSubclassProc, SHAppBarMessage, SetWindowSubclass, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND, GA_ROOT, GetAncestor, GetClientRect,
    GetCursorPos, GWL_EXSTYLE, GetForegroundWindow, GetMessageW, GetWindowLongW, GetWindowRect, HWND_BOTTOM, MSG,
    STYLESTRUCT, SW_SHOWNORMAL, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetWindowLongW, SetWindowPos, WINEVENT_OUTOFCONTEXT, WM_DISPLAYCHANGE, WM_DWMCOMPOSITIONCHANGED,
    WM_NCCALCSIZE, WM_SIZE, WM_STYLECHANGING, WS_EX_TOOLWINDOW, WindowFromPoint,
};

/// claude.ai's page of plan usage.
const CLAUDE_USAGE_PAGE: &str = "https://claude.ai/settings/usage";
/// The Claude app from the Microsoft Store (MSIX), as the Start menu launches it: packaged apps
/// can't be started from their exe.
const CLAUDE_STORE_APP: &str = r"shell:AppsFolder\Claude_pzs8sxrjxfjjc!Claude";

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn taskbar_rect() -> Option<RECT> {
    let mut data: APPBARDATA = unsafe { zeroed() };
    data.cbSize = size_of::<APPBARDATA>() as u32;
    let found = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } != 0;
    found.then_some(data.rc)
}

/// Windows draws a light taskbar (Settings > Colors > default Windows mode).
pub fn light_taskbar() -> bool {
    theme_setting("SystemUsesLightTheme") == Some(1)
}

/// This process's popup menus are dark: `dark_menus` follows Windows' default app mode, which
/// is dark (Settings > Colors > default app mode).
pub fn menus_are_dark() -> bool {
    dark_menus_supported() && theme_setting("AppsUseLightTheme") == Some(0)
}

/// One of Windows' light and dark mode settings.
fn theme_setting(name: &str) -> Option<u32> {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide(name);
    let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
    let data = (&mut value as *mut u32).cast();
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(), data, &mut size)
    };
    (status == 0).then_some(value)
}

/// A string value from the registry.
fn registry_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let read = |data: *mut c_void, size: &mut u32| unsafe {
        RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), data, size)
    };
    let mut size = 0;
    if read(null_mut(), &mut size) != 0 {
        return None;
    }
    let mut value = vec![0u16; size as usize / 2];
    if read(value.as_mut_ptr().cast(), &mut size) != 0 {
        return None;
    }
    value.truncate(size as usize / 2);
    Some(String::from_utf16_lossy(&value).trim_end_matches('\0').to_owned())
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Cuota";

/// How Windows' Run key starts this exe.
fn startup_command() -> Option<String> {
    std::env::current_exe().ok().map(|exe| format!("\"{}\"", exe.display()))
}

/// Whether Windows starts this exe when the user signs in. An entry for a copy of Cuota that
/// has since moved doesn't count, so turning the option on again fixes it.
pub fn starts_with_windows() -> bool {
    let entry = registry_string(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
    entry.zip(startup_command()).is_some_and(|(entry, command)| entry.eq_ignore_ascii_case(&command))
}

/// Adds this exe to the programs Windows starts at sign-in, or takes it off. Returns whether
/// that worked.
pub fn set_start_with_windows(on: bool) -> bool {
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    let status = if on {
        let Some(command) = startup_command() else { return false };
        let data = wide(&command);
        let bytes = (data.len() * size_of::<u16>()) as u32;
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), bytes) }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => 0,
            status => status,
        }
    };
    let on_off = if on { "on" } else { "off" };
    if status == 0 {
        crate::log::info!("Start with Windows turned {on_off}");
    } else {
        crate::log::warn!("Couldn't turn Start with Windows {on_off}: error {status}");
    }
    status == 0
}

/// Such as "Windows 11 24H2 (build 26100)", for the log.
pub fn windows_version() -> String {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let Some(build) = registry_string(HKEY_LOCAL_MACHINE, KEY, "CurrentBuild") else { return "Windows".into() };
    // Windows 11 still calls itself Windows 10 in ProductName; its builds start at 22000.
    let name = if build.parse::<u32>().is_ok_and(|build| build >= 22_000) { "Windows 11" } else { "Windows 10" };
    match registry_string(HKEY_LOCAL_MACHINE, KEY, "DisplayVersion") {
        Some(version) => format!("{name} {version} (build {build})"),
        None => format!("{name} (build {build})"),
    }
}

/// How to start the Claude desktop app, from the handler for claude:// links that it registers:
/// `"C:\...\claude.exe" "%1"`. `None` when it isn't installed.
fn claude_app() -> Option<String> {
    let command = registry_string(HKEY_CURRENT_USER, r"Software\Classes\claude\shell\open\command", "")?;
    let exe = command.trim_start().strip_prefix('"')?.split('"').next()?;
    Some(if exe.to_lowercase().contains(r"\windowsapps\") { CLAUDE_STORE_APP.to_owned() } else { exe.to_owned() })
}

pub fn claude_app_installed() -> bool {
    claude_app().is_some()
}

/// Opens the Claude app, or claude.ai's usage page in the browser when `app` is false or the
/// app isn't installed.
pub fn open_claude(app: bool) {
    let targets = app.then(claude_app).flatten().into_iter().chain([CLAUDE_USAGE_PAGE.to_owned()]).collect();
    shell_open(targets, None);
}

/// Opens a file, program, script or shortcut as double-clicking it in Explorer would, from its
/// own folder so a script finds what's beside it.
pub fn open_target(target: &str) {
    shell_open(vec![target.to_owned()], folder_of(target));
}

/// Opens a web page in the default browser.
pub fn open_page(url: &str) {
    shell_open(vec![url.to_owned()], None);
}

/// The folder `path` is in, if it names one.
fn folder_of(path: &str) -> Option<String> {
    let folder = Path::new(path).parent()?.to_string_lossy();
    (!folder.is_empty()).then(|| folder.into_owned())
}

/// Opens the first of `targets` the shell can, in `directory` if given. Starting things can
/// take a moment, so it happens on a thread of its own.
fn shell_open(targets: Vec<String>, directory: Option<String>) {
    std::thread::spawn(move || unsafe {
        // Some links are handled by shell extensions, which need COM.
        CoInitializeEx(null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as _);
        let verb = wide("open");
        let directory = directory.map(|directory| wide(&directory));
        let directory = directory.as_ref().map_or(null(), |directory| directory.as_ptr());
        let mut failure = 0;
        for target in &targets {
            let wide_target = wide(target);
            // Values above 32 mean it opened; the others say why not.
            failure = ShellExecuteW(0, verb.as_ptr(), wide_target.as_ptr(), null(), directory, SW_SHOWNORMAL as _);
            if failure > 32 {
                return;
            }
        }
        crate::log::warn!("Couldn't open {} (error {failure})", targets.join(" or "));
    });
}

/// Opens File Explorer on the folder `file` is in, with `file` picked out.
pub fn show_in_folder(file: &Path) {
    let file = file.display().to_string();
    std::thread::spawn(move || unsafe {
        CoInitializeEx(null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as _);
        let (verb, explorer, select) = (wide("open"), wide("explorer.exe"), wide(&format!("/select,\"{file}\"")));
        if ShellExecuteW(0, verb.as_ptr(), explorer.as_ptr(), select.as_ptr(), null(), SW_SHOWNORMAL as _) <= 32 {
            crate::log::warn!("Couldn't show {file} in File Explorer");
        }
    });
}

/// Asks for a file with Windows' Open dialog, owned by the active window and starting beside
/// `near` when that's a path. The dialog blocks the thread it runs on, so it gets one of its
/// own and the windows keep drawing. `done` gets the path picked, or `None` when cancelled.
pub fn browse_file(near: &str, done: impl FnOnce(Option<String>) + Send + 'static) {
    let owner = unsafe { GetActiveWindow() };
    let start_in = folder_of(near).map(|folder| wide(&folder));
    std::thread::spawn(move || {
        let picked = unsafe {
            CoInitializeEx(null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as _);
            let title = wide("Open on double-click");
            // Names and patterns in turn, each ended by a NUL, with one more from `wide`.
            let filter = wide("All files\0*.*\0Programs and scripts\0*.exe;*.bat;*.cmd;*.ps1;*.ahk;*.lnk\0");
            let mut path = vec![0u16; 32 * 1024];
            let mut dialog: OPENFILENAMEW = zeroed();
            dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
            dialog.hwndOwner = owner;
            dialog.lpstrFilter = filter.as_ptr();
            dialog.lpstrFile = path.as_mut_ptr();
            dialog.nMaxFile = path.len() as u32;
            dialog.lpstrInitialDir = start_in.as_ref().map_or(null(), |folder| folder.as_ptr());
            dialog.lpstrTitle = title.as_ptr();
            // A shortcut is kept as one, so one to a Store app still opens it.
            dialog.Flags = OFN_EXPLORER
                | OFN_FILEMUSTEXIST
                | OFN_PATHMUSTEXIST
                | OFN_HIDEREADONLY
                | OFN_NOCHANGEDIR
                | OFN_NODEREFERENCELINKS;
            (GetOpenFileNameW(&mut dialog) != 0).then(|| {
                let len = path.iter().position(|&c| c == 0).unwrap_or(0);
                String::from_utf16_lossy(&path[..len])
            })
        };
        done(picked);
    });
}

/// Whether a process is still running. One running as administrator can't be looked into,
/// which still means it's there.
pub fn process_alive(pid: u32) -> bool {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0;
        let read = GetExitCodeProcess(process, &mut code) != 0;
        CloseHandle(process);
        read && code == STILL_ACTIVE as u32
    }
}

/// Claims "one Cuota per sign-in" until the process exits. False when another Cuota has it.
pub fn first_instance() -> bool {
    let name = wide(r"Local\Cuota");
    unsafe {
        // The handle stays open, and so the name taken, for as long as this process runs.
        let mutex = CreateMutexW(null(), 0, name.as_ptr());
        mutex == 0 || GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Lets this process's popup menus follow Windows' dark mode, as Windows' own apps do. That
/// takes uxtheme's unnamed SetPreferredAppMode (ordinal 135) and FlushMenuThemes (136), which
/// are stable from Windows 10 1903 on; older Windows keeps light menus.
pub fn dark_menus() {
    if !dark_menus_supported() {
        return;
    }
    const ALLOW_DARK: i32 = 1;
    type Proc = unsafe extern "system" fn() -> isize;
    unsafe {
        let uxtheme = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if uxtheme == 0 {
            return;
        }
        if let Some(set_preferred_app_mode) = GetProcAddress(uxtheme, 135 as *const u8) {
            transmute::<Proc, unsafe extern "system" fn(i32) -> i32>(set_preferred_app_mode)(ALLOW_DARK);
        }
        if let Some(flush_menu_themes) = GetProcAddress(uxtheme, 136 as *const u8) {
            transmute::<Proc, unsafe extern "system" fn()>(flush_menu_themes)();
        }
    }
}

/// Windows 10 1903 or later, where `dark_menus` works.
fn dark_menus_supported() -> bool {
    let build = registry_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber");
    build.and_then(|b| b.parse::<u32>().ok()).is_some_and(|build| build >= 18362)
}

pub fn cursor() -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point) };
    point
}

/// Whether a point is on some monitor, so a saved position can't strand the window on a
/// monitor that has since been unplugged.
pub fn on_screen(x: i32, y: i32) -> bool {
    unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) != 0 }
}

/// The part of the monitor nearest a point that the taskbar leaves free.
pub fn work_area(x: i32, y: i32) -> Option<RECT> {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcWork)
    }
}

/// Physical pixels per point on the primary monitor, before any window exists.
pub fn system_scale() -> f32 {
    let dpi = unsafe { GetDpiForSystem() };
    dpi as f32 / 96.0
}

/// The panel's corner radius in physical pixels (as f32 bits), which its window is cut to.
static CORNER_RADIUS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn panel_window(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        // Returning 0 leaves the proposed rectangle as is: the client area is the whole window.
        WM_NCCALCSIZE if wparam != 0 => return 0,
        // winit rewrites the extended style whenever the window shows, hides or changes level.
        WM_STYLECHANGING if wparam as i32 == GWL_EXSTYLE => unsafe {
            (*(lparam as *mut STYLESTRUCT)).styleNew |= WS_EX_TOOLWINDOW;
        },
        // The new width and height are the low and high words.
        WM_SIZE => cut_corners(window, (lparam & 0xffff) as i32, (lparam >> 16 & 0xffff) as i32),
        WM_DWMCOMPOSITIONCHANGED | WM_DISPLAYCHANGE => see_through(window),
        _ => {}
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

/// Sets up the panel's borderless window: its client area covers all of it, it's see-through,
/// it's cut to the panel's rounded corners as it resizes (see [`round_corners`]), and it's a
/// tool window.
///
/// egui asks winit for a drop shadow on every borderless window, which winit gets by moving
/// the client area down a pixel: that leaves a line along the top and cuts the bottom row (the
/// panel's bottom border) off.
///
/// Tools that arrange app windows leave tool windows alone. One that moved new windows to the
/// cursor's monitor also put the panel back to its first size, cropping it. It also keeps the
/// panel out of Alt+Tab.
pub fn set_up_panel(window: HWND) {
    unsafe {
        SetWindowSubclass(window, Some(panel_window), 1, 0);
        SetWindowLongW(window, GWL_EXSTYLE, GetWindowLongW(window, GWL_EXSTYLE) | WS_EX_TOOLWINDOW as i32);
        let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        SetWindowPos(window, 0, 0, 0, 0, 0, flags);
    }
    see_through(window);
}

/// Lets the desktop show through wherever the window's pixels are transparent. winit asks
/// Windows for this once, when it makes the window; if Windows doesn't keep it, the panel's
/// transparent pixels turn black. Asked again here once the window is drawing, and after
/// display and compositor changes.
fn see_through(window: HWND) {
    unsafe {
        // An empty blur region: see-through without blurring what's behind.
        let region = CreateRectRgn(0, 0, -1, -1);
        let behind = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: 1,
            hRgnBlur: region,
            fTransitionOnMaximized: 0,
        };
        let result = DwmEnableBlurBehindWindow(window, &behind);
        DeleteObject(region);
        if result < 0 {
            crate::log::warn!("Windows couldn't make the panel see-through (error {result:#x})");
        }
    }
}

/// Cuts the panel's window to rounded corners of `radius` physical pixels, now and whenever it
/// resizes, so the corners stay clear even if Windows stops showing the desktop through it.
/// Only acts when the radius changes, so it can be called every frame.
pub fn round_corners(window: HWND, radius: f32) {
    if CORNER_RADIUS.swap(radius.to_bits(), Ordering::Relaxed) != radius.to_bits() {
        let mut client: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(window, &mut client) };
        cut_corners(window, client.right, client.bottom);
    }
}

fn cut_corners(window: HWND, width: i32, height: i32) {
    let insets = corner_insets(f32::from_bits(CORNER_RADIUS.load(Ordering::Relaxed)));
    let rows = (insets.len() as i32).min(height / 2);
    unsafe {
        if rows == 0 {
            SetWindowRgn(window, 0, 1);
            return;
        }
        let region = CreateRectRgn(0, rows, width, height - rows);
        for (y, &inset) in (0..rows).zip(&insets) {
            let inset = inset.min(width / 2);
            for top in [y, height - 1 - y] {
                let row = CreateRectRgn(inset, top, width - inset, top + 1);
                CombineRgn(region, region, row, RGN_OR);
                DeleteObject(row);
            }
        }
        // Windows owns the region from here on.
        SetWindowRgn(window, region, 1);
    }
}

/// How many pixels each row of a rounded corner leaves out, from the outermost row in: the ones
/// wholly outside an arc of `radius` pixels, widened by half a pixel for its anti-aliased edge.
fn corner_insets(radius: f32) -> Vec<i32> {
    let reach = radius + 0.5;
    (0..radius.ceil() as i32)
        .map(|y| {
            // From the arc's centre to the row's nearest edge, and how far the arc spreads there.
            let down = (radius - (y + 1) as f32).max(0.0);
            let spread = (reach * reach - down * down).max(0.0).sqrt();
            // Pixel x is out when even its inner edge, at x + 1, lies beyond the arc.
            ((radius - spread - 1.0).ceil() as i32).max(0)
        })
        .collect()
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(key)) };
    state < 0
}

fn left_button_down() -> bool {
    key_down(VK_LBUTTON)
}

/// Either Ctrl key is held down, whichever window has the keyboard.
pub fn ctrl_down() -> bool {
    key_down(VK_CONTROL)
}

/// A mouse button is held down, wherever the pointer is.
pub fn mouse_down() -> bool {
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].into_iter().any(key_down)
}

/// Where a window is on screen.
pub fn window_rect(window: HWND) -> RECT {
    let mut rect: RECT = unsafe { zeroed() };
    unsafe { GetWindowRect(window, &mut rect) };
    rect
}

/// Physical pixels per point on the window's monitor.
pub fn window_scale(window: HWND) -> f32 {
    match unsafe { GetDpiForWindow(window) } {
        0 => system_scale(),
        dpi => dpi as f32 / 96.0,
    }
}

/// Whether `window` is what shows at a point, rather than another window over it.
pub fn shows_at(window: HWND, x: i32, y: i32) -> bool {
    unsafe {
        let shown = WindowFromPoint(POINT { x, y });
        shown != 0 && GetAncestor(shown, GA_ROOT) == window
    }
}

/// The radius the panel's window is cut to, in physical pixels.
pub fn corner_radius() -> f32 {
    f32::from_bits(CORNER_RADIUS.load(Ordering::Relaxed))
}

thread_local! {
    static BOTTOM: RefCell<Option<(HWND, Arc<AtomicBool>)>> = const { RefCell::new(None) };
}

unsafe extern "system" fn on_window_event(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    let Some((window, active)) = BOTTOM.with_borrow(Clone::clone) else { return };
    if active.load(Ordering::Relaxed) && unsafe { GetForegroundWindow() } == window {
        // Clicked or being dragged: drop back behind other windows once the mouse is released.
        while left_button_down() {
            std::thread::sleep(Duration::from_millis(50));
        }
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
        unsafe { SetWindowPos(window, HWND_BOTTOM, 0, 0, 0, 0, flags) };
    }
}

/// Desktop mode: Windows raises a window when it's clicked, so this sends it back behind the
/// others afterwards, while `active` is set. Event-driven (activation and finished drags in
/// this process), so it costs nothing while idle.
pub fn keep_at_bottom(window: HWND, active: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        BOTTOM.set(Some((window, active)));
        unsafe {
            for event in [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND] {
                SetWinEventHook(event, event, 0, Some(on_window_event), std::process::id(), 0, WINEVENT_OUTOFCONTEXT);
            }
            let mut message: MSG = zeroed();
            while GetMessageW(&mut message, 0, 0, 0) > 0 {
                DispatchMessageW(&message);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_of_a_path() {
        assert_eq!(folder_of(r"C:\Tools\notes.bat").as_deref(), Some(r"C:\Tools"));
        assert_eq!(folder_of("notes.bat"), None);
        assert_eq!(folder_of(""), None);
    }

    #[test]
    fn corners_keep_every_pixel_the_edge_touches() {
        assert!(corner_insets(0.0).is_empty());
        let insets = corner_insets(12.0);
        assert_eq!(insets.len(), 12);
        // Pixel 5 of the top row is 12.53 px from the arc's centre, past its 12.5 px reach; pixel 6 is within it.
        assert_eq!(insets[0], 6);
        assert_eq!(insets[11], 0);
        assert!(insets.windows(2).all(|pair| pair[0] >= pair[1]));
        // A fractional radius, as at a smaller panel size: its last row holds the arc's centre and keeps everything.
        assert_eq!(corner_insets(9.6).last(), Some(&0));
    }

    #[test]
    fn the_panel_window_is_cut_to_its_rounded_corners() {
        use windows_sys::Win32::Graphics::Gdi::{GetWindowRgn, PtInRegion};
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};
        unsafe {
            let class = wide("STATIC");
            let window = CreateWindowExW(0, class.as_ptr(), null(), WS_POPUP, 0, 0, 200, 80, 0, 0, 0, null());
            assert_ne!(window, 0);
            round_corners(window, 12.0);
            let region = CreateRectRgn(0, 0, 0, 0);
            GetWindowRgn(window, region);
            let inside = |x, y| PtInRegion(region, x, y) != 0;
            // Each corner loses the pixels its top row leaves out, mirrored.
            assert!(!inside(0, 0) && !inside(5, 0) && inside(6, 0));
            assert!(!inside(194, 79) && inside(193, 79));
            assert!(inside(0, 40) && inside(199, 40) && inside(100, 0) && inside(100, 79));
            DeleteObject(region);
            DestroyWindow(window);
        }
    }
}
