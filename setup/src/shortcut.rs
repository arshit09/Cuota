//! Start menu shortcuts, made with the shell's IShellLinkW. windows-sys has no COM wrappers,
//! so the two interfaces are declared here, as far as setup calls them.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows_sys::Win32::UI::Shell::ShellLink;
use windows_sys::core::{GUID, HRESULT, PCWSTR};

const IID_ISHELLLINKW: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_IPERSISTFILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

type Unused = usize;

#[repr(C)]
struct ShellLinkVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: Unused,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_path: Unused,
    get_id_list: Unused,
    set_id_list: Unused,
    get_description: Unused,
    set_description: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_working_directory: Unused,
    set_working_directory: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_arguments: Unused,
    set_arguments: Unused,
    get_hotkey: Unused,
    set_hotkey: Unused,
    get_show_cmd: Unused,
    set_show_cmd: Unused,
    get_icon_location: Unused,
    set_icon_location: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
    set_relative_path: Unused,
    resolve: Unused,
    set_path: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
}

#[repr(C)]
struct PersistFileVtbl {
    query_interface: Unused,
    add_ref: Unused,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_class_id: Unused,
    is_dirty: Unused,
    load: Unused,
    save: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
}

/// Saves a shortcut at `at` that starts `exe` from its folder, with `icon`'s picture.
pub fn create(at: &Path, exe: &Path, icon: &Path, description: &str) -> Result<(), String> {
    if let Some(folder) = at.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    unsafe {
        let initialized = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0;
        let made = save(at, exe, icon, description);
        if initialized {
            CoUninitialize();
        }
        made.map_err(|code| format!("error {code:#010x}"))
    }
}

unsafe fn save(at: &Path, exe: &Path, icon: &Path, description: &str) -> Result<(), HRESULT> {
    let check = |code: HRESULT| if code >= 0 { Ok(()) } else { Err(code) };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let folder = exe.parent().unwrap_or(exe);
    unsafe {
        let mut link = null_mut();
        check(CoCreateInstance(&ShellLink, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut link))?;
        let calls = &**(link as *mut *const ShellLinkVtbl);
        let made = (|| {
            check((calls.set_path)(link, wide(&exe.display().to_string()).as_ptr()))?;
            check((calls.set_working_directory)(link, wide(&folder.display().to_string()).as_ptr()))?;
            check((calls.set_description)(link, wide(description).as_ptr()))?;
            check((calls.set_icon_location)(link, wide(&icon.display().to_string()).as_ptr(), 0))?;
            let mut file = null_mut();
            check((calls.query_interface)(link, &IID_IPERSISTFILE, &mut file))?;
            let file_calls = &**(file as *mut *const PersistFileVtbl);
            let saved = (file_calls.save)(file, wide(&at.display().to_string()).as_ptr(), 1);
            (file_calls.release)(file);
            check(saved)
        })();
        (calls.release)(link);
        made
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_a_shortcut_to_an_exe() {
        let folder = std::env::temp_dir().join(format!("cuota-setup-test-{}", std::process::id()));
        let at = folder.join("Start").join("Cuota.lnk");
        let exe = std::env::current_exe().unwrap();
        create(&at, &exe, &exe, "Cuota").unwrap();
        let bytes = std::fs::read(&at).unwrap();
        // A shell link starts with its header size and class id.
        assert_eq!(bytes[..4], [0x4C, 0, 0, 0]);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
