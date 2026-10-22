//! The panel's colors, dark or light as picked under Appearance in settings. The panel's copies,
//! its expanded details and the desktop pet's card and status all draw with them. The settings
//! window stays dark, like Windows 11's own Settings it copies.

use crate::settings::Theme;
use eframe::egui::Color32;

/// What the panel draws in: its background and edge, its text, the meters, and the details'
/// cards. Claude's orange and the colors of what the week went to are the same in both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The panel's background, see-through as far as the Opacity setting says.
    pub background: Color32,
    pub border: Color32,
    pub text: Color32,
    /// Labels, time left, and notes that aren't news.
    pub dim: Color32,
    /// A meter's empty part, and the base line of the charts.
    pub track: Color32,
    /// A meter's used part, until it's high enough to be [`amber`](Self::amber) or [`red`](Self::red).
    pub fill: Color32,
    pub amber: Color32,
    pub red: Color32,
    /// The expanded details' cards, and the lines in and around them.
    pub card: Color32,
    pub line: Color32,
    /// What the week went to besides the apps named, in the split bar.
    pub other: Color32,
    /// Around the desktop pet's status, so it reads over whatever is behind it.
    pub rim: Color32,
}

impl Palette {
    pub const DARK: Self = Self {
        background: Color32::from_rgb(28, 28, 30),
        border: Color32::from_rgb(56, 56, 60),
        text: Color32::from_rgb(232, 232, 232),
        dim: Color32::from_rgb(150, 150, 156),
        track: Color32::from_rgb(64, 64, 70),
        fill: Color32::from_rgb(200, 200, 204),
        amber: Color32::from_rgb(232, 163, 61),
        red: Color32::from_rgb(229, 72, 77),
        card: Color32::from_rgb(37, 37, 41),
        line: Color32::from_rgb(49, 49, 54),
        other: Color32::from_rgb(110, 110, 118),
        rim: Color32::from_black_alpha(200),
    };

    /// Windows 11's light flyouts: dark text on a pale gray, with white cards. The amber and
    /// red are deeper than the dark theme's, to stay readable as text on it.
    pub const LIGHT: Self = Self {
        background: Color32::from_rgb(243, 243, 245),
        border: Color32::from_rgb(208, 208, 214),
        text: Color32::from_rgb(28, 28, 30),
        dim: Color32::from_rgb(98, 98, 106),
        track: Color32::from_rgb(218, 218, 224),
        fill: Color32::from_rgb(92, 92, 100),
        amber: Color32::from_rgb(180, 104, 0),
        red: Color32::from_rgb(200, 38, 48),
        card: Color32::WHITE,
        line: Color32::from_rgb(224, 224, 230),
        other: Color32::from_rgb(150, 150, 158),
        // White, 200 opaque.
        rim: Color32::from_rgba_premultiplied(200, 200, 200, 200),
    };

    /// The colors for `theme`, where `windows_light` is whether Windows' apps are in light mode.
    pub fn of(theme: Theme, windows_light: bool) -> &'static Self {
        match theme {
            Theme::Dark => &Self::DARK,
            Theme::Light => &Self::LIGHT,
            Theme::Windows if windows_light => &Self::LIGHT,
            Theme::Windows => &Self::DARK,
        }
    }

    /// A meter's color at `percent` used: red from 90%, amber from 75%.
    pub fn level(&self, percent: f32) -> Color32 {
        match percent {
            p if p >= 90.0 => self.red,
            p if p >= 75.0 => self.amber,
            _ => self.fill,
        }
    }

    /// The background, as see-through as `opacity` (0 to 1) makes it.
    pub fn background(&self, opacity: f32) -> Color32 {
        let [r, g, b, _] = self.background.to_array();
        Color32::from_rgba_unmultiplied(r, g, b, (opacity * 255.0) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_theme_follows_windows_and_the_others_stay_put() {
        assert_eq!(Palette::of(Theme::Dark, true), &Palette::DARK);
        assert_eq!(Palette::of(Theme::Light, false), &Palette::LIGHT);
        assert_eq!(Palette::of(Theme::Windows, true), &Palette::LIGHT);
        assert_eq!(Palette::of(Theme::Windows, false), &Palette::DARK);
    }

    /// Relative luminance, as WCAG works out contrast.
    fn luminance(color: Color32) -> f32 {
        let linear = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Text, and the notes in amber and red, read on the panel and on the details' cards, and
    /// a meter's used part stands out from its track, in either theme.
    #[test]
    fn text_and_meters_stand_out_in_both_themes() {
        for palette in [Palette::DARK, Palette::LIGHT] {
            for under in [palette.background, palette.card] {
                assert!(contrast(palette.text, under) >= 7.0, "{palette:?}");
                for color in [palette.dim, palette.amber, palette.red] {
                    assert!(contrast(color, under) >= 3.0, "{color:?} on {under:?}");
                }
            }
            for percent in [10.0, 80.0, 95.0] {
                assert!(contrast(palette.level(percent), palette.track) >= 2.0, "{percent}% in {palette:?}");
            }
        }
    }

    #[test]
    fn the_background_is_as_see_through_as_the_opacity_setting() {
        let background = Palette::LIGHT.background(0.5);
        assert_eq!(background.a(), 127);
        assert_eq!(Palette::DARK.background(1.0), Palette::DARK.background);
    }
}
