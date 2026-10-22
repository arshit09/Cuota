//! Gives Cuota.exe its icon, Clawd, from `setup/cuota.ico`: Explorer, Start and the taskbar
//! show it, and Cuota's windows take it from the exe (see `win::use_exe_icon`).

#[path = "setup/resources.rs"]
mod resources;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=setup/cuota.ico");
    println!("cargo:rerun-if-changed=setup/resources.rs");
    let resources = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("cuota.res");
    std::fs::write(&resources, resources::resources_file(&std::fs::read("setup/cuota.ico").unwrap(), None)).unwrap();
    println!("cargo:rustc-link-arg-bins={}", resources.display());
}
