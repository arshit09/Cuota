//! User preferences, saved as JSON in `%APPDATA%\Cuota\settings.json`.

use crate::log;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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
    /// Clawd on his own, walking about the screen with no panel around him (see pet.rs). Goes
    /// with any of the others.
    Pet,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Bars,
    Pie,
}

/// How much the sound effects have to say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundMode {
    /// Only what matters: work starting, Claude asking you something, work done, and trouble.
    #[default]
    Normal,
    /// A sound for each kind of step Claude takes too, so you can hear what it's up to.
    Lively,
}

/// The panel's colors (see theme.rs). The settings window stays dark whichever is picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Light text on a dark panel, as Cuota always was.
    #[default]
    Dark,
    /// Dark text on a light panel.
    Light,
    /// Light or dark as Windows' apps are (Settings > Colors > default app mode), changing
    /// along with it.
    Windows,
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
    /// A line above the meters saying what Clawd's up to, like "Percolating…". The panel only.
    pub status: bool,
    pub percent: bool,
    pub time_left: bool,
    pub style: Style,
    /// Labels of the limits the user hid, e.g. "Fable".
    pub hidden: BTreeSet<String>,
    /// More it shows under the meters, from what the expanded panel's details tell. The panel
    /// only, and left out of the file until changed.
    #[serde(skip_serializing_if = "Details::is_default")]
    pub details: Details,
}

impl Default for View {
    fn default() -> Self {
        Self {
            clawd: true,
            status: true,
            percent: true,
            time_left: true,
            style: Style::Bars,
            hidden: BTreeSet::new(),
            details: Details::default(),
        }
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

/// Something more the panel can show under its meters, from what its expanded details tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Detail {
    /// When each limit resets.
    Resets,
    /// Where each limit is headed at the pace it's going.
    Pace,
    /// How much of the Weekly limit each model's own limit has used.
    WeeklyShare,
    /// What the week's usage went to, as a split bar.
    Apps,
    /// The plan, and since when.
    Plan,
    /// Whether extra usage is on, and what it has spent.
    ExtraUsage,
    /// How many Claude Code sessions are running, and how many of them work.
    Sessions,
    /// The running sessions' titles.
    SessionTitles,
    /// Claude Code's replies, tool calls and tokens this week.
    ClaudeCode,
    /// Claude Code's tokens each day this week, as little bars.
    TokensPerDay,
    /// The hour of the day it replies most.
    BusiestHour,
    /// The model Claude Code used most this week, by tokens.
    TopModel,
    /// The folder it worked in most.
    TopProject,
    /// When usage was last fetched.
    Updated,
}

impl Detail {
    /// In the order the expanded panel's details tell them.
    pub const ALL: [Self; 14] = [
        Self::Resets,
        Self::Pace,
        Self::WeeklyShare,
        Self::Apps,
        Self::Plan,
        Self::ExtraUsage,
        Self::Sessions,
        Self::SessionTitles,
        Self::ClaudeCode,
        Self::TokensPerDay,
        Self::BusiestHour,
        Self::TopModel,
        Self::TopProject,
        Self::Updated,
    ];

    /// Whether it comes from Claude Code's transcripts, which are only read while something
    /// shows them.
    pub fn uses_stats(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::TokensPerDay | Self::BusiestHour | Self::TopModel | Self::TopProject)
    }
}

/// Every [`Detail`] in the order the panel draws them, and whether each one shows. None do
/// until picked.
#[derive(Clone, Debug, PartialEq)]
pub struct Details(Vec<(Detail, bool)>);

impl Default for Details {
    fn default() -> Self {
        Self(Detail::ALL.map(|detail| (detail, false)).to_vec())
    }
}

/// Keeps the first of each detail, in the order given, and adds any left out at the end,
/// hidden, so a detail newer than the list still has a place in it.
impl FromIterator<(Detail, bool)> for Details {
    fn from_iter<I: IntoIterator<Item = (Detail, bool)>>(items: I) -> Self {
        let mut list: Vec<(Detail, bool)> = Vec::new();
        for (detail, shown) in items {
            if !list.iter().any(|(listed, _)| *listed == detail) {
                list.push((detail, shown));
            }
        }
        for detail in Detail::ALL {
            if !list.iter().any(|(listed, _)| *listed == detail) {
                list.push((detail, false));
            }
        }
        Self(list)
    }
}

impl Details {
    /// Each detail, in order, and whether it shows.
    pub fn items(&self) -> &[(Detail, bool)] {
        &self.0
    }

    /// The details that show, in order.
    pub fn shown(&self) -> impl Iterator<Item = Detail> + '_ {
        self.0.iter().filter(|(_, shown)| *shown).map(|(detail, _)| *detail)
    }

    /// Whether any detail that shows needs Claude Code's stats.
    pub fn use_stats(&self) -> bool {
        self.shown().any(Detail::uses_stats)
    }

    /// Shows the detail at `index`, or hides it if it shows.
    pub fn toggle(&mut self, index: usize) {
        if let Some((_, shown)) = self.0.get_mut(index) {
            *shown = !*shown;
        }
    }

    /// Moves the detail at `from` to `to`, where it lands once out of the way, sliding the ones
    /// between over by one.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from < self.0.len() {
            let item = self.0.remove(from);
            self.0.insert(to.min(self.0.len()), item);
        }
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A detail as saved: by name, so a name this version doesn't know (one from a newer version)
/// is skipped rather than failing the whole file.
#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(deserialize_with = "known")]
    detail: Option<Detail>,
    #[serde(default)]
    shown: bool,
}

/// The detail named, or `None` for one this version doesn't know, whatever form its name takes.
fn known<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Detail>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

impl Serialize for Details {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let saved: Vec<Saved> = self.0.iter().map(|&(detail, shown)| Saved { detail: Some(detail), shown }).collect();
        saved.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Details {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = Vec::<Saved>::deserialize(deserializer)?;
        Ok(saved.into_iter().filter_map(|saved| Some((saved.detail?, saved.shown))).collect())
    }
}

/// The steps of the panel's Size slider, as multiples of its original size: small, medium
/// (the original) and large, and the steps between.
pub const SIZES: [f32; 10] = [0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6];

/// The step of [`SIZES`] nearest to `size`.
pub fn size_step(size: f32) -> usize {
    let off = |step: &usize| (SIZES[*step] - size).abs();
    (0..SIZES.len()).min_by(|a, b| off(a).total_cmp(&off(b))).unwrap_or_default()
}

/// Clawd on his own as a desktop pet, when Show as includes [`Mode::Pet`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pet {
    /// How big he is: one of [`SIZES`].
    pub size: f32,
    /// What he's up to, like "Percolating…", under him.
    pub status: bool,
    /// His eyes follow the pointer, and now and then he walks over to it to say hi, keep it
    /// company or nap beside it.
    pub react: bool,
    /// Resting the pointer on him shows a card beside him, with what `card` picks.
    pub hover: bool,
    /// What that card shows: its meters, and the details under them. Its `clawd` and `status`
    /// aren't used.
    pub card: View,
    /// Where he last settled, so he's there next time: the top-left of his frame (see
    /// `animations::FRAME`), in pixels.
    pub pos: Option<[f32; 2]>,
    /// He walks over to the monitors in `monitors` too, rather than keeping to the one he's on.
    pub travel: bool,
    /// The monitors he travels across, by Windows' name for them, like `\\.\DISPLAY2`.
    pub monitors: BTreeSet<String>,
}

impl Default for Pet {
    fn default() -> Self {
        // A quick look: the limits, and the sessions behind them.
        let details = [(Detail::Sessions, true), (Detail::Updated, true)].into_iter().collect();
        Self {
            size: 1.0,
            status: true,
            react: true,
            hover: true,
            card: View { details, ..View::default() },
            pos: None,
            travel: false,
            monitors: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show: BTreeSet<Mode>,
    pub animate: bool,
    /// Clawd's 8-bit sound effects while Claude works.
    pub sounds: bool,
    pub sound_mode: SoundMode,
    pub opacity: f32,
    /// The panel's colors, which the desktop pet's card shares.
    pub theme: Theme,
    /// How big the panel is: one of [`SIZES`].
    pub size: f32,
    /// Scrolling the mouse wheel over the panel steps its size up and down.
    pub wheel_resize: bool,
    /// The panel slides out of the way of a pointer slowing down as it comes to it, to click
    /// what's behind (see dodge.rs). Off until turned on, since the pointer then needs Ctrl
    /// held to reach the panel, even to right-click it.
    pub move_aside: bool,
    pub panel: View,
    pub taskbar: View,
    pub pet: Pet,
    pub double_click: DoubleClick,
    /// What a Custom double-click opens: the path of a file, program, script or shortcut.
    /// Empty until one is picked.
    pub custom_target: String,
    /// Top-left of the panel in Always on top and Desktop modes, in points.
    pub panel_pos: Option<[f32; 2]>,
    /// How tall the panel was there without its details, in points, so it opens with its middle
    /// where it was, however tall it is now.
    pub panel_height: Option<f32>,
    /// The monitors that each show a copy of the panel in Always on top and Desktop modes, by
    /// Windows' name for them, like `\\.\DISPLAY2`. With fewer than two of them plugged in,
    /// there's just the one panel, wherever it's dragged.
    pub panel_monitors: BTreeSet<String>,
    /// Where the panel sits on each monitor while it shows on several: its top-left from the
    /// top-left of what the taskbar leaves of that monitor, in points.
    pub panel_spots: BTreeMap<String, [f32; 2]>,
    /// Notes how much of each limit Claude Code's tasks use (see usage.rs).
    pub track_usage: bool,
    /// Looks on GitHub for a new release of Cuota once a day, and installs it (see update.rs).
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show: BTreeSet::from([Mode::AlwaysOnTop]),
            animate: true,
            sounds: true,
            sound_mode: SoundMode::Normal,
            opacity: 0.96,
            theme: Theme::Dark,
            size: 1.0,
            wheel_resize: true,
            move_aside: false,
            panel: View::default(),
            taskbar: View::default(),
            pet: Pet::default(),
            double_click: DoubleClick::default(),
            custom_target: String::new(),
            panel_pos: None,
            panel_height: None,
            panel_monitors: BTreeSet::new(),
            panel_spots: BTreeMap::new(),
            track_usage: true,
            check_updates: true,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Cuota").join("settings.json"))
    }

    pub fn load() -> Self {
        let mut settings = Self::path().and_then(|path| Self::read(&path)).unwrap_or_default();
        // Showing nowhere would leave nothing to right-click for settings.
        if settings.show.is_empty() {
            settings.show = Self::default().show;
        }
        // A size edited into the file by hand snaps to the nearest step.
        settings.size = SIZES[settings.size_step()];
        settings.pet.size = SIZES[size_step(settings.pet.size)];
        settings
    }

    /// The settings saved at `path`. None are saved before the first change, but settings that
    /// can't be read are logged, since the defaults replace them at the next save.
    fn read(path: &Path) -> Option<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return None,
            Err(e) => {
                log::warn!("Couldn't open the settings in {}, so they start over: {e}", path.display());
                return None;
            }
        };
        serde_json::from_str(&text)
            .inspect_err(|e| log::warn!("Couldn't read the settings in {}, so they start over: {e}", path.display()))
            .ok()
    }

    /// The Size slider's step nearest to `size`.
    pub fn size_step(&self) -> usize {
        size_step(self.size)
    }

    /// Moves the size `steps` steps up (or down, when negative), stopping at either end.
    pub fn step_size(&mut self, steps: i32) {
        let step = self.size_step() as i32 + steps;
        self.size = SIZES[step.clamp(0, SIZES.len() as i32 - 1) as usize];
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
            Mode::Taskbar | Mode::Pet => false,
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
        let save = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, serde_json::to_string_pretty(self)?)
        };
        if let Err(e) = save() {
            log::error!("Couldn't save the settings to {}: {e}", path.display());
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
    fn sound_effects_are_on_unless_turned_off_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"animate":false}"#).unwrap();
        assert!(older.sounds && !older.animate);
        assert_eq!(older.sound_mode, SoundMode::Normal);
        let lively = Settings { sound_mode: SoundMode::Lively, ..Settings::default() };
        let text = serde_json::to_string(&lively).unwrap();
        assert!(text.contains(r#""sound_mode":"Lively""#), "{text}");
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().sound_mode, SoundMode::Lively);
        let off = Settings { sounds: false, ..Settings::default() };
        let text = serde_json::to_string(&off).unwrap();
        assert!(!serde_json::from_str::<Settings>(&text).unwrap().sounds);
    }

    #[test]
    fn usage_is_tracked_unless_turned_off_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"animate":false}"#).unwrap();
        assert!(older.track_usage);
        let off = Settings { track_usage: false, ..Settings::default() };
        let text = serde_json::to_string(&off).unwrap();
        assert!(!serde_json::from_str::<Settings>(&text).unwrap().track_usage);
    }

    #[test]
    fn clawds_status_shows_unless_turned_off_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"panel":{"clawd":true,"percent":false}}"#).unwrap();
        assert!(older.panel.status && !older.panel.percent);
        let off = Settings { panel: View { status: false, ..View::default() }, ..Settings::default() };
        let text = serde_json::to_string(&off).unwrap();
        assert!(!serde_json::from_str::<Settings>(&text).unwrap().panel.status);
    }

    fn order(details: &Details) -> Vec<Detail> {
        details.items().iter().map(|(detail, _)| *detail).collect()
    }

    #[test]
    fn the_panel_shows_no_details_until_some_are_picked_and_older_settings_still_load() {
        let settings = Settings::default();
        assert_eq!(settings.panel.details.shown().count(), 0);
        assert_eq!(order(&settings.panel.details), Detail::ALL);
        let saved = serde_json::to_value(&settings).unwrap();
        for view in ["panel", "taskbar"] {
            assert!(saved[view].get("details").is_none(), "untouched, they stay out of the file: {saved}");
        }
        let older: Settings = serde_json::from_str(r#"{"panel":{"clawd":true,"percent":false},"opacity":0.8}"#).unwrap();
        assert_eq!(older.panel.details, Details::default());
        assert_eq!(older.opacity, 0.8);
    }

    #[test]
    fn picked_details_keep_their_order_when_saved() {
        let mut settings = Settings::default();
        let details = &mut settings.panel.details;
        details.move_item(Detail::ALL.len() - 1, 0);
        details.toggle(0);
        details.toggle(3);
        let shown: Vec<Detail> = settings.panel.details.shown().collect();
        assert_eq!(shown, [Detail::Updated, Detail::WeeklyShare]);
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains(r#"{"detail":"Updated","shown":true}"#), "{text}");
        let loaded: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded.panel.details, settings.panel.details);
        assert_eq!(loaded.taskbar.details, Details::default());
    }

    #[test]
    fn details_from_another_version_load_without_losing_the_rest() {
        // A detail this version doesn't know, one listed twice, and most of them left out.
        let text = r#"{"opacity":0.7,"panel":{"details":[
            {"detail":"TokensPerDay","shown":true},
            {"detail":"Horoscope","shown":true},
            {"detail":{"Weather":"Oslo"},"shown":true},
            {"detail":"Pace"},
            {"detail":"TokensPerDay","shown":false}
        ]}}"#;
        let settings: Settings = serde_json::from_str(text).unwrap();
        assert_eq!(settings.opacity, 0.7);
        let details = &settings.panel.details;
        assert_eq!(details.shown().collect::<Vec<_>>(), [Detail::TokensPerDay]);
        let listed = order(details);
        assert_eq!(listed[..2], [Detail::TokensPerDay, Detail::Pace]);
        // The rest follow in their usual order, hidden.
        let rest: Vec<Detail> = Detail::ALL.into_iter().filter(|d| ![Detail::TokensPerDay, Detail::Pace].contains(d)).collect();
        assert_eq!(listed[2..], rest);
    }

    #[test]
    fn a_moved_detail_lands_where_it_was_dropped() {
        let mut details = Details::default();
        let [a, b, c, d] = [Detail::ALL[0], Detail::ALL[1], Detail::ALL[2], Detail::ALL[3]];
        details.move_item(0, 2);
        assert_eq!(order(&details)[..4], [b, c, a, d], "down");
        details.move_item(2, 0);
        assert_eq!(order(&details)[..4], [a, b, c, d], "and back up");
        details.move_item(3, 1);
        assert_eq!(order(&details)[..4], [a, d, b, c], "up past two");
        details.move_item(1, 1);
        assert_eq!(order(&details)[..4], [a, d, b, c], "dropped where it was");
        details.move_item(0, 99);
        assert_eq!(*order(&details).last().unwrap(), a, "past the end, to the end");
        details.move_item(99, 0);
        assert_eq!(order(&details).len(), Detail::ALL.len(), "nothing there to move");
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

    #[test]
    fn the_wheel_steps_the_size_and_stops_at_either_end() {
        let mut settings = Settings::default();
        settings.step_size(1);
        assert_eq!(settings.size, 1.1);
        settings.step_size(-3);
        assert_eq!(settings.size, 0.8);
        settings.step_size(-5);
        assert_eq!(settings.size, SIZES[0]);
        settings.step_size(20);
        assert_eq!(settings.size, SIZES[9]);
    }

    #[test]
    fn wheel_resizing_is_on_unless_turned_off_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"size":1.2}"#).unwrap();
        assert!(older.wheel_resize);
        let off = Settings { wheel_resize: false, ..Settings::default() };
        let text = serde_json::to_string(&off).unwrap();
        assert!(!serde_json::from_str::<Settings>(&text).unwrap().wheel_resize);
    }

    #[test]
    fn the_desktop_pet_goes_with_any_mode_and_keeps_its_own_settings() {
        let mut settings = Settings::default();
        assert!(!settings.shows_as(Mode::Pet), "off until picked");
        settings.set_shown_as(Mode::Pet, true);
        settings.set_shown_as(Mode::Desktop, true);
        assert_eq!(settings.show, BTreeSet::from([Mode::Desktop, Mode::Pet]));
        // Pet alone: no panel stays on screen.
        settings.set_shown_as(Mode::Desktop, false);
        assert_eq!(settings.pinned(), None);

        // Older settings get him with everything on, and a card with a couple of details.
        let older: Settings = serde_json::from_str(r#"{"show":["Taskbar","Pet"],"size":1.2}"#).unwrap();
        assert!(older.shows_as(Mode::Pet) && older.pet.status && older.pet.react && older.pet.hover);
        assert_eq!(older.pet.size, 1.0);
        assert_eq!(older.pet.card.details.shown().collect::<Vec<_>>(), [Detail::Sessions, Detail::Updated]);
        let mut changed = Settings { pet: Pet { react: false, size: 1.4, pos: Some([10.0, 20.0]), ..Pet::default() }, ..older };
        changed.pet.card.details.toggle(0);
        let text = serde_json::to_string(&changed).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), changed);
        // Picking none of the card's details is kept, though it matches View's default.
        let none = Pet { card: View::default(), ..Pet::default() };
        let text = serde_json::to_string(&Settings { pet: none.clone(), ..Settings::default() }).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().pet, none);
        assert_eq!(SIZES[size_step(1.43)], 1.4);
    }

    #[test]
    fn the_panel_and_the_pet_keep_to_one_monitor_until_more_are_picked_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"panel_pos":[10.0,20.0],"pet":{"react":false}}"#).unwrap();
        assert!(older.panel_monitors.is_empty() && older.panel_spots.is_empty());
        assert!(!older.pet.travel && older.pet.monitors.is_empty() && !older.pet.react);
        let mut picked = older;
        picked.panel_monitors = BTreeSet::from([r"\\.\DISPLAY1".to_owned(), r"\\.\DISPLAY2".to_owned()]);
        picked.panel_spots.insert(r"\\.\DISPLAY2".to_owned(), [40.0, 12.5]);
        picked.pet.travel = true;
        picked.pet.monitors = BTreeSet::from([r"\\.\DISPLAY2".to_owned()]);
        let text = serde_json::to_string(&picked).unwrap();
        assert!(text.contains(r#""panel_monitors":["\\\\.\\DISPLAY1","\\\\.\\DISPLAY2"]"#), "{text}");
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), picked);
    }

    #[test]
    fn moving_aside_is_off_until_turned_on_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"size":1.2}"#).unwrap();
        assert!(!older.move_aside);
        let on = Settings { move_aside: true, ..Settings::default() };
        let text = serde_json::to_string(&on).unwrap();
        assert!(serde_json::from_str::<Settings>(&text).unwrap().move_aside);
    }

    #[test]
    fn the_panel_stays_dark_until_another_theme_is_picked_even_in_older_settings() {
        let older: Settings = serde_json::from_str(r#"{"opacity":0.8}"#).unwrap();
        assert_eq!(older.theme, Theme::Dark);
        for theme in [Theme::Light, Theme::Windows] {
            let text = serde_json::to_string(&Settings { theme, ..Settings::default() }).unwrap();
            assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().theme, theme);
        }
    }
}
