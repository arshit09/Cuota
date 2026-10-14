//! Clawd's sound effects: little 8-bit tunes in the manner of the NES's pulse and triangle
//! channels, made in code the first time each plays and handed to Windows' PlaySound. They
//! follow Claude Code's sessions whether or not the panel shows: a chirp as work starts, a soft
//! blip now and then while it carries on, a jingle once it's done, and a low buzz when usage
//! can't be read. They keep quiet while the screen is locked, a game runs full screen or
//! you're presenting.

use crate::animations::{DOZE, Rng};
use crate::claude::State;
use chrono::{DateTime, Utc};
use std::f32::consts::TAU;
use std::ops::Range;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows_sys::Win32::UI::Shell::{
    QUNS_NOT_PRESENT, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};

/// While Claude works, a thinking blip comes this often, in seconds...
const BLIPS: Range<u64> = 20..60;
/// ...and work this long earns a jingle when it's done. A quick answer, watched as it came in,
/// doesn't need one.
const DONE_AFTER: Duration = Duration::from_secs(10);
/// No chirp or buzz comes this soon after another sound.
const HUSH: Duration = Duration::from_secs(4);
/// Trouble reading usage buzzes at most this often, in case it comes and goes.
const BUZZ_AGAIN: Duration = Duration::from_secs(10 * 60);

/// Samples a second, each 16 bits, in one channel.
const RATE: u32 = 44_100;
/// A note gets loud this fast, in seconds, holds for this share of it, then fades away.
const ATTACK: f32 = 0.003;
const HOLD: f32 = 0.3;
/// Vibrato wobbles this many times a second.
const VIBRATO_HZ: f32 = 6.0;
/// The square waves are softened above this, as by a TV's speaker.
const SOFTEN_HZ: f32 = 6_000.0;
/// Silence after the last note, so it isn't cut short.
const TAIL: f32 = 0.03;

// Notes, in hertz.
const E3: f32 = 164.81;
const F3: f32 = 174.61;
const B3: f32 = 246.94;
const C5: f32 = 523.25;
const A5: f32 = 880.0;
const C6: f32 = 1046.5;
const D6: f32 = 1174.66;
const E6: f32 = 1318.51;
const G6: f32 = 1567.98;
const C7: f32 = 2093.0;

const SQUARE: Wave = Wave::Pulse(0.5);
const PULSE: Wave = Wave::Pulse(0.25);
const TRIANGLE: Wave = Wave::Triangle;

/// Work starts: a quick chirp, sliding up into a higher note.
const START: Tune = Tune { peak: 0.13, notes: &[note(PULSE, C6, 0.0, 0.05).slide(E6), note(PULSE, G6, 0.055, 0.1)] };
/// Work carries on: two or three soft notes, like Clawd thinking aloud.
const THINKING: &[Tune] = &[
    Tune { peak: 0.1, notes: &[note(TRIANGLE, E6, 0.0, 0.045), note(TRIANGLE, A5, 0.065, 0.06)] },
    Tune {
        peak: 0.1,
        notes: &[note(TRIANGLE, A5, 0.0, 0.035), note(TRIANGLE, D6, 0.05, 0.035), note(TRIANGLE, A5, 0.1, 0.05)],
    },
    Tune {
        peak: 0.1,
        notes: &[note(TRIANGLE, C6, 0.0, 0.04), note(TRIANGLE, E6, 0.055, 0.04), note(TRIANGLE, D6, 0.11, 0.055)],
    },
    Tune { peak: 0.1, notes: &[note(TRIANGLE, G6, 0.0, 0.035), note(TRIANGLE, D6, 0.055, 0.06)] },
];
/// Work's done: up a major chord to a high note held with vibrato, over a soft low one.
const DONE: Tune = Tune {
    peak: 0.14,
    notes: &[
        note(SQUARE, C6, 0.0, 0.055),
        note(SQUARE, E6, 0.06, 0.055),
        note(SQUARE, G6, 0.12, 0.055),
        note(SQUARE, C7, 0.18, 0.24).level(0.7).vibrato(0.012),
        note(TRIANGLE, C5, 0.18, 0.24),
    ],
};
/// Usage can't be read: two low notes falling a tritone, the second sagging.
const ERROR: Tune =
    Tune { peak: 0.1, notes: &[note(SQUARE, B3, 0.0, 0.09), note(SQUARE, F3, 0.11, 0.2).slide(E3).vibrato(0.02)] };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Start,
    /// One of the [`THINKING`] tunes.
    Think(usize),
    Done,
    Error,
}

impl Sound {
    fn tune(self) -> &'static Tune {
        match self {
            Self::Start => &START,
            Self::Think(tune) => &THINKING[tune % THINKING.len()],
            Self::Done => &DONE,
            Self::Error => &ERROR,
        }
    }

    /// The sound as a WAV file, made the first time it's asked for. PlaySound reads it as it
    /// plays, so it's kept for good.
    fn wav(self) -> &'static [u8] {
        static WAVS: [OnceLock<Vec<u8>>; 3 + THINKING.len()] = [const { OnceLock::new() }; 3 + THINKING.len()];
        let index = match self {
            Self::Start => 0,
            Self::Done => 1,
            Self::Error => 2,
            Self::Think(tune) => 3 + tune % THINKING.len(),
        };
        WAVS[index].get_or_init(|| wav(&self.tune().render()))
    }
}

/// Plays `sound`, cutting short any still playing, unless Windows says not to disturb.
pub fn play(sound: Sound) {
    if do_not_disturb() {
        return;
    }
    let wav = sound.wav();
    unsafe { PlaySoundW(wav.as_ptr().cast(), 0, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
}

/// The screen is locked or its saver is on, a game runs full screen, or presentation settings
/// are on. Other full-screen windows, like an editor's, don't count.
fn do_not_disturb() -> bool {
    let mut state = 0;
    let read = unsafe { SHQueryUserNotificationState(&mut state) } >= 0;
    read && matches!(state, QUNS_NOT_PRESENT | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
}

/// Picks the sound, if any, for what Claude's doing. It keeps up with sounds off too, so
/// turning them on mid-work doesn't greet work long under way.
pub struct Cues {
    rng: Rng,
    /// Whether sounds were on at the last look, from the first look on.
    on: Option<bool>,
    /// The sessions busy at the last look, by name and start time.
    busy: Vec<(String, Option<DateTime<Utc>>)>,
    /// When the work under way started.
    working: Option<Instant>,
    /// When Claude was last seen working, or seen to have stopped.
    worked: Option<Instant>,
    /// Whether usage couldn't be read at the last look.
    erred: bool,
    /// When the last sound played, and the last buzz.
    played: Option<Instant>,
    buzzed: Option<Instant>,
    /// When the next thinking blip is due, while Claude works.
    blip: Option<Instant>,
}

impl Cues {
    pub fn new() -> Self {
        Self {
            rng: Rng::new(),
            on: None,
            busy: Vec::new(),
            working: None,
            worked: None,
            erred: false,
            played: None,
            buzzed: None,
            blip: None,
        }
    }

    /// The sound `state` calls for at `now`, if sounds are `on`. Turning them on plays one
    /// straight away, to hear what they're like.
    pub fn update(&mut self, state: &State, on: bool, now: Instant) -> Option<Sound> {
        let since = |t: Option<Instant>| t.map_or(Duration::MAX, |t| now.saturating_duration_since(t));
        let busy: Vec<_> = state.sessions.iter().filter(|s| s.busy).map(|s| (s.name.clone(), s.started)).collect();
        let started =
            state.working && (self.working.is_none() || busy.iter().any(|session| !self.busy.contains(session)));
        self.busy = busy;
        // How long the work that just ended lasted.
        let ended = if state.working {
            self.working.get_or_insert(now);
            None
        } else {
            self.working.take().map(|began| now.saturating_duration_since(began))
        };
        // As for Clawd, work ends at the first look that finds it over.
        if state.working || ended.is_some() {
            self.worked = Some(now);
        }
        let erred = state.error.is_some() && !self.erred;
        self.erred = state.error.is_some();
        // Trouble is worth a buzz only while Clawd's awake.
        let awake = since(self.worked) < DOZE;
        let hushed = since(self.played) < HUSH;

        let mut sound = if ended.is_some_and(|lasted| lasted >= DONE_AFTER) {
            Some(Sound::Done)
        } else if started && !hushed {
            Some(Sound::Start)
        } else if erred && awake && !hushed && since(self.buzzed) >= BUZZ_AGAIN {
            Some(Sound::Error)
        } else if state.working && self.blip.is_some_and(|due| now >= due) {
            Some(Sound::Think(self.rng.below(THINKING.len())))
        } else {
            None
        };
        if on && self.on == Some(false) {
            sound = sound.or(Some(Sound::Start));
        }
        self.on = Some(on);
        if sound.is_some() {
            self.played = Some(now);
        }
        if sound == Some(Sound::Error) {
            self.buzzed = Some(now);
        }
        // The next blip comes a while after the last sound.
        if !state.working {
            self.blip = None;
        } else if sound.is_some() || self.blip.is_none() {
            self.blip = Some(now + self.rng.seconds(BLIPS));
        }
        sound.filter(|_| on)
    }

    /// When the next thinking blip is due, while Claude works and sounds are on.
    pub fn due(&self) -> Option<Instant> {
        self.blip.filter(|_| self.on == Some(true))
    }
}

#[derive(Clone, Copy)]
enum Wave {
    /// High for this share of each cycle, like the NES's two pulse channels.
    Pulse(f32),
    /// Like its triangle channel: rounder and softer, in 16 steps.
    Triangle,
}

impl Wave {
    /// Its level at `phase`, from 0 to 1 through a cycle. It averages 0 over a cycle, so notes
    /// don't thump.
    fn at(self, phase: f32) -> f32 {
        match self {
            Self::Pulse(duty) => 2.0 * if phase < duty { 1.0 - duty } else { -duty },
            Self::Triangle => {
                // Down from 15 to 0 and back up, in 32 steps a cycle.
                let step = (phase * 32.0) as i32;
                let level = if step < 16 { 15 - step } else { step - 16 };
                level as f32 / 7.5 - 1.0
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Note {
    wave: Wave,
    /// Its pitch in hertz, sliding from one to the other.
    from: f32,
    to: f32,
    /// When it starts and how long it lasts, in seconds.
    at: f32,
    length: f32,
    /// How loud it is beside the others.
    level: f32,
    /// How far its pitch wobbles, as a share of it.
    vibrato: f32,
}

const fn note(wave: Wave, hz: f32, at: f32, length: f32) -> Note {
    Note { wave, from: hz, to: hz, at, length, level: 1.0, vibrato: 0.0 }
}

impl Note {
    const fn slide(self, to: f32) -> Self {
        Self { to, ..self }
    }

    const fn level(self, level: f32) -> Self {
        Self { level, ..self }
    }

    const fn vibrato(self, vibrato: f32) -> Self {
        Self { vibrato, ..self }
    }
}

/// Notes, one after another or together, and how loud the loudest moment is, as a share of
/// full scale.
struct Tune {
    notes: &'static [Note],
    peak: f32,
}

impl Tune {
    fn render(&self) -> Vec<i16> {
        let rate = RATE as f32;
        let end = self.notes.iter().map(|n| n.at + n.length).fold(0.0, f32::max) + TAIL;
        let mut mix = vec![0.0; (end * rate).ceil() as usize];
        for note in self.notes {
            let mut phase = 0.0;
            let samples = mix.iter_mut().skip((note.at * rate) as usize).take((note.length * rate) as usize);
            for (i, sample) in samples.enumerate() {
                let t = i as f32 / rate;
                let wobble = 1.0 + note.vibrato * (TAU * VIBRATO_HZ * t).sin();
                let hz = note.from * (note.to / note.from).powf(t / note.length) * wobble;
                *sample += note.level * envelope(t, note.length) * note.wave.at(phase);
                phase = (phase + hz / rate).fract();
            }
        }
        let smoothing = 1.0 - (-TAU * SOFTEN_HZ / rate).exp();
        let mut soft = 0.0;
        for sample in &mut mix {
            soft += smoothing * (*sample - soft);
            *sample = soft;
        }
        let loudest = mix.iter().fold(0.0, |loudest: f32, s| loudest.max(s.abs()));
        let gain = if loudest > 0.0 { self.peak * f32::from(i16::MAX) / loudest } else { 0.0 };
        mix.iter().map(|s| (s * gain).round() as i16).collect()
    }
}

/// How loud a note is `t` seconds into its `length`: up in an instant, held a while, then
/// fading away, in the NES's 16 steps of volume.
fn envelope(t: f32, length: f32) -> f32 {
    let rise = (t / ATTACK).min(1.0);
    let fall = ((1.0 - t / length) / (1.0 - HOLD)).clamp(0.0, 1.0);
    (rise * fall * 15.0).round() / 15.0
}

/// `samples` as a WAV file.
fn wav(samples: &[i16]) -> Vec<u8> {
    let data = 2 * samples.len() as u32;
    let header: [&[u8]; 12] = [
        b"RIFF",
        &(36 + data).to_le_bytes(),
        b"WAVEfmt ",
        // The format's size, then PCM in one channel: samples and bytes a second, and bytes and
        // bits a sample.
        &16u32.to_le_bytes(),
        &1u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &RATE.to_le_bytes(),
        &(2 * RATE).to_le_bytes(),
        &2u16.to_le_bytes(),
        &16u16.to_le_bytes(),
        b"data",
        &data.to_le_bytes(),
    ];
    let mut wav = header.concat();
    wav.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    wav
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::Session;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const SOUNDS: [(&str, Sound); 7] = [
        ("start", Sound::Start),
        ("thinking-1", Sound::Think(0)),
        ("thinking-2", Sound::Think(1)),
        ("thinking-3", Sound::Think(2)),
        ("thinking-4", Sound::Think(3)),
        ("done", Sound::Done),
        ("error", Sound::Error),
    ];

    /// Claude Code with the `busy` sessions working and the `idle` ones open.
    fn state(busy: &[&str], idle: &[&str]) -> State {
        let session = |name: &str, busy| Session {
            name: name.into(),
            title: None,
            project: None,
            app: "Terminal".into(),
            busy,
            started: None,
        };
        let sessions = busy.iter().map(|&name| session(name, true)).chain(idle.iter().map(|&name| session(name, false)));
        State { working: !busy.is_empty(), sessions: sessions.collect(), ..State::default() }
    }

    fn erring(state: State) -> State {
        State { error: Some("Network error: offline".into()), ..state }
    }

    #[test]
    fn every_sound_is_a_short_quiet_wav_that_starts_and_ends_silent() {
        assert_eq!(SOUNDS.len(), 3 + THINKING.len());
        for (name, sound) in SOUNDS {
            let wav = sound.wav();
            let u16_at = |i: usize| u16::from_le_bytes([wav[i], wav[i + 1]]);
            let u32_at = |i: usize| u32::from_le_bytes([wav[i], wav[i + 1], wav[i + 2], wav[i + 3]]);
            let tags: [&[u8]; 3] = [b"RIFF", b"WAVEfmt ", b"data"];
            assert_eq!([&wav[..4], &wav[8..16], &wav[36..40]], tags, "{name}");
            assert_eq!(u32_at(4) as usize, wav.len() - 8, "{name}");
            // PCM in one channel at 44.1 kHz, 16 bits a sample.
            let format = (u32_at(16), u16_at(20), u16_at(22), u32_at(24), u32_at(28), u16_at(32), u16_at(34));
            assert_eq!(format, (16, 1, 1, RATE, 2 * RATE, 2, 16), "{name}");
            assert_eq!(u32_at(40) as usize, wav.len() - 44, "{name}");

            let samples: Vec<i16> = wav[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
            let seconds = samples.len() as f32 / RATE as f32;
            assert!((0.1..0.6).contains(&seconds), "{name} lasts {seconds} s");
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or_default();
            assert_eq!(peak, (sound.tune().peak * f32::from(i16::MAX)).round() as u16, "{name}");
            assert!(f32::from(peak) <= 0.2 * f32::from(i16::MAX), "{name} is loud");
            assert_eq!((samples[0], samples[samples.len() - 1]), (0, 0), "{name} clicks");
        }
    }

    #[test]
    fn each_sound_is_made_once() {
        assert!(std::ptr::eq(Sound::Done.wav(), Sound::Done.wav()));
        assert!(std::ptr::eq(Sound::Think(1).wav(), Sound::Think(1 + THINKING.len()).wav()));
        assert!(!std::ptr::eq(Sound::Think(0).wav(), Sound::Think(1).wav()));
    }

    #[test]
    fn work_starting_chirps_and_work_done_a_while_later_jingles_once() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&state(&[], &["a"]), true, at(0)), None);
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(2)), Some(Sound::Start));
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(4)), None);
        assert_eq!(cues.update(&state(&[], &["a"]), true, at(16)), Some(Sound::Done));
        assert_eq!(cues.update(&state(&[], &["a"]), true, at(18)), None);
        // A quick answer chirps, but doesn't jingle.
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(30)), Some(Sound::Start));
        assert_eq!(cues.update(&state(&[], &["a"]), true, at(36)), None);
    }

    #[test]
    fn another_session_starting_chirps_unless_a_sound_just_played() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&state(&["a", "b"], &[]), true, at(2)), None);
        assert_eq!(cues.update(&state(&["a", "b", "c"], &[]), true, at(10)), Some(Sound::Start));
        // One finishing while others carry on is still work.
        assert_eq!(cues.update(&state(&["a", "c"], &["b"]), true, at(15)), None);
        assert_eq!(cues.update(&state(&["a", "b", "c"], &[]), true, at(18)), Some(Sound::Start));
    }

    #[test]
    fn thinking_blips_come_every_so_often_while_claude_works() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let working = state(&["a"], &["b"]);
        let mut sounds = Vec::new();
        // Half an hour of work, looked at every second.
        for second in 0..=1800 {
            let now = start + Duration::from_secs(second);
            if let Some(sound) = cues.update(&working, true, now) {
                sounds.push((second, sound));
            }
            assert!(cues.due().is_some_and(|due| due > now), "the next blip is due at {second} s");
        }
        assert_eq!(sounds[0], (0, Sound::Start));
        for pair in sounds.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            assert!(BLIPS.contains(&gap), "{gap} s between blips");
            assert!(matches!(pair[1].1, Sound::Think(tune) if tune < THINKING.len()), "{pair:?}");
        }
        assert!((31..=91).contains(&sounds.len()), "{} sounds", sounds.len());
        let tune = |sound: &Sound| if let Sound::Think(tune) = *sound { Some(tune) } else { None };
        let tunes: BTreeSet<usize> = sounds.iter().filter_map(|(_, sound)| tune(sound)).collect();
        assert!(tunes.len() >= 3, "only {tunes:?}");
    }

    #[test]
    fn nothing_plays_while_claude_is_idle() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        for second in (0..7200).step_by(10) {
            let now = start + Duration::from_secs(second);
            assert_eq!(cues.update(&state(&[], &["a", "b"]), true, now), None);
            assert_eq!(cues.due(), None);
        }
        assert_eq!(cues.update(&State::default(), true, start + Duration::from_secs(7200)), None);
    }

    #[test]
    fn with_sounds_off_nothing_plays_and_turning_them_on_plays_one() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        let working = state(&["a"], &[]);
        for second in 0..=100 {
            assert_eq!(cues.update(&working, false, at(second)), None);
            assert_eq!(cues.due(), None);
        }
        assert!(cues.update(&working, true, at(101)).is_some());
        // Work long under way isn't greeted, and no blip follows hard on the heels.
        assert_eq!(cues.update(&working, true, at(102)), None);
        assert!(cues.due().is_some_and(|due| due >= at(101) + Duration::from_secs(BLIPS.start)));
        assert_eq!(cues.update(&state(&[], &["a"]), false, at(120)), None);
    }

    #[test]
    fn trouble_reading_usage_buzzes_once_while_clawds_awake() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&erring(state(&["a"], &[])), true, at(10)), Some(Sound::Error));
        assert_eq!(cues.update(&erring(state(&["a"], &[])), true, at(12)), None);
        // Coming and going, it doesn't buzz again soon.
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(14)), None);
        assert_eq!(cues.update(&erring(state(&["a"], &[])), true, at(16)), None);

        // Nobody's around: Claude hasn't worked since Cuota started, or not for a while.
        let mut cues = Cues::new();
        assert_eq!(cues.update(&erring(state(&[], &["a"])), true, at(0)), None);
        let mut cues = Cues::new();
        assert_eq!(cues.update(&state(&["a"], &[]), true, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&state(&[], &["a"]), true, at(20)), Some(Sound::Done));
        assert_eq!(cues.update(&erring(state(&[], &["a"])), true, at(20) + DOZE), None);
    }

    /// Writes each sound to a WAV file to listen to, in the folder named by `CUOTA_SOUNDS` or
    /// else `target\sounds`: `cargo test write_sounds -- --ignored`.
    #[test]
    #[ignore]
    fn write_sounds() {
        let dir = std::env::var_os("CUOTA_SOUNDS").map_or_else(|| PathBuf::from("target").join("sounds"), PathBuf::from);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, sound) in SOUNDS {
            std::fs::write(dir.join(format!("{name}.wav")), sound.wav()).unwrap();
        }
    }
}
