//! Clawd's animations: the Mascot Studio GIFs in `assets/mascot` (see its NOTICE.md), built
//! into the exe. While Claude works, Clawd acts out a random activity, switching to another
//! whenever a session starts working and every so often in between. He greets when work
//! starts and celebrates when it's done; with nothing to do he waits around, or sleeps when
//! no session is open.

use crate::claude::State;
use chrono::{DateTime, Utc};
use eframe::egui::{
    Color32, ColorImage, Context, Painter, Rect, TextureFilter, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use std::ops::Range;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A GIF frame, in pixels. Clawd stands in its lower middle, with room above him for a hat or
/// a thought bubble and beside him for whatever he's working with.
const FRAME: Vec2 = vec2(480.0, 440.0);
/// Points per GIF pixel: Clawd himself, 172 px tall, comes out about 52 pt.
const SCALE: f32 = 0.3;
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
/// Sessions are open, but none is busy.
const WAITING: &[Clip] = &clips![
    "idle", "idle-wizard-hat", "idle-fluid",
    "waiting-for-you", "waiting-for-you-fluid",
    "typing", "typing-fluid",
    "gag-coffee",
];
/// No session is open.
const ASLEEP: &[Clip] = &clips!["asleep", "asleep-nightcap", "asleep-fluid", "gag-nap"];
/// Cuota can't read the usage.
const ERROR: &[Clip] = &clips!["error", "error-propeller-cap", "error-fluid"];
/// Played once as work starts...
const GREETING: &[Clip] = &clips!["greeting", "greeting-crown", "greeting-fluid"];
/// ...and once as it ends.
const DONE: &[Clip] = &clips!["done", "done-fluid"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mood {
    Working,
    Waiting,
    Asleep,
    Error,
}

impl Mood {
    fn of(state: &State) -> Self {
        if state.working {
            Self::Working
        } else if state.error.is_some() {
            Self::Error
        } else if state.sessions.is_empty() {
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
}

pub struct Clawd {
    texture: TextureHandle,
    rng: Rng,
    mood: Mood,
    /// When the mood last changed.
    since: Instant,
    /// The sessions that were busy at the last paint, by name and start time.
    busy: Vec<(String, Option<DateTime<Utc>>)>,
    playing: Playing,
}

impl Clawd {
    pub fn new(ctx: &Context) -> Self {
        let mut rng = Rng::new();
        let now = Instant::now();
        let playing = Playing::new(pick(&mut rng, ASLEEP, ""), now, now + rng.seconds(REST_SPELL));
        let blank = ColorImage::filled(playing.frames[0].image.size, Color32::TRANSPARENT);
        let texture = ctx.load_texture("clawd", blank, OPTIONS);
        Self { texture, rng, mood: Mood::Asleep, since: now, busy: Vec::new(), playing }
    }

    /// Paints Clawd into `rect` (sized by [`SIZE`]) and returns when his next frame is due,
    /// or `None` when `animate` is off and he holds still.
    pub fn paint(&mut self, painter: &Painter, rect: Rect, state: &State, animate: bool) -> Option<Duration> {
        let now = Instant::now();
        let mood = Mood::of(state);
        let busy: Vec<_> = state.sessions.iter().filter(|s| s.busy).map(|s| (s.name.clone(), s.started)).collect();
        let started_working = busy.iter().any(|session| !self.busy.contains(session));
        self.busy = busy;

        if mood != self.mood {
            let marked = animate && now.saturating_duration_since(self.since) >= SPELL_WORTH_MARKING;
            let (pool, once) = match (self.mood, mood) {
                (Mood::Working, _) if marked => (DONE, true),
                (_, Mood::Working) if marked => (GREETING, true),
                _ => (mood.clips(), false),
            };
            self.mood = mood;
            self.since = now;
            self.play(pool, once, now);
        } else if mood == Mood::Working && started_working && !self.playing.once {
            self.play(WORKING, false, now);
        } else if now >= self.playing.ends() {
            self.play(mood.clips(), false, now);
        }

        let (index, lasts) = self.playing.frame(if animate { now } else { self.playing.started });
        if self.playing.shown != Some(index) {
            self.texture.set(self.playing.frames[index].image.clone(), OPTIONS);
            self.playing.shown = Some(index);
        }
        // The clip's drawing centred in the slot, with the frame around it cut off past SPILL.
        let bounds = self.playing.bounds;
        let frame = Rect::from_min_size(rect.center() - bounds.center().to_vec2() * SCALE, FRAME * SCALE);
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect.expand(SPILL)));
        painter.image(self.texture.id(), frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        animate.then(|| lasts.min(self.playing.ends().saturating_duration_since(now)).max(Duration::from_millis(1)))
    }

    /// Starts another clip from `pool`: one play if `once`, otherwise looping for a spell.
    fn play(&mut self, pool: &'static [Clip], once: bool, now: Instant) {
        let clip = pick(&mut self.rng, pool, self.playing.clip.name);
        let due = if once { now } else { now + self.rng.seconds(self.mood.spell()) };
        self.playing = Playing::new(clip, now, due);
        self.playing.once = once;
    }
}

/// A random clip from `pool`, another than `current` when the pool has one.
fn pick(rng: &mut Rng, pool: &'static [Clip], current: &str) -> &'static Clip {
    loop {
        let clip = &pool[rng.below(pool.len())];
        if pool.len() == 1 || clip.name != current {
            return clip;
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
    /// The frame in the texture.
    shown: Option<usize>,
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
        Self { clip, frames, length, bounds, started, due, once: false, shown: None }
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
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Self((nanos ^ (u64::from(std::process::id()) << 32)) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn seconds(&mut self, range: Range<u64>) -> Duration {
        Duration::from_secs(range.start + self.next() % (range.end - range.start))
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
    fn picks_another_clip_when_the_pool_has_one() {
        let mut rng = Rng::new();
        for _ in 0..200 {
            assert_ne!(pick(&mut rng, WORKING, "thinking").name, "thinking");
        }
        assert_eq!(pick(&mut rng, &clips!["idle"], "idle").name, "idle");
        let picked: BTreeSet<&str> = (0..500).map(|_| pick(&mut rng, ASLEEP, "").name).collect();
        assert_eq!(picked.len(), ASLEEP.len(), "every clip gets a turn");
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
        let session = |busy| Session { name: "x".into(), project: None, app: "VS Code".into(), busy, started: None };
        let state = State { working: true, sessions: vec![session(true)], error: Some("down".into()), ..State::default() };
        assert_eq!(Mood::of(&state), Mood::Working);
        let state = State { sessions: vec![session(false)], error: Some("down".into()), ..State::default() };
        assert_eq!(Mood::of(&state), Mood::Error);
        let state = State { sessions: vec![session(false)], ..State::default() };
        assert_eq!(Mood::of(&state), Mood::Waiting);
        assert_eq!(Mood::of(&State::default()), Mood::Asleep);
    }
}
