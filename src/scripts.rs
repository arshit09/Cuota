//! Windows' fonts for the writing systems Segoe UI lacks. Claude Code titles a conversation in
//! the language it's held in, so the panel may have to draw Gujarati, Thai or Chinese. Each
//! font is read only once some text needs it: the Chinese one alone is 20 MB.

use crate::widgets::Fonts;
use eframe::egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use eframe::egui::{self, FontData, FontFamily};
use std::ops::RangeInclusive;
use std::path::PathBuf;

/// A font for some scripts.
struct Fallback {
    /// Its file, and which font in it when that's a collection: the first one Windows has.
    files: &'static [(&'static str, u32)],
    /// The characters it's for.
    ranges: &'static [RangeInclusive<char>],
}

const FALLBACKS: &[Fallback] = &[
    // Nirmala UI: Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu, Kannada,
    // Malayalam, Sinhala, Ol Chiki and Meetei Mayek.
    Fallback {
        files: &[("Nirmala.ttf", 0), ("Nirmala.ttc", 0)],
        ranges: &[
            '\u{900}'..='\u{DFF}',
            '\u{1C50}'..='\u{1C7F}',
            '\u{1CD0}'..='\u{1CFF}',
            '\u{A830}'..='\u{A83F}',
            '\u{A8E0}'..='\u{A8FF}',
            '\u{ABC0}'..='\u{ABFF}',
        ],
    },
    // Leelawadee UI: Thai, Lao and Khmer.
    Fallback { files: &[("LeelawUI.ttf", 0)], ranges: &['\u{E00}'..='\u{EFF}', '\u{1780}'..='\u{17FF}', '\u{19E0}'..='\u{1A1F}'] },
    // Microsoft Himalaya: Tibetan.
    Fallback { files: &[("himalaya.ttf", 0)], ranges: &['\u{F00}'..='\u{FFF}'] },
    // Myanmar Text.
    Fallback { files: &[("mmrtext.ttf", 0)], ranges: &['\u{1000}'..='\u{109F}', '\u{A9E0}'..='\u{A9FF}', '\u{AA60}'..='\u{AA7F}'] },
    // Ebrima: Ethiopic, N'Ko, Tifinagh, Vai and Adlam.
    Fallback {
        files: &[("ebrima.ttf", 0)],
        ranges: &[
            '\u{7C0}'..='\u{7FF}',
            '\u{1200}'..='\u{139F}',
            '\u{2D30}'..='\u{2DDF}',
            '\u{A500}'..='\u{A63F}',
            '\u{AB00}'..='\u{AB2F}',
            '\u{1E900}'..='\u{1E95F}',
        ],
    },
    // Gadugi: Cherokee and Canadian Aboriginal syllabics.
    Fallback { files: &[("gadugi.ttf", 0)], ranges: &['\u{13A0}'..='\u{167F}', '\u{18B0}'..='\u{18FF}', '\u{AB70}'..='\u{ABBF}'] },
    // Malgun Gothic: Hangul.
    Fallback {
        files: &[("malgun.ttf", 0)],
        ranges: &['\u{1100}'..='\u{11FF}', '\u{3130}'..='\u{318F}', '\u{A960}'..='\u{A97F}', '\u{AC00}'..='\u{D7FF}'],
    },
    // Microsoft YaHei UI, which has the Japanese kana too, then Yu Gothic UI.
    Fallback {
        files: &[("msyh.ttc", 1), ("YuGothM.ttc", 1)],
        ranges: &[
            '\u{2E80}'..='\u{2FDF}',
            '\u{3000}'..='\u{312F}',
            '\u{3190}'..='\u{33FF}',
            '\u{3400}'..='\u{4DBF}',
            '\u{4E00}'..='\u{9FFF}',
            '\u{F900}'..='\u{FAFF}',
            '\u{FE30}'..='\u{FE4F}',
            '\u{FF00}'..='\u{FFEF}',
            '\u{20000}'..='\u{3FFFF}',
        ],
    },
];

/// Windows' font folder.
pub fn folder() -> PathBuf {
    PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts")
}

/// Which fallbacks have been looked for, by their place in [`FALLBACKS`].
#[derive(Default)]
pub struct Scripts {
    tried: [bool; FALLBACKS.len()],
}

impl Scripts {
    /// Adds the fallbacks `texts` need to the end of each of `fonts`' text families. egui takes
    /// them up from its next pass, so returns whether one was added.
    pub fn cover<'a>(&mut self, ctx: &egui::Context, fonts: &Fonts, texts: impl IntoIterator<Item = &'a str>) -> bool {
        let mut added = false;
        for c in texts.into_iter().flat_map(str::chars).filter(|c| !c.is_ascii()) {
            let wanted = FALLBACKS.iter().position(|fallback| fallback.ranges.iter().any(|range| range.contains(&c)));
            if let Some(i) = wanted
                && !std::mem::replace(&mut self.tried[i], true)
            {
                added |= add(ctx, fonts, &FALLBACKS[i]);
            }
        }
        added
    }
}

/// Reads `fallback`'s font, if Windows has it, as the last resort of each family.
fn add(ctx: &egui::Context, fonts: &Fonts, fallback: &Fallback) -> bool {
    let found = fallback.files.iter().find_map(|&(file, index)| Some((file, index, std::fs::read(folder().join(file)).ok()?)));
    let Some((file, index, bytes)) = found else { return false };
    let data = FontData { index, ..FontData::from_owned(bytes) };
    let families = [FontFamily::Proportional, FontFamily::Monospace, fonts.strong.clone()]
        .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest });
    ctx.add_font(FontInsert::new(file, data, families.into()));
    true
}
