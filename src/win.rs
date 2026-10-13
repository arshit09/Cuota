//! The few Win32 calls the window needs beyond winit. Coordinates here are physical pixels.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows_sys::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND, GetCursorPos,
    GetForegroundWindow, GetMessageW, HWND_BOTTOM, MSG, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, WINEVENT_OUTOFCONTEXT,
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
