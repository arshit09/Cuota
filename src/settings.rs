//! User preferences, saved as JSON in `%APPDATA%\Cuota\settings.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// A floating panel above all windows.
    #[default]
    Popup,
    /// A panel behind all windows, like a desktop widget.
    Desktop,
    /// Icons in the taskbar's notification area; clicking one opens the panel.
    Taskbar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Bars,
    Pie,
}

/// What one surface (the panel, or the taskbar icons) shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub clawd: bool,
    pub percent: bool,
    pub time_left: bool,
    pub style: Style,
    /// Labels of the limits the user hid, e.g. "Fable".
    pub hidden: BTreeSet<String>,
}

impl Default for View {
    fn default() -> Self {
        Self { clawd: true, percent: true, time_left: true, style: Style::Bars, hidden: BTreeSet::new() }
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub animate: bool,
    pub opacity: f32,
    pub panel: View,
    pub taskbar: View,
    /// Top-left of the panel in Popup and Desktop modes, in points.
    pub panel_pos: Option<[f32; 2]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Popup,
            animate: true,
            opacity: 0.96,
            panel: View::default(),
            taskbar: View::default(),
            panel_pos: None,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Cuota").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
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
