//! The few Win32 calls the window needs beyond winit. Coordinates here are physical pixels.

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, transmute, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows_sys::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, DefSubclassProc, SHAppBarMessage, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND, GetCursorPos,
    GetForegroundWindow, GetMessageW, HWND_BOTTOM, MSG, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WINEVENT_OUTOFCONTEXT,
    WM_NCCALCSIZE,
};

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
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
    let data = (&mut value as *mut u32).cast();
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(), data, &mut size)
    };
    status == 0 && value == 1
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
    let build = registry_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber");
    if build.and_then(|b| b.parse::<u32>().ok()).is_none_or(|build| build < 18362) {
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
