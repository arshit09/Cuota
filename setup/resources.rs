//! Writes a compiled resource file (.res) for the linker to put in an exe, for the build
//! scripts of both Cuota and its setup.

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_MANIFEST: u16 = 24;
const ENGLISH_US: u16 = 0x0409;

/// A compiled resource file (.res), which the linker takes as is: each image of the .ico as an
/// icon, the group that lists them (id 1, which Windows shows for the exe), and the manifest.
pub fn resources_file(ico: &[u8], manifest: Option<&str>) -> Vec<u8> {
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
    if let Some(manifest) = manifest {
        entry(&mut res, RT_MANIFEST, 1, ENGLISH_US, manifest.as_bytes());
    }
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
