//! Finds the Cuota.exe that setup carries, and gives setup its icon and manifest.
//!
//! Setup embeds `cuota.exe` from the same target folder and profile, so build Cuota first:
//! `cargo build --release`, then `cargo build --release -p cuota-setup` (or build-setup.bat).
//! `CUOTA_EXE` names another exe instead.

mod resources;

use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-env-changed=CUOTA_EXE");
    let exe = match std::env::var_os("CUOTA_EXE") {
        Some(exe) => PathBuf::from(exe),
        // OUT_DIR is <target>/<profile>/build/cuota-setup-<hash>/out.
        None => out.ancestors().nth(3).unwrap().join("cuota.exe"),
    };
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let exe = if exe.is_file() {
        println!("cargo:rerun-if-changed={}", exe.display());
        exe
    } else if release {
        panic!("{} is missing: build Cuota first with `cargo build --release`", exe.display());
    } else {
        // Debug builds and tests run without one; installing then says so.
        println!("cargo:warning=No Cuota.exe at {}, so this setup can't install", exe.display());
        let empty = out.join("missing.exe");
        std::fs::write(&empty, []).unwrap();
        empty
    };
    println!("cargo:rustc-env=CUOTA_EXE={}", exe.display());

    // Cuota's version, which Settings > Apps shows.
    println!("cargo:rerun-if-changed=../Cargo.toml");
    let manifest = std::fs::read_to_string("../Cargo.toml").unwrap();
    let version = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .expect("Cuota's Cargo.toml has a version");
    println!("cargo:rustc-env=CUOTA_VERSION={version}");

    println!("cargo:rerun-if-changed=cuota.ico");
    println!("cargo:rerun-if-changed=resources.rs");
    let resources = out.join("setup.res");
    std::fs::write(&resources, resources::resources_file(&std::fs::read("cuota.ico").unwrap(), Some(MANIFEST))).unwrap();
    println!("cargo:rustc-link-arg-bins={}", resources.display());
}

/// Runs setup without asking for admin rights: it asks only if it must.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>
"#;
