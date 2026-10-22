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
    GetCurrentProcessId, OpenMutexW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, SYNCHRONIZATION_SYNCHRONIZE, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{FOLDERID_Programs, FOLDERID_UserProgramFiles, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId, PostMessageW, WM_APP};
use windows_sys::core::GUID;

/// Held by the running Cuota: one per sign-in.
pub const MUTEX: &str = r"Local\Cuota";
/// The class of the hidden window that owns Cuota's tray icons.
pub const TRAY_CLASS: &str = "CuotaTray";
/// Posted to that window, asks Cuota to quit, as its tray menu's Quit does.
pub const WM_QUIT_CUOTA: u32 = WM_APP + 4;
/// The argument Settings › Apps runs Cuota with to uninstall it.
pub const UNINSTALL: &str = "--uninstall";
/// Setup starts the installed Cuota with this, so it takes over Start with Windows from the
/// copy it replaced (see [`take_over_startup`]).
pub const INSTALLED: &str = "--installed";
pub const EXE: &str = "Cuota.exe";
/// Cuota.exe has no icon of its own, so its shortcut and Apps entry point at this one.
pub const ICON: &str = "Cuota.ico";

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "Cuota";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Cuota";

/// `%LOCALAPPDATA%\Programs\Cuota`, which needn't exist yet.
#[allow(dead_code)] // Setup's, like `register` and `remove_old_copy`.
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
    let trays = tray_windows();
    for &(window, _) in &trays {
        unsafe { PostMessageW(window, WM_QUIT_CUOTA, 0, 0) };
    }
    if gone_within(wait) {
        return true;
    }
    // By process, for copies renamed from Cuota.exe, and by name, for any without a tray.
    trays.iter().for_each(|&(_, process)| end_process(process));
    end_processes(EXE);
    gone_within(Duration::from_secs(3))
}

/// Every running Cuota's tray window, which each version has had, with its process.
fn tray_windows() -> Vec<(isize, u32)> {
    let class = wide(TRAY_CLASS);
    let mut found = Vec::new();
    unsafe {
        let mut window = 0;
        loop {
            window = FindWindowExW(0, window, class.as_ptr(), null());
            if window == 0 {
                break;
            }
            let mut process = 0;
            GetWindowThreadProcessId(window, &mut process);
            if process != GetCurrentProcessId() {
                found.push((window, process));
            }
        }
    }
    found
}

/// Where the running Cuotas were started from, whatever their files are called.
#[allow(dead_code)] // Setup's.
pub fn running_exes() -> Vec<PathBuf> {
    tray_windows().into_iter().filter_map(|(_, process)| process_exe(process)).collect()
}

fn process_exe(id: u32) -> Option<PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id);
        if process == 0 {
            return None;
        }
        let mut path = [0u16; 1024];
        let mut length = path.len() as u32;
        let read = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length) != 0;
        CloseHandle(process);
        read.then(|| PathBuf::from(OsString::from_wide(&path[..length as usize])))
    }
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

fn end_process(id: u32) {
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZATION_SYNCHRONIZE, 0, id);
        if process != 0 {
            TerminateProcess(process, 1);
            WaitForSingleObject(process, 2000);
            CloseHandle(process);
        }
    }
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
                end_process(entry.th32ProcessID);
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

pub fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

/// If Windows starts some Cuota at sign-in, as an earlier copy's Start with Windows set up,
/// it starts this one instead. Every version keeps that in the one value, so there's never
/// two. Cuota does this itself when setup starts it with [`INSTALLED`]: Windows Defender takes
/// a setup that points sign-in at the exe it just wrote for malware staying on the PC.
#[allow(dead_code)] // Cuota's.
pub fn take_over_startup() {
    let Ok(exe) = std::env::current_exe() else { return };
    let command = format!("\"{}\"", exe.display());
    if get_string(RUN_KEY, RUN_VALUE).is_some_and(|entry| !entry.eq_ignore_ascii_case(&command)) {
        let _ = set_string(RUN_KEY, RUN_VALUE, &command);
    }
}

/// The Cuota that Windows starts at sign-in, if any.
#[allow(dead_code)]
pub fn startup_exe() -> Option<PathBuf> {
    get_string(RUN_KEY, RUN_VALUE).and_then(|command| command_exe(&command))
}

/// The program a command line starts: quoted, or up to the first space.
fn command_exe(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let exe = match command.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => command.split(' ').next()?,
    };
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

/// Deletes `exe`, a Cuota run from outside setup's folder, such as the portable Cuota.exe that
/// releases had before setup, and what its updates left beside it. It waits a little for the
/// file to be let go of, as a Cuota that has just quit holds it a moment longer. Cuotas that
/// cargo built are left alone.
#[allow(dead_code)]
pub fn remove_old_copy(exe: &Path) {
    let is_exe = exe.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !is_exe || built_by_cargo(exe) {
        return;
    }
    let until = Instant::now() + Duration::from_secs(3);
    while exe.exists() && std::fs::remove_file(exe).is_err() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    for leftover in ["old", "new"] {
        let mut name = exe.as_os_str().to_owned();
        name.push(".");
        name.push(leftover);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

/// Whether `exe` is in a cargo project's `target` folder.
fn built_by_cargo(exe: &Path) -> bool {
    exe.ancestors().any(|folder| {
        folder.file_name().is_some_and(|name| name.eq_ignore_ascii_case("target"))
            && folder.parent().is_some_and(|project| project.join("Cargo.toml").is_file())
    })
}

/// Takes the installed Cuota off this PC: closes it, and removes its shortcut, its sign-in
/// entry and its Apps listing. Its files go with [`delete_files_once_closed`]. Settings stay in
/// `%APPDATA%\Cuota`.
pub fn uninstall() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe.parent().ok_or("Cuota's folder is missing")?;
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
    Ok(())
}

/// Deletes setup's files from this exe's folder, and the folder if that empties it. This exe
/// can't delete itself while it runs, so a hidden script does: it tries every second until
/// this process has quit (for up to a minute), then deletes itself.
pub fn delete_files_once_closed() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe.parent().ok_or("Cuota's folder is missing")?;
    let quoted = |name: &str| format!("\"{}\"", folder.join(name).display());
    // UTF-8 from the second line on, for user names outside the console's code page.
    let script = format!(
        "@chcp 65001 >nul\r\n\
         @for /l %%i in (1,1,60) do @(ping -n 2 127.0.0.1 >nul & del /f /q {exe} 2>nul & if not exist {exe} goto gone)\r\n\
         :gone\r\n\
         @del /f /q {others} 2>nul\r\n\
         @rmdir \"{folder}\" 2>nul\r\n\
         @del /f /q \"%~f0\"\r\n",
        exe = quoted(EXE),
        others = [ICON, "Cuota.exe.old", "Cuota.exe.new"].map(quoted).join(" "),
        folder = folder.display(),
    );
    let path = std::env::temp_dir().join(format!("cuota-uninstall-{}.cmd", std::process::id()));
    std::fs::write(&path, script).map_err(|e| format!("Couldn't delete Cuota's files: {e}"))?;
    std::process::Command::new("cmd.exe")
        .args(["/d", "/c"])
        .raw_arg(format!("\"{}\"", path.display()))
        .current_dir(std::env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Couldn't delete Cuota's files: {e}"))?;
    Ok(())
}

/// Keeps the script that deletes Cuota's files out of sight.
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
    fn reads_the_program_from_a_startup_command() {
        let exe = |command| command_exe(command).map(|path| path.display().to_string());
        assert_eq!(exe(r#""C:\Users\A B\Downloads\Cuota.exe""#).as_deref(), Some(r"C:\Users\A B\Downloads\Cuota.exe"));
        assert_eq!(exe(r#""C:\Cuota\Cuota.exe" --demo"#).as_deref(), Some(r"C:\Cuota\Cuota.exe"));
        assert_eq!(exe(r"C:\Cuota.exe --demo").as_deref(), Some(r"C:\Cuota.exe"));
        assert_eq!(exe("  "), None);
    }

    #[test]
    fn removes_an_old_copy_and_its_leftovers_but_not_cargos() {
        let folder = std::env::temp_dir().join(format!("cuota-old-copy-{}", std::process::id()));
        let built = folder.join("project").join("target").join("release");
        std::fs::create_dir_all(&built).unwrap();
        std::fs::write(folder.join("project").join("Cargo.toml"), "").unwrap();
        for name in ["Cuota (1).exe", "Cuota (1).exe.old", "Cuota (1).exe.new", "notes.txt"] {
            std::fs::write(folder.join(name), b"MZ").unwrap();
        }
        std::fs::write(built.join("cuota.exe"), b"MZ").unwrap();

        remove_old_copy(&folder.join("Cuota (1).exe"));
        remove_old_copy(&folder.join("notes.txt"));
        remove_old_copy(&built.join("cuota.exe"));
        let left: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|f| f.unwrap().file_name()).collect();
        assert_eq!(left, ["notes.txt", "project"]);
        assert!(built.join("cuota.exe").exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn compares_folders_whatever_their_case() {
        assert!(same_path(Path::new(r"C:\Users\A\AppData"), Path::new(r"c:\users\a\appdata")));
        assert!(!same_path(Path::new(r"C:\Users\A"), Path::new(r"C:\Users\B")));
    }
}
