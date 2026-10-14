//! User preferences, saved as JSON in `%APPDATA%\Cuota\settings.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Where Cuota shows. Several can be on at once, but Always on top and Desktop place the same
/// panel, so at most one of those two is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Mode {
    /// A floating panel above all windows. Saved as "Popup" before it was renamed.
    #[serde(alias = "Popup")]
    AlwaysOnTop,
    /// A panel behind all windows, like a desktop widget.
    Desktop,
    /// Icons in the taskbar's notification area. Clicking one opens the panel, or hides and
    /// shows it when Always on top or Desktop keeps it on screen.
    Taskbar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Bars,
    Pie,
}

/// What double-clicking the panel does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoubleClick {
    /// Expands the panel with usage details.
    #[default]
    Details,
    /// Opens the usage page on claude.ai.
    Website,
    /// Opens the Claude desktop app.
    App,
    /// Opens the file, program or script at [`Settings::custom_target`].
    Custom,
}

/// What one surface (the panel, or the taskbar icons) shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub clawd: bool,
    /// A word under Clawd for what he's up to, like "Percolating…". The panel only.
    pub status: bool,
    pub percent: bool,
    pub time_left: bool,
    pub style: Style,
    /// Labels of the limits the user hid, e.g. "Fable".
    pub hidden: BTreeSet<String>,
}

impl Default for View {
    fn default() -> Self {
        Self { clawd: true, status: true, percent: true, time_left: true, style: Style::Bars, hidden: BTreeSet::new() }
    }
}

impl View {
    pub fn shows(&self, label: &str) -> bool {
        !self.hidden.contains(label)
    }

    pub fn set_shown(&mut self, label: &str, shown: bool) {
        if shown {
            self.hidden.remove(label);
        } else {
            self.hidden.insert(label.to_owned());
        }
    }
}

/// The steps of the panel's Size slider, as multiples of its original size: small, medium
/// (the original) and large, and the steps between.
pub const SIZES: [f32; 10] = [0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show: BTreeSet<Mode>,
    pub animate: bool,
    pub opacity: f32,
    /// How big the panel is: one of [`SIZES`].
    pub size: f32,
    pub panel: View,
    pub taskbar: View,
    pub double_click: DoubleClick,
    /// What a Custom double-click opens: the path of a file, program, script or shortcut.
    /// Empty until one is picked.
    pub custom_target: String,
    /// Top-left of the panel in Always on top and Desktop modes, in points.
    pub panel_pos: Option<[f32; 2]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show: BTreeSet::from([Mode::AlwaysOnTop]),
            animate: true,
            opacity: 0.96,
            size: 1.0,
            panel: View::default(),
            taskbar: View::default(),
            double_click: DoubleClick::default(),
            custom_target: String::new(),
            panel_pos: None,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Cuota").join("settings.json"))
    }

    pub fn load() -> Self {
        let mut settings: Self = Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        // Showing nowhere would leave nothing to right-click for settings.
        if settings.show.is_empty() {
            settings.show = Self::default().show;
        }
        // A size edited into the file by hand snaps to the nearest step.
        settings.size = SIZES[settings.size_step()];
        settings
    }

    /// The Size slider's step nearest to `size`.
    pub fn size_step(&self) -> usize {
        let off = |step: &usize| (SIZES[*step] - self.size).abs();
        (0..SIZES.len()).min_by(|a, b| off(a).total_cmp(&off(b))).unwrap_or_default()
    }

    pub fn shows_as(&self, mode: Mode) -> bool {
        self.show.contains(&mode)
    }

    /// Turning on Always on top turns off Desktop, and the other way round.
    pub fn set_shown_as(&mut self, mode: Mode, shown: bool) {
        if !shown {
            self.show.remove(&mode);
            return;
        }
        match mode {
            Mode::AlwaysOnTop => self.show.remove(&Mode::Desktop),
            Mode::Desktop => self.show.remove(&Mode::AlwaysOnTop),
            Mode::Taskbar => false,
        };
        self.show.insert(mode);
    }

    /// Always on top or Desktop, whichever keeps the panel on screen. `None` when the panel only opens
    /// from the tray icons.
    pub fn pinned(&self) -> Option<Mode> {
        [Mode::AlwaysOnTop, Mode::Desktop].into_iter().find(|&mode| self.shows_as(mode))
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taskbar_goes_with_either_panel_but_always_on_top_and_desktop_exclude_each_other() {
        let mut settings = Settings::default();
        settings.set_shown_as(Mode::Taskbar, true);
        assert_eq!(settings.pinned(), Some(Mode::AlwaysOnTop));
        settings.set_shown_as(Mode::Desktop, true);
        assert_eq!(settings.show, BTreeSet::from([Mode::Desktop, Mode::Taskbar]));
        assert_eq!(settings.pinned(), Some(Mode::Desktop));
        settings.set_shown_as(Mode::Desktop, false);
        assert_eq!(settings.pinned(), None);
    }

    #[test]
    fn saves_the_modes_as_a_list() {
        let settings = Settings { show: BTreeSet::from([Mode::AlwaysOnTop, Mode::Taskbar]), ..Settings::default() };
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains(r#""show":["AlwaysOnTop","Taskbar"]"#), "{text}");
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn loads_the_old_popup_name_as_always_on_top() {
        let settings: Settings = serde_json::from_str(r#"{"show":["Popup","Taskbar"]}"#).unwrap();
        assert_eq!(settings.show, BTreeSet::from([Mode::AlwaysOnTop, Mode::Taskbar]));
    }

    #[test]
    fn clawds_status_shows_unless_turned_off_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"panel":{"clawd":true,"percent":false}}"#).unwrap();
        assert!(older.panel.status && !older.panel.percent);
        let off = Settings { panel: View { status: false, ..View::default() }, ..Settings::default() };
        let text = serde_json::to_string(&off).unwrap();
        assert!(!serde_json::from_str::<Settings>(&text).unwrap().panel.status);
    }

    #[test]
    fn a_custom_double_click_keeps_what_it_opens_and_older_settings_still_load() {
        let settings = Settings {
            double_click: DoubleClick::Custom,
            custom_target: r"C:\Tools\notes.bat".to_owned(),
            ..Settings::default()
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
        let older: Settings = serde_json::from_str(r#"{"double_click":"App"}"#).unwrap();
        assert_eq!(older.double_click, DoubleClick::App);
        assert_eq!(older.custom_target, "");
    }

    #[test]
    fn the_panel_starts_at_its_original_size_in_the_middle_of_ten_steps() {
        assert_eq!(SIZES.len(), 10);
        assert!(SIZES.windows(2).all(|pair| pair[0] < pair[1]));
        let older: Settings = serde_json::from_str(r#"{"opacity":0.8}"#).unwrap();
        assert_eq!(older.size, 1.0);
        assert_eq!(SIZES[older.size_step()], 1.0);
        assert!((3..=6).contains(&older.size_step()), "medium sits mid-slider");
    }

    #[test]
    fn a_size_between_or_past_the_steps_snaps_to_the_nearest() {
        let step = |size| SIZES[Settings { size, ..Settings::default() }.size_step()];
        assert_eq!(step(1.04), 1.0);
        assert_eq!(step(1.16), 1.2);
        assert_eq!(step(0.1), SIZES[0]);
        assert_eq!(step(5.0), SIZES[9]);
    }
}
