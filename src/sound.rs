//! Clawd's sound effects: little 8-bit tunes in the manner of the NES's pulse, triangle and
//! noise channels, made in code on a quiet thread as Cuota starts and handed to Windows'
//! PlaySound. They follow Claude Code's sessions whether or not the panel shows. In Normal mode
//! only what matters sounds: a chirp as work starts, a knock when Claude needs you, a jingle
//! once it's done, and a low buzz when usage can't be read. Lively mode adds a sound of its own
//! for each kind of step Claude takes, a clock when work runs long, and soft blips while it
//! thinks. They keep quiet while the screen is locked, a game runs full screen or you're
//! presenting.

use crate::animations::{DOZE, Rng};
use crate::claude::{Session, State, Step};
use crate::settings::SoundMode;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::ops::Range;
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};
use windows_sys::Win32::UI::Shell::{
    QUNS_NOT_PRESENT, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};

/// In Lively mode, while Claude works, a thinking blip comes this often, in seconds...
const BLIPS: Range<u64> = 20..60;
/// ...a step's sound comes this long after any other sound, and this long after the last of
/// its kind, so a run of edits doesn't rattle...
const STEP_GAP: Duration = Duration::from_secs(3);
const SAME_STEP: Duration = Duration::from_secs(12);
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
const C4: f32 = 261.63;
const G4: f32 = 392.0;
const A4: f32 = 440.0;
const B4: f32 = 493.88;
const C5: f32 = 523.25;
const A5: f32 = 880.0;
const B5: f32 = 987.77;
const C6: f32 = 1046.5;
const D6: f32 = 1174.66;
const E6: f32 = 1318.51;
const G6: f32 = 1567.98;
const A6: f32 = 1760.0;
const B6: f32 = 1975.53;
const C7: f32 = 2093.0;

const SQUARE: Wave = Wave::Pulse(0.5);
const PULSE: Wave = Wave::Pulse(0.25);
/// The thinnest pulse: a reedy click when short.
const THIN: Wave = Wave::Pulse(0.125);
const TRIANGLE: Wave = Wave::Triangle;
const NOISE: Wave = Wave::Noise;

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
/// Claude needs you: knock knock, then a note rising like a question.
const ASK: Tune = Tune {
    peak: 0.13,
    notes: &[note(PULSE, A5, 0.0, 0.05), note(PULSE, A5, 0.09, 0.05), note(SQUARE, E6, 0.2, 0.18).slide(A6).level(0.8)],
};
/// Work runs long: a clock's tick and tock, twice.
const LONG: Tune = Tune {
    peak: 0.09,
    notes: &[
        note(THIN, E6, 0.0, 0.025),
        note(TRIANGLE, B4, 0.17, 0.04),
        note(THIN, E6, 0.34, 0.025),
        note(TRIANGLE, B4, 0.51, 0.04),
    ],
};
/// Reading or searching: three quick blips climbing, like eyes running down a page.
const LOOK: Tune =
    Tune { peak: 0.07, notes: &[note(THIN, E6, 0.0, 0.03), note(THIN, G6, 0.045, 0.03), note(THIN, B6, 0.09, 0.04)] };
/// Editing a file: typewriter keys, then its bell.
const EDIT: Tune = Tune {
    peak: 0.08,
    notes: &[
        note(NOISE, 5000.0, 0.0, 0.018),
        note(NOISE, 3500.0, 0.07, 0.018),
        note(NOISE, 5000.0, 0.14, 0.018),
        note(TRIANGLE, C7, 0.2, 0.14).level(0.8),
    ],
};
/// Running a command: a rising whoosh, and a low thunk as it lands.
const RUN: Tune =
    Tune { peak: 0.08, notes: &[note(NOISE, 1500.0, 0.0, 0.14).slide(9000.0).level(0.6), note(SQUARE, A4, 0.13, 0.07)] };
/// Going online: a signal warbling between two notes.
const WEB: Tune = Tune {
    peak: 0.07,
    notes: &[
        note(TRIANGLE, E6, 0.0, 0.035),
        note(TRIANGLE, B6, 0.035, 0.035),
        note(TRIANGLE, E6, 0.07, 0.035),
        note(TRIANGLE, B6, 0.105, 0.035),
        note(TRIANGLE, E6, 0.14, 0.035),
        note(TRIANGLE, B6, 0.175, 0.05),
    ],
};
/// Handing off to an agent: a call, and its echo answering.
const DELEGATE: Tune = Tune {
    peak: 0.08,
    notes: &[
        note(PULSE, B5, 0.0, 0.05),
        note(PULSE, E6, 0.06, 0.07),
        note(PULSE, B5, 0.17, 0.05).level(0.4),
        note(PULSE, E6, 0.23, 0.07).level(0.4),
    ],
};
/// Planning: two bright notes up, like ticking a box.
const PLAN: Tune = Tune { peak: 0.07, notes: &[note(SQUARE, D6, 0.0, 0.04), note(SQUARE, G6, 0.05, 0.09).level(0.8)] };
/// A step failed or was turned down: a bonk.
const FAIL: Tune =
    Tune { peak: 0.09, notes: &[note(TRIANGLE, G4, 0.0, 0.14).slide(C4), note(NOISE, 800.0, 0.0, 0.03).level(0.5)] };
/// Making room in its context: a long slide down an octave, settling on a low note.
const COMPACT: Tune =
    Tune { peak: 0.07, notes: &[note(PULSE, C6, 0.0, 0.3).slide(C5).vibrato(0.01), note(TRIANGLE, C4, 0.28, 0.08)] };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    Start,
    /// One of the [`THINKING`] tunes.
    Think(usize),
    Done,
    Error,
    Ask,
    Long,
    Look,
    Edit,
    Run,
    Web,
    Delegate,
    Plan,
    Fail,
    Compact,
}

/// A sound as the settings list it, to hear what it means.
pub struct Guide {
    pub sound: Sound,
    pub title: &'static str,
    pub about: &'static str,
    /// It plays in Normal mode too, not just Lively.
    pub normal: bool,
}

const fn guide(sound: Sound, title: &'static str, about: &'static str, normal: bool) -> Guide {
    Guide { sound, title, about, normal }
}

/// Every kind of sound, the ones Normal mode plays first.
pub const GUIDE: [Guide; 14] = [
    guide(Sound::Start, "Work starts", "A quick chirp up", true),
    guide(Sound::Ask, "Claude needs you", "Knock knock, then a rising note: a permission or a question", true),
    guide(Sound::Done, "Work's done", "A little fanfare, once work that took a while ends", true),
    guide(Sound::Error, "Usage can't be read", "A low buzz", true),
    guide(Sound::Think(0), "Thinking", "Soft notes as it thinks, and now and then while it works", false),
    guide(Sound::Look, "Reading or searching", "Quick blips, like eyes running down a page", false),
    guide(Sound::Edit, "Editing a file", "Typewriter keys, then the bell", false),
    guide(Sound::Run, "Running a command", "A whoosh and a thunk, like a launch", false),
    guide(Sound::Web, "Going online", "A warbling signal", false),
    guide(Sound::Delegate, "Handing off to an agent", "A call, and its echo", false),
    guide(Sound::Plan, "Planning", "Two bright notes, like ticking a box", false),
    guide(Sound::Fail, "A step failed or was turned down", "A bonk", false),
    guide(Sound::Compact, "Making room", "A long slide down as it sums up the conversation", false),
    guide(Sound::Long, "Taking a while", "A ticking clock, 2 and 5 minutes in, then every 10", false),
];

impl Sound {
    const ALL: [Sound; 13 + THINKING.len()] = [
        Self::Start,
        Self::Think(0),
        Self::Think(1),
        Self::Think(2),
        Self::Think(3),
        Self::Done,
        Self::Error,
        Self::Ask,
        Self::Long,
        Self::Look,
        Self::Edit,
        Self::Run,
        Self::Web,
        Self::Delegate,
        Self::Plan,
        Self::Fail,
        Self::Compact,
    ];

    /// The sound of a step. Thinking picks one of its tunes with `rng`.
    fn of_step(step: Step, rng: &mut Rng) -> Self {
        match step {
            Step::Think => Self::Think(rng.below(THINKING.len())),
            Step::Look => Self::Look,
            Step::Edit => Self::Edit,
            Step::Run => Self::Run,
            Step::Web => Self::Web,
            Step::Delegate => Self::Delegate,
            Step::Plan => Self::Plan,
            Step::Fail => Self::Fail,
            Step::Compact => Self::Compact,
        }
    }

    /// The sound, with the thinking tunes counted as one.
    fn kind(self) -> Self {
        match self {
            Self::Think(_) => Self::Think(0),
            sound => sound,
        }
    }

    fn tune(self) -> &'static Tune {
        match self {
            Self::Start => &START,
            Self::Think(tune) => &THINKING[tune % THINKING.len()],
            Self::Done => &DONE,
            Self::Error => &ERROR,
            Self::Ask => &ASK,
            Self::Long => &LONG,
            Self::Look => &LOOK,
            Self::Edit => &EDIT,
            Self::Run => &RUN,
            Self::Web => &WEB,
            Self::Delegate => &DELEGATE,
            Self::Plan => &PLAN,
            Self::Fail => &FAIL,
            Self::Compact => &COMPACT,
        }
    }

    /// Where the sound is kept among [`Sound::ALL`]'s.
    fn slot(self) -> usize {
        match self {
            Self::Start => 0,
            Self::Done => 1,
            Self::Error => 2,
            Self::Ask => 3,
            Self::Long => 4,
            Self::Look => 5,
            Self::Edit => 6,
            Self::Run => 7,
            Self::Web => 8,
            Self::Delegate => 9,
            Self::Plan => 10,
            Self::Fail => 11,
            Self::Compact => 12,
            Self::Think(tune) => 13 + tune % THINKING.len(),
        }
    }

    /// The sound as a WAV file, made the first time it's asked for. PlaySound reads it as it
    /// plays, so it's kept for good.
    fn wav(self) -> &'static [u8] {
        static WAVS: [OnceLock<Vec<u8>>; Sound::ALL.len()] = [const { OnceLock::new() }; Sound::ALL.len()];
        WAVS[self.slot()].get_or_init(|| wav(&self.tune().render()))
    }
}

/// Makes every sound on a thread of its own, at low priority so it never holds up the panel,
/// and once only. A sound wanted before then waits for it, or is made there and then.
pub fn prepare() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("sounds".into()).spawn(|| {
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) };
            for sound in Sound::ALL {
                sound.wav();
            }
        });
    });
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

/// A session, told apart from the others by its name and when it started.
type Key = (String, Option<DateTime<Utc>>);

fn key(session: &Session) -> Key {
    (session.name.clone(), session.started)
}

/// Picks the sound, if any, for what Claude's doing. It keeps up with sounds off too, so
/// turning them on mid-work doesn't greet work long under way.
pub struct Cues {
    rng: Rng,
    /// Whether sounds were on at the last look, from the first look on, and in Lively mode.
    on: Option<bool>,
    lively: bool,
    /// The sessions busy, and waiting on you, at the last look.
    busy: Vec<Key>,
    waiting: Vec<Key>,
    /// How many steps each session had taken at the last look.
    steps: HashMap<Key, u32>,
    /// When the work under way started, counting time spent waiting on you, and how many of
    /// its "taking a while" marks have passed.
    working: Option<Instant>,
    marks: usize,
    /// When Claude was last seen working, or seen to have stopped.
    worked: Option<Instant>,
    /// Whether usage couldn't be read at the last look.
    erred: bool,
    /// When the last sound played, the last buzz, and the last of each kind.
    played: Option<Instant>,
    buzzed: Option<Instant>,
    kinds: HashMap<Sound, Instant>,
    /// When the next thinking blip is due, while Claude works in Lively mode.
    blip: Option<Instant>,
}

impl Cues {
    pub fn new() -> Self {
        Self {
            rng: Rng::new(),
            on: None,
            lively: false,
            busy: Vec::new(),
            waiting: Vec::new(),
            steps: HashMap::new(),
            working: None,
            marks: 0,
            worked: None,
            erred: false,
            played: None,
            buzzed: None,
            kinds: HashMap::new(),
            blip: None,
        }
    }

    /// The sound `state` calls for at `now`, if sounds are `on`, in `mode`. Turning them on
    /// plays one straight away, to hear what they're like.
    pub fn update(&mut self, state: &State, on: bool, mode: SoundMode, now: Instant) -> Option<Sound> {
        let since = |t: Option<Instant>| t.map_or(Duration::MAX, |t| now.saturating_duration_since(t));
        let busy: Vec<_> = state.sessions.iter().filter(|s| s.busy).map(key).collect();
        let waiting: Vec<_> = state.sessions.iter().filter(|s| s.waiting).map(key).collect();
        // Waiting on you is still work under way: answering doesn't start it again.
        let engaged = !busy.is_empty() || !waiting.is_empty();
        let started = !busy.is_empty()
            && (self.working.is_none() || busy.iter().any(|s| !self.busy.contains(s) && !self.waiting.contains(s)));
        let asked = waiting.iter().any(|s| !self.waiting.contains(s));
        // The latest step of a session seen to take one since the last look. What sessions did
        // before Cuota first saw them isn't news.
        let stepped = state
            .sessions
            .iter()
            .filter(|s| self.steps.get(&key(s)).is_some_and(|&seen| seen != s.steps))
            .find_map(|s| s.step);
        self.steps = state.sessions.iter().map(|s| (key(s), s.steps)).collect();
        self.busy = busy;
        self.waiting = waiting;
        // How long the work that just ended lasted.
        let ended = if engaged {
            self.working.get_or_insert(now);
            None
        } else {
            self.marks = 0;
            self.working.take().map(|began| now.saturating_duration_since(began))
        };
        // As for Clawd, work ends at the first look that finds it over.
        if engaged || ended.is_some() {
            self.worked = Some(now);
        }
        // Marks passed while waiting on you go by quietly.
        let lasted = since(self.working);
        let mut long = false;
        while self.working.is_some() && lasted >= long_mark(self.marks) {
            self.marks += 1;
            long = !self.busy.is_empty();
        }
        let erred = state.error.is_some() && !self.erred;
        self.erred = state.error.is_some();
        // Trouble is worth a buzz only while Clawd's awake.
        let awake = since(self.worked) < DOZE;
        let hushed = since(self.played) < HUSH;
        let lively = mode == SoundMode::Lively;
        let step = stepped
            .filter(|_| lively && since(self.played) >= STEP_GAP)
            .map(|step| Sound::of_step(step, &mut self.rng))
            .filter(|sound| since(self.kinds.get(&sound.kind()).copied()) >= SAME_STEP);

        let mut sound = if ended.is_some_and(|lasted| lasted >= DONE_AFTER) {
            Some(Sound::Done)
        } else if asked {
            Some(Sound::Ask)
        } else if started && !hushed {
            Some(Sound::Start)
        } else if erred && awake && !hushed && since(self.buzzed) >= BUZZ_AGAIN {
            Some(Sound::Error)
        } else if !lively {
            None
        } else if long {
            Some(Sound::Long)
        } else if step.is_some() {
            step
        } else if !self.busy.is_empty() && self.blip.is_some_and(|due| now >= due) {
            Some(Sound::Think(self.rng.below(THINKING.len())))
        } else {
            None
        };
        if on && self.on == Some(false) {
            sound = sound.or(Some(Sound::Start));
        }
        self.on = Some(on);
        self.lively = lively;
        if let Some(sound) = sound {
            self.played = Some(now);
            self.kinds.insert(sound.kind(), now);
        }
        if sound == Some(Sound::Error) {
            self.buzzed = Some(now);
        }
        // The next blip comes a while after the last sound.
        if !lively || self.busy.is_empty() {
            self.blip = None;
        } else if sound.is_some() || self.blip.is_none() {
            self.blip = Some(now + self.rng.seconds(BLIPS));
        }
        sound.filter(|_| on)
    }

    /// When the next thinking blip or "taking a while" mark is due, while Claude works and
    /// sounds are on in Lively mode.
    pub fn due(&self) -> Option<Instant> {
        if self.on != Some(true) || !self.lively || self.busy.is_empty() {
            return None;
        }
        let mark = self.working.map(|began| began + long_mark(self.marks));
        self.blip.into_iter().chain(mark).min()
    }
}

/// How far into a piece of work its `n`th "taking a while" mark comes: 2 and 5 minutes in,
/// then every 10.
fn long_mark(n: usize) -> Duration {
    let minutes = match n {
        0 => 2,
        1 => 5,
        n => 10 * (n as u64 - 1),
    };
    Duration::from_secs(60 * minutes)
}

#[derive(Clone, Copy)]
enum Wave {
    /// High for this share of each cycle, like the NES's two pulse channels.
    Pulse(f32),
    /// Like its triangle channel: rounder and softer, in 16 steps.
    Triangle,
    /// Like its noise channel: high or low at random, changing at the note's pitch, for
    /// clicks, hisses and whooshes.
    Noise,
}

impl Wave {
    /// Its level at `phase`, from 0 to 1 through the `cycle`th cycle. It averages 0 over a
    /// cycle, or over many for noise, so notes don't thump.
    fn at(self, phase: f32, cycle: u32) -> f32 {
        match self {
            Self::Noise => {
                let hash = cycle.wrapping_mul(0x9E37_79B9) ^ (cycle >> 7);
                if hash & 0x1000 == 0 { 1.0 } else { -1.0 }
            }
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
            let (mut phase, mut cycle) = (0.0, 0);
            let samples = mix.iter_mut().skip((note.at * rate) as usize).take((note.length * rate) as usize);
            for (i, sample) in samples.enumerate() {
                let t = i as f32 / rate;
                let wobble = 1.0 + note.vibrato * (TAU * VIBRATO_HZ * t).sin();
                let hz = note.from * (note.to / note.from).powf(t / note.length) * wobble;
                *sample += note.level * envelope(t, note.length) * note.wave.at(phase, cycle);
                let next: f32 = phase + hz / rate;
                cycle = cycle.wrapping_add(next as u32);
                phase = next.fract();
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
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const SOUNDS: [(&str, Sound); 17] = [
        ("start", Sound::Start),
        ("thinking-1", Sound::Think(0)),
        ("thinking-2", Sound::Think(1)),
        ("thinking-3", Sound::Think(2)),
        ("thinking-4", Sound::Think(3)),
        ("done", Sound::Done),
        ("error", Sound::Error),
        ("ask", Sound::Ask),
        ("long", Sound::Long),
        ("look", Sound::Look),
        ("edit", Sound::Edit),
        ("run", Sound::Run),
        ("web", Sound::Web),
        ("delegate", Sound::Delegate),
        ("plan", Sound::Plan),
        ("fail", Sound::Fail),
        ("compact", Sound::Compact),
    ];
    const NORMAL: SoundMode = SoundMode::Normal;
    const LIVELY: SoundMode = SoundMode::Lively;

    fn session(name: &str, busy: bool, waiting: bool) -> Session {
        Session { name: name.into(), app: "Terminal".into(), busy, waiting, ..Session::default() }
    }

    /// Claude Code with the `busy` sessions working and the `idle` ones open.
    fn state(busy: &[&str], idle: &[&str]) -> State {
        asking(busy, &[], idle)
    }

    /// Claude Code with the `busy` sessions working, the `waiting` ones waiting on you, and the
    /// `idle` ones open.
    fn asking(busy: &[&str], waiting: &[&str], idle: &[&str]) -> State {
        let sessions = busy
            .iter()
            .map(|&name| session(name, true, false))
            .chain(waiting.iter().map(|&name| session(name, false, true)))
            .chain(idle.iter().map(|&name| session(name, false, false)));
        State { working: !busy.is_empty(), sessions: sessions.collect(), ..State::default() }
    }

    /// One session at work, `steps` steps in, the latest of them `step`.
    fn stepping(steps: u32, step: Step) -> State {
        let session = Session { steps, step: Some(step), ..session("a", true, false) };
        State { working: true, sessions: vec![session], ..State::default() }
    }

    fn erring(state: State) -> State {
        State { error: Some("Network error: offline".into()), ..state }
    }

    #[test]
    fn preparing_makes_every_sound_each_in_its_own_slot() {
        let mut slots: Vec<usize> = Sound::ALL.iter().map(|sound| sound.slot()).collect();
        slots.sort();
        assert_eq!(slots, (0..Sound::ALL.len()).collect::<Vec<_>>());
        assert!(SOUNDS.iter().all(|(_, sound)| Sound::ALL.contains(sound)));
        assert!(Sound::ALL.iter().all(|sound| SOUNDS.iter().any(|(_, listed)| listed == sound)));
    }

    #[test]
    fn the_settings_list_every_kind_of_sound_once_normal_ones_first() {
        let kinds: BTreeSet<usize> = GUIDE.iter().map(|guide| guide.sound.kind().slot()).collect();
        let all: BTreeSet<usize> = Sound::ALL.iter().map(|sound| sound.kind().slot()).collect();
        assert_eq!((kinds.len(), &kinds), (GUIDE.len(), &all));
        let normal = GUIDE.iter().take_while(|guide| guide.normal).count();
        assert!(GUIDE[normal..].iter().all(|guide| !guide.normal));
        assert_eq!(normal, 4);
    }

    #[test]
    fn every_sound_is_a_short_quiet_wav_that_starts_and_ends_silent() {
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
    fn noise_is_high_and_low_about_as_often() {
        let high = (0..10_000).filter(|&cycle| Wave::Noise.at(0.0, cycle) > 0.0).count();
        assert!((4_500..5_500).contains(&high), "{high} of 10000 high");
    }

    #[test]
    fn each_sound_is_made_once() {
        assert!(std::ptr::eq(Sound::Done.wav(), Sound::Done.wav()));
        assert!(std::ptr::eq(Sound::Think(1).wav(), Sound::Think(1 + THINKING.len()).wav()));
        assert!(!std::ptr::eq(Sound::Think(0).wav(), Sound::Think(1).wav()));
    }

    #[test]
    fn work_starting_chirps_and_work_done_a_while_later_jingles_once() {
        for mode in [NORMAL, LIVELY] {
            let (mut cues, start) = (Cues::new(), Instant::now());
            let at = |secs| start + Duration::from_secs(secs);
            assert_eq!(cues.update(&state(&[], &["a"]), true, mode, at(0)), None);
            assert_eq!(cues.update(&state(&["a"], &[]), true, mode, at(2)), Some(Sound::Start));
            assert_eq!(cues.update(&state(&["a"], &[]), true, mode, at(4)), None);
            assert_eq!(cues.update(&state(&[], &["a"]), true, mode, at(16)), Some(Sound::Done));
            assert_eq!(cues.update(&state(&[], &["a"]), true, mode, at(18)), None);
            // A quick answer chirps, but doesn't jingle.
            assert_eq!(cues.update(&state(&["a"], &[]), true, mode, at(30)), Some(Sound::Start));
            assert_eq!(cues.update(&state(&[], &["a"]), true, mode, at(36)), None);
        }
    }

    #[test]
    fn another_session_starting_chirps_unless_a_sound_just_played() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&state(&["a"], &[]), true, NORMAL, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&state(&["a", "b"], &[]), true, NORMAL, at(2)), None);
        assert_eq!(cues.update(&state(&["a", "b", "c"], &[]), true, NORMAL, at(10)), Some(Sound::Start));
        // One finishing while others carry on is still work.
        assert_eq!(cues.update(&state(&["a", "c"], &["b"]), true, NORMAL, at(15)), None);
        assert_eq!(cues.update(&state(&["a", "b", "c"], &[]), true, NORMAL, at(18)), Some(Sound::Start));
    }

    #[test]
    fn claude_asking_knocks_and_answering_carries_the_work_on() {
        for mode in [NORMAL, LIVELY] {
            let (mut cues, start) = (Cues::new(), Instant::now());
            let at = |secs| start + Duration::from_secs(secs);
            assert_eq!(cues.update(&state(&["a"], &[]), true, mode, at(0)), Some(Sound::Start));
            // Even hard on the heels of the chirp: it needs you.
            assert_eq!(cues.update(&asking(&[], &["a"], &[]), true, mode, at(2)), Some(Sound::Ask));
            assert_eq!(cues.update(&asking(&[], &["a"], &[]), true, mode, at(40)), None);
            // Answered, it carries on without a chirp, and it all counts when it's done.
            assert_eq!(cues.update(&state(&["a"], &[]), true, mode, at(60)), None);
            assert_eq!(cues.update(&state(&[], &["a"]), true, mode, at(65)), Some(Sound::Done));
            // Asking again later knocks again.
            assert_eq!(cues.update(&asking(&[], &["a"], &[]), true, mode, at(90)), Some(Sound::Ask));
        }
    }

    #[test]
    fn normal_mode_keeps_quiet_through_steps_and_long_work() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let steps = [Step::Think, Step::Look, Step::Edit, Step::Run, Step::Fail];
        for second in 0..=1800 {
            let state = stepping(second, steps[second as usize % steps.len()]);
            let sound = cues.update(&state, true, NORMAL, start + Duration::from_secs(second.into()));
            assert_eq!(sound, (second == 0).then_some(Sound::Start), "at {second} s");
            assert_eq!(cues.due(), None);
        }
    }

    #[test]
    fn lively_mode_sounds_each_kind_of_step_but_not_too_often() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&stepping(0, Step::Think), true, LIVELY, at(0)), Some(Sound::Start));
        // Too soon after the chirp, then each kind as it comes.
        assert_eq!(cues.update(&stepping(1, Step::Look), true, LIVELY, at(2)), None);
        assert_eq!(cues.update(&stepping(2, Step::Look), true, LIVELY, at(4)), Some(Sound::Look));
        assert_eq!(cues.update(&stepping(3, Step::Edit), true, LIVELY, at(6)), None);
        assert_eq!(cues.update(&stepping(4, Step::Edit), true, LIVELY, at(8)), Some(Sound::Edit));
        assert_eq!(cues.update(&stepping(5, Step::Run), true, LIVELY, at(12)), Some(Sound::Run));
        // The same kind again waits a while.
        assert_eq!(cues.update(&stepping(6, Step::Edit), true, LIVELY, at(16)), None);
        assert_eq!(cues.update(&stepping(7, Step::Edit), true, LIVELY, at(20)), Some(Sound::Edit));
        // No new step, no sound.
        assert_eq!(cues.update(&stepping(7, Step::Edit), true, LIVELY, at(30)), None);
        for (n, (step, sound)) in [
            (Step::Web, Sound::Web),
            (Step::Delegate, Sound::Delegate),
            (Step::Plan, Sound::Plan),
            (Step::Fail, Sound::Fail),
            (Step::Compact, Sound::Compact),
        ]
        .into_iter()
        .enumerate()
        {
            let n = n as u64;
            assert_eq!(cues.update(&stepping(8 + n as u32, step), true, LIVELY, at(44 + 4 * n)), Some(sound));
        }
        let think = cues.update(&stepping(20, Step::Think), true, LIVELY, at(70));
        assert!(matches!(think, Some(Sound::Think(tune)) if tune < THINKING.len()), "{think:?}");
    }

    #[test]
    fn steps_taken_before_cuota_saw_a_session_make_no_sound() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&stepping(500, Step::Edit), true, LIVELY, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&stepping(500, Step::Edit), true, LIVELY, at(10)), None);
        assert_eq!(cues.update(&stepping(501, Step::Run), true, LIVELY, at(12)), Some(Sound::Run));
    }

    #[test]
    fn lively_work_ticks_at_its_marks_and_blips_every_so_often() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let working = state(&["a"], &["b"]);
        let mut sounds = Vec::new();
        // Half an hour of work with no steps, looked at every second.
        for second in 0..=1800 {
            let now = start + Duration::from_secs(second);
            if let Some(sound) = cues.update(&working, true, LIVELY, now) {
                sounds.push((second, sound));
            }
            assert!(cues.due().is_some_and(|due| due > now), "the next sound is due at {second} s");
        }
        assert_eq!(sounds[0], (0, Sound::Start));
        let ticks: Vec<u64> = sounds.iter().filter(|(_, sound)| *sound == Sound::Long).map(|(second, _)| *second).collect();
        assert_eq!(ticks, [120, 300, 600, 1200, 1800]);
        for pair in sounds.windows(2) {
            if let Sound::Think(tune) = pair[1].1 {
                let gap = pair[1].0 - pair[0].0;
                assert!(BLIPS.contains(&gap), "{gap} s before a blip");
                assert!(tune < THINKING.len());
            }
        }
        let tune = |sound: &Sound| if let Sound::Think(tune) = *sound { Some(tune) } else { None };
        let tunes: BTreeSet<usize> = sounds.iter().filter_map(|(_, sound)| tune(sound)).collect();
        assert!(tunes.len() >= 3, "only {tunes:?}");
        assert!((31..=96).contains(&sounds.len()), "{} sounds", sounds.len());
    }

    #[test]
    fn nothing_plays_while_claude_is_idle() {
        for mode in [NORMAL, LIVELY] {
            let (mut cues, start) = (Cues::new(), Instant::now());
            for second in (0..7200).step_by(10) {
                let now = start + Duration::from_secs(second);
                assert_eq!(cues.update(&state(&[], &["a", "b"]), true, mode, now), None);
                assert_eq!(cues.due(), None);
            }
            assert_eq!(cues.update(&State::default(), true, mode, start + Duration::from_secs(7200)), None);
        }
    }

    #[test]
    fn with_sounds_off_nothing_plays_and_turning_them_on_plays_one() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        let working = state(&["a"], &[]);
        for second in 0..=100 {
            assert_eq!(cues.update(&working, false, LIVELY, at(second)), None);
            assert_eq!(cues.due(), None);
        }
        assert!(cues.update(&working, true, LIVELY, at(101)).is_some());
        // Work long under way isn't greeted, and no blip follows hard on the heels.
        for second in 102..120 {
            assert_eq!(cues.update(&working, true, LIVELY, at(second)), None);
        }
        assert_eq!(cues.update(&state(&[], &["a"]), false, LIVELY, at(120)), None);
    }

    #[test]
    fn trouble_reading_usage_buzzes_once_while_clawds_awake() {
        let (mut cues, start) = (Cues::new(), Instant::now());
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(cues.update(&state(&["a"], &[]), true, NORMAL, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&erring(state(&["a"], &[])), true, NORMAL, at(10)), Some(Sound::Error));
        assert_eq!(cues.update(&erring(state(&["a"], &[])), true, NORMAL, at(12)), None);
        // Coming and going, it doesn't buzz again soon.
        assert_eq!(cues.update(&state(&["a"], &[]), true, NORMAL, at(14)), None);
        assert_eq!(cues.update(&erring(state(&["a"], &[])), true, NORMAL, at(16)), None);
        // Nobody's around: Claude hasn't worked since Cuota started, or not for a while.
        let mut cues = Cues::new();
        assert_eq!(cues.update(&erring(state(&[], &["a"])), true, NORMAL, at(0)), None);
        let mut cues = Cues::new();
        assert_eq!(cues.update(&state(&["a"], &[]), true, NORMAL, at(0)), Some(Sound::Start));
        assert_eq!(cues.update(&state(&[], &["a"]), true, NORMAL, at(20)), Some(Sound::Done));
        assert_eq!(cues.update(&erring(state(&[], &["a"])), true, NORMAL, at(20) + DOZE), None);
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
