//! Finds the Cuota.exe that setup carries, and gives setup its icon and manifest.
//!
//! Setup embeds `cuota.exe` from the same target folder and profile, so build Cuota first:
//! `cargo build --release`, then `cargo build --release -p cuota-setup` (or build-setup.bat).
//! `CUOTA_EXE` names another exe instead.

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
    let resources = out.join("setup.res");
    std::fs::write(&resources, resources_file(&std::fs::read("cuota.ico").unwrap())).unwrap();
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

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_MANIFEST: u16 = 24;
const ENGLISH_US: u16 = 0x0409;

/// A compiled resource file (.res), which the linker takes as is: each image of the .ico as an
/// icon, the group that lists them, and the manifest.
fn resources_file(ico: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap());
    let count = u16_at(4);
    let mut res = Vec::new();
    // A .res file starts with an empty entry.
    entry(&mut res, 0, 0, 0, &[]);
    let mut group = vec![0, 0, 1, 0];
    group.extend(count.to_le_bytes());
    for i in 0..count {
        let at = 6 + 16 * usize::from(i);
        let (size, offset) = (u32_at(at + 8) as usize, u32_at(at + 12) as usize);
        entry(&mut res, RT_ICON, i + 1, ENGLISH_US, &ico[offset..offset + size]);
        // The .ico's directory entry, with the icon's id in place of its offset.
        group.extend(&ico[at..at + 12]);
        group.extend((i + 1).to_le_bytes());
    }
    entry(&mut res, RT_GROUP_ICON, 1, ENGLISH_US, &group);
    entry(&mut res, RT_MANIFEST, 1, ENGLISH_US, MANIFEST.as_bytes());
    res
}

fn entry(res: &mut Vec<u8>, kind: u16, id: u16, language: u16, data: &[u8]) {
    const HEADER: u32 = 32;
    // Moveable, pure and discardable, as rc marks icons; nothing reads it today.
    let flags: u16 = if kind == 0 { 0 } else { 0x1030 };
    res.extend((data.len() as u32).to_le_bytes());
    res.extend(HEADER.to_le_bytes());
    res.extend([0xFFFF, kind, 0xFFFF, id].iter().flat_map(|n: &u16| n.to_le_bytes()));
    res.extend(0u32.to_le_bytes()); // data version
    res.extend(flags.to_le_bytes());
    res.extend(language.to_le_bytes());
    res.extend([0u8; 8]); // version, characteristics
    res.extend(data);
    res.resize(res.len().next_multiple_of(4), 0);
}
