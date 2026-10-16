//! Clawd's animations: the Mascot Studio GIFs in `assets/mascot` (see its NOTICE.md), built
//! into the exe. While Claude works, Clawd acts out a random activity, switching to another
//! whenever a session starts working and every so often in between. He greets when work
//! starts and celebrates when it's done; with nothing to do he waits around a little, then
//! falls asleep.

use crate::claude::State;
use chrono::{DateTime, Utc};
use eframe::egui::{
    Color32, ColorImage, Context, Painter, Pos2, Rect, TextureFilter, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A GIF frame, in pixels. Clawd stands in its lower middle, with room above him for a hat or
/// a thought bubble and beside him for whatever he's working with.
pub const FRAME: Vec2 = vec2(480.0, 440.0);
/// Clawd himself in every frame, arms and legs included, in GIF pixels. His feet are on the
/// same line in every clip, so he stays put from one to the next.
pub const BODY: Rect = Rect::from_min_max(pos2(106.0, 220.0), pos2(364.0, 392.0));
/// Points per GIF pixel: Clawd himself, 172 px tall, comes out about 52 pt.
pub const SCALE: f32 = 0.3;
/// Clawd's slot in the panel, in points. What a clip draws is centred in it, so the text beside
/// him lines up with him whether he has a hat on or not. 380 x 320 px takes everything but
/// the odd flying prop, which may stray up to `SPILL` outside the slot, into the panel's margin.
pub const SIZE: Vec2 = vec2(380.0 * SCALE, 320.0 * SCALE);
pub const SPILL: f32 = 6.0;
/// The GIFs are 2x nearest-neighbour enlargements, so half size loses nothing and saves memory.
const SHRINK: usize = 2;
/// How long one activity shows before another takes over, in seconds, while working...
const WORK_SPELL: Range<u64> = 20..60;
/// ...and while waiting, asleep or in trouble.
const REST_SPELL: Range<u64> = 60..180;
/// Work this long earns a celebration when it ends, and a rest this long a greeting when work
/// resumes; shorter turns just switch clips, so a quick chat isn't all hellos and confetti.
const SPELL_WORTH_MARKING: Duration = Duration::from_secs(45);
/// With no session busy for this long, Clawd falls asleep. Sessions are often left open all
/// day, so he doesn't wait for them to close.
pub const DOZE: Duration = Duration::from_secs(60);
/// Mipmaps keep the pixel art smooth when it's drawn smaller than the frame.
const OPTIONS: TextureOptions = TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR };

struct Clip {
    name: &'static str,
    gif: &'static [u8],
}

macro_rules! clips {
    ($($name:literal),* $(,)?) => {
        [$(Clip { name: $name, gif: include_bytes!(concat!("../assets/mascot/", $name, ".gif")) }),*]
    };
}

/// A session is busy.
const WORKING: &[Clip] = &clips![
    "thinking", "thinking-wizard-hat", "thinking-fluid",
    "using-tool", "using-tool-fluid",
    "directing-helpers", "directing-helpers-fluid",
    "reading", "reading-wizard-hat", "reading-fluid",
    "writing", "writing-sunglasses", "writing-fluid",
    "running-commands", "running-commands-sunglasses", "running-commands-fluid",
    "searching-web", "searching-web-fluid",
    "searching-code", "searching-code-fluid",
    "testing", "testing-fluid",
    "committing", "committing-fluid",
    "compacting", "compacting-fluid",
    "context-nearly-full", "context-nearly-full-fluid",
    "plan-progress", "plan-progress-fluid",
    "server-running", "server-running-fluid",
    "walking", "walking-fluid",
    "gag-hallucinate", "gag-tokens", "gag-context", "gag-prompt",
    "gag-robot", "gag-dance", "gag-buffering", "gag-deploy",
];
/// No session is busy, but one was a moment ago.
const WAITING: &[Clip] = &clips![
    "idle", "idle-wizard-hat", "idle-fluid",
    "waiting-for-you", "waiting-for-you-fluid",
    "typing", "typing-fluid",
    "gag-coffee",
];
/// No session has been busy for a while, or none is open.
const ASLEEP: &[Clip] = &clips!["asleep", "asleep-nightcap", "asleep-fluid", "gag-nap"];
/// Cuota can't read the usage.
const ERROR: &[Clip] = &clips!["error", "error-propeller-cap", "error-fluid"];
/// Played once as work starts...
const GREETING: &[Clip] = &clips!["greeting", "greeting-crown", "greeting-fluid"];
/// ...and once as it ends.
const DONE: &[Clip] = &clips!["done", "done-fluid"];

/// The desktop pet walking from place to place: a brisk scuttle, two steps every 200 ms, or an
/// amble swinging his arms. He only plays them on the move, though the panel's Clawd walks on
/// the spot as one of his activities.
const WALK: &str = "walking";
const AMBLE: &str = "walking-fluid";
/// Telling you it's your turn.
const ASKS: &str = "waiting-for-you";
/// Clips where his eyes are two plain squares in every frame, so they can follow the pointer.
const LOOKERS: &[&str] = &["idle", "idle-wizard-hat", "waiting-for-you", "walking"];
/// Clawd's eyes are solid black squares this many pixels a side at half size (less tall while
/// he blinks). The art moves them an art pixel, `LOOK_STEP` of those, at a time.
const EYE: usize = 11;
pub const LOOK_STEP: i32 = 5;

/// Clawd's status, a word like the one Claude Code's spinner shows. A new one comes
/// with each clip. While a session is busy:
const BUSY_WORDS: &[&str] = &[
    "Accomplishing", "Actioning", "Alchemizing", "Baking", "Bedazzling", "Booping", "Brainstorming",
    "Brewing", "Calculating", "Cerebrating", "Channeling", "Churning", "Clauding", "Coalescing",
    "Cogitating", "Combobulating", "Computing", "Concocting", "Conjuring", "Considering",
    "Contemplating", "Cooking", "Crafting", "Creating", "Crunching", "Deciphering", "Deliberating",
    "Determining", "Discombobulating", "Distilling", "Divining", "Doodling", "Elucidating",
    "Enchanting", "Envisioning", "Fermenting", "Fiddling", "Finagling", "Flibbertigibbeting",
    "Forging", "Frolicking", "Galumphing", "Germinating", "Hatching", "Herding", "Honking",
    "Hustling", "Ideating", "Imagining", "Incubating", "Inferring", "Jiving", "Juggling", "Kneading",
    "Manifesting", "Marinating", "Meandering", "Mosaicing", "Moseying", "Mulling", "Musing",
    "Mustering", "Noodling", "Percolating", "Perusing", "Philosophizing", "Pondering",
    "Pontificating", "Processing", "Pursuing", "Puttering", "Puzzling", "Recombobulating",
    "Reticulating", "Riffing", "Ruminating", "Rummaging", "Scheming", "Schlepping", "Sculpting",
    "Shimmying", "Simmering", "Sleuthing", "Smooshing", "Spelunking", "Spitballing", "Stewing",
    "Sussing", "Synthesizing", "Thinking", "Tinkering", "Transmuting", "Unfurling", "Unraveling",
    "Untangling", "Vibing", "Weaving", "Whirring", "Whittling", "Wibbling", "Wizarding", "Wondering",
    "Working", "Wrangling",
];
/// No session is busy, but one was a moment ago.
const WAITING_WORDS: &[&str] = &[
    "Waiting", "Idling", "Standing by", "Lounging", "Loafing", "Lollygagging", "Dawdling",
    "Daydreaming", "Twiddling thumbs", "Loitering", "Chilling",
];
/// No session has been busy for a while, or none is open.
const ASLEEP_WORDS: &[&str] = &["Sleeping", "Snoozing", "Dozing", "Napping", "Hibernating", "Slumbering", "Snoring"];
/// Cuota can't read the usage.
const ERROR_WORDS: &[&str] = &["Stumped", "Befuddled", "Flummoxed", "Perplexed", "Bamboozled", "Baffled"];
/// Work just ended.
const DONE_WORDS: &[&str] = &["Done", "All done", "Ta-da", "Nailed it", "Finished", "Shipped"];
/// The desktop pet walking somewhere, with no work on.
const WANDER_WORDS: &[&str] =
    &["Wandering", "Strolling", "Moseying", "Scuttling", "Pottering", "Roaming", "Exploring", "Sauntering"];
/// Heading over to the pointer.
const VISIT_WORDS: &[&str] = &["Coming over", "Scuttling over", "On my way", "Heading your way"];
/// Saying hi there.
const HELLO_WORDS: &[&str] = &["Hi", "Hello", "Hey there", "Hiya", "Howdy"];
/// Up from a nap, before finding somewhere else to sleep.
const WAKE_WORDS: &[&str] = &["Waking up", "Yawning", "Stretching"];
/// A session's waiting on you.
const ASK_WORDS: &[&str] = &["Your turn", "Over to you", "Claude needs you", "Your move"];

/// What Clawd's up to, as his mood calls for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Working,
    Waiting,
    Asleep,
    Error,
}

impl Mood {
    /// `worked` is when a session was last busy, if one has been since Cuota started.
    fn of(state: &State, worked: Option<Instant>, now: Instant) -> Self {
        if state.working {
            Self::Working
        } else if state.error.is_some() {
            Self::Error
        } else if state.sessions.is_empty() || worked.is_none_or(|t| now.saturating_duration_since(t) >= DOZE) {
            Self::Asleep
        } else {
            Self::Waiting
        }
    }

    fn clips(self) -> &'static [Clip] {
        match self {
            Self::Working => WORKING,
            Self::Waiting => WAITING,
            Self::Asleep => ASLEEP,
            Self::Error => ERROR,
        }
    }

    fn spell(self) -> Range<u64> {
        if self == Self::Working { WORK_SPELL } else { REST_SPELL }
    }

    /// Words for Clawd's status, and how they end. A clip played `once` outside work is the
    /// celebration as it ends.
    fn words(self, once: bool) -> (&'static [&'static str], &'static str) {
        match self {
            _ if once && self != Self::Working => (DONE_WORDS, "!"),
            Self::Working => (BUSY_WORDS, "…"),
            Self::Waiting => (WAITING_WORDS, "…"),
            Self::Asleep => (ASLEEP_WORDS, "…"),
            Self::Error => (ERROR_WORDS, "…"),
        }
    }
}

/// What the desktop pet does instead of what his mood calls for, while it lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Whatever his mood calls for, as in the panel.
    Mood,
    /// Walking somewhere briskly...
    Walk,
    /// ...or taking his time.
    Amble,
    /// Scuttling over to the pointer.
    Visit,
    /// Saying hi, once.
    Hello,
    /// Waking up, once, before moving somewhere else to sleep.
    Wake,
    /// Telling you Claude's waiting on you.
    Ask,
}

impl Act {
    /// The clips it plays, and whether once. [`Act::Mood`] has none of its own.
    fn clips(self) -> (&'static [Clip], bool) {
        let named = |name: &str| {
            let clip = [WORKING, WAITING].into_iter().flatten().find(|clip| clip.name == name);
            std::slice::from_ref(clip.unwrap_or(&WAITING[0]))
        };
        match self {
            Self::Mood => (&[], false),
            Self::Walk | Self::Visit => (named(WALK), false),
            Self::Amble => (named(AMBLE), false),
            Self::Hello | Self::Wake => (GREETING, true),
            Self::Ask => (named(ASKS), false),
        }
    }

    /// Words for his status while he does it, and how they end. `None` keeps the one he has.
    fn words(self, mood: Mood) -> Option<(&'static [&'static str], &'static str)> {
        match self {
            Self::Mood => Some(mood.words(false)),
            // Busy while he walks, he's still at it.
            Self::Walk | Self::Amble if mood == Mood::Working => None,
            Self::Walk | Self::Amble => Some((WANDER_WORDS, "…")),
            Self::Visit => Some((VISIT_WORDS, "…")),
            Self::Hello => Some((HELLO_WORDS, "!")),
            Self::Wake => Some((WAKE_WORDS, "…")),
            Self::Ask => Some((ASK_WORDS, "!")),
        }
    }
}

pub struct Clawd {
    texture: TextureHandle,
    rng: Rng,
    mood: Mood,
    /// What he's doing instead, as the desktop pet.
    act: Act,
    /// He's the desktop pet, who walks for real rather than on the spot.
    roams: bool,
    /// When the mood last changed.
    since: Instant,
    /// When a session was last busy, if one has been since he started.
    worked: Option<Instant>,
    /// The sessions that were busy at the last paint, by name and start time.
    busy: Vec<(String, Option<DateTime<Utc>>)>,
    playing: Playing,
    /// What he's up to, in a word.
    status: String,
}

impl Clawd {
    pub fn new(ctx: &Context) -> Self {
        Self::with_texture(ctx, "clawd", false)
    }

    /// Clawd as the desktop pet, who walks about for real: his walking clips only play when
    /// [`Self::update`] is given an [`Act`] that walks.
    pub fn roaming(ctx: &Context) -> Self {
        Self::with_texture(ctx, "pet", true)
    }

    fn with_texture(ctx: &Context, name: &str, roams: bool) -> Self {
        let mut rng = Rng::new();
        let now = Instant::now();
        let playing = Playing::new(pick(&mut rng, ASLEEP, "", roams), now, now + rng.seconds(REST_SPELL));
        let blank = ColorImage::filled(playing.frames[0].image.size, Color32::TRANSPARENT);
        let texture = ctx.load_texture(name, blank, OPTIONS);
        let status = status(&mut rng, Mood::Asleep.words(false), "");
        Self {
            texture,
            rng,
            mood: Mood::Asleep,
            act: Act::Mood,
            roams,
            since: now,
            worked: None,
            busy: Vec::new(),
            playing,
            status,
        }
    }

    /// His status, like "Percolating…", and whether it's lively (about work, or saying
    /// something) rather than waiting, sleeping or trouble. It changes with each clip.
    pub fn status(&self) -> (&str, bool) {
        let lively = match self.act {
            Act::Mood => self.mood == Mood::Working || self.playing.once,
            Act::Walk | Act::Amble => self.mood == Mood::Working,
            Act::Visit | Act::Hello | Act::Wake | Act::Ask => true,
        };
        (&self.status, lively)
    }

    /// The mood `state` puts him in now, which [`Self::update`] will play.
    pub fn feeling(&self, state: &State) -> Mood {
        let now = Instant::now();
        let worked = if state.working || self.mood == Mood::Working { Some(now) } else { self.worked };
        Mood::of(state, worked, now)
    }

    /// Whether the clip he played once, for a greeting say, is over.
    pub fn played(&self) -> bool {
        self.playing.once && Instant::now() >= self.playing.ends()
    }

    /// Moves on to another clip, and status, when `state` or `act` calls for one. Call before
    /// laying out, so the panel makes room for the status he paints with. The panel's Clawd
    /// always does what his mood calls for, [`Act::Mood`].
    pub fn update(&mut self, state: &State, animate: bool, act: Act) {
        let now = Instant::now();
        // Work ends at the first update that finds it over, however long ago the last one was.
        if state.working || self.mood == Mood::Working {
            self.worked = Some(now);
        }
        let mood = Mood::of(state, self.worked, now);
        let busy: Vec<_> = state.sessions.iter().filter(|s| s.busy).map(|s| (s.name.clone(), s.started)).collect();
        let started_working = busy.iter().any(|session| !self.busy.contains(session));
        self.busy = busy;

        // Acting, he plays its clip until it's over. His mood waits, so that a greeting or a
        // celebration it calls for still comes afterwards.
        if act != Act::Mood {
            let (pool, once) = act.clips();
            if act != self.act || (!once && now >= self.playing.ends()) {
                self.act = act;
                self.play(pool, once, act.words(self.mood), now);
            }
            return;
        }
        let acted = std::mem::replace(&mut self.act, Act::Mood) != Act::Mood;
        if mood != self.mood {
            let marked = animate && now.saturating_duration_since(self.since) >= SPELL_WORTH_MARKING;
            let (pool, once) = match (self.mood, mood) {
                (Mood::Working, _) if marked => (DONE, true),
                (_, Mood::Working) if marked => (GREETING, true),
                _ => (mood.clips(), false),
            };
            self.mood = mood;
            self.since = now;
            self.play(pool, once, Some(mood.words(once)), now);
        } else if acted || (mood == Mood::Working && started_working && !self.playing.once) || now >= self.playing.ends() {
            self.play(mood.clips(), false, Some(mood.words(false)), now);
        }
    }

    /// Paints Clawd into `rect`, sized by [`SIZE`] times `scale`, and returns when his next
    /// frame is due, or `None` when `animate` is off and he holds still.
    pub fn paint(&mut self, painter: &Painter, rect: Rect, animate: bool, scale: f32) -> Option<Duration> {
        let now = Instant::now();
        let (index, lasts) = self.playing.frame(if animate { now } else { self.playing.started });
        self.show(index, [0, 0]);
        // The clip's drawing centred in the slot, with the frame around it cut off past SPILL.
        let bounds = self.playing.bounds;
        let (zoom, spill) = (SCALE * scale, SPILL * scale);
        let top = rect.center().y - bounds.center().y * zoom;
        let frame = Rect::from_min_size(pos2(rect.center().x - bounds.center().x * zoom, top), FRAME * zoom);
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect.expand(spill)));
        painter.image(self.texture.id(), frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        animate.then(|| lasts.min(self.playing.ends().saturating_duration_since(now)).max(Duration::from_millis(1)))
    }

    /// Paints the whole frame, [`FRAME`] times `zoom` points, with its top-left at `origin`, so
    /// his feet stand on the same spot whatever the clip. His eyes look `look` art pixels
    /// across and down where the clip lets them (see [`LOOKERS`]). Returns when his next frame
    /// is due, as [`Self::paint`] does.
    pub fn paint_frame(
        &mut self,
        painter: &Painter,
        origin: Pos2,
        zoom: f32,
        animate: bool,
        look: [i32; 2],
    ) -> Option<Duration> {
        let now = Instant::now();
        let (index, lasts) = self.playing.frame(if animate { now } else { self.playing.started });
        let look = if LOOKERS.contains(&self.playing.clip.name) { look } else { [0, 0] };
        self.show(index, look);
        let frame = Rect::from_min_size(origin, FRAME * zoom);
        painter.image(self.texture.id(), frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        animate.then(|| lasts.min(self.playing.ends().saturating_duration_since(now)).max(Duration::from_millis(1)))
    }

    /// Where the clip playing draws anything, over all its frames, in GIF pixels: rows of
    /// rectangles, so his window can be cut to his shape and let clicks around him through.
    pub fn outline(&mut self) -> &[Rect] {
        let playing = &mut self.playing;
        playing.outline.get_or_insert_with(|| outline(&playing.frames))
    }

    /// Puts frame `index` of the clip in the texture, with the eyes moved by `look`, unless
    /// it's there already. Each frame keeps its eyes for the last look, which a walk calls for
    /// some 50 times a second.
    fn show(&mut self, index: usize, look: [i32; 2]) {
        let playing = &mut self.playing;
        if playing.shown == Some((index, look)) {
            return;
        }
        let image = &playing.frames[index].image;
        if look == [0, 0] {
            self.texture.set(image.clone(), OPTIONS);
        } else {
            let looked = &mut playing.looked[index];
            if looked.as_ref().is_none_or(|(cached, _)| *cached != look) {
                let shifted = shift_eyes(image, look.map(|step| step * LOOK_STEP)).unwrap_or_else(|| image.clone());
                *looked = Some((look, Arc::new(shifted)));
            }
            if let Some((_, image)) = looked {
                self.texture.set(image.clone(), OPTIONS);
            }
        }
        playing.shown = Some((index, look));
    }

    /// Starts another clip from `pool`: one play if `once`, otherwise looping for a spell. His
    /// status changes to one of `words`, or stays when there are none.
    fn play(&mut self, pool: &'static [Clip], once: bool, words: Option<(&[&str], &str)>, now: Instant) {
        let clip = pick(&mut self.rng, pool, self.playing.clip.name, self.roams);
        let due = if once { now } else { now + self.rng.seconds(self.mood.spell()) };
        self.playing = Playing::new(clip, now, due);
        self.playing.once = once;
        if let Some(words) = words {
            self.status = status(&mut self.rng, words, &self.status);
        }
    }
}

/// A random clip from `pool`, another than `current` when the pool has one. When `roams`, the
/// walking clips are left for his actual walks.
fn pick(rng: &mut Rng, pool: &'static [Clip], current: &str, roams: bool) -> &'static Clip {
    let walking = |clip: &Clip| roams && [WALK, AMBLE].contains(&clip.name) && pool.len() > 2;
    loop {
        let clip = &pool[rng.below(pool.len())];
        if (pool.len() == 1 || clip.name != current) && !walking(clip) {
            return clip;
        }
    }
}

/// A random status from `words` and how they end, another than `current`.
fn status(rng: &mut Rng, (words, end): (&[&str], &str), current: &str) -> String {
    loop {
        let status = format!("{}{end}", words[rng.below(words.len())]);
        if words.len() == 1 || status != current {
            return status;
        }
    }
}

struct Frame {
    image: ColorImage,
    /// When it ends, from the start of the clip.
    end: Duration,
}

struct Playing {
    clip: &'static Clip,
    frames: Vec<Frame>,
    length: Duration,
    /// The smallest rectangle, in GIF pixels, holding everything the frames draw.
    bounds: Rect,
    started: Instant,
    /// When another clip is due. It takes over at the end of the loop running then, so `started`
    /// means after one play.
    due: Instant,
    once: bool,
    /// The frame in the texture, and where its eyes look.
    shown: Option<(usize, [i32; 2])>,
    /// Each frame with its eyes moved, for the last look it was shown with.
    looked: Vec<Option<([i32; 2], Arc<ColorImage>)>>,
    /// Where it draws, once asked: see [`Clawd::outline`].
    outline: Option<Vec<Rect>>,
}

impl Playing {
    fn new(clip: &'static Clip, started: Instant, due: Instant) -> Self {
        Self::with_frames(clip, decode(clip.gif), started, due)
    }

    fn with_frames(clip: &'static Clip, mut frames: Vec<Frame>, started: Instant, due: Instant) -> Self {
        if frames.is_empty() {
            let image = ColorImage::filled([480 / SHRINK, 440 / SHRINK], Color32::TRANSPARENT);
            frames.push(Frame { image, end: Duration::from_secs(1) });
        }
        let length = frames.last().map_or(Duration::ZERO, |f| f.end);
        let bounds = bounds(&frames);
        let looked = vec![None; frames.len()];
        Self { clip, frames, length, bounds, started, due, once: false, shown: None, looked, outline: None }
    }

    /// When the clip gives way: the end of the loop running when another is due.
    fn ends(&self) -> Instant {
        let waited = self.due.saturating_duration_since(self.started).as_secs_f64();
        let loops = (waited / self.length.as_secs_f64()).ceil().max(1.0);
        self.started + self.length.mul_f64(loops)
    }

    /// The frame showing at `now`, and how much longer it shows.
    fn frame(&self, now: Instant) -> (usize, Duration) {
        let position = now.saturating_duration_since(self.started).as_secs_f64() % self.length.as_secs_f64();
        let position = Duration::from_secs_f64(position);
        let index = self.frames.partition_point(|f| f.end <= position).min(self.frames.len() - 1);
        (index, self.frames[index].end.saturating_sub(position))
    }
}

/// Decodes a GIF into half-size frames with their end times. A frame usually holds just the
/// part that changed, so each is composited onto a canvas, after clearing the area the
/// previous frame asked to have restored, and only the area touched is shrunk again.
fn decode(gif: &[u8]) -> Vec<Frame> {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::RGBA);
    let Ok(mut decoder) = options.read_info(gif) else { return Vec::new() };
    let (width, height) = (usize::from(decoder.width()), usize::from(decoder.height()));
    let size = [width / SHRINK, height / SHRINK];
    let mut canvas = vec![0u8; width * height * 4];
    let mut small = vec![Color32::TRANSPARENT; size[0] * size[1]];
    let mut restore: Option<Area> = None;
    let mut frames = Vec::new();
    let mut end = Duration::ZERO;
    while let Ok(Some(frame)) = decoder.read_next_frame() {
        let (left, top) = (usize::from(frame.left), usize::from(frame.top));
        let stride = usize::from(frame.width);
        // Clipped to the canvas, in case the file lies.
        let area = Area {
            left,
            top,
            right: (left + stride).min(width),
            bottom: (top + usize::from(frame.height)).min(height),
        };
        let mut changed = area;
        if let Some(gone) = restore.take() {
            for row in canvas.chunks_exact_mut(width * 4).take(gone.bottom).skip(gone.top) {
                row[gone.left * 4..gone.right * 4].fill(0);
            }
            changed = changed.union(gone);
        }
        let (w, h) = (area.right.saturating_sub(left), area.bottom.saturating_sub(top));
        if w > 0 {
            for (y, row) in frame.buffer.chunks_exact(stride * 4).enumerate().take(h) {
                let dest = &mut canvas[((top + y) * width + left) * 4..][..w * 4];
                for (d, s) in dest.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                    if s[3] != 0 {
                        d.copy_from_slice(s);
                    }
                }
            }
        }
        if frame.dispose == DisposalMethod::Background {
            restore = Some(area);
        }
        shrink(&canvas, width, changed, &mut small, size[0]);
        // Players treat no delay as a tenth of a second.
        let delay = if frame.delay == 0 { 10 } else { frame.delay };
        end += Duration::from_millis(u64::from(delay) * 10);
        frames.push(Frame { image: ColorImage::new(size, small.clone()), end });
    }
    frames
}

/// The smallest rectangle holding every opaque pixel of `frames`, in GIF pixels: the whole frame
/// when they draw nothing.
fn bounds(frames: &[Frame]) -> Rect {
    let mut drawn = Rect::NOTHING;
    for frame in frames {
        let [width, _] = frame.image.size;
        for (y, row) in frame.image.pixels.chunks_exact(width).enumerate() {
            let mut opaque = row.iter().enumerate().filter(|(_, p)| p.a() > 0).map(|(x, _)| x as f32);
            if let Some(first) = opaque.next() {
                let last = opaque.next_back().unwrap_or(first);
                drawn = drawn.union(Rect::from_min_max(pos2(first, y as f32), pos2(last + 1.0, y as f32 + 1.0)));
            }
        }
    }
    if drawn.is_positive() { drawn * SHRINK as f32 } else { Rect::from_min_size(pos2(0.0, 0.0), FRAME) }
}

/// Where any of `frames` draws, in GIF pixels: for each band of `CELL` half-size rows, the runs
/// of `CELL`-wide columns touched, as rectangles.
fn outline(frames: &[Frame]) -> Vec<Rect> {
    const CELL: usize = 4;
    let Some([width, height]) = frames.first().map(|f| f.image.size) else { return Vec::new() };
    let (columns, rows) = (width.div_ceil(CELL), height.div_ceil(CELL));
    let mut touched = vec![false; columns * rows];
    for frame in frames {
        for (i, pixel) in frame.image.pixels.iter().enumerate() {
            if pixel.a() > 0 {
                touched[(i / width / CELL) * columns + i % width / CELL] = true;
            }
        }
    }
    let cell = (CELL * SHRINK) as f32;
    let mut rects = Vec::new();
    for (row, cells) in touched.chunks_exact(columns).enumerate() {
        let mut column = 0;
        while column < columns {
            if !cells[column] {
                column += 1;
                continue;
            }
            let start = column;
            while column < columns && cells[column] {
                column += 1;
            }
            let min = pos2(start as f32 * cell, row as f32 * cell);
            rects.push(Rect::from_min_max(min, pos2(column as f32 * cell, min.y + cell)));
        }
    }
    rects
}

/// `image` (half size) with Clawd's eyes looking `dx` across and `dy` down, in its pixels:
/// across from the middle of his face, as his own art looks from there, so eyes a clip draws
/// off to one side look where they're told too. `None` when it has no two eyes to move or
/// they'd land on something other than his face: a prop over them, or the edge of his head.
fn shift_eyes(image: &ColorImage, [dx, dy]: [i32; 2]) -> Option<ColorImage> {
    if dx == 0 && dy == 0 {
        return None;
    }
    let [width, height] = image.size;
    let dark = |p: Color32| p.a() > 200 && p.r() < 40 && p.g() < 40 && p.b() < 40;
    // Each patch of dark pixels: its bounds, and how many it has, to tell solid squares.
    let mut seen = vec![false; width * height];
    let mut eyes = Vec::new();
    for start in 0..width * height {
        if seen[start] || !dark(image.pixels[start]) {
            continue;
        }
        seen[start] = true;
        let (mut stack, mut count) = (vec![start], 0);
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
        while let Some(i) = stack.pop() {
            count += 1;
            let (x, y) = (i % width, i / width);
            (left, top, right, bottom) = (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1));
            let (left, right) = ((x > 0).then(|| i - 1), (x + 1 < width).then(|| i + 1));
            let (up, down) = ((y > 0).then(|| i - width), (y + 1 < height).then(|| i + width));
            for j in [left, right, up, down].into_iter().flatten() {
                if !seen[j] && dark(image.pixels[j]) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let (w, h) = (right - left, bottom - top);
        let eye = count == w * h && (EYE - 1..=EYE + 1).contains(&w) && (3..=EYE + 1).contains(&h);
        if eye {
            eyes.push([left, top, right, bottom]);
        } else if w >= EYE - 1 && h >= 3 {
            // Something bigger and dark, like sunglasses: leave the eyes be.
            return None;
        }
    }
    let [first, second] = eyes[..] else { return None };
    if first[1] != second[1] || first[0] == 0 {
        return None;
    }
    let face = image.pixels[first[1] * width + first[0] - 1];
    if face.a() < 255 || dark(face) {
        return None;
    }
    let in_eye = |x: usize, y: usize| eyes.iter().any(|e| (e[0]..e[2]).contains(&x) && (e[1]..e[3]).contains(&y));
    // His face's edges along the top of his eyes, to centre them between.
    let on_face = |x: usize| in_eye(x, first[1]) || image.pixels[first[1] * width + x] == face;
    let (mut left_edge, mut right_edge) = (first[0], second[2]);
    while left_edge > 0 && on_face(left_edge - 1) {
        left_edge -= 1;
    }
    while right_edge < width && on_face(right_edge) {
        right_edge += 1;
    }
    let dx = dx + ((left_edge + right_edge) as i32 - (first[0] + second[2]) as i32) / 2;
    if dx == 0 && dy == 0 {
        return Some(image.clone());
    }
    let moved = |[left, top, right, bottom]: [usize; 4]| {
        let shift = |v: usize, d: i32| usize::try_from(v as i32 + d).ok();
        Some([shift(left, dx)?, shift(top, dy)?, shift(right, dx)?, shift(bottom, dy)?])
    };
    let targets = [moved(first)?, moved(second)?];
    for [left, top, right, bottom] in targets {
        if right > width || bottom > height {
            return None;
        }
        for y in top..bottom {
            for x in left..right {
                if !in_eye(x, y) && image.pixels[y * width + x] != face {
                    return None;
                }
            }
        }
    }
    let mut shifted = image.clone();
    for [left, top, right, bottom] in eyes {
        for y in top..bottom {
            shifted.pixels[y * width + left..y * width + right].fill(face);
        }
    }
    for [left, top, right, bottom] in targets {
        for y in top..bottom {
            shifted.pixels[y * width + left..y * width + right].fill(Color32::BLACK);
        }
    }
    Some(shifted)
}

/// Part of the canvas, in pixels; `right` and `bottom` are just outside it.
#[derive(Clone, Copy)]
struct Area {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl Area {
    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// Brings `small`, the `SHRINK` times smaller copy of `canvas`, up to date within `area`. Each
/// block is averaged with colours weighted by alpha, so transparent pixels don't darken edges.
fn shrink(canvas: &[u8], width: usize, area: Area, small: &mut [Color32], small_width: usize) {
    let rows = area.top / SHRINK..area.bottom.div_ceil(SHRINK).min(small.len() / small_width);
    let cols = area.left / SHRINK..area.right.div_ceil(SHRINK).min(small_width);
    for by in rows {
        for bx in cols.clone() {
            let first = (by * SHRINK * width + bx * SHRINK) * 4;
            let pixel = &canvas[first..first + 4];
            // Nearly every block is one enlarged pixel.
            let uniform = (0..SHRINK).all(|dy| {
                let row = first + dy * width * 4;
                canvas[row..row + SHRINK * 4].chunks_exact(4).all(|p| p == pixel)
            });
            let (mut rgb, mut alpha) = ([0u32; 3], 0u32);
            if uniform {
                rgb = [pixel[0], pixel[1], pixel[2]].map(|c| u32::from(c) * u32::from(pixel[3]) * (SHRINK * SHRINK) as u32);
                alpha = u32::from(pixel[3]) * (SHRINK * SHRINK) as u32;
            } else {
                for dy in 0..SHRINK {
                    for dx in 0..SHRINK {
                        let i = first + (dy * width + dx) * 4;
                        let a = u32::from(canvas[i + 3]);
                        for (sum, &channel) in rgb.iter_mut().zip(&canvas[i..i + 3]) {
                            *sum += u32::from(channel) * a;
                        }
                        alpha += a;
                    }
                }
            }
            small[by * small_width + bx] = if alpha == 0 {
                Color32::TRANSPARENT
            } else {
                let [r, g, b] = rgb.map(|c| (c / alpha) as u8);
                Color32::from_rgba_unmultiplied(r, g, b, (alpha / (SHRINK * SHRINK) as u32) as u8)
            };
        }
    }
}

/// xorshift64*, seeded from the clock: plenty for picking animations.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Self((nanos ^ (u64::from(std::process::id()) << 32)) | 1)
    }

    /// The same picks every time, for tests.
    #[cfg(test)]
    pub fn seeded(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn seconds(&mut self, range: Range<u64>) -> Duration {
        Duration::from_secs(range.start + self.next() % (range.end - range.start))
    }

    /// Somewhere in `range`.
    pub fn between(&mut self, range: Range<f32>) -> f32 {
        let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        range.start + unit * (range.end - range.start)
    }

    /// True this often, from 0 (never) to 1 (always).
    pub fn chance(&mut self, odds: f32) -> bool {
        self.between(0.0..1.0) < odds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::Session;
    use std::collections::BTreeSet;

    const POOLS: [&[Clip]; 6] = [WORKING, WAITING, ASLEEP, ERROR, GREETING, DONE];

    #[test]
    fn every_mascot_studio_clip_is_in_one_pool_and_decodes() {
        let names: BTreeSet<&str> = POOLS.iter().flat_map(|pool| pool.iter().map(|c| c.name)).collect();
        assert_eq!(names.len(), 62);
        assert_eq!(names.len(), POOLS.iter().map(|pool| pool.len()).sum::<usize>(), "a clip is in two pools");
        // Side by side: debug builds take a while over 1,200 frames.
        std::thread::scope(|scope| {
            for clip in POOLS.iter().flat_map(|pool| pool.iter()) {
                scope.spawn(move || {
                    let frames = decode(clip.gif);
                    assert!(!frames.is_empty(), "{} has no frames", clip.name);
                    assert_eq!(frames[0].image.size, [240, 220], "{}", clip.name);
                    let length = frames.last().unwrap().end;
                    assert!(length >= Duration::from_millis(200), "{} lasts {length:?}", clip.name);
                    assert!(frames.iter().all(|f| f.end > Duration::ZERO));
                    let drawn = frames.iter().any(|f| f.image.pixels.iter().any(|p| p.a() > 0));
                    assert!(drawn, "{} is blank", clip.name);
                    // Clawd himself is 172 px tall; the rest is props, which the slot has room for.
                    let drawn = bounds(&frames);
                    assert!(Rect::from_min_size(pos2(0.0, 0.0), FRAME).contains_rect(drawn), "{} {drawn:?}", clip.name);
                    assert!(drawn.height() >= 170.0, "{} draws only {drawn:?}", clip.name);
                    // What shows at once: the slot plus the spill either side. Only things on
                    // their way off the frame's own edge, like confetti, lose a little more.
                    let room = SIZE / SCALE + Vec2::splat(2.0 * SPILL / SCALE);
                    let lost = (drawn.size() - room).max(Vec2::ZERO) / 2.0 * SCALE;
                    assert!(lost.x <= 10.0 && lost.y <= 10.0, "{} loses {lost:?} pt a side", clip.name);
                });
            }
        });
    }

    #[test]
    fn bounds_hold_every_frames_opaque_pixels_at_gif_size() {
        let frame = |x: usize, y: usize| {
            let mut image = ColorImage::filled([4, 4], Color32::TRANSPARENT);
            image[(x, y)] = Color32::RED;
            Frame { image, end: Duration::from_secs(1) }
        };
        assert_eq!(bounds(&[frame(1, 1), frame(3, 2)]), Rect::from_min_max(pos2(2.0, 2.0), pos2(8.0, 6.0)));
        let blank = Frame { image: ColorImage::filled([4, 4], Color32::TRANSPARENT), end: Duration::from_secs(1) };
        assert_eq!(bounds(&[blank]), Rect::from_min_size(pos2(0.0, 0.0), FRAME));
    }

    #[test]
    fn clawd_without_props_fits_the_slot() {
        let drawn = bounds(&decode(clips!["idle"][0].gif));
        assert!(drawn.width() * SCALE <= SIZE.x && drawn.height() * SCALE <= SIZE.y, "{drawn:?}");
    }

    #[test]
    fn status_words_suit_the_mood_and_change_with_each_clip() {
        let lists = [BUSY_WORDS, WAITING_WORDS, ASLEEP_WORDS, ERROR_WORDS, DONE_WORDS];
        for words in lists.into_iter().chain([WANDER_WORDS, VISIT_WORDS, HELLO_WORDS, WAKE_WORDS, ASK_WORDS]) {
            let unique: BTreeSet<&str> = words.iter().copied().collect();
            assert_eq!(unique.len(), words.len(), "a word is in twice");
            assert!(words.iter().all(|w| !w.is_empty() && w.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '-')));
        }
        let mut rng = Rng::new();
        let mut last = String::new();
        for _ in 0..200 {
            let next = status(&mut rng, Mood::Working.words(false), &last);
            assert_ne!(next, last);
            assert!(next.ends_with('…') && BUSY_WORDS.contains(&next.trim_end_matches('…')), "{next}");
            last = next;
        }
        // The greeting is work starting; a clip played once otherwise is work done.
        assert!(BUSY_WORDS.contains(&status(&mut rng, Mood::Working.words(true), "").trim_end_matches('…')));
        let done = status(&mut rng, Mood::Waiting.words(true), "");
        assert!(DONE_WORDS.contains(&done.trim_end_matches('!')), "{done}");
        assert!(ASLEEP_WORDS.contains(&status(&mut rng, Mood::Asleep.words(false), "").trim_end_matches('…')));
        // The desktop pet's acts: hello says hi, walking at work keeps the busy word.
        let hello = status(&mut rng, Act::Hello.words(Mood::Waiting).unwrap(), "");
        assert!(HELLO_WORDS.contains(&hello.trim_end_matches('!')), "{hello}");
        assert_eq!(Act::Walk.words(Mood::Working), None);
        assert_eq!(Act::Amble.words(Mood::Asleep).unwrap().0, WANDER_WORDS);
        // Your turn: waiting for you, in so many words.
        assert_eq!(Act::Ask.words(Mood::Waiting), Some((ASK_WORDS, "!")));
        assert_eq!(Act::Ask.clips().0[0].name, "waiting-for-you");
        assert!(LOOKERS.contains(&ASKS), "his eyes find you as he asks");
    }

    #[test]
    fn picks_another_clip_when_the_pool_has_one() {
        let mut rng = Rng::new();
        for _ in 0..200 {
            assert_ne!(pick(&mut rng, WORKING, "thinking", false).name, "thinking");
        }
        assert_eq!(pick(&mut rng, &clips!["idle"], "idle", false).name, "idle");
        let picked: BTreeSet<&str> = (0..500).map(|_| pick(&mut rng, ASLEEP, "", false).name).collect();
        assert_eq!(picked.len(), ASLEEP.len(), "every clip gets a turn");
        // The panel's Clawd walks on the spot now and then; the desktop pet only on the move.
        let mut walks = |roams| (0..2000).filter(|_| [WALK, AMBLE].contains(&pick(&mut rng, WORKING, "", roams).name)).count();
        assert!(walks(false) > 0);
        assert_eq!(walks(true), 0);
        assert_eq!(pick(&mut rng, Act::Walk.clips().0, WALK, true).name, WALK);
        assert_eq!(pick(&mut rng, Act::Amble.clips().0, "", true).name, AMBLE);
    }

    #[test]
    fn eyes_follow_the_pointer_an_art_pixel_at_a_time_in_every_frame_of_the_clips_that_look() {
        std::thread::scope(|scope| {
            for clip in POOLS.iter().flat_map(|pool| pool.iter()).filter(|clip| LOOKERS.contains(&clip.name)) {
                scope.spawn(move || {
                    for (i, frame) in decode(clip.gif).iter().enumerate() {
                        // Every way pet::eyes looks: level when far to the side.
                        let looks = [[-2, 0], [2, 0], [-1, -1], [-1, 0], [-1, 1], [0, -1], [0, 1], [1, -1], [1, 0], [1, 1]];
                        for look in looks {
                            let step = look.map(|s| s * LOOK_STEP);
                            let moved = shift_eyes(&frame.image, step);
                            let moved = moved.unwrap_or_else(|| panic!("{} frame {i} can't look {look:?}", clip.name));
                            // Only the eyes moved: as much black as before, the same outline.
                            let black = |image: &ColorImage| image.pixels.iter().filter(|&&p| p == Color32::BLACK).count();
                            assert_eq!(black(&moved), black(&frame.image), "{} frame {i}", clip.name);
                            let changed = moved.pixels.iter().zip(&frame.image.pixels).filter(|(a, b)| a != b).count();
                            assert!(changed <= 4 * EYE * EYE, "{} frame {i} changed {changed}", clip.name);
                            assert!(moved.pixels.iter().zip(&frame.image.pixels).all(|(a, b)| (a.a() == 0) == (b.a() == 0)));
                        }
                        assert!(shift_eyes(&frame.image, [0, 0]).is_none());
                    }
                });
            }
        });
        // Told the same look, eyes that clips draw in different places end up in the same place.
        let left_eye = |image: &ColorImage| (0..image.size[0]).find(|&x| image.pixels[126 * image.size[0] + x] == Color32::BLACK);
        let clip = |name| POOLS.iter().flat_map(|pool| pool.iter()).find(|clip| clip.name == name).unwrap();
        let look = |name| shift_eyes(&decode(clip(name).gif)[0].image, [LOOK_STEP, 0]).unwrap();
        assert_eq!(left_eye(&look("idle")), Some(90));
        assert_eq!(left_eye(&look("walking")), Some(90));
        // Sunglasses aren't eyes, and eyes can't look off his face.
        let shades = decode(clips!["writing-sunglasses"][0].gif);
        assert!(shift_eyes(&shades[0].image, [LOOK_STEP, 0]).is_none());
        let idle = decode(clips!["idle"][0].gif);
        assert!(shift_eyes(&idle[0].image, [12 * LOOK_STEP, 0]).is_none());
    }

    #[test]
    fn the_outline_covers_everything_a_clip_draws_and_little_else() {
        for clip in [&clips!["idle"][0], &clips!["walking"][0], &clips!["gag-coffee"][0]] {
            let frames = decode(clip.gif);
            let outline = outline(&frames);
            for frame in &frames {
                let [width, _] = frame.image.size;
                for (i, _) in frame.image.pixels.iter().enumerate().filter(|(_, p)| p.a() > 0) {
                    let at = pos2(((i % width) * SHRINK) as f32 + 0.5, ((i / width) * SHRINK) as f32 + 0.5);
                    assert!(outline.iter().any(|r| r.contains(at)), "{} draws outside its outline at {at:?}", clip.name);
                }
            }
            let area: f32 = outline.iter().map(|r| r.area()).sum();
            assert!(area < 0.5 * FRAME.x * FRAME.y, "{} covers {area}", clip.name);
            assert!(outline.iter().all(|r| r.min.x >= 0.0 && r.max.y <= FRAME.y + 8.0));
        }
        // Clawd's body sits inside every outline.
        let idle = outline(&decode(clips!["idle"][0].gif));
        assert!(idle.iter().any(|r| r.contains(BODY.center())));
    }

    #[test]
    fn clawds_feet_stand_on_the_same_line_whatever_he_does() {
        for clip in [&clips!["idle"][0], &clips!["walking"][0], &clips!["asleep"][0], &clips!["thinking"][0]] {
            let drawn = bounds(&decode(clip.gif));
            // Lying down, he spreads a little lower.
            assert!((drawn.bottom() - BODY.bottom()).abs() <= 12.0, "{} ends at {}", clip.name, drawn.bottom());
        }
        // Without props, BODY is all of him.
        assert!(BODY.contains_rect(bounds(&decode(clips!["idle"][0].gif))));
        assert!(BODY.contains_rect(bounds(&decode(clips!["walking"][0].gif))));
    }

    #[test]
    fn a_clip_gives_way_at_the_end_of_a_loop() {
        let start = Instant::now();
        let frames = |ends: &[u64]| {
            ends.iter()
                .map(|&ms| Frame { image: ColorImage::filled([2, 2], Color32::RED), end: Duration::from_millis(ms) })
                .collect()
        };
        let looping = Playing::with_frames(&WORKING[0], frames(&[1000, 2000]), start, start + Duration::from_secs(3));
        assert_eq!(looping.ends(), start + Duration::from_secs(4));
        assert_eq!(looping.frame(start + Duration::from_millis(2500)), (0, Duration::from_millis(500)));
        assert_eq!(looping.frame(start + Duration::from_millis(1000)), (1, Duration::from_millis(1000)));
        let once = Playing::with_frames(&DONE[0], frames(&[500, 2000]), start, start);
        assert_eq!(once.ends(), start + Duration::from_secs(2));
        let empty = Playing::with_frames(&DONE[0], Vec::new(), start, start);
        assert_eq!(empty.frame(start + Duration::from_secs(5)).0, 0);
    }

    #[test]
    fn mood_follows_the_sessions() {
        let session = |busy| Session { name: "x".into(), app: "VS Code".into(), busy, ..Session::default() };
        let now = Instant::now();
        let just = Some(now);
        let state = State { working: true, sessions: vec![session(true)], error: Some("down".into()), ..State::default() };
        assert_eq!(Mood::of(&state, None, now), Mood::Working);
        let state = State { sessions: vec![session(false)], error: Some("down".into()), ..State::default() };
        assert_eq!(Mood::of(&state, None, now), Mood::Error);
        let state = State { sessions: vec![session(false)], ..State::default() };
        assert_eq!(Mood::of(&state, just, now), Mood::Waiting);
        assert_eq!(Mood::of(&state, just, now + DOZE - Duration::from_secs(1)), Mood::Waiting);
        assert_eq!(Mood::of(&State::default(), just, now), Mood::Asleep);
    }

    #[test]
    fn the_desktop_pet_acts_until_told_to_follow_his_mood_again() {
        let ctx = Context::default();
        let mut clawd = Clawd::roaming(&ctx);
        let resting = State::default();
        clawd.update(&resting, true, Act::Amble);
        assert_eq!(clawd.playing.clip.name, AMBLE);
        assert!(WANDER_WORDS.contains(&clawd.status().0.trim_end_matches('…')), "{}", clawd.status().0);
        clawd.update(&resting, true, Act::Hello);
        assert!(GREETING.iter().any(|clip| clip.name == clawd.playing.clip.name) && clawd.playing.once);
        assert!(!clawd.played(), "a greeting takes a moment");
        assert!(clawd.status().1, "saying hi is lively");
        // Back to his mood: asleep, with nothing going on.
        clawd.update(&resting, true, Act::Mood);
        assert!(ASLEEP.iter().any(|clip| clip.name == clawd.playing.clip.name), "{}", clawd.playing.clip.name);
        assert!(ASLEEP_WORDS.contains(&clawd.status().0.trim_end_matches('…')));

        // At work, he keeps his busy word while he walks, and never walks on the spot.
        let session = Session { name: "x".into(), app: "VS Code".into(), busy: true, ..Session::default() };
        let working = State { working: true, sessions: vec![session], ..State::default() };
        assert_eq!(clawd.feeling(&working), Mood::Working);
        clawd.update(&working, false, Act::Mood);
        let word = clawd.status().0.to_owned();
        assert!(BUSY_WORDS.contains(&word.trim_end_matches('…')));
        clawd.update(&working, false, Act::Walk);
        assert_eq!((clawd.playing.clip.name, clawd.status()), (WALK, (word.as_str(), true)));
        for _ in 0..50 {
            clawd.update(&working, false, Act::Mood);
            clawd.playing.due = Instant::now();
            clawd.playing.started = Instant::now() - Duration::from_secs(60);
            assert!(![WALK, AMBLE].contains(&clawd.playing.clip.name));
        }
    }

    #[test]
    fn clawd_falls_asleep_when_nothing_happens_though_sessions_are_open() {
        let session = Session { name: "x".into(), app: "VS Code".into(), ..Session::default() };
        let state = State { sessions: vec![session], ..State::default() };
        let now = Instant::now();
        assert_eq!(Mood::of(&state, Some(now), now + DOZE), Mood::Asleep);
        // Nothing has worked since Cuota started.
        assert_eq!(Mood::of(&state, None, now), Mood::Asleep);
        let (words, _) = Mood::Asleep.words(false);
        assert!(words.contains(&"Sleeping"));
    }
}
