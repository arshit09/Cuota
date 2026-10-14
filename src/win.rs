//! The few Win32 calls the window needs beyond winit. Coordinates here are physical pixels.

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, transmute, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HWND, LPARAM,
    LRESULT, POINT, RECT, STILL_ACTIVE, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint,
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
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, GetAsyncKeyState, VK_LBUTTON};
use windows_sys::Win32::UI::Shell::{
    ABM_GETTASKBARPOS, APPBARDATA, DefSubclassProc, SHAppBarMessage, SetWindowSubclass, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND, GetCursorPos,
    GetForegroundWindow, GetMessageW, HWND_BOTTOM, MSG, SW_SHOWNORMAL, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WINEVENT_OUTOFCONTEXT,
    WM_NCCALCSIZE,
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
    status == 0
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
        for target in targets {
            let target = wide(&target);
            // Values above 32 mean it opened.
            if ShellExecuteW(0, verb.as_ptr(), target.as_ptr(), null(), directory, SW_SHOWNORMAL as _) > 32 {
                break;
            }
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

unsafe extern "system" fn whole_client_area(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    // Returning 0 leaves the proposed rectangle as is: the client area is the whole window.
    if message == WM_NCCALCSIZE && wparam != 0 {
        return 0;
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

/// Makes a borderless window's client area cover all of it. egui asks winit for a drop shadow
/// on every borderless window, which winit gets by moving the client area down a pixel: that
/// leaves a line along the top and cuts the bottom row (the panel's bottom border) off.
pub fn fill_window(window: HWND) {
    unsafe {
        SetWindowSubclass(window, Some(whole_client_area), 1, 0);
        let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        SetWindowPos(window, 0, 0, 0, 0, 0, flags);
    }
}

fn left_button_down() -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON)) };
    state < 0
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
}
