//! The install itself, on a thread of its own while the window shows how it's going.

use crate::{install, shortcut};
use eframe::egui::Context;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// The Cuota this setup installs, built before it (see build.rs).
/// Statics, not consts, which would put a copy in each place they're used.
pub static CUOTA: &[u8] = include_bytes!(env!("CUOTA_EXE"));
pub static ICON: &[u8] = include_bytes!("../cuota.ico");
pub const VERSION: &str = env!("CUOTA_VERSION");
const PUBLISHER: &str = "Arshit Vaghasiya";
/// Setup runs a copy of itself as admin with this and a folder, when copying there needs it.
pub const COPY_FILES: &str = "--copy-files";
/// Each step shows at least this long, so its status can be read.
const STEP: Duration = Duration::from_millis(500);

#[derive(Clone, PartialEq)]
pub enum Stage {
    Ready,
    Working(&'static str),
    /// Waiting for the user to let setup run as admin.
    Asking,
    Done,
    Failed(String),
}

/// Installs Cuota, moving `stage` along as it goes. `window` is setup's, for Windows to put
/// its permission prompt in front of.
pub fn start(stage: Arc<Mutex<Stage>>, ctx: Context, window: isize) {
    std::thread::spawn(move || {
        let mut shown = Instant::now() - STEP;
        let mut show = |next: Stage| {
            std::thread::sleep(STEP.saturating_sub(shown.elapsed()));
            *stage.lock().unwrap() = next;
            ctx.request_repaint();
            shown = Instant::now();
        };
        let result = install(&mut show, window);
        show(match result {
            Ok(()) => Stage::Done,
            Err(e) => Stage::Failed(e),
        });
    });
}

fn install(show: &mut dyn FnMut(Stage), window: isize) -> Result<(), String> {
    if CUOTA.is_empty() {
        return Err("This setup was built without Cuota.".into());
    }
    let folder = install::folder().ok_or("Couldn't find your Programs folder.")?;
    if install::running() {
        show(Stage::Working("Closing Cuota…"));
        if !install::close_running(Duration::from_secs(5)) {
            return Err("Cuota is still running. Quit it from its tray icon, then try again.".into());
        }
    }
    show(Stage::Working("Unpacking Cuota…"));
    match put_files(&folder, CUOTA) {
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            show(Stage::Asking);
            put_files_as_admin(&folder, window)?;
        }
        copied => copied.map_err(|e| format!("Couldn't copy Cuota: {e}"))?,
    }

    show(Stage::Working("Adding Cuota to Start…"));
    let exe = folder.join(install::EXE);
    let link = install::shortcut().ok_or("Couldn't find your Start menu.")?;
    shortcut::create(&link, &exe, &folder.join(install::ICON), "Your Claude usage limits, with Clawd")
        .map_err(|e| format!("Couldn't add Cuota to Start: {e}"))?;
    install::register(&folder, VERSION, PUBLISHER)?;
    install::point_startup_at(&exe);

    show(Stage::Working("Starting Cuota…"));
    Command::new(&exe).current_dir(&folder).spawn().map_err(|e| format!("Couldn't start Cuota: {e}"))?;
    Ok(())
}

/// Writes `cuota` as Cuota.exe into `folder`, with its icon. A Cuota.exe there already moves
/// aside first, as Cuota's updater does, in case something still has it open.
pub fn put_files(folder: &Path, cuota: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let exe = folder.join(install::EXE);
    let (new, old) = (folder.join("Cuota.exe.new"), folder.join("Cuota.exe.old"));
    std::fs::write(&new, cuota)?;
    if exe.exists() {
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old)?;
    }
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(e);
    }
    // Still open, it stays until the next install or update.
    let _ = std::fs::remove_file(&old);
    std::fs::write(folder.join(install::ICON), ICON)
}

/// Runs this setup again as admin to copy the files, and waits for it. Windows asks the user
/// first; the rest of the install stays with them, not with whoever the admin is.
fn put_files_as_admin(folder: &Path, window: isize) -> Result<(), String> {
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let (verb, file) = (wide("runas"), wide(&me.display().to_string()));
    let parameters = wide(&format!("{COPY_FILES} \"{}\"", folder.display()));
    unsafe {
        let mut run: SHELLEXECUTEINFOW = std::mem::zeroed();
        run.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        run.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        run.hwnd = window;
        run.lpVerb = verb.as_ptr();
        run.lpFile = file.as_ptr();
        run.lpParameters = parameters.as_ptr();
        run.nShow = SW_HIDE;
        if ShellExecuteExW(&mut run) == 0 {
            return Err(if GetLastError() == ERROR_CANCELLED {
                "Cuota needs your permission to install. Try again, and choose Yes.".into()
            } else {
                "Couldn't ask Windows for permission to install.".into()
            });
        }
        WaitForSingleObject(run.hProcess, INFINITE);
        let mut code = 1;
        GetExitCodeProcess(run.hProcess, &mut code);
        CloseHandle(run.hProcess);
        if code == 0 { Ok(()) } else { Err(format!("Couldn't copy Cuota into {}, even as admin.", folder.display())) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puts_cuota_in_place_over_an_older_copy() {
        let folder = std::env::temp_dir().join(format!("cuota-setup-files-{}", std::process::id()));
        put_files(&folder, b"old").unwrap();
        put_files(&folder, b"new").unwrap();
        assert_eq!(std::fs::read(folder.join(install::EXE)).unwrap(), b"new");
        assert_eq!(std::fs::read(folder.join(install::ICON)).unwrap(), ICON);
        let mut names: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|f| f.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["Cuota.exe", "Cuota.ico"]);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
