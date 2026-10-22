//! Where setup puts Cuota, and how Settings › Apps takes it off again. The setup program in
//! `setup/` includes this file too, so it uses nothing else from Cuota.
//!
//! Setup copies Cuota into `%LOCALAPPDATA%\Programs\Cuota`, which needs no admin rights and lets
//! Cuota update itself in place, adds a Start menu shortcut, and lists it in Settings › Apps,
//! which uninstalls it by running `Cuota.exe --uninstall`.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteKeyW, RegGetValueW,
    RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenMutexW, OpenProcess, PROCESS_TERMINATE, SYNCHRONIZATION_SYNCHRONIZE, TerminateProcess,
    WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{FOLDERID_Programs, FOLDERID_UserProgramFiles, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, PostMessageW, WM_APP};
use windows_sys::core::GUID;

/// Held by the running Cuota: one per sign-in.
pub const MUTEX: &str = r"Local\Cuota";
/// The class of the hidden window that owns Cuota's tray icons.
pub const TRAY_CLASS: &str = "CuotaTray";
/// Posted to that window, asks Cuota to quit, as its tray menu's Quit does.
pub const WM_QUIT_CUOTA: u32 = WM_APP + 4;
/// The argument Settings › Apps runs Cuota with to uninstall it.
pub const UNINSTALL: &str = "--uninstall";
pub const EXE: &str = "Cuota.exe";
/// Cuota.exe has no icon of its own, so its shortcut and Apps entry point at this one.
pub const ICON: &str = "Cuota.ico";

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "Cuota";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Cuota";

/// `%LOCALAPPDATA%\Programs\Cuota`, which needn't exist yet.
#[allow(dead_code)] // Setup's, like `register` and `point_startup_at`.
pub fn folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_UserProgramFiles).map(|programs| programs.join("Cuota"))
}

/// Cuota's shortcut in the signed-in user's Start menu.
pub fn shortcut() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs).map(|programs| programs.join("Cuota.lnk"))
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let mut path = null_mut();
        let found = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY as u32, 0, &mut path) == 0 && !path.is_null();
        let folder = found.then(|| {
            let length = (0..).take_while(|&i| *path.add(i) != 0).count();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(path, length)))
        });
        CoTaskMemFree(path.cast());
        folder
    }
}

/// Whether a Cuota is running for this sign-in.
pub fn running() -> bool {
    let name = wide(MUTEX);
    unsafe {
        let mutex = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr());
        if mutex != 0 {
            CloseHandle(mutex);
        }
        mutex != 0
    }
}

/// Asks each running Cuota to quit and gives them `wait` to go. Copies from before Cuota knew
/// that request, or that don't answer, are ended. Returns whether none is left running.
pub fn close_running(wait: Duration) -> bool {
    let class = wide(TRAY_CLASS);
    unsafe {
        let mut window = 0;
        loop {
            window = FindWindowExW(0, window, class.as_ptr(), null());
            if window == 0 {
                break;
            }
            PostMessageW(window, WM_QUIT_CUOTA, 0, 0);
        }
    }
    if gone_within(wait) {
        return true;
    }
    end_processes(EXE);
    gone_within(Duration::from_secs(3))
}

fn gone_within(wait: Duration) -> bool {
    let until = Instant::now() + wait;
    while running() {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// Ends every process named `name` but this one.
fn end_processes(name: &str) {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let length = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if exe.eq_ignore_ascii_case(name) && entry.th32ProcessID != GetCurrentProcessId() {
                let process = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZATION_SYNCHRONIZE, 0, entry.th32ProcessID);
                if process != 0 {
                    TerminateProcess(process, 1);
                    WaitForSingleObject(process, 2000);
                    CloseHandle(process);
                }
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
}

/// Lists Cuota, installed in `folder`, in Settings › Apps.
#[allow(dead_code)]
pub fn register(folder: &Path, version: &str, publisher: &str) -> Result<(), String> {
    let exe = folder.join(EXE);
    let kilobytes = std::fs::metadata(&exe).map_or(0, |m| m.len() / 1024) as u32;
    let strings = [
        ("DisplayName", "Cuota".to_owned()),
        ("DisplayVersion", version.to_owned()),
        ("Publisher", publisher.to_owned()),
        ("DisplayIcon", folder.join(ICON).display().to_string()),
        ("InstallLocation", folder.display().to_string()),
        ("UninstallString", format!("\"{}\" {UNINSTALL}", exe.display())),
    ];
    for (name, value) in strings {
        set_string(UNINSTALL_KEY, name, &value)?;
    }
    for (name, value) in [("NoModify", 1), ("NoRepair", 1), ("EstimatedSize", kilobytes)] {
        let status = unsafe {
            let (key, name) = (wide(UNINSTALL_KEY), wide(name));
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_DWORD, (&raw const value).cast(), 4)
        };
        if status != 0 {
            return Err(format!("Couldn't list Cuota in Apps: error {status}"));
        }
    }
    Ok(())
}

/// The folder Settings › Apps knows Cuota by, if it's listed there.
fn registered_folder() -> Option<PathBuf> {
    get_string(UNINSTALL_KEY, "InstallLocation").map(PathBuf::from)
}

/// After an update, shows the new version in Settings › Apps, if this is the installed copy.
pub fn note_version(version: &str) {
    if installed_copy() {
        let _ = set_string(UNINSTALL_KEY, "DisplayVersion", version);
    }
}

/// Whether this exe is the one setup installed.
fn installed_copy() -> bool {
    let here = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.to_path_buf()));
    here.zip(registered_folder()).is_some_and(|(here, folder)| same_path(&here, &folder))
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

/// If Windows starts some Cuota at sign-in, as an earlier copy's Start with Windows set up,
/// it starts `exe` instead.
#[allow(dead_code)]
pub fn point_startup_at(exe: &Path) {
    if get_string(RUN_KEY, RUN_VALUE).is_some() {
        let _ = set_string(RUN_KEY, RUN_VALUE, &format!("\"{}\"", exe.display()));
    }
}

/// Takes the installed Cuota off this PC: closes it, removes its shortcut, its sign-in entry
/// and its Apps listing, and deletes its files once this process has quit. Settings stay in
/// `%APPDATA%\Cuota`.
pub fn uninstall() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe.parent().ok_or("Cuota's folder is missing")?.to_path_buf();
    if !installed_copy() {
        return Err(format!("This Cuota, in {}, wasn't installed by setup, so there's nothing to uninstall. Delete it to remove it.", folder.display()));
    }
    close_running(Duration::from_secs(5));
    if let Some(shortcut) = shortcut() {
        let _ = std::fs::remove_file(shortcut);
    }
    let starts = format!("\"{}\"", exe.display());
    if get_string(RUN_KEY, RUN_VALUE).is_some_and(|command| command.eq_ignore_ascii_case(&starts)) {
        let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    }
    let key = wide(UNINSTALL_KEY);
    match unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()) } {
        0 | ERROR_FILE_NOT_FOUND => {}
        status => return Err(format!("Couldn't take Cuota off the Apps list: error {status}")),
    }
    // This exe can't delete itself while it runs, so a hidden cmd does once it has quit. Only
    // setup's files go, and the folder only if that empties it.
    let files = [EXE, ICON, "Cuota.exe.old", "Cuota.exe.new"].map(|name| format!("\"{}\"", folder.join(name).display()));
    let script = format!("ping -n 3 127.0.0.1 >nul & del /f /q {} & rmdir \"{}\"", files.join(" "), folder.display());
    std::process::Command::new("cmd.exe")
        .args(["/d", "/c"])
        .raw_arg(&script)
        .current_dir(std::env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Couldn't delete Cuota's files: {e}"))?;
    Ok(())
}

/// Keeps the cmd that deletes Cuota's files out of sight.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn get_string(key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let read = |data: *mut std::ffi::c_void, size: &mut u32| unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), data, size)
    };
    let mut size = 0u32;
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

fn set_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    let data = wide(value);
    let bytes = (data.len() * size_of::<u16>()) as u32;
    let status = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), REG_SZ, data.as_ptr().cast(), bytes)
    };
    if status == 0 { Ok(()) } else { Err(format!("Couldn't write {name}: error {status}")) }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_into_the_users_programs_folder() {
        let folder = folder().unwrap();
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert_eq!(folder, local.join("Programs").join("Cuota"));
        let shortcut = shortcut().unwrap();
        assert!(shortcut.ends_with(r"Start Menu\Programs\Cuota.lnk"), "{}", shortcut.display());
    }

    #[test]
    fn compares_folders_whatever_their_case() {
        assert!(same_path(Path::new(r"C:\Users\A\AppData"), Path::new(r"c:\users\a\appdata")));
        assert!(!same_path(Path::new(r"C:\Users\A"), Path::new(r"C:\Users\B")));
    }
}
